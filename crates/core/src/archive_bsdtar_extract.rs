//! Extraction of tar(.gz/.bz2/.xz/.zst), 7z, RAR, ZIP64/oversized ZIP and
//! single compressed files through an external tool into private staging.
//!
//! Review lists the archive (no output) and rejects unsafe members and
//! budget overruns before anything is created. Execution re-opens the pinned
//! source, creates a fresh 0700 `.files-extract-<hex>` directory next to the
//! destination, runs the tool there, verifies the staged tree on disk
//! (`archive_bsdtar::verify_tree`), syncs and publishes with
//! `renameat2(RENAME_NOREPLACE)` under a unique name. Failures and
//! cancellation keep the staging directory, renamed to
//! `.files-extract-failed-<hex>`, as evidence; nothing is ever deleted
//! recursively, merged or overwritten.
#![cfg(target_os = "linux")]

use crate::{
    archive_bsdtar::{
        self as tool, ENCRYPTED, Identity, Input, Listing, MAX_ENTRIES, Output, SPACE_RESERVE,
        Spec, Stream, Top, Totals, error,
    },
    archive_format::Format,
    copy_engine::Control,
};
use std::{
    ffi::{OsStr, OsString},
    fs::{self, File},
    io,
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Debug)]
pub struct Prepared {
    source: PathBuf,
    source_id: Identity,
    parent: PathBuf,
    parent_id: (u64, u64),
    name: OsString,
    format: Format,
    entries: u64,
    files: u64,
    directories: u64,
    symlinks: u64,
    bytes: Option<u64>,
    top: Top,
    reported: String,
}

#[derive(Debug)]
pub struct Done {
    pub destination: PathBuf,
    /// The planned name was occupied and a numbered sibling was used.
    pub renamed: bool,
    pub totals: Totals,
    /// Empty staging directory that could not be removed (harmless, reported).
    pub leftover: Option<PathBuf>,
}

#[derive(Debug)]
pub struct Failure {
    pub cause: io::Error,
    /// Staging directory kept as failed evidence; never deleted automatically.
    pub retained: Option<PathBuf>,
    pub cancelled: bool,
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}\nSalida: sin publicar", self.cause)?;
        if let Some(path) = &self.retained {
            write!(
                f,
                "\nTemporal conservado como evidencia (revísalo y bórralo a mano): {}",
                path.display()
            )?;
        }
        Ok(())
    }
}
impl std::error::Error for Failure {}

fn fail(cause: io::Error, retained: Option<PathBuf>) -> Box<Failure> {
    let cancelled = cause.kind() == io::ErrorKind::Interrupted;
    Box::new(Failure {
        cause,
        retained,
        cancelled,
    })
}

fn directory_identity(file: &File) -> io::Result<(u64, u64)> {
    let m = file.metadata()?;
    Ok((m.dev(), m.ino()))
}

impl Prepared {
    /// Read-only review. `name` is the folder used when the archive has
    /// several top-level entries (a single top-level entry keeps its own name).
    pub fn capture(
        source: &Path,
        parent: &Path,
        name: &OsStr,
        control: &Control,
    ) -> io::Result<Self> {
        if !parent.is_absolute() {
            return Err(error("Destino: se requiere ruta absoluta"));
        }
        crate::copy_unique_name::Names::new(name, true, 255)?;
        let parent_dir = tool::open_directory(parent)?;
        let parent_id = directory_identity(&parent_dir)?;
        let (file, format, source_id) = tool::open_source(source)?;
        if let Format::Single(c) = format {
            tool::program(tool::decompressor(c).expect("compressed single file"))?;
        }
        let listing: Listing = tool::list_open(file, format, source, control, 0)?;
        listing.require_safe()?;
        if listing.entries == 0 {
            return Err(error("El archivo está vacío: no hay nada que extraer"));
        }
        let free = tool::free_bytes(&parent_dir)?;
        if let Some(bytes) = listing.bytes
            && bytes.saturating_add(SPACE_RESERVE) > free
        {
            return Err(error(format!(
                "Espacio insuficiente: se necesitan {} más {} de reserva y hay {} libres",
                crate::format_size(bytes),
                crate::format_size(SPACE_RESERVE),
                crate::format_size(free)
            )));
        }
        Ok(Self {
            source: source.into(),
            source_id,
            parent: parent.into(),
            parent_id,
            name: name.into(),
            format,
            entries: listing.entries,
            files: listing.files,
            directories: listing.directories,
            symlinks: listing.symlinks,
            bytes: listing.bytes,
            top: listing.top,
            reported: listing.reported,
        })
    }
    pub fn source(&self) -> &Path {
        &self.source
    }
    pub fn format(&self) -> Format {
        self.format
    }
    /// Planned destination before the no-replace rename picks a free name.
    pub fn destination(&self) -> PathBuf {
        match (&self.top, self.format) {
            (Top::Single { name, .. }, _) => self.parent.join(OsStr::from_bytes(name)),
            _ => self.parent.join(&self.name),
        }
    }
    pub fn declared_bytes(&self) -> Option<u64> {
        self.bytes
    }
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.source.as_os_str().len()
            + self.parent.as_os_str().len()
            + self.name.len()
            + self.reported.len()
            + match &self.top {
                Top::Single { name, .. } => name.len(),
                _ => 0,
            }
    }
    pub fn summary(&self) -> String {
        let size = self
            .bytes
            .map_or_else(|| "tamaño desconocido".to_owned(), crate::format_size);
        format!(
            "{} · {} entradas ({} archivos, {} carpetas, {} enlaces) · {size}\nSe publicará como «{}» sin sobrescribir (nombre numerado si existe).",
            self.format.label(),
            self.entries,
            self.files,
            self.directories,
            self.symlinks,
            self.destination()
                .file_name()
                .map(|n| tool::display(n.as_bytes()))
                .unwrap_or_default()
        )
    }

    /// Run on a worker. `progress(bytes_written, entries_verified)`.
    pub fn execute(
        self,
        control: &Control,
        mut progress: impl FnMut(u64, u64),
    ) -> Result<Done, Box<Failure>> {
        let parent = tool::open_directory(&self.parent).map_err(|e| fail(e, None))?;
        if directory_identity(&parent).map_err(|e| fail(e, None))? != self.parent_id {
            return Err(fail(
                error("La carpeta destino cambió desde la revisión"),
                None,
            ));
        }
        let source = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(&self.source)
            .map_err(|e| fail(e, None))?;
        let now = source.metadata().map_err(|e| fail(e, None))?;
        if Identity::of(&now) != self.source_id {
            return Err(fail(
                error("El archivo comprimido cambió desde la revisión; vuelve a revisar"),
                None,
            ));
        }
        let start_free = tool::free_bytes(&parent).map_err(|e| fail(e, None))?;
        if let Some(bytes) = self.bytes
            && bytes.saturating_add(SPACE_RESERVE) > start_free
        {
            return Err(fail(error("Espacio insuficiente para extraer"), None));
        }
        let budget = start_free.saturating_sub(SPACE_RESERVE);
        control.checkpoint().map_err(|_| {
            fail(
                io::Error::new(io::ErrorKind::Interrupted, "Extracción cancelada"),
                None,
            )
        })?;
        let hex = tool::random_hex().map_err(|e| fail(e, None))?;
        let stage_name = OsString::from(format!(".files-extract-{hex}"));
        let stage_path = self.parent.join(&stage_name);
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&stage_path)
            .map_err(|e| fail(e, None))?;
        let stage = match tool::open_directory(&stage_path) {
            Ok(s) => s,
            Err(e) => return Err(fail(e, Some(stage_path))),
        };
        let retain = |cause: io::Error| -> Box<Failure> {
            let failed = OsString::from(format!(".files-extract-failed-{hex}"));
            let kept = match tool::rename_noreplace(&parent, &stage_name, &parent, &failed) {
                Ok(()) => self.parent.join(failed),
                Err(_) => stage_path.clone(),
            };
            let _ = parent.sync_all();
            fail(cause, Some(kept))
        };
        let run = self.run_tool(
            source,
            &stage,
            &stage_path,
            budget,
            start_free,
            control,
            &mut progress,
        );
        if let Err(cause) = run {
            return Err(retain(cause));
        }
        let mut top = Vec::new();
        match fs::read_dir(&stage_path) {
            Ok(entries) => {
                for entry in entries.take(2) {
                    match entry.and_then(|e| Ok((e.file_name(), e.metadata()?))) {
                        Ok(item) => top.push(item),
                        Err(cause) => return Err(retain(cause)),
                    }
                }
            }
            Err(cause) => return Err(retain(cause)),
        }
        let single = match top.as_slice() {
            [] => return Err(retain(error("La extracción no produjo contenido"))),
            [(name, m)] if m.is_dir() || m.is_file() => Some((name.clone(), m.is_dir())),
            _ => None,
        };
        // Links are confined to what is actually published: the single
        // top-level folder, or the whole staging folder when it is published.
        let verify_root = match &single {
            Some((name, true)) => stage_path.join(name),
            _ => stage_path.clone(),
        };
        let totals = match tool::verify_tree(&verify_root, budget, MAX_ENTRIES, control) {
            Ok(mut t) => {
                if verify_root != stage_path {
                    t.directories += 1;
                }
                t
            }
            Err(cause) => return Err(retain(cause)),
        };
        progress(
            totals.bytes,
            totals.files + totals.directories + totals.symlinks,
        );
        if let Err(cause) = control.checkpoint() {
            return Err(retain(cause));
        }
        // SAFETY: syncfs on our staging descriptor flushes the filesystem.
        if unsafe { libc::syncfs(std::os::fd::AsRawFd::as_raw_fd(&stage)) } != 0 {
            return Err(retain(io::Error::last_os_error()));
        }
        if directory_identity(&parent).ok() != Some(self.parent_id)
            || tool::open_directory(&self.parent)
                .and_then(|p| directory_identity(&p))
                .ok()
                != Some(self.parent_id)
        {
            return Err(retain(error(
                "La carpeta destino cambió durante la extracción",
            )));
        }
        let published = match &single {
            Some((name, directory)) => {
                tool::publish_unique(&stage, name, &parent, name, *directory)
            }
            None => {
                fs::set_permissions(&stage_path, fs::Permissions::from_mode(0o755)).and_then(|()| {
                    tool::publish_unique(&parent, &stage_name, &parent, &self.name, true)
                })
            }
        };
        let (name, renamed) = match published {
            Ok(v) => v,
            Err(cause) => return Err(retain(cause)),
        };
        let mut leftover = None;
        if single.is_some() && fs::remove_dir(&stage_path).is_err() {
            // Only an empty directory is ever removed; keep and report otherwise.
            leftover = Some(stage_path.clone());
        }
        let _ = parent.sync_all();
        Ok(Done {
            destination: self.parent.join(name),
            renamed,
            totals,
            leftover,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn run_tool(
        &self,
        source: File,
        stage: &File,
        stage_path: &Path,
        budget: u64,
        start_free: u64,
        control: &Control,
        progress: &mut impl FnMut(u64, u64),
    ) -> io::Result<()> {
        let mut last = Instant::now();
        let start_inodes = tool::space(stage)?.1;
        let mut output_file: Option<File> = None;
        let mut tick = |written: u64| -> io::Result<()> {
            if last.elapsed() < Duration::from_millis(250) {
                return Ok(());
            }
            last = Instant::now();
            let (free, inodes) = tool::space(stage)?;
            if start_inodes > 0 && start_inodes.saturating_sub(inodes) > MAX_ENTRIES {
                return Err(error(format!(
                    "La extracción supera {MAX_ENTRIES} entradas; detenida"
                )));
            }
            if free < SPACE_RESERVE {
                return Err(error(format!(
                    "Espacio libre por debajo de la reserva de {}; extracción detenida",
                    crate::format_size(SPACE_RESERVE)
                )));
            }
            let used = start_free.saturating_sub(free).max(written);
            if used > budget {
                return Err(error("La extracción supera el espacio presupuestado"));
            }
            progress(used, 0);
            Ok(())
        };
        let mut encrypted = false;
        let (program, args, stdout) = match self.format {
            Format::Single(c) => {
                let name = Format::stem(self.source.file_name().unwrap_or_default());
                let file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o644)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                    .open(stage_path.join(&name))?;
                output_file = Some(file.try_clone()?);
                let tool_name = tool::decompressor(c).expect("compressed single file");
                let mut args = vec!["-d", "-c"];
                if matches!(c, crate::archive_format::Compression::Zstd) {
                    args.push("-q");
                }
                (
                    tool::program(tool_name)?,
                    args.into_iter().map(OsString::from).collect(),
                    Output::File(file),
                )
            }
            _ => {
                let mut args: Vec<OsString> = [
                    "-x",
                    "-f",
                    "-",
                    "--no-same-owner",
                    "--no-same-permissions",
                    "--no-acls",
                    "--no-xattrs",
                    "--no-fflags",
                    "-C",
                ]
                .map(OsString::from)
                .to_vec();
                args.push(stage_path.into());
                (tool::program("bsdtar")?, args, Output::Null)
            }
        };
        let finished = tool::supervise(
            Spec {
                program: &program,
                args,
                cwd: stage_path,
                stdin: Input::File(source),
                stdout,
            },
            control,
            |stream, line, _| {
                if stream == Stream::Err && tool::encrypted_line(line) {
                    encrypted = true;
                    return Err(error(ENCRYPTED));
                }
                Ok(())
            },
            || {
                let written = match &output_file {
                    Some(f) => f.metadata()?.size(),
                    None => 0,
                };
                tick(written)
            },
        );
        let finished = match finished {
            Err(e) if encrypted => return Err(io::Error::new(e.kind(), ENCRYPTED)),
            other => other?,
        };
        if !finished.status.success() {
            return Err(error(format!(
                "La herramienta de extracción falló ({}): {}",
                finished.status,
                finished.stderr.text()
            )));
        }
        if finished.stderr.lines > 0 && !matches!(self.format, Format::Single(_)) {
            // Conservative: bsdtar warnings (renamed/skipped members) fail the run.
            return Err(error(format!(
                "bsdtar avisó durante la extracción: {}",
                finished.stderr.text()
            )));
        }
        if let Some(file) = output_file {
            file.sync_all()?;
        }
        Ok(())
    }
}

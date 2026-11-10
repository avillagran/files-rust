//! Creation of .tar.xz, .tar.gz, .tar.zst and .7z through `bsdtar -c`.
//!
//! Review walks the selected roots with lstat (never following symlinks),
//! rejects devices/FIFOs/sockets, hard-linked files for tar formats (they would
//! become hardlink members that this application refuses to extract), output
//! inside the selection and budget overruns. Execution runs bsdtar with
//! `-C <parent>`, the member names NUL-separated on stdin (`--null -T -`, so a
//! leading `-`, `@` or a newline is never an option or directive), symlinks
//! stored as links, and writes to stdout redirected into an exclusive 0600
//! `.files-create-<hex>.partial` file in the destination directory. The file
//! is synced and published with `renameat2(RENAME_NOREPLACE)` under a unique
//! name. Sources are never modified. On failure only that exact partial file,
//! identified by device/inode, is unlinked.
#![cfg(target_os = "linux")]

use crate::{
    archive_bsdtar::{self as tool, Input, MAX_ENTRIES, Output, SPACE_RESERVE, Spec, error},
    archive_format::CreateFormat,
    copy_engine::Control,
};
use std::{
    ffi::{OsStr, OsString},
    fs, io,
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub const MAX_ROOTS: usize = 4096;

#[derive(Debug)]
pub struct Prepared {
    source: PathBuf,
    names: Vec<OsString>,
    parent: PathBuf,
    parent_id: (u64, u64),
    name: OsString,
    format: CreateFormat,
    entries: u64,
    bytes: u64,
}

#[derive(Debug)]
pub struct Done {
    pub destination: PathBuf,
    pub renamed: bool,
    pub bytes: u64,
    pub entries: u64,
}

#[derive(Debug)]
pub struct Failure {
    pub cause: io::Error,
    pub cancelled: bool,
    /// Partial output that could not be proven ours and was therefore kept.
    pub retained: Option<PathBuf>,
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}\nSalida: sin publicar · origen sin cambios",
            self.cause
        )?;
        if let Some(path) = &self.retained {
            write!(f, "\nArchivo parcial conservado: {}", path.display())?;
        }
        Ok(())
    }
}
impl std::error::Error for Failure {}

fn identity(path: &Path) -> io::Result<(u64, u64)> {
    let m = fs::symlink_metadata(path)?;
    Ok((m.dev(), m.ino()))
}

impl Prepared {
    pub fn capture(
        source: &Path,
        names: &[OsString],
        parent: &Path,
        name: &OsStr,
        control: &Control,
    ) -> io::Result<Self> {
        let format = CreateFormat::from_name(name).ok_or_else(|| {
            error("Formato: el nombre debe terminar en .zip, .tar.xz, .tar.gz, .tar.zst o .7z")
        })?;
        if format == CreateFormat::Zip {
            return Err(error("ZIP usa el compresor nativo"));
        }
        if !source.is_absolute() || !parent.is_absolute() {
            return Err(error("Comprimir: se requieren rutas absolutas"));
        }
        crate::copy_unique_name::Names::new(name, false, 255)?;
        if names.is_empty() || names.len() > MAX_ROOTS {
            return Err(error(format!("Selecciona entre 1 y {MAX_ROOTS} elementos")));
        }
        for n in names {
            let raw = n.as_bytes();
            if raw.is_empty() || raw == b"." || raw == b".." || raw.contains(&b'/') {
                return Err(error("Nombre de origen inválido"));
            }
        }
        tool::program("bsdtar")?;
        let parent_dir = tool::open_directory(parent)?;
        let pm = parent_dir.metadata()?;
        let canonical_parent = fs::canonicalize(parent)?;
        let tar = format != CreateFormat::SevenZip;
        let mut entries = 0u64;
        let mut bytes = 0u64;
        let mut stack: Vec<(PathBuf, usize)> = Vec::new();
        for n in names {
            let root = source.join(n);
            let m = fs::symlink_metadata(&root)?;
            if m.is_dir() && canonical_parent.starts_with(fs::canonicalize(&root)?) {
                return Err(error(
                    "El archivo de salida quedaría dentro de la selección; elige otro destino",
                ));
            }
            stack.push((root, 1));
        }
        while let Some((path, depth)) = stack.pop() {
            if control.is_cancelled() {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Revisión cancelada",
                ));
            }
            let m = fs::symlink_metadata(&path)?;
            entries += 1;
            if entries > MAX_ENTRIES {
                return Err(error(format!("La selección supera {MAX_ENTRIES} entradas")));
            }
            if depth > tool::MAX_DEPTH || path.as_os_str().len() > tool::MAX_PATH_BYTES {
                return Err(error(format!(
                    "Ruta demasiado profunda o larga: {}",
                    tool::display(path.as_os_str().as_bytes())
                )));
            }
            let kind = m.file_type();
            if kind.is_symlink() {
                continue;
            } else if kind.is_dir() {
                for entry in fs::read_dir(&path)? {
                    stack.push((entry?.path(), depth + 1));
                }
            } else if kind.is_file() {
                if tar && m.nlink() > 1 {
                    return Err(error(format!(
                        "«{}» tiene enlaces duros; usa .zip o .7z (tar los guardaría como enlace duro)",
                        tool::display(path.as_os_str().as_bytes())
                    )));
                }
                bytes = bytes.saturating_add(m.size());
            } else {
                return Err(error(format!(
                    "«{}» es un dispositivo, FIFO o socket: no se comprime",
                    tool::display(path.as_os_str().as_bytes())
                )));
            }
        }
        let free = tool::free_bytes(&parent_dir)?;
        if bytes.saturating_add(SPACE_RESERVE) > free {
            return Err(error(format!(
                "Espacio insuficiente: hasta {} de salida más {} de reserva; hay {} libres",
                crate::format_size(bytes),
                crate::format_size(SPACE_RESERVE),
                crate::format_size(free)
            )));
        }
        Ok(Self {
            source: source.into(),
            names: names.to_vec(),
            parent: parent.into(),
            parent_id: (pm.dev(), pm.ino()),
            name: name.into(),
            format,
            entries,
            bytes,
        })
    }
    pub fn destination(&self) -> PathBuf {
        self.parent.join(&self.name)
    }
    pub fn entries(&self) -> u64 {
        self.entries
    }
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
    pub fn format(&self) -> CreateFormat {
        self.format
    }
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.source.as_os_str().len()
            + self.parent.as_os_str().len()
            + self.name.len()
            + self.names.iter().map(|n| n.len() + 24).sum::<usize>()
    }
    /// `-f /proc/self/fd/1` (our exclusive file) instead of `-f -`: libarchive
    /// pads stdout to a 10 KiB block, which corrupts zstd streams; a regular
    /// file opened by name is written unpadded.
    fn arguments(&self) -> Vec<OsString> {
        let mut args: Vec<OsString> = [
            "-c",
            "-f",
            "/proc/self/fd/1",
            "--no-acls",
            "--no-xattrs",
            "--no-fflags",
        ]
        .map(OsString::from)
        .to_vec();
        match self.format {
            CreateFormat::TarXz => args.extend(["--format", "pax", "--xz"].map(OsString::from)),
            CreateFormat::TarGz => args.extend(["--format", "pax", "--gzip"].map(OsString::from)),
            CreateFormat::TarZst => args.extend(["--format", "pax", "--zstd"].map(OsString::from)),
            CreateFormat::SevenZip => args.extend(["--format", "7zip"].map(OsString::from)),
            CreateFormat::Zip => args.extend(["--format", "zip"].map(OsString::from)),
        }
        args.push("-C".into());
        args.push(self.source.clone().into());
        args.extend(["--null", "-T", "-"].map(OsString::from));
        args
    }

    /// Run on a worker. `progress(bytes_written, entries_reviewed)`.
    pub fn execute(
        self,
        control: &Control,
        mut progress: impl FnMut(u64, u64),
    ) -> Result<Done, Box<Failure>> {
        let failure = |cause: io::Error, retained: Option<PathBuf>| {
            Box::new(Failure {
                cancelled: cause.kind() == io::ErrorKind::Interrupted,
                cause,
                retained,
            })
        };
        let parent = tool::open_directory(&self.parent).map_err(|e| failure(e, None))?;
        let m = parent.metadata().map_err(|e| failure(e, None))?;
        if (m.dev(), m.ino()) != self.parent_id {
            return Err(failure(
                error("La carpeta destino cambió desde la revisión"),
                None,
            ));
        }
        control.checkpoint().map_err(|e| failure(e, None))?;
        let hex = tool::random_hex().map_err(|e| failure(e, None))?;
        let partial = OsString::from(format!(".files-create-{hex}.partial"));
        let partial_path = self.parent.join(&partial);
        let output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&partial_path)
            .map_err(|e| failure(e, None))?;
        let ours = {
            let m = output
                .metadata()
                .map_err(|e| failure(e, Some(partial_path.clone())))?;
            (m.dev(), m.ino())
        };
        // Unlink exactly our partial file (single file, never a tree).
        let discard = |cause: io::Error| {
            if identity(&partial_path).ok() == Some(ours) && fs::remove_file(&partial_path).is_ok()
            {
                let _ = parent.sync_all();
                failure(cause, None)
            } else {
                failure(cause, Some(partial_path.clone()))
            }
        };
        let mut list = Vec::new();
        for n in &self.names {
            list.extend_from_slice(n.as_bytes());
            list.push(0);
        }
        let program = match tool::program("bsdtar") {
            Ok(p) => p,
            Err(e) => return Err(discard(e)),
        };
        let watch = match output.try_clone() {
            Ok(f) => f,
            Err(e) => return Err(discard(e)),
        };
        let start_free = tool::free_bytes(&parent).unwrap_or(u64::MAX);
        let mut last = Instant::now();
        let entries = self.entries;
        let finished = tool::supervise(
            Spec {
                program: &program,
                args: self.arguments(),
                cwd: &self.source,
                stdin: Input::Bytes(list),
                stdout: Output::File(output),
            },
            control,
            |_, _, _| Ok(()),
            || {
                if last.elapsed() < Duration::from_millis(250) {
                    return Ok(());
                }
                last = Instant::now();
                let written = watch.metadata()?.size();
                let free = tool::free_bytes(&watch)?;
                if free < SPACE_RESERVE && start_free >= SPACE_RESERVE {
                    return Err(error(format!(
                        "Espacio libre por debajo de la reserva de {}; compresión detenida",
                        crate::format_size(SPACE_RESERVE)
                    )));
                }
                progress(written, 0);
                Ok(())
            },
        );
        let finished = match finished {
            Ok(f) => f,
            Err(e) => return Err(discard(e)),
        };
        if !finished.status.success() || finished.stderr.lines > 0 {
            return Err(discard(error(format!(
                "bsdtar no pudo crear el archivo ({}): {}",
                finished.status,
                finished.stderr.text()
            ))));
        }
        if let Err(e) = watch.sync_all() {
            return Err(discard(e));
        }
        let written = watch.metadata().map(|m| m.size()).unwrap_or(0);
        if let Err(e) = control.checkpoint() {
            return Err(discard(e));
        }
        let (name, renamed) =
            match tool::publish_unique(&parent, &partial, &parent, &self.name, false) {
                Ok(v) => v,
                Err(e) => return Err(discard(e)),
            };
        let _ = parent.sync_all();
        progress(written, entries);
        Ok(Done {
            destination: self.parent.join(name),
            renamed,
            bytes: written,
            entries,
        })
    }
}

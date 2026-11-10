//! External archive tools as ephemeral child processes: libarchive `bsdtar`
//! (BSD-2-Clause, part of the Arch/Omarchy base system) and the stock
//! `gzip`/`bzip2`/`xz`/`zstd` decompressors for single compressed files.
//!
//! Child contract: fixed argv from fixed system directories (never `$PATH`, no
//! shell), `LC_ALL=C.UTF-8`, cleared environment, a new session without controlling
//! terminal (so a passphrase prompt can never block on a TTY), `SIGKILL` on
//! parent death, on cancellation and on drop of the guard, bounded line and
//! stderr capture, and `SIGSTOP`/`SIGCONT` of the whole group while paused.
//!
//! Listing output is untrusted: every name is unescaped from bsdtar's
//! C-locale escaping and checked lexically. Extraction safety does not rely on
//! the listing; `verify_tree` re-checks the staged tree on disk.
#![cfg(target_os = "linux")]

use crate::{
    archive_format::{Compression, Format},
    copy_engine::Control,
};
use std::{
    collections::VecDeque,
    ffi::{OsStr, OsString},
    fs::{self, File},
    io::{self, Read},
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::{FileExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, ExitStatus, Stdio},
    sync::mpsc::{self, RecvTimeoutError, SyncSender},
    time::Duration,
};

/// Hard ceiling on archive members (listing, runtime and staged tree).
pub const MAX_ENTRIES: u64 = 2_000_000;
/// Relative path bytes below the staging root (Linux PATH_MAX).
pub const MAX_PATH_BYTES: usize = 4096;
pub const MAX_COMPONENT_BYTES: usize = 255;
pub const MAX_DEPTH: usize = 128;
/// Free space that extraction/creation must leave on the destination filesystem.
pub const SPACE_RESERVE: u64 = 256 * 1024 * 1024;
const MAX_LINE: usize = 64 * 1024;
const DIAGNOSTIC_HEAD: usize = 2048;
const DIAGNOSTIC_TAIL: usize = 2048;
const TOOL_DIRS: [&str; 2] = ["/usr/bin", "/bin"];
pub const ENCRYPTED: &str = "archivo cifrado: no admitido todavía";

pub(crate) fn error(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}

/// Locate a tool in fixed system directories only.
pub fn program(name: &str) -> io::Result<PathBuf> {
    program_in(&TOOL_DIRS, name)
}
pub fn program_in(dirs: &[&str], name: &str) -> io::Result<PathBuf> {
    dirs.iter()
        .map(|dir| Path::new(dir).join(name))
        .find(|path| {
            fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("formato no disponible: falta {name}"),
            )
        })
}
pub fn decompressor(c: Compression) -> Option<&'static str> {
    match c {
        Compression::Gzip => Some("gzip"),
        Compression::Bzip2 => Some("bzip2"),
        Compression::Xz => Some("xz"),
        Compression::Zstd => Some("zstd"),
        Compression::None => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Identity {
    pub dev: u64,
    pub ino: u64,
    pub size: u64,
    pub mtime: i64,
    pub mtime_nsec: i64,
}
impl Identity {
    pub fn of(m: &fs::Metadata) -> Self {
        Self {
            dev: m.dev(),
            ino: m.ino(),
            size: m.size(),
            mtime: m.mtime(),
            mtime_nsec: m.mtime_nsec(),
        }
    }
}

/// Open a regular archive without following a final symlink.
pub(crate) fn open_source(path: &Path) -> io::Result<(File, Format, Identity)> {
    if !path.is_absolute() || path.as_os_str().len() > MAX_PATH_BYTES {
        return Err(error("Archivo: se requiere ruta absoluta de hasta 4096 B"));
    }
    let name = path
        .file_name()
        .ok_or_else(|| error("Archivo sin nombre"))?;
    let format = Format::from_name(name).ok_or_else(|| {
        error("formato no reconocido: usa zip, tar(.gz/.bz2/.xz/.zst), 7z, rar, gz, bz2, xz o zst")
    })?;
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(error("El archivo comprimido debe ser un archivo regular"));
    }
    let mut head = [0u8; 512];
    let mut filled = 0;
    while filled < head.len() {
        match file.read_at(&mut head[filled..], filled as u64) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    if !format.magic_matches(&head[..filled]) {
        return Err(error(format!(
            "El contenido no corresponde a {}; no se abre con otro lector",
            format.label()
        )));
    }
    Ok((file, format, Identity::of(&metadata)))
}

pub(crate) fn free_bytes(file: &File) -> io::Result<u64> {
    space(file).map(|(bytes, _)| bytes)
}
/// Available bytes and inodes (inodes are 0 on filesystems without a fixed table).
pub(crate) fn space(file: &File) -> io::Result<(u64, u64)> {
    use std::os::fd::AsRawFd;
    let mut s = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: fstatvfs fills the struct when it returns 0.
    if unsafe { libc::fstatvfs(file.as_raw_fd(), s.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let s = unsafe { s.assume_init() };
    #[allow(clippy::unnecessary_cast)]
    Ok((
        (s.f_bavail as u64).saturating_mul(s.f_frsize as u64),
        s.f_favail as u64,
    ))
}

// ---------------------------------------------------------------- children

pub(crate) enum Input {
    #[cfg_attr(not(test), allow(dead_code))]
    Null,
    File(File),
    Bytes(Vec<u8>),
}
pub(crate) enum Output {
    Null,
    File(File),
    Lines,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stream {
    Out,
    Err,
}
pub(crate) struct Spec<'a> {
    pub program: &'a Path,
    pub args: Vec<OsString>,
    pub cwd: &'a Path,
    pub stdin: Input,
    pub stdout: Output,
}
enum Event {
    Line(Stream, Vec<u8>, bool),
    End,
}

#[derive(Default, Debug)]
pub(crate) struct Diagnostic {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    omitted: bool,
    pub lines: u64,
}
impl Diagnostic {
    fn push(&mut self, line: &[u8]) {
        self.lines += 1;
        for &b in line.iter().chain(b"\n") {
            if self.head.len() < DIAGNOSTIC_HEAD {
                self.head.push(b);
            } else {
                if self.tail.len() == DIAGNOSTIC_TAIL {
                    self.tail.pop_front();
                    self.omitted = true;
                }
                self.tail.push_back(b);
            }
        }
    }
    pub fn text(&self) -> String {
        let mut raw = self.head.clone();
        if self.omitted {
            raw.extend_from_slice(b"\n[...]\n");
        }
        raw.extend(self.tail.iter());
        String::from_utf8_lossy(&raw)
            .chars()
            .map(|c| if c.is_control() && c != '\n' { '?' } else { c })
            .collect::<String>()
            .trim()
            .to_owned()
    }
}

pub(crate) fn encrypted_line(line: &[u8]) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.windows(10).any(|w| w == b"passphrase")
        || lower.windows(7).any(|w| w == b"encrypt")
        || lower.windows(7).any(|w| w == b"decrypt")
}

struct Guard {
    child: Child,
    reaped: bool,
}
impl Guard {
    fn signal(&self, signal: i32) {
        // The child called setsid(), so its pid is also its process group id.
        let pid = self.child.id() as i32;
        // SAFETY: plain kill(2) on our own child's group.
        unsafe {
            libc::kill(-pid, signal);
        }
    }
    fn kill(&mut self) {
        if !self.reaped {
            self.signal(libc::SIGKILL);
            let _ = self.child.kill();
            let _ = self.child.wait();
            self.reaped = true;
        }
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        self.kill();
    }
}

fn pump(mut reader: impl Read, stream: Stream, tx: SyncSender<Event>) {
    let mut buffer = [0u8; 16 * 1024];
    let mut line = Vec::new();
    let mut truncated = false;
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                let mut rest = &buffer[..n];
                while let Some(at) = rest.iter().position(|&b| b == b'\n') {
                    let room = MAX_LINE.saturating_sub(line.len());
                    truncated |= at > room;
                    line.extend_from_slice(&rest[..at.min(room)]);
                    if tx
                        .send(Event::Line(stream, std::mem::take(&mut line), truncated))
                        .is_err()
                    {
                        return;
                    }
                    truncated = false;
                    rest = &rest[at + 1..];
                }
                let room = MAX_LINE.saturating_sub(line.len());
                truncated |= rest.len() > room;
                line.extend_from_slice(&rest[..rest.len().min(room)]);
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    if !line.is_empty() {
        let _ = tx.send(Event::Line(stream, line, truncated));
    }
    let _ = tx.send(Event::End);
}

pub(crate) struct Finished {
    pub status: ExitStatus,
    pub stderr: Diagnostic,
}

/// Run one child to completion. `on_line` sees each bounded line (with a
/// truncation flag); `tick` runs at least every 100 ms. An error from either,
/// or cancellation, kills the whole child group before returning.
pub(crate) fn supervise(
    spec: Spec<'_>,
    control: &Control,
    mut on_line: impl FnMut(Stream, &[u8], bool) -> io::Result<()>,
    mut tick: impl FnMut() -> io::Result<()>,
) -> io::Result<Finished> {
    let mut command = std::process::Command::new(spec.program);
    command
        .args(&spec.args)
        .current_dir(spec.cwd)
        .env_clear()
        // C.UTF-8, not plain C: libarchive converts 7z/ZIP/pax names to the
        // locale charset and skips every non-ASCII member under "C".
        .env("LC_ALL", "C.UTF-8")
        .env("PATH", "/usr/bin:/bin")
        .stderr(Stdio::piped());
    let mut feed = None;
    match spec.stdin {
        Input::Null => {
            command.stdin(Stdio::null());
        }
        Input::File(file) => {
            command.stdin(Stdio::from(file));
        }
        Input::Bytes(bytes) => {
            command.stdin(Stdio::piped());
            feed = Some(bytes);
        }
    }
    let lines = matches!(spec.stdout, Output::Lines);
    match spec.stdout {
        Output::Null => command.stdout(Stdio::null()),
        Output::File(file) => command.stdout(Stdio::from(file)),
        Output::Lines => command.stdout(Stdio::piped()),
    };
    // SAFETY: getpid() is read before fork; the closure only calls
    // async-signal-safe functions (setsid, prctl, getppid, _exit).
    let parent = unsafe { libc::getpid() };
    unsafe {
        command.pre_exec(move || {
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong) != 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::getppid() != parent {
                libc::_exit(127);
            }
            Ok(())
        });
    }
    let mut guard = Guard {
        child: command.spawn()?,
        reaped: false,
    };
    let (tx, rx) = mpsc::sync_channel::<Event>(64);
    let mut streams = 1;
    if let Some(stderr) = guard.child.stderr.take() {
        let tx = tx.clone();
        std::thread::spawn(move || pump(stderr, Stream::Err, tx));
    }
    if lines && let Some(stdout) = guard.child.stdout.take() {
        streams += 1;
        let tx = tx.clone();
        std::thread::spawn(move || pump(stdout, Stream::Out, tx));
    }
    drop(tx);
    if let (Some(bytes), Some(mut stdin)) = (feed, guard.child.stdin.take()) {
        std::thread::spawn(move || {
            use std::io::Write;
            let _ = stdin.write_all(&bytes);
            // Dropping stdin closes the pipe: no further input can be read.
        });
    }
    let mut diagnostic = Diagnostic::default();
    let mut ended = 0;
    let mut stopped = false;
    let fail = |guard: &mut Guard, e: io::Error| {
        guard.kill();
        Err(e)
    };
    loop {
        if control.is_cancelled() {
            return fail(
                &mut guard,
                io::Error::new(io::ErrorKind::Interrupted, "Operación cancelada"),
            );
        }
        let paused = control.is_paused();
        if paused != stopped {
            guard.signal(if paused { libc::SIGSTOP } else { libc::SIGCONT });
            stopped = paused;
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Event::Line(stream, line, truncated)) => {
                if stream == Stream::Err {
                    diagnostic.push(&line);
                }
                if let Err(e) = on_line(stream, &line, truncated) {
                    return fail(&mut guard, e);
                }
            }
            Ok(Event::End) => ended += 1,
            Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => {}
        }
        if let Err(e) = tick() {
            return fail(&mut guard, e);
        }
        if ended >= streams
            && !stopped
            && let Some(status) = guard.child.try_wait()?
        {
            guard.reaped = true;
            return Ok(Finished {
                status,
                stderr: diagnostic,
            });
        }
    }
}

// ---------------------------------------------------------------- listing

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Directory,
    Symlink,
    Hardlink,
    Special(u8),
}
#[derive(Clone, Debug)]
pub struct Row {
    pub kind: Kind,
    /// Raw member name after unescaping bsdtar's C-locale display escapes.
    pub name: Vec<u8>,
    pub size: u64,
    pub mode: String,
    pub target: Option<Vec<u8>>,
    /// First safety problem of this member, if any.
    pub problem: Option<&'static str>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Top {
    Empty,
    Single { name: Vec<u8>, directory: bool },
    Many,
}
#[derive(Clone, Debug)]
pub struct Listing {
    pub format: Format,
    pub reported: String,
    pub entries: u64,
    pub files: u64,
    pub directories: u64,
    pub symlinks: u64,
    /// Declared uncompressed bytes; unknown for single compressed files.
    pub bytes: Option<u64>,
    pub top: Top,
    pub rows: Vec<Row>,
    pub unsafe_entries: u64,
    pub first_problem: Option<(Vec<u8>, &'static str)>,
}
impl Listing {
    pub fn require_safe(&self) -> io::Result<()> {
        if let Some((name, problem)) = &self.first_problem {
            return Err(error(format!(
                "Archivo rechazado: {problem} en «{}» ({} entradas inseguras); no se extrae nada",
                display(name),
                self.unsafe_entries
            )));
        }
        Ok(())
    }
}

/// Human display of raw bytes: control, backslash and non-UTF-8 escaped.
pub fn display(raw: &[u8]) -> String {
    crate::archive_list::display_name(raw)
}

fn unescape(text: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        let b = text[i];
        if b != b'\\' {
            out.push(b);
            i += 1;
            continue;
        }
        let next = *text.get(i + 1)?;
        let simple = match next {
            b'\\' => Some(b'\\'),
            b'a' => Some(7),
            b'b' => Some(8),
            b'f' => Some(12),
            b'n' => Some(b'\n'),
            b'r' => Some(b'\r'),
            b't' => Some(b'\t'),
            b'v' => Some(11),
            _ => None,
        };
        if let Some(value) = simple {
            out.push(value);
            i += 2;
            continue;
        }
        let digits = text.get(i + 1..i + 4)?;
        if !digits.iter().all(|d| (b'0'..=b'7').contains(d)) {
            return None;
        }
        let value = (digits[0] - b'0') as u32 * 64
            + (digits[1] - b'0') as u32 * 8
            + (digits[2] - b'0') as u32;
        out.push(u8::try_from(value).ok()?);
        i += 4;
    }
    (!out.contains(&0)).then_some(out)
}

/// Split one `bsdtar -tv --numeric-owner` line: mode, links, uid, gid, size,
/// three date fields, then the escaped name (and link target).
pub(crate) fn parse_row(line: &[u8]) -> Option<Row> {
    let text = std::str::from_utf8(line).ok()?;
    let mut rest = text;
    let mut fields = Vec::with_capacity(8);
    for _ in 0..8 {
        let trimmed = rest.trim_start_matches(' ');
        let end = trimmed.find(' ')?;
        fields.push(&trimmed[..end]);
        rest = &trimmed[end..];
    }
    let name = rest.strip_prefix(' ')?;
    let mode = fields[0];
    if mode.len() < 10 {
        return None;
    }
    let kind = match mode.as_bytes()[0] {
        b'-' => Kind::File,
        b'd' => Kind::Directory,
        b'l' => Kind::Symlink,
        b'h' => Kind::Hardlink,
        other => Kind::Special(other),
    };
    let size = if matches!(kind, Kind::Special(_)) {
        0
    } else {
        fields[4].parse().ok()?
    };
    let split = |sep: &str| -> (Option<&str>, Option<&str>) {
        let mut parts = name.match_indices(sep);
        match (parts.next(), parts.next()) {
            (Some((at, _)), None) => (Some(&name[..at]), Some(&name[at + sep.len()..])),
            _ => (None, None),
        }
    };
    let (name, target) = match kind {
        Kind::Symlink => match split(" -> ") {
            (Some(n), Some(t)) => (n, Some(t)),
            _ => (name, None),
        },
        Kind::Hardlink => match split(" link to ") {
            (Some(n), Some(t)) => (n, Some(t)),
            _ => (name, None),
        },
        _ => (name, None),
    };
    let name = name.strip_suffix('/').unwrap_or(name);
    Some(Row {
        kind,
        name: unescape(name.as_bytes())?,
        size,
        mode: mode.into(),
        target: match target {
            Some(t) => Some(unescape(t.as_bytes())?),
            None => None,
        },
        problem: None,
    })
}

/// Lexical member path components, or the reason the name is unsafe.
pub(crate) fn components(name: &[u8]) -> Result<Vec<&[u8]>, &'static str> {
    if name.first() == Some(&b'/') {
        return Err("ruta absoluta");
    }
    if name.len() > MAX_PATH_BYTES {
        return Err("ruta demasiado larga");
    }
    let mut out = Vec::new();
    for part in name.split(|&b| b == b'/') {
        match part {
            b"" | b"." => {}
            b".." => return Err("ruta con «..»"),
            p if p.len() > MAX_COMPONENT_BYTES => return Err("nombre de más de 255 B"),
            p => out.push(p),
        }
    }
    if out.len() > MAX_DEPTH {
        return Err("demasiados niveles de carpetas");
    }
    Ok(out)
}

/// Lexical confinement of a link target relative to the link's directory.
fn lexically_confined(parent_depth: usize, target: &[u8]) -> bool {
    if target.is_empty() || target.first() == Some(&b'/') {
        return false;
    }
    let mut depth = parent_depth as i64;
    for part in target.split(|&b| b == b'/') {
        match part {
            b"" | b"." => {}
            b".." => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => depth += 1,
        }
    }
    true
}

struct Collector {
    listing: Listing,
    max_rows: usize,
    max_entries: u64,
    bytes: u64,
}
impl Collector {
    fn new(format: Format, max_rows: usize) -> Self {
        Self::limited(format, max_rows, MAX_ENTRIES)
    }
    fn limited(format: Format, max_rows: usize, max_entries: u64) -> Self {
        Self {
            listing: Listing {
                format,
                reported: String::new(),
                entries: 0,
                files: 0,
                directories: 0,
                symlinks: 0,
                bytes: Some(0),
                top: Top::Empty,
                rows: Vec::new(),
                unsafe_entries: 0,
                first_problem: None,
            },
            max_rows,
            max_entries,
            bytes: 0,
        }
    }
    fn add(&mut self, mut row: Row) -> io::Result<()> {
        let l = &mut self.listing;
        l.entries += 1;
        if l.entries > self.max_entries {
            return Err(error(format!(
                "Archivo rechazado: más de {} entradas (límite de extracción)",
                self.max_entries
            )));
        }
        let parts = components(&row.name);
        let problem = match (&parts, &row.kind) {
            (Err(reason), _) => Some(*reason),
            (_, Kind::Hardlink) => Some("enlace duro (no admitido)"),
            (_, Kind::Special(_)) => Some("dispositivo, FIFO o socket"),
            (Ok(p), Kind::Symlink) => match &row.target {
                Some(t) if t.first() == Some(&b'/') => Some("enlace simbólico absoluto"),
                Some(t) if !lexically_confined(p.len().saturating_sub(1), t) => {
                    Some("enlace simbólico que sale del archivo")
                }
                _ => None,
            },
            _ => None,
        };
        match row.kind {
            Kind::File => {
                l.files += 1;
                self.bytes = self.bytes.saturating_add(row.size);
            }
            Kind::Directory => l.directories += 1,
            Kind::Symlink => l.symlinks += 1,
            _ => {}
        }
        if let Some(reason) = problem {
            l.unsafe_entries += 1;
            if l.first_problem.is_none() {
                l.first_problem = Some((row.name.clone(), reason));
            }
        }
        if let Ok(parts) = &parts
            && let Some(first) = parts.first()
        {
            let directory = parts.len() > 1 || row.kind == Kind::Directory;
            l.top = match std::mem::replace(&mut l.top, Top::Empty) {
                Top::Empty => Top::Single {
                    name: first.to_vec(),
                    directory,
                },
                Top::Single {
                    name,
                    directory: was,
                } if name == *first => Top::Single {
                    name,
                    directory: was || directory,
                },
                _ => Top::Many,
            };
        }
        row.problem = problem;
        if l.rows.len() < self.max_rows {
            l.rows.push(row);
        }
        Ok(())
    }
}

/// List an archive without extracting anything. For tar/zip/7z/rar the
/// reader is `bsdtar -tvv`; its reported reader must match the extension.
pub fn list(path: &Path, control: &Control, max_rows: usize) -> io::Result<Listing> {
    let (file, format, _) = open_source(path)?;
    list_open(file, format, path, control, max_rows)
}

pub(crate) fn list_open(
    file: File,
    format: Format,
    path: &Path,
    control: &Control,
    max_rows: usize,
) -> io::Result<Listing> {
    list_limited(file, format, path, control, max_rows, MAX_ENTRIES)
}
pub(crate) fn list_limited(
    file: File,
    format: Format,
    path: &Path,
    control: &Control,
    max_rows: usize,
    max_entries: u64,
) -> io::Result<Listing> {
    if let Format::Single(_) = format {
        let name = Format::stem(path.file_name().unwrap_or_default());
        let mut collector = Collector::new(format, max_rows);
        collector.add(Row {
            kind: Kind::File,
            name: name.into_vec(),
            size: 0,
            mode: "-rw-r--r--".into(),
            target: None,
            problem: None,
        })?;
        let mut listing = collector.listing;
        listing.bytes = None;
        listing.reported = format!("Archivo único comprimido con {}", format.label());
        return Ok(listing);
    }
    let bsdtar = program("bsdtar")?;
    let mut collector = Collector::limited(format, max_rows, max_entries);
    let mut reported = None;
    let mut encrypted = false;
    let finished = supervise(
        Spec {
            program: &bsdtar,
            args: ["-t", "-v", "-v", "--numeric-owner", "-f", "-"]
                .map(OsString::from)
                .to_vec(),
            cwd: Path::new("/"),
            stdin: Input::File(file),
            stdout: Output::Lines,
        },
        control,
        |stream, line, truncated| match stream {
            Stream::Err => {
                if encrypted_line(line) {
                    encrypted = true;
                    return Err(error(ENCRYPTED));
                }
                Ok(())
            }
            Stream::Out => {
                if let Some(rest) = line.strip_prefix(b"Archive Format: ") {
                    reported = Some(String::from_utf8_lossy(rest).into_owned());
                    return Ok(());
                }
                if reported.is_some() {
                    return Err(error("Listado de bsdtar fuera de contrato"));
                }
                let row = (!truncated)
                    .then(|| parse_row(line))
                    .flatten()
                    .ok_or_else(|| error("Línea de listado no reconocida; no se extrae"))?;
                collector.add(row)
            }
        },
        || Ok(()),
    );
    let finished = match finished {
        Err(e) if encrypted => return Err(io::Error::new(e.kind(), ENCRYPTED)),
        other => other?,
    };
    if !finished.status.success() {
        return Err(error(format!(
            "bsdtar no pudo leer el archivo: {}",
            finished.stderr.text()
        )));
    }
    let reported = reported.ok_or_else(|| error("bsdtar no informó el formato"))?;
    let lower = reported.to_ascii_lowercase();
    if lower.contains("aes") || lower.contains("encrypt") {
        return Err(error(ENCRYPTED));
    }
    if !format.reader_matches(&reported) {
        return Err(error(format!(
            "Formato interno no admitido para .{}: {reported}",
            format.label()
        )));
    }
    let mut listing = collector.listing;
    listing.reported = reported;
    listing.bytes = Some(collector.bytes);
    Ok(listing)
}

// ---------------------------------------------------------------- staged tree

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    pub files: u64,
    pub directories: u64,
    pub symlinks: u64,
    pub bytes: u64,
}

/// Resolve a symlink target inside `root` exactly as the kernel would, using
/// lstat/readlink on the staged tree. Returns false when any step leaves the
/// root (absolute target, `..` above it, or a loop). Missing components end
/// symlink expansion; the remainder is then purely lexical from the real depth.
pub(crate) fn confined(root: &Path, link: &[Vec<u8>], target: &[u8]) -> io::Result<bool> {
    let mut stack: Vec<Vec<u8>> = link[..link.len().saturating_sub(1)].to_vec();
    let mut pending: VecDeque<Vec<u8>> = VecDeque::new();
    if target.first() == Some(&b'/') || target.is_empty() {
        return Ok(false);
    }
    pending.extend(target.split(|&b| b == b'/').map(<[u8]>::to_vec));
    let mut hops = 0;
    let mut missing = false;
    while let Some(part) = pending.pop_front() {
        match part.as_slice() {
            b"" | b"." => continue,
            b".." => {
                if stack.pop().is_none() {
                    return Ok(false);
                }
                continue;
            }
            _ => {}
        }
        stack.push(part);
        if missing {
            continue;
        }
        let mut path = root.to_path_buf();
        for c in &stack {
            path.push(OsStr::from_bytes(c));
        }
        match fs::symlink_metadata(&path) {
            Ok(m) if m.file_type().is_symlink() => {
                hops += 1;
                if hops > 40 {
                    return Ok(false);
                }
                let next = fs::read_link(&path)?.into_os_string().into_vec();
                if next.first() == Some(&b'/') || next.is_empty() {
                    return Ok(false);
                }
                stack.pop();
                for c in next.split(|&b| b == b'/').rev() {
                    pending.push_front(c.to_vec());
                }
            }
            Ok(_) => {}
            Err(e)
                if e.kind() == io::ErrorKind::NotFound
                    || e.raw_os_error() == Some(libc::ENOTDIR) =>
            {
                missing = true;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(true)
}

/// Walk the staged tree without following symlinks and enforce the contract:
/// only regular files (single link), directories and confined symlinks;
/// path/depth/entry/byte budgets. Special permission bits are stripped and
/// directories get owner rwx so the tree stays traversable and removable.
pub(crate) fn verify_tree(
    root: &Path,
    max_bytes: u64,
    max_entries: u64,
    control: &Control,
) -> io::Result<Totals> {
    let mut totals = Totals::default();
    let mut entries = 0u64;
    let mut stack: Vec<Vec<Vec<u8>>> = vec![Vec::new()];
    while let Some(relative) = stack.pop() {
        if control.is_cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Verificación cancelada",
            ));
        }
        let mut directory = root.to_path_buf();
        for c in &relative {
            directory.push(OsStr::from_bytes(c));
        }
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            entries += 1;
            if entries > max_entries {
                return Err(error(format!(
                    "Contenido extraído supera {max_entries} entradas"
                )));
            }
            let mut child = relative.clone();
            child.push(entry.file_name().into_vec());
            let length: usize = child.iter().map(|c| c.len() + 1).sum();
            if length > MAX_PATH_BYTES || child.len() > MAX_DEPTH {
                return Err(error("Contenido extraído con ruta o profundidad excesiva"));
            }
            let path = entry.path();
            // DirEntry::metadata uses fstatat(AT_SYMLINK_NOFOLLOW).
            let metadata = entry.metadata()?;
            let kind = metadata.file_type();
            let mode = metadata.mode() & 0o7777;
            let shown = || display(&child.join(&b'/'));
            if kind.is_symlink() {
                let target = fs::read_link(&path)?.into_os_string().into_vec();
                if !confined(root, &child, &target)? {
                    return Err(error(format!(
                        "Enlace simbólico «{}» apunta fuera de la carpeta extraída",
                        shown()
                    )));
                }
                totals.symlinks += 1;
            } else if kind.is_dir() {
                if mode & 0o7700 != 0o700 {
                    fs::set_permissions(&path, fs::Permissions::from_mode((mode & 0o777) | 0o700))?;
                }
                totals.directories += 1;
                stack.push(child);
            } else if kind.is_file() {
                if metadata.nlink() != 1 {
                    return Err(error(format!("Enlace duro en «{}»: no admitido", shown())));
                }
                if mode & 0o7000 != 0 {
                    fs::set_permissions(&path, fs::Permissions::from_mode(mode & 0o777))?;
                }
                totals.files += 1;
                totals.bytes = totals.bytes.saturating_add(metadata.size());
                if totals.bytes > max_bytes {
                    return Err(error("Contenido extraído supera el espacio presupuestado"));
                }
            } else {
                return Err(error(format!(
                    "«{}» es un dispositivo, FIFO o socket: no admitido",
                    shown()
                )));
            }
        }
    }
    Ok(totals)
}

// ---------------------------------------------------------------- names

pub(crate) fn random_hex() -> io::Result<String> {
    let mut data = [0u8; 8];
    let mut at = 0;
    while at < data.len() {
        // SAFETY: getrandom writes at most the remaining buffer length.
        let n = unsafe { libc::getrandom(data[at..].as_mut_ptr().cast(), data.len() - at, 0) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        at += n as usize;
    }
    Ok(data.iter().map(|b| format!("{b:02x}")).collect())
}

fn c_name(name: &OsStr) -> io::Result<std::ffi::CString> {
    std::ffi::CString::new(name.as_bytes()).map_err(|_| error("NUL en nombre"))
}

/// renameat2(RENAME_NOREPLACE) within directory descriptors.
pub(crate) fn rename_noreplace(
    from_dir: &File,
    from: &OsStr,
    to_dir: &File,
    to: &OsStr,
) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let (from, to) = (c_name(from)?, c_name(to)?);
    // SAFETY: valid descriptors and NUL-terminated names.
    let rc = unsafe {
        libc::renameat2(
            from_dir.as_raw_fd(),
            from.as_ptr(),
            to_dir.as_raw_fd(),
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Publish `from` under the first free numbered variant of `name` in `to_dir`
/// (`name`, `name (2)`, ...). Never replaces an existing entry.
pub(crate) fn publish_unique(
    from_dir: &File,
    from: &OsStr,
    to_dir: &File,
    name: &OsStr,
    directory: bool,
) -> io::Result<(OsString, bool)> {
    let mut names = crate::copy_unique_name::Names::new(name, directory, 255)?;
    loop {
        match rename_noreplace(from_dir, from, to_dir, names.name()) {
            Ok(()) => return Ok((names.name().to_os_string(), names.changed())),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => names.advance()?,
            Err(e) => return Err(e),
        }
    }
}

pub(crate) fn open_directory(path: &Path) -> io::Result<File> {
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rows_parse_escaped_names_links_and_kinds() {
        let row =
            parse_row(b"-rw-r--r--  0 1000   1000        1 Nov 10 16:19 d1/sp ace\\\\n\\nl\\377")
                .unwrap();
        assert_eq!(row.kind, Kind::File);
        assert_eq!(row.name, b"d1/sp ace\\n\nl\xff");
        let link =
            parse_row(b"lrwxrwxrwx  0 0      0           0 Dec 31  1969 a/l -> ../b").unwrap();
        assert_eq!(link.kind, Kind::Symlink);
        assert_eq!(link.target.as_deref(), Some(&b"../b"[..]));
        let hard = parse_row(b"hrw-r--r--  0 1 1 0 Nov 10 16:19 d1/hard link to d1/a.txt").unwrap();
        assert_eq!(hard.kind, Kind::Hardlink);
        let dir = parse_row(b"drwxr-xr-x  0 1 1 0 Nov  1 16:19 d1/").unwrap();
        assert_eq!((dir.kind, dir.name), (Kind::Directory, b"d1".to_vec()));
        assert!(parse_row(b"garbage").is_none());
        assert!(parse_row(b"-rw-r--r--  0 1 1 0 Nov 10 16:19 bad\\000nul").is_none());
    }
    #[test]
    fn lexical_checks_reject_absolute_traversal_and_escaping_links() {
        assert_eq!(components(b"/etc/passwd"), Err("ruta absoluta"));
        assert_eq!(components(b"a/../../x"), Err("ruta con «..»"));
        assert_eq!(components(b"./a//b/").unwrap(), vec![&b"a"[..], b"b"]);
        assert!(lexically_confined(1, b"../x"));
        assert!(!lexically_confined(1, b"../../x"));
        assert!(!lexically_confined(0, b"/x"));
    }
    #[test]
    fn collector_tracks_top_level_and_first_problem() {
        let mut c = Collector::new(Format::SevenZip, 10);
        for name in [&b"top/a"[..], b"top/b/c"] {
            c.add(Row {
                kind: Kind::File,
                name: name.to_vec(),
                size: 5,
                mode: "-rw-r--r--".into(),
                target: None,
                problem: None,
            })
            .unwrap();
        }
        assert_eq!(
            c.listing.top,
            Top::Single {
                name: b"top".to_vec(),
                directory: true
            }
        );
        c.add(Row {
            kind: Kind::Symlink,
            name: b"top/l".to_vec(),
            size: 0,
            mode: "lrwxrwxrwx".into(),
            target: Some(b"../../etc".to_vec()),
            problem: None,
        })
        .unwrap();
        assert!(c.listing.require_safe().is_err());
        assert_eq!(c.bytes, 10);
    }
    #[test]
    fn missing_tool_reports_spanish_unavailable_format() {
        let e = program_in(&[], "bsdtar").unwrap_err();
        assert_eq!(e.to_string(), "formato no disponible: falta bsdtar");
    }
    #[test]
    fn encrypted_diagnostics_are_detected() {
        assert!(encrypted_line(b"Enter passphrase:"));
        assert!(encrypted_line(b"RAR encryption support unavailable."));
        assert!(!encrypted_line(b"x d1/a.txt"));
    }
}

//! Network locations through GVfs, the virtual filesystem the desktop
//! already runs (Omarchy ships `gvfs`, `gvfs-smb`, `gvfs-nfs` and the sftp,
//! ftp and dav backends). Files starts no daemon of its own: `gio mount`
//! asks the session's GVfs to connect, and every connected location appears
//! as a plain folder under `$XDG_RUNTIME_DIR/gvfs` (the FUSE bridge), so the
//! listing, previews, copies and other apps keep working on ordinary paths.
//!
//! Credentials are written to `gio`'s standard input, never to its argument
//! list or to a log, and are not stored by Files. `gio` prompts line by line
//! (`User`, `Domain`, `Password`, `Choice`); [`mount`] answers the ones it
//! can and reports the rest so the interface can ask the person.
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

/// URI schemes handed to GVfs. `ssh` is GVfs's alias of `sftp`.
pub const SCHEMES: [&str; 9] = [
    "smb", "sftp", "ssh", "ftp", "ftps", "dav", "davs", "nfs", "afp",
];
/// Longest URI accepted from the location field.
pub const URI_LIMIT: usize = 2048;
/// A connection that takes longer than this is abandoned.
pub const MOUNT_TIMEOUT: Duration = Duration::from_secs(60);
const QUERY_TIMEOUT: Duration = Duration::from_secs(20);
/// Bytes of `gio` output kept for prompts and error messages.
const OUTPUT_LIMIT: usize = 16 * 1024;

/// A connected network location as the FUSE bridge exposes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Share {
    /// `/run/user/1000/gvfs/smb-share:server=nas,share=fotos`.
    pub path: PathBuf,
    /// What `gio mount -u` takes; the FUSE path when the kind is unknown.
    pub uri: String,
    /// Short protocol label: SMB, SFTP, FTP, WebDAV, NFS…
    pub protocol: &'static str,
    /// Sidebar label: `fotos en nas`, `host`.
    pub title: String,
    /// User the location is connected as, when the bridge name says so.
    pub user: Option<String>,
}

impl Share {
    /// Second sidebar line: protocol, then the user (`SFTP · ana`).
    pub fn detail(&self) -> String {
        match &self.user {
            Some(user) => format!("{} · {user}", self.protocol),
            None => self.protocol.to_string(),
        }
    }
}

/// The FUSE bridge directory of this session, when GVfs runs.
pub fn gvfs_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("FILES_TEST_GVFS_DIR") {
        return Some(dir.into());
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let dir = Path::new(&runtime).join("gvfs");
    dir.is_dir().then_some(dir)
}

/// Connected locations, sorted by title. Reading the bridge root is answered
/// by the local `gvfsd-fuse` from memory; it never touches the network.
pub fn scan(dir: &Path) -> Vec<Share> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<Share> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let name = name.to_str()?;
            Some(share(&dir.join(name), name))
        })
        .collect();
    out.sort_by(|a, b| a.title.cmp(&b.title).then(a.path.cmp(&b.path)));
    out
}

/// Interprets a bridge entry name such as `sftp:host=h,port=2222,user=ana`.
pub fn share(path: &Path, name: &str) -> Share {
    let fallback = || Share {
        path: path.to_path_buf(),
        uri: path.to_string_lossy().into_owned(),
        protocol: "Red",
        title: name.to_string(),
        user: None,
    };
    let Some((kind, rest)) = name.split_once(':') else {
        return fallback();
    };
    let mut fields = std::collections::BTreeMap::new();
    for pair in rest.split(',') {
        if let Some((k, v)) = pair.split_once('=') {
            fields.insert(k, unescape(v));
        }
    }
    let get = |k: &str| fields.get(k).map(String::as_str).filter(|v| !v.is_empty());
    let user = get("user").map(|u| format!("{}@", escape(u, false)));
    let port = get("port").map(|p| format!(":{p}"));
    let (user, port) = (user.unwrap_or_default(), port.unwrap_or_default());
    let host = get("host").or(get("server"));
    let Some(host) = host else {
        return fallback();
    };
    let (protocol, uri, title) = match kind {
        "smb-share" => {
            let Some(share) = get("share") else {
                return fallback();
            };
            let domain = get("domain")
                .filter(|_| !user.is_empty())
                .map(|d| format!("{};", escape(d, false)))
                .unwrap_or_default();
            (
                "SMB",
                format!(
                    "smb://{domain}{user}{}{port}/{}",
                    escape(host, false),
                    escape(share, false)
                ),
                format!("{share} en {host}"),
            )
        }
        "smb-server" => (
            "SMB",
            format!("smb://{}{port}/", escape(host, false)),
            host.to_string(),
        ),
        "sftp" => (
            "SFTP",
            format!("sftp://{user}{}{port}/", escape(host, false)),
            host.to_string(),
        ),
        "ftp" | "ftps" => (
            if kind == "ftp" { "FTP" } else { "FTPS" },
            format!("{kind}://{user}{}{port}/", escape(host, false)),
            host.to_string(),
        ),
        "dav" => {
            let scheme = if get("ssl") == Some("true") {
                "davs"
            } else {
                "dav"
            };
            let prefix = get("prefix").unwrap_or("/");
            (
                "WebDAV",
                format!(
                    "{scheme}://{user}{}{port}{}",
                    escape(host, false),
                    escape(prefix, true)
                ),
                format!("{}{}", host, prefix.trim_end_matches('/')),
            )
        }
        "nfs" => {
            let prefix = get("prefix").unwrap_or("/");
            (
                "NFS",
                format!(
                    "nfs://{}{port}{}",
                    escape(host, false),
                    escape(prefix, true)
                ),
                format!("{host}:{prefix}"),
            )
        }
        "afp-volume" => {
            let Some(volume) = get("volume") else {
                return fallback();
            };
            (
                "AFP",
                format!(
                    "afp://{user}{}{port}/{}",
                    escape(host, false),
                    escape(volume, false)
                ),
                format!("{volume} en {host}"),
            )
        }
        _ => return fallback(),
    };
    Share {
        path: path.to_path_buf(),
        uri,
        protocol,
        title,
        user: get("user").map(str::to_string),
    }
}

fn unescape(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(Some(h)), Some(Some(l))) = (
                bytes.get(i + 1).map(|b| hex(*b)),
                bytes.get(i + 2).map(|b| hex(*b)),
            )
        {
            out.push((h * 16 + l) as u8);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn escape(value: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) || keep_slash && b == b'/' {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// True when `text` names a network location GVfs can mount.
pub fn is_network_uri(text: &str) -> bool {
    scheme(text).is_some()
}

fn scheme(text: &str) -> Option<&'static str> {
    let (scheme, _) = text.split_once("://")?;
    SCHEMES
        .iter()
        .copied()
        .find(|s| s.eq_ignore_ascii_case(scheme))
}

/// A checked location: scheme, optional user and the URI without any
/// password, plus the password when one was typed inline (`ftp://u:pw@h`).
#[derive(Clone, PartialEq, Eq)]
pub struct Location {
    pub uri: String,
    pub user: Option<String>,
    pub inline_password: Option<String>,
}

impl std::fmt::Debug for Location {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Location")
            .field("uri", &self.uri)
            .field("user", &self.user)
            .field(
                "inline_password",
                &self.inline_password.as_ref().map(|_| "…"),
            )
            .finish()
    }
}

/// Validates a typed network URI: known scheme, a host, one line, no spaces.
pub fn parse(text: &str) -> Result<Location, &'static str> {
    let text = text.trim();
    let Some(scheme) = scheme(text) else {
        return Err("Usa smb://, sftp://, ftp://, ftps://, dav://, davs://, nfs:// o afp://");
    };
    if text.len() > URI_LIMIT {
        return Err("La dirección es demasiado larga");
    }
    if text.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("La dirección no puede tener espacios ni controles: usa %20");
    }
    let rest = &text[text.find("://").unwrap_or(0) + 3..];
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (userinfo, host) = match authority.rfind('@') {
        Some(i) => (Some(&authority[..i]), &authority[i + 1..]),
        None => (None, authority),
    };
    if host.is_empty() || host.starts_with(':') {
        return Err("Falta el servidor: smb://servidor/recurso");
    }
    let (user, password) = match userinfo {
        Some(info) => match info.split_once(':') {
            Some((u, p)) => (u, Some(unescape(p))),
            None => (info, None),
        },
        None => ("", None),
    };
    let user = (!user.is_empty()).then(|| user.to_string());
    let uri = format!(
        "{scheme}://{}{host}{path}",
        user.as_ref().map(|u| format!("{u}@")).unwrap_or_default()
    );
    Ok(Location {
        uri,
        user: user.map(|u| unescape(u.rsplit(';').next().unwrap_or(&u))),
        inline_password: password,
    })
}

/// How a connection attempt ended when it did not fail outright.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Connected (or it already was).
    Mounted,
    /// The server wants a password; `rejected` when the one given was wrong.
    NeedsPassword { rejected: bool },
    /// The server wants a user name and the URI has none.
    NeedsUser,
    /// GVfs asks the person to choose, e.g. to trust an unknown host key.
    /// Files never answers it on their behalf.
    Question(String),
}

fn gio() -> std::ffi::OsString {
    std::env::var_os("FILES_TEST_GIO").unwrap_or_else(|| "gio".into())
}

fn command(program: &std::ffi::OsStr, args: &[&str]) -> Command {
    let mut c = Command::new(program);
    c.args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    c
}

/// Reads a pipe on its own thread, chunk by chunk, up to `OUTPUT_LIMIT`.
fn pump(mut pipe: impl Read + Send + 'static) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = [0u8; 1024];
        let mut total = 0;
        while let Ok(n) = pipe.read(&mut buf) {
            if n == 0 || total >= OUTPUT_LIMIT {
                break;
            }
            total += n;
            if tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    rx
}

fn stop(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn drain(rx: &mpsc::Receiver<Vec<u8>>, out: &mut Vec<u8>) {
    while let Ok(chunk) = rx.recv_timeout(Duration::from_millis(200)) {
        out.extend_from_slice(&chunk);
        if out.len() > OUTPUT_LIMIT {
            break;
        }
    }
}

/// `gio`'s error as one readable line, without its program prefix.
fn reason(stderr: &[u8], status: std::process::ExitStatus) -> String {
    let text = String::from_utf8_lossy(stderr);
    let line = text
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or("")
        .trim_start_matches("gio: ");
    // `gio: smb://nas/x: Failed to mount…` → drop the echoed URI.
    let line = match line.split_once(": ") {
        Some((head, tail)) if head.contains("://") || head.starts_with('/') => tail,
        _ => line,
    };
    let line: String = line.chars().take(300).collect();
    if line.is_empty() {
        format!("gio falló ({status})")
    } else {
        line
    }
}

/// Connects `uri` through GVfs, answering `gio`'s prompts with `user`,
/// `password` and the server's suggested domain. A URI without a user that
/// the server wants a name for is retried once as anonymous (guest/FTP).
pub fn mount(location: &Location, password: Option<&str>) -> Result<Outcome, String> {
    mount_with(&gio(), location, password)
}

fn mount_with(
    program: &std::ffi::OsStr,
    location: &Location,
    password: Option<&str>,
) -> Result<Outcome, String> {
    match mount_once(program, location, password, false)? {
        Outcome::NeedsUser if location.user.is_none() => {
            match mount_once(program, location, password, true)? {
                Outcome::NeedsPassword { .. } => Ok(Outcome::NeedsUser),
                other => Ok(other),
            }
        }
        other => Ok(other),
    }
}

fn mount_once(
    program: &std::ffi::OsStr,
    location: &Location,
    password: Option<&str>,
    anonymous: bool,
) -> Result<Outcome, String> {
    let mut args = vec!["mount"];
    if anonymous {
        args.push("-a");
    }
    args.push(&location.uri);
    let mut child = command(program, &args)
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("gio no disponible: {e}"))?;
    let mut stdin = child.stdin.take();
    let stdout = pump(child.stdout.take().expect("piped stdout"));
    let stderr = pump(child.stderr.take().expect("piped stderr"));
    let deadline = Instant::now() + MOUNT_TIMEOUT;
    let mut pending = String::new();
    let mut passwords = 0;
    let answer = |stdin: &mut Option<std::process::ChildStdin>, line: &str| -> bool {
        stdin
            .as_mut()
            .is_some_and(|s| s.write_all(line.as_bytes()).and_then(|_| s.flush()).is_ok())
    };
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            stop(&mut child);
            return Err("El servidor no respondió a tiempo".into());
        }
        match stdout.recv_timeout(left.min(Duration::from_millis(250))) {
            Ok(chunk) => {
                pending.push_str(&String::from_utf8_lossy(&chunk));
                if pending.len() > OUTPUT_LIMIT {
                    stop(&mut child);
                    return Err("Respuesta inesperada de gio".into());
                }
                if !pending.ends_with(": ") {
                    continue;
                }
                let prompt = pending.rsplit('\n').next().unwrap_or("").trim_start();
                let has_default = prompt.contains('[');
                if prompt.starts_with("User") {
                    let reply = match (&location.user, has_default) {
                        (Some(user), _) => format!("{user}\n"),
                        (None, true) => "\n".into(),
                        (None, false) => {
                            stop(&mut child);
                            return Ok(Outcome::NeedsUser);
                        }
                    };
                    if !answer(&mut stdin, &reply) {
                        break;
                    }
                } else if prompt.starts_with("Domain") {
                    if !answer(&mut stdin, "\n") {
                        break;
                    }
                } else if prompt.starts_with("Password") {
                    passwords += 1;
                    match password {
                        Some(pw) if passwords == 1 => {
                            if !answer(&mut stdin, &format!("{pw}\n")) {
                                break;
                            }
                        }
                        _ => {
                            stop(&mut child);
                            return Ok(Outcome::NeedsPassword {
                                rejected: password.is_some(),
                            });
                        }
                    }
                } else if prompt.starts_with("Choice") {
                    stop(&mut child);
                    let question = pending[..pending.len() - prompt.len()]
                        .lines()
                        .map(str::trim)
                        .filter(|l| !l.is_empty() && !l.starts_with('['))
                        .collect::<Vec<_>>()
                        .join(" ");
                    return Ok(Outcome::Question(question.chars().take(400).collect()));
                } else {
                    continue;
                }
                pending.clear();
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if child.try_wait().ok().flatten().is_some() {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    drop(stdin);
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if Instant::now() >= deadline {
            stop(&mut child);
            return Err("El servidor no respondió a tiempo".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut err = Vec::new();
    drain(&stderr, &mut err);
    if status.success() || String::from_utf8_lossy(&err).contains("already mounted") {
        Ok(Outcome::Mounted)
    } else {
        Err(reason(&err, status))
    }
}

/// Runs a short `gio` query with a deadline; stdout on success.
fn query(args: &[&str]) -> Result<String, String> {
    let mut child = command(&gio(), args)
        .spawn()
        .map_err(|e| format!("gio no disponible: {e}"))?;
    let stdout = pump(child.stdout.take().expect("piped stdout"));
    let stderr = pump(child.stderr.take().expect("piped stderr"));
    let deadline = Instant::now() + QUERY_TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if Instant::now() >= deadline {
            stop(&mut child);
            return Err("gio no respondió a tiempo".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let (mut out, mut err) = (Vec::new(), Vec::new());
    drain(&stdout, &mut out);
    drain(&stderr, &mut err);
    if status.success() {
        Ok(String::from_utf8_lossy(&out).into_owned())
    } else {
        Err(reason(&err, status))
    }
}

/// The folder a mounted URI is reachable at through the FUSE bridge.
pub fn local_path(uri: &str) -> Result<PathBuf, String> {
    let text = query(&["info", "-a", "standard::type", uri])?;
    text.lines()
        .find_map(|l| l.strip_prefix("local path: "))
        .map(|p| PathBuf::from(p.trim_end()))
        .ok_or_else(|| "GVfs no expone esta ubicación como carpeta local (falta gvfs-fuse)".into())
}

/// Disconnects a location (its URI or its FUSE path). The FUSE bridge
/// releases closed files a moment after `close()`, so a "busy" answer right
/// after reading is retried a few times before it is reported.
pub fn unmount(target: &str) -> Result<(), String> {
    let mut tries = 0;
    loop {
        match query(&["mount", "-u", target]) {
            Err(e) if e.contains("busy") && tries < 5 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(300 * tries));
            }
            Err(e) if e.contains("busy") => {
                return Err(
                    "La ubicación sigue en uso: cierra los archivos abiertos en ella".into(),
                );
            }
            other => return other.map(|_| ()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str) -> Share {
        share(&Path::new("/run/user/1/gvfs").join(name), name)
    }

    #[test]
    fn bridge_names_become_uris_and_titles() {
        let s = named("smb-share:domain=WG,server=nas,share=fotos%20viejas,user=ana");
        assert_eq!(s.uri, "smb://WG;ana@nas/fotos%20viejas");
        assert_eq!(
            (s.protocol, s.title.as_str()),
            ("SMB", "fotos viejas en nas")
        );
        let s = named("sftp:host=example.org,port=2222,user=bob");
        assert_eq!(s.uri, "sftp://bob@example.org:2222/");
        assert_eq!(
            (s.title.as_str(), s.detail()),
            ("example.org", "SFTP · bob".into())
        );
        assert_eq!(
            named("ftp:host=ftp.example.org").uri,
            "ftp://ftp.example.org/"
        );
        let s = named("dav:host=cloud,prefix=%2Fremote.php%2Fdav,ssl=true,user=u");
        assert_eq!(s.uri, "davs://u@cloud/remote.php/dav");
        assert_eq!(s.title, "cloud/remote.php/dav");
        assert_eq!(
            named("nfs:host=srv,prefix=%2Fexport").uri,
            "nfs://srv/export"
        );
        let odd = named("mtp:something");
        assert_eq!(odd.uri, "/run/user/1/gvfs/mtp:something");
        assert_eq!(odd.protocol, "Red");
    }

    #[test]
    fn typed_uris_are_checked_and_inline_passwords_split_off() {
        assert!(is_network_uri("SMB://nas/x"));
        assert!(!is_network_uri("file:///tmp"));
        assert!(!is_network_uri("http://example.org"));
        let l = parse("ftp://ana:s%3Ecret@host/pub").unwrap();
        assert_eq!(l.uri, "ftp://ana@host/pub");
        assert_eq!(l.user.as_deref(), Some("ana"));
        assert_eq!(l.inline_password.as_deref(), Some("s>cret"));
        assert!(!format!("{l:?}").contains("cret"));
        let l = parse("smb://WG;ana@nas").unwrap();
        assert_eq!(
            (l.uri.as_str(), l.user.as_deref()),
            ("smb://WG;ana@nas/", Some("ana"))
        );
        assert!(parse("smb:///share").is_err());
        assert!(parse("sftp://host/a b").is_err());
        assert!(parse("gopher://host/").is_err());
    }

    /// A stand-in for `gio mount` that prompts like the real one. The URI's
    /// host picks the scenario; it writes nothing outside its own output.
    fn fake_gio() -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "files-network-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir(&dir).unwrap();
        let script = dir.join("gio");
        std::fs::write(
            &script,
            r#"#!/bin/sh
anon=no
[ "$2" = "-a" ] && anon=yes
for a; do uri="$a"; done
case "$uri" in
*good*)
  printf 'Password required for share x on good\n'
  printf 'User [ana]: '; read u
  printf 'Domain [WG]: '; read d
  printf 'Password: '; read p
  [ "$u" = ana ] && [ -z "$d" ] && [ "$p" = "good pw" ] && exit 0
  printf 'Password: '; read p; exit 2 ;;
*guest*)
  [ $anon = yes ] && exit 0
  printf 'User: '; read u; exit 2 ;;
*named*)
  [ $anon = yes ] && { printf 'Password: '; read p; exit 2; }
  printf 'User: '; read u; exit 2 ;;
*hostkey*)
  printf "Can't verify the identity of \"hostkey\".\n[1] Log In Anyway\n[2] Cancel Login\n"
  printf 'Choice: '; read c; exit 2 ;;
*already*) echo "gio: $uri: Location is already mounted" >&2; exit 2 ;;
*) echo "gio: $uri: Connection refused" >&2; exit 2 ;;
esac
"#,
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        (dir, script)
    }

    #[test]
    fn mount_answers_prompts_and_reports_what_the_person_must_decide() {
        let (_dir, gio) = fake_gio();
        let gio = gio.as_os_str();
        let at = |uri: &str| parse(uri).unwrap();
        assert_eq!(
            mount_with(gio, &at("smb://ana@good/x"), Some("good pw")),
            Ok(Outcome::Mounted)
        );
        assert_eq!(
            mount_with(gio, &at("smb://ana@good/x"), None),
            Ok(Outcome::NeedsPassword { rejected: false })
        );
        assert_eq!(
            mount_with(gio, &at("smb://ana@good/x"), Some("bad")),
            Ok(Outcome::NeedsPassword { rejected: true })
        );
        assert_eq!(
            mount_with(gio, &at("ftp://guest/"), None),
            Ok(Outcome::Mounted)
        );
        assert_eq!(
            mount_with(gio, &at("ftp://named/"), None),
            Ok(Outcome::NeedsUser)
        );
        match mount_with(gio, &at("sftp://hostkey/"), None) {
            Ok(Outcome::Question(q)) => {
                assert!(q.contains("verify the identity"), "{q}");
                assert!(!q.contains("Log In Anyway"), "{q}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            mount_with(gio, &at("smb://already/x"), None),
            Ok(Outcome::Mounted)
        );
        assert_eq!(
            mount_with(gio, &at("smb://down/x"), None),
            Err("Connection refused".into())
        );
    }

    /// Manual check against the session's real GVfs and a loopback server:
    /// `FILES_NETWORK_E2E_URI=ftp://ana@127.0.0.1:2121/ FILES_NETWORK_E2E_PASSWORD=…
    /// FILES_NETWORK_E2E_FILE=docs/nota.txt cargo test -p files-core real_gvfs -- --ignored`.
    /// It mounts, reads one file through the FUSE bridge and unmounts.
    #[test]
    #[ignore]
    fn real_gvfs_connects_reads_and_disconnects() {
        let var = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k} is required"));
        let location = parse(&var("FILES_NETWORK_E2E_URI")).unwrap();
        let password = var("FILES_NETWORK_E2E_PASSWORD");
        assert_eq!(
            mount(&location, None),
            Ok(Outcome::NeedsPassword { rejected: false })
        );
        assert_eq!(
            mount(&location, Some("wrong")),
            Ok(Outcome::NeedsPassword { rejected: true })
        );
        assert_eq!(mount(&location, Some(&password)), Ok(Outcome::Mounted));
        let root = local_path(&location.uri).unwrap();
        let listed = scan(&gvfs_dir().unwrap());
        let share = listed
            .iter()
            .find(|s| root.starts_with(&s.path))
            .expect("listed");
        eprintln!("FILES_NETWORK_E2E share={share:?} root={}", root.display());
        let text = std::fs::read_to_string(root.join(var("FILES_NETWORK_E2E_FILE"))).unwrap();
        eprintln!("FILES_NETWORK_E2E read={text:?}");
        assert_eq!(
            mount(&location, None),
            Ok(Outcome::Mounted),
            "already mounted"
        );
        unmount(&share.uri).unwrap();
        assert!(
            !scan(&gvfs_dir().unwrap())
                .iter()
                .any(|s| s.path == share.path)
        );
    }

    #[test]
    fn gio_errors_read_as_one_line() {
        use std::os::unix::process::ExitStatusExt;
        let status = std::process::ExitStatus::from_raw(256);
        assert_eq!(
            reason(
                b"gio: smb://nas/x: Failed to mount Windows share: Connection refused\n",
                status
            ),
            "Failed to mount Windows share: Connection refused"
        );
        assert_eq!(reason(b"", status), format!("gio falló ({status})"));
    }
}

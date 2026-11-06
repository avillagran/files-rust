//! Read-only file-manager preflight, isolated from the UI and D-Bus worker.
use files_core::file_manager::{Method, Request};
use std::{
    io::{self, Read, Write},
    path::PathBuf,
    time::{Duration, Instant},
};
const MAX_REQUEST: usize = 256 * 1024;

fn decode(bytes: &[u8]) -> Result<Request, String> {
    let bytes = bytes
        .strip_suffix(b"\n")
        .ok_or("Preflight requires one newline-terminated request")?;
    if bytes.contains(&b'\n') {
        return Err("Preflight accepts exactly one request".into());
    }
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| "Invalid preflight JSON")?;
    let object = value
        .as_object()
        .ok_or("Preflight request must be an object")?;
    if object.len() != 2 || !object.contains_key("method") || !object.contains_key("paths") {
        return Err("Unexpected preflight request fields".into());
    }
    let method = match value.get("method").and_then(|v| v.as_str()) {
        Some("ShowFolders") => Method::ShowFolders,
        Some("ShowItems") => Method::ShowItems,
        Some("ShowItemProperties") => Method::ShowItemProperties,
        _ => return Err("Unknown preflight method".into()),
    };
    let paths = value
        .get("paths")
        .and_then(|v| v.as_array())
        .ok_or("Preflight paths must be an array")?;
    if paths.is_empty() || paths.len() > files_core::file_manager::MAX_URIS {
        return Err("Preflight path count exceeds its limit".into());
    }
    let mut decoded = Vec::with_capacity(paths.len());
    for path in paths {
        let text = path
            .as_str()
            .ok_or("Preflight paths must be hexadecimal strings")?;
        if text.len() > 8192 || text.len() % 2 != 0 {
            return Err("Invalid preflight path length".into());
        }
        let hex = |b: u8| match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        };
        let raw: Vec<u8> = text
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                hex(pair[0])
                    .zip(hex(pair[1]))
                    .map(|(a, b)| a * 16 + b)
                    .ok_or("Invalid path hexadecimal")
            })
            .collect::<Result<_, _>>()?;
        #[cfg(unix)]
        let path = {
            use std::os::unix::ffi::OsStringExt;
            PathBuf::from(std::ffi::OsString::from_vec(raw))
        };
        #[cfg(not(unix))]
        let path = PathBuf::from(String::from_utf8(raw).map_err(|_| "Invalid path UTF-8")?);
        decoded.push(path);
    }
    Request::from_paths(method, decoded)
}
fn read_stdin() -> Result<Vec<u8>, String> {
    let mut input = io::stdin().lock();
    let mut bytes = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("Preflight request deadline exceeded".into());
        }
        #[cfg(target_os = "linux")]
        {
            let mut fd = libc::pollfd {
                fd: 0,
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe {
                libc::poll(
                    &mut fd,
                    1,
                    remaining.as_millis().min(i32::MAX as u128) as i32,
                )
            };
            if ready < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e.to_string());
            }
            if ready == 0 {
                return Err("Preflight request deadline exceeded".into());
            }
        }
        let mut chunk = [0u8; 4096];
        let n = input.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(bytes);
        }
        if n > MAX_REQUEST.saturating_sub(bytes.len()) {
            return Err("Preflight request exceeds 256 KiB".into());
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
}
pub fn run() -> io::Result<()> {
    let result = read_stdin()
        .and_then(|bytes| decode(&bytes))
        .and_then(|request| request.preflight());
    let value = match result {
        Ok(()) => serde_json::json!({"ok":true}),
        Err(mut error) => {
            if error.len() > 2048 {
                let mut end = 2048;
                while !error.is_char_boundary(end) {
                    end -= 1;
                }
                error.truncate(end);
            }
            serde_json::json!({"ok":false,"error":error})
        }
    };
    let mut out = io::stdout().lock();
    serde_json::to_writer(&mut out, &value)?;
    out.write_all(b"\n")?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preflight_decoder_rejects_extra_records_fields_and_bad_paths() {
        for bytes in [
            b"{\"method\":\"ShowItems\",\"paths\":[\"2f61\"]}".as_slice(),
            b"{\"method\":\"ShowItems\",\"paths\":[\"2f61\"]}\n{}\n",
            b"{\"method\":\"ShowItems\",\"paths\":[\"2f61\"],\"extra\":1}\n",
            b"{\"method\":\"ShowItems\",\"paths\":[\"gg\"]}\n",
            b"{\"method\":\"ShowItems\",\"paths\":[\"61\"]}\n",
        ] {
            assert!(decode(bytes).is_err());
        }
        #[cfg(unix)]
        {
            let request =
                decode(b"{\"method\":\"ShowItems\",\"paths\":[\"2f7261772dff\"]}\n").unwrap();
            assert_eq!(
                request.paths[0].as_os_str().as_encoded_bytes(),
                b"/raw-\xff"
            );
        }
    }
}

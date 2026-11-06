//! Isolate read-only filesystem preflight in the bounded, cancellable helper.
use files_core::file_manager::{Method, Request};
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread::JoinHandle,
    time::Duration,
};

const MAX_REQUEST: usize = 256 * 1024;
const MAX_RESPONSE: usize = 4096;

fn encode(request: &Request) -> Result<Vec<u8>, String> {
    if request.paths.is_empty() || request.paths.len() > files_core::file_manager::MAX_URIS {
        return Err("Invalid file-manager preflight path count".into());
    }
    let mut total = 0usize;
    for path in &request.paths {
        let bytes = path.as_os_str().as_encoded_bytes();
        if !path.is_absolute() || bytes.len() > 4096 || bytes.contains(&0) {
            return Err("Invalid file-manager preflight path".into());
        }
        total = total
            .checked_add(bytes.len())
            .filter(|&n| n <= files_core::file_manager::MAX_BYTES)
            .ok_or("File-manager preflight path budget exceeded")?;
    }
    let method = match request.method {
        Method::ShowFolders => "ShowFolders",
        Method::ShowItems => "ShowItems",
        Method::ShowItemProperties => "ShowItemProperties",
    };
    let paths: Vec<String> = request
        .paths
        .iter()
        .map(|path| {
            path.as_os_str()
                .as_encoded_bytes()
                .iter()
                .flat_map(|&b| {
                    const HEX: &[u8; 16] = b"0123456789abcdef";
                    [
                        HEX[(b >> 4) as usize] as char,
                        HEX[(b & 15) as usize] as char,
                    ]
                })
                .collect()
        })
        .collect();
    let mut bytes = serde_json::to_vec(&serde_json::json!({"method":method,"paths":paths}))
        .map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    if bytes.len() > MAX_REQUEST {
        return Err("File-manager preflight exceeds its 256 KiB wire limit".into());
    }
    Ok(bytes)
}
fn response(bytes: &[u8]) -> Result<(), String> {
    let bytes = bytes
        .strip_suffix(b"\n")
        .ok_or("Incomplete file-manager preflight acknowledgement")?;
    if bytes.contains(&b'\n') {
        return Err("Multiple file-manager preflight records".into());
    }
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|_| "Malformed file-manager preflight acknowledgement")?;
    let object = value
        .as_object()
        .ok_or("Invalid file-manager preflight acknowledgement")?;
    match value.get("ok").and_then(|v| v.as_bool()) {
        Some(true) if object.len() == 1 => Ok(()),
        Some(false) if object.len() == 2 => {
            let error = value
                .get("error")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty() && s.len() <= 2048)
                .ok_or("Invalid preflight error")?;
            Err(error.into())
        }
        _ => Err("Invalid file-manager preflight acknowledgement".into()),
    }
}
fn check_wire(request: &[u8], stop: &AtomicBool) -> Result<(), String> {
    let executable = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .with_file_name("files-helper");
    let mut bytes = Vec::with_capacity(256);
    crate::space_map_job::run_helper_stream(
        request,
        stop,
        executable,
        Duration::from_secs(3),
        "file_manager_preflight",
        |chunk| {
            if chunk.len() > MAX_RESPONSE.saturating_sub(bytes.len()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "File-manager preflight acknowledgement exceeds 4 KiB",
                ));
            }
            bytes.extend_from_slice(chunk);
            Ok(())
        },
    )
    .map_err(|e| e.to_string())?;
    if stop.load(Ordering::Acquire) {
        return Err("File-manager preflight cancelled".into());
    }
    response(&bytes)
}
pub struct Job {
    cancel: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
    result: Receiver<Result<(), String>>,
    worker: Option<JoinHandle<()>>,
}
impl Job {
    pub fn start(request: &Request) -> Result<Self, String> {
        let bytes = encode(request)?;
        let cancel = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let (tx, result) = mpsc::sync_channel(1);
        let (stop, done) = (cancel.clone(), finished.clone());
        let worker = std::thread::Builder::new()
            .name("files-filemanager-preflight".into())
            .spawn(move || {
                let value = check_wire(&bytes, &stop);
                let _ = tx.send(value);
                done.store(true, Ordering::Release);
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            cancel,
            finished,
            result,
            worker: Some(worker),
        })
    }
    pub fn poll(&mut self) -> Option<Result<(), String>> {
        self.result.try_recv().ok()
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
    pub fn finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }
    pub fn drained(&self) -> bool {
        self.finished()
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel();
        if self.worker.as_ref().is_some_and(|w| w.is_finished()) {
            let _ = self.worker.take().expect("finished worker").join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preflight_ack_requires_one_bounded_complete_record() {
        assert!(response(b"{\"ok\":true}\n").is_ok());
        for bytes in [
            b"{\"ok\":true}".as_slice(),
            b"{\"ok\":true}\n{}\n",
            b"{\"ok\":true,\"extra\":1}\n",
            b"{\"ok\":false}\n",
        ] {
            assert!(response(bytes).is_err());
        }
        assert_eq!(
            response(b"{\"ok\":false,\"error\":\"missing fixture\"}\n"),
            Err("missing fixture".into())
        );
    }
}

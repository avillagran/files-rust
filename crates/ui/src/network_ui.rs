//! Sidebar network locations through the session's GVfs: a watcher thread
//! lists the connected locations, and connect / disconnect run one at a time
//! in their own thread. A password the server asks for is typed in the
//! location field (masked) and handed to `gio` once; Files never stores it.
use crate::model::Model;
use files_core::network::{Location, Outcome, Share};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    time::Duration,
};

/// The bridge root is re-read this often, and at once after an own job.
/// Reading it is served from `gvfsd-fuse`'s memory and never reaches a server.
const RESCAN: Duration = Duration::from_secs(10);

enum Done {
    Connected(PathBuf),
    Ask(Location, Outcome),
    Disconnected,
    Failed(String),
}

#[derive(Default)]
pub struct State {
    pub list: Vec<Share>,
    updates: Option<Receiver<Vec<Share>>>,
    poke: Option<Sender<()>>,
    /// What is running (for the message and the busy row) and its result.
    job: Option<(String, Receiver<Done>)>,
    /// The location waiting for a password typed in the location field.
    pub prompt: Option<Prompt>,
}

#[derive(Debug)]
pub struct Prompt {
    pub location: Location,
}

impl State {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }
}

/// Text the location field starts with for "Connect to server".
pub const CONNECT_TEMPLATE: &str = "smb://";

impl Model {
    /// Starts the watcher (once).
    pub fn start_network(&mut self) {
        if self.network.updates.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let (poke, poked) = mpsc::channel::<()>();
        let spawned = std::thread::Builder::new()
            .name("files-network".into())
            .spawn(move || {
                let mut last = None;
                loop {
                    let list = files_core::network::gvfs_dir()
                        .map(|d| files_core::network::scan(&d))
                        .unwrap_or_default();
                    if last.as_ref() != Some(&list) {
                        if tx.send(list.clone()).is_err() {
                            return;
                        }
                        last = Some(list);
                    }
                    match poked.recv_timeout(RESCAN) {
                        Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                }
            });
        if spawned.is_ok() {
            self.network.updates = Some(rx);
            self.network.poke = Some(poke);
        }
    }

    /// "Connect to server…": the location field, ready for a URI.
    pub fn connect_server_prompt(&mut self) {
        self.begin_path_edit();
        if self.editing {
            self.edit = CONNECT_TEMPLATE.into();
            self.replace_edit = false;
            self.path_edit = crate::path_edit::State::default();
            self.path_edit
                .cursor
                .motion(&self.edit, crate::line_edit::Motion::End, false);
            self.operation_message = Some(
                "Escribe smb://, sftp://, ftp://, dav:// o nfs:// y el servidor · Enter conecta"
                    .into(),
            );
        }
    }

    /// Connects a typed network URI (from the location field).
    pub fn connect_network(&mut self, text: &str) {
        match files_core::network::parse(text) {
            Ok(mut location) => {
                let password = location.inline_password.take();
                self.start_connect(location, password);
            }
            Err(e) => self.error = format!("Ruta: {e}"),
        }
    }

    fn start_connect(&mut self, location: Location, password: Option<String>) {
        if self.network.busy() {
            self.operation_message = Some("Espera la conexión de red en curso".into());
            return;
        }
        let (tx, rx) = mpsc::sync_channel(1);
        let uri = location.uri.clone();
        let spawned = std::thread::Builder::new()
            .name("files-gio-mount".into())
            .spawn(move || {
                let done = match files_core::network::mount(&location, password.as_deref()) {
                    Ok(Outcome::Mounted) => match files_core::network::local_path(&location.uri) {
                        Ok(path) => Done::Connected(path),
                        Err(e) => Done::Failed(e),
                    },
                    Ok(other) => Done::Ask(location, other),
                    Err(e) => Done::Failed(e),
                };
                let _ = tx.send(done);
            });
        if spawned.is_err() {
            self.error = "No se pudo iniciar la conexión".into();
            return;
        }
        self.operation_message = Some(format!("Conectando a {uri}…"));
        self.network.job = Some((uri, rx));
    }

    /// Disconnects the share at `path`; leaves it first when it is open.
    pub fn disconnect_network(&mut self, path: PathBuf) {
        let Some(share) = self.network.list.iter().find(|s| s.path == path).cloned() else {
            return;
        };
        if self.network.busy() {
            self.operation_message = Some("Espera la conexión de red en curso".into());
            return;
        }
        if self.path.starts_with(&share.path) {
            self.navigate(files_core::theme::home());
        }
        let (tx, rx) = mpsc::sync_channel(1);
        let target = share.uri.clone();
        let spawned = std::thread::Builder::new()
            .name("files-gio-unmount".into())
            .spawn(move || {
                let _ = tx.send(match files_core::network::unmount(&target) {
                    Ok(()) => Done::Disconnected,
                    Err(e) => Done::Failed(e),
                });
            });
        if spawned.is_ok() {
            self.operation_message = Some(format!("Desconectando {}…", share.title));
            self.network.job = Some((share.title, rx));
        }
    }

    /// The password typed in the masked location field for the pending prompt.
    pub(crate) fn submit_network_password(&mut self) {
        let Some(prompt) = self.network.prompt.take() else {
            return;
        };
        let password = std::mem::take(&mut self.edit);
        self.end_path_edit();
        self.start_connect(prompt.location, Some(password));
    }

    fn ask_password(&mut self, location: Location, rejected: bool) {
        self.begin_path_edit();
        if !self.editing {
            return;
        }
        self.edit.clear();
        self.replace_edit = false;
        self.path_edit = crate::path_edit::State::default();
        self.operation_message = Some(format!(
            "{} {} · Enter conecta, Esc cancela",
            if rejected {
                "Contraseña rechazada; vuelve a escribirla para"
            } else {
                "Contraseña para"
            },
            location.uri
        ));
        self.network.prompt = Some(Prompt { location });
    }

    pub fn pump_network(&mut self) -> bool {
        let mut dirty = false;
        if let Some(rx) = &self.network.updates {
            let mut latest = None;
            while let Ok(list) = rx.try_recv() {
                latest = Some(list);
            }
            if let Some(list) = latest
                && list != self.network.list
            {
                self.network.list = list;
                dirty = true;
            }
        }
        let Some((label, rx)) = &self.network.job else {
            return dirty;
        };
        let Ok(done) = rx.try_recv() else {
            return dirty;
        };
        let label = label.clone();
        self.network.job = None;
        if let Some(poke) = &self.network.poke {
            let _ = poke.send(());
        }
        let text = match done {
            Done::Connected(path) => {
                self.navigate(path);
                format!("Conectado a {label}")
            }
            Done::Disconnected => format!("{label} desconectado"),
            Done::Ask(location, Outcome::NeedsPassword { rejected }) => {
                eprintln!("FILES_NETWORK_DONE uri={label} result=password rejected={rejected}");
                self.ask_password(location, rejected);
                return true;
            }
            Done::Ask(_, Outcome::NeedsUser) => {
                format!("{label}: el servidor pide usuario; escribe usuario@servidor")
            }
            Done::Ask(_, Outcome::Question(q)) => format!(
                "{label}: {q} · Files no acepta servidores sin verificar; conéctate una vez con ssh o gio para confiar en él"
            ),
            Done::Ask(_, Outcome::Mounted) => format!("Conectado a {label}"),
            Done::Failed(e) => format!("{label}: {e}"),
        };
        eprintln!("FILES_NETWORK_DONE uri={label} result={text}");
        self.operation_message = Some(text);
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::model::Model;
    use fltk::enums::Key;

    #[test]
    fn password_prompt_masks_refuses_copy_and_forgets_on_escape() {
        let mut m = Model::new("/fixture-network".into());
        m.loaded = true;
        let location = files_core::network::parse("smb://ana@nas/x").unwrap();
        m.ask_password(location, false);
        assert!(m.editing && m.network.prompt.is_some());
        m.path_input("s3cret").unwrap();
        assert!(matches!(
            m.path_key(Key::from_char('c'), true, false, false),
            crate::path_edit::KeyResult::Handled
        ));
        assert!(matches!(
            m.path_key(Key::from_char('x'), true, false, false),
            crate::path_edit::KeyResult::Handled
        ));
        assert_eq!(m.edit, "s3cret");
        m.path_key(Key::Escape, false, false, false);
        assert!(!m.editing && m.network.prompt.is_none());
        assert!(m.edit.is_empty());
    }

    #[test]
    fn location_field_routes_network_uris_and_keeps_bad_ones_for_fixing() {
        let mut m = Model::new("/fixture-network".into());
        m.loaded = true;
        m.connect_server_prompt();
        assert_eq!(m.edit, super::CONNECT_TEMPLATE);
        m.path_input("/share").unwrap();
        m.commit_path_edit();
        assert!(m.editing, "a URI without a server stays in the field");
        assert!(m.error.contains("Falta el servidor"), "{}", m.error);
        assert!(m.network.prompt.is_none() && !m.network.busy());
    }
}

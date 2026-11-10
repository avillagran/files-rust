//! Starred items in the window: the list read once at start, toggled with
//! [`Action::StarToggle`](crate::model::Action::StarToggle), written by
//! `files_core::starred` on short-lived threads. The UI only keeps the set
//! for the row emblem and the menus; the newest finished write wins.
use crate::model::Model;
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::mpsc::{self, Receiver},
};

type Reply = Receiver<Result<Vec<PathBuf>, String>>;

pub struct State {
    /// Folder holding `files/starred` (injectable for tests).
    pub data_home: PathBuf,
    pub list: Vec<PathBuf>,
    pub set: HashSet<PathBuf>,
    load: Option<Reply>,
    writes: Vec<(u64, Reply)>,
    next: u64,
    applied: u64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            data_home: files_core::apps::data_home(),
            list: Vec::new(),
            set: HashSet::new(),
            load: None,
            writes: Vec::new(),
            next: 1,
            applied: 0,
        }
    }
}

impl State {
    pub fn contains(&self, path: &std::path::Path) -> bool {
        self.set.contains(path)
    }

    /// A read or write is still running.
    #[cfg(test)]
    pub fn pending(&self) -> bool {
        self.load.is_some() || !self.writes.is_empty()
    }

    fn replace(&mut self, list: Vec<PathBuf>) {
        self.set = list.iter().cloned().collect();
        self.list = list;
    }
}

impl Model {
    /// Reads the starred list on a thread (once per window).
    pub fn start_starred(&mut self) {
        if self.starred.load.is_some() {
            return;
        }
        let (tx, rx) = mpsc::sync_channel(1);
        let data = self.starred.data_home.clone();
        let spawned = std::thread::Builder::new()
            .name("files-starred".into())
            .spawn(move || {
                let _ = tx.send(files_core::starred::read(&data).map_err(|e| e.to_string()));
            });
        if spawned.is_ok() {
            self.starred.load = Some(rx);
        }
    }

    /// Stars (`on`) or unstars `items`; the set follows at once and the
    /// write result confirms or corrects it.
    pub fn set_starred(&mut self, items: Vec<PathBuf>, on: bool) {
        if items.is_empty() {
            return;
        }
        for item in &items {
            if on {
                if self.starred.set.insert(item.clone()) {
                    self.starred.list.push(item.clone());
                }
            } else if self.starred.set.remove(item) {
                self.starred.list.retain(|p| p != item);
            }
        }
        let (tx, rx) = mpsc::sync_channel(1);
        let data = self.starred.data_home.clone();
        let spawned = std::thread::Builder::new()
            .name("files-starred-write".into())
            .spawn(move || {
                let _ = tx.send(files_core::starred::set(&data, &items, on));
            });
        match spawned {
            Ok(_) => {
                let id = self.starred.next;
                self.starred.next += 1;
                self.starred.writes.push((id, rx));
            }
            Err(e) => self.operation_message = Some(format!("Destacados: {e}")),
        }
    }

    /// Ctrl+E outside virtual listings: the marked items (or the focused
    /// one) become starred, or all lose their star when all had one.
    pub fn toggle_star_selection(&mut self) {
        let mut indices: Vec<usize> = self.selection.iter().copied().collect();
        if indices.is_empty() {
            indices.extend(self.selected);
        }
        indices.sort_unstable();
        let items: Vec<PathBuf> = indices
            .into_iter()
            .filter_map(|i| self.entries.get(i))
            .filter(|e| files_core::visibility::accepts(&e.name, self.hidden_visible()))
            .map(|e| self.path.join(&e.name))
            .collect();
        if items.is_empty() {
            self.operation_message = Some("Destacar: selecciona un archivo o carpeta".into());
            return;
        }
        let on = !items.iter().all(|p| self.starred.contains(p));
        let count = items.len();
        self.set_starred(items, on);
        self.operation_message = Some(match (on, count) {
            (true, 1) => "Destacado · Ctrl+Alt+E abre Destacados".into(),
            (true, n) => format!("{n} elementos destacados"),
            (false, 1) => "Quitado de destacados".into(),
            (false, n) => format!("{n} elementos quitados de destacados"),
        });
    }

    pub fn pump_starred(&mut self) -> bool {
        let mut dirty = false;
        if let Some(rx) = &self.starred.load {
            match rx.try_recv() {
                Err(mpsc::TryRecvError::Empty) => {}
                done => {
                    self.starred.load = None;
                    match done {
                        Ok(Ok(list)) if self.starred.writes.is_empty() => {
                            self.starred.replace(list);
                        }
                        Ok(Err(e)) => {
                            eprintln!("FILES_STARRED unreadable: {e}");
                            self.operation_message = Some(format!("Destacados: {e}"));
                        }
                        _ => {}
                    }
                    dirty = true;
                }
            }
        }
        let mut i = 0;
        let mut changed = false;
        let mut reload = false;
        while i < self.starred.writes.len() {
            let (id, rx) = &self.starred.writes[i];
            let id = *id;
            match rx.try_recv() {
                Err(mpsc::TryRecvError::Empty) => i += 1,
                done => {
                    self.starred.writes.swap_remove(i);
                    match done {
                        Ok(Ok(list)) if id > self.starred.applied => {
                            self.starred.applied = id;
                            self.starred.replace(list);
                            changed = true;
                        }
                        Ok(Err(e)) => {
                            self.operation_message = Some(format!("Destacados: {e}"));
                            changed = true;
                            reload = true;
                        }
                        _ => {}
                    }
                    dirty = true;
                }
            }
        }
        if reload && self.starred.writes.is_empty() {
            // The optimistic set may be wrong: read the list again.
            self.start_starred();
        }
        if changed {
            self.virtual_starred_changed();
        }
        dirty
    }
}

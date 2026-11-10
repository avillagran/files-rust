//! Favorites shared with Ofolder: a watcher thread re-reads its favorites
//! file when it changes (one `stat` every few seconds), and adding or
//! removing a favorite in Files updates Ofolder's list in its own thread.
//! Recent places from Ofolder join the "Recientes" menu (see `lib.rs`).
use crate::model::Model;
use files_core::ofolder::{Favorite, Paths};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    time::{Duration, SystemTime},
};

/// How often the favorites file's size and date are checked.
const CHECK: Duration = Duration::from_secs(3);

#[derive(Default)]
pub struct State {
    /// Ofolder's favorites in its order (empty when it is not installed).
    pub favorites: Vec<Favorite>,
    updates: Option<Receiver<Vec<Favorite>>>,
    poke: Option<Sender<()>>,
    writes: Vec<Receiver<Result<bool, String>>>,
}

impl Model {
    /// Starts the watcher (once).
    pub fn start_ofolder(&mut self) {
        if self.ofolder.updates.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let (poke, poked) = mpsc::channel::<()>();
        let paths = Paths::from_env();
        let spawned = std::thread::Builder::new()
            .name("files-ofolder".into())
            .spawn(move || {
                let mut seen: Option<Option<(u64, SystemTime)>> = None;
                loop {
                    let stamp = std::fs::metadata(&paths.favorites)
                        .ok()
                        .map(|m| (m.len(), m.modified().unwrap_or(SystemTime::UNIX_EPOCH)));
                    if seen != Some(stamp) {
                        seen = Some(stamp);
                        let list = match files_core::ofolder::read_favorites(&paths.favorites) {
                            Ok(list) => list,
                            Err(e) => {
                                eprintln!("FILES_OFOLDER favorites unreadable: {e}");
                                Vec::new()
                            }
                        };
                        if tx.send(list).is_err() {
                            return;
                        }
                    }
                    match poked.recv_timeout(CHECK) {
                        Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                }
            });
        if spawned.is_ok() {
            self.ofolder.updates = Some(rx);
            self.ofolder.poke = Some(poke);
        }
    }

    /// Mirrors a favorite added (`on`) or removed in Files into Ofolder.
    pub fn share_favorite(&mut self, path: PathBuf, on: bool) {
        let (tx, rx) = mpsc::sync_channel(1);
        let spawned = std::thread::Builder::new()
            .name("files-ofolder-write".into())
            .spawn(move || {
                let _ = tx.send(files_core::ofolder::set_favorite(
                    &Paths::from_env(),
                    &path,
                    on,
                ));
            });
        if spawned.is_ok() {
            self.ofolder.writes.push(rx);
        }
    }

    pub fn pump_ofolder(&mut self) -> bool {
        let mut dirty = false;
        if let Some(rx) = &self.ofolder.updates {
            let mut latest = None;
            while let Ok(list) = rx.try_recv() {
                latest = Some(list);
            }
            if let Some(list) = latest
                && list != self.ofolder.favorites
            {
                self.ofolder.favorites = list;
                dirty = true;
            }
        }
        let mut changed = false;
        let mut i = 0;
        while i < self.ofolder.writes.len() {
            match self.ofolder.writes[i].try_recv() {
                Err(mpsc::TryRecvError::Empty) => i += 1,
                done => {
                    self.ofolder.writes.swap_remove(i);
                    match done {
                        Ok(Ok(true)) => changed = true,
                        Ok(Err(e)) => {
                            self.operation_message = Some(format!("Favoritos de Ofolder: {e}"));
                            dirty = true;
                        }
                        _ => {}
                    }
                }
            }
        }
        if changed && let Some(poke) = &self.ofolder.poke {
            let _ = poke.send(());
        }
        dirty
    }

    /// Ofolder favorites Files does not already list as its own bookmarks.
    pub fn ofolder_only_favorites(&self) -> Vec<&Favorite> {
        self.ofolder
            .favorites
            .iter()
            .filter(|f| {
                !self
                    .bookmarks
                    .document
                    .rows
                    .iter()
                    .any(|b| b.path == f.path)
            })
            .collect()
    }
}

/// Recent files and folders from Ofolder as entries of the "Recientes" menu,
/// merged with the desktop's list: newest first, one per path.
pub fn merge_recents(
    mut list: Vec<files_core::recent::Recent>,
    shared: Vec<files_core::ofolder::Recent>,
) -> Vec<files_core::recent::Recent> {
    for r in shared {
        let when = files_core::ofolder::iso_utc(r.at);
        match list.iter_mut().find(|e| e.path == r.path) {
            Some(e) if e.when < when => e.when = when,
            Some(_) => {}
            None => list.push(files_core::recent::Recent {
                path: r.path,
                when,
                folder: r.folder,
            }),
        }
    }
    list.sort_by(|a, b| b.when.cmp(&a.when));
    list.truncate(files_core::recent::MAX_ITEMS);
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use files_core::recent::Recent;

    #[test]
    fn recents_from_both_sources_merge_newest_first_once_per_path() {
        let desktop = vec![
            Recent {
                path: "/d/a.txt".into(),
                when: "2026-11-09T10:00:00Z".into(),
                folder: false,
            },
            Recent {
                path: "/d/b.txt".into(),
                when: "2026-11-08T10:00:00Z".into(),
                folder: false,
            },
        ];
        let shared = vec![
            files_core::ofolder::Recent {
                path: "/d".into(),
                folder: true,
                at: 1_794_000_000,
            },
            files_core::ofolder::Recent {
                path: "/d/b.txt".into(),
                folder: false,
                at: 1_794_300_000,
            },
        ];
        let merged = merge_recents(desktop, shared);
        let order: Vec<_> = merged.iter().map(|r| r.path.to_str().unwrap()).collect();
        assert_eq!(order, ["/d/b.txt", "/d/a.txt", "/d"]);
        assert!(merged[2].folder && !merged[0].folder);
    }

    #[test]
    fn ofolder_favorites_already_bookmarked_are_not_listed_twice() {
        let mut m = Model::new("/fixture-ofolder".into());
        let mut doc = files_core::bookmarks::Document::default();
        doc.rows.push(files_core::bookmarks::Entry {
            id: 1,
            label: "Work".into(),
            path: "/fixture-ofolder/work".into(),
        });
        m.bookmarks.document = std::sync::Arc::new(doc);
        m.ofolder.favorites = vec![
            Favorite {
                path: "/fixture-ofolder/work".into(),
                name: None,
            },
            Favorite {
                path: "/fixture-ofolder/music".into(),
                name: Some("Música".into()),
            },
        ];
        let only: Vec<_> = m
            .ofolder_only_favorites()
            .iter()
            .map(|f| f.label())
            .collect();
        assert_eq!(only, ["Música"]);
    }
}

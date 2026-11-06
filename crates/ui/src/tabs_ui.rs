//! Per-tab browser snapshots; operations and all supervisors belong to Model.
use crate::{
    model::{Action, Hit, Model},
    painter::{Painter, Rect},
    refresh_selection::Snapshot,
    tabs::{self, Command, Saved, Tab},
};
impl Model {
    fn tab_charge(&self) -> Result<usize, &'static str> {
        files_core::navigation_history::validate(&self.path)?;
        if self.query.len() > 4096 {
            return Err("Filtro demasiado largo para guardar pestaña");
        }
        let selection = if let Some(snapshot) = &self.refresh_selection {
            snapshot.retained_bytes()
        } else {
            Snapshot::estimate(
                self.entries
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| self.selection.contains(i))
                    .map(|(_, e)| e.name.as_os_str()),
                self.selected
                    .and_then(|i| self.entries.get(i))
                    .map(|e| e.name.as_os_str()),
                self.anchor
                    .and_then(|i| self.entries.get(i))
                    .map(|e| e.name.as_os_str()),
                tabs::SELECTION_LIMIT,
            )?
        };
        if selection > tabs::SELECTION_LIMIT {
            return Err("Selección de pestaña supera256KiB; reduce selección antes de cambiar");
        }
        let charge = selection
            + self.history.retained_bytes()
            + self.path.as_os_str().len()
            + self.query.len()
            + 8192;
        if charge > tabs::SNAPSHOT_LIMIT {
            return Err("Estado de pestaña supera1MiB; reduce selección");
        }
        Ok(charge)
    }
    fn capture_tab(&mut self, charge: usize) -> Result<Saved, &'static str> {
        let selection = if let Some(snapshot) = self.refresh_selection.take() {
            snapshot
        } else {
            Snapshot::capture_with_anchor(
                self.entries
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| self.selection.contains(i))
                    .map(|(_, e)| e.name.as_os_str()),
                self.selected
                    .and_then(|i| self.entries.get(i))
                    .map(|e| e.name.as_os_str()),
                self.anchor
                    .and_then(|i| self.entries.get(i))
                    .map(|e| e.name.as_os_str()),
                tabs::SELECTION_LIMIT,
            )?
        };
        Ok(Saved {
            path: self.path.clone(),
            history: std::mem::take(&mut self.history),
            selection,
            view: self.view,
            sort: self.sort,
            descending: self.descending,
            query: self.query.clone(),
            scroll: self.scroll,
            grid_zoom: self.grid_zoom,
            pan_x: self.pan_x,
            charge,
        })
    }
    pub(crate) fn restore_tab(&mut self, saved: Saved) {
        self.view = saved.view;
        self.load_location(saved.path);
        self.folder_restore = None;
        self.history = saved.history;
        self.query = saved.query;
        self.sort = saved.sort;
        self.descending = saved.descending;
        self.grid_zoom = saved.grid_zoom;
        self.pan_x = saved.pan_x;
        self.scroll = saved.scroll;
        self.refresh_selection = Some(saved.selection);
        self.settings = false;
        self.help = None;
        self.text_viewport = Default::default();
        self.content_height = 0;
    }
    /// Add metadata only; the active listing/search and worker generation stay untouched.
    #[cfg(test)]
    pub(crate) fn background_tab(
        &mut self,
        path: &std::path::Path,
        focus: Option<&std::ffi::OsStr>,
    ) -> Result<u64, &'static str> {
        self.background_tabs(std::iter::once((path, focus)))
            .map(|ids| ids[0])
    }
    fn background_tab_admission(&self) -> Result<(), &'static str> {
        if self.closing
            || self.mutations.busy
            || self.mutation_applied
            || self.rename_dialog.is_some()
            || self.properties_dialog.is_some()
            || self.copy_form.is_some()
            || self.notice_open
            || self.editing
        {
            return Err("Termina la edición u operación antes de abrir otra pestaña");
        }
        Ok(())
    }

    pub(crate) fn background_tabs<'a>(
        &mut self,
        targets: impl Iterator<Item = (&'a std::path::Path, Option<&'a std::ffi::OsStr>)> + Clone,
    ) -> Result<Vec<u64>, &'static str> {
        self.background_tab_admission()?;
        let reserved = self.dual.other_stored();
        publish_background_tabs(
            &mut self.tabs,
            targets,
            BackgroundStyle {
                view: self.view,
                sort: self.sort,
                descending: self.descending,
                grid_zoom: self.grid_zoom,
            },
            reserved,
        )
    }

    /// Resolve only a current visible folder; no stat, selection or pane switch.
    /// Background tabs belong to the pane whose folder was clicked.
    pub(crate) fn background_folder_tab(
        &mut self,
        action: &Action,
    ) -> Result<Option<u64>, &'static str> {
        let folder = |path: &std::path::Path,
                      entries: &[files_core::listing::Entry],
                      order: &[usize],
                      i: usize| {
            let e = entries.get(i)?;
            (order.contains(&i) && e.is_dir()).then(|| path.join(&e.name))
        };
        let mut inactive = false;
        let path = match action {
            Action::Navigate(path) => Some(path.clone()),
            Action::Entry(i) => folder(&self.path, &self.entries, &self.order, *i),
            Action::PaneEntry(id, revision, i) if self.dual.enabled => {
                if *id == self.dual.active_id && *revision == self.pane_revision {
                    folder(&self.path, &self.entries, &self.order, *i)
                } else if let Some(other) = self.dual.other.as_ref()
                    && other.id == *id
                    && other.revision == *revision
                {
                    inactive = true;
                    folder(&other.path, &other.entries, &other.order, *i)
                } else {
                    None
                }
            }
            Action::ColumnEntry(id, i)
                if self.view == crate::model::View::Columns && !self.dual.enabled =>
            {
                self.columns
                    .columns
                    .iter()
                    .enumerate()
                    .find(|(_, c)| c.id == *id)
                    .and_then(|(index, c)| {
                        if index == self.columns.active {
                            (c.path == self.path)
                                .then(|| folder(&self.path, &self.entries, &self.order, *i))
                                .flatten()
                        } else {
                            folder(&c.path, &c.data.entries, &c.data.order, *i)
                        }
                    })
            }
            _ => None,
        };
        let Some(path) = path else { return Ok(None) };
        if !inactive {
            return self
                .background_tabs(std::iter::once((path.as_path(), None)))
                .map(|ids| Some(ids[0]));
        }
        self.background_tab_admission()?;
        let reserved = self.tabs.stored_bytes().saturating_add(
            self.dual
                .other_stored()
                .saturating_sub(self.dual.other.as_ref().unwrap().tabs.stored_bytes()),
        );
        let other = self.dual.other.as_mut().unwrap();
        publish_background_tabs(
            &mut other.tabs,
            std::iter::once((path.as_path(), None)),
            BackgroundStyle {
                view: self.view,
                sort: other.sort,
                descending: other.descending,
                grid_zoom: self.grid_zoom,
            },
            reserved,
        )
        .map(|ids| Some(ids[0]))
    }
    /// Preflight the whole context action before moving any snapshots or IDs.
    fn close_tab_set(&mut self, target: u64, right_only: bool) {
        let Some(at) = self.tabs.index(target) else {
            return;
        };
        let closing = self
            .tabs
            .rows
            .iter()
            .enumerate()
            .filter(|(i, tab)| {
                if right_only {
                    *i > at
                } else {
                    tab.id != target
                }
            })
            .map(|(_, tab)| tab.id)
            .collect::<Vec<_>>();
        if closing.is_empty() {
            return;
        }
        // Both commands always retain the clicked tab, including a lone tab.
        let closes_active = closing.contains(&self.tabs.active);
        let evicted = (self.tabs.closed.len() + closing.len()).saturating_sub(tabs::MAX_TABS);
        let mut released = self
            .tabs
            .closed
            .iter()
            .take(evicted)
            .try_fold(0usize, |sum, saved| sum.checked_add(saved.charge));
        if self
            .tabs
            .rows
            .iter()
            .any(|tab| tab.id != self.tabs.active && tab.saved.is_none())
        {
            self.error = "Pestañas: falta el estado de una pestaña inactiva".into();
            return;
        }
        if closes_active {
            let incoming = self.tabs.rows[at]
                .saved
                .as_ref()
                .expect("inactive retained tab")
                .charge;
            released = released.and_then(|sum| sum.checked_add(incoming));
        }
        let outgoing_charge = if closes_active {
            self.tab_charge()
        } else {
            Ok(0)
        };
        let charge = match outgoing_charge.and_then(|charge| {
            let released = released.ok_or("Presupuesto de pestañas agotado")?;
            self.tabs
                .admit_with_reserved(charge, released, self.dual.other_stored())?;
            Ok(charge)
        }) {
            Ok(charge) => charge,
            Err(error) => {
                self.error = error.into();
                return;
            }
        };
        let mut outgoing = if closes_active {
            if !self.save_layout() {
                return;
            }
            match self.capture_tab(charge) {
                Ok(saved) => Some(saved),
                Err(error) => {
                    self.error = error.into();
                    return;
                }
            }
        } else {
            None
        };
        let incoming = if closes_active {
            Some(
                self.tabs.rows[at]
                    .saved
                    .take()
                    .expect("retained tab snapshot"),
            )
        } else {
            None
        };
        let rows = std::mem::take(&mut self.tabs.rows);
        for tab in rows {
            if closing.contains(&tab.id) {
                let saved = if tab.id == self.tabs.active {
                    outgoing.take().expect("active outgoing snapshot")
                } else {
                    tab.saved.expect("inactive outgoing snapshot")
                };
                self.tabs.push_closed(saved);
            } else {
                self.tabs.rows.push(tab);
            }
        }
        if let Some(incoming) = incoming {
            self.tabs.active = target;
            self.restore_tab(incoming);
        }
        self.tabs.first = self.tabs.first.min(self.tabs.rows.len() - 1);
        self.hits.clear();
    }
    pub fn tab_command(&mut self, command: Command) {
        if self.closing {
            return;
        }
        // Apply/undo/redo and edits cannot race a tab change. Their worker still
        // refreshes its captured directory under this explicit UI invariant.
        if self.mutations.busy
            || self.mutation_applied
            || self.rename_dialog.is_some()
            || self.properties_dialog.is_some()
            || self.copy_form.is_some()
            || self.notice_open
            || self.editing
        {
            self.error = "Termina o cancela la edición u operación antes de cambiar pestaña".into();
            return;
        }
        match command {
            Command::Move { id, before } => {
                if self.tabs.move_before(id, before) {
                    self.hits.clear();
                }
                return;
            }
            Command::CloseOthers(id) => {
                self.close_tab_set(id, false);
                return;
            }
            Command::CloseRight(id) => {
                self.close_tab_set(id, true);
                return;
            }
            _ => {}
        }
        let command = if let Command::Cycle(delta) = command {
            let i = self.tabs.index(self.tabs.active).expect("active tab");
            let target = (i as i32 + delta).rem_euclid(self.tabs.rows.len() as i32) as usize;
            Command::Select(self.tabs.rows[target].id)
        } else {
            command
        };
        if let Command::Select(id) = command
            && (id == self.tabs.active || self.tabs.index(id).is_none())
        {
            return;
        }
        if let Command::Close(id) = command {
            let Some(index) = self.tabs.index(id) else {
                return;
            };
            if id != self.tabs.active {
                let tab = self.tabs.rows.remove(index);
                self.tabs.push_closed(tab.saved.expect("inactive saved"));
                self.hits.clear();
                return;
            }
            if self.tabs.rows.len() == 1 {
                if self.dual.enabled {
                    self.close_focused_pane();
                    return;
                }
                self.closing = true;
                return;
            }
        }
        if matches!(command, Command::New | Command::Reopen)
            && self.tabs.rows.len() == tabs::MAX_TABS
        {
            self.error = "Máximo ocho pestañas; cierra una antes de abrir otra".into();
            return;
        }
        if matches!(command, Command::Reopen) && self.tabs.closed.is_empty() {
            return;
        }
        let incoming_id = match command {
            Command::Select(id) => Some(id),
            Command::Close(_) => {
                let i = self.tabs.index(self.tabs.active).unwrap();
                Some(
                    self.tabs.rows[if i + 1 < self.tabs.rows.len() {
                        i + 1
                    } else {
                        i - 1
                    }]
                    .id,
                )
            }
            _ => None,
        };
        let released = incoming_id
            .and_then(|id| self.tabs.rows[self.tabs.index(id)?].saved.as_ref())
            .map_or(0, |s| s.charge)
            + if matches!(command, Command::Reopen) {
                self.tabs.closed.last().map_or(0, |s| s.charge)
            } else if matches!(command, Command::Close(_))
                && self.tabs.closed.len() == tabs::MAX_TABS
            {
                self.tabs.closed[0].charge
            } else {
                0
            };
        let charge = match self.tab_charge().and_then(|n| {
            self.tabs
                .admit_with_reserved(n, released, self.dual.other_stored())
                .map(|_| n)
        }) {
            Ok(n) => n,
            Err(e) => {
                self.error = format!("Pestañas: {e}");
                return;
            }
        };
        if !self.save_layout() {
            return;
        }
        let outgoing = match self.capture_tab(charge) {
            Ok(s) => s,
            Err(e) => {
                self.error = e.into();
                return;
            }
        };
        let old = self.tabs.index(self.tabs.active).unwrap();
        let incoming = match command {
            Command::New => {
                let id = self.tabs.fresh_id();
                let path = outgoing.path.clone();
                let saved = Saved {
                    path: path.clone(),
                    history: Default::default(),
                    selection: Snapshot::capture(std::iter::empty(), None).expect("empty snapshot"),
                    view: outgoing.view,
                    sort: outgoing.sort,
                    descending: outgoing.descending,
                    query: String::new(),
                    scroll: 0,
                    grid_zoom: outgoing.grid_zoom,
                    pan_x: 0.,
                    charge: 0,
                };
                self.tabs.rows[old].saved = Some(outgoing);
                self.tabs.rows.push(Tab {
                    id,
                    path,
                    saved: None,
                });
                self.tabs.active = id;
                saved
            }
            Command::Reopen => {
                let saved = self.tabs.closed.pop().unwrap();
                let id = self.tabs.fresh_id();
                self.tabs.rows[old].saved = Some(outgoing);
                self.tabs.rows.push(Tab {
                    id,
                    path: saved.path.clone(),
                    saved: None,
                });
                self.tabs.active = id;
                saved
            }
            Command::Select(id) | Command::Close(id) => {
                let next = if matches!(command, Command::Close(_)) {
                    incoming_id.unwrap()
                } else {
                    id
                };
                let index = self.tabs.index(next).unwrap();
                let saved = self.tabs.rows[index].saved.take().unwrap();
                if matches!(command, Command::Close(_)) {
                    self.tabs.rows.remove(old);
                    self.tabs.push_closed(outgoing);
                } else {
                    self.tabs.rows[old].saved = Some(outgoing);
                }
                self.tabs.active = next;
                saved
            }
            Command::Cycle(_)
            | Command::Move { .. }
            | Command::CloseOthers(_)
            | Command::CloseRight(_) => unreachable!(),
        };
        self.restore_tab(incoming);
    }
}
/// The same admission/publication path is used for active and inactive panes.
struct BackgroundStyle {
    view: crate::model::View,
    sort: files_core::listing::Sort,
    descending: bool,
    grid_zoom: f32,
}
fn publish_background_tabs<'a>(
    tabs: &mut tabs::Tabs,
    targets: impl Iterator<Item = (&'a std::path::Path, Option<&'a std::ffi::OsStr>)> + Clone,
    style: BackgroundStyle,
    reserved: usize,
) -> Result<Vec<u64>, &'static str> {
    let BackgroundStyle {
        view,
        sort,
        descending,
        grid_zoom,
    } = style;
    let count = targets.clone().count();
    if count == 0 {
        return Err("No quedan resultados marcados; vuelve a elegir");
    }
    if count > tabs::MAX_TABS.saturating_sub(tabs.rows.len()) {
        return Err("Máximo ocho pestañas; el lote no cabe, reduce las marcas o cierra pestañas");
    }
    let history = files_core::navigation_history::History::default();
    let mut total = 0usize;
    for (path, focus) in targets.clone() {
        files_core::navigation_history::validate(path)?;
        if focus.is_some_and(|n| {
            n.is_empty()
                || n == "."
                || n == ".."
                || n.as_encoded_bytes().iter().any(|b| matches!(b, 0 | b'/'))
        }) {
            return Err("Nombre enfocado inválido para la pestaña");
        }
        let charge = Snapshot::estimate(focus.into_iter(), focus, focus, tabs::SELECTION_LIMIT)?
            + history.retained_bytes()
            + path.as_os_str().len()
            + 8192;
        total = total
            .checked_add(charge)
            .ok_or("Presupuesto de pestañas agotado")?;
    }
    tabs.admit_with_reserved(total, 0, reserved)?;
    // Construct every bounded snapshot before publishing any new tab/ID.
    let prepared = targets
        .map(|(path, focus)| {
            let selection = Snapshot::capture_with_anchor(
                focus.into_iter(),
                focus,
                focus,
                tabs::SELECTION_LIMIT,
            )?;
            let charge = selection.retained_bytes()
                + history.retained_bytes()
                + path.as_os_str().len()
                + 8192;
            Ok(Saved {
                path: path.to_path_buf(),
                history: Default::default(),
                selection,
                view,
                sort,
                descending,
                query: String::new(),
                scroll: 0,
                grid_zoom,
                pan_x: 0.,
                charge,
            })
        })
        .collect::<Result<Vec<_>, &'static str>>()?;
    let mut ids = Vec::with_capacity(count);
    for saved in prepared {
        let id = tabs.fresh_id();
        let path = saved.path.clone();
        tabs.rows.push(Tab {
            id,
            path,
            saved: Some(saved),
        });
        ids.push(id);
    }
    Ok(ids)
}
/// Horizontal strip never enters the draggable header. Stable IDs, clipped label
/// targets and independent24px close targets; reveal active tab at narrow widths.
pub fn paint(p: &mut dyn Painter, m: &mut Model, w: i32) {
    let c = m.palette.clone();
    let y = 58;
    p.rect(Rect::new(0, y, w, tabs::HEIGHT), c.darker_background);
    p.rect(Rect::new(0, y + tabs::HEIGHT - 1, w, 1), c.muted);
    if m.tabs_widget {
        let new = Rect::new(w - 34, y + 4, 28, 24);
        p.text(new, crate::fonts::icon("", "+"), 13, c.accent, true, false);
        m.hits.push(Hit {
            generation: m.generation,
            rect: new,
            action: Action::Tab(Command::New),
        });
        return;
    }
    let slots = ((w - 106).max(120) / 180).max(1) as usize;
    let active = m.tabs.index(m.tabs.active).unwrap_or(0);
    if active < m.tabs.first {
        m.tabs.first = active;
    }
    if active >= m.tabs.first + slots {
        m.tabs.first = active + 1 - slots;
    }
    m.tabs.first = m.tabs.first.min(m.tabs.rows.len().saturating_sub(1));
    let mut controls = Vec::with_capacity(20);
    let mut x = 8;
    for tab in m.tabs.rows.iter().skip(m.tabs.first).take(slots) {
        let width = 180.min(w - x - 100).max(64);
        let area = Rect::new(x, y + 3, width, 26);
        let on = tab.id == m.tabs.active;
        p.rounded_rect(
            area,
            5 * m.radius,
            if on {
                c.lighter_background
            } else {
                c.darker_background
            },
        );
        if on {
            p.rect(
                Rect::new(x + 4, y + tabs::HEIGHT - 3, width - 8, 2),
                c.accent,
            );
        }
        let label = tab
            .path
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("/"));
        let text = files_core::name_label::label(label);
        p.clip(Rect::new(x + 8, y + 3, width - 38, 26));
        p.text(
            Rect::new(x + 8, y + 3, width - 38, 26),
            &text,
            12,
            if on { c.foreground } else { c.dark_foreground },
            true,
            false,
        );
        p.unclip();
        controls.push((Rect::new(x, y + 3, width - 28, 26), Command::Select(tab.id)));
        p.text(
            Rect::new(x + width - 27, y + 4, 24, 24),
            crate::fonts::icon("󰅖", "×"),
            13,
            c.dark_foreground,
            true,
            false,
        );
        controls.push((
            Rect::new(x + width - 27, y + 4, 24, 24),
            Command::Close(tab.id),
        ));
        x += width + 4;
    }
    let new = Rect::new(w - 91, y + 4, 28, 24);
    p.text(new, crate::fonts::icon("", "+"), 13, c.accent, true, false);
    controls.push((new, Command::New));
    for (x, delta, label) in [
        (w - 61, -1, crate::fonts::icon("", "‹")),
        (w - 33, 1, crate::fonts::icon("", "›")),
    ] {
        let r = Rect::new(x, y + 4, 26, 24);
        p.text(r, label, 14, c.foreground, true, false);
        controls.push((r, Command::Cycle(delta)));
    }
    for (rect, command) in controls {
        m.hits.push(Hit {
            generation: m.generation,
            rect,
            action: Action::Tab(command),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        time::{Duration, Instant},
    };
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = files_core::theme::home()
                .join(".cache/files-test")
                .join(format!(
                    "tabs-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ));
            fs::create_dir_all(root.join("A")).unwrap();
            fs::create_dir(root.join("B")).unwrap();
            for n in ["A", "B"] {
                fs::write(root.join(n).join("same.txt"), n).unwrap();
                fs::write(root.join(n).join("other.txt"), n).unwrap();
                fs::write(root.join(n).join(".hidden"), n).unwrap();
            }
            Self(root)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn wait(m: &mut Model) {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            m.pump();
            if m.loaded && m.refresh_selection.is_none() {
                break;
            }
            assert!(Instant::now() < until, "{}", m.error);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn select(m: &mut Model, name: &str, toggle: bool) {
        let i = m.entries.iter().position(|e| e.name == name).unwrap();
        m.select_entry(i, toggle, false);
    }
    fn tab_ids(m: &Model) -> Vec<u64> {
        m.tabs.rows.iter().map(|tab| tab.id).collect()
    }
    #[test]
    fn reorder_uses_stable_insertion_without_switching_or_recharging_tabs() {
        let f = Fixture::new();
        let mut m = Model::new(f.0.join("A"));
        wait(&mut m);
        select(&mut m, "same.txt", false);
        select(&mut m, "other.txt", true);
        m.query = "same|other".into();
        m.scroll = 37;
        let active = m.tabs.active;
        let generation = m.generation;
        let selection = m.selection.clone();
        let selected = m.selected;
        let anchor = m.anchor;
        let history = (m.history.paths().0.clone(), m.history.paths().1.clone());
        let path = m.path.clone();
        let b = m.background_tab(&f.0.join("B"), None).unwrap();
        let c = m.background_tab(&f.0.join("A"), None).unwrap();
        let stored = m.tabs.stored_bytes();
        m.tab_command(Command::Move {
            id: c,
            before: Some(active),
        });
        assert_eq!(tab_ids(&m), [c, active, b]);
        m.tab_command(Command::Move {
            id: c,
            before: None,
        });
        assert_eq!(tab_ids(&m), [active, b, c]);
        for (id, before) in [
            (active, Some(active)),
            (b, Some(c)),
            (99, None),
            (b, Some(99)),
        ] {
            m.tab_command(Command::Move { id, before });
            assert_eq!(tab_ids(&m), [active, b, c]);
        }
        assert_eq!(m.tabs.active, active);
        assert_eq!(m.generation, generation);
        assert_eq!(m.path, path);
        assert_eq!(m.query, "same|other");
        assert_eq!(m.scroll, 37);
        assert_eq!(m.selection, selection);
        assert_eq!(m.selected, selected);
        assert_eq!(m.anchor, anchor);
        assert_eq!(m.history.paths(), (&history.0, &history.1));
        assert_eq!(m.tabs.stored_bytes(), stored);
        assert!(m.tabs.closed.is_empty());
        m.editing = true;
        m.tab_command(Command::Move {
            id: c,
            before: Some(active),
        });
        assert_eq!(tab_ids(&m), [active, b, c]);
    }
    #[test]
    fn background_context_closures_preserve_active_browser_and_never_close_last_tab() {
        let f = Fixture::new();
        let mut m = Model::new(f.0.join("A"));
        wait(&mut m);
        select(&mut m, "same.txt", false);
        let active = m.tabs.active;
        let generation = m.generation;
        let selection = m.selection.clone();
        let selected = m.selected;
        let anchor = m.anchor;
        let path = m.path.clone();
        let b = m.background_tab(&f.0.join("B"), None).unwrap();
        let c = m.background_tab(&f.0.join("A"), None).unwrap();
        m.tab_command(Command::CloseRight(b));
        assert_eq!(tab_ids(&m), [active, b]);
        assert!(m.tabs.index(c).is_none());
        assert_eq!(m.tabs.active, active);
        assert_eq!(m.generation, generation);
        assert_eq!(m.path, path);
        assert_eq!(m.selection, selection);
        assert_eq!(m.selected, selected);
        assert_eq!(m.anchor, anchor);
        m.tab_command(Command::CloseOthers(active));
        assert_eq!(tab_ids(&m), [active]);
        assert_eq!(m.tabs.closed.len(), 2);
        for command in [
            Command::CloseOthers(active),
            Command::CloseRight(active),
            Command::CloseOthers(999),
            Command::CloseRight(999),
        ] {
            m.tab_command(command);
            assert_eq!(tab_ids(&m), [active]);
            assert!(!m.closing);
        }
    }
    #[test]
    fn context_closing_active_preflights_all_tabs_before_switching_to_clicked_survivor() {
        let f = Fixture::new();
        let mut m = Model::new(f.0.join("A"));
        wait(&mut m);
        let a = m.tabs.active;
        let b = m.background_tab(&f.0.join("B"), None).unwrap();
        let c = m.background_tab(&f.0.join("A"), None).unwrap();
        let path = m.path.clone();
        let generation = m.generation;
        m.query = "x".repeat(4097);
        m.tab_command(Command::CloseOthers(b));
        assert_eq!(tab_ids(&m), [a, b, c]);
        assert_eq!(m.tabs.active, a);
        assert_eq!(m.path, path);
        assert_eq!(m.generation, generation);
        assert!(m.tabs.closed.is_empty());
        m.query.clear();
        m.error.clear();
        m.tab_command(Command::CloseOthers(b));
        assert_eq!(tab_ids(&m), [b]);
        assert_eq!(m.tabs.active, b);
        assert!(!m.closing);
        wait(&mut m);
        assert_eq!(m.path, f.0.join("B"));
        assert_eq!(m.tabs.closed.len(), 2);
        m.tab_command(Command::Reopen);
        wait(&mut m);
        assert_eq!(m.path, f.0.join("A"));
    }
    fn add_closed_history(m: &mut Model, path: &std::path::Path, count: usize) {
        for i in 0..count {
            let id = m.background_tab(path, None).unwrap();
            m.tab_command(Command::Close(id));
            m.tabs.closed.last_mut().unwrap().query = format!("closed-{i}");
        }
    }
    #[test]
    fn batch_context_close_reserves_final_budget_without_partially_removing_tabs() {
        let f = Fixture::new();
        let mut m = Model::new(f.0.join("A"));
        wait(&mut m);
        let a = m.tabs.active;
        let b = m.background_tab(&f.0.join("B"), None).unwrap();
        let c = m.background_tab(&f.0.join("A"), None).unwrap();
        add_closed_history(&mut m, &f.0.join("B"), 3);
        let live = m
            .tabs
            .rows
            .iter()
            .filter_map(|t| t.saved.as_ref())
            .map(|s| s.charge)
            .sum::<usize>();
        m.tabs.closed[0].charge = tabs::SNAPSHOT_LIMIT;
        m.tabs.closed[1].charge = tabs::SNAPSHOT_LIMIT;
        m.tabs.closed[2].charge = tabs::SNAPSHOT_LIMIT - live - 1;
        assert_eq!(m.tabs.stored_bytes(), tabs::STORED_LIMIT - 1);
        let closed = m
            .tabs
            .closed
            .iter()
            .map(|s| (s.query.clone(), s.charge))
            .collect::<Vec<_>>();
        let generation = m.generation;
        let path = m.path.clone();
        m.query = "x".repeat(4096);
        m.tab_command(Command::CloseOthers(b));
        assert!(m.error.contains("presupuesto"));
        assert_eq!(tab_ids(&m), [a, b, c]);
        assert_eq!(m.tabs.active, a);
        assert_eq!(m.path, path);
        assert_eq!(m.generation, generation);
        assert_eq!(
            m.tabs
                .closed
                .iter()
                .map(|s| (s.query.clone(), s.charge))
                .collect::<Vec<_>>(),
            closed
        );
    }
    #[test]
    fn batch_context_close_accounts_for_all_closed_history_evictions() {
        let f = Fixture::new();
        let mut m = Model::new(f.0.join("A"));
        wait(&mut m);
        let a = m.tabs.active;
        let b = m.background_tab(&f.0.join("B"), None).unwrap();
        let c = m.background_tab(&f.0.join("A"), None).unwrap();
        add_closed_history(&mut m, &f.0.join("B"), tabs::MAX_TABS);
        let live = m
            .tabs
            .rows
            .iter()
            .filter_map(|t| t.saved.as_ref())
            .map(|s| s.charge)
            .sum::<usize>();
        let tail = m
            .tabs
            .closed
            .iter()
            .skip(3)
            .map(|s| s.charge)
            .sum::<usize>();
        m.tabs.closed[0].charge = tabs::SNAPSHOT_LIMIT;
        m.tabs.closed[1].charge = tabs::SNAPSHOT_LIMIT;
        m.tabs.closed[2].charge = tabs::SNAPSHOT_LIMIT - live - tail;
        assert_eq!(m.tabs.stored_bytes(), tabs::STORED_LIMIT);
        m.tab_command(Command::CloseOthers(b));
        assert_eq!(tab_ids(&m), [b]);
        assert_eq!(m.tabs.active, b);
        assert!(m.tabs.index(a).is_none() && m.tabs.index(c).is_none());
        assert_eq!(m.tabs.closed.len(), tabs::MAX_TABS);
        assert_eq!(m.tabs.closed[0].query, "closed-2");
        assert!(m.tabs.stored_bytes() <= tabs::STORED_LIMIT);
        wait(&mut m);
        assert_eq!(m.path, f.0.join("B"));
    }
    #[test]
    fn close_right_uses_visual_order_and_switches_only_if_active_is_removed() {
        let f = Fixture::new();
        let mut m = Model::new(f.0.join("A"));
        wait(&mut m);
        let a = m.tabs.active;
        let b = m.background_tab(&f.0.join("B"), None).unwrap();
        let c = m.background_tab(&f.0.join("A"), None).unwrap();
        m.tab_command(Command::Move {
            id: a,
            before: None,
        });
        assert_eq!(tab_ids(&m), [b, c, a]);
        m.tab_command(Command::CloseRight(b));
        assert_eq!(tab_ids(&m), [b]);
        assert_eq!(m.tabs.active, b);
        assert_eq!(m.tabs.closed.len(), 2);
        wait(&mut m);
        assert_eq!(m.path, f.0.join("B"));
    }
    #[test]
    fn switches_before_first_batch_keep_names_query_sort_anchor_and_global_hidden() {
        let f = Fixture::new();
        let mut m = Model::new(f.0.join("A"));
        wait(&mut m);
        m.show_hidden = true;
        m.rebuild();
        select(&mut m, "other.txt", false);
        select(&mut m, "same.txt", true);
        m.view = crate::model::View::Grid;
        m.grid_zoom = 1.4;
        m.pan_x = 57.;
        m.scroll = 321;
        m.sort = files_core::listing::Sort::Size;
        m.descending = true;
        let a = m.tabs.active;
        m.tab_command(Command::New);
        let b = m.tabs.active;
        m.navigate(f.0.join("B"));
        // B's path may have no batches yet; its old indices must never select A.
        m.tab_command(Command::Select(a));
        m.tab_command(Command::Select(b));
        m.tab_command(Command::Select(a));
        wait(&mut m);
        assert_eq!(m.path, f.0.join("A"));
        assert!(
            m.view == crate::model::View::Grid
                && m.grid_zoom == 1.4
                && m.pan_x == 57.
                && m.scroll == 321
                && m.descending
        );
        assert!(matches!(m.sort, files_core::listing::Sort::Size));
        assert_eq!(m.selection.len(), 2);
        assert_eq!(m.entries[m.anchor.unwrap()].name, "other.txt");
        assert_eq!(m.entries[m.selected.unwrap()].name, "same.txt");
        m.tab_command(Command::Select(b));
        wait(&mut m);
        assert_eq!(m.path, f.0.join("B"));
        m.query = "same".into();
        m.rebuild();
        select(&mut m, "same.txt", false);
        m.tab_command(Command::Select(a));
        m.show_hidden = false;
        m.rebuild();
        wait(&mut m);
        m.tab_command(Command::Select(b));
        wait(&mut m);
        assert_eq!(m.query, "same");
        assert!(!m.show_hidden);
        assert_eq!(m.selection.len(), 1);
        assert_eq!(m.entries[m.selected.unwrap()].name, "same.txt");
        assert!(m.history.can_back());
        assert_eq!(fs::read(m.path.join("same.txt")).unwrap(), b"B");
    }
    #[test]
    fn refresh_selection_intersects_changed_filter_and_hidden_before_copy_rename() {
        let f = Fixture::new();
        let mut m = Model::new(f.0.join("A"));
        wait(&mut m);
        m.show_hidden = true;
        m.rebuild();
        m.action(Action::Selection(crate::selection_tools::Command::All));
        m.refresh_current();
        m.query = "same".into();
        m.show_hidden = false;
        wait(&mut m);
        assert_eq!(m.selection.len(), 1);
        assert_eq!(m.entries[m.selected.unwrap()].name, "same.txt");
        m.open_copy();
        let form = m.copy_form.as_ref().unwrap();
        assert_eq!(form.names.as_ref(), [std::ffi::OsString::from("same.txt")]);
        m.copy_form = None;
        m.open_rename(crate::rename_dialog::Mode::Rename);
        assert_eq!(
            m.rename_dialog.as_ref().unwrap().selected.as_ref(),
            [std::ffi::OsString::from("same.txt")]
        );
    }
    #[test]
    fn stable_ids_close_reopen_limit_and_last_tab_close_request() {
        let f = Fixture::new();
        let mut m = Model::new(f.0.join("A"));
        wait(&mut m);
        let a = m.tabs.active;
        m.tab_command(Command::New);
        let b = m.tabs.active;
        m.navigate(f.0.join("B"));
        m.tab_command(Command::Close(b));
        assert_eq!(m.tabs.active, a);
        m.tab_command(Command::Select(b));
        assert_eq!(m.tabs.active, a);
        m.tab_command(Command::Reopen);
        assert!(m.tabs.active > b);
        wait(&mut m);
        assert_eq!(m.path, f.0.join("B"));
        for _ in 0..6 {
            m.tab_command(Command::New);
        }
        assert_eq!(m.tabs.rows.len(), 8);
        let active = m.tabs.active;
        m.tab_command(Command::New);
        assert_eq!(m.tabs.active, active);
        assert!(m.error.contains("ocho"));
        let ids = m
            .tabs
            .rows
            .iter()
            .map(|t| t.id)
            .filter(|id| *id != active)
            .collect::<Vec<_>>();
        for id in ids {
            m.tab_command(Command::Close(id));
        }
        assert_eq!(m.tabs.rows.len(), 1);
        m.tab_command(Command::Close(active));
        assert!(m.closing);
    }
    #[test]
    fn budget_rejection_and_apply_lock_preserve_active_listing() {
        let f = Fixture::new();
        let mut m = Model::new(f.0.join("A"));
        wait(&mut m);
        let i = m.entries.iter().position(|e| e.name == "same.txt").unwrap();
        m.entries[i].name = std::ffi::OsString::from("x".repeat(4097));
        m.selection.clear();
        m.selection.insert(i);
        m.selected = Some(i);
        let generation = m.generation;
        m.tab_command(Command::New);
        assert_eq!(m.tabs.rows.len(), 1);
        assert_eq!(m.generation, generation);
        assert!(m.error.contains("Pestañas"));
        m.entries[i].name = "same.txt".into();
        m.mutation_applied = true;
        m.tab_command(Command::New);
        assert_eq!(m.tabs.rows.len(), 1);
        m.mutation_applied = false;
        m.tab_command(Command::New);
        assert_eq!(m.tabs.rows.len(), 2);
    }
    #[test]
    fn snapshot_selection_is_pruned_when_hidden_changes_while_inactive() {
        let f = Fixture::new();
        let mut m = Model::new(f.0.join("A"));
        wait(&mut m);
        m.show_hidden = true;
        m.rebuild();
        select(&mut m, ".hidden", false);
        let a = m.tabs.active;
        m.tab_command(Command::New);
        m.show_hidden = false;
        m.tab_command(Command::Select(a));
        wait(&mut m);
        m.rebuild();
        m.pump();
        m.rebuild();
        assert!(m.selected.is_none() && m.selection.is_empty() && m.anchor.is_none());
        m.open_copy();
        assert!(m.copy_form.is_none());
        m.notice_open = false;
        m.open_rename(crate::rename_dialog::Mode::Rename);
        assert!(m.rename_dialog.is_none());
        crate::input(&mut m, "same");
        assert_eq!(m.selection.len(), 1);
        assert_eq!(m.entries[m.selected.unwrap()].name, "same.txt");
        assert_eq!(m.path.as_path(), f.0.join("A").as_path());
    }
    #[test]
    fn undo_a_after_failed_create_b_keeps_b_selection_and_refresh_generation() {
        let f = Fixture::new();
        let a = f.0.join("A");
        let b = f.0.join("B");
        let mut m = Model::new(a.clone());
        wait(&mut m);
        let aid = m.tabs.active;
        let prepared = files_core::rename_transaction::PreparedRename::capture(
            &a,
            vec![("same.txt".into(), "renamed.txt".into())],
        )
        .unwrap();
        assert!(
            m.mutations
                .submit(crate::rename_job::Request::Apply { id: 1, prepared })
                .is_ok()
        );
        m.mutation_id = 1;
        m.mutation_applied = true;
        let deadline = Instant::now() + Duration::from_secs(10);
        let drain = |m: &mut Model| {
            while !m.mutations_drained() {
                m.pump();
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        drain(&mut m);
        assert!(m.can_undo);
        m.tab_command(Command::New);
        m.navigate(b.clone());
        wait(&mut m);
        m.open_rename(crate::rename_dialog::Mode::File);
        m.rename_dialog.as_mut().unwrap().fields[0] = "same.txt".into();
        m.apply_rename();
        drain(&mut m);
        m.notice_open = false;
        wait(&mut m);
        assert!(m.can_undo);
        m.query = "same".into();
        m.rebuild();
        select(&mut m, "same.txt", false);
        let generation = m.generation;
        m.undo_rename(false);
        drain(&mut m);
        assert_eq!(m.generation, generation);
        assert_eq!(m.path, b);
        assert_eq!(m.query, "same");
        assert_eq!(m.entries[m.selected.unwrap()].name, "same.txt");
        assert_eq!(fs::read(a.join("same.txt")).unwrap(), b"A");
        assert_eq!(fs::read(b.join("same.txt")).unwrap(), b"B");
        assert!(!a.join("renamed.txt").exists());
        m.tab_command(Command::Select(aid));
        wait(&mut m);
        assert!(m.entries.iter().any(|e| e.name == "same.txt"));
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use std::{
        fs,
        time::{Duration, Instant},
    };
    #[test]
    fn copy_b_finishes_while_a_active_without_refreshing_a_then_b_reloads_and_last_close_drains() {
        let base = files_core::theme::home()
            .join(".cache/files-test")
            .join(format!(
                "tab-copy-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        let a = base.join("A");
        let b = base.join("B");
        let src = base.join("source");
        for p in [&a, &b, &src] {
            fs::create_dir_all(p).unwrap();
        }
        fs::write(a.join("same.txt"), b"A").unwrap();
        fs::write(b.join("same.txt"), b"B").unwrap();
        fs::write(src.join("copied.txt"), b"C").unwrap();
        let mut m = Model::new(a.clone());
        let aid = m.tabs.active;
        m.tab_command(Command::New);
        let bid = m.tabs.active;
        m.navigate(b.clone());
        let paths = vec![files_core::copy_engine::CopyRequest {
            source: src.join("copied.txt"),
            destination: b.join("copied.txt"),
        }];
        m.copy
            .enqueue(crate::copy_job::Request::new(1, paths, Default::default()).start_paused())
            .unwrap();
        m.copy_locations.insert(1, b.clone());
        m.tab_command(Command::Select(aid));
        let generation = m.generation;
        let deadline = Instant::now() + Duration::from_secs(10);
        while m.copy.active_id().is_none() {
            m.pump();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        m.copy.resume(1);
        while m.copy_busy() {
            m.pump();
            assert!(Instant::now() < deadline, "{}", m.error);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(m.path, a);
        assert_eq!(m.generation, generation, "copy to B must not refresh A");
        assert_eq!(fs::read(a.join("same.txt")).unwrap(), b"A");
        m.tab_command(Command::Select(bid));
        while !m.loaded {
            m.pump();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(m.entries.iter().any(|e| e.name == "copied.txt"));
        assert_eq!(fs::read(b.join("same.txt")).unwrap(), b"B");
        m.tab_command(Command::Close(aid));
        let paths = vec![files_core::copy_engine::CopyRequest {
            source: src.join("copied.txt"),
            destination: b.join("second.txt"),
        }];
        m.copy
            .enqueue(crate::copy_job::Request::new(2, paths, Default::default()).start_paused())
            .unwrap();
        m.pump();
        m.tab_command(Command::Close(bid));
        assert!(m.closing);
        assert!(!m.close_ready());
        while !m.close_ready() {
            m.pump();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(m.copy.is_closed());
        assert!(!b.join("second.txt").exists());
        drop(m);
        fs::remove_dir_all(base).unwrap();
    }
}

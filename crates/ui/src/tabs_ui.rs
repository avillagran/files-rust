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
            reveal_hidden: self.reveal_hidden,
            reveal_selection: self.external_reveal.as_ref().is_some_and(|reveal| {
                reveal.owner == self.pane_owner() && reveal.generation == self.generation
            }),
            query: self.query.clone(),
            scroll: self.scroll,
            grid_zoom: self.grid_zoom,
            pan_x: self.pan_x,
            charge,
        })
    }
    pub(crate) fn restore_tab(&mut self, saved: Saved) {
        let reveal = saved.reveal_selection.then(|| {
            let mut names: Vec<_> = saved
                .selection
                .names()
                .map(std::ffi::OsStr::to_os_string)
                .collect();
            // Snapshot membership is sorted, but the external request's
            // first focused item must retain priority when a background tab opens.
            if let Some(focus) = saved.selection.focus()
                && let Some(at) = names.iter().position(|name| name == focus)
            {
                names.swap(0, at);
            }
            names
        });
        self.view = saved.view;
        self.load_location(saved.path);
        self.reveal_hidden = saved.reveal_hidden;
        self.folder_restore = None;
        self.history = saved.history;
        self.query = saved.query;
        self.sort = saved.sort;
        self.descending = saved.descending;
        self.grid_zoom = saved.grid_zoom;
        self.pan_x = saved.pan_x;
        self.scroll = saved.scroll;
        self.refresh_selection = Some(saved.selection);
        self.external_reveal = reveal.map(|names| crate::file_manager_ui::Reveal {
            owner: self.pane_owner(),
            generation: self.generation,
            names,
        });
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

    /// Prepare the complete external navigation before publishing any row or
    /// identity. The caller has already performed advisory filesystem preflight.
    pub(crate) fn external_locations(
        &mut self,
        groups: &[files_core::file_manager::Group],
        fresh: bool,
    ) -> Result<Vec<u64>, String> {
        use files_core::{file_manager::MAX_URIS, navigation_history::History};
        if crate::tab_gestures::blocked(self) {
            return Err(
                "Finish the current edit, dialog or operation before showing locations".into(),
            );
        }
        if groups.is_empty() || groups.len() > MAX_URIS {
            return Err("External location count is outside the supported range".into());
        }
        let active = self.tabs.active;
        let old_index = self
            .tabs
            .index(active)
            .ok_or("The active tab is unavailable")?;
        if self.tabs.rows[old_index].path.as_os_str() != self.path.as_os_str()
            || self.tabs.rows.iter().enumerate().any(|(i, tab)| {
                self.tabs.rows[..i].iter().any(|prior| prior.id == tab.id)
                    || (tab.id == active) != tab.saved.is_none()
                    || tab
                        .saved
                        .as_ref()
                        .is_some_and(|saved| saved.path.as_os_str() != tab.path.as_os_str())
            })
        {
            return Err("Tab identity or stored location is inconsistent".into());
        }
        if fresh
            && (self.tabs.rows.len() != 1
                || active != 1
                || !self.tabs.closed.is_empty()
                || self.dual.enabled)
        {
            return Err("A fresh request requires an untouched placeholder tab".into());
        }
        let clone_history = |history: &History| {
            let (back, forward) = history.paths();
            History::restore(back.clone(), forward.clone()).map_err(str::to_string)
        };
        let mut prepared = Vec::with_capacity(groups.len());
        let mut released = 0usize;
        for (i, group) in groups.iter().enumerate() {
            files_core::navigation_history::validate(&group.directory)?;
            if groups[..i]
                .iter()
                .any(|g| g.directory.as_os_str() == group.directory.as_os_str())
            {
                return Err("External location groups must have distinct directories".into());
            }
            if group.names.len() > MAX_URIS
                || group.names.iter().any(|name| {
                    name.is_empty()
                        || name == "."
                        || name == ".."
                        || name.len() > 4096
                        || name
                            .as_encoded_bytes()
                            .iter()
                            .any(|b| matches!(b, 0 | b'/'))
                })
            {
                return Err("External selection names are invalid or oversized".into());
            }
            let id = if fresh {
                (i == 0).then_some(active)
            } else if self.path.as_os_str() == group.directory.as_os_str() {
                Some(active)
            } else {
                self.tabs
                    .rows
                    .iter()
                    .find(|tab| tab.path.as_os_str() == group.directory.as_os_str())
                    .map(|tab| tab.id)
            };
            let (history, view, sort, descending, grid_zoom, retained_hidden) =
                if !fresh && id == Some(active) {
                    (
                        clone_history(&self.history)?,
                        self.view,
                        self.sort,
                        self.descending,
                        self.grid_zoom,
                        self.reveal_hidden,
                    )
                } else if let Some(saved) = id.filter(|_| !fresh).and_then(|id| {
                    self.tabs.rows[self.tabs.index(id).expect("live target")]
                        .saved
                        .as_ref()
                }) {
                    released = released
                        .checked_add(saved.charge)
                        .ok_or("Tab state budget overflow")?;
                    (
                        clone_history(&saved.history)?,
                        saved.view,
                        saved.sort,
                        saved.descending,
                        saved.grid_zoom,
                        saved.reveal_hidden,
                    )
                } else {
                    (
                        History::default(),
                        self.view,
                        self.sort,
                        self.descending,
                        self.grid_zoom,
                        false,
                    )
                };
            let focus = group.names.first().map(std::ffi::OsString::as_os_str);
            let selection = Snapshot::capture_with_anchor(
                group.names.iter().map(std::ffi::OsString::as_os_str),
                focus,
                focus,
                tabs::SELECTION_LIMIT,
            )?;
            let charge = selection
                .retained_bytes()
                .checked_add(history.retained_bytes())
                .and_then(|n| n.checked_add(group.directory.as_os_str().len()))
                .and_then(|n| n.checked_add(8192))
                .filter(|&n| n <= tabs::SNAPSHOT_LIMIT)
                .ok_or("External tab snapshot exceeds the retained-state limit")?;
            let saved = Saved {
                path: group.directory.clone(),
                history,
                selection,
                view: if matches!(
                    view,
                    crate::model::View::SpaceMap | crate::model::View::Columns
                ) && !group.names.is_empty()
                {
                    crate::model::View::List
                } else {
                    view
                },
                sort,
                descending,
                reveal_hidden: retained_hidden
                    || group
                        .names
                        .iter()
                        .any(|n| n.as_encoded_bytes().first() == Some(&b'.')),
                reveal_selection: true,
                query: String::new(),
                scroll: 0,
                grid_zoom,
                pan_x: 0.,
                charge,
            };
            prepared.push((id, saved));
        }
        let new_count = prepared.iter().filter(|(id, _)| id.is_none()).count();
        if self
            .tabs
            .rows
            .len()
            .checked_add(new_count)
            .is_none_or(|n| n > tabs::MAX_TABS)
        {
            return Err("The complete request does not fit within eight tabs".into());
        }
        self.tabs.check_new_ids(new_count)?;
        let preserve_outgoing = !fresh && !prepared.iter().any(|(id, _)| *id == Some(active));
        let outgoing_charge = if preserve_outgoing {
            self.tab_charge()?
        } else {
            0
        };
        let stored = self
            .tabs
            .rows
            .iter()
            .filter_map(|tab| tab.saved.as_ref())
            .chain(self.tabs.closed.iter())
            .try_fold(0usize, |sum, saved| sum.checked_add(saved.charge))
            .ok_or("Tab state budget overflow")?;
        let prepared_stored = prepared
            .iter()
            .skip(1)
            .try_fold(0usize, |sum, (_, saved)| sum.checked_add(saved.charge))
            .ok_or("Tab state budget overflow")?;
        if stored
            .checked_sub(released)
            .and_then(|n| n.checked_add(prepared_stored))
            .and_then(|n| n.checked_add(outgoing_charge))
            .and_then(|n| n.checked_add(self.dual.other_stored()))
            .is_none_or(|n| n > tabs::STORED_LIMIT)
        {
            return Err("The complete request exceeds the shared tab-state budget".into());
        }
        let mut ids = Vec::new();
        ids.try_reserve_exact(prepared.len())
            .map_err(|_| "No memory to retain external tab identities")?;
        self.tabs
            .rows
            .try_reserve_exact(new_count)
            .map_err(|_| "No memory to publish external tab rows")?;
        // All requested snapshots, paths, names and charges are ready before
        // save admission or active snapshot capture can change app state.
        if !fresh && !self.save_layout() {
            return Err(if self.error.is_empty() {
                "The current layout could not be saved".into()
            } else {
                self.error.clone()
            });
        }
        let outgoing = if preserve_outgoing {
            Some(self.capture_tab(outgoing_charge)?)
        } else {
            None
        };
        if let Some(outgoing) = outgoing {
            self.tabs.rows[old_index].saved = Some(outgoing);
        }
        let mut foreground = None;
        for (i, (existing, saved)) in prepared.into_iter().enumerate() {
            let id = existing.unwrap_or_else(|| self.tabs.fresh_id());
            let path = saved.path.clone();
            let saved = if i == 0 {
                foreground = Some(saved);
                None
            } else {
                Some(saved)
            };
            if let Some(index) = self.tabs.index(id) {
                self.tabs.rows[index].path = path;
                self.tabs.rows[index].saved = saved;
            } else {
                self.tabs.rows.push(Tab { id, path, saved });
            }
            ids.push(id);
        }
        self.session.restore_allowed = false;
        self.tabs.active = ids[0];
        self.tabs.first = self.tabs.index(ids[0]).expect("foreground was published");
        self.restore_tab(foreground.expect("nonempty prepared request"));
        self.hits.clear();
        Ok(ids)
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
                    reveal_hidden: outgoing.reveal_hidden,
                    reveal_selection: false,
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
                reveal_hidden: false,
                reveal_selection: false,
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
            let base = std::env::var_os("TMPDIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/var/tmp/files-ui-tab-fixtures"));
            if !base.exists() {
                let mut create = fs::DirBuilder::new();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    create.mode(0o700);
                }
                create.create(&base).unwrap();
            }
            let metadata = fs::symlink_metadata(&base).unwrap();
            assert!(metadata.is_dir() && !metadata.file_type().is_symlink());
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(metadata.permissions().mode() & 0o777, 0o700);
            }
            let root = base.join(format!(
                "tabs-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let mut create = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                create.mode(0o700);
            }
            create.create(&root).unwrap();
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
    // Owned fixture trees are retained; tests never remove or follow links
    // during cleanup.
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
    fn external_group(path: PathBuf, names: &[&str]) -> files_core::file_manager::Group {
        files_core::file_manager::Group {
            directory: path,
            names: names.iter().map(std::ffi::OsString::from).collect(),
        }
    }
    fn charged_snapshot(path: PathBuf, charge: usize) -> Saved {
        Saved {
            path,
            history: Default::default(),
            selection: Snapshot::capture(std::iter::empty(), None).unwrap(),
            view: crate::model::View::List,
            sort: files_core::listing::Sort::Name,
            descending: false,
            reveal_hidden: false,
            reveal_selection: false,
            query: String::new(),
            scroll: 0,
            grid_zoom: 1.,
            pan_x: 0.,
            charge,
        }
    }
    #[test]
    #[cfg(unix)]
    fn external_fresh_request_reuses_only_placeholder_and_preserves_multiple_raw_names() {
        use std::os::unix::ffi::OsStringExt;
        let f = Fixture::new();
        let raw = std::ffi::OsString::from_vec(b"raw-\xff".to_vec());
        fs::write(f.0.join("B").join(&raw), b"raw fixture").unwrap();
        let mut m = Model::new(f.0.join("A"));
        wait(&mut m);
        m.history
            .visit(&f.0.join("discarded-placeholder-history"), &f.0.join("A"))
            .unwrap();
        m.view = crate::model::View::SpaceMap;
        let preference = m.show_hidden;
        let config_dirty = m.config_dirty;
        let groups = [
            files_core::file_manager::Group {
                directory: f.0.join("B"),
                names: vec![raw.clone(), ".hidden".into()],
            },
            external_group(f.0.join("A"), &["same.txt"]),
        ];
        let ids = m.external_locations(&groups, true).unwrap();
        assert_eq!(ids, [1, 2]);
        assert_eq!(tab_ids(&m), [1, 2]);
        assert_eq!(m.path, f.0.join("B"));
        assert!(m.history.paths().0.is_empty());
        assert!(m.tabs.closed.is_empty());
        assert_eq!(m.tabs.rows[1].path, f.0.join("A"));
        assert!(m.view == crate::model::View::List);
        assert!(m.reveal_hidden);
        assert_eq!(m.show_hidden, preference);
        assert_eq!(m.config_dirty, config_dirty);
        let snapshot = m.refresh_selection.as_ref().unwrap();
        assert!(snapshot.contains(&raw) && snapshot.contains(std::ffi::OsStr::new(".hidden")));
        assert_eq!(snapshot.focus(), Some(raw.as_os_str()));
        assert_eq!(snapshot.anchor(), Some(raw.as_os_str()));
        assert_eq!(
            m.external_reveal.as_ref().unwrap().names,
            vec![raw.clone(), ".hidden".into()]
        );
        wait(&mut m);
        let selected: std::collections::HashSet<_> = m
            .selection
            .iter()
            .map(|&i| m.entries[i].name.clone())
            .collect();
        assert_eq!(
            selected,
            [raw.clone(), ".hidden".into()].into_iter().collect()
        );
        assert_eq!(m.entries[m.selected.unwrap()].name, raw);
    }
    #[test]
    fn external_targets_reuse_stable_ids_active_preference_history_and_style() {
        use crate::model::View;
        use files_core::listing::Sort;
        let f = Fixture::new();
        let mut m = Model::new(f.0.join("A"));
        wait(&mut m);
        m.history
            .visit(&f.0.join("A-history"), &f.0.join("A"))
            .unwrap();
        m.view = View::Grid;
        m.sort = Sort::Modified;
        m.descending = true;
        m.grid_zoom = 1.4;
        let active = m.tabs.active;
        let b = m.background_tab(&f.0.join("B"), None).unwrap();
        let duplicate = m.background_tab(&f.0.join("A"), None).unwrap();
        m.tabs.move_before(duplicate, Some(active));
        let b_index = m.tabs.index(b).unwrap();
        let saved = m.tabs.rows[b_index].saved.as_mut().unwrap();
        saved
            .history
            .visit(&f.0.join("B-history"), &f.0.join("B"))
            .unwrap();
        saved.view = View::Columns;
        saved.sort = Sort::Size;
        saved.reveal_hidden = true;
        let ids = m
            .external_locations(
                &[
                    external_group(f.0.join("B"), &["other.txt"]),
                    external_group(f.0.join("A"), &["same.txt", "other.txt"]),
                ],
                false,
            )
            .unwrap();
        assert_eq!(ids, [b, active]);
        assert_eq!(m.tabs.rows.len(), 3);
        assert_eq!(m.path, f.0.join("B"));
        assert!(m.view == View::List);
        assert_eq!(m.sort, Sort::Size);
        assert!(m.reveal_hidden);
        assert_eq!(m.history.paths().0.back(), Some(&f.0.join("B-history")));
        let a = m.tabs.rows[m.tabs.index(active).unwrap()]
            .saved
            .as_ref()
            .unwrap();
        assert_eq!(a.history.paths().0.back(), Some(&f.0.join("A-history")));
        assert!(a.view == View::Grid);
        assert_eq!(a.sort, Sort::Modified);
        assert!(a.descending);
        assert_eq!(a.grid_zoom, 1.4);
        assert!(a.query.is_empty());
        assert!(a.reveal_selection);
        assert!(
            a.selection.contains(std::ffi::OsStr::new("same.txt"))
                && a.selection.contains(std::ffi::OsStr::new("other.txt"))
        );
        assert!(
            m.tabs.rows[m.tabs.index(duplicate).unwrap()]
                .saved
                .as_ref()
                .unwrap()
                .selection
                .names()
                .next()
                .is_none()
        );
        assert_eq!(
            m.background_tab(&f.0.join("C"), None).unwrap(),
            duplicate + 1
        );
    }
    #[test]
    fn external_untargeted_active_and_other_pane_keep_exact_state() {
        let f = Fixture::new();
        let mut m = Model::new(f.0.join("A"));
        wait(&mut m);
        m.toggle_dual();
        let until = Instant::now() + Duration::from_secs(10);
        while !m.dual.other.as_ref().unwrap().loaded {
            m.pump();
            assert!(Instant::now() < until);
            std::thread::yield_now();
        }
        select(&mut m, "same.txt", false);
        select(&mut m, "other.txt", true);
        m.query = "retained query".into();
        m.reveal_hidden = true;
        m.history.visit(&f.0.join("old-A"), &f.0.join("A")).unwrap();
        let active = m.tabs.active;
        let names = m
            .selection
            .iter()
            .map(|&i| m.entries[i].name.clone())
            .collect::<Vec<_>>();
        let b = m.dual.other.as_ref().unwrap();
        let other = (
            b.id,
            b.revision,
            b.path.clone(),
            b.query.clone(),
            b.selection.clone(),
            b.tabs.active,
            b.tabs.rows.iter().map(|t| t.id).collect::<Vec<_>>(),
            b.reveal_hidden,
        );
        m.external_locations(&[external_group(f.0.join("B"), &["other.txt"])], false)
            .unwrap();
        let outgoing = m.tabs.rows[m.tabs.index(active).unwrap()]
            .saved
            .as_ref()
            .unwrap();
        assert_eq!(outgoing.query, "retained query");
        assert!(outgoing.reveal_hidden);
        assert_eq!(outgoing.history.paths().0.back(), Some(&f.0.join("old-A")));
        assert!(names.iter().all(|name| outgoing.selection.contains(name)));
        let b = m.dual.other.as_ref().unwrap();
        assert_eq!(
            other,
            (
                b.id,
                b.revision,
                b.path.clone(),
                b.query.clone(),
                b.selection.clone(),
                b.tabs.active,
                b.tabs.rows.iter().map(|t| t.id).collect::<Vec<_>>(),
                b.reveal_hidden
            )
        );
    }
    #[test]
    fn external_capacity_rejection_consumes_no_rows_or_ids() {
        let f = Fixture::new();
        let mut m = Model::new(f.0.join("A"));
        wait(&mut m);
        for i in 0..6 {
            m.background_tab(&f.0.join(format!("existing-{i}")), None)
                .unwrap();
        }
        let ids = tab_ids(&m);
        let active = m.tabs.active;
        let generation = m.generation;
        let path = m.path.clone();
        assert!(
            m.external_locations(
                &[
                    external_group(f.0.join("new-1"), &[]),
                    external_group(f.0.join("new-2"), &[])
                ],
                false
            )
            .is_err()
        );
        assert_eq!(tab_ids(&m), ids);
        assert_eq!(m.tabs.active, active);
        assert_eq!(m.generation, generation);
        assert_eq!(m.path, path);
        assert_eq!(
            m.background_tab(&f.0.join("after-rejection"), None)
                .unwrap(),
            8
        );
    }
    #[test]
    fn external_shared_budget_rejection_is_atomic_before_active_capture() {
        let f = Fixture::new();
        let mut m = Model::new(f.0.join("A"));
        wait(&mut m);
        select(&mut m, "same.txt", false);
        let existing = m.background_tab(&f.0.join("B"), None).unwrap();
        let stored = m.tabs.stored_bytes();
        for charge in [
            tabs::SNAPSHOT_LIMIT,
            tabs::SNAPSHOT_LIMIT,
            tabs::SNAPSHOT_LIMIT - stored - 1,
        ] {
            m.tabs
                .closed
                .push(charged_snapshot(f.0.join("closed"), charge));
        }
        let ids = tab_ids(&m);
        let selected = m.selected;
        let selection = m.selection.clone();
        let generation = m.generation;
        assert!(
            m.external_locations(
                &[
                    external_group(f.0.join("C"), &[]),
                    external_group(f.0.join("D"), &[])
                ],
                false
            )
            .is_err()
        );
        assert_eq!(tab_ids(&m), ids);
        assert_eq!(m.selected, selected);
        assert_eq!(m.selection, selection);
        assert_eq!(m.generation, generation);
        assert!(
            m.tabs.rows[m.tabs.index(m.tabs.active).unwrap()]
                .saved
                .is_none()
        );
        m.tabs.closed.clear();
        assert_eq!(
            m.background_tab(&f.0.join("after-rejection"), None)
                .unwrap(),
            existing + 1
        );
    }
    #[test]
    fn external_bad_later_name_rejects_before_rows_history_or_ids_change() {
        let f = Fixture::new();
        let mut m = Model::new(f.0.join("A"));
        wait(&mut m);
        m.history.visit(&f.0.join("old"), &f.0.join("A")).unwrap();
        let generation = m.generation;
        assert!(
            m.external_locations(
                &[
                    external_group(f.0.join("B"), &["same.txt"]),
                    external_group(f.0.join("C"), &["bad/name"])
                ],
                false
            )
            .is_err()
        );
        assert_eq!(tab_ids(&m), [1]);
        assert_eq!(m.history.paths().0.back(), Some(&f.0.join("old")));
        assert_eq!(m.generation, generation);
        assert_eq!(m.background_tab(&f.0.join("B"), None).unwrap(), 2);
    }
    #[test]
    fn external_other_pane_budget_is_reserved_before_any_foreground_publication() {
        let f = Fixture::new();
        let mut m = Model::new(f.0.join("A"));
        wait(&mut m);
        m.toggle_dual();
        let baseline = m.dual.other_stored();
        for charge in [
            tabs::SNAPSHOT_LIMIT,
            tabs::SNAPSHOT_LIMIT,
            tabs::SNAPSHOT_LIMIT - baseline,
        ] {
            m.dual
                .other
                .as_mut()
                .unwrap()
                .tabs
                .closed
                .push(charged_snapshot(f.0.join("other-closed"), charge));
        }
        assert_eq!(m.dual.other_stored(), tabs::STORED_LIMIT);
        m.history.visit(&f.0.join("old"), &f.0.join("A")).unwrap();
        let generation = m.generation;
        assert!(
            m.external_locations(&[external_group(f.0.join("B"), &["same.txt"])], false)
                .is_err()
        );
        assert_eq!(tab_ids(&m), [1]);
        assert_eq!(m.tabs.active, 1);
        assert_eq!(m.generation, generation);
        assert_eq!(m.history.paths().0.back(), Some(&f.0.join("old")));
        assert_eq!(m.dual.other_stored(), tabs::STORED_LIMIT);
        m.dual.other.as_mut().unwrap().tabs.closed.clear();
        assert_eq!(m.background_tab(&f.0.join("B"), None).unwrap(), 2);
    }
    #[test]
    fn external_pending_reveal_survives_switch_away_and_return() {
        let f = Fixture::new();
        let mut m = Model::new(f.0.join("A"));
        wait(&mut m);
        let ids = m
            .external_locations(
                &[
                    external_group(f.0.join("B"), &["other.txt"]),
                    external_group(f.0.join("A"), &["same.txt"]),
                ],
                false,
            )
            .unwrap();
        m.tab_command(Command::Select(ids[1]));
        let saved = m.tabs.rows[m.tabs.index(ids[0]).unwrap()]
            .saved
            .as_ref()
            .unwrap();
        assert!(saved.reveal_selection);
        m.tab_command(Command::Select(ids[0]));
        let reveal = m.external_reveal.as_ref().unwrap();
        assert_eq!(reveal.owner, m.pane_owner());
        assert_eq!(reveal.generation, m.generation);
        assert_eq!(reveal.names, [std::ffi::OsString::from("other.txt")]);
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

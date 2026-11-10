//! Browser keyboard commands that are plain model operations, so the window's
//! key dispatcher stays thin and each binding is unit-tested without a display.
use crate::{
    model::{Action, Model, View},
    places_sidebar::{Kind, Pick, Rail},
    shortcuts::{self, Cmd},
    tabs::Command,
};
use fltk::enums::{Key, Shortcut};
use std::path::PathBuf;

/// Enter on a marked batch opens at most this many items at once.
pub const MAX_OPEN_MARKED: usize = 20;

/// Keyboard focus targets the window can move to after an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusTarget {
    Rail,
    Tabs,
}

/// Tab chords besides Ctrl+Tab: Ctrl+PgUp/PgDn previous/next and Alt+1..9
/// jump to that tab (Alt+9 is always the last one, as in browsers).
pub fn tab_chord(m: &Model, key: Key, mods: Shortcut) -> Option<Command> {
    let ctrl = mods.intersects(Shortcut::Ctrl | Shortcut::Meta);
    let alt = mods.intersects(Shortcut::Alt);
    let shift = mods.intersects(Shortcut::Shift);
    if key != Key::Tab {
        if shortcuts::is(m.shortcuts, Cmd::TabNext, key, mods) {
            return Some(Command::Cycle(1));
        }
        if shortcuts::is(m.shortcuts, Cmd::TabPrev, key, mods) {
            return Some(Command::Cycle(-1));
        }
    }
    if alt && !ctrl && !shift {
        let n = (1..=9u8).find(|n| key == Key::from_char((b'0' + n) as char))?;
        let rows = &m.tabs.rows;
        let tab = if n == 9 {
            rows.last()
        } else {
            rows.get(usize::from(n) - 1)
        }?;
        return Some(Command::Select(tab.id));
    }
    None
}

/// F1 or Ctrl+? (Ctrl+Shift+/ on US layouts, or whatever key types "?").
pub fn help_chord(profile: shortcuts::Profile, key: Key, mods: Shortcut, text: &str) -> bool {
    shortcuts::is(profile, Cmd::Help, key, mods)
        || (mods.intersects(Shortcut::Ctrl | Shortcut::Meta) && text == "?")
}

/// The dedicated chord that moves keyboard focus between the list and the
/// places rail from anywhere, dual-pane mode included.
pub fn rail_chord(profile: shortcuts::Profile, key: Key, mods: Shortcut) -> bool {
    shortcuts::is(profile, Cmd::FocusRail, key, mods)
}

/// What a key does while the places rail holds keyboard focus. The toolkit
/// Sidebar already moves its cursor (arrows, Home/End, PgUp/PgDn), activates
/// rows (Enter, Space), opens the row menu (Menu, Shift+F10) and runs a row's
/// trailing action (Delete). This covers the keys it leaves to the window.
#[derive(Debug)]
pub enum RailKey {
    /// Give keyboard focus back to the file list.
    ReturnToList,
    /// Run this rail pick (new tab, reorder, remove bookmark).
    Pick(Pick),
    /// Open this folder in a background tab.
    Background(PathBuf),
    /// Give focus back to the list and let the list handle the chord.
    Pass,
    /// Hand keyboard focus on to the tab strip (Nautilus F6 cycle).
    ToTabs,
    /// Swallow the key: it would act on the file list behind the rail.
    Consume,
}

/// Chords that keep working while the rail has focus. Everything else that
/// the rail does not use is swallowed so a key such as Delete or F2 never
/// reaches the file selection the user is not looking at.
pub fn rail_passthrough(profile: shortcuts::Profile, key: Key, mods: Shortcut) -> bool {
    // Non-destructive window commands of the active profile (Nautilus F5
    // reload, Ctrl+N new window, F3 dual pane...) also reach the list.
    if matches!(
        shortcuts::resolve(profile, key, mods),
        Some(
            Cmd::Refresh
                | Cmd::NewWindow
                | Cmd::RailToggle
                | Cmd::DualPane
                | Cmd::Search
                | Cmd::Filter
                | Cmd::Hidden
                | Cmd::Location
                | Cmd::Commands
        )
    ) {
        return true;
    }
    let ctrl = mods.intersects(Shortcut::Ctrl | Shortcut::Meta);
    let alt = mods.intersects(Shortcut::Alt);
    let shift = mods.intersects(Shortcut::Shift);
    let char_is = |c: char| key == Key::from_char(c);
    if key == Key::F1 || key == Key::F9 || key == Key::F10 && !shift && !ctrl {
        return true;
    }
    if ctrl && !alt {
        return char_is('t')
            || char_is('w')
            || char_is('l')
            || char_is('h')
            || char_is('r')
            || char_is('f')
            || char_is(',')
            || (char_is('p') && shift)
            || matches!(key, Key::Tab | Key::PageUp | Key::PageDown);
    }
    if alt && !ctrl && !shift {
        return matches!(key, Key::Left | Key::Right | Key::Home)
            || (1..=9u8).any(|n| char_is((b'0' + n) as char));
    }
    false
}

/// Folder a rail row can open in a tab: places, bookmarks, Ofolder
/// favorites, network locations and mounted devices.
pub fn rail_folder(rail: &Rail, i: usize) -> Option<PathBuf> {
    rail.drop_dir(i).map(|p| p.to_path_buf())
}

pub fn rail_key(m: &Model, rail: &Rail, cursor: usize, key: Key, mods: Shortcut) -> RailKey {
    let ctrl = mods.intersects(Shortcut::Ctrl | Shortcut::Meta);
    let alt = mods.intersects(Shortcut::Alt);
    let shift = mods.intersects(Shortcut::Shift);
    if (key == Key::Escape && !ctrl && !alt) || rail_chord(m.shortcuts, key, mods) {
        return RailKey::ReturnToList;
    }
    if shortcuts::is(m.shortcuts, Cmd::FocusCycle, key, mods) {
        return RailKey::ToTabs;
    }
    if matches!(key, Key::Enter | Key::KPEnter) && !alt && (ctrl || shift) {
        return match rail_folder(rail, cursor) {
            Some(path) if ctrl => RailKey::Pick(Pick::Run(vec![
                Action::Tab(Command::New),
                Action::Navigate(path),
            ])),
            Some(path) => RailKey::Background(path),
            None => RailKey::Consume,
        };
    }
    if matches!(key, Key::Up | Key::Down) && alt && !ctrl && !shift {
        // Same rule as dragging inside "Marcadores": only Files' own bookmarks
        // move, never Ofolder rows or another section from the keyboard.
        let Some(item) = rail
            .items
            .get(cursor)
            .filter(|i| i.key.starts_with("mark:"))
        else {
            return RailKey::Consume;
        };
        let keys = crate::places_sidebar::movable_keys(rail, "h:marks");
        let Some(from) = keys.iter().position(|k| *k == item.key) else {
            return RailKey::Consume;
        };
        let to = if key == Key::Up {
            from.checked_sub(1)
        } else {
            (from + 1 < keys.len()).then_some(from + 1)
        };
        return to
            .and_then(|to| crate::places_sidebar::drag_pick(m, rail, &item.key, "h:marks", to))
            .map_or(RailKey::Consume, RailKey::Pick);
    }
    if key == Key::Delete && !ctrl && !alt && !shift {
        return match rail.kinds.get(cursor) {
            Some(Kind::Bookmark(path)) => RailKey::Pick(Pick::RemoveBookmark(path.clone())),
            _ => RailKey::Consume,
        };
    }
    if rail_passthrough(m.shortcuts, key, mods) {
        return RailKey::Pass;
    }
    RailKey::Consume
}

/// Whether the marked batch is only the focused row (or nothing): Enter and
/// Esc keep their single-item behaviour then.
fn focus_only(m: &Model) -> bool {
    m.selection.is_empty()
        || (m.selection.len() == 1 && m.selected.is_some_and(|i| m.selection.contains(&i)))
}

impl Model {
    /// `/` or `~` typed in the list with no filter or edit open starts the
    /// location field seeded with that character, cursor at its end.
    pub fn seed_location(&mut self, text: &str) -> bool {
        if !matches!(text, "/" | "~")
            || self.editing
            || self.filter_edit.active
            || !self.query.is_empty()
            || self.settings
            || self.actions_menu
            || self.context_menu.is_some()
            || self.breadcrumbs.open
        {
            return false;
        }
        self.begin_path_edit();
        if !self.editing {
            return true;
        }
        if let Err(error) = self.path_input(text) {
            self.error = error.into();
        }
        true
    }

    /// Ctrl+Enter (foreground) or Shift+Enter (background) on the focused
    /// folder opens it in a new tab through the middle-click path. A file
    /// opens as with Enter.
    pub fn open_focused_in_tab(&mut self, foreground: bool) {
        let Some(i) = self.selected else { return };
        if !self.entries.get(i).is_some_and(|e| e.browses_as_dir()) {
            self.open_selected();
            return;
        }
        if let Some(why) = crate::map_actions::reason(self, &Action::Open) {
            self.error = format!("Espacio: {why}");
            return;
        }
        if crate::tab_gestures::blocked(self) {
            return;
        }
        match self.background_folder_tab(&Action::Entry(i)) {
            Ok(Some(id)) => {
                self.session.restore_allowed = false;
                if foreground {
                    self.tab_command(Command::Select(id));
                } else {
                    self.tab_feedback = Some(std::time::Instant::now());
                }
            }
            Ok(None) => {}
            Err(error) => self.operation_message = Some(error.into()),
        }
    }

    /// Enter on a marked batch beyond the focused row: marked folders open
    /// in background tabs and marked files with their applications, one
    /// launcher at a time. Returns false when Enter keeps its usual meaning.
    pub fn open_marked(&mut self) -> bool {
        if focus_only(self) || self.selection_refreshing() {
            return false;
        }
        if let Some(why) = crate::map_actions::reason(self, &Action::Open) {
            self.error = format!("Espacio: {why}");
            return true;
        }
        let hidden = self.hidden_visible();
        let marked: Vec<usize> = self
            .order
            .iter()
            .copied()
            .filter(|i| self.selection.contains(i))
            .filter(|&i| {
                self.entries
                    .get(i)
                    .is_some_and(|e| files_core::visibility::accepts(&e.name, hidden))
            })
            .collect();
        if marked.is_empty() {
            return false;
        }
        if marked.len() > MAX_OPEN_MARKED {
            self.error = format!(
                "Hay {} elementos marcados; Enter abre como máximo {MAX_OPEN_MARKED} a la vez",
                marked.len()
            );
            return true;
        }
        let (dirs, files): (Vec<usize>, Vec<usize>) = marked
            .into_iter()
            .partition(|&i| self.entries[i].browses_as_dir());
        let join = |m: &Model, i: usize| m.path.join(&m.entries[i].name);
        let dirs: Vec<PathBuf> = dirs.into_iter().map(|i| join(self, i)).collect();
        let files: Vec<PathBuf> = files.into_iter().map(|i| join(self, i)).collect();
        if files
            .iter()
            .any(|p| p.as_os_str().as_encoded_bytes().len() > 4096)
        {
            self.error = "Ruta de apertura demasiado larga (máximo 4096 bytes)".into();
            return true;
        }
        if !files.is_empty() && (self.open_job.is_some() || !self.open_queue.is_empty()) {
            self.error = "Ya hay una apertura en curso".into();
            return true;
        }
        if !dirs.is_empty() {
            if crate::tab_gestures::blocked(self) {
                return true;
            }
            match self.background_tabs(dirs.iter().map(|p| (p.as_path(), None))) {
                Ok(_) => {
                    self.session.restore_allowed = false;
                    self.tab_feedback = Some(std::time::Instant::now());
                }
                Err(error) => {
                    self.error = error.into();
                    return true;
                }
            }
        }
        self.error.clear();
        self.open_queue.extend(files);
        self.start_next_open();
        true
    }

    /// Starts the next queued launcher once the previous one finished.
    pub fn start_next_open(&mut self) {
        while self.open_job.is_none() {
            let Some(path) = self.open_queue.pop_front() else {
                return;
            };
            match crate::open_job::Job::start(path) {
                Ok(job) => self.open_job = Some(job),
                Err(error) => self.error = error,
            }
        }
    }

    /// Esc in the browser once nothing else claimed it: an active filter is
    /// cleared first (back to the top), then a marked batch shrinks to the
    /// focused row. With neither, the view and its scroll stay as they are.
    pub fn escape_browser(&mut self) {
        if !self.query.is_empty() {
            self.folder_restore = None;
            self.query.clear();
            self.preserve_empty_selection = false;
            self.scroll = 0;
            self.rebuild();
            self.restore_filter_view();
            return;
        }
        if focus_only(self) || self.selection_refreshing() {
            return;
        }
        self.folder_restore = None;
        self.selection.clear();
        if let Some(i) = self.selected {
            self.selection.insert(i);
        }
        self.anchor = self.selected;
        self.preserve_empty_selection = self.selection.is_empty();
    }

    /// Back to the view type-ahead left (Space), when still in the List it
    /// switched to and in the same folder.
    pub fn restore_filter_view(&mut self) {
        if let Some((view, path)) = self.filter_return_view.take()
            && self.view == View::List
            && path == self.path
            && !self.dual.enabled
        {
            self.action(Action::View(view));
        }
    }

    /// Nautilus F6: list → places rail → tab strip → list. The rail and the
    /// tab strip hand focus on with the same chord; dual mode switches panes.
    pub fn focus_cycle(&mut self) {
        if self.dual.enabled {
            self.switch_dual();
            return;
        }
        self.focus_request = Some(if self.sidebar_widget {
            FocusTarget::Rail
        } else {
            FocusTarget::Tabs
        });
    }

    /// A new Files window on the current folder, through the injected
    /// spawner (a detached process of this binary in the real window).
    pub fn new_window(&mut self) {
        let path = self.path.clone();
        if !path.is_absolute() {
            self.error = "Nueva ventana: la carpeta actual no es una ruta absoluta".into();
            return;
        }
        if let Err(error) = (self.window_spawner)(&path) {
            self.error = error;
        }
    }

    /// Applies a shortcut profile live and persists it with the other
    /// preferences.
    pub fn set_shortcut_profile(&mut self, profile: crate::shortcuts::Profile) {
        if self.shortcuts == profile {
            return;
        }
        self.shortcuts = profile;
        self.settings = false;
        self.hits.clear();
        self.config_revision += 1;
        self.config_dirty = true;
        self.config_error = None;
        self.dispatch_config();
    }

    pub fn go_home(&mut self) {
        self.go_home_at(files_core::theme::home());
    }

    fn go_home_at(&mut self, home: PathBuf) {
        if home.is_absolute() && home != self.path {
            self.navigate(home);
        }
    }

    /// Ctrl+plus/minus/0. Only the Grid (and the legacy spatial layout) has
    /// a zoom; List, Groups, Columns and Space have none.
    pub fn zoom_step(&mut self, step: i8) {
        let factor = match step.signum() {
            1 => 1.15,
            -1 => 1. / 1.15,
            _ => 0.,
        };
        match self.view {
            View::Grid if step == 0 => {
                self.grid_zoom = 1.0;
                self.scroll = 0;
                self.hits.clear();
                self.visible.clear();
            }
            View::Grid => self.action(Action::GridZoom(factor)),
            View::Spatial if step == 0 => {
                if self.layout_ready && self.layout.zoom > 0. {
                    let reset = 1. / self.layout.zoom;
                    self.action(Action::Zoom(reset));
                }
            }
            View::Spatial => self.action(Action::Zoom(factor)),
            _ => {
                let chord = shortcuts::primary(self.shortcuts, Cmd::ViewGrid);
                self.error = if chord.is_empty() {
                    "El zoom sólo existe en la vista Grilla".into()
                } else {
                    format!("El zoom sólo existe en la vista Grilla ({chord})")
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aux_io::AuxIo;
    use files_core::listing::{Entry, Kind as EntryKind};
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    };

    /// A private, retained directory per fixture under the test TMPDIR. The
    /// model never writes inside it here and nothing is deleted afterwards.
    fn model() -> Model {
        static ID: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "files-key-commands-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let mut m = Model::new(root);
        let (tx, _) = mpsc::sync_channel(4);
        let (_, rx) = mpsc::channel();
        m.aux = AuxIo { tx, rx };
        m
    }

    fn entry(name: &str, kind: EntryKind) -> Entry {
        Entry {
            name: name.into(),
            kind,
            metadata: None,
            stat_attempted: false,
            link: None,
        }
    }

    fn listed(m: &mut Model) {
        m.entries = vec![
            entry("a-dir", EntryKind::Directory),
            entry("b-dir", EntryKind::Directory),
            entry("c.txt", EntryKind::File),
        ];
        m.order = vec![0, 1, 2];
        m.loaded = true;
    }

    #[test]
    fn slash_and_tilde_seed_the_location_field_only_when_idle() {
        let mut m = model();
        assert!(m.seed_location("/"));
        assert!(m.editing);
        assert_eq!(m.edit, "/");
        assert!(!m.replace_edit);
        assert_eq!(m.path_edit.cursor.range(&m.edit), 1..1);
        m.end_path_edit();
        assert!(m.seed_location("~"));
        assert_eq!(m.edit, "~");
        // Already editing, other text or an active filter: not a seed.
        assert!(!m.seed_location("/"));
        m.end_path_edit();
        assert!(!m.seed_location("a"));
        m.query = "x".into();
        assert!(!m.seed_location("/"));
        assert!(!m.editing);
    }

    #[test]
    fn tab_chords_cycle_and_jump() {
        let mut m = model();
        let ctrl = Shortcut::Ctrl;
        assert!(matches!(
            tab_chord(&m, Key::PageDown, ctrl),
            Some(Command::Cycle(1))
        ));
        assert!(matches!(
            tab_chord(&m, Key::PageUp, ctrl),
            Some(Command::Cycle(-1))
        ));
        // Ctrl+Shift+PgUp/PgDn keeps moving the tab instead.
        assert!(tab_chord(&m, Key::PageUp, ctrl | Shortcut::Shift).is_none());
        let first = m.tabs.active;
        for _ in 0..2 {
            m.tab_command(Command::New);
        }
        let ids: Vec<u64> = m.tabs.rows.iter().map(|t| t.id).collect();
        assert_eq!(ids.len(), 3);
        let alt = Shortcut::Alt;
        assert!(matches!(
            tab_chord(&m, Key::from_char('1'), alt),
            Some(Command::Select(id)) if id == first
        ));
        assert!(matches!(
            tab_chord(&m, Key::from_char('2'), alt),
            Some(Command::Select(id)) if id == ids[1]
        ));
        assert!(tab_chord(&m, Key::from_char('5'), alt).is_none());
        assert!(matches!(
            tab_chord(&m, Key::from_char('9'), alt),
            Some(Command::Select(id)) if id == ids[2]
        ));
        assert!(tab_chord(&m, Key::from_char('1'), ctrl).is_none());
        m.tab_command(tab_chord(&m, Key::from_char('1'), alt).unwrap());
        assert_eq!(m.tabs.active, first);
        m.tab_command(tab_chord(&m, Key::PageUp, ctrl).unwrap());
        assert_eq!(m.tabs.active, ids[2]);
    }

    #[test]
    fn help_and_rail_chords() {
        let p = shortcuts::Profile::Files;
        assert!(help_chord(
            p,
            Key::from_char('/'),
            Shortcut::Ctrl | Shortcut::Shift,
            ""
        ));
        assert!(help_chord(p, Key::from_char('\''), Shortcut::Ctrl, "?"));
        assert!(!help_chord(p, Key::from_char('/'), Shortcut::Ctrl, "/"));
        assert!(help_chord(p, Key::F1, Shortcut::None, ""));
        assert!(rail_chord(
            p,
            Key::from_char('s'),
            Shortcut::Ctrl | Shortcut::Alt
        ));
        assert!(!rail_chord(p, Key::from_char('s'), Shortcut::Ctrl));
    }

    #[test]
    fn ctrl_and_shift_enter_open_the_focused_folder_in_a_tab() {
        let mut m = model();
        listed(&mut m);
        let path = m.path.clone();
        let first = m.tabs.active;
        m.selected = Some(0);
        m.selection = [0].into();
        m.open_focused_in_tab(false);
        assert_eq!(m.tabs.rows.len(), 2);
        assert_eq!(m.tabs.active, first);
        assert_eq!(m.path, path);
        assert_eq!(m.tabs.rows[1].path, path.join("a-dir"));
        m.selected = Some(1);
        m.open_focused_in_tab(true);
        assert_eq!(m.tabs.rows.len(), 3);
        assert_eq!(m.path, path.join("b-dir"));
        assert_ne!(m.tabs.active, first);
    }

    #[test]
    fn enter_on_a_marked_batch_opens_folders_in_tabs_within_limits() {
        let mut m = model();
        listed(&mut m);
        let path = m.path.clone();
        // Only the focus marked: Enter keeps its usual meaning.
        m.selected = Some(0);
        m.selection = [0].into();
        assert!(!m.open_marked());
        m.selection.clear();
        assert!(!m.open_marked());
        m.selection = [0, 1].into();
        assert!(m.open_marked());
        assert_eq!(m.path, path);
        assert_eq!(m.tabs.rows.len(), 3);
        assert_eq!(m.tabs.rows[1].path, path.join("a-dir"));
        assert_eq!(m.tabs.rows[2].path, path.join("b-dir"));
        assert!(m.open_queue.is_empty() && m.open_job.is_none());
        // Beyond the cap nothing opens and the reason is shown.
        m.entries = (0..=MAX_OPEN_MARKED)
            .map(|i| entry(&format!("d{i}"), EntryKind::Directory))
            .collect();
        m.order = (0..=MAX_OPEN_MARKED).collect();
        m.selection = (0..=MAX_OPEN_MARKED).collect();
        assert!(m.open_marked());
        assert!(m.error.contains("máximo 20"));
        assert_eq!(m.tabs.rows.len(), 3);
        // More folders than free tabs: refused before publishing any tab.
        m.error.clear();
        m.selection = (0..6).collect();
        assert!(m.open_marked());
        assert!(m.error.starts_with("Máximo ocho pestañas"));
        assert_eq!(m.tabs.rows.len(), 3);
    }

    #[test]
    fn escape_clears_filter_then_marks_and_keeps_scroll_otherwise() {
        let mut m = model();
        listed(&mut m);
        m.scroll = 120;
        m.selected = Some(1);
        m.selection = [0, 1, 2].into();
        m.escape_browser();
        assert_eq!(m.selection, [1].into());
        assert_eq!(m.selected, Some(1));
        assert_eq!(m.scroll, 120);
        m.escape_browser();
        assert_eq!(m.selection, [1].into());
        assert_eq!(m.scroll, 120);
        m.query = "dir".into();
        m.escape_browser();
        assert!(m.query.is_empty());
        assert_eq!(m.scroll, 0);
    }

    #[test]
    fn type_ahead_filters_and_escape_restores_the_view_it_left() {
        let mut m = model();
        listed(&mut m);
        // List: typing filters, Esc in the field clears it.
        m.view = View::List;
        m.filter_input("dir").unwrap();
        assert_eq!(m.query, "dir");
        assert!(m.filter_edit.active);
        m.filter_key(Key::Escape, Shortcut::None, "", 0);
        assert!(m.query.is_empty() && !m.filter_edit.active);
        assert!(m.view == View::List);
        // Space cannot filter: type-ahead continues in List, Esc returns.
        m.view = View::SpaceMap;
        m.filter_input("c").unwrap();
        assert!(m.view == View::List);
        assert_eq!(m.query, "c");
        m.filter_key(Key::Escape, Shortcut::None, "", 0);
        assert!(m.view == View::SpaceMap);
        // Leaving the field first (Enter) keeps the filter; the browser Esc
        // then clears it and also returns to Space.
        m.filter_input("c").unwrap();
        m.filter_key(Key::Enter, Shortcut::None, "", 0);
        assert!(m.view == View::List);
        assert_eq!(m.query, "c");
        m.escape_browser();
        assert!(m.query.is_empty());
        assert!(m.view == View::SpaceMap);
        // An explicit view change in between cancels the return.
        m.filter_input("c").unwrap();
        m.action(Action::View(View::Grid));
        m.action(Action::View(View::List));
        m.escape_browser();
        m.end_filter_edit();
        m.action(Action::ClearFilter);
        assert!(m.view == View::List);
    }

    #[test]
    fn sort_menu_lists_criteria_focused_on_the_current_one() {
        let mut m = model();
        listed(&mut m);
        m.action(Action::Sort(files_core::listing::Sort::Size));
        let descending = m.descending;
        m.action(Action::SortMenu(1000, 650));
        let menu = m.context_menu.as_ref().expect("sort menu");
        assert_eq!(menu.target, crate::context_menu::Target::Sort);
        assert_eq!(menu.items.len(), 5);
        assert_eq!(menu.focused, 1);
        assert!(menu.items[1].label.contains("(actual)"));
        // Enter on the reverse row flips the current criterion.
        crate::context_menu::key(&mut m, Key::End);
        crate::context_menu::key(&mut m, Key::Enter);
        assert!(m.context_menu.is_none());
        assert_eq!(m.sort, files_core::listing::Sort::Size);
        assert_ne!(m.descending, descending);
        // Down then Enter picks the next criterion.
        m.action(Action::SortMenu(1000, 650));
        crate::context_menu::key(&mut m, Key::Down);
        crate::context_menu::key(&mut m, Key::Enter);
        assert_eq!(m.sort, files_core::listing::Sort::Modified);
    }

    #[test]
    fn zoom_resets_and_steps_only_where_it_exists() {
        let mut m = model();
        m.view = View::Grid;
        m.action(Action::ZoomStep(1));
        assert!(m.grid_zoom > 1.0);
        m.action(Action::ZoomStep(0));
        assert_eq!(m.grid_zoom, 1.0);
        m.action(Action::ZoomStep(-1));
        assert!(m.grid_zoom < 1.0);
        m.view = View::List;
        let zoom = m.grid_zoom;
        m.action(Action::ZoomStep(0));
        assert_eq!(m.grid_zoom, zoom);
        assert!(m.error.contains("Grilla"));
    }

    #[test]
    fn go_home_navigates_to_an_absolute_home_only() {
        // A private stand-in for $HOME: the test never lists the real one.
        let mut m = model();
        let home = m.path.join("home");
        std::fs::create_dir(&home).unwrap();
        m.go_home_at("relative".into());
        assert_ne!(m.path, home);
        m.go_home_at(home.clone());
        assert_eq!(m.path, home);
    }

    #[test]
    fn focus_actions_request_window_focus() {
        let mut m = model();
        m.action(Action::FocusRail);
        assert_eq!(m.focus_request, Some(FocusTarget::Rail));
        m.action(Action::FocusTabs);
        assert_eq!(m.focus_request, Some(FocusTarget::Tabs));
    }

    #[test]
    fn rail_keys_open_tabs_reorder_and_remove_only_bookmarks() {
        let m = model();
        let mut rail = Rail::default();
        let mut push = |item: fltk_omarchy::widgets::SidebarItem, kind: Kind| {
            rail.items.push(item);
            rail.actions.push(None);
            rail.secondary.push(None);
            rail.kinds.push(kind);
        };
        use fltk_omarchy::widgets::SidebarItem;
        push(SidebarItem::header("Lugares").key("h:places"), Kind::Header);
        push(
            SidebarItem::new("", "Inicio").key("place:home"),
            Kind::Place("/home/u".into()),
        );
        push(
            SidebarItem::header("Marcadores").key("h:marks"),
            Kind::Header,
        );
        push(
            SidebarItem::new("", "A").key("mark:1"),
            Kind::Bookmark("/a".into()),
        );
        push(
            SidebarItem::new("", "B").key("mark:2"),
            Kind::Bookmark("/b".into()),
        );
        push(
            SidebarItem::new("", "O").key("ofav:x"),
            Kind::Shared("/o".into()),
        );
        let none = Shortcut::None;
        let alt = Shortcut::Alt;
        assert!(matches!(
            rail_key(&m, &rail, 1, Key::Escape, none),
            RailKey::ReturnToList
        ));
        assert!(matches!(
            rail_key(&m, &rail, 1, Key::Enter, Shortcut::Ctrl),
            RailKey::Pick(Pick::Run(ref a)) if matches!(a[..], [Action::Tab(Command::New), Action::Navigate(ref p)] if p == std::path::Path::new("/home/u"))
        ));
        assert!(matches!(
            rail_key(&m, &rail, 1, Key::KPEnter, Shortcut::Shift),
            RailKey::Background(ref p) if p == std::path::Path::new("/home/u")
        ));
        assert!(matches!(
            rail_key(&m, &rail, 0, Key::Enter, Shortcut::Ctrl),
            RailKey::Consume
        ));
        assert!(matches!(
            rail_key(&m, &rail, 3, Key::Down, alt),
            RailKey::Pick(Pick::MoveBookmark(ref k, 1)) if k == "mark:1"
        ));
        assert!(matches!(
            rail_key(&m, &rail, 4, Key::Up, alt),
            RailKey::Pick(Pick::MoveBookmark(ref k, 0)) if k == "mark:2"
        ));
        // Edges, Ofolder rows and places do not move from the keyboard.
        assert!(matches!(
            rail_key(&m, &rail, 3, Key::Up, alt),
            RailKey::Consume
        ));
        assert!(matches!(
            rail_key(&m, &rail, 4, Key::Down, alt),
            RailKey::Consume
        ));
        assert!(matches!(
            rail_key(&m, &rail, 5, Key::Up, alt),
            RailKey::Consume
        ));
        assert!(matches!(
            rail_key(&m, &rail, 1, Key::Down, alt),
            RailKey::Consume
        ));
        assert!(matches!(
            rail_key(&m, &rail, 4, Key::Delete, none),
            RailKey::Pick(Pick::RemoveBookmark(ref p)) if p == std::path::Path::new("/b")
        ));
        assert!(matches!(
            rail_key(&m, &rail, 1, Key::Delete, none),
            RailKey::Consume
        ));
        assert!(matches!(
            rail_key(&m, &rail, 5, Key::Delete, none),
            RailKey::Consume
        ));
        // List chords that are safe to forward; destructive keys are swallowed.
        assert!(matches!(
            rail_key(&m, &rail, 1, Key::from_char('t'), Shortcut::Ctrl),
            RailKey::Pass
        ));
        assert!(matches!(
            rail_key(&m, &rail, 1, Key::F1, none),
            RailKey::Pass
        ));
        assert!(matches!(
            rail_key(&m, &rail, 1, Key::F2, none),
            RailKey::Consume
        ));
        assert!(matches!(
            rail_key(&m, &rail, 1, Key::F8, none),
            RailKey::Consume
        ));
        assert!(matches!(
            rail_key(&m, &rail, 1, Key::F6, none),
            RailKey::Consume
        ));
        assert!(matches!(
            rail_key(&m, &rail, 1, Key::Delete, Shortcut::Shift),
            RailKey::Consume
        ));
        assert!(matches!(
            rail_key(&m, &rail, 1, Key::from_char('x'), Shortcut::Ctrl),
            RailKey::Consume
        ));
        assert!(matches!(
            rail_key(
                &m,
                &rail,
                1,
                Key::from_char('s'),
                Shortcut::Ctrl | Shortcut::Alt
            ),
            RailKey::ReturnToList
        ));
    }

    #[test]
    fn escape_leaves_the_focused_row_marked_with_fill_and_ring() {
        use crate::painter::{Image, Painter, Rect};
        use files_core::theme::Color;
        #[derive(Default)]
        struct Fills(Vec<(Rect, Color)>);
        impl Painter for Fills {
            fn rect(&mut self, r: Rect, c: Color) {
                self.0.push((r, c));
            }
            fn rounded_rect(&mut self, r: Rect, _: i32, c: Color) {
                self.0.push((r, c));
            }
            fn text(&mut self, _: Rect, _: &str, _: i32, _: Color, _: bool, _: bool) {}
            fn image(&mut self, _: Rect, _: &Image<'_>) {}
            fn clip(&mut self, _: Rect) {}
            fn unclip(&mut self) {}
        }
        fltk_omarchy::anim::set_enabled(false);
        let mut m = model();
        listed(&mut m);
        m.view = View::List;
        m.selected = Some(1);
        m.selection = [0, 1, 2].into();
        let frame = |m: &mut Model| {
            let mut p = Fills::default();
            crate::view::paint(&mut p, m, 1000, 650);
            let (selection, accent) = (m.palette.selection, m.palette.accent);
            let rows: Vec<i32> =
                p.0.iter()
                    .filter(|(r, c)| *c == selection && r.h == 38)
                    .map(|(r, _)| r.y)
                    .collect();
            // The ring's top strip: 2 px of accent across the row.
            let ring =
                p.0.iter()
                    .any(|(r, c)| *c == accent && r.h == 2 && r.w > 100);
            (rows, ring)
        };
        let (marked, ring) = frame(&mut m);
        assert_eq!(marked.len(), 3);
        assert!(ring);
        m.escape_browser();
        let (marked, ring) = frame(&mut m);
        // Only the focused row keeps the mark fill, and it still has the ring.
        assert_eq!(marked.len(), 1);
        assert!(ring);
        m.selection.clear();
        let (marked, ring) = frame(&mut m);
        // Focus alone: the ring without any mark fill.
        assert!(marked.is_empty());
        assert!(ring);
        fltk_omarchy::anim::set_enabled(None);
    }

    #[test]
    fn nautilus_profile_rail_keys_cycle_focus_and_pass_reload() {
        let mut m = model();
        m.shortcuts = shortcuts::Profile::Nautilus;
        let rail = Rail::default();
        let none = Shortcut::None;
        assert!(matches!(
            rail_key(&m, &rail, 0, Key::F6, none),
            RailKey::ToTabs
        ));
        assert!(matches!(
            rail_key(&m, &rail, 0, Key::F5, none),
            RailKey::Pass
        ));
        assert!(matches!(
            rail_key(&m, &rail, 0, Key::from_char('n'), Shortcut::Ctrl),
            RailKey::Pass
        ));
        assert!(matches!(
            rail_key(&m, &rail, 0, Key::F2, none),
            RailKey::Consume
        ));
        // List focus: F6 asks for the rail when it exists, else the tabs;
        // dual mode switches panes instead.
        m.sidebar_widget = true;
        m.action(Action::FocusCycle);
        assert_eq!(m.focus_request.take(), Some(FocusTarget::Rail));
        m.sidebar_widget = false;
        m.action(Action::FocusCycle);
        assert_eq!(m.focus_request.take(), Some(FocusTarget::Tabs));
    }

    #[test]
    fn mark_toggle_keeps_focus_and_insert_advances() {
        let mut m = model();
        listed(&mut m);
        m.selected = Some(1);
        m.selection.clear();
        m.action(Action::MarkToggle);
        assert_eq!(m.selected, Some(1));
        assert!(m.selection.contains(&1));
        m.action(Action::MarkToggle);
        assert_eq!(m.selected, Some(1));
        assert!(!m.selection.contains(&1));
        m.mark_and_advance();
        assert!(m.selection.contains(&1));
        assert_eq!(m.selected, Some(2));
    }

    #[test]
    fn new_window_uses_the_injected_spawner_on_the_current_folder() {
        use std::sync::Mutex;
        static SEEN: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
        fn record(path: &std::path::Path) -> Result<(), String> {
            SEEN.lock().unwrap().push(path.to_path_buf());
            Ok(())
        }
        fn fail(_: &std::path::Path) -> Result<(), String> {
            Err("Nueva ventana: sin ejecutable".into())
        }
        let mut m = model();
        m.window_spawner = record;
        m.action(Action::NewWindow);
        assert_eq!(SEEN.lock().unwrap().as_slice(), [m.path.clone()]);
        m.window_spawner = fail;
        m.action(Action::NewWindow);
        assert!(m.error.contains("sin ejecutable"));
    }

    #[test]
    fn profile_switch_is_live_and_persisted_with_the_preferences() {
        let mut m = model();
        let (tx, rx) = mpsc::sync_channel(4);
        m.aux.tx = tx;
        m.action(Action::ShortcutProfile(shortcuts::Profile::Nautilus));
        assert_eq!(m.shortcuts, shortcuts::Profile::Nautilus);
        let crate::aux_io::Request::Preferences(.., profile) = rx.try_recv().unwrap() else {
            panic!("preferences request");
        };
        assert_eq!(profile, shortcuts::Profile::Nautilus);
        // Zoom outside the grid names the grid chord of the active profile.
        m.view = View::List;
        m.action(Action::ZoomStep(1));
        assert!(m.error.contains("(Ctrl+2)"), "{}", m.error);
    }
}

//! The single table of keyboard chords per shortcut profile.
//!
//! Every browser command that has a chord is listed here once. The key
//! dispatcher asks [`is`] whether an event is a command's chord, and the help
//! view, command palette, menus, operation bar and tooltips print [`label`],
//! so the advertised chords always match the active profile.
use crate::model::{Action, View};
pub use files_core::theme::ShortcutProfile as Profile;
use fltk::enums::{Key, Shortcut};
use std::borrow::Cow;

/// A browser command that can own chords.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cmd {
    // Window-wide: these work above overlays such as help or a preview.
    Commands,
    Help,
    TabNew,
    TabClose,
    TabReopen,
    TabNext,
    TabPrev,
    TabMenu,
    TabMoveLeft,
    TabMoveRight,
    SizesManager,
    OperationBar,
    FocusTabs,
    NoticeFocus,
    // Browser list.
    FocusRail,
    FocusCycle,
    DualPane,
    RailToggle,
    Search,
    Filter,
    SizeFocused,
    SizesListed,
    BookmarkAdd,
    Bookmarks,
    MarkAdvance,
    MarkToggle,
    OpenWith,
    SelectPattern,
    LinkCopied,
    CreateLinks,
    Terminal,
    CopyPath,
    Preview,
    Trash,
    DeletePermanently,
    Breadcrumbs,
    Open,
    OpenInTab,
    OpenInBackground,
    Back,
    Forward,
    Parent,
    Home,
    Properties,
    ClipboardCopy,
    ClipboardCut,
    ClipboardPaste,
    NativePaste,
    Hidden,
    Refresh,
    ContextMenu,
    Inspector,
    Actions,
    Settings,
    CopyFiles,
    MoveFiles,
    Rename,
    NewFolder,
    NewFile,
    NewWindow,
    Undo,
    Redo,
    Location,
    ViewGroups,
    ViewList,
    ViewColumns,
    ViewSpace,
    ViewGrid,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    SelectAll,
    SelectFiltered,
    ClearSelection,
    InvertSelection,
    SortMenu,
    GoRecent,
    GoStarred,
    StarToggle,
    OpenContaining,
}

type Row = (Cmd, &'static [&'static str]);

/// Chords shared by both profiles unless a profile table overrides them.
const COMMON: &[Row] = &[
    (Cmd::Commands, &["Ctrl+Shift+P"]),
    (Cmd::Help, &["F1", "Ctrl+?"]),
    (Cmd::TabNew, &["Ctrl+T"]),
    (Cmd::TabClose, &["Ctrl+W"]),
    (Cmd::TabReopen, &["Ctrl+Shift+T"]),
    (Cmd::TabNext, &["Ctrl+Tab", "Ctrl+AvPág"]),
    (Cmd::TabPrev, &["Ctrl+Shift+Tab", "Ctrl+RePág"]),
    (Cmd::TabMenu, &["Ctrl+Shift+F10"]),
    (Cmd::TabMoveLeft, &["Ctrl+Shift+RePág"]),
    (Cmd::TabMoveRight, &["Ctrl+Shift+AvPág"]),
    (Cmd::SizesManager, &["Ctrl+Alt+Espacio"]),
    (Cmd::OperationBar, &["Ctrl+Alt+B"]),
    (Cmd::FocusTabs, &["Ctrl+Alt+T"]),
    (Cmd::NoticeFocus, &["Ctrl+Alt+N"]),
    (Cmd::FocusRail, &["Ctrl+Alt+S"]),
    (Cmd::FocusCycle, &[]),
    (Cmd::DualPane, &["F9"]),
    (Cmd::RailToggle, &[]),
    (Cmd::Search, &["Ctrl+Shift+F"]),
    (Cmd::Filter, &["Ctrl+F"]),
    (Cmd::SizeFocused, &["Ctrl+Espacio"]),
    (Cmd::SizesListed, &["Ctrl+Shift+Espacio"]),
    (Cmd::BookmarkAdd, &["Ctrl+D"]),
    (Cmd::Bookmarks, &["Ctrl+Alt+D"]),
    (Cmd::MarkAdvance, &["Insert"]),
    (Cmd::MarkToggle, &["Ctrl+Insert"]),
    (Cmd::OpenWith, &["Ctrl+Shift+O"]),
    (Cmd::SelectPattern, &["Ctrl+S"]),
    (Cmd::LinkCopied, &["Ctrl+M"]),
    (Cmd::CreateLinks, &["Ctrl+Shift+M"]),
    (Cmd::Terminal, &["Ctrl+."]),
    (Cmd::CopyPath, &["Ctrl+Alt+C"]),
    (Cmd::Preview, &["Espacio", "F3"]),
    (Cmd::Trash, &["Supr", "F8"]),
    (Cmd::DeletePermanently, &["Shift+Supr"]),
    (Cmd::Breadcrumbs, &["Alt+Abajo"]),
    (Cmd::Open, &["Enter"]),
    (Cmd::OpenInTab, &["Ctrl+Enter"]),
    (Cmd::OpenInBackground, &["Shift+Enter"]),
    (Cmd::Back, &["Alt+Izquierda"]),
    (Cmd::Forward, &["Alt+Derecha"]),
    (Cmd::Parent, &["Backspace", "Alt+Arriba"]),
    (Cmd::Home, &["Alt+Inicio"]),
    (Cmd::Properties, &["Alt+Enter"]),
    (Cmd::ClipboardCopy, &["Ctrl+C"]),
    (Cmd::ClipboardCut, &["Ctrl+X"]),
    (Cmd::ClipboardPaste, &["Ctrl+V"]),
    (Cmd::NativePaste, &["Ctrl+Shift+V"]),
    (Cmd::Hidden, &["Ctrl+H"]),
    (Cmd::Refresh, &["Ctrl+R"]),
    (Cmd::ContextMenu, &["Menú", "Shift+F10"]),
    (Cmd::Inspector, &["Ctrl+Alt+I"]),
    (Cmd::Actions, &["F10"]),
    (Cmd::Settings, &["Ctrl+,"]),
    (Cmd::CopyFiles, &["F5"]),
    (Cmd::MoveFiles, &["F6"]),
    (Cmd::Rename, &["F2"]),
    (Cmd::NewFolder, &["F7", "Ctrl+Shift+N"]),
    (Cmd::NewFile, &["Ctrl+N"]),
    (Cmd::NewWindow, &[]),
    (Cmd::Undo, &["Ctrl+Z"]),
    (Cmd::Redo, &["Ctrl+Shift+Z"]),
    (Cmd::Location, &["Ctrl+L"]),
    (Cmd::ViewGroups, &["Ctrl+1"]),
    (Cmd::ViewList, &["Ctrl+2"]),
    (Cmd::ViewColumns, &["Ctrl+3"]),
    (Cmd::ViewSpace, &["Ctrl+4"]),
    (Cmd::ViewGrid, &["Ctrl+5"]),
    (Cmd::ZoomIn, &["Ctrl++"]),
    (Cmd::ZoomOut, &["Ctrl+-"]),
    (Cmd::ZoomReset, &["Ctrl+0"]),
    (Cmd::SelectAll, &["Ctrl+A"]),
    (Cmd::SelectFiltered, &["Ctrl+Shift+A"]),
    (Cmd::ClearSelection, &["Ctrl+Shift+D"]),
    (Cmd::InvertSelection, &["Ctrl+I"]),
    (Cmd::SortMenu, &["Ctrl+Shift+S"]),
    // Free in both profiles (GNOME Files binds none of them; Ctrl+Alt+O is
    // its "Open Item Location" in Recent and Search).
    (Cmd::GoRecent, &["Ctrl+Alt+R"]),
    (Cmd::GoStarred, &["Ctrl+Alt+E"]),
    (Cmd::StarToggle, &["Ctrl+E"]),
    (Cmd::OpenContaining, &["Ctrl+Alt+O"]),
];

/// Files keeps today's bindings: everything comes from [`COMMON`].
const FILES: &[Row] = &[];

/// GNOME Files (Nautilus) conventions. Commands whose chord moves away stay
/// reachable from the command palette and menus.
const NAUTILUS: &[Row] = &[
    (Cmd::Refresh, &["F5", "Ctrl+R"]),
    (Cmd::CopyFiles, &[]),
    (Cmd::MoveFiles, &[]),
    (Cmd::FocusCycle, &["F6"]),
    (Cmd::RailToggle, &["F9"]),
    (Cmd::DualPane, &["F3"]),
    (Cmd::Preview, &["Espacio"]),
    (Cmd::ViewList, &["Ctrl+1"]),
    (Cmd::ViewGrid, &["Ctrl+2"]),
    (Cmd::ViewGroups, &["Ctrl+3"]),
    (Cmd::ViewColumns, &["Ctrl+4"]),
    (Cmd::ViewSpace, &["Ctrl+5"]),
    (Cmd::Properties, &["Ctrl+I", "Alt+Enter"]),
    (Cmd::InvertSelection, &["Ctrl+Shift+I"]),
    (Cmd::NewWindow, &["Ctrl+N"]),
    (Cmd::NewFile, &[]),
    (Cmd::NewFolder, &["Ctrl+Shift+N", "F7"]),
    (Cmd::MarkToggle, &["Ctrl+Espacio", "Ctrl+Insert"]),
    (Cmd::SizeFocused, &[]),
    (Cmd::Open, &["Enter", "Alt+Abajo"]),
    (Cmd::Breadcrumbs, &[]),
    (Cmd::Search, &["Ctrl+F"]),
    (Cmd::Filter, &["Ctrl+Shift+F"]),
];

/// Matched but never printed: layout variants of a listed chord.
const ALIASES: &[Row] = &[
    (Cmd::ZoomIn, &["Ctrl+="]),
    (Cmd::Help, &["Ctrl+Shift+/"]),
    (Cmd::Preview, &["Shift+Espacio"]),
];

fn table(profile: Profile) -> &'static [Row] {
    match profile {
        Profile::Files => FILES,
        Profile::Nautilus => NAUTILUS,
    }
}

/// Every command, in table order.
pub fn commands() -> impl Iterator<Item = Cmd> {
    COMMON.iter().map(|(cmd, _)| *cmd)
}

/// The printed chords of a command in a profile, most prominent first.
pub fn chords(profile: Profile, cmd: Cmd) -> &'static [&'static str] {
    table(profile)
        .iter()
        .chain(COMMON)
        .find(|(c, _)| *c == cmd)
        .map_or(&[], |(_, chords)| chords)
}

fn aliases(profile: Profile, cmd: Cmd) -> &'static [&'static str] {
    if chords(profile, cmd).is_empty() {
        return &[];
    }
    ALIASES
        .iter()
        .find(|(c, _)| *c == cmd)
        .map_or(&[], |(_, chords)| chords)
}

/// "F1 / Ctrl+?" style label; empty when the command has no chord.
pub fn label(profile: Profile, cmd: Cmd) -> String {
    chords(profile, cmd).join(" / ")
}

/// The first chord only, for narrow places such as menu accelerators.
pub fn primary(profile: Profile, cmd: Cmd) -> &'static str {
    chords(profile, cmd).first().copied().unwrap_or("")
}

/// A parsed chord: one key plus exact modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord {
    pub key: Key,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

fn key_named(name: &str) -> Option<Key> {
    Some(match name {
        "F1" => Key::F1,
        "F2" => Key::F2,
        "F3" => Key::F3,
        "F4" => Key::F4,
        "F5" => Key::F5,
        "F6" => Key::F6,
        "F7" => Key::F7,
        "F8" => Key::F8,
        "F9" => Key::F9,
        "F10" => Key::F10,
        "F11" => Key::F11,
        "F12" => Key::F12,
        "Espacio" => Key::from_char(' '),
        "Supr" => Key::Delete,
        "Insert" => Key::Insert,
        "Enter" => Key::Enter,
        "Backspace" => Key::BackSpace,
        "Tab" => Key::Tab,
        "Esc" => Key::Escape,
        "Izquierda" => Key::Left,
        "Derecha" => Key::Right,
        "Arriba" => Key::Up,
        "Abajo" => Key::Down,
        "Inicio" => Key::Home,
        "Fin" => Key::End,
        "RePág" => Key::PageUp,
        "AvPág" => Key::PageDown,
        "Menú" => Key::Menu,
        _ => {
            let mut chars = name.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            Key::from_char(c.to_ascii_lowercase())
        }
    })
}

/// Parses a printed chord such as "Ctrl+Shift+F" or "Ctrl++".
pub fn parse(text: &str) -> Option<Chord> {
    let (mods, key) = if let Some(mods) = text.strip_suffix("++") {
        (mods, "+")
    } else {
        match text.rsplit_once('+') {
            Some((mods, key)) => (mods, key),
            None => ("", text),
        }
    };
    let mut chord = Chord {
        key: key_named(key)?,
        ctrl: false,
        shift: false,
        alt: false,
    };
    for m in mods.split('+').filter(|m| !m.is_empty()) {
        match m {
            "Ctrl" => chord.ctrl = true,
            "Shift" => chord.shift = true,
            "Alt" => chord.alt = true,
            _ => return None,
        }
    }
    Some(chord)
}

/// Symbols that need Shift on common layouts match with or without it.
fn loose_shift(key: Key) -> bool {
    ['+', '=', '-', '?']
        .iter()
        .any(|&c| key == Key::from_char(c))
}

impl Chord {
    pub fn matches(&self, key: Key, mods: Shortcut) -> bool {
        let key = if key == Key::KPEnter { Key::Enter } else { key };
        self.key == key
            && self.ctrl == mods.intersects(Shortcut::Ctrl | Shortcut::Meta)
            && self.alt == mods.intersects(Shortcut::Alt)
            && (self.shift == mods.intersects(Shortcut::Shift) || loose_shift(self.key))
    }
}

/// Whether this key event is one of the command's chords in the profile.
pub fn is(profile: Profile, cmd: Cmd, key: Key, mods: Shortcut) -> bool {
    chords(profile, cmd)
        .iter()
        .chain(aliases(profile, cmd))
        .filter_map(|c| parse(c))
        .any(|c| c.matches(key, mods))
}

/// The command a key event means in the profile, if any.
pub fn resolve(profile: Profile, key: Key, mods: Shortcut) -> Option<Cmd> {
    commands().find(|&cmd| is(profile, cmd, key, mods))
}

/// The command palette entry that runs this command, when there is one.
pub fn palette_id(cmd: Cmd) -> Option<&'static str> {
    Some(match cmd {
        Cmd::Help => "help",
        Cmd::TabNew => "tab-new",
        Cmd::TabClose => "tab-close",
        Cmd::TabReopen => "tab-reopen",
        Cmd::TabNext => "tab-next",
        Cmd::TabPrev => "tab-prev",
        Cmd::SizesManager => "sizes-manager",
        Cmd::OperationBar => "operation-bar",
        Cmd::FocusTabs => "focus-tabs",
        Cmd::FocusRail => "focus-rail",
        Cmd::FocusCycle => "focus-cycle",
        Cmd::DualPane => "dual",
        Cmd::RailToggle => "rail-toggle",
        Cmd::Search => "search",
        Cmd::Filter => "filter",
        Cmd::SizeFocused => "sizes-one",
        Cmd::SizesListed => "sizes-list",
        Cmd::BookmarkAdd => "bookmark-add",
        Cmd::Bookmarks => "bookmarks",
        Cmd::MarkToggle => "mark-toggle",
        Cmd::OpenWith => "open-with",
        Cmd::SelectPattern => "select-pattern",
        Cmd::LinkCopied => "paste-link",
        Cmd::CreateLinks => "create-link",
        Cmd::Terminal => "terminal",
        Cmd::CopyPath => "copy-path",
        Cmd::Preview => "preview",
        Cmd::Trash => "trash",
        Cmd::DeletePermanently => "delete-permanently",
        Cmd::Breadcrumbs => "breadcrumbs",
        Cmd::Open => "open",
        Cmd::Back => "history-back",
        Cmd::Forward => "history-forward",
        Cmd::Parent => "parent",
        Cmd::Home => "go-home",
        Cmd::Properties => "properties",
        Cmd::ClipboardCopy => "clipboard-copy",
        Cmd::ClipboardCut => "clipboard-cut",
        Cmd::ClipboardPaste => "clipboard-paste",
        Cmd::Hidden => "hidden",
        Cmd::Refresh => "refresh",
        Cmd::Inspector => "inspector",
        Cmd::Actions => "actions-menu",
        Cmd::Settings => "settings",
        Cmd::CopyFiles => "copy",
        Cmd::MoveFiles => "move",
        Cmd::Rename => "rename",
        Cmd::NewFolder => "new-dir",
        Cmd::NewFile => "new-file",
        Cmd::NewWindow => "new-window",
        Cmd::Undo => "undo",
        Cmd::Redo => "redo",
        Cmd::Location => "edit-location",
        Cmd::ViewGroups => "view-groups",
        Cmd::ViewList => "view-list",
        Cmd::ViewColumns => "view-columns",
        Cmd::ViewSpace => "view-space",
        Cmd::ViewGrid => "view-grid",
        Cmd::ZoomIn => "zoom-in",
        Cmd::ZoomOut => "zoom-out",
        Cmd::ZoomReset => "zoom-reset",
        Cmd::SelectAll => "select-all",
        Cmd::SelectFiltered => "filtered",
        Cmd::ClearSelection => "clear",
        Cmd::InvertSelection => "invert",
        Cmd::GoRecent => "go-recent",
        Cmd::GoStarred => "go-starred",
        Cmd::StarToggle => "star-toggle",
        Cmd::OpenContaining => "open-containing",
        _ => return None,
    })
}

/// The command behind a palette entry id.
pub fn palette_cmd(id: &str) -> Option<Cmd> {
    commands().find(|&cmd| palette_id(cmd) == Some(id))
}

/// The command an existing Action stands for, so menus and tooltips can
/// print its chord. Context-dependent wrappers resolve to the plain command.
pub fn cmd_for_action(action: &Action) -> Option<Cmd> {
    use crate::selection_tools::Command as Sel;
    use crate::tabs::Command as Tab;
    Some(match action {
        Action::Commands => Cmd::Commands,
        Action::Help => Cmd::Help,
        Action::Tab(Tab::New) => Cmd::TabNew,
        Action::Tab(Tab::Close(_)) => Cmd::TabClose,
        Action::Tab(Tab::Reopen) => Cmd::TabReopen,
        Action::Tab(Tab::Cycle(1)) => Cmd::TabNext,
        Action::Tab(Tab::Cycle(-1)) => Cmd::TabPrev,
        Action::Sizes => Cmd::SizesManager,
        Action::ToggleOperationBar => Cmd::OperationBar,
        Action::FocusTabs => Cmd::FocusTabs,
        Action::FocusRail => Cmd::FocusRail,
        Action::FocusCycle => Cmd::FocusCycle,
        Action::DualPane => Cmd::DualPane,
        Action::RailPin(_) => Cmd::RailToggle,
        Action::Search => Cmd::Search,
        Action::Filter => Cmd::Filter,
        Action::CalculateSize => Cmd::SizeFocused,
        Action::CalculateListedSizes => Cmd::SizesListed,
        Action::BookmarkAdd => Cmd::BookmarkAdd,
        Action::Bookmarks => Cmd::Bookmarks,
        Action::MarkToggle => Cmd::MarkToggle,
        Action::OpenWith => Cmd::OpenWith,
        Action::SelectPattern => Cmd::SelectPattern,
        Action::LinkCopied => Cmd::LinkCopied,
        Action::CreateLinks => Cmd::CreateLinks,
        Action::Terminal => Cmd::Terminal,
        Action::CopyPath => Cmd::CopyPath,
        Action::Preview => Cmd::Preview,
        Action::Trash => Cmd::Trash,
        Action::DeletePermanently => Cmd::DeletePermanently,
        Action::Breadcrumbs => Cmd::Breadcrumbs,
        Action::Open => Cmd::Open,
        Action::History(false) => Cmd::Back,
        Action::History(true) => Cmd::Forward,
        Action::Parent => Cmd::Parent,
        Action::GoHome => Cmd::Home,
        Action::Properties => Cmd::Properties,
        Action::ClipboardCopy => Cmd::ClipboardCopy,
        Action::ClipboardCut => Cmd::ClipboardCut,
        Action::ClipboardPaste => Cmd::ClipboardPaste,
        Action::NativePaste => Cmd::NativePaste,
        Action::ToggleHidden => Cmd::Hidden,
        Action::Refresh => Cmd::Refresh,
        Action::Inspector => Cmd::Inspector,
        Action::Actions => Cmd::Actions,
        Action::Settings => Cmd::Settings,
        Action::CopyFiles => Cmd::CopyFiles,
        Action::MoveFiles => Cmd::MoveFiles,
        Action::Rename => Cmd::Rename,
        Action::NewEntry(true) => Cmd::NewFolder,
        Action::NewEntry(false) => Cmd::NewFile,
        Action::NewWindow => Cmd::NewWindow,
        Action::Undo(false) => Cmd::Undo,
        Action::Undo(true) => Cmd::Redo,
        Action::Omni => Cmd::Location,
        Action::View(View::Groups) => Cmd::ViewGroups,
        Action::View(View::List) => Cmd::ViewList,
        Action::View(View::Columns | View::Spatial) => Cmd::ViewColumns,
        Action::View(View::SpaceMap) => Cmd::ViewSpace,
        Action::View(View::Grid) => Cmd::ViewGrid,
        Action::ZoomStep(1) => Cmd::ZoomIn,
        Action::ZoomStep(-1) => Cmd::ZoomOut,
        Action::ZoomStep(0) => Cmd::ZoomReset,
        Action::Selection(Sel::All) => Cmd::SelectAll,
        Action::Selection(Sel::Filtered) => Cmd::SelectFiltered,
        Action::Selection(Sel::Clear) => Cmd::ClearSelection,
        Action::Selection(Sel::Invert) => Cmd::InvertSelection,
        Action::SortMenu(..) => Cmd::SortMenu,
        Action::OpenRecentView => Cmd::GoRecent,
        Action::OpenStarredView => Cmd::GoStarred,
        Action::StarToggle => Cmd::StarToggle,
        Action::OpenContaining => Cmd::OpenContaining,
        Action::OperationBar(_, _, index) => crate::operation_bar::command(*index)?,
        _ => return None,
    })
}

/// The chord label of an action in the profile; empty when it has none.
pub fn action_label(profile: Profile, action: &Action) -> String {
    cmd_for_action(action).map_or_else(String::new, |cmd| label(profile, cmd))
}

/// `base` followed by " · chord" when the action has a chord.
pub fn with_chord(profile: Profile, base: &str, action: &Action) -> Cow<'static, str> {
    match cmd_for_action(action).map(|cmd| primary(profile, cmd)) {
        Some(chord) if !chord.is_empty() => format!("{base} · {chord}").into(),
        _ => base.to_owned().into(),
    }
}

/// Hover text: the action's accessible name plus its chord in the profile.
pub fn tooltip(profile: Profile, action: &Action) -> Option<Cow<'static, str>> {
    let name = action.accessible_name()?;
    let chord = action_label(profile, action);
    Some(if chord.is_empty() {
        name.into()
    } else {
        format!("{name}, {chord}").into()
    })
}

/// Starts a detached Files window on `path`: same binary, no session
/// restore, its own process group so it outlives this window's terminal.
pub fn spawn_window(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe().map_err(|e| format!("Nueva ventana: {e}"))?;
    let mut child = std::process::Command::new(exe)
        .arg("--no-session")
        .arg("--")
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|e| format!("Nueva ventana: {e}"))?;
    // Reap the child when it exits so it never lingers as a zombie.
    std::thread::Builder::new()
        .name("files-new-window".into())
        .spawn(move || {
            let _ = child.wait();
        })
        .map(|_| ())
        .map_err(|e| format!("Nueva ventana: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOTH: [Profile; 2] = [Profile::Files, Profile::Nautilus];

    fn key(text: &str) -> (Key, Shortcut) {
        let c = parse(text).unwrap_or_else(|| panic!("chord {text}"));
        let mut mods = Shortcut::None;
        if c.ctrl {
            mods |= Shortcut::Ctrl;
        }
        if c.shift {
            mods |= Shortcut::Shift;
        }
        if c.alt {
            mods |= Shortcut::Alt;
        }
        (c.key, mods)
    }

    fn resolves(profile: Profile, text: &str) -> Option<Cmd> {
        let (k, m) = key(text);
        resolve(profile, k, m)
    }

    #[test]
    fn every_printed_chord_parses_and_resolves_to_its_own_command() {
        for profile in BOTH {
            for cmd in commands() {
                for chord in chords(profile, cmd).iter().chain(aliases(profile, cmd)) {
                    assert_eq!(resolves(profile, chord), Some(cmd), "{profile:?} {chord}");
                }
            }
        }
    }

    #[test]
    fn no_two_commands_share_a_chord_in_a_profile() {
        for profile in BOTH {
            let mut seen: Vec<(Chord, Cmd)> = Vec::new();
            for cmd in commands() {
                for chord in chords(profile, cmd).iter().chain(aliases(profile, cmd)) {
                    let parsed = parse(chord).unwrap();
                    if let Some((_, other)) = seen.iter().find(|(c, _)| *c == parsed) {
                        panic!("{profile:?}: {chord} is both {other:?} and {cmd:?}");
                    }
                    seen.push((parsed, cmd));
                }
            }
            // Loose-shift symbols must not collide with a shifted twin either.
            for (a, ca) in &seen {
                for (b, cb) in &seen {
                    if ca != cb && a.key == b.key && a.ctrl == b.ctrl && a.alt == b.alt {
                        assert!(!loose_shift(a.key), "{profile:?}: {a:?} vs {b:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn every_profile_override_names_a_common_command_once() {
        for profile in BOTH {
            let mut seen = std::collections::HashSet::new();
            for (cmd, _) in table(profile) {
                assert!(seen.insert(*cmd), "{profile:?} repeats {cmd:?}");
                assert!(COMMON.iter().any(|(c, _)| c == cmd));
            }
        }
        let mut seen = std::collections::HashSet::new();
        for (cmd, _) in COMMON {
            assert!(seen.insert(*cmd), "COMMON repeats {cmd:?}");
        }
    }

    #[test]
    fn files_profile_keeps_the_historical_bindings() {
        let p = Profile::Files;
        for (chord, cmd) in [
            ("F5", Cmd::CopyFiles),
            ("F6", Cmd::MoveFiles),
            ("F9", Cmd::DualPane),
            ("F3", Cmd::Preview),
            ("Espacio", Cmd::Preview),
            ("Ctrl+1", Cmd::ViewGroups),
            ("Ctrl+2", Cmd::ViewList),
            ("Ctrl+3", Cmd::ViewColumns),
            ("Ctrl+4", Cmd::ViewSpace),
            ("Ctrl+5", Cmd::ViewGrid),
            ("Ctrl+I", Cmd::InvertSelection),
            ("Alt+Enter", Cmd::Properties),
            ("Ctrl+N", Cmd::NewFile),
            ("Ctrl+Shift+N", Cmd::NewFolder),
            ("F7", Cmd::NewFolder),
            ("Ctrl+Espacio", Cmd::SizeFocused),
            ("Ctrl+Insert", Cmd::MarkToggle),
            ("Insert", Cmd::MarkAdvance),
            ("Alt+Abajo", Cmd::Breadcrumbs),
            ("Ctrl+F", Cmd::Filter),
            ("Ctrl+Shift+F", Cmd::Search),
            ("Ctrl+R", Cmd::Refresh),
            ("Ctrl+Shift+S", Cmd::SortMenu),
            ("Ctrl+=", Cmd::ZoomIn),
            ("Ctrl+Shift+/", Cmd::Help),
        ] {
            assert_eq!(resolves(p, chord), Some(cmd), "{chord}");
        }
        assert_eq!(resolves(p, "Ctrl+Shift+I"), None);
        for profile in BOTH {
            assert_eq!(resolves(profile, "Ctrl+Alt+R"), Some(Cmd::GoRecent));
            assert_eq!(resolves(profile, "Ctrl+Alt+E"), Some(Cmd::GoStarred));
            assert_eq!(resolves(profile, "Ctrl+E"), Some(Cmd::StarToggle));
            assert_eq!(resolves(profile, "Ctrl+Alt+O"), Some(Cmd::OpenContaining));
        }
        assert!(chords(p, Cmd::NewWindow).is_empty());
        assert!(chords(p, Cmd::FocusCycle).is_empty());
    }

    #[test]
    fn nautilus_profile_remaps_the_gnome_chords() {
        let p = Profile::Nautilus;
        for (chord, cmd) in [
            ("F5", Cmd::Refresh),
            ("Ctrl+R", Cmd::Refresh),
            ("F6", Cmd::FocusCycle),
            ("F9", Cmd::RailToggle),
            ("F3", Cmd::DualPane),
            ("Espacio", Cmd::Preview),
            ("Ctrl+1", Cmd::ViewList),
            ("Ctrl+2", Cmd::ViewGrid),
            ("Ctrl+3", Cmd::ViewGroups),
            ("Ctrl+4", Cmd::ViewColumns),
            ("Ctrl+5", Cmd::ViewSpace),
            ("Ctrl+I", Cmd::Properties),
            ("Alt+Enter", Cmd::Properties),
            ("Ctrl+Shift+I", Cmd::InvertSelection),
            ("Ctrl+N", Cmd::NewWindow),
            ("Ctrl+Shift+N", Cmd::NewFolder),
            ("Ctrl+Espacio", Cmd::MarkToggle),
            ("Ctrl+Insert", Cmd::MarkToggle),
            ("Alt+Abajo", Cmd::Open),
            ("Enter", Cmd::Open),
            ("Ctrl+F", Cmd::Search),
            ("Ctrl+Shift+F", Cmd::Filter),
            ("Ctrl+Shift+S", Cmd::SortMenu),
        ] {
            assert_eq!(resolves(p, chord), Some(cmd), "{chord}");
        }
        // Displaced commands lose their chord but keep a palette entry.
        for cmd in [
            Cmd::CopyFiles,
            Cmd::MoveFiles,
            Cmd::NewFile,
            Cmd::SizeFocused,
            Cmd::Breadcrumbs,
        ] {
            assert!(chords(p, cmd).is_empty(), "{cmd:?}");
            assert!(palette_id(cmd).is_some(), "{cmd:?}");
        }
    }

    #[test]
    fn modifiers_are_exact_except_for_shifted_symbols() {
        let p = Profile::Files;
        assert_eq!(
            resolve(p, Key::from_char('r'), Shortcut::Ctrl | Shortcut::Shift),
            None
        );
        assert_eq!(
            resolve(p, Key::from_char('+'), Shortcut::Ctrl | Shortcut::Shift),
            Some(Cmd::ZoomIn)
        );
        assert_eq!(
            resolve(p, Key::from_char('r'), Shortcut::Meta),
            Some(Cmd::Refresh)
        );
        assert_eq!(resolve(p, Key::KPEnter, Shortcut::None), Some(Cmd::Open));
        assert_eq!(resolve(p, Key::from_char('/'), Shortcut::Ctrl), None);
    }

    #[test]
    fn labels_join_printed_chords_and_skip_aliases() {
        assert_eq!(label(Profile::Files, Cmd::Help), "F1 / Ctrl+?");
        assert_eq!(label(Profile::Files, Cmd::ZoomIn), "Ctrl++");
        assert_eq!(label(Profile::Nautilus, Cmd::CopyFiles), "");
        assert_eq!(primary(Profile::Nautilus, Cmd::Refresh), "F5");
        assert_eq!(
            with_chord(Profile::Nautilus, "Propiedades", &Action::Properties),
            "Propiedades · Ctrl+I"
        );
        assert_eq!(
            with_chord(Profile::Nautilus, "Copiar selección", &Action::CopyFiles),
            "Copiar selección"
        );
    }

    #[test]
    fn palette_ids_are_unique_and_map_back() {
        let mut seen = std::collections::HashSet::new();
        for cmd in commands() {
            if let Some(id) = palette_id(cmd) {
                assert!(seen.insert(id), "{id}");
                assert_eq!(palette_cmd(id), Some(cmd));
            }
        }
    }
}

//! "Recientes" and "Destacados": virtual locations shown over the browser.
//!
//! Files models places that are not one folder (the trash catalog, search
//! results) as panels the model owns, not as listings of `Model::path`:
//! every file action in Files joins an entry name to the current folder,
//! so a list of absolute paths from many folders cannot safely pose as one.
//! This panel follows the trash browser's model: it paints over the listing
//! area (the sidebar stays usable), owns its keys, loads off the UI thread
//! and leaves the folder underneath untouched, so closing it (Backspace,
//! Alt+Left, Esc) returns exactly where the person was.
//!
//! While it is open [`gate`] refuses every file action at the model level,
//! whatever asked for it (key, palette, menu, toolbar), and says why: trash,
//! delete, rename, cut, paste, move, copy and new items cannot run here.
//! Delete only removes the entry from the list; the file is never touched.
use crate::{
    model::{Action, Hit, Model},
    painter::{Painter, Rect},
};
use files_core::recent::MAX_LISTED;
use fltk::{
    app,
    enums::{Event, Key, Shortcut},
};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver},
    },
};

const ROW: i32 = 38;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Recent,
    Starred,
}

impl Source {
    pub fn title(self) -> &'static str {
        match self {
            Self::Recent => "Recientes",
            Self::Starred => "Destacados",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub path: PathBuf,
    pub folder: bool,
    /// The path exists (missing starred items stay listed, unavailable).
    pub available: bool,
    /// ISO 8601 UTC: last use (Recientes) or modification (Destacados).
    pub when: String,
    /// Listed in `recently-used.xbel`.
    pub desktop: bool,
    /// Last use known from Ofolder's recents (Unix seconds).
    pub shared_at: Option<u64>,
}

impl Item {
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|n| files_core::name_label::label(n).into_owned())
            .unwrap_or_else(|| self.path.display().to_string())
    }
}

#[derive(Clone, Debug)]
pub enum Command {
    Close,
    Reload,
    Select(usize),
    Open,
    OpenContaining,
    /// Remove the entry from the list (never the file).
    Remove,
    Star,
    /// The row's menu (open, containing folder, remove, star).
    Menu,
}

pub struct Panel {
    pub id: u64,
    pub source: Source,
    pub items: Vec<Item>,
    /// Type-ahead filter on names.
    pub filter: String,
    /// Indices of `items` that pass the filter, in order.
    pub shown: Vec<usize>,
    /// Position in `shown`.
    pub selected: usize,
    pub loading: bool,
    pub error: String,
    /// Last explanation shown in the caption (refusals, removals).
    pub notice: String,
    scroll: i32,
    page: i32,
    load: Option<Receiver<Result<Vec<Item>, String>>>,
    removals: Vec<Receiver<Result<(), String>>>,
}

impl Panel {
    fn new(source: Source) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            source,
            items: Vec::new(),
            filter: String::new(),
            shown: Vec::new(),
            selected: 0,
            loading: false,
            error: String::new(),
            notice: String::new(),
            scroll: 0,
            page: 1,
            load: None,
            removals: Vec::new(),
        }
    }

    pub fn focused(&self) -> Option<&Item> {
        self.shown
            .get(self.selected)
            .and_then(|&i| self.items.get(i))
    }

    /// Recomputes the filtered rows, keeping the focused item when it stays.
    pub fn refilter(&mut self) {
        let keep = self.focused().map(|i| i.path.clone());
        let needle = self.filter.to_lowercase();
        self.shown = (0..self.items.len())
            .filter(|&i| needle.is_empty() || self.items[i].name().to_lowercase().contains(&needle))
            .collect();
        self.selected = keep
            .and_then(|p| self.shown.iter().position(|&i| self.items[i].path == p))
            .unwrap_or(0);
        self.reveal();
    }

    pub fn select(&mut self, index: usize) {
        self.selected = index.min(self.shown.len().saturating_sub(1));
        self.reveal();
    }

    fn reveal(&mut self) {
        let y = self.selected as i32 * ROW;
        if y < self.scroll {
            self.scroll = y;
        } else if y + ROW > self.scroll + self.page * ROW {
            self.scroll = y + ROW - self.page * ROW;
        }
        self.scroll = self.scroll.max(0);
    }

    /// Takes the focused item out of the list.
    fn take_focused(&mut self) -> Option<Item> {
        let index = *self.shown.get(self.selected)?;
        let item = self.items.remove(index);
        let selected = self.selected;
        self.refilter();
        self.select(selected);
        Some(item)
    }
}

/// Where the listings read from; injectable so tests never touch the
/// person's lists.
pub struct State {
    pub panel: Option<Panel>,
    pub data_home: PathBuf,
    pub ofolder_recents: PathBuf,
}

impl Default for State {
    fn default() -> Self {
        Self {
            panel: None,
            data_home: files_core::apps::data_home(),
            ofolder_recents: files_core::ofolder::Paths::from_env().recents,
        }
    }
}

/// Recent files and folders: the desktop list and Ofolder's, newest first,
/// existing paths only, entries the person removed left out.
pub fn load_recent(data_home: &Path, ofolder_recents: &Path) -> Result<Vec<Item>, String> {
    let desktop = files_core::recent::read_listing(data_home, MAX_LISTED)
        .map_err(|e| format!("No se pudieron leer los recientes: {e}"))?;
    let mut items: Vec<Item> = desktop
        .into_iter()
        .map(|r| Item {
            path: r.path,
            folder: r.folder,
            available: true,
            when: r.when,
            desktop: true,
            shared_at: None,
        })
        .collect();
    let hidden = files_core::recent_store::hidden::read(data_home);
    for r in files_core::ofolder::read_recents(ofolder_recents, MAX_LISTED) {
        let Ok(meta) = std::fs::metadata(&r.path) else {
            continue;
        };
        let when = files_core::ofolder::iso_utc(r.at);
        match items.iter_mut().find(|i| i.path == r.path) {
            Some(item) => {
                item.shared_at = Some(r.at);
                if item.when < when {
                    item.when = when;
                }
            }
            None => {
                if hidden.iter().any(|(p, at)| *p == r.path && r.at <= *at) {
                    continue;
                }
                items.push(Item {
                    path: r.path,
                    folder: meta.is_dir(),
                    available: true,
                    when,
                    desktop: false,
                    shared_at: Some(r.at),
                })
            }
        }
    }
    items.sort_by(|a, b| b.when.cmp(&a.when));
    items.truncate(MAX_LISTED);
    Ok(items)
}

/// Starred items in starring order, newest first; missing ones stay listed.
pub fn load_starred(list: &[PathBuf]) -> Vec<Item> {
    list.iter()
        .rev()
        .map(|path| {
            let meta = std::fs::metadata(path).ok();
            let when = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| files_core::ofolder::iso_utc(d.as_secs()))
                .unwrap_or_default();
            Item {
                path: path.clone(),
                folder: meta.as_ref().is_some_and(|m| m.is_dir()),
                available: meta.is_some(),
                when,
                desktop: false,
                shared_at: None,
            }
        })
        .collect()
}

/// The action that may run while a virtual listing is open, or None when it
/// was handled or refused here. File actions are refused with a reason.
pub fn gate(m: &mut Model, a: Action) -> Option<Action> {
    let Some(panel) = m.virtual_list.panel.as_ref() else {
        return Some(a);
    };
    let id = panel.id;
    match a {
        Action::None
        | Action::CancelClose
        | Action::DiscardClose
        | Action::RetrySaves
        | Action::Commands
        | Action::CommandSelect(..)
        | Action::CommandActivate(_)
        | Action::CommandRun
        | Action::CommandClose
        | Action::CommandFocus
        | Action::CommandRecapture
        | Action::Help
        | Action::HelpClose
        | Action::HelpScroll(_)
        | Action::ContextItem(_)
        | Action::Settings
        | Action::Actions
        | Action::RailPin(_)
        | Action::RailOrder(_)
        | Action::FocusRail
        | Action::FocusTabs
        | Action::ShowThumbnails(_)
        | Action::AutoFolderSizes(_)
        | Action::WindowMode(_)
        | Action::RecordRecentToggle
        | Action::OpenRecentView
        | Action::OpenStarredView
        | Action::VirtualList(..) => Some(a),
        Action::Open => Some(Action::VirtualList(id, Command::Open)),
        Action::StarToggle => Some(Action::VirtualList(id, Command::Star)),
        Action::OpenContaining => Some(Action::VirtualList(id, Command::OpenContaining)),
        Action::Refresh => Some(Action::VirtualList(id, Command::Reload)),
        // Back where the person was: the folder underneath never changed.
        Action::History(false) | Action::Parent => {
            m.close_virtual();
            None
        }
        Action::Navigate(_)
        | Action::GoHome
        | Action::History(true)
        | Action::OpenTrash
        | Action::Bookmarks
        | Action::Search
        | Action::Tab(_) => {
            m.close_virtual();
            Some(a)
        }
        _ => {
            m.refuse_virtual();
            None
        }
    }
}

impl Model {
    /// The `record_recent` preference: applies at once, saved off the UI thread.
    pub fn set_record_recent(&mut self, on: bool) {
        self.record_recent = on;
        crate::open_job::set_recording(on);
        let path = files_core::theme::config_dir().join("config.toml");
        let spawned = std::thread::Builder::new()
            .name("files-config-recent".into())
            .spawn(move || {
                if let Err(e) =
                    files_core::persistence::update_config(&path, "record_recent", on.into())
                {
                    eprintln!("FILES_CONFIG record_recent not saved: {e}");
                }
            });
        if let Err(e) = spawned {
            eprintln!("FILES_CONFIG record_recent not saved: {e}");
        }
        self.operation_message = Some(
            if on {
                "Recientes: Files registra los archivos que abre"
            } else {
                "Recientes: Files ya no registra los archivos que abre"
            }
            .into(),
        );
    }

    pub fn open_virtual(&mut self, source: Source) {
        if self.closing
            || self.trash_browser.is_some()
            || self.search.visible
            || self.preview.visible()
            || self.rename_dialog.is_some()
            || self.copy_form.is_some()
            || self.properties_dialog.is_some()
            || self.mutation_applied
            || self.recovery_catalog.open
            || self.size_panel.is_some()
            || self.checksums.is_some()
            || self.comparison.is_some()
            || self.bookmarks.panel.is_some()
            || self.saved_searches.panel.is_some()
            || self.sftp.form.is_some()
        {
            return;
        }
        self.end_path_edit();
        self.end_filter_edit();
        self.help = None;
        self.commands = None;
        self.context_menu = None;
        self.actions_menu = false;
        self.settings = false;
        self.wheel_scroll.cancel();
        self.hits.clear();
        let mut panel = Panel::new(source);
        self.start_virtual_load(&mut panel);
        self.virtual_list.panel = Some(panel);
    }

    fn start_virtual_load(&mut self, panel: &mut Panel) {
        let (tx, rx) = mpsc::sync_channel(1);
        let spawned = match panel.source {
            Source::Recent => {
                let data = self.virtual_list.data_home.clone();
                let shared = self.virtual_list.ofolder_recents.clone();
                std::thread::Builder::new()
                    .name("files-recent-list".into())
                    .spawn(move || {
                        let _ = tx.send(load_recent(&data, &shared));
                    })
            }
            Source::Starred => {
                let list = self.starred.list.clone();
                std::thread::Builder::new()
                    .name("files-starred-list".into())
                    .spawn(move || {
                        let _ = tx.send(Ok(load_starred(&list)));
                    })
            }
        };
        match spawned {
            Ok(_) => {
                panel.loading = true;
                panel.load = Some(rx);
            }
            Err(e) => panel.error = format!("No se pudo leer la lista: {e}"),
        }
    }

    pub fn close_virtual(&mut self) {
        if self.virtual_list.panel.take().is_some() {
            self.hits.clear();
            if matches!(
                self.context_menu.as_ref().map(|m| m.target),
                Some(crate::context_menu::Target::Virtual)
            ) {
                self.context_menu = None;
            }
        }
    }

    /// A file action asked for while a virtual listing is open.
    pub fn refuse_virtual(&mut self) {
        let Some(panel) = self.virtual_list.panel.as_mut() else {
            return;
        };
        let containing =
            crate::shortcuts::primary(self.shortcuts, crate::shortcuts::Cmd::OpenContaining);
        let text = format!(
            "{} es una lista: acciones sobre archivos desactivadas · Supr solo quita la entrada · {containing} abre su carpeta",
            panel.source.title()
        );
        panel.notice = text.clone();
        self.operation_message = Some(text);
    }

    /// Items of the open listing, as loaded.
    #[cfg(test)]
    pub fn virtual_items(&self) -> Vec<Item> {
        self.virtual_list
            .panel
            .as_ref()
            .map(|p| p.items.clone())
            .unwrap_or_default()
    }

    pub fn virtual_command(&mut self, id: u64, command: Command) {
        let Some(panel) = self.virtual_list.panel.as_mut().filter(|p| p.id == id) else {
            return;
        };
        match command {
            Command::Close => self.close_virtual(),
            Command::Reload => {
                let source = panel.source;
                let filter = panel.filter.clone();
                let mut fresh = Panel::new(source);
                fresh.id = id;
                fresh.filter = filter;
                self.start_virtual_load(&mut fresh);
                self.virtual_list.panel = Some(fresh);
                self.hits.clear();
            }
            Command::Select(i) => panel.select(i),
            Command::Open => {
                let Some(item) = panel.focused().cloned() else {
                    return;
                };
                if !item.available {
                    panel.notice = "No disponible: ya no existe en esa ruta · Supr lo quita".into();
                    return;
                }
                if item.folder {
                    self.close_virtual();
                    self.navigate(item.path);
                    return;
                }
                if self.open_job.is_some() {
                    panel.notice = "Ya hay una apertura en curso".into();
                    return;
                }
                match crate::open_job::Job::start(item.path) {
                    Ok(job) => {
                        self.open_job = Some(job);
                        panel.notice = "Abriendo con la aplicación predeterminada…".into();
                    }
                    Err(e) => panel.notice = e,
                }
            }
            Command::OpenContaining => {
                let Some(item) = panel.focused().cloned() else {
                    return;
                };
                if !item.path.parent().is_some_and(Path::is_dir) {
                    panel.notice = "La carpeta contenedora ya no existe".into();
                    return;
                }
                self.close_virtual();
                crate::places_sidebar::locate(self, &item.path);
            }
            Command::Remove => {
                let source = panel.source;
                let Some(item) = panel.take_focused() else {
                    return;
                };
                panel.notice = format!(
                    "Quitado de {}: {} · el archivo no se tocó",
                    source.title(),
                    item.name()
                );
                match source {
                    Source::Starred => self.set_starred(vec![item.path], false),
                    Source::Recent => {
                        let (tx, rx) = mpsc::sync_channel(1);
                        let data = self.virtual_list.data_home.clone();
                        let spawned = std::thread::Builder::new()
                            .name("files-recent-remove".into())
                            .spawn(move || {
                                let _ = tx.send(forget_recent(&data, &item));
                            });
                        match spawned {
                            Ok(_) => panel.removals.push(rx),
                            Err(e) => panel.error = e.to_string(),
                        }
                    }
                }
            }
            Command::Star => {
                let Some(item) = panel.focused().cloned() else {
                    return;
                };
                let on = !self.starred.contains(&item.path);
                if !on && panel.source == Source::Starred {
                    panel.take_focused();
                }
                panel.notice = if on {
                    format!("Destacado: {}", item.name())
                } else {
                    format!(
                        "Quitado de destacados: {} · el archivo no se tocó",
                        item.name()
                    )
                };
                self.set_starred(vec![item.path], on);
            }
            Command::Menu => {
                let (x, y) = self.virtual_row_anchor();
                crate::context_menu::open_virtual(self, x, y);
            }
        }
    }

    /// Centre of the focused row as last painted.
    fn virtual_row_anchor(&self) -> (i32, i32) {
        let Some(p) = self.virtual_list.panel.as_ref() else {
            return (0, 0);
        };
        self.hits
            .iter()
            .find(|h| matches!(h.action, Action::VirtualList(id, Command::Select(i)) if id == p.id && i == p.selected))
            .map(|h| (h.rect.x + 40, h.rect.y + h.rect.h / 2))
            .unwrap_or((self.listing_rect.x + 40, self.listing_rect.y + 60))
    }

    /// The starred set changed elsewhere: the Destacados listing follows.
    pub fn virtual_starred_changed(&mut self) {
        let starred = &self.starred;
        if let Some(panel) = self
            .virtual_list
            .panel
            .as_mut()
            .filter(|p| p.source == Source::Starred && !p.loading)
        {
            let before = panel.items.len();
            panel.items.retain(|i| starred.contains(&i.path));
            if panel.items.len() != before {
                panel.refilter();
            }
        }
    }

    pub fn pump_virtual(&mut self) -> bool {
        let Some(panel) = self.virtual_list.panel.as_mut() else {
            return false;
        };
        let mut dirty = false;
        if let Some(rx) = &panel.load {
            match rx.try_recv() {
                Err(mpsc::TryRecvError::Empty) => {}
                done => {
                    panel.load = None;
                    panel.loading = false;
                    match done {
                        Ok(Ok(items)) => {
                            panel.items = items;
                            panel.error.clear();
                        }
                        Ok(Err(e)) => panel.error = e,
                        Err(_) => panel.error = "La lectura terminó sin respuesta".into(),
                    }
                    panel.refilter();
                    dirty = true;
                }
            }
        }
        panel.removals.retain(|rx| match rx.try_recv() {
            Err(mpsc::TryRecvError::Empty) => true,
            Ok(Err(e)) => {
                panel.error = format!("No se pudo quitar de Recientes: {e}");
                dirty = true;
                false
            }
            _ => {
                dirty = true;
                false
            }
        });
        dirty
    }
}

/// Removes a recent entry from the desktop list and hides Ofolder's copy.
/// The file itself is never opened or changed.
pub fn forget_recent(data_home: &Path, item: &Item) -> Result<(), String> {
    use files_core::recent_store::{Outcome, hidden, remove};
    if item.desktop
        && let Outcome::Skipped(reason) = remove(data_home, &item.path)?
    {
        eprintln!("FILES_RECENT remove skipped: {reason}");
        return Err(format!("lista de recientes intacta ({reason})"));
    }
    if let Some(at) = item.shared_at {
        hidden::add(data_home, &item.path, at)?;
    }
    Ok(())
}

/// Paints the listing over the browser area and replaces its hit targets.
pub fn paint(p: &mut dyn Painter, m: &mut Model) {
    let Some(mut s) = m.virtual_list.panel.take() else {
        return;
    };
    let c = m.palette.clone();
    let area = m.listing_rect;
    let area = Rect::new(area.x - 8, area.y - 4, area.w + 16, area.h + 8);
    m.hits.retain(|h| {
        !(h.rect.x < area.x + area.w
            && h.rect.x + h.rect.w > area.x
            && h.rect.y < area.y + area.h
            && h.rect.y + h.rect.h > area.y)
    });
    p.rect(area, c.background);
    p.clip(area);
    let x = area.x + 8;
    let w = area.w - 16;
    let title = if s.filter.is_empty() {
        s.source.title().to_string()
    } else {
        format!("{} · filtro: {}", s.source.title(), s.filter)
    };
    p.text(
        Rect::new(x, area.y + 2, w - 220, 26),
        &title,
        16,
        c.bright_foreground,
        true,
        false,
    );
    let buttons = [
        ("Recargar", Command::Reload, 90),
        ("Cerrar · Esc", Command::Close, 110),
    ];
    let mut bx = x + w;
    for (label, command, bw) in buttons {
        bx -= bw + 6;
        let r = Rect::new(bx, area.y + 2, bw, 26);
        p.rounded_rect(r, 4, c.lighter_background);
        p.text(r, label, 11, c.foreground, false, false);
        m.hits.push(Hit {
            generation: m.generation,
            rect: r,
            action: Action::VirtualList(s.id, command),
        });
    }
    let caption = if s.loading {
        "Leyendo…".to_string()
    } else if !s.error.is_empty() {
        s.error.clone()
    } else if !s.notice.is_empty() {
        s.notice.clone()
    } else {
        format!(
            "{} elementos · lista virtual: las acciones sobre archivos están desactivadas",
            s.shown.len()
        )
    };
    p.text(
        Rect::new(x, area.y + 30, w, 20),
        &caption,
        11,
        if s.error.is_empty() {
            c.dark_foreground
        } else {
            c.red
        },
        false,
        false,
    );
    // Columns: name, containing folder, when.
    let when_w = 130.min(w / 4);
    let folder_w = (w / 3).max(0);
    let name_w = (w - folder_w - when_w - 44).max(40);
    let header_y = area.y + 52;
    for (cx, cw, label) in [
        (x + 36, name_w, "Nombre"),
        (x + 40 + name_w, folder_w, "Carpeta"),
        (
            x + w - when_w,
            when_w,
            if s.source == Source::Recent {
                "Último uso (UTC)"
            } else {
                "Modificado (UTC)"
            },
        ),
    ] {
        p.text(
            Rect::new(cx, header_y, cw, 18),
            label,
            10,
            c.dark_foreground,
            true,
            false,
        );
    }
    let footer = 22;
    let body = Rect::new(
        x,
        header_y + 20,
        w,
        (area.y + area.h - header_y - 20 - footer).max(ROW),
    );
    s.page = (body.h / ROW).max(1);
    let max = (s.shown.len() as i32 * ROW - s.page * ROW).max(0);
    s.scroll = s.scroll.clamp(0, max);
    p.clip(body);
    if !s.loading && s.error.is_empty() && s.shown.is_empty() {
        let empty = match (s.source, s.filter.is_empty()) {
            (_, false) => "Ningún nombre coincide con el filtro",
            (Source::Recent, true) => "No hay archivos recientes",
            (Source::Starred, true) => {
                "No hay destacados · Ctrl+E destaca el elemento enfocado en cualquier carpeta"
            }
        };
        p.text(body, empty, 13, c.dark_foreground, false, false);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let first = (s.scroll / ROW) as usize;
    for (row, &index) in s
        .shown
        .iter()
        .enumerate()
        .skip(first)
        .take(s.page as usize + 2)
    {
        let item = &s.items[index];
        let r = Rect::new(
            body.x,
            body.y + row as i32 * ROW - s.scroll,
            body.w,
            ROW - 2,
        );
        if row == s.selected {
            p.rounded_rect(r, 6, c.selection);
            p.rect(Rect::new(r.x + 1, r.y + 4, 2, (r.h - 8).max(0)), c.accent);
        }
        let glyph = if item.folder {
            crate::fonts::icon("\u{f07b}", "D")
        } else {
            crate::fonts::icon("\u{f15b}", "F")
        };
        let fg = if item.available {
            c.bright_foreground
        } else {
            c.dark_foreground
        };
        p.text(
            Rect::new(r.x + 6, r.y, 24, r.h),
            glyph,
            14,
            if item.folder { c.blue } else { fg },
            false,
            false,
        );
        if m.starred.contains(&item.path) {
            p.text(
                Rect::new(r.x + 20, r.y + 18, 14, 14),
                crate::fonts::icon("\u{f005}", "*"),
                9,
                c.yellow,
                true,
                false,
            );
        }
        let name = if item.available {
            item.name()
        } else {
            format!("{} · no disponible", item.name())
        };
        p.text(
            Rect::new(r.x + 36, r.y, name_w, r.h),
            &name,
            13,
            fg,
            false,
            false,
        );
        let folder = item.path.parent().map_or(String::new(), |parent| {
            match home.as_deref().and_then(|h| parent.strip_prefix(h).ok()) {
                Some(rest) if rest.as_os_str().is_empty() => "~".into(),
                Some(rest) => format!("~/{}", rest.display()),
                None => parent.display().to_string(),
            }
        });
        p.text(
            Rect::new(r.x + 40 + name_w, r.y, folder_w, r.h),
            &folder,
            11,
            c.foreground,
            false,
            false,
        );
        p.text(
            Rect::new(r.x + r.w - when_w, r.y, when_w, r.h),
            &files_core::recent::short_time(&item.when),
            11,
            c.dark_foreground,
            false,
            false,
        );
        let top = r.y.max(body.y);
        let bottom = (r.y + r.h).min(body.y + body.h);
        if bottom > top {
            m.hits.push(Hit {
                generation: m.generation,
                rect: Rect::new(r.x, top, r.w, bottom - top),
                action: Action::VirtualList(s.id, Command::Select(row)),
            });
        }
    }
    p.unclip();
    let profile = m.shortcuts;
    let chord = |cmd| crate::shortcuts::primary(profile, cmd);
    use crate::shortcuts::Cmd;
    let hint = format!(
        "Enter abrir · {} carpeta contenedora · Supr quitar de la lista · {} destacar · escribir filtra · Retroceso/Alt+← volver",
        chord(Cmd::OpenContaining),
        chord(Cmd::StarToggle)
    );
    p.text(
        Rect::new(x, area.y + area.h - footer, w, footer),
        &hint,
        10,
        c.dark_foreground,
        false,
        false,
    );
    p.unclip();
    m.virtual_list.panel = Some(s);
}

/// Keys and pointer of an open listing; always consumes the event.
pub fn event(m: &mut Model, ev: Event) -> bool {
    let Some(id) = m.virtual_list.panel.as_ref().map(|p| p.id) else {
        return false;
    };
    match ev {
        Event::Push => {
            let (x, y) = (app::event_x(), app::event_y());
            let double = app::event_clicks();
            // Only painted targets react: empty space never reaches the
            // browser underneath (no rubber band, no selection change).
            let hit = m
                .hits
                .iter()
                .rev()
                .find(|h| h.generation == m.generation && h.rect.contains(x, y))
                .map(|h| h.action.clone());
            let Some(action) = hit else {
                return true;
            };
            let row = matches!(action, Action::VirtualList(_, Command::Select(_)));
            m.action(action);
            if !row {
                return true;
            }
            if app::event_button() == 3 {
                m.virtual_command(id, Command::Menu);
            } else if double {
                m.virtual_command(id, Command::Open);
            }
        }
        Event::MouseWheel => {
            if let Some(p) = m.virtual_list.panel.as_mut() {
                let step = (app::event_dy_value() as f64 * 38.0) as i32;
                p.scroll = (p.scroll + step).max(0);
            }
        }
        Event::KeyDown | Event::Shortcut => {
            key(m, app::event_key(), app::event_state(), &app::event_text());
        }
        _ => {}
    }
    true
}

/// One key in an open listing (separate from FLTK for model tests).
pub fn key(m: &mut Model, key: Key, mods: Shortcut, text: &str) {
    use crate::shortcuts::{self, Cmd};
    let Some(panel) = m.virtual_list.panel.as_mut() else {
        return;
    };
    let id = panel.id;
    let profile = m.shortcuts;
    let is = |cmd| shortcuts::is(profile, cmd, key, mods);
    let plain = !mods.intersects(Shortcut::Ctrl | Shortcut::Alt | Shortcut::Meta);
    let command = if is(Cmd::OpenContaining) {
        Some(Command::OpenContaining)
    } else if is(Cmd::StarToggle) {
        Some(Command::Star)
    } else if is(Cmd::Refresh) && key != Key::F5 || key == Key::F5 && plain {
        Some(Command::Reload)
    } else if is(Cmd::ContextMenu) {
        Some(Command::Menu)
    } else if is(Cmd::Back) {
        Some(Command::Close)
    } else {
        None
    };
    if let Some(command) = command {
        m.virtual_command(id, command);
        return;
    }
    if is(Cmd::GoRecent) {
        m.action(Action::OpenRecentView);
        return;
    }
    if is(Cmd::GoStarred) {
        m.action(Action::OpenStarredView);
        return;
    }
    match key {
        Key::Escape => {
            if panel.filter.is_empty() {
                m.close_virtual();
            } else {
                panel.filter.clear();
                panel.refilter();
            }
        }
        Key::BackSpace if plain => {
            if panel.filter.pop().is_some() {
                panel.refilter();
            } else {
                m.close_virtual();
            }
        }
        Key::Enter | Key::KPEnter if plain && !mods.intersects(Shortcut::Shift) => {
            m.virtual_command(id, Command::Open)
        }
        Key::Delete if plain && !mods.intersects(Shortcut::Shift) => {
            m.virtual_command(id, Command::Remove)
        }
        Key::Down => panel.select(panel.selected.saturating_add(1)),
        Key::Up => panel.select(panel.selected.saturating_sub(1)),
        Key::PageDown => panel.select(panel.selected.saturating_add(panel.page as usize)),
        Key::PageUp => panel.select(panel.selected.saturating_sub(panel.page as usize)),
        Key::Home => panel.select(0),
        Key::End => panel.select(usize::MAX),
        _ if plain
            && !text.is_empty()
            && text.chars().all(|c| !c.is_control())
            && panel.filter.chars().count() < 128 =>
        {
            panel.filter.push_str(text);
            panel.refilter();
        }
        _ => {
            // F8 is the trash chord: here it removes the entry, like Supr.
            if key == Key::F8 && plain {
                m.virtual_command(id, Command::Remove);
            } else if shortcuts::resolve(profile, key, mods).is_some() {
                m.refuse_virtual();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aux_io::AuxIo;
    use files_core::listing::{Entry, Kind};
    use std::sync::atomic::AtomicU64;

    /// A model over a fresh private folder (under the private `TMPDIR`) whose
    /// listings read only that folder.
    fn fixture(label: &str) -> (Model, PathBuf) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "files-virtual-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join("data")).unwrap();
        let mut m = Model::new(root.clone());
        let (tx, _) = mpsc::sync_channel(8);
        let (_, rx) = mpsc::channel();
        m.aux = AuxIo { tx, rx };
        m.virtual_list.data_home = root.join("data");
        m.virtual_list.ofolder_recents = root.join("data/ofolder-recents.json");
        m.starred.data_home = root.join("data");
        m.loaded = true;
        (m, root)
    }

    fn file(root: &Path, name: &str) -> PathBuf {
        let path = root.join(name);
        std::fs::write(&path, b"contenido").unwrap();
        path
    }

    fn settle(m: &mut Model) {
        for _ in 0..500 {
            m.pump_virtual();
            m.pump_starred();
            let busy = m
                .virtual_list
                .panel
                .as_ref()
                .is_some_and(|p| p.loading || !p.removals.is_empty())
                || m.starred.pending();
            if !busy {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("listing did not settle");
    }

    fn focus(m: &mut Model, path: &Path) {
        let p = m.virtual_list.panel.as_mut().unwrap();
        let at = p
            .shown
            .iter()
            .position(|&i| p.items[i].path == path)
            .unwrap();
        p.select(at);
    }

    fn browser_entry(m: &mut Model, name: &str) {
        m.entries = vec![Entry {
            name: name.into(),
            kind: Kind::File,
            metadata: None,
            stat_attempted: false,
            link: None,
        }];
        m.rebuild();
        m.selected = Some(0);
        m.selection.insert(0);
    }

    #[test]
    fn virtual_listings_refuse_destructive_file_actions_at_the_model_level() {
        let (mut m, root) = fixture("refuse");
        let kept = file(&root, "conservar.txt");
        browser_entry(&mut m, "conservar.txt");
        files_core::recent_store::record(
            &root.join("data"),
            &kept,
            "text/plain",
            std::time::SystemTime::now(),
        )
        .unwrap();
        for open in [Action::OpenRecentView, Action::OpenStarredView] {
            m.action(open);
            settle(&mut m);
            for action in [
                Action::Trash,
                Action::DeletePermanently,
                Action::Rename,
                Action::ClipboardCut,
                Action::ClipboardCopy,
                Action::ClipboardPaste,
                Action::NativePaste,
                Action::MoveFiles,
                Action::CopyFiles,
                Action::NewEntry(true),
                Action::NewEntry(false),
                Action::CreateLinks,
                Action::FolderWithSelection,
                Action::Entry(0),
            ] {
                m.action(action);
                let panel = m.virtual_list.panel.as_ref().expect("listing stays open");
                assert!(panel.notice.contains("desactivadas"), "{}", panel.notice);
                assert!(m.delete.confirm.is_none());
                assert!(m.rename_dialog.is_none());
                assert!(m.copy_form.is_none());
                assert_eq!(m.clipboard_state.count(), 0);
            }
            // Shift+Supr and F2 from the keyboard are refused as well.
            key(&mut m, Key::Delete, Shortcut::Shift, "");
            key(&mut m, Key::F2, Shortcut::None, "");
            assert!(m.delete.confirm.is_none() && m.rename_dialog.is_none());
            assert!(kept.exists());
            // The row menu offers listing commands only, never file actions.
            if !m.virtual_items().is_empty() {
                key(&mut m, Key::F10, Shortcut::Shift, "");
                let menu = m.context_menu.take().expect("row menu");
                assert_eq!(menu.target, crate::context_menu::Target::Virtual);
                assert_eq!(menu.items.len(), 4);
                assert!(
                    menu.items
                        .iter()
                        .all(|i| matches!(i.intent, Some(Action::VirtualList(_, _))))
                );
            }
            key(&mut m, Key::Escape, Shortcut::None, "");
            assert!(m.virtual_list.panel.is_none());
        }
        assert_eq!(m.path, root, "the folder underneath never changed");
    }

    #[test]
    fn delete_in_recent_removes_only_the_entry_and_keeps_the_file() {
        let (mut m, root) = fixture("remove");
        let data = root.join("data");
        let gone = file(&root, "quitar de la lista.txt");
        let stays = file(&root, "sigue.txt");
        let t = std::time::SystemTime::now();
        files_core::recent_store::record(&data, &gone, "text/plain", t).unwrap();
        files_core::recent_store::record(
            &data,
            &stays,
            "text/plain",
            t + std::time::Duration::from_secs(1),
        )
        .unwrap();
        m.action(Action::OpenRecentView);
        settle(&mut m);
        let items = m.virtual_items();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].path, stays, "newest first");
        focus(&mut m, &gone);
        key(&mut m, Key::Delete, Shortcut::None, "");
        settle(&mut m);
        assert!(gone.exists(), "the file itself is untouched");
        assert_eq!(std::fs::read(&gone).unwrap(), b"contenido");
        let listed: Vec<_> = files_core::recent::read(&data)
            .unwrap()
            .into_iter()
            .map(|r| r.path)
            .collect();
        assert_eq!(listed, vec![stays.clone()]);
        assert_eq!(m.virtual_items().len(), 1);
        let panel = m.virtual_list.panel.as_ref().unwrap();
        assert!(panel.notice.contains("el archivo no se tocó"));
        assert!(panel.error.is_empty(), "{}", panel.error);
    }

    #[test]
    fn ofolder_only_recent_is_hidden_until_used_again() {
        let (mut m, root) = fixture("ofolder");
        let folder = root.join("proyecto");
        std::fs::create_dir(&folder).unwrap();
        let shared = root.join("data/ofolder-recents.json");
        std::fs::write(
            &shared,
            format!(
                "{{\"folders\":[{{\"path\":\"{}\",\"apps\":[],\"at\":1800000000}}],\"files\":[]}}",
                folder.display()
            ),
        )
        .unwrap();
        m.action(Action::OpenRecentView);
        settle(&mut m);
        assert_eq!(m.virtual_items().len(), 1);
        assert!(m.virtual_items()[0].folder);
        key(&mut m, Key::Delete, Shortcut::None, "");
        settle(&mut m);
        m.action(Action::Refresh);
        settle(&mut m);
        assert!(m.virtual_items().is_empty());
        assert!(folder.is_dir());
        // A newer use in Ofolder brings it back.
        std::fs::write(
            &shared,
            format!(
                "{{\"folders\":[{{\"path\":\"{}\",\"apps\":[],\"at\":1800000100}}],\"files\":[]}}",
                folder.display()
            ),
        )
        .unwrap();
        m.action(Action::Refresh);
        settle(&mut m);
        assert_eq!(m.virtual_items().len(), 1);
    }

    #[test]
    fn starred_listing_keeps_missing_items_removable_and_toggles_from_the_browser() {
        let (mut m, root) = fixture("starred");
        let a = file(&root, "a.txt");
        browser_entry(&mut m, "a.txt");
        m.action(Action::StarToggle);
        settle(&mut m);
        assert!(m.starred.contains(&a));
        let missing = root.join("ya no existe");
        m.set_starred(vec![missing.clone()], true);
        settle(&mut m);
        assert_eq!(
            files_core::starred::read(&root.join("data")).unwrap(),
            vec![a.clone(), missing.clone()]
        );
        m.action(Action::OpenStarredView);
        settle(&mut m);
        let items = m.virtual_items();
        assert_eq!(items[0].path, missing, "newest star first");
        assert!(!items[0].available && items[1].available);
        // Enter on a missing item explains instead of opening.
        key(&mut m, Key::Enter, Shortcut::None, "");
        assert!(m.open_job.is_none());
        key(&mut m, Key::Delete, Shortcut::None, "");
        settle(&mut m);
        assert_eq!(
            files_core::starred::read(&root.join("data")).unwrap(),
            vec![a.clone()]
        );
        // Ctrl+E in the listing unstars the focused item and drops its row.
        key(&mut m, Key::from_char('e'), Shortcut::Ctrl, "");
        settle(&mut m);
        assert!(m.virtual_items().is_empty());
        assert!(
            files_core::starred::read(&root.join("data"))
                .unwrap()
                .is_empty()
        );
        assert!(a.exists());
    }

    #[test]
    fn listings_are_keyboard_operable_and_return_where_you_were() {
        let (mut m, root) = fixture("keys");
        let sub = root.join("sub");
        std::fs::create_dir(&sub).unwrap();
        let inner = file(&sub, "nota.md");
        m.set_starred(vec![inner.clone(), sub.clone()], true);
        settle(&mut m);
        m.action(Action::OpenStarredView);
        settle(&mut m);
        assert_eq!(m.virtual_items().len(), 2);
        // Type-ahead filter, Backspace edits it, then goes back.
        key(&mut m, Key::from_char('n'), Shortcut::None, "n");
        key(&mut m, Key::from_char('o'), Shortcut::None, "o");
        let p = m.virtual_list.panel.as_ref().unwrap();
        assert_eq!(p.filter, "no");
        assert_eq!(p.shown.len(), 1);
        assert_eq!(p.focused().unwrap().path, inner);
        key(&mut m, Key::BackSpace, Shortcut::None, "");
        key(&mut m, Key::BackSpace, Shortcut::None, "");
        assert!(m.virtual_list.panel.is_some());
        key(&mut m, Key::BackSpace, Shortcut::None, "");
        assert!(m.virtual_list.panel.is_none());
        assert_eq!(m.path, root);
        // Alt+Left goes back too; the history is untouched.
        m.action(Action::OpenStarredView);
        settle(&mut m);
        key(&mut m, Key::Left, Shortcut::Alt, "");
        assert!(m.virtual_list.panel.is_none());
        assert_eq!(m.path, root);
        // Ctrl+Alt+O locates the item in its folder.
        m.action(Action::OpenStarredView);
        settle(&mut m);
        focus(&mut m, &inner);
        key(
            &mut m,
            Key::from_char('o'),
            Shortcut::Ctrl | Shortcut::Alt,
            "",
        );
        assert!(m.virtual_list.panel.is_none());
        assert_eq!(m.path, sub);
        assert!(
            m.search_locate
                .as_ref()
                .is_some_and(|(_, n)| n == "nota.md")
        );
        // Enter on a folder navigates there.
        m.action(Action::Navigate(root.clone()));
        assert_eq!(m.path, root);
        m.action(Action::OpenStarredView);
        settle(&mut m);
        focus(&mut m, &sub);
        key(&mut m, Key::End, Shortcut::None, "");
        key(&mut m, Key::Home, Shortcut::None, "");
        focus(&mut m, &sub);
        key(&mut m, Key::Enter, Shortcut::None, "");
        assert!(m.virtual_list.panel.is_none());
        assert_eq!(m.path, sub);
        // Ctrl+Alt+R / Ctrl+Alt+E switch between the two listings.
        m.action(Action::OpenStarredView);
        settle(&mut m);
        key(
            &mut m,
            Key::from_char('r'),
            Shortcut::Ctrl | Shortcut::Alt,
            "",
        );
        assert_eq!(
            m.virtual_list.panel.as_ref().map(|p| p.source),
            Some(Source::Recent)
        );
        settle(&mut m);
        key(
            &mut m,
            Key::from_char('e'),
            Shortcut::Ctrl | Shortcut::Alt,
            "",
        );
        assert_eq!(
            m.virtual_list.panel.as_ref().map(|p| p.source),
            Some(Source::Starred)
        );
        // Sidebar places still work: they close the listing first.
        let home = root.clone();
        m.action(Action::Navigate(home.clone()));
        assert!(m.virtual_list.panel.is_none());
    }
}

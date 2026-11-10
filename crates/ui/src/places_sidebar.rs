//! Places sidebar adapter: the rail rows (favorites, places, bookmarks,
//! devices, trash) as toolkit `Sidebar` items, each paired with the Files
//! action it runs. No filesystem work here; the model stays the source.
use crate::model::{Action, Model};
use fltk_omarchy::{popups::PopupItem, widgets::SidebarItem};
use std::path::{Path, PathBuf};

/// Rail widths, collapsed and expanded (the approved mockup's 52/170 px).
pub const RAIL: i32 = 52;
pub const WIDE: i32 = 170;

/// Glyph of the n-th XDG place (Home, Downloads, Documents, Pictures, Music,
/// Videos, then the filesystem root).
pub fn place_glyph(kind: usize) -> &'static str {
    match kind {
        0 => crate::fonts::icon("\u{f015}", "H"),
        1 => crate::fonts::icon("\u{f019}", "D"),
        2 => crate::fonts::icon("\u{f15c}", "T"),
        3 => crate::fonts::icon("\u{f03e}", "I"),
        4 => crate::fonts::icon("\u{f001}", "M"),
        5 => crate::fonts::icon("\u{f008}", "V"),
        _ => crate::fonts::icon("\u{f1c0}", "FS"),
    }
}

pub fn recent_glyph() -> &'static str {
    crate::fonts::icon("\u{f017}", "R")
}

pub fn star_glyph() -> &'static str {
    crate::fonts::icon("\u{f005}", "*")
}

pub fn trash_glyph() -> &'static str {
    crate::fonts::icon("\u{f1f8}", "TR")
}

pub fn bookmark_glyph() -> &'static str {
    crate::fonts::icon("\u{f02e}", "B")
}

pub fn device_glyph(removable: bool) -> &'static str {
    crate::fonts::icon(if removable { "\u{f287}" } else { "\u{f0a0}" }, "D")
}

pub fn pin_glyph() -> &'static str {
    crate::fonts::icon("\u{f08d}", "P")
}

pub fn network_glyph() -> &'static str {
    crate::fonts::icon("\u{f233}", "N")
}

pub fn connect_glyph() -> &'static str {
    crate::fonts::icon("\u{f067}", "+")
}

pub fn eject_glyph() -> &'static str {
    crate::fonts::icon("\u{f052}", "^")
}

/// The rail as the toolkit sees it: items, the action of each row (none for
/// headers), the secondary (eject) action of each row, and the active row.
#[derive(Clone, Default)]
pub struct Rail {
    pub items: Vec<SidebarItem>,
    pub actions: Vec<Option<Action>>,
    pub secondary: Vec<Option<Action>>,
    /// What each row stands for (context menu, drop target).
    pub kinds: Vec<Kind>,
    /// Index of the row drawn active, `usize::MAX` when none matches.
    pub active: usize,
}

/// What a rail row stands for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Header,
    Favorites,
    /// Recently used files (XDG recently-used.xbel), opened as a menu.
    Recent,
    Place(PathBuf),
    Bookmark(PathBuf),
    Device {
        node: PathBuf,
        mount: Option<PathBuf>,
        removable: bool,
    },
    /// A connected network location (GVfs), at its FUSE folder.
    Network(PathBuf),
    /// "Connect to server…".
    Connect,
    Trash,
}

impl Rail {
    /// Current visible sibling keys in a named section, in rendered order.
    pub fn section_keys(&self, parent: &str) -> Vec<String> {
        let Some(start) = self
            .items
            .iter()
            .position(|item| item.key == parent && item.header)
        else {
            return Vec::new();
        };
        self.items[start + 1..]
            .iter()
            .take_while(|item| !item.header && !item.footer)
            .map(|item| item.key.clone())
            .collect()
    }

    fn push(
        &mut self,
        item: SidebarItem,
        action: Option<Action>,
        secondary: Option<Action>,
        kind: Kind,
    ) {
        self.items.push(item);
        self.actions.push(action);
        self.secondary.push(secondary);
        self.kinds.push(kind);
    }

    /// The folder a drop on row `i` lands in: places, bookmarks and mounted
    /// devices; none for headers, Favorites, the trash or unmounted devices.
    pub fn drop_dir(&self, i: usize) -> Option<&Path> {
        match self.kinds.get(i)? {
            Kind::Place(p) | Kind::Bookmark(p) | Kind::Network(p) => Some(p),
            Kind::Device { mount, .. } => mount.as_deref(),
            _ => None,
        }
    }
}

pub fn rail(m: &Model, pinned: bool, order: &[String]) -> Rail {
    let mut out = Rail {
        active: usize::MAX,
        ..Rail::default()
    };
    let activate = |out: &mut Rail, on: bool| {
        if on && out.active == usize::MAX {
            out.active = out.items.len();
        }
    };
    out.push(
        SidebarItem::new(star_glyph(), "Favoritos").key("fav"),
        Some(Action::Bookmarks),
        None,
        Kind::Favorites,
    );
    out.push(
        SidebarItem::new(recent_glyph(), "Recientes").key("recent"),
        None,
        None,
        Kind::Recent,
    );
    out.push(
        SidebarItem::header("Lugares").key("h:places"),
        None,
        None,
        Kind::Header,
    );
    let place_keys: Vec<_> = m
        .places
        .iter()
        .map(|place| crate::sidebar_order::place_key(&place.path, &place.label))
        .collect();
    for i in crate::sidebar_order::ordered_indices(&place_keys, order) {
        let place = &m.places[i];
        activate(&mut out, m.path == place.path);
        out.push(
            SidebarItem::new(place_glyph(i), &place.label).key(&place_keys[i]),
            Some(Action::Navigate(place.path.clone())),
            None,
            Kind::Place(place.path.clone()),
        );
    }
    let marks: Vec<_> = m.bookmarks.document.rows.iter().collect();
    if !marks.is_empty() {
        out.push(
            SidebarItem::header("Marcadores").key("h:marks"),
            None,
            None,
            Kind::Header,
        );
    }
    for mark in marks {
        activate(&mut out, m.path == mark.path);
        out.push(
            SidebarItem::new(bookmark_glyph(), &mark.label).key(&format!("mark:{}", mark.id)),
            Some(Action::Navigate(mark.path.clone())),
            None,
            Kind::Bookmark(mark.path.clone()),
        );
    }
    if !m.devices.list.is_empty() {
        out.push(
            SidebarItem::header("Dispositivos")
                .collapsible()
                .key("h:devices"),
            None,
            None,
            Kind::Header,
        );
    }
    let busy = m.devices.busy_node();
    let device_keys: Vec<_> = m
        .devices
        .list
        .iter()
        .map(|d| crate::sidebar_order::path_key("dev", &d.node))
        .collect();
    for i in crate::sidebar_order::ordered_indices(&device_keys, order) {
        let d = &m.devices.list[i];
        let mounted = d.mount.is_some();
        activate(
            &mut out,
            d.mount.as_deref().is_some_and(|mp| m.path.starts_with(mp)),
        );
        let is_busy = busy == Some(&d.node);
        let detail = match (mounted, d.free) {
            (true, Some(free)) => format!("{} libres", files_core::format_size(free)),
            (true, None) => "montado".into(),
            (false, _) => "sin montar".into(),
        };
        let mut item = SidebarItem::new(device_glyph(d.removable), &d.title())
            .key(&device_keys[i])
            .detail(&detail)
            .busy(is_busy);
        if let (true, Some(free)) = (mounted, d.free)
            && d.size > 0
        {
            item = item.usage(1.0 - (free.min(d.size) as f64 / d.size as f64));
        }
        if !mounted {
            item = item.dimmed();
        }
        let eject = mounted.then_some(if d.removable {
            files_core::devices::Op::Eject
        } else {
            files_core::devices::Op::Unmount
        });
        if let Some(op) = eject {
            item = item.secondary(
                eject_glyph(),
                if op == files_core::devices::Op::Eject {
                    "Expulsar"
                } else {
                    "Desmontar"
                },
            );
        }
        out.push(
            item,
            Some(Action::Device(d.node.clone(), None)),
            eject.map(|op| Action::Device(d.node.clone(), Some(op))),
            Kind::Device {
                node: d.node.clone(),
                mount: d.mount.clone(),
                removable: d.removable,
            },
        );
    }
    network_rows(m, &mut out, &activate);
    // Pinning is a small trailing button on the last footer row rather than a
    // row of its own: it only matters while the rail is expanded.
    out.push(
        SidebarItem::new(trash_glyph(), "Papelera")
            .key("trash")
            .footer()
            .secondary(
                pin_glyph(),
                if pinned {
                    "Desfijar barra"
                } else {
                    "Fijar barra"
                },
            ),
        Some(Action::OpenTrash),
        Some(Action::RailPin(!pinned)),
        Kind::Trash,
    );
    out
}

/// The "Red" section: connected GVfs locations and "Connect to server…".
fn network_rows(m: &Model, out: &mut Rail, activate: &dyn Fn(&mut Rail, bool)) {
    out.push(
        SidebarItem::header("Red").collapsible().key("h:network"),
        None,
        None,
        Kind::Header,
    );
    let busy = m.network.busy();
    for share in &m.network.list {
        activate(out, m.path.starts_with(&share.path));
        let item = SidebarItem::new(network_glyph(), &share.title)
            .key(&crate::sidebar_order::path_key("net", &share.path))
            .detail(&share.detail())
            .secondary(eject_glyph(), "Desconectar");
        out.push(
            item,
            Some(Action::Navigate(share.path.clone())),
            Some(Action::NetworkDisconnect(share.path.clone())),
            Kind::Network(share.path.clone()),
        );
    }
    out.push(
        SidebarItem::new(connect_glyph(), "Conectar…")
            .key("net:connect")
            .busy(busy),
        Some(Action::NetworkConnect),
        None,
        Kind::Connect,
    );
}

/// What a context-menu entry of the rail does.
#[derive(Clone)]
pub enum Pick {
    /// Files actions, run in order.
    Run(Vec<Action>),
    /// Remove the bookmark of this path (the saved entry, as the panel does).
    RemoveBookmark(PathBuf),
    /// Open the recent files menu (the window does it).
    OpenRecent,
    MoveBookmark(String, usize),
}

impl std::fmt::Debug for Pick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Pick::Run(actions) => {
                let names: Vec<String> = actions.iter().map(action_name).collect();
                write!(f, "Run({})", names.join(","))
            }
            Pick::RemoveBookmark(p) => write!(f, "RemoveBookmark({})", p.display()),
            Pick::OpenRecent => write!(f, "OpenRecent"),
            Pick::MoveBookmark(key, index) => write!(f, "MoveBookmark({key},{index})"),
        }
    }
}

fn action_name(a: &Action) -> String {
    match a {
        Action::Navigate(p) => format!("Navigate({})", p.display()),
        Action::Device(n, op) => format!("Device({},{op:?})", n.display()),
        Action::Tab(crate::tabs::Command::New) => "TabNew".into(),
        Action::Bookmarks => "Bookmarks".into(),
        Action::BookmarkAdd => "BookmarkAdd".into(),
        Action::OpenTrash => "OpenTrash".into(),
        _ => "Other".into(),
    }
}

/// The context menu of rail row `i`: entries and what each one does.
pub fn menu(m: &Model, rail: &Rail, i: usize) -> Vec<(PopupItem, Pick)> {
    use files_core::devices::Op;
    let open = |p: &Path| Pick::Run(vec![Action::Navigate(p.to_path_buf())]);
    let tab = |p: &Path| {
        Pick::Run(vec![
            Action::Tab(crate::tabs::Command::New),
            Action::Navigate(p.to_path_buf()),
        ])
    };
    let folder = crate::fonts::icon("\u{f07c}", "");
    let plus = crate::fonts::icon("\u{f067}", "");
    let mut out = Vec::new();
    match rail.kinds.get(i) {
        Some(Kind::Favorites) => {
            out.push((
                PopupItem::new(star_glyph(), "Abrir favoritos"),
                Pick::Run(vec![Action::Bookmarks]),
            ));
            out.push((
                PopupItem::new(plus, "Añadir esta carpeta a favoritos"),
                Pick::Run(vec![Action::BookmarkAdd]),
            ));
        }
        Some(Kind::Recent) => out.push((
            PopupItem::new(recent_glyph(), "Abrir recientes"),
            Pick::OpenRecent,
        )),
        Some(Kind::Place(p)) => {
            out.push((PopupItem::new(folder, "Abrir"), open(p)));
            out.push((PopupItem::new(plus, "Abrir en pestaña nueva"), tab(p)));
        }
        Some(Kind::Bookmark(p)) => {
            out.push((PopupItem::new(folder, "Abrir"), open(p)));
            out.push((
                PopupItem::new(plus, "Abrir en pestaña nueva").divider(),
                tab(p),
            ));
            out.push((
                PopupItem::new(trash_glyph(), "Quitar de favoritos"),
                Pick::RemoveBookmark(p.clone()),
            ));
        }
        Some(Kind::Device {
            node,
            mount,
            removable,
        }) => {
            let run = |op| Pick::Run(vec![Action::Device(node.clone(), op)]);
            match mount {
                Some(mp) => {
                    out.push((PopupItem::new(folder, "Abrir"), open(mp)));
                    out.push((
                        PopupItem::new(plus, "Abrir en pestaña nueva").divider(),
                        tab(mp),
                    ));
                    out.push((
                        PopupItem::new(eject_glyph(), "Desmontar"),
                        run(Some(Op::Unmount)),
                    ));
                    if *removable {
                        out.push((
                            PopupItem::new(eject_glyph(), "Expulsar"),
                            run(Some(Op::Eject)),
                        ));
                    }
                }
                None => {
                    out.push((PopupItem::new(folder, "Montar y abrir"), run(None)));
                    out.push((PopupItem::new("", "Montar"), run(Some(Op::Mount))));
                }
            }
        }
        Some(Kind::Network(p)) => {
            out.push((PopupItem::new(folder, "Abrir"), open(p)));
            out.push((
                PopupItem::new(plus, "Abrir en pestaña nueva").divider(),
                tab(p),
            ));
            out.push((
                PopupItem::new(eject_glyph(), "Desconectar"),
                Pick::Run(vec![Action::NetworkDisconnect(p.clone())]),
            ));
        }
        Some(Kind::Connect) => out.push((
            PopupItem::new(connect_glyph(), "Conectar a servidor…"),
            Pick::Run(vec![Action::NetworkConnect]),
        )),
        Some(Kind::Trash) => out.push((
            PopupItem::new(trash_glyph(), "Abrir Papelera"),
            Pick::Run(vec![Action::OpenTrash]),
        )),
        Some(Kind::Header) | None => {}
    }
    let parent = match rail.kinds.get(i) {
        Some(Kind::Place(_)) => Some("h:places"),
        Some(Kind::Device { .. }) => Some("h:devices"),
        Some(Kind::Bookmark(_)) => Some("h:marks"),
        _ => None,
    };
    if let Some(parent) = parent {
        let keys = rail.section_keys(parent);
        if let Some(key) = rail.items.get(i).map(|item| &item.key)
            && let Some(index) = keys.iter().position(|k| k == key)
        {
            for (label, icon, target) in [
                ("Mover arriba", "", index.checked_sub(1)),
                (
                    "Mover abajo",
                    "",
                    (index + 1 < keys.len()).then_some(index + 1),
                ),
            ] {
                if let Some(target) = target {
                    let pick = if parent == "h:marks" {
                        Some(Pick::MoveBookmark(key.clone(), target))
                    } else {
                        crate::sidebar_order::moved(&m.rail_order, &keys, key, target)
                            .map(|order| Pick::Run(vec![Action::RailOrder(order)]))
                    };
                    if let Some(pick) = pick {
                        out.push((PopupItem::new(icon, label), pick));
                    }
                }
            }
        }
    }
    out
}

/// Sections whose rows can be reordered by dragging or with the row menu.
pub const ORDERED_SECTIONS: [&str; 3] = ["h:places", "h:devices", "h:marks"];

/// What dropping row `key` at `index` among the other rows of section
/// `parent` does: the same order change as "Mover arriba/abajo".
pub fn drag_pick(m: &Model, rail: &Rail, key: &str, parent: &str, index: usize) -> Option<Pick> {
    if !ORDERED_SECTIONS.contains(&parent) {
        return None;
    }
    let keys = rail.section_keys(parent);
    let from = keys.iter().position(|k| k == key)?;
    if from == index {
        return None;
    }
    if parent == "h:marks" {
        return Some(Pick::MoveBookmark(key.to_string(), index));
    }
    crate::sidebar_order::moved(&m.rail_order, &keys, key, index)
        .map(|order| Pick::Run(vec![Action::RailOrder(order)]))
}

/// Moves the bookmark identified by its stable key to `to` in document order.
pub fn move_bookmark(m: &mut Model, key: &str, to: usize) -> bool {
    let Some(id) = key
        .strip_prefix("mark:")
        .and_then(|s| s.parse::<u64>().ok())
    else {
        return false;
    };
    let entry = m
        .bookmarks
        .document
        .rows
        .iter()
        .find(|e| e.id == id)
        .cloned();
    match entry {
        Some(expected) if m.bookmarks.can_request() => {
            let ok = m
                .bookmarks
                .request(files_core::bookmarks::Op::Move { expected, to });
            if !ok {
                m.bookmarks.notice = "Espera, reintenta u omite el intento anterior".into();
            }
            ok
        }
        Some(_) => {
            m.bookmarks.notice = "Espera, reintenta u omite el intento anterior".into();
            false
        }
        None => false,
    }
}

/// The rail rows that may be dragged to reorder (bookmarks, among themselves).
#[cfg(test)]
pub fn movable(key: &str, parent: &str) -> bool {
    match parent {
        "h:marks" => key
            .strip_prefix("mark:")
            .is_some_and(|id| id.parse::<u64>().is_ok()),
        "h:places" => key.starts_with("place:"),
        "h:devices" => key.starts_with("dev:"),
        _ => false,
    }
}

/// Runs a rail menu pick on the model.
pub fn apply(m: &mut Model, pick: Pick) {
    match pick {
        Pick::Run(actions) => {
            for action in actions {
                m.action(action);
            }
        }
        Pick::RemoveBookmark(path) => {
            let entry = m
                .bookmarks
                .document
                .rows
                .iter()
                .find(|e| e.path == path)
                .cloned();
            match entry {
                Some(e) if m.bookmarks.can_request() => {
                    if !m.bookmarks.request(files_core::bookmarks::Op::Remove(e)) {
                        m.bookmarks.notice = "Espera, reintenta u omite el intento anterior".into();
                    }
                }
                Some(_) => {
                    m.bookmarks.notice = "Espera, reintenta u omite el intento anterior".into()
                }
                None => m.operation_message = Some("El favorito ya no existe".into()),
            }
        }
        Pick::OpenRecent => {}
        Pick::MoveBookmark(key, index) => {
            move_bookmark(m, &key, index);
        }
    }
}

/// Goes to the folder of `path` and selects it there (as the search and the
/// checksum report do); false when the folder could not be opened.
pub fn locate(m: &mut Model, path: &Path) -> bool {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return false;
    };
    let parent = parent.to_path_buf();
    m.navigate(parent.clone());
    if m.path != parent {
        return false;
    }
    m.query.clear();
    m.rebuild();
    m.search_locate = Some((m.generation, name.to_os_string()));
    true
}

/// Rows of the recent files menu: the name, then the folder (`~` for home)
/// and the time as detail.
pub fn recent_items(list: &[files_core::recent::Recent]) -> Vec<fltk_omarchy::overlay::MenuItem> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    list.iter()
        .map(|r| {
            let name = r
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let folder = r.path.parent().map_or(String::new(), |p| {
                match home.as_deref().and_then(|h| p.strip_prefix(h).ok()) {
                    Some(rest) if rest.as_os_str().is_empty() => "~".into(),
                    Some(rest) => format!("~/{}", rest.display()),
                    None => p.display().to_string(),
                }
            });
            fltk_omarchy::overlay::MenuItem::new(crate::fonts::icon("\u{f15b}", ""), &name).detail(
                &format!("{folder} · {}", files_core::recent::short_time(&r.when)),
            )
        })
        .collect()
}

/// True while a full-window or modal surface painted by the canvas covers
/// the browser (dialogs, previews, panels): the rail widget then hides so it
/// is not drawn over them and cannot take their input.
pub fn covered(m: &Model) -> bool {
    m.delete.confirm.is_some()
        || m.open_with.is_some()
        || m.trash_browser.is_some()
        || m.closing
        || m.comparison.is_some()
        || m.checksums.is_some()
        || m.sftp.form.is_some()
        || m.saved_searches.panel.is_some()
        || m.bookmarks.panel.is_some()
        || m.commands.is_some()
        || m.search.visible
        || m.size_panel.is_some()
        || m.help.is_some()
        || (m.context_menu.is_some() && !m.toolkit_popup)
        || m.properties_dialog.is_some()
        || m.recovery_catalog.open
        || m.notice_open
        || m.copy_form.is_some()
        || m.preview.visible()
        || m.rename_dialog.is_some()
        || m.mutation_applied
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rail_pairs_every_row_with_its_action_and_ends_with_trash() {
        let mut m = Model::new("/fixture-rail".into());
        m.loaded = true;
        let rail = rail(&m, false, &[]);
        assert_eq!(rail.items.len(), rail.actions.len());
        assert_eq!(rail.items.len(), rail.secondary.len());
        assert!(matches!(rail.actions[0], Some(Action::Bookmarks)));
        let last = rail.items.len() - 1;
        assert!(rail.items[last].footer);
        assert!(matches!(rail.actions[last], Some(Action::OpenTrash)));
        assert!(matches!(rail.secondary[last], Some(Action::RailPin(true))));
        assert_eq!(rail.items[last].secondary_tooltip, "Fijar barra");
        let pinned = super::rail(&m, true, &[]);
        assert!(matches!(
            pinned.secondary[last],
            Some(Action::RailPin(false))
        ));
        assert_eq!(pinned.items[last].secondary_tooltip, "Desfijar barra");
        for ((item, action), kind) in rail.items.iter().zip(&rail.actions).zip(&rail.kinds) {
            let window_only = matches!(kind, Kind::Recent);
            assert_eq!(
                item.header || window_only,
                action.is_none(),
                "{}",
                item.label
            );
        }
        assert_eq!(rail.kinds[1], Kind::Recent);
        let mut keys: Vec<_> = rail.items.iter().map(|i| i.key.clone()).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), rail.items.len(), "keys are unique");
    }

    fn labels(menu: &[(PopupItem, Pick)]) -> Vec<String> {
        menu.iter().map(|(i, _)| i.label.clone()).collect()
    }

    #[test]
    fn rail_menus_match_each_row_and_drop_dirs_only_folders() {
        use files_core::devices::Device;
        let mut m = Model::new("/fixture-rail".into());
        m.loaded = true;
        let mut doc = files_core::bookmarks::Document::default();
        doc.rows.push(files_core::bookmarks::Entry {
            id: 7,
            label: "Work".into(),
            path: "/fixture-rail/work".into(),
        });
        m.bookmarks.document = std::sync::Arc::new(doc);
        let device = |node: &str, mount: Option<&str>, removable| Device {
            node: node.into(),
            label: node.into(),
            fstype: "ext4".into(),
            size: 100,
            mount: mount.map(Into::into),
            removable,
            disk: None,
            free: Some(40),
        };
        m.devices.list = vec![
            device("/dev/sdz1", Some("/fixture-rail/usb"), true),
            device("/dev/sdy2", None, false),
        ];
        let rail = rail(&m, false, &[]);
        let at = |key: &str| rail.items.iter().position(|i| i.key == key).unwrap();
        let mark = at("mark:7");
        assert_eq!(rail.kinds[mark - 1], Kind::Header);
        assert_eq!(
            labels(&menu(&m, &rail, mark)),
            ["Abrir", "Abrir en pestaña nueva", "Quitar de favoritos"]
        );
        let usb = at(&crate::sidebar_order::path_key(
            "dev",
            Path::new("/dev/sdz1"),
        ));
        assert_eq!(
            labels(&menu(&m, &rail, usb)),
            // Devices reorder among themselves since the persistent sidebar order.
            [
                "Abrir",
                "Abrir en pestaña nueva",
                "Desmontar",
                "Expulsar",
                "Mover abajo"
            ]
        );
        assert!((rail.items[usb].usage.unwrap() - 0.6).abs() < 1e-9);
        let datos = at(&crate::sidebar_order::path_key(
            "dev",
            Path::new("/dev/sdy2"),
        ));
        assert!(rail.items[datos].dimmed && rail.items[datos].secondary_icon.is_empty());
        assert_eq!(
            labels(&menu(&m, &rail, datos)),
            ["Montar y abrir", "Montar", "Mover arriba"]
        );
        assert_eq!(labels(&menu(&m, &rail, at("trash"))), ["Abrir Papelera"]);
        assert!(menu(&m, &rail, at("h:devices")).is_empty());
        assert_eq!(rail.drop_dir(mark), Some(Path::new("/fixture-rail/work")));
        assert_eq!(rail.drop_dir(usb), Some(Path::new("/fixture-rail/usb")));
        for key in ["fav", "trash", "h:devices"] {
            assert_eq!(rail.drop_dir(at(key)), None, "{key}");
        }
        assert_eq!(rail.drop_dir(datos), None);
    }

    #[test]
    fn network_section_lists_shares_and_always_offers_connect() {
        let mut m = Model::new("/fixture-rail".into());
        m.loaded = true;
        let empty = rail(&m, false, &[]);
        let connect = empty
            .items
            .iter()
            .position(|i| i.key == "net:connect")
            .unwrap();
        assert_eq!(empty.items[connect - 1].key, "h:network");
        assert!(matches!(
            empty.actions[connect],
            Some(Action::NetworkConnect)
        ));
        assert_eq!(labels(&menu(&m, &empty, connect)), ["Conectar a servidor…"]);
        let path = Path::new("/fixture-rail/gvfs/smb-share:server=nas,share=fotos");
        m.network.list = vec![files_core::network::share(
            path,
            "smb-share:server=nas,share=fotos",
        )];
        m.path = path.join("2024");
        let rail = rail(&m, false, &[]);
        let row = rail
            .items
            .iter()
            .position(|i| i.label == "fotos en nas")
            .unwrap();
        assert_eq!(rail.items[row].detail, "SMB");
        assert_eq!(rail.active, row);
        assert_eq!(rail.drop_dir(row), Some(path));
        assert!(matches!(&rail.secondary[row], Some(Action::NetworkDisconnect(p)) if p == path));
        assert_eq!(
            labels(&menu(&m, &rail, row)),
            ["Abrir", "Abrir en pestaña nueva", "Desconectar"]
        );
        assert_eq!(rail.drop_dir(row + 1), None);
    }

    #[test]
    fn only_bookmarks_move_and_only_among_themselves() {
        assert!(movable("mark:7", "h:marks"));
        assert!(!movable("mark:7", "h:devices"));
        assert!(!movable("mark:/a", "h:marks"));
        assert!(!movable("place:aa", ""));
        assert!(movable("place:aa", "h:places"));
        assert!(movable("dev:aa", "h:devices"));
        assert!(!movable("trash", ""));
    }

    #[test]
    fn active_row_follows_the_open_place() {
        let mut m = Model::new("/fixture-rail".into());
        m.loaded = true;
        let Some(first) = m.places.first().map(|p| p.path.clone()) else {
            return;
        };
        m.path = first;
        let rail = rail(&m, false, &[]);
        assert_eq!(rail.active, 3);
        assert!(matches!(&rail.actions[3], Some(Action::Navigate(p)) if *p == m.path));
    }

    #[test]
    fn dragging_a_place_saves_the_same_order_as_the_row_menu() {
        let mut m = Model::new("/fixture-rail-drag".into());
        m.loaded = true;
        let rail = rail(&m, false, &[]);
        let keys = rail.section_keys("h:places");
        assert!(keys.len() >= 3, "{keys:?}");
        let Some(Pick::Run(actions)) = drag_pick(&m, &rail, &keys[0], "h:places", 2) else {
            panic!("a place drag saves an order");
        };
        let [Action::RailOrder(order)] = actions.as_slice() else {
            panic!("one RailOrder action");
        };
        let mut expected = keys.clone();
        let first = expected.remove(0);
        expected.insert(2, first);
        assert_eq!(order, &expected);
        assert!(drag_pick(&m, &rail, &keys[1], "h:places", 1).is_none());
        assert!(drag_pick(&m, &rail, "fav", "", 0).is_none());
        assert!(drag_pick(&m, &rail, &keys[0], "h:devices", 0).is_none());
    }
}

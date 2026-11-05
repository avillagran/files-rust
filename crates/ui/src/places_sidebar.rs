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
    Place(PathBuf),
    Bookmark(PathBuf),
    Device {
        node: PathBuf,
        mount: Option<PathBuf>,
        removable: bool,
    },
    Trash,
}

impl Rail {
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
            Kind::Place(p) | Kind::Bookmark(p) => Some(p),
            Kind::Device { mount, .. } => mount.as_deref(),
            _ => None,
        }
    }
}

/// Bookmarks shown in the rail (the first eight, as before).
pub const MAX_BOOKMARKS: usize = 8;

pub fn rail(m: &Model) -> Rail {
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
    for (i, place) in m.places.iter().enumerate() {
        activate(&mut out, m.path == place.path);
        out.push(
            SidebarItem::new(place_glyph(i), &place.label)
                .key(&format!("place:{}", place.path.display())),
            Some(Action::Navigate(place.path.clone())),
            None,
            Kind::Place(place.path.clone()),
        );
    }
    let marks: Vec<_> = m
        .bookmarks
        .document
        .rows
        .iter()
        .take(MAX_BOOKMARKS)
        .collect();
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
            SidebarItem::new(bookmark_glyph(), &mark.label)
                .key(&format!("mark:{}", mark.path.display())),
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
    for d in &m.devices.list {
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
            .key(&format!("dev:{}", d.node.display()))
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
    out.push(
        SidebarItem::new(trash_glyph(), "Papelera")
            .key("trash")
            .footer(),
        Some(Action::OpenTrash),
        None,
        Kind::Trash,
    );
    out
}

/// What a context-menu entry of the rail does.
#[derive(Clone)]
pub enum Pick {
    /// Files actions, run in order.
    Run(Vec<Action>),
    /// Remove the bookmark of this path (the saved entry, as the panel does).
    RemoveBookmark(PathBuf),
}

impl std::fmt::Debug for Pick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Pick::Run(actions) => {
                let names: Vec<String> = actions.iter().map(action_name).collect();
                write!(f, "Run({})", names.join(","))
            }
            Pick::RemoveBookmark(p) => write!(f, "RemoveBookmark({})", p.display()),
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
        Some(Kind::Trash) => out.push((
            PopupItem::new(trash_glyph(), "Abrir Papelera"),
            Pick::Run(vec![Action::OpenTrash]),
        )),
        Some(Kind::Header) | None => {}
    }
    let _ = m;
    out
}

/// Moves the bookmark of `path` to position `to` among the bookmarks (the
/// rail shows the first ones in document order, so its index is theirs).
pub fn move_bookmark(m: &mut Model, path: &Path, to: usize) -> bool {
    let entry = m
        .bookmarks
        .document
        .rows
        .iter()
        .find(|e| e.path == path)
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
pub fn movable(key: &str, parent: &str) -> bool {
    key.starts_with("mark:") && parent == "h:marks"
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
    }
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
        let rail = rail(&m);
        assert_eq!(rail.items.len(), rail.actions.len());
        assert_eq!(rail.items.len(), rail.secondary.len());
        assert!(matches!(rail.actions[0], Some(Action::Bookmarks)));
        let last = rail.items.len() - 1;
        assert!(rail.items[last].footer);
        assert!(matches!(rail.actions[last], Some(Action::OpenTrash)));
        for (item, action) in rail.items.iter().zip(&rail.actions) {
            assert_eq!(item.header, action.is_none(), "{}", item.label);
        }
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
        let rail = rail(&m);
        let at = |key: &str| rail.items.iter().position(|i| i.key == key).unwrap();
        let mark = at("mark:/fixture-rail/work");
        assert_eq!(rail.kinds[mark - 1], Kind::Header);
        assert_eq!(
            labels(&menu(&m, &rail, mark)),
            ["Abrir", "Abrir en pestaña nueva", "Quitar de favoritos"]
        );
        let usb = at("dev:/dev/sdz1");
        assert_eq!(
            labels(&menu(&m, &rail, usb)),
            ["Abrir", "Abrir en pestaña nueva", "Desmontar", "Expulsar"]
        );
        assert!((rail.items[usb].usage.unwrap() - 0.6).abs() < 1e-9);
        let datos = at("dev:/dev/sdy2");
        assert!(rail.items[datos].dimmed && rail.items[datos].secondary_icon.is_empty());
        assert_eq!(
            labels(&menu(&m, &rail, datos)),
            ["Montar y abrir", "Montar"]
        );
        assert_eq!(labels(&menu(&m, &rail, at("trash"))), ["Abrir Papelera"]);
        assert!(menu(&m, &rail, at("h:devices")).is_empty());
        assert_eq!(rail.drop_dir(mark), Some(Path::new("/fixture-rail/work")));
        assert_eq!(rail.drop_dir(usb), Some(Path::new("/fixture-rail/usb")));
        for key in ["fav", "trash", "h:devices", "dev:/dev/sdy2"] {
            assert_eq!(rail.drop_dir(at(key)), None, "{key}");
        }
    }

    #[test]
    fn only_bookmarks_move_and_only_among_themselves() {
        assert!(movable("mark:/a", "h:marks"));
        assert!(!movable("mark:/a", "h:devices"));
        assert!(!movable("place:/home", ""));
        assert!(!movable("dev:/dev/sdz1", "h:devices"));
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
        let rail = rail(&m);
        assert_eq!(rail.active, 1);
        assert!(matches!(&rail.actions[1], Some(Action::Navigate(p)) if *p == m.path));
    }
}

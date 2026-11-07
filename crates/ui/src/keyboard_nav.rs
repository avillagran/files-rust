//! Browser key navigation. No event globals, filesystem work or per-key order clone.
use crate::model::{Model, View};
use fltk::enums::Key;
use std::{
    collections::VecDeque,
    ffi::OsString,
    path::{Path, PathBuf},
};

const RETURN_POSITIONS: usize = 16;

struct Position {
    owner: (u64, u64),
    parent: PathBuf,
    child: OsString,
    view: View,
    dual: bool,
    scroll: i32,
}
struct PendingReturn {
    owner: (u64, u64),
    generation: u64,
    parent: PathBuf,
    child: OsString,
    position: Option<Position>,
}
#[derive(Default)]
pub(crate) struct ParentReturn {
    positions: VecDeque<Position>,
    pending: Option<PendingReturn>,
}
impl ParentReturn {
    pub(crate) fn rebind_refresh(&mut self, owner: (u64, u64), path: &Path, old: u64, new: u64) {
        if let Some(pending) = &mut self.pending
            && pending.owner == owner
            && pending.parent == path
            && pending.generation == old
        {
            pending.generation = new;
        }
    }
}
impl Model {
    pub(crate) fn cancel_parent_return_focus(&mut self) {
        self.parent_return.pending = None;
    }
    pub(crate) fn remember_parent_return_position(&mut self, next: &Path) {
        self.cancel_parent_return_focus();
        if !self.loaded || next.parent() != Some(self.path.as_path()) {
            return;
        }
        let Some(child) = next.file_name() else {
            return;
        };
        if self.path.as_os_str().len() > 4096 || child.len() > 4096 {
            return;
        }
        let owner = self.pane_owner();
        self.parent_return
            .positions
            .retain(|p| p.owner != owner || p.parent != self.path || p.child != child);
        if self.parent_return.positions.len() == RETURN_POSITIONS {
            self.parent_return.positions.pop_front();
        }
        self.parent_return.positions.push_back(Position {
            owner,
            parent: self.path.clone(),
            child: child.to_owned(),
            view: self.view,
            dual: self.dual.enabled,
            scroll: self.scroll.max(0),
        });
    }
    pub(crate) fn pump_parent_return_focus(&mut self) -> bool {
        let Some(pending) = self.parent_return.pending.as_ref() else {
            return false;
        };
        if self.closing
            || !self.error.is_empty()
            || pending.owner != self.pane_owner()
            || pending.generation != self.generation
            || pending.parent != self.path
        {
            self.cancel_parent_return_focus();
            return false;
        }
        if !self.loaded || self.selection_refreshing() {
            return false;
        }
        let pending = self
            .parent_return
            .pending
            .take()
            .expect("ready parent return");
        let target = self
            .order
            .iter()
            .copied()
            .find(|&i| self.entries[i].name == pending.child && self.entries[i].is_dir());
        if let Some(index) = target {
            self.select_entry(index, false, false);
            if let Some(position) = pending.position
                && position.view == self.view
                && position.dual == self.dual.enabled
            {
                self.scroll = position.scroll;
            }
            // Painting reveals only as much as necessary in the current
            // geometry after applying the retained scroll.
            self.search_reveal = true;
        } else {
            // A renamed or removed child must not select its new neighbour.
            self.selection.clear();
            self.selected = None;
            self.anchor = None;
            self.preserve_empty_selection = true;
        }
        true
    }
}

/// Context-sensitive F5, shared by the real event dispatcher and owned fixtures.
pub fn f5_action(m: &Model) -> crate::model::Action {
    if crate::selection_tools::is_map(m) {
        crate::model::Action::SpaceRefresh
    } else {
        crate::model::Action::CopyFiles
    }
}
pub fn f5(m: &mut Model) {
    m.action(f5_action(m));
}
/// Listing focus: parent navigation, even when a retained filter is nonempty.
/// Focused input consumes Backspace before reaching this browser handler.
pub fn backspace(m: &mut Model) {
    let Some(parent) = m.path.parent().map(Path::to_path_buf) else {
        return;
    };
    let Some(child) = m.path.file_name().map(std::ffi::OsStr::to_owned) else {
        return;
    };
    let owner = m.pane_owner();
    let generation = m.generation;
    m.navigate(parent.clone());
    if m.path != parent || m.generation == generation || m.pane_owner() != owner {
        return;
    }
    let position = m
        .parent_return
        .positions
        .iter()
        .position(|p| p.owner == owner && p.parent == parent && p.child == child)
        .and_then(|index| m.parent_return.positions.remove(index));
    m.parent_return.pending = Some(PendingReturn {
        owner,
        generation: m.generation,
        parent,
        child,
        position,
    });
}
/// Saturating rank movement; an unfocused Down starts at the first row, Up at last.
pub fn target(order: &[usize], selected: Option<usize>, delta: i32) -> Option<usize> {
    if order.is_empty() {
        return None;
    }
    if delta == i32::MIN {
        return Some(0);
    }
    if delta == i32::MAX {
        return Some(order.len() - 1);
    }
    Some(
        match selected.and_then(|i| order.iter().position(|&j| j == i)) {
            Some(p) => (p as i64 + i64::from(delta)).clamp(0, order.len() as i64 - 1) as usize,
            None => {
                if delta < 0 {
                    order.len() - 1
                } else {
                    0
                }
            }
        },
    )
}
pub fn key(m: &mut Model, key: Key, ctrl: bool, shift: bool, w: i32, h: i32) -> bool {
    if !matches!(
        key,
        Key::Up
            | Key::Down
            | Key::Left
            | Key::Right
            | Key::Home
            | Key::End
            | Key::PageUp
            | Key::PageDown
    ) {
        return false;
    }
    if m.editing || m.closing || m.selection_refreshing() {
        return true;
    }
    if m.view == View::Columns
        && !m.dual.enabled
        && !ctrl
        && !shift
        && matches!(key, Key::Left | Key::Right)
    {
        m.column_step(key == Key::Right);
        return true;
    }
    let grid = m.view == View::Grid && !m.dual.enabled;
    let groups = m.view == View::Groups && !m.dual.enabled;
    let folder = groups
        && m.selected
            .and_then(|i| m.entries.get(i))
            .is_some_and(|e| e.is_dir());
    let metrics =
        crate::grid_view::Metrics::new(crate::grid_view::width(m, w), m.grid_zoom, m.order.len());
    let columns = if grid {
        metrics.columns as i32
    } else if folder {
        let rail = m.rail_width();
        let inspector = if m.inspector && w >= 800 { 240 } else { 0 };
        ((w - inspector - rail - 32) / 240).max(1)
    } else {
        1
    };
    let h = crate::operation_bar::browser_height(m, h);
    let page = if m.view == View::SpaceMap && !m.dual.enabled {
        m.space_geometry.page_rows()
    } else if grid {
        columns * ((h - 170) / metrics.cell_height).max(1)
    } else if m.dual.enabled || m.view == View::Columns {
        ((h - 230) / 36).max(1)
    } else if folder {
        columns * ((h - 162) / 82).max(1)
    } else {
        ((h - 164) / 38).max(1)
    };
    let mut delta = match key {
        Key::Home => i32::MIN,
        Key::End => i32::MAX,
        Key::Up => -columns,
        Key::Down => columns,
        Key::Left => -1,
        Key::Right => 1,
        Key::PageUp => -page,
        Key::PageDown => page,
        _ => unreachable!(),
    };
    if folder && matches!(key, Key::Down | Key::PageDown) {
        let count = m.order.partition_point(|&i| m.entries[i].is_dir());
        if let Some(rank) = m
            .selected
            .and_then(|i| m.order.iter().position(|&j| j == i))
            && count < m.order.len()
            && rank.saturating_add(delta as usize) >= count
        {
            delta = (count - rank) as i32;
        }
    }
    m.move_browser_focus(delta, shift, ctrl);
    crate::view::reveal_selection(m, w, h);
    true
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_focus_boundaries_and_extremes_never_skip_or_overflow() {
        let a = [4, 2, 7];
        assert_eq!(target(&[], None, 1), None);
        assert_eq!(target(&a, None, 1), Some(0));
        assert_eq!(target(&a, None, -1), Some(2));
        assert_eq!(target(&a, Some(2), i32::MIN), Some(0));
        assert_eq!(target(&a, Some(2), i32::MAX), Some(2));
        assert_eq!(target(&a, Some(4), -200), Some(0));
        assert_eq!(target(&a, Some(7), 200), Some(2));
    }
    #[cfg(unix)]
    mod parent_returns {
        use super::*;
        use files_core::listing::{Entry, Kind};
        use std::os::unix::ffi::OsStringExt;
        fn fixture() -> PathBuf {
            let root = crate::thumbnail_disk_cache::retained_test_directory("parent-return");
            std::fs::create_dir(root.join("A")).unwrap();
            std::fs::create_dir(root.join("A/B")).unwrap();
            std::fs::create_dir(root.join("A/B/Leaf")).unwrap();
            root
        }
        fn model(path: PathBuf) -> Model {
            let mut m = Model::new(path);
            let (tx, _) = std::sync::mpsc::sync_channel(4);
            let (_, rx) = std::sync::mpsc::channel();
            m.aux = crate::aux_io::AuxIo { tx, rx };
            m.view = View::List;
            m.auto_folder_sizes = false;
            m
        }
        fn listing(m: &mut Model, names: &[OsString]) {
            m.entries = names
                .iter()
                .cloned()
                .map(|name| Entry {
                    name,
                    kind: Kind::Directory,
                    metadata: None,
                    stat_attempted: true,
                })
                .collect();
            m.loaded = true;
            m.rebuild();
        }
        fn focused(m: &Model) -> Option<&std::ffi::OsStr> {
            m.selected
                .and_then(|i| m.entries.get(i))
                .map(|e| e.name.as_os_str())
        }
        struct NoopPainter;
        impl crate::painter::Painter for NoopPainter {
            fn rect(&mut self, _: crate::painter::Rect, _: files_core::theme::Color) {}
            fn rounded_rect(
                &mut self,
                _: crate::painter::Rect,
                _: i32,
                _: files_core::theme::Color,
            ) {
            }
            fn text(
                &mut self,
                _: crate::painter::Rect,
                _: &str,
                _: i32,
                _: files_core::theme::Color,
                _: bool,
                _: bool,
            ) {
            }
            fn image(&mut self, _: crate::painter::Rect, _: &crate::painter::Image<'_>) {}
            fn clip(&mut self, _: crate::painter::Rect) {}
            fn unclip(&mut self) {}
        }
        #[test]
        fn enter_then_parent_restores_exact_raw_child_and_retained_scroll() {
            let root = fixture();
            let raw = OsString::from_vec(b"Folder20-\xff\n".to_vec());
            std::fs::create_dir(root.join("A").join(&raw)).unwrap();
            let mut names = (0..100)
                .map(|n| OsString::from(format!("Folder{n:02}")))
                .collect::<Vec<_>>();
            names[20] = raw.clone();
            let mut m = model(root.join("A"));
            listing(&mut m, &names);
            m.select_entry(20, false, false);
            m.scroll = 500;
            m.open_selected();
            assert_eq!(m.path, root.join("A").join(&raw));
            backspace(&mut m);
            assert_eq!(m.path, root.join("A"));
            assert!(!m.pump_parent_return_focus(), "wait for the parent listing");
            names.swap(20, 73);
            listing(&mut m, &names);
            assert!(m.pump_parent_return_focus());
            assert_eq!(focused(&m), Some(raw.as_os_str()));
            assert_eq!(m.selected, m.anchor);
            assert_eq!(m.selection.iter().copied().collect::<Vec<_>>(), vec![73]);
            assert_eq!(m.scroll, 500);
            assert!(m.search_reveal);
            // The first return frame still has the empty child's painted
            // height. Idle wheel state must not clamp against that stale bound.
            m.content_height = 0;
            for _ in 0..2 {
                assert!(!crate::wheel_scroll::advance_offsets(&mut m, 650));
                assert_eq!(
                    m.scroll, 500,
                    "idle wheel must retain the restored position"
                );
                crate::view::paint(&mut NoopPainter, &mut m, 1000, 650);
                m.clamp_scroll(650 - 134);
                assert_eq!(
                    m.scroll, 500,
                    "the current geometry permits the retained scroll"
                );
                assert!(m.visible.contains(&m.selected.unwrap()));
                assert_eq!(focused(&m), Some(raw.as_os_str()));
            }
        }
        #[test]
        fn pending_parent_focus_rebinds_refresh_and_rejects_changed_owner_or_generation() {
            for change in 0..3 {
                let root = fixture();
                let mut m = model(root.join("A/B"));
                backspace(&mut m);
                if change == 0 {
                    m.refresh_current();
                } else if change == 1 {
                    m.generation = m.generation.wrapping_add(1);
                } else {
                    m.tabs.active = m.tabs.fresh_id();
                }
                listing(&mut m, &["other".into(), "B".into()]);
                m.refresh_selection = None;
                if change == 0 {
                    assert!(m.pump_parent_return_focus());
                    assert_eq!(focused(&m), Some(std::ffi::OsStr::new("B")));
                } else {
                    let before = m.selected;
                    assert!(!m.pump_parent_return_focus());
                    assert_eq!(m.selected, before);
                }
                assert!(m.parent_return.pending.is_none());
            }
        }
        #[test]
        fn rapid_multilevel_parent_navigation_keeps_only_the_latest_child() {
            let root = fixture();
            let mut m = model(root.join("A/B/Leaf"));
            backspace(&mut m);
            backspace(&mut m);
            assert_eq!(m.path, root.join("A"));
            listing(&mut m, &["Leaf".into(), "B".into()]);
            assert!(m.pump_parent_return_focus());
            assert_eq!(focused(&m), Some(std::ffi::OsStr::new("B")));
            m.path = PathBuf::from("/");
            let generation = m.generation;
            backspace(&mut m);
            assert_eq!(m.path, Path::new("/"));
            assert_eq!(m.generation, generation, "root is a no-op");
        }
        #[test]
        fn explicit_selection_or_missing_child_never_restores_a_neighbour() {
            let root = fixture();
            let mut m = model(root.join("A/B"));
            backspace(&mut m);
            listing(&mut m, &["B".into(), "other".into()]);
            m.select_entry(1, false, false);
            assert!(!m.pump_parent_return_focus());
            assert_eq!(focused(&m), Some(std::ffi::OsStr::new("other")));
            m.navigate(root.join("A/B"));
            backspace(&mut m);
            listing(&mut m, &["other".into()]);
            assert!(m.pump_parent_return_focus());
            assert!(m.selected.is_none() && m.anchor.is_none() && m.selection.is_empty());
            assert!(m.preserve_empty_selection);
        }
    }
}

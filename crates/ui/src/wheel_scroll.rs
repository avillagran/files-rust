//! Adapts the toolkit's shared motion to the bounded canvas scroll offsets.
use crate::{
    columns::MAX_COLUMNS,
    model::{Model, View},
};
use fltk::{frame::Frame, prelude::*};
use fltk_omarchy::{anim, scroll::Scroll};

const SLOTS: usize = MAX_COLUMNS + 3;
#[derive(PartialEq, Eq)]
struct Scope {
    generation: u64,
    view: View,
    active: u64,
    other: Option<(u64, u64)>,
    dual: bool,
    column: usize,
    columns: [u64; MAX_COLUMNS],
}

#[derive(Default)]
pub struct State {
    scope: Option<Scope>,
    slots: [Scroll; SLOTS],
    /// Where a keyboard glide is heading, per slot. Only meaningful while the
    /// slot is running; the model's offset is the painted value.
    targets: [Option<i32>; SLOTS],
}
impl State {
    pub fn running(&self) -> bool {
        self.slots.iter().any(Scroll::running)
    }
    /// The offset a keyboard reveal must be computed against: the glide's
    /// destination while one is in flight, otherwise the painted offset.
    fn logical(&self, slot: usize, painted: i32) -> i32 {
        match self.targets[slot] {
            Some(t) if self.slots[slot].running() => t,
            _ => painted,
        }
    }
    /// Starts (or retargets) a glide from the painted offset to `to`.
    fn glide(&mut self, slot: usize, from: i32, to: i32, max: i32) -> i32 {
        if to == self.logical(slot, from) {
            return from;
        }
        self.slots[slot].jump(from as i64);
        let v = self.slots[slot].wheel(from as i64, (to - from) as f64, max as i64) as i32;
        self.targets[slot] = Some(to);
        v
    }
    pub fn cancel(&mut self) {
        *self = Self::default();
    }
    fn sync(&mut self, m: &Model) {
        let scope = Scope {
            generation: m.generation,
            view: m.view,
            active: m.dual.active_id,
            other: m.dual.other.as_ref().map(|b| (b.id, b.revision)),
            dual: m.dual.enabled,
            column: m.columns.active,
            columns: std::array::from_fn(|i| m.columns.columns.get(i).map_or(0, |c| c.id)),
        };
        if self.scope.as_ref() != Some(&scope) {
            self.cancel();
            self.scope = Some(scope);
        }
    }
}

fn offsets(m: &Model, h: i32) -> [Option<(i32, i32)>; SLOTS] {
    let mut out = [None; SLOTS];
    let browser_h = crate::operation_bar::browser_height(m, h);
    if m.dual.enabled {
        let available = (browser_h - 226).max(0);
        out[0] = Some((m.scroll, (m.order.len() as i32 * 36 - available).max(0)));
        if let Some(b) = &m.dual.other {
            out[1] = Some((b.scroll, (b.order.len() as i32 * 36 - available).max(0)));
        }
    } else if m.view == View::Columns {
        let available = (browser_h - 228).max(0);
        for (i, c) in m.columns.columns.iter().take(MAX_COLUMNS).enumerate() {
            let (top, count) = if i == m.columns.active {
                (m.scroll, m.order.len())
            } else {
                (c.data.scroll, c.data.order.len())
            };
            out[2 + i] = Some((top, (count as i32 * 36 - available).max(0)));
        }
        out[SLOTS - 1] = Some((
            m.columns.horizontal,
            (m.columns.columns.len() as i32 * crate::columns_view::WIDTH - m.columns.area.w).max(0),
        ));
    } else {
        let max = if m.view == View::SpaceMap {
            m.space_geometry.max_scroll()
        } else {
            (m.content_height - (browser_h - 134).max(0)).max(0)
        };
        out[0] = Some((m.scroll, max));
    }
    out
}
fn set(m: &mut Model, slot: usize, value: i32) {
    match slot {
        0 => m.scroll = value,
        1 => {
            if let Some(b) = &mut m.dual.other {
                b.scroll = value;
            }
        }
        s if s == SLOTS - 1 => m.columns.horizontal = value,
        s => {
            let i = s - 2;
            if i == m.columns.active {
                m.scroll = value;
            } else if let Some(c) = m.columns.columns.get_mut(i) {
                c.data.scroll = value;
            }
        }
    }
}

pub fn wheel(m: &mut Model, x: i32, delta: i32, horizontal: bool, h: i32) {
    if delta == 0 {
        return;
    }
    let (slot, step) = if m.dual.enabled {
        let left = m.browser_left();
        let on_right = x >= left + m.dual_width.max(0) / 2;
        (usize::from(on_right != m.dual.right_active), 36)
    } else if m.view == View::Columns && horizontal {
        (SLOTS - 1, 60)
    } else if m.view == View::Columns {
        let i = ((x - m.columns.area.x + m.columns.horizontal) / crate::columns_view::WIDTH).max(0)
            as usize;
        if i >= MAX_COLUMNS {
            return;
        }
        (2 + i, 36)
    } else {
        (0, if m.view == View::SpaceMap { 28 } else { 38 })
    };
    let Some((position, max)) = offsets(m, h)[slot] else {
        return;
    };
    let mut state = std::mem::take(&mut m.wheel_scroll);
    state.sync(m);
    state.targets[slot] = None;
    let value =
        state.slots[slot].wheel(position as i64, delta as f64 * step as f64, max as i64) as i32;
    set(m, slot, value);
    m.wheel_scroll = state;
}

pub fn advance(m: &mut Model, canvas: &Frame) {
    if advance_offsets(m, canvas.h()) {
        anim::request_redraw(canvas);
    }
}

/// Moves every running slot to its current painted offset; true while one
/// still runs. A slot that stops forgets its glide target.
pub(crate) fn advance_offsets(m: &mut Model, height: i32) -> bool {
    let mut state = std::mem::take(&mut m.wheel_scroll);
    state.sync(m);
    let offsets = offsets(m, height);
    let mut running = false;
    for (slot, offset) in offsets.into_iter().enumerate() {
        // Idle motion must not clamp programmatic restoration against the
        // previous frame's geometry. The model clamps after the current paint.
        if !state.slots[slot].running() {
            state.targets[slot] = None;
            continue;
        }
        let Some((position, max)) = offset else {
            continue;
        };
        let value = state.slots[slot].position(position as i64, max as i64) as i32;
        set(m, slot, value);
        if state.slots[slot].running() {
            running = true;
        } else {
            state.targets[slot] = None;
        }
    }
    m.wheel_scroll = state;
    running
}

/// Lands every keyboard glide in flight on its logical target and stops
/// motion. A same-folder refresh calls this before it captures the viewport,
/// so the refreshed listing restores where the reveal was heading, not a
/// mid-glide offset that may leave the cursor partly off screen.
pub(crate) fn settle(m: &mut Model) {
    let mut state = std::mem::take(&mut m.wheel_scroll);
    state.sync(m);
    for slot in 0..SLOTS {
        if let Some(target) = state.targets[slot]
            && state.slots[slot].running()
        {
            set(m, slot, target);
        }
    }
    state.cancel();
    m.wheel_scroll = state;
}

/// Runs a keyboard reveal and glides the painted offsets to its result.
///
/// The reveal is computed against the logical target (the destination of a
/// glide in flight), so holding a key keeps pace: each repeat retargets the
/// same `Scroll` slot the wheel uses instead of queueing. The model's offset
/// stays the painted value; the selection was already updated by the caller.
/// With animations off the reveal applies instantly, as before.
pub fn glide(m: &mut Model, h: i32, reveal: impl FnOnce(&mut Model)) {
    if !anim::enabled() {
        reveal(m);
        return;
    }
    let mut state = std::mem::take(&mut m.wheel_scroll);
    state.sync(m);
    let painted = offsets(m, h);
    for (slot, o) in painted.iter().enumerate() {
        if let Some((from, _)) = o {
            let logical = state.logical(slot, *from);
            if logical != *from {
                set(m, slot, logical);
            }
        }
    }
    reveal(m);
    let after = offsets(m, h);
    for slot in 0..SLOTS {
        if let (Some((from, _)), Some((to, max))) = (painted[slot], after[slot]) {
            // `glide` starts the slot (`Scroll::wheel`), so it counts as
            // running and `advance_offsets` steps it.
            let value = state.glide(slot, from, to, max);
            set(m, slot, value);
        }
    }
    m.wheel_scroll = state;
}

/// Same for the Columns horizontal offset, whose reveal runs during paint.
/// `reveal` maps the logical horizontal offset to the wanted one.
pub fn glide_horizontal(m: &mut Model, max: i32, reveal: impl FnOnce(i32) -> i32) -> i32 {
    let painted = m.columns.horizontal;
    if !anim::enabled() {
        return reveal(painted);
    }
    let mut state = std::mem::take(&mut m.wheel_scroll);
    state.sync(m);
    let to = reveal(state.logical(SLOTS - 1, painted)).clamp(0, max);
    let value = state.glide(SLOTS - 1, painted, to, max);
    m.wheel_scroll = state;
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use files_core::listing::{Entry, Kind};
    use std::time::Duration;

    fn list_model(root: &std::path::Path) -> Model {
        let mut m = Model::new(root.to_path_buf());
        m.view = View::List;
        m.entries = (0..100)
            .map(|i| Entry {
                name: format!("item-{i:03}").into(),
                kind: Kind::File,
                metadata: None,
                stat_attempted: false,
                link: None,
            })
            .collect();
        m.rebuild();
        m.content_height = 100 * 38 + 60;
        m.selected = Some(7);
        m.selection = [7].into_iter().collect();
        m
    }
    fn root(tag: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("files-glide-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn keyboard_reveal_glides_after_the_selection_is_final() {
        anim::set_enabled(true);
        let dir = root("glide");
        let mut m = list_model(&dir);
        crate::keyboard_nav::key(&mut m, fltk::enums::Key::Down, false, false, 800, 440);
        // Input did not wait: the cursor moved, the painted scroll has not yet.
        assert_eq!(m.selected, Some(8));
        assert!(m.wheel_scroll.running());
        assert!(advance_offsets(&mut m, 440));
        assert!(m.scroll < 66, "still gliding at {}", m.scroll);
        std::thread::sleep(Duration::from_millis(170));
        assert!(!advance_offsets(&mut m, 440));
        assert_eq!(m.scroll, 66);
        anim::set_enabled(None);
    }

    #[test]
    fn held_key_retargets_and_ends_where_the_instant_reveal_ends() {
        anim::set_enabled(false);
        let instant_dir = root("instant");
        let mut instant = list_model(&instant_dir);
        for _ in 0..20 {
            crate::keyboard_nav::key(&mut instant, fltk::enums::Key::Down, false, false, 800, 440);
        }
        assert!(!instant.wheel_scroll.running());
        let want = instant.scroll;
        assert!(want > 0);

        anim::set_enabled(true);
        let dir = root("held");
        let mut m = list_model(&dir);
        let mut last = 0;
        for _ in 0..20 {
            crate::keyboard_nav::key(&mut m, fltk::enums::Key::Down, false, false, 800, 440);
            advance_offsets(&mut m, 440);
            // Never overshoots the final offset and never goes backwards.
            assert!(m.scroll <= want && m.scroll >= last);
            last = m.scroll;
        }
        assert_eq!(m.selected, instant.selected);
        std::thread::sleep(Duration::from_millis(170));
        advance_offsets(&mut m, 440);
        assert_eq!(m.scroll, want);
        anim::set_enabled(None);
    }

    #[test]
    fn settle_lands_a_glide_in_flight_on_its_logical_target() {
        anim::set_enabled(true);
        let dir = root("settle");
        let mut m = list_model(&dir);
        crate::keyboard_nav::key(&mut m, fltk::enums::Key::Down, false, false, 800, 440);
        assert!(m.wheel_scroll.running());
        assert!(m.scroll < 66);
        // A same-folder refresh settles before capturing the viewport.
        settle(&mut m);
        assert_eq!(m.scroll, 66);
        assert!(!m.wheel_scroll.running());
        assert!(!advance_offsets(&mut m, 440));
        assert_eq!(m.scroll, 66);
        anim::set_enabled(None);
    }

    #[test]
    fn disabled_animation_reveals_instantly() {
        anim::set_enabled(false);
        let dir = root("off");
        let mut m = list_model(&dir);
        crate::keyboard_nav::key(&mut m, fltk::enums::Key::Down, false, false, 800, 440);
        assert_eq!(m.scroll, 66);
        assert!(!m.wheel_scroll.running());
        anim::set_enabled(None);
    }
}

//! Compact, right-anchored controls with a single animated label reveal.
use crate::{
    model::{Action, Hit, Model, View},
    painter::{Painter, Rect},
    shortcuts::Cmd,
};
use fltk_omarchy::anim::{Anim, Ease};
use std::time::{Duration, Instant};
const TOOLTIP_DELAY: Duration = Duration::from_secs(2);

#[derive(Default)]
pub struct State {
    hot: Option<usize>,
    rows: Vec<Rect>,
    motion: [Anim<f64>; 10],
    widths: Vec<i32>,
    since: Option<Instant>,
    suppressed: bool,
    visible: bool,
    tooltip: Option<(String, Rect)>,
    navigation: [Rect; 3],
}
impl State {
    pub fn hover(&mut self, pointer: Option<(i32, i32)>) -> bool {
        self.hover_at(pointer, Instant::now())
    }
    fn hover_at(&mut self, pointer: Option<(i32, i32)>, now: Instant) -> bool {
        let hot = pointer.and_then(|(x, y)| {
            self.rows.iter().position(|r| r.contains(x, y)).or_else(|| {
                self.navigation
                    .iter()
                    .position(|r| r.contains(x, y))
                    .map(|i| i + 10)
            })
        });
        let changed = self.hot != hot;
        if changed {
            self.since = hot.map(|_| now);
            self.suppressed = false;
            self.visible = false;
            self.tooltip = None;
        }
        self.hot = hot;
        changed
    }
    pub fn suppress(&mut self) -> bool {
        let changed = self.visible;
        self.suppressed = true;
        self.visible = false;
        self.since = None;
        self.tooltip = None;
        changed
    }
    pub fn tick(&mut self, now: Instant, active: bool) -> bool {
        if !active {
            return self.suppress();
        }
        let visible = !self.suppressed
            && self.hot.is_some()
            && self
                .since
                .is_some_and(|at| now.saturating_duration_since(at) >= TOOLTIP_DELAY);
        let changed = self.visible != visible;
        self.visible = visible;
        changed
    }
    pub fn schedule(&self, canvas: &fltk::frame::Frame) {
        if self.motion.iter().any(Anim::running) {
            fltk_omarchy::anim::request_redraw(canvas);
        }
    }
}
/// The hover hint of a toolbar control: its chord in the active profile.
fn shortcut(profile: crate::shortcuts::Profile, action: &Action) -> String {
    match action {
        Action::ClearFilter => "Esc".into(),
        Action::Filter
        | Action::Omni
        | Action::View(_)
        | Action::Inspector
        | Action::Settings
        | Action::Actions => crate::shortcuts::action_label(profile, action),
        _ => String::new(),
    }
}
struct Item {
    icon: &'static str,
    label: &'static str,
    on: bool,
    action: Action,
}
fn items(m: &Model, w: i32, h: i32) -> Vec<Item> {
    let mut controls = vec![
        Item {
            icon: if m.query.is_empty() { "" } else { "" },
            label: if m.query.is_empty() {
                "Buscar"
            } else {
                "Limpiar filtro"
            },
            on: m.filter_edit.active || !m.query.is_empty(),
            action: if m.query.is_empty() {
                Action::Filter
            } else {
                Action::ClearFilter
            },
        },
        Item {
            icon: "",
            label: "Editar ruta",
            on: false,
            action: Action::Omni,
        },
    ];
    if w < 640 {
        controls.extend([
            Item {
                icon: "",
                label: "Vistas",
                on: false,
                action: Action::ViewMenu(w, h),
            },
            Item {
                icon: "",
                label: "Acciones",
                on: m.actions_menu,
                action: Action::Actions,
            },
            Item {
                icon: "",
                label: "Preferencias",
                on: m.settings,
                action: Action::Settings,
            },
        ]);
        return controls;
    }
    controls.extend([
        Item {
            icon: "",
            label: "Grupos",
            on: m.view == View::Groups,
            action: Action::View(View::Groups),
        },
        Item {
            icon: "",
            label: "Lista",
            on: m.view == View::List,
            action: Action::View(View::List),
        },
        Item {
            icon: "",
            label: "Grilla",
            on: m.view == View::Grid,
            action: Action::View(View::Grid),
        },
        Item {
            icon: "",
            label: "Espacio",
            on: m.view == View::SpaceMap,
            action: Action::View(View::SpaceMap),
        },
        Item {
            icon: "",
            label: "Columnas",
            on: m.view == View::Columns,
            action: Action::View(View::Columns),
        },
        Item {
            icon: "",
            label: "Inspector",
            on: m.inspector || m.dual.enabled,
            action: Action::Inspector,
        },
        Item {
            icon: "",
            label: "Acciones",
            on: m.actions_menu,
            action: Action::Actions,
        },
        Item {
            icon: "",
            label: "Preferencias",
            on: m.settings,
            action: Action::Settings,
        },
    ]);
    controls
}
pub fn base_right(w: i32) -> i32 {
    w - 12 - (if w < 640 { 5 } else { 10 }) * 40 + 4 - 14
}
pub fn marker(m: &Model, w: i32, h: i32) -> serde_json::Value {
    serde_json::json!({
        "hot":m.toolbar.hot,"animating":m.toolbar.motion.iter().any(Anim::running),
        "enabled":fltk_omarchy::anim::enabled(),
        "up_rect":[84,13,30,32],
        "tooltip":if m.toolbar.visible {m.toolbar.tooltip.as_ref().map(|(text,r)|serde_json::json!({"text":text,"rect":[r.x,r.y,r.w,r.h]}))}else{None},
        "items":items(m,w,h).iter().enumerate().map(|(i,item)|serde_json::json!({
            "index":i,"label":item.label,"on":item.on,
            "shortcut":shortcut(m.shortcuts,&item.action),
            "rect":m.toolbar.rows.get(i).map(|r|[r.x,r.y,r.w,r.h]),
        })).collect::<Vec<_>>()
    })
}
/// Sample once before painting the address field so it shares the row geometry.
pub fn prepare(p: &mut dyn Painter, m: &mut Model, w: i32, h: i32) -> i32 {
    let items = items(m, w, h);
    let count = items.len();
    if m.toolbar.rows.len() != count {
        m.toolbar.hot = None;
        for motion in &mut m.toolbar.motion {
            motion.jump(0.);
        }
    }
    m.toolbar.navigation = [
        Rect::new(12, 13, 30, 32),
        Rect::new(48, 13, 30, 32),
        Rect::new(84, 13, 30, 32),
    ];
    if m.toolbar.hot.is_some_and(|i| i >= count && i < 10) {
        m.toolbar.hot = None;
    }
    let allowance = (base_right(w) - 176).clamp(0, 160);
    let mut remaining = allowance;
    m.toolbar.widths.clear();
    for (i, item) in items.iter().enumerate() {
        let target = if m.toolbar.hot == Some(i) {
            (p.text_width(item.label, 12, false).ceil() as i32 + 12).min(allowance)
        } else {
            0
        };
        let extra = m.toolbar.motion[i]
            .value(f64::from(target), 160, Ease::OutCubic)
            .round()
            .max(0.) as i32;
        let extra = extra.min(remaining);
        remaining -= extra;
        m.toolbar.widths.push(36 + extra);
    }
    let total = m.toolbar.widths.iter().sum::<i32>() + (count as i32 - 1) * 4;
    let mut x = w - 12 - total;
    m.toolbar.rows.clear();
    for width in &m.toolbar.widths {
        m.toolbar.rows.push(Rect::new(x, 10, *width, 36));
        x += width + 4;
    }
    w - 12 - total - 14
}

/// This canvas-owned surface avoids the toolkit's global warm-tooltip delay.
/// It has no hit target and is painted after native tab/header children.
pub fn paint_tooltip(p: &mut dyn Painter, m: &mut Model, w: i32) {
    m.toolbar.tooltip = None;
    if !m.toolbar.visible {
        return;
    }
    let Some(index) = m.toolbar.hot else {
        return;
    };
    let (text, anchor) = if index >= 10 {
        let i = index - 10;
        let Some(anchor) = m.toolbar.navigation.get(i) else {
            return;
        };
        let cmd = [Cmd::Back, Cmd::Forward, Cmd::Parent][i];
        (crate::shortcuts::label(m.shortcuts, cmd), *anchor)
    } else {
        let controls = items(m, w, 0);
        let Some(item) = controls.get(index) else {
            return;
        };
        (shortcut(m.shortcuts, &item.action), m.toolbar.rows[index])
    };
    if text.is_empty() {
        return;
    }
    let text = text.as_str();
    let width = (p.text_width(text, 12, true).ceil() as i32 + 24).min(w - 16);
    let rect = Rect::new(
        (anchor.x + anchor.w / 2 - width / 2).clamp(8, (w - width - 8).max(8)),
        52,
        width,
        28,
    );
    p.rounded_rect(rect, 5 * m.radius, m.palette.lighter_background);
    p.rounded_rect(
        Rect::new(rect.x + 1, rect.y + 1, rect.w - 2, rect.h - 2),
        5 * m.radius,
        m.palette.darker_background,
    );
    p.text(
        Rect::new(rect.x + 12, rect.y, rect.w - 24, rect.h),
        text,
        12,
        m.palette.foreground,
        true,
        false,
    );
    m.toolbar.tooltip = Some((text.into(), rect));
}

pub fn paint(p: &mut dyn Painter, m: &mut Model, w: i32, h: i32) {
    for (i, item) in items(m, w, h).into_iter().enumerate() {
        let rect = m.toolbar.rows[i];
        let hot = m.toolbar.hot == Some(i);
        let c = &m.palette;
        if item.on || hot {
            p.rounded_rect(
                rect,
                7 * m.radius,
                if item.on {
                    c.lighter_background
                } else {
                    c.darker_background
                },
            );
        }
        let color = if item.on || hot {
            c.accent
        } else {
            c.dark_foreground
        };
        // Keep the icon on the stable right edge; reveal the label to its left.
        p.text(
            Rect::new(rect.x + rect.w - 26, rect.y, 24, rect.h),
            crate::fonts::icon(item.icon, "·"),
            16,
            color,
            true,
            false,
        );
        if rect.w > 44 {
            let text = Rect::new(rect.x + 8, rect.y, rect.w - 40, rect.h);
            p.clip(text);
            p.text(text, item.label, 12, color, false, false);
            p.unclip();
        }
        m.hits.push(Hit {
            generation: m.generation,
            rect,
            action: item.action,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shortcut_delay_restarts_for_each_control_without_warm_shortcut() {
        let mut state = State {
            rows: vec![Rect::new(0, 0, 36, 36), Rect::new(40, 0, 36, 36)],
            ..Default::default()
        };
        let start = Instant::now();
        state.hover_at(Some((5, 5)), start);
        assert!(!state.tick(start + Duration::from_millis(1999), true));
        assert!(!state.visible);
        assert!(state.tick(start + TOOLTIP_DELAY, true));
        assert!(state.visible);
        state.hover_at(Some((45, 5)), start + TOOLTIP_DELAY);
        state.tick(start + Duration::from_millis(3999), true);
        assert!(!state.visible);
        state.tick(start + Duration::from_secs(4), true);
        assert!(state.visible);
    }
    #[test]
    fn input_and_exit_cancel_tooltip_until_a_new_hover() {
        let mut state = State {
            rows: vec![Rect::new(0, 0, 36, 36)],
            ..Default::default()
        };
        let now = Instant::now();
        state.hover_at(Some((1, 1)), now);
        state.tick(now + TOOLTIP_DELAY, true);
        assert!(state.suppress());
        state.tick(now + Duration::from_secs(20), true);
        assert!(!state.visible);
        state.hover_at(None, now);
        state.hover_at(Some((1, 1)), now);
        state.tick(now + TOOLTIP_DELAY, true);
        assert!(state.visible);
        state.hover_at(None, now);
        assert!(!state.visible);
    }
}

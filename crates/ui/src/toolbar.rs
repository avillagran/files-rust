//! Compact, right-anchored controls with a single animated label reveal.
use crate::{
    model::{Action, Hit, Model, View},
    painter::{Painter, Rect},
};
use fltk_omarchy::anim::{Anim, Ease};

#[derive(Default)]
pub struct State {
    hot: Option<usize>,
    rows: Vec<Rect>,
    motion: [Anim<f64>; 8],
    widths: Vec<i32>,
}
impl State {
    pub fn hover(&mut self, pointer: Option<(i32, i32)>) -> bool {
        let hot = pointer.and_then(|(x, y)| self.rows.iter().position(|r| r.contains(x, y)));
        let changed = self.hot != hot;
        self.hot = hot;
        changed
    }
    pub fn schedule(&self, canvas: &fltk::frame::Frame) {
        if self.motion.iter().any(Anim::running) {
            fltk_omarchy::anim::request_redraw(canvas);
        }
    }
}
struct Item {
    icon: &'static str,
    label: &'static str,
    on: bool,
    action: Action,
}
fn items(m: &Model, w: i32, h: i32) -> Vec<Item> {
    if w < 640 {
        return vec![
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
        ];
    }
    vec![
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
    ]
}
pub fn base_right(w: i32) -> i32 {
    w - 12 - (if w < 640 { 3 } else { 8 }) * 40 + 4 - 14
}
pub fn marker(m: &Model, w: i32, h: i32) -> serde_json::Value {
    serde_json::json!({
        "hot":m.toolbar.hot,"animating":m.toolbar.motion.iter().any(Anim::running),
        "enabled":fltk_omarchy::anim::enabled(),
        "items":items(m,w,h).iter().enumerate().map(|(i,item)|serde_json::json!({
            "index":i,"label":item.label,"on":item.on,
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
    if m.toolbar.hot.is_some_and(|i| i >= count) {
        m.toolbar.hot = None;
    }
    let allowance = (base_right(w) - 232).clamp(0, 160);
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

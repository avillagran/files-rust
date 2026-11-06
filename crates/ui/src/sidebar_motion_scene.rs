//! Private rendered-frame fixture for the Sidebar/content animation contract.
use crate::{
    model::{Action, Model},
    painter::Rect,
};
use fltk::{frame::Frame, prelude::*, window::Window};
use fltk_omarchy::{
    anim,
    testkit::{Gate, Rules},
    widgets::Sidebar,
};
use serde_json::{Value, json};
use std::{cell::RefCell, rc::Rc};

fn entry(m: &Model) -> Option<Rect> {
    m.hits
        .iter()
        .find_map(|h| matches!(h.action, Action::Entry(_)).then_some(h.rect))
}

pub fn run(win: &Window, canvas: &Frame, sidebar: &Sidebar, model: &Rc<RefCell<Model>>) -> Value {
    let old_motion = anim::enabled();
    let old_pin = sidebar.is_pinned();
    let old_expanded = sidebar.is_expanded();
    let frozen = anim::clock::frozen();
    anim::clock::freeze();
    anim::set_enabled(true);
    sidebar.set_pinned(false);
    sidebar.collapse();
    let mut gate = Gate::new(win);
    let m = model.clone();
    gate.track("browser", move || {
        Some((m.borrow().browser_left(), 90, 1, 1))
    });
    let rail = sidebar.clone();
    gate.track("rail-edge", move || {
        Some((rail.x() + rail.w(), rail.y(), 1, rail.h()))
    });
    let m = model.clone();
    gate.track("entry", move || {
        entry(&m.borrow()).map(|r| (r.x, r.y, r.w, r.h))
    });
    let mut captures = Vec::new();
    let mut capture = |name: &str| {
        if let Some(dir) = std::env::var_os("FILES_TEST_MOTION_SHOTS") {
            let dir = std::path::PathBuf::from(dir);
            let path = dir.join(format!("{name}.png"));
            // SAFETY: the fixture receives a live window on its UI thread.
            let mut w = unsafe { fltk::window::DoubleWindow::from_widget_ptr(win.as_widget_ptr()) };
            if std::fs::create_dir_all(&dir).is_ok()
                && fltk_omarchy::window::snapshot(&mut w, &path).is_ok()
            {
                captures.push(path);
            }
        }
    };
    canvas.clone().redraw();
    gate.play(240);
    let baseline = gate.frames();
    capture("collapsed");
    gate.act("expand", || sidebar.expand());
    gate.play(64);
    capture("expanding");
    gate.play(176);
    capture("expanded");
    let expanded_end = gate.frames();
    gate.act("collapse", || sidebar.collapse());
    gate.play(240);
    let collapse_end = gate.frames();
    // Reverse before the preceding transition finishes, then settle.
    gate.act("expand-again", || sidebar.expand());
    gate.play(48);
    gate.act("reverse", || sidebar.collapse());
    gate.play(240);
    let original = (win.x(), win.y(), win.w(), win.h());
    let mut sized = win.clone();
    gate.act("window-resize", || {
        sized.resize(original.0, original.1, original.2 - 80, original.3 - 40)
    });
    gate.play(48);
    let resize_aligned = sidebar.w() == model.borrow().rail_width()
        && model.borrow().browser_left() == sidebar.w() + 16;
    gate.act("window-restore", || {
        sized.resize(original.0, original.1, original.2, original.3)
    });
    gate.play(48);
    let findings: Vec<_> = gate
        .check(&Rules::default())
        .iter()
        .map(ToString::to_string)
        .collect();
    let xs = |name: &str| -> Vec<i32> {
        gate.series(name)
            .unwrap_or_default()
            .iter()
            .filter_map(|s| s.map(|s| s.rect.0))
            .collect()
    };
    let browser = xs("browser");
    let edge = xs("rail-edge");
    let entries = xs("entry");
    let aligned =
        browser.len() == edge.len() && browser.iter().zip(&edge).all(|(b, r)| *b == *r + 16);
    let stable_entry = entries.len() == browser.len()
        && entries
            .iter()
            .zip(&browser)
            .all(|(e, b)| *e - *b == entries[0] - browser[0]);
    let expansion = &browser[baseline..expanded_end];
    let collapse = &browser[expanded_end..collapse_end];
    let intermediate = expansion.iter().filter(|&&x| x > 68 && x < 186).count() >= 3;
    let monotonic =
        expansion.windows(2).all(|p| p[0] <= p[1]) && collapse.windows(2).all(|p| p[0] >= p[1]);
    let settled = browser.last() == Some(&68) && expansion.last() == Some(&186);
    // Reduced motion reaches the endpoint in the input frame.
    anim::set_enabled(false);
    gate.act("disabled-expand", || sidebar.expand());
    let disabled_expand = model.borrow().browser_left() == 186 && sidebar.w() == 170;
    gate.act("disabled-collapse", || sidebar.collapse());
    let disabled_collapse = model.borrow().browser_left() == 68 && sidebar.w() == 52;
    sidebar.set_pinned(true);
    sidebar.collapse();
    gate.frame();
    let pinned =
        sidebar.is_expanded() && sidebar.w() == 170 && model.borrow().browser_left() == 186;
    sidebar.set_pinned(false);
    sidebar.collapse();
    gate.frame();
    // A covering surface hides the widget and uses endpoint-only geometry.
    model.borrow_mut().action(Action::Help);
    gate.frame();
    let covered = !sidebar.visible() && !model.borrow().sidebar_widget;
    model.borrow_mut().help = None;
    gate.frame();
    let restored =
        sidebar.visible() && model.borrow().sidebar_widget && model.borrow().browser_left() == 68;
    sidebar.set_pinned(old_pin);
    if old_expanded {
        sidebar.expand();
    } else {
        sidebar.collapse();
    }
    gate.frame();
    anim::set_enabled(old_motion);
    if !frozen {
        anim::clock::resume();
    }
    json!({"ok":aligned && stable_entry && intermediate && monotonic && settled && findings.is_empty()
        && disabled_expand && disabled_collapse && covered && restored && resize_aligned && pinned,
        "aligned":aligned, "entry_aligned":stable_entry, "intermediate":intermediate,
        "monotonic":monotonic, "settled":settled, "findings":findings,
        "disabled_expand":disabled_expand,"disabled_collapse":disabled_collapse,
        "modal_hidden":covered,"modal_restored":restored,"resize_aligned":resize_aligned,"pinned":pinned,
        "browser_x":browser,"rail_right":edge,"entry_x":entries,"captures":captures,
        "expand_frames":[baseline,expanded_end],"collapse_frames":[expanded_end,collapse_end]})
}

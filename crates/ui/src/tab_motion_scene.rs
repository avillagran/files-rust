//! Private rendered-frame acceptance for the keyed strip's shared indicator.
use crate::{
    model::{Action, Model},
    tabs::{self, Command},
};
use fltk::{frame::Frame, prelude::*, window::Window};
use fltk_omarchy::{anim, testkit::Gate};
use serde_json::{Value, json};
use std::{cell::RefCell, path::Path, rc::Rc};

struct Restore {
    model: Rc<RefCell<Model>>,
    canvas: Frame,
    active: u64,
    motion: bool,
    frozen: bool,
}
impl Drop for Restore {
    fn drop(&mut self) {
        anim::set_enabled(false);
        if let Ok(mut m) = self.model.try_borrow_mut()
            && m.tabs.index(self.active).is_some()
        {
            m.action(Action::Tab(Command::Select(self.active)));
        }
        self.canvas.redraw();
        anim::set_enabled(self.motion);
        if !self.frozen {
            anim::clock::resume();
        }
    }
}

fn target(m: &Model, id: u64) -> Option<[i32; 4]> {
    let hits: Vec<_> = m
        .hits
        .iter()
        .filter(|h| {
            h.generation == m.generation
                && matches!(h.action, Action::Tab(Command::Select(v) | Command::Close(v)) if v == id)
        })
        .map(|h| h.rect)
        .collect();
    let x = hits.iter().map(|r| r.x).min()?;
    let right = hits.iter().map(|r| r.x + r.w).max()?;
    Some([x, 58, right - x, tabs::HEIGHT])
}

fn capture(
    win: &Window,
    model: &Rc<RefCell<Model>>,
    directory: &Path,
    label: &str,
    elapsed: u64,
    frames: usize,
) -> Result<Value, String> {
    let path = directory.join(format!("{label}.png"));
    // SAFETY: this fixture runs on the UI thread with a live Files window.
    let mut window = unsafe { fltk::window::DoubleWindow::from_widget_ptr(win.as_widget_ptr()) };
    fltk_omarchy::window::snapshot(&mut window, &path).map_err(|error| error.to_string())?;
    let m = model.borrow();
    let rect = target(&m, m.tabs.active).ok_or("selected tab geometry is not painted")?;
    Ok(
        json!({"label":label,"path":path,"elapsed_ms":elapsed,"frames":frames,
        "active":m.tabs.active,"generation":m.generation,"target_rect":rect}),
    )
}

pub fn run(win: &Window, canvas: &Frame, model: &Rc<RefCell<Model>>) -> Value {
    let ids: Vec<_> = model.borrow().tabs.rows.iter().map(|tab| tab.id).collect();
    if ids.len() < 3 {
        return json!({"ok":false,"error":"tab motion needs three seeded tabs"});
    }
    let Some(directory) = std::env::var_os("FILES_TEST_TAB_MOTION_SHOTS") else {
        return json!({"ok":false,"error":"tab motion capture directory is absent"});
    };
    let directory = std::path::PathBuf::from(directory);
    if !directory.is_absolute() {
        return json!({"ok":false,"error":"tab motion capture directory must be absolute"});
    }
    if let Err(error) = std::fs::create_dir_all(&directory) {
        return json!({"ok":false,"error":error.to_string()});
    }
    let restore = Restore {
        model: model.clone(),
        canvas: canvas.clone(),
        active: model.borrow().tabs.active,
        motion: anim::enabled(),
        frozen: anim::clock::frozen(),
    };
    anim::clock::freeze();
    let mut gate = Gate::new(win);
    let mut captures = Vec::new();
    let mut elapsed = 0;
    let first = ids[0];
    let last = *ids.last().expect("three seeded tabs");
    let select = |id| {
        model.borrow_mut().action(Action::Tab(Command::Select(id)));
        canvas.clone().redraw();
    };
    let result = (|| -> Result<(), String> {
        // Establish an exact endpoint before measuring any transition.
        anim::set_enabled(false);
        gate.act("baseline-first", || select(first));
        elapsed += 16;
        anim::set_enabled(true);
        gate.play(240);
        elapsed += 240;
        captures.push(capture(
            win,
            model,
            &directory,
            "first-settled",
            elapsed,
            gate.frames(),
        )?);

        gate.act("select-last", || select(last));
        elapsed += 16;
        captures.push(capture(
            win,
            model,
            &directory,
            "forward-input",
            elapsed,
            gate.frames(),
        )?);
        gate.play(64);
        elapsed += 64;
        captures.push(capture(
            win,
            model,
            &directory,
            "forward-64",
            elapsed,
            gate.frames(),
        )?);
        gate.play(176);
        elapsed += 176;
        captures.push(capture(
            win,
            model,
            &directory,
            "last-settled",
            elapsed,
            gate.frames(),
        )?);

        gate.act("return-toward-first", || select(first));
        elapsed += 16;
        gate.play(48);
        elapsed += 48;
        captures.push(capture(
            win,
            model,
            &directory,
            "reverse-48",
            elapsed,
            gate.frames(),
        )?);
        gate.act("reverse-before-endpoint", || select(last));
        elapsed += 16;
        captures.push(capture(
            win,
            model,
            &directory,
            "reversed-input",
            elapsed,
            gate.frames(),
        )?);
        gate.play(240);
        elapsed += 240;
        captures.push(capture(
            win,
            model,
            &directory,
            "reversed-settled",
            elapsed,
            gate.frames(),
        )?);

        // Disabled motion must reach the target in the input frame.
        anim::set_enabled(false);
        gate.act("disabled-first", || select(first));
        elapsed += 16;
        gate.act("disabled-last", || select(last));
        elapsed += 16;
        captures.push(capture(
            win,
            model,
            &directory,
            "disabled-last",
            elapsed,
            gate.frames(),
        )?);
        Ok(())
    })();
    let stable_ids = model
        .borrow()
        .tabs
        .rows
        .iter()
        .map(|tab| tab.id)
        .eq(ids.iter().copied());
    anim::set_enabled(false);
    select(restore.active);
    gate.frame();
    let restored_active = model.borrow().tabs.active == restore.active;
    let accent = fltk_omarchy::theme::with(|t| {
        [
            t.palette.accent.r(),
            t.palette.accent.g(),
            t.palette.accent.b(),
        ]
    });
    let mut report = json!({"ok":result.is_ok() && stable_ids && restored_active && captures.len()==8,
        "error":result.err(),"captures":captures,"stable_ids":stable_ids,"restored_active":restored_active,
        "first":first,"last":last,"accent_rgb":accent,"underline_y":58+tabs::HEIGHT-2,
        "strip_rect":[8,58,(win.w()-48).max(0),tabs::HEIGHT],"viewport":[win.w(),win.h()],
        "motion_restored_to":restore.motion,"clock_remains_frozen":restore.frozen,
        "method":"Frozen toolkit clock and full-window rendered PNGs; pixel acceptance runs in the Python driver, not native partial-damage capture"});
    let policy = (restore.motion, restore.frozen);
    drop(restore);
    let restored_policy = anim::enabled() == policy.0 && anim::clock::frozen() == policy.1;
    report["restored_policy"] = restored_policy.into();
    if !restored_policy {
        report["ok"] = false.into();
    }
    report
}

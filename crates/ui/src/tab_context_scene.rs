//! Private tab-context scene. Commands drive the app's painted tab targets.
use crate::{
    model::{Action, Model},
    tabs::Command,
};
use fltk::{frame::Frame, prelude::*};
use serde_json::Value;
pub fn command(m: &mut Model, text: &str) -> Result<(), &'static str> {
    match text {
        "tab-context-seed" => {
            let parent = m.path.parent().ok_or("fixture parent absent")?;
            let paths = [parent.join("B"), parent.join("C")];
            m.background_tabs(paths.iter().map(|p| (p.as_path(), None)))?;
        }
        "tab-context-first" | "tab-context-last" => {
            let id = if text == "tab-context-first" {
                m.tabs.rows.first()
            } else {
                m.tabs.rows.last()
            }
            .ok_or("no tab")?
            .id;
            let r = m
                .hits
                .iter()
                .find_map(|h| {
                    (h.generation == m.generation
                        && matches!(h.action,Action::Tab(Command::Select(v))if v==id))
                    .then_some(h.rect)
                })
                .ok_or("tab target not painted")?;
            crate::tab_context::pointer(m, r.x + r.w / 2, r.y + r.h / 2);
        }
        _ => return Err("unknown tab context fixture"),
    }
    Ok(())
}
pub fn state(m: &Model, canvas: &Frame) -> Value {
    let mut value = crate::tab_gesture_scene::state(m);
    let object = value.as_object_mut().expect("tab state object");
    object.insert(
        "context_open".into(),
        (m.tab_context.is_some() && m.toolkit_popup).into(),
    );
    object.insert(
        "focus_browser".into(),
        fltk::app::focus()
            .is_some_and(|w| w.as_widget_ptr() == canvas.as_widget_ptr())
            .into(),
    );
    value
}

//! Private rendered breadcrumb fixture, using production hits and menu routes.
use crate::model::{Action, Model};
use fltk::{frame::Frame, prelude::*};
use serde_json::{Value, json};
pub fn command(m: &mut Model, command: &str) -> Result<(), &'static str> {
    match command {
        "breadcrumb-select-file" => {
            let i = m
                .entries
                .iter()
                .position(|e| e.name == "example.txt")
                .ok_or("fixture file not loaded")?;
            m.select_entry(i, false, false);
        }
        "breadcrumb-reset" => {
            let root=files_core::theme::home().join("ancestor-with-a-long-readable-name/second-long-ancestor/third-long-ancestor/parent/leaf");
            m.action(Action::Navigate(root));
        }
        "breadcrumb-open" => {
            let r = m.breadcrumbs.overflow.ok_or("overflow not visible")?;
            m.pointer_down(r.x + r.w / 2, r.y + r.h / 2, false, false, false);
        }
        "breadcrumb-middle-visible" => {
            let path = m
                .breadcrumbs
                .visible
                .iter()
                .find(|(c, _)| c.path != m.path)
                .map(|(c, _)| c.path.clone())
                .ok_or("no visible ancestor")?;
            crate::tab_gestures::middle(m, Some(Action::Navigate(path)));
        }
        "breadcrumb-middle-overflow" => crate::tab_gestures::middle(m, Some(Action::Breadcrumbs)),
        _ => return Err("unknown breadcrumb fixture command"),
    }
    Ok(())
}
pub fn state(m: &Model, canvas: &Frame) -> Value {
    let rect = |r: crate::painter::Rect| json!([r.x, r.y, r.w, r.h]);
    let hx = |n: &std::ffi::OsStr| {
        n.as_encoded_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    let mut selected: Vec<_> = m
        .selection
        .iter()
        .filter_map(|&i| m.entries.get(i))
        .map(|e| hx(&e.name))
        .collect();
    selected.sort();
    json!({"path":m.path,"path_raw":hx(m.path.as_os_str()),"editing":m.editing,"filter":m.query,
        "overflow":m.breadcrumbs.hidden.iter().map(|c|json!({"label":c.label,"path":c.path,"path_raw":hx(c.path.as_os_str())})).collect::<Vec<_>>(),
        "visible":m.breadcrumbs.visible.iter().map(|(c,r)|json!({"label":c.label,"path":c.path,"rect":rect(*r)})).collect::<Vec<_>>(),
        "overflow_rect":m.breadcrumbs.overflow.map(rect),"omni_rect":rect(m.breadcrumbs.area),
        "generation":m.generation,"active":m.tabs.active,"tabs":m.tabs.rows.iter().map(|t|json!({"id":t.id,"path":t.path})).collect::<Vec<_>>(),
        "selected_raw":m.selected.and_then(|i|m.entries.get(i)).map(|e|hx(&e.name)),"selection_raw":selected,
        "anchor_raw":m.anchor.and_then(|i|m.entries.get(i)).map(|e|hx(&e.name)),
        "focus_browser":fltk::app::focus().is_some_and(|w|w.as_widget_ptr()==canvas.as_widget_ptr()),
        "popup_open":m.breadcrumbs.open && m.toolkit_popup,
        "loaded":m.loaded,"settled":m.listing_sync.settled(),"refreshing":m.selection_refreshing()})
}

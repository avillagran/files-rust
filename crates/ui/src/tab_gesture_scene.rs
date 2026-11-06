//! Private auxiliary-pointer fixture; uses the same painted targets as input.
use crate::{
    model::{Action, Model, View},
    tabs::Command,
};
use serde_json::{Value, json};
use std::ffi::OsStr;
fn hex(n: &OsStr) -> String {
    n.as_encoded_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn name<'a>(m: &'a Model, action: &Action) -> Option<&'a OsStr> {
    match *action {
        Action::Entry(i) => m.entries.get(i).map(|e| e.name.as_os_str()),
        Action::ColumnEntry(id, i) => {
            let (index, c) = m
                .columns
                .columns
                .iter()
                .enumerate()
                .find(|(_, c)| c.id == id)?;
            if index == m.columns.active {
                m.entries.get(i)
            } else {
                c.data.entries.get(i)
            }
            .map(|e| e.name.as_os_str())
        }
        Action::PaneEntry(id, rev, i) if m.dual.active_id == id && m.pane_revision == rev => {
            m.entries.get(i).map(|e| e.name.as_os_str())
        }
        Action::PaneEntry(id, rev, i) => m
            .dual
            .other
            .as_ref()
            .filter(|b| b.id == id && b.revision == rev)?
            .entries
            .get(i)
            .map(|e| e.name.as_os_str()),
        _ => None,
    }
}
fn target(m: &Model, label: &str) -> Option<crate::painter::Rect> {
    let first = m.tabs.rows.first()?.id;
    let last = m.tabs.rows.last()?.id;
    m.hits
        .iter()
        .rev()
        .find(|h| {
            h.generation == m.generation
                && match label {
                    "folder" | "file" => {
                        name(m, &h.action)
                            == Some(OsStr::new(if label == "folder" {
                                "folder"
                            } else {
                                "file.txt"
                            }))
                            && !matches!(h.action,Action::PaneEntry(id,_,_) if id!=m.dual.active_id)
                    }
                    "inactive-folder" => {
                        name(m, &h.action) == Some(OsStr::new("folder"))
                            && matches!(h.action,Action::PaneEntry(id,_,_) if id!=m.dual.active_id)
                    }
                    "tab-last" => matches!(h.action,Action::Tab(Command::Select(id)) if id==last),
                    "tab-first" => matches!(h.action,Action::Tab(Command::Select(id)) if id==first),
                    "tab-close-button-last" => {
                        matches!(h.action,Action::Tab(Command::Close(id)) if id==last)
                    }
                    "tab-new-button" => matches!(h.action, Action::Tab(Command::New)),
                    _ => false,
                }
        })
        .map(|h| h.rect)
}
pub fn command(m: &mut Model, label: &str, _w: i32, _h: i32) -> Result<(), &'static str> {
    match label {
        "select-file" => {
            let i = m
                .entries
                .iter()
                .position(|e| e.name == "file.txt")
                .ok_or("fixture file not loaded")?;
            m.select_entry(i, false, false);
        }
        "view-list" => m.action(Action::View(View::List)),
        "view-grid" => m.action(Action::View(View::Grid)),
        "view-columns" => m.action(Action::View(View::Columns)),
        "dual" => m.action(Action::DualPane),
        label => {
            let r = target(
                m,
                if label == "blocked-folder" {
                    "folder"
                } else {
                    label
                },
            )
            .ok_or("fixture target not visible")?;
            let action = crate::tab_gestures::hit(m, r.x + r.w / 2, r.y + r.h / 2);
            if label == "blocked-folder" {
                m.help = Some(Default::default());
            }
            crate::tab_gestures::middle(m, action);
            if label == "blocked-folder" {
                m.help = None;
            }
        }
    }
    Ok(())
}
pub fn state(m: &Model) -> Value {
    let tabs = |t: &crate::tabs::Tabs| {
        t.rows
            .iter()
            .map(|t| json!({"id":t.id,"path":t.path}))
            .collect::<Vec<_>>()
    };
    let mut selected: Vec<_> = m
        .selection
        .iter()
        .filter_map(|&i| m.entries.get(i))
        .map(|e| hex(&e.name))
        .collect();
    selected.sort();
    let targets: serde_json::Map<String, Value> = [
        "folder",
        "file",
        "inactive-folder",
        "tab-last",
        "tab-first",
        "tab-close-button-last",
        "tab-new-button",
    ]
    .into_iter()
    .filter_map(|key| target(m, key).map(|r| (key.into(), json!([r.x, r.y, r.w, r.h]))))
    .collect();
    json!({"path":m.path,"active":m.tabs.active,"tabs":tabs(&m.tabs),
        "selected_raw":m.selected.and_then(|i|m.entries.get(i)).map(|e|hex(&e.name)),
        "selection_raw":selected,"anchor_raw":m.anchor.and_then(|i|m.entries.get(i)).map(|e|hex(&e.name)),
        "generation":m.generation,"view":match m.view {View::Groups=>"groups",View::List=>"list",View::Grid=>"grid",View::Columns=>"columns",View::SpaceMap=>"space-map",View::Spatial=>"spatial"},"pane_id":m.dual.active_id,
        "other_tabs":m.dual.other.as_ref().map(|b|tabs(&b.tabs)).unwrap_or_default(),
        "other_path":m.dual.other.as_ref().map(|b|&b.path),"other_loaded":m.dual.other.as_ref().is_some_and(|b|b.loaded),
        "settled":m.listing_sync.settled(),"refreshing":m.selection_refreshing(),"closed":m.tabs.closed.len(),"closing":m.closing,"error":m.error,"loaded":m.loaded,"targets":targets})
}

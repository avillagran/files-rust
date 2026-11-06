//! Toolkit presentation for one app-owned operation result. Callbacks enqueue
//! identities only; reports and mutation admission remain in Model.
use crate::{
    model::Model,
    operation_feedback::{Action, Context, Snapshot, Source, Stamp},
};
use fltk::{app, frame::Frame, prelude::*};
use fltk_omarchy::overlay::{Notice, NoticeAction, NoticeDismiss, NoticeHandle};
use std::{cell::RefCell, collections::VecDeque, rc::Rc};

const REQUEST_LIMIT: usize = 8;

pub fn context(m: &Model) -> Context {
    let visible = !crate::tab_gestures::blocked(m)
        && !m.copy_busy()
        && !m.mutation_applied
        && !m.delete.busy();
    Context {
        visible,
        available: visible && !m.mutations.busy,
        copy_result: m.copy_last.as_ref().map(|r| r.id),
        can_undo: m.can_undo,
        can_redo: m.can_redo,
    }
}

/// Owned-fixture setup only. The ordinary review form remains open; a native
/// confirmation must start the operation, exactly as in the application.
pub fn fixture_form(m: &mut Model, rename: bool) -> Result<(), &'static str> {
    if std::env::var_os("FILES_TEST_OPERATION_NOTICES").as_deref()
        != Some(std::ffi::OsStr::new("1"))
        || !context(m).available
        || !m.loaded
        || m.selection_refreshing()
    {
        return Err("Private notice fixture is not ready");
    }
    let home = files_core::theme::home();
    if m.path.as_os_str() != home.join("source").as_os_str() {
        return Err("Private source location is required");
    }
    let name = if rename {
        "rename-before.txt"
    } else {
        "copy-payload.txt"
    };
    let index = m
        .entries
        .iter()
        .position(|e| e.name == name)
        .ok_or("Private fixture file is missing")?;
    m.select_entry(index, false, false);
    if rename {
        m.open_rename(crate::rename_dialog::Mode::Rename);
        let d = m
            .rename_dialog
            .as_mut()
            .ok_or("Rename review did not open")?;
        d.input("rename-after.txt");
    } else {
        m.open_copy();
        m.copy_form
            .as_mut()
            .ok_or("Copy review did not open")?
            .capture_destination(home.join("target"));
    }
    Ok(())
}

struct Request {
    stamp: Stamp,
    action: Action,
}
struct Active {
    snapshot: Snapshot,
    handle: NoticeHandle,
}

#[derive(Clone, Default)]
pub struct Inbox {
    requests: Rc<RefCell<VecDeque<Request>>>,
    active: Rc<RefCell<Option<Active>>>,
}
impl Inbox {
    fn push(&self, stamp: Stamp, action: Action) {
        let mut requests = self.requests.borrow_mut();
        if requests.len() < REQUEST_LIMIT {
            requests.push_back(Request { stamp, action });
        }
    }
    /// Read current native geometry, including toolkit animation frames.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.active
            .borrow()
            .as_ref()
            .and_then(|a| a.handle.rect())
            .is_some_and(|(rx, ry, rw, rh)| x >= rx && x < rx + rw && y >= ry && y < ry + rh)
    }
    pub fn focus(&self) -> bool {
        let active = self.active.borrow();
        let Some(active) = active.as_ref() else {
            return false;
        };
        let action = active
            .snapshot
            .controls
            .iter()
            .flatten()
            .find(|c| c.enabled)
            .map_or(Action::Dismiss, |c| c.action);
        active.handle.focus_action(action_id(action))
    }
}

fn action_id(action: Action) -> &'static str {
    match action {
        Action::Details => "details",
        Action::Undo => "undo",
        Action::Redo => "redo",
        Action::Dismiss => "dismiss",
    }
}
fn label(action: Action) -> &'static str {
    match action {
        Action::Details => "Ver informe",
        Action::Undo => "Deshacer",
        Action::Redo => "Rehacer",
        Action::Dismiss => "Entendido",
    }
}

#[derive(Default)]
pub struct Manager {
    inbox: Inbox,
    pub accepted: u64,
    pub rejected: u64,
}
impl Manager {
    pub fn inbox(&self) -> Inbox {
        self.inbox.clone()
    }
    pub fn close(&mut self) {
        let active = self.inbox.active.borrow_mut().take();
        if let Some(active) = active {
            active.handle.close();
        }
    }
    /// Closing may restore native focus, so callers must not hold Model.
    pub fn drain(&mut self, model: &Rc<RefCell<Model>>) -> bool {
        let mut dirty = false;
        loop {
            let request = self.inbox.requests.borrow_mut().pop_front();
            let Some(request) = request else { break };
            let valid = {
                let mut m = model.borrow_mut();
                let Model {
                    operation_feedback,
                    operation_message,
                    ..
                } = &mut *m;
                operation_feedback.observe_info(operation_message.as_deref());
                m.operation_feedback
                    .permits(request.stamp, request.action, context(&m))
            };
            if !valid {
                self.rejected += 1;
                continue;
            }
            self.close();
            let mut m = model.borrow_mut();
            if !m
                .operation_feedback
                .permits(request.stamp, request.action, context(&m))
            {
                self.rejected += 1;
                continue;
            }
            self.accepted += 1;
            match request.action {
                Action::Dismiss => {
                    m.operation_feedback.dismiss(request.stamp);
                }
                Action::Details => m.show_copy_details(),
                Action::Undo => m.undo_rename(false),
                Action::Redo => m.undo_rename(true),
            }
            dirty = true;
        }
        dirty
    }
    /// A failed toolkit anchor leaves the existing canvas feedback available.
    pub fn sync(&mut self, snapshot: Option<Snapshot>, anchor: &Frame) -> bool {
        let Some(snapshot) = snapshot else {
            self.close();
            return true;
        };
        if self
            .inbox
            .active
            .borrow()
            .as_ref()
            .is_some_and(|a| a.snapshot == snapshot && a.handle.is_open())
        {
            return true;
        }
        self.close();
        let summary = if snapshot.stamp.source == Source::Info {
            "Files"
        } else {
            "Resultado de operación"
        };
        let mut notice = Notice::new(summary, &snapshot.body).icon("");
        for control in snapshot.controls.iter().flatten() {
            let mut action = NoticeAction::new(action_id(control.action), label(control.action));
            if !control.enabled {
                action = action.disabled();
            }
            notice = notice.action(action);
        }
        notice = notice.action(NoticeAction::new("dismiss", "Entendido"));
        let inbox = self.inbox.clone();
        let stamp = snapshot.stamp;
        notice = notice.on_action(move |_, id| {
            let action = match id {
                "details" => Action::Details,
                "undo" => Action::Undo,
                "redo" => Action::Redo,
                "dismiss" => Action::Dismiss,
                _ => return,
            };
            inbox.push(stamp, action);
        });
        let inbox = self.inbox.clone();
        notice = notice.on_dismiss(move |why| {
            if why != NoticeDismiss::Closed {
                inbox.push(stamp, Action::Dismiss);
            }
        });
        let Some(handle) = notice.show(anchor) else {
            return false;
        };
        *self.inbox.active.borrow_mut() = Some(Active { snapshot, handle });
        true
    }
    pub fn fixture(&self, m: &Model, canvas: &Frame) -> serde_json::Value {
        let focus = app::focus();
        let active = self.inbox.active.borrow();
        let card = active.as_ref().map(|a| {
            let r = a.handle.rect();
            serde_json::json!({"revision":a.snapshot.stamp.revision,"source":format!("{:?}",a.snapshot.stamp.source),
                "text":&*a.snapshot.body,"rect":r,"visible":a.handle.is_open(),
                "details_rect":a.handle.action_rect("details"),"undo_rect":a.handle.action_rect("undo"),
                "redo_rect":a.handle.action_rect("redo"),"dismiss_rect":a.handle.action_rect("dismiss"),"close_rect":a.handle.close_rect(),
                "focused":focus.as_ref().is_some_and(|w|r==Some((w.x(),w.y(),w.w(),w.h())))})
        });
        serde_json::json!({"card":card,"capacity":1,"queue_len":self.inbox.requests.borrow().len(),"request_capacity":REQUEST_LIMIT,
            "accepted":self.accepted,"rejected":self.rejected,"browser_focus":focus.as_ref().is_some_and(|w|w.as_widget_ptr()==canvas.as_widget_ptr()),
            "focus_rect":focus.map(|w|(w.x(),w.y(),w.w(),w.h())),"generation":m.generation,"owner":m.pane_owner(),
            "selection_raw":m.selection.iter().filter_map(|i|m.entries.get(*i)).map(|e|e.name.as_encoded_bytes()).collect::<Vec<_>>(),
            "selected_raw":m.selected.and_then(|i|m.entries.get(i)).map(|e|e.name.as_encoded_bytes()),
            "anchor_raw":m.anchor.and_then(|i|m.entries.get(i)).map(|e|e.name.as_encoded_bytes()),
            "modal":m.notice_open,"copy_result":m.copy_last.as_ref().map(|r|r.id),"can_undo":m.can_undo,"can_redo":m.can_redo,
            "mutation_busy":m.mutations.busy,"worker_pending":m.copy_busy(),"closing":m.closing,"legacy_feedback":!m.toolkit_feedback,
            "record_revision":m.operation_feedback.stamp().map(|s|s.revision),
            "rename_form":m.rename_dialog.as_ref().map(|d|serde_json::json!({"prepared":d.prepared.is_some(),"changed":d.changed,"error":d.error})),
            "copy_form":m.copy_form.as_ref().map(|d|serde_json::json!({"preparing":d.preparing,"error":d.error})),
            "width":canvas.w(),"height":canvas.h()})
    }
}
impl Drop for Manager {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> Rc<RefCell<Model>> {
        // No widget is created. The auxiliary persistence worker cannot publish
        // this model fixture's preferences or sessions.
        let mut model = Model::new(std::env::temp_dir().join("operation-notice-model-only"));
        model.worker.suspend();
        let (tx, _) = std::sync::mpsc::sync_channel(4);
        let (_, rx) = std::sync::mpsc::channel();
        model.aux = crate::aux_io::AuxIo { tx, rx };
        Rc::new(RefCell::new(model))
    }

    #[test]
    fn queued_dismiss_cannot_acknowledge_a_critical_review_or_release_its_queue() {
        let model = model();
        let stamp = {
            let mut m = model.borrow_mut();
            m.operation_message = Some("Estado de archivos sin confirmar".into());
            m.operation_details = vec!["El resultado requiere una decisión explícita".into()];
            m.notice_open = true;
            m.copy_notice_id = Some(7);
            m.copy.hold_queue(true);
            m.operation_feedback
                .publish_copy(7, "Estado de archivos sin confirmar")
        };
        let mut manager = Manager::default();
        manager.inbox.push(stamp, Action::Dismiss);
        assert!(!manager.drain(&model));
        assert_eq!((manager.accepted, manager.rejected), (0, 1));
        let m = model.borrow();
        assert!(m.notice_open && m.copy.queue_held());
        assert_eq!(m.copy_notice_id, Some(7));
        assert_eq!(
            m.operation_message.as_deref(),
            Some("Estado de archivos sin confirmar")
        );
        assert_eq!(
            m.operation_details,
            ["El resultado requiere una decisión explícita"]
        );
        assert_eq!(m.operation_feedback.stamp(), Some(stamp));
        assert!(
            m.operation_feedback
                .snapshot(Context {
                    visible: true,
                    ..Context::default()
                })
                .is_some(),
            "the protected result was not dismissed"
        );
    }

    #[test]
    fn queued_history_action_rechecks_a_new_legacy_message_before_dispatch() {
        let model = model();
        let (old, mutation_id) = {
            let mut m = model.borrow_mut();
            m.can_undo = true;
            let old = m
                .operation_feedback
                .publish_mutation(18, "Nombres actualizados", true);
            // This assignment occurred after the widget's last synchronization.
            m.operation_message = Some("Selecciona un archivo".into());
            (old, m.mutation_id)
        };
        let mut manager = Manager::default();
        manager.inbox.push(old, Action::Undo);
        assert!(!manager.drain(&model));
        let info = model.borrow().operation_feedback.stamp().unwrap();
        assert_eq!(info.source, Source::Info);
        manager.inbox.push(info, Action::Undo);
        assert!(!manager.drain(&model));
        assert_eq!((manager.accepted, manager.rejected), (0, 2));
        let m = model.borrow();
        assert_eq!(m.mutation_id, mutation_id);
        assert!(m.can_undo && !m.mutation_applied && !m.mutations.busy);
        assert_eq!(
            m.operation_message.as_deref(),
            Some("Selecciona un archivo")
        );
    }

    #[test]
    fn bounded_inbox_accepts_dismiss_once_without_clearing_the_model_payload() {
        let model = model();
        let stamp = {
            let mut m = model.borrow_mut();
            m.operation_message = Some("Resultado conservado".into());
            m.operation_details = vec!["El informe sigue disponible".into()];
            m.operation_feedback
                .observe_info(Some("Resultado conservado"));
            m.operation_feedback.stamp().unwrap()
        };
        let mut manager = Manager::default();
        for _ in 0..REQUEST_LIMIT + 5 {
            manager.inbox.push(stamp, Action::Dismiss);
        }
        assert_eq!(manager.inbox.requests.borrow().len(), REQUEST_LIMIT);
        assert!(manager.drain(&model));
        assert_eq!(manager.accepted, 1);
        assert_eq!(manager.rejected, (REQUEST_LIMIT - 1) as u64);
        assert!(manager.inbox.requests.borrow().is_empty());
        let m = model.borrow();
        assert_eq!(m.operation_message.as_deref(), Some("Resultado conservado"));
        assert_eq!(m.operation_details, ["El informe sigue disponible"]);
        assert!(!m.notice_open && !m.copy.queue_held());
        assert!(m.operation_feedback.snapshot(context(&m)).is_none());
    }

    #[test]
    fn queued_dismiss_of_an_identical_old_typed_result_cannot_hide_its_replacement() {
        let model = model();
        let (old, current) = {
            let mut m = model.borrow_mut();
            m.operation_message = Some("Copia terminada".into());
            let old = m.operation_feedback.publish_copy(41, "Copia terminada");
            let current = m.operation_feedback.publish_copy(42, "Copia terminada");
            (old, current)
        };
        let mut manager = Manager::default();
        manager.inbox.push(old, Action::Dismiss);
        assert!(!manager.drain(&model));
        assert!(
            model
                .borrow()
                .operation_feedback
                .snapshot(context(&model.borrow()))
                .is_some()
        );
        manager.inbox.push(current, Action::Dismiss);
        manager.inbox.push(current, Action::Dismiss);
        assert!(manager.drain(&model));
        assert_eq!((manager.accepted, manager.rejected), (1, 2));
        let m = model.borrow();
        assert_eq!(m.operation_feedback.stamp(), Some(current));
        assert!(m.operation_feedback.snapshot(context(&m)).is_none());
        assert_eq!(m.operation_message.as_deref(), Some("Copia terminada"));
    }
}

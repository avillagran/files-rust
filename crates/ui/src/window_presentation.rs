//! One-shot native presentation after external UI admission.
use crate::{file_manager_ui::PresentationIntent, model::Model};
use fltk::{frame::Frame, prelude::*, window::Window};
use fltk_omarchy::native::{self, PresentBackend, PresentError, PresentOptions, PresentStatus};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

const FIRST_PAINT_DEADLINE: Duration = Duration::from_secs(2);

struct Pending {
    intent: PresentationIntent,
    after_paint: Option<u64>,
    queued_at: Instant,
}
impl Pending {
    fn gate(&self, current: bool, paint: u64, now: Instant) -> Gate {
        paint_gate(current, self.after_paint, paint, self.queued_at, now)
    }
}
fn paint_gate(
    current: bool,
    after_paint: Option<u64>,
    paint: u64,
    queued_at: Instant,
    now: Instant,
) -> Gate {
    if !current {
        Gate::Stale
    } else if now.saturating_duration_since(queued_at) >= FIRST_PAINT_DEADLINE {
        Gate::Expired
    } else if after_paint.is_some_and(|before| paint <= before) {
        Gate::Wait
    } else {
        Gate::Ready
    }
}
#[derive(Debug, PartialEq, Eq)]
enum Gate {
    Wait,
    Ready,
    Stale,
    Expired,
}

#[derive(Default)]
pub struct Manager {
    close_requested: Rc<Cell<bool>>,
    pending: Option<Pending>,
    sequence: u64,
    attempts: u64,
    superseded: u64,
    skipped: u64,
    errors: u64,
    last: Option<Value>,
}
impl Manager {
    pub fn new(close_requested: Rc<Cell<bool>>) -> Self {
        Self {
            close_requested,
            ..Default::default()
        }
    }
    /// A newer admitted request consumes any older context, even if stale.
    /// Mapping and focus calls must run without a Model borrow: FLTK may
    /// synchronously dispatch widget callbacks.
    pub fn schedule(
        &mut self,
        intent: PresentationIntent,
        model: &Rc<RefCell<Model>>,
        window: &mut Window,
        canvas: &mut Frame,
        paint: u64,
    ) -> bool {
        if self.pending.take().is_some() {
            self.superseded += 1;
        }
        self.sequence += 1;
        let has_hint = !intent.startup_id().is_empty();
        let current =
            !self.close_requested.get() && model.try_borrow().is_ok_and(|m| intent.is_current(&m));
        if !current || window.was_deleted() || canvas.was_deleted() {
            self.skipped += 1;
            self.record(has_hint, false, "stale", None, None);
            return false;
        }
        let hidden = !window.shown();
        self.pending = Some(Pending {
            intent,
            after_paint: hidden.then_some(paint),
            queued_at: Instant::now(),
        });
        if hidden {
            window.show();
        }
        if window.was_deleted() || canvas.was_deleted() || !window.shown() {
            self.pending = None;
            self.skipped += 1;
            self.record(has_hint, false, "window-lost", None, None);
            return false;
        }
        canvas.redraw();
        self.record(
            has_hint,
            true,
            if hidden { "awaiting-paint" } else { "queued" },
            None,
            None,
        );
        true
    }

    /// Run before listing/watch pumping so the initial attachment refresh
    /// cannot invalidate an otherwise current first-frame request. User
    /// navigation, edits and competing dialogs still invalidate ownership.
    pub fn pump(
        &mut self,
        model: &Rc<RefCell<Model>>,
        window: &mut Window,
        canvas: &mut Frame,
        paint: u64,
    ) {
        let Some(pending) = self.pending.as_ref() else {
            return;
        };
        if window.was_deleted() || canvas.was_deleted() || !window.shown() {
            let has_hint = !pending.intent.startup_id().is_empty();
            self.pending = None;
            self.skipped += 1;
            self.record(has_hint, false, "window-lost", None, None);
            return;
        }
        let Ok(m) = model.try_borrow() else {
            return;
        };
        let gate = pending.gate(
            !self.close_requested.get() && pending.intent.is_current(&m),
            paint,
            Instant::now(),
        );
        drop(m);
        if gate == Gate::Wait {
            return;
        }
        let pending = self.pending.take().expect("checked pending presentation");
        let has_hint = !pending.intent.startup_id().is_empty();
        if gate != Gate::Ready {
            self.skipped += 1;
            self.record(
                has_hint,
                window.shown(),
                if gate == Gate::Stale {
                    "stale"
                } else {
                    "expired"
                },
                None,
                None,
            );
            return;
        }
        if window.was_deleted() || canvas.was_deleted() || !window.shown() {
            self.skipped += 1;
            self.record(has_hint, false, "window-lost", None, None);
            return;
        }
        // This restores the existing canvas keyboard target, not native
        // foreground ownership. Native activation is exclusively the API below.
        let _ = canvas.take_focus();
        if window.was_deleted() || canvas.was_deleted() || !window.shown() {
            self.skipped += 1;
            self.record(has_hint, false, "window-lost", None, None);
            return;
        }
        if self.close_requested.get()
            || !model
                .try_borrow()
                .is_ok_and(|m| pending.intent.is_current(&m))
        {
            self.skipped += 1;
            self.record(has_hint, window.shown(), "stale", None, None);
            return;
        }
        self.attempts += 1;
        let result = native::present(
            window,
            PresentOptions {
                startup_id: Some(pending.intent.startup_id()),
                ..Default::default()
            },
        );
        match result {
            Ok(result) => self.record(
                has_hint,
                window.shown(),
                status(result.status),
                Some(backend(result.backend)),
                None,
            ),
            Err(error) => {
                self.errors += 1;
                self.record(
                    has_hint,
                    !window.was_deleted() && window.shown(),
                    "error",
                    None,
                    Some(error_kind(&error)),
                );
            }
        }
    }

    pub fn close(&mut self) {
        self.pending = None;
    }

    fn record(
        &mut self,
        has_hint: bool,
        visible: bool,
        status: &str,
        backend: Option<&str>,
        error: Option<&str>,
    ) {
        self.last = Some(json!({"sequence":self.sequence,"has_startup_id":has_hint,
            "visible":visible,"status":status,"backend":backend,"error":error}));
    }

    pub fn marker(&self) -> Value {
        json!({"pending":self.pending.is_some(),"requests":self.sequence,
            "attempts":self.attempts,"superseded":self.superseded,
            "skipped":self.skipped,"errors":self.errors,"last":self.last})
    }
}
fn backend(value: PresentBackend) -> &'static str {
    match value {
        PresentBackend::Wayland => "wayland",
        PresentBackend::X11 => "x11",
        PresentBackend::Windows => "windows",
        PresentBackend::MacOs => "macos",
        PresentBackend::Other => "other",
    }
}
fn status(value: PresentStatus) -> &'static str {
    match value {
        PresentStatus::Unsupported => "unsupported",
        PresentStatus::Requested => "requested",
        PresentStatus::Granted => "granted",
        PresentStatus::Refused => "refused",
    }
}
fn error_kind(value: &PresentError) -> &'static str {
    match value {
        PresentError::WrongThread => "wrong-thread",
        PresentError::DeletedWindow => "deleted-window",
        PresentError::ChildWindow => "child-window",
        PresentError::InvalidHint => "invalid-hint",
        PresentError::Busy => "busy",
        PresentError::TimedOut => "timed-out",
        PresentError::Native(_) => "native-error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_paint_wait_is_bounded_and_stale_never_presents() {
        let now = Instant::now();
        assert_eq!(paint_gate(true, Some(4), 4, now, now), Gate::Wait);
        assert_eq!(paint_gate(true, Some(4), 5, now, now), Gate::Ready);
        assert_eq!(paint_gate(true, None, 0, now, now), Gate::Ready);
        for paint in [4, 5] {
            assert_eq!(paint_gate(false, Some(4), paint, now, now), Gate::Stale);
            assert_eq!(
                paint_gate(true, Some(4), paint, now, now + FIRST_PAINT_DEADLINE),
                Gate::Expired
            );
        }
    }
    #[test]
    fn diagnostic_never_copies_native_error_or_claims_request_as_grant() {
        let mut manager = Manager::default();
        manager.record(
            true,
            true,
            status(PresentStatus::Requested),
            Some(backend(PresentBackend::Wayland)),
            Some(error_kind(&PresentError::Native(
                "private-context-secret".into(),
            ))),
        );
        let marker = manager.marker();
        assert_eq!(marker["last"]["status"], "requested");
        assert_eq!(marker["last"]["error"], "native-error");
        assert!(!marker.to_string().contains("private-context-secret"));
        assert_ne!(
            status(PresentStatus::Granted),
            status(PresentStatus::Requested)
        );
        manager.close();
        assert_eq!(manager.marker()["pending"], false);
    }
}

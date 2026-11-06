//! Keyed toolkit presentation. Files owns admission, snapshots and selection.
use crate::{
    model::{Action, Hit, Model},
    painter::Rect,
    tabs::{self, Command},
};
use fltk::{frame::Frame, group::Group, prelude::*, window::Window};
use fltk_omarchy::{State, Subscription, model::ListModel, widgets::KeyedTabBar};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::Rc,
};

pub type Key = (u64, u64);
#[derive(Clone, PartialEq)]
struct Row {
    key: Key,
    title: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp {
    owner: Key,
    generation: u64,
}
#[derive(Clone, Debug)]
enum Kind {
    Select,
    Close,
    Context(i32, i32),
    Move(Option<Key>),
}
#[derive(Clone, Debug)]
struct Request {
    stamp: Stamp,
    key: Key,
    kind: Kind,
}
fn interactive(m: &Model) -> bool {
    !crate::tab_gestures::blocked(m)
}
fn admit(m: &Model, request: &Request) -> bool {
    interactive(m)
        && request.stamp.owner == m.pane_owner()
        && request.stamp.generation == m.generation
        && request.key.0 == m.dual.active_id
        && m.tabs.index(request.key.1).is_some()
        && match request.kind {
            Kind::Move(Some(before)) => {
                before.0 == request.key.0
                    && before != request.key
                    && m.tabs.index(before.1).is_some()
            }
            _ => true,
        }
}
#[derive(Clone, Default)]
struct Inbox {
    stamp: Rc<Cell<Option<Stamp>>>,
    syncing: Rc<Cell<bool>>,
    queue: Rc<RefCell<VecDeque<Request>>>,
    isolated: bool,
    fixture: Rc<Cell<bool>>,
    reconcile: Rc<Cell<bool>>,
}
impl Inbox {
    fn push(&self, key: Key, kind: Kind) {
        if self.syncing.get() {
            return;
        }
        // Even a covered or isolated widget may have changed its local State
        // before notifying us. Restore accepted app state on the next paint.
        self.reconcile.set(true);
        if self.isolated && !self.fixture.get() {
            return;
        }
        if let Some(stamp) = self.stamp.get() {
            let mut queue = self.queue.borrow_mut();
            // A native event normally produces one request. Bound retained input.
            if queue.len() < 32 {
                queue.push_back(Request { stamp, key, kind });
            }
        }
    }
    fn take(&self) -> VecDeque<Request> {
        let mut result: VecDeque<Request> = VecDeque::new();
        for request in std::mem::take(&mut *self.queue.borrow_mut()) {
            // Native cycling updates widget selection synchronously. If a
            // backend delivers a burst before the app pump, retain its final
            // intent without intermediate browser loads or stale rejection.
            if matches!(request.kind, Kind::Select)
                && result.back().is_some_and(|last| {
                    last.stamp == request.stamp && matches!(last.kind, Kind::Select)
                })
            {
                result.pop_back();
            }
            result.push_back(request);
        }
        result
    }
}
#[derive(Clone)]
pub struct Strip {
    pub widget: KeyedTabBar<Key>,
    group: Group,
    background: Rc<Cell<(u8, u8, u8)>>,
    displayed: Rc<Cell<Option<Stamp>>>,
    rows: ListModel<Key, Row>,
    selected: State<Option<Key>>,
    inbox: Inbox,
    _subscription: Rc<Subscription>,
}
impl Strip {
    pub fn new(width: i32, isolated: bool, fixture: Rc<Cell<bool>>) -> Self {
        let rows = ListModel::new(|row: &Row| row.key);
        let mut group = Group::new(8, 58, (width - 48).max(0), tabs::HEIGHT, "");
        let selected = State::new(None);
        let inbox = Inbox {
            isolated,
            fixture,
            ..Inbox::default()
        };
        let selected_inbox = inbox.clone();
        let subscription = selected.subscribe(move |key| {
            if let Some(key) = key {
                selected_inbox.push(*key, Kind::Select);
            }
        });
        let (close, context, reorder) = (inbox.clone(), inbox.clone(), inbox.clone());
        let mut widget =
            fltk_omarchy::widgets::TabBar::from_model(&rows, &selected, |row| row.title.clone())
                .on_close(move |key| close.push(key, Kind::Close))
                .on_context(move |r| {
                    // Focused widgets see KeyDown before their parent window.
                    // Keep the app's active-tab chord distinct from cursor Menu.
                    let active_chord = fltk::app::event_key() == fltk::enums::Key::F10
                        && fltk::app::event_state()
                            .contains(fltk::enums::Shortcut::Ctrl | fltk::enums::Shortcut::Shift);
                    let key = if active_chord {
                        context.stamp.get().map_or(r.key, |s| s.owner)
                    } else {
                        r.key
                    };
                    context.push(key, Kind::Context(r.x, r.y));
                })
                .reorderable(move |r| reorder.push(r.key, Kind::Move(r.before)));
        widget.resize(8, 58, (width - 48).max(0), tabs::HEIGHT);
        group.end();
        let background = Rc::new(Cell::new((0, 0, 0)));
        let color = background.clone();
        let mut child = widget.clone();
        group.draw(move |g| {
            // Transparent toolkit controls need their host's background on
            // every child animation frame, not only full canvas redraws.
            use crate::painter::Painter;
            let c = color.get();
            crate::painter::FltkPainter.rect(
                Rect::new(g.x(), g.y(), g.w(), g.h()),
                files_core::theme::Color(c.0, c.1, c.2),
            );
            g.draw_child(&mut *child);
        });
        group.super_draw(false);
        Self {
            widget,
            group,
            background,
            displayed: Rc::new(Cell::new(None)),
            rows,
            selected,
            inbox,
            _subscription: Rc::new(subscription),
        }
    }
    /// Run outside widget callbacks and before session/listing pumps. Rejected
    /// requests never write the subscribed State reentrantly.
    pub fn drain(&self, m: &mut Model) -> bool {
        let requests = self.inbox.take();
        let dirty = self.inbox.reconcile.replace(false) || !requests.is_empty();
        for request in requests {
            if !admit(m, &request) {
                continue;
            }
            m.session.restore_allowed = false;
            match request.kind {
                Kind::Select => m.action(Action::Tab(Command::Select(request.key.1))),
                Kind::Close => m.action(Action::Tab(Command::Close(request.key.1))),
                Kind::Move(before) => m.action(Action::Tab(Command::Move {
                    id: request.key.1,
                    before: before.map(|key| key.1),
                })),
                Kind::Context(x, y) => {
                    crate::tab_context::open(m, request.key.1, x, y);
                    if let Some(menu) = &mut m.tab_context {
                        menu.retain_focus = fltk::app::focus()
                            .is_some_and(|w| w.as_widget_ptr() == self.widget.as_widget_ptr());
                    }
                }
            }
        }
        dirty
    }
    /// Call with the app model held, protected from synchronous selection
    /// notifications. The exact guard also covers deferred batch flushes.
    pub fn sync(&mut self, m: &mut Model, width: i32) {
        m.tabs_widget = true;
        let color = m.palette.darker_background;
        self.background.set((color.0, color.1, color.2));
        self.displayed.set(Some(Stamp {
            owner: m.pane_owner(),
            generation: m.generation,
        }));
        let visible = !crate::places_sidebar::covered(m);
        self.inbox.stamp.set(if interactive(m) && visible {
            Some(Stamp {
                owner: m.pane_owner(),
                generation: m.generation,
            })
        } else {
            None
        });
        self.inbox.syncing.set(true);
        fltk_omarchy::batch(|| {
            self.rows.sync_from(m.tabs.rows.iter().map(|tab| {
                Row {
                    key: (m.dual.active_id, tab.id),
                    title: files_core::name_label::label(
                        tab.path
                            .file_name()
                            .unwrap_or_else(|| std::ffi::OsStr::new("/")),
                    )
                    .into_owned(),
                }
            }));
            self.selected.set_if_changed(Some(m.pane_owner()));
        });
        self.inbox.syncing.set(false);
        let rect = (8, 58, (width - 48).max(0), tabs::HEIGHT);
        if (
            self.widget.x(),
            self.widget.y(),
            self.widget.w(),
            self.widget.h(),
        ) != rect
        {
            self.group.resize(rect.0, rect.1, rect.2, rect.3);
            self.widget.resize(rect.0, rect.1, rect.2, rect.3);
        }
        if visible {
            if !self.widget.visible() {
                self.widget.show();
            }
            if !self.group.visible() {
                self.group.show();
            }
        } else if self.widget.visible() {
            self.widget.hide();
            self.group.hide();
        }
    }
    /// Export authoritative geometry after draw/reveal, never approximate widths.
    pub fn paint(&mut self, m: &mut Model, win: &Window) {
        if !self.widget.visible_r() {
            return;
        }
        win.draw_child(&mut self.group);
        self.refresh_hits(m);
    }
    pub fn refresh_hits(&self, m: &mut Model) {
        m.hits.retain(|h| {
            !matches!(
                h.action,
                Action::Tab(Command::Select(_) | Command::Close(_))
            )
        });
        if !self.widget.visible_r()
            || self.displayed.get()
                != Some(Stamp {
                    owner: m.pane_owner(),
                    generation: m.generation,
                })
        {
            return;
        }
        for tab in &m.tabs.rows {
            let key = (m.dual.active_id, tab.id);
            if let Some(rect) = self.body_rect(key) {
                m.hits.push(Hit {
                    generation: m.generation,
                    rect,
                    action: Action::Tab(Command::Select(tab.id)),
                });
            }
            if let Some((x, y, w, h)) = self.widget.close_rect_for_key(&key) {
                m.hits.push(Hit {
                    generation: m.generation,
                    rect: Rect::new(x, y, w, h),
                    action: Action::Tab(Command::Close(tab.id)),
                });
            }
        }
    }
    fn body_rect(&self, key: Key) -> Option<Rect> {
        let (x, w) = self.widget.rect_for_key(&key)?;
        let (left, right) = if let Some((cx, _, cw, _)) = self.widget.close_rect_for_key(&key) {
            if cx <= x { (cx + cw, x + w) } else { (x, cx) }
        } else {
            (x, x + w)
        };
        (right > left).then_some(Rect::new(
            left,
            self.widget.y(),
            right - left,
            self.widget.h(),
        ))
    }
    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.widget.visible_r()
            && x >= self.widget.x()
            && x < self.widget.x() + self.widget.w()
            && y >= self.widget.y()
            && y < self.widget.y() + self.widget.h()
    }
    pub fn active_context(&self, m: &mut Model) {
        if !interactive(m) {
            return;
        }
        let x = self
            .widget
            .rect_for_key(&m.pane_owner())
            .map_or(self.widget.x(), |(x, _)| x);
        crate::tab_context::open(m, m.tabs.active, x, self.widget.y() + self.widget.h());
        if let Some(menu) = &mut m.tab_context {
            menu.retain_focus = fltk::app::focus()
                .is_some_and(|w| w.as_widget_ptr() == self.widget.as_widget_ptr());
        }
    }
    pub fn marker(&self, m: &Model, canvas: &Frame) -> serde_json::Value {
        let mut state = crate::tab_context_scene::state(m, canvas);
        state["context_tab"] = m
            .tab_context
            .as_ref()
            .map(|menu| serde_json::json!([menu.owner.0, menu.id]))
            .unwrap_or_default();
        let rows: Vec<_> = m
            .tabs
            .rows
            .iter()
            .map(|tab| {
                let key = (m.dual.active_id, tab.id);
                let body = self.body_rect(key).map(|r| [r.x, r.y, r.w, r.h]);
                serde_json::json!({"key":key,"id":tab.id,"path":tab.path,"body_rect":body,
                "close_rect":self.widget.close_rect_for_key(&key)})
            })
            .collect();
        state["widget"] = serde_json::json!({"visible":self.widget.visible_r(),
            "focus":fltk::app::focus().is_some_and(|f|f.as_widget_ptr()==self.widget.as_widget_ptr()),
            "keyboard_mode":fltk_omarchy::focus::keyboard_mode(),
            "cursor":self.widget.focused_key(),"rows":rows,
            "rect":[self.widget.x(),self.widget.y(),self.widget.w(),self.widget.h()]});
        state
    }
    pub fn fixture(&self, m: &mut Model, text: &str) -> Result<(), &'static str> {
        if let Some(id) = text.strip_prefix("tab-widget-focus:") {
            let id = id.parse().map_err(|_| "invalid tab fixture id")?;
            if !self.widget.focus_key(&(m.dual.active_id, id)) {
                return Err("tab focus unavailable");
            }
        } else {
            match text {
                "tab-widget-seed-dual" => {
                    if !m.dual.enabled {
                        m.toggle_dual();
                    }
                }
                "tab-widget-switch-pane" => m.switch_dual(),
                "tab-widget-seed-overflow" => {
                    let parent = m.path.parent().ok_or("fixture parent absent")?;
                    let paths: Vec<_> = (1..=tabs::MAX_TABS - m.tabs.rows.len())
                        .map(|i| parent.join(format!("extra-long-tab-label-{i:02}")))
                        .collect();
                    m.background_tabs(paths.iter().map(|p| (p.as_path(), None)))?;
                }
                _ => return Err("unknown keyed tab fixture"),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(m: &Model, key: Key, kind: Kind) -> Request {
        Request {
            stamp: Stamp {
                owner: m.pane_owner(),
                generation: m.generation,
            },
            key,
            kind,
        }
    }
    #[test]
    fn keyed_requests_reject_stale_generation_foreign_panes_and_removed_targets() {
        let mut m = Model::new("/fixture-keyed-requests".into());
        let key = m.pane_owner();
        let r = request(&m, key, Kind::Close);
        assert!(admit(&m, &r));
        assert!(!admit(&m, &request(&m, (key.0 + 1, key.1), Kind::Close)));
        assert!(!admit(
            &m,
            &request(&m, key, Kind::Move(Some((key.0 + 1, 99))))
        ));
        assert!(!admit(&m, &request(&m, key, Kind::Move(Some((key.0, 99))))));
        m.generation += 1;
        assert!(!admit(&m, &r));
        let r = request(&m, key, Kind::Select);
        m.editing = true;
        assert!(!admit(&m, &r));
    }
    #[test]
    fn keyed_inbox_is_bounded_and_programmatic_sync_does_not_emit_input() {
        let q = Inbox::default();
        q.stamp.set(Some(Stamp {
            owner: (1, 1),
            generation: 3,
        }));
        q.syncing.set(true);
        q.push((1, 2), Kind::Select);
        assert!(q.queue.borrow().is_empty());
        q.syncing.set(false);
        for _ in 0..40 {
            q.push((1, 2), Kind::Close);
        }
        assert_eq!(q.queue.borrow().len(), 32);
        q.stamp.set(None);
        q.reconcile.set(false);
        q.push((1, 3), Kind::Select);
        assert_eq!(q.queue.borrow().len(), 32);
        assert!(
            q.reconcile.get(),
            "reconcile a rejected local selection without admitting an action"
        );
    }
    #[test]
    fn keyed_selection_bursts_keep_latest_intent_without_crossing_other_actions() {
        let q = Inbox::default();
        q.stamp.set(Some(Stamp {
            owner: (1, 1),
            generation: 4,
        }));
        q.push((1, 2), Kind::Select);
        q.push((1, 3), Kind::Select);
        q.push((1, 4), Kind::Close);
        q.push((1, 2), Kind::Select);
        q.stamp.set(Some(Stamp {
            owner: (2, 1),
            generation: 5,
        }));
        q.push((2, 3), Kind::Select);
        let requests = q.take();
        assert_eq!(
            requests.iter().map(|r| r.key).collect::<Vec<_>>(),
            [(1, 3), (1, 4), (1, 2), (2, 3)]
        );
        assert!(q.queue.borrow().is_empty());
    }
}

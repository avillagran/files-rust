//! Native form controls around the model-owned mutation workflow.
//! All native notifications enqueue stamped intents; publishing never borrows Model.
use crate::{
    mutation_sheet_state::{Action, Controller, FIELD_BYTES, Inbox, Snapshot, Stamp},
    rename_dialog::Mode,
};
use files_core::rename_plan::Case;
use fltk::{
    app, draw,
    enums::{Align, Event, FrameType, Key},
    frame::Frame,
    group::{Group, Scroll, ScrollType},
    prelude::*,
};
use fltk_omarchy::{
    State,
    layout::{self, FlexExt},
    overlay::{Sheet, SheetDismiss},
    paint, theme,
    widgets::{Button, Label, TextField, Tone},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

type Published = Rc<RefCell<Option<Snapshot>>>;

#[derive(Clone)]
struct Controls {
    fields: Vec<TextField>,
    labels: Vec<Label>,
    form: Group,
    scroll: Scroll,
    directory: Label,
    status: Label,
    case: Option<Button>,
    regex: Option<Button>,
    preview: Option<Frame>,
    apply: Button,
    cancel: Button,
}

struct Active {
    epoch: u64,
    sheet: Sheet,
    title: State<String>,
    dismissible: State<bool>,
    published: Published,
    stamp: Rc<Cell<Option<Stamp>>>,
    rejected_edit: Rc<Cell<bool>>,
    controls: Rc<RefCell<Option<Controls>>>,
    builds: Rc<Cell<u64>>,
    focus: usize,
    positions: [(i32, i32); 5],
    mounted_build: u64,
    native_focus: Option<usize>,
    viewport: (i32, i32),
}

#[derive(Default)]
pub struct Manager {
    pub controller: Controller,
    active: Option<Active>,
    retiring: Vec<(Instant, Active)>,
}

impl Manager {
    pub fn is_open(&self) -> bool {
        self.active.as_ref().is_some_and(|a| a.sheet.is_open())
    }
    pub fn close(&mut self) {
        if let Some(a) = &self.active {
            a.stamp.set(None);
            a.sheet.close();
        }
    }

    pub fn sync(&mut self, snapshot: Option<Snapshot>, anchor: &Frame) -> bool {
        self.retiring
            .retain(|(at, _)| at.elapsed() < Duration::from_millis(500));
        let Some(snapshot) = snapshot else {
            self.close();
            return true;
        };
        if self
            .active
            .as_ref()
            .is_none_or(|a| a.epoch != snapshot.stamp.dialog)
        {
            if let Some(a) = self.active.take() {
                a.stamp.set(None);
                a.sheet.close();
                self.retiring.push((Instant::now(), a));
                if self.retiring.len() > 2 {
                    self.retiring.remove(0);
                }
            }
            // Each logical opening has fresh native undo/selection state.
            self.active = Some(Active::new(anchor, &snapshot, self.controller.inbox()));
        }
        let a = self.active.as_mut().unwrap();
        let opening = !a.sheet.is_open();
        let changed = a.published.borrow().as_ref() != Some(&snapshot);
        a.stamp.set(Some(snapshot.stamp));
        a.title.set(title(&snapshot));
        a.dismissible.set(snapshot.can_dismiss);
        *a.published.borrow_mut() = Some(snapshot.clone());
        if !a.sheet.open() {
            return false;
        }
        if let Some(controls) = a.controls.borrow().as_ref().cloned() {
            let rollback = controls
                .fields
                .iter()
                .enumerate()
                .any(|(i, f)| f.value() != snapshot.fields[i]);
            if opening || changed || rollback || a.rejected_edit.get() {
                controls.sync(&snapshot, a.rejected_edit.get());
                // The invalid native value is now visibly rolled back. A
                // subsequent Apply can use the unchanged accepted draft.
                a.rejected_edit.set(false);
            }
            if opening {
                let cold = a.mounted_build != a.builds.get();
                let mut input = controls.fields[a.focus].input();
                let _ = input.take_focus();
                if a.mounted_build == 0 {
                    let _ = input.set_position(input.value().len() as i32);
                    let _ = input.set_mark(0);
                } else if cold {
                    let value = input.value();
                    let (position, mark) = a.positions[a.focus];
                    let _ = input.set_position(boundary(&value, position));
                    let _ = input.set_mark(boundary(&value, mark));
                }
                a.mounted_build = a.builds.get();
                controls.reveal(&input);
            }
            if let Some(focused) = app::focus() {
                let pointer = focused.as_widget_ptr() as usize;
                let viewport = (controls.scroll.w(), controls.scroll.h());
                if a.native_focus != Some(pointer) || a.viewport != viewport {
                    let mut parent = Some(focused.as_base_widget());
                    while let Some(w) = parent {
                        if w.as_widget_ptr() == controls.form.as_widget_ptr() {
                            controls.reveal(&focused);
                            break;
                        }
                        parent = w.parent().map(|p| p.as_base_widget());
                    }
                }
                a.native_focus = Some(pointer);
                a.viewport = viewport;
                for (i, field) in controls.fields.iter().enumerate() {
                    let input = field.input();
                    if focused.as_widget_ptr() == input.as_widget_ptr() {
                        a.focus = i;
                        a.positions[i] = (input.position(), input.mark());
                    }
                }
            }
        }
        true
    }

    /// Private-fixture diagnostics only; callers explicitly request this marker.
    pub fn marker(&self) -> serde_json::Value {
        let Some(a) = &self.active else {
            return serde_json::json!({"open":false});
        };
        let s = a.published.borrow();
        let c = a.controls.borrow();
        let geometry = c.as_ref().filter(|c| !c.form.was_deleted() && !c.scroll.was_deleted()).map(|c| serde_json::json!({
            "fields": c.fields.iter().map(|f| rect(&f.input())).collect::<Vec<_>>(),
            "apply":rect(&*c.apply),"cancel":rect(&*c.cancel),
            "scroll":rect(&c.scroll),"preview":c.preview.as_ref().map(rect),
            "case":c.case.as_ref().map(|b| rect(&**b)),
            "regex":c.regex.as_ref().map(|b| rect(&**b)),
            "focus_field":c.fields.iter().position(|f| app::focus().is_some_and(|w| w.as_widget_ptr()==f.input().as_widget_ptr())),
            "values":c.fields.iter().map(TextField::value).collect::<Vec<_>>()
            ,"field_enabled":c.fields.iter().map(|f|f.input().active_r()).collect::<Vec<_>>()
            ,"apply_enabled":c.apply.active_r(),"status":c.status.text()
            ,"control_count":{"fields":c.fields.len(),"buttons":if c.case.is_some(){4}else{2},"preview":usize::from(c.preview.is_some())}
        }));
        serde_json::json!({
            "open":a.sheet.is_open(),"built":a.sheet.is_built(),"builds":a.builds.get(),
            "rect":a.sheet.rect(),"close":a.sheet.close_rect(),"dismissible":a.sheet.is_dismissible(),
            "epoch":a.epoch,"revision":s.as_ref().map(|s|s.stamp.revision),
            "planning":s.as_ref().is_some_and(|s|s.planning),"applying":s.as_ref().is_some_and(|s|s.applying),
            "closing":s.as_ref().is_some_and(|s|s.closing),"cancel_requested":s.as_ref().is_some_and(|s|s.cancel_requested),
            "can_edit":s.as_ref().is_some_and(|s|s.can_edit),"can_apply":s.as_ref().is_some_and(|s|s.can_apply),
            "preview_total":s.as_ref().map(|s|s.preview_total),"preview_start":s.as_ref().map(|s|s.preview_start),
            "error":s.as_ref().map(|s|s.error.as_str()),"geometry":geometry
        })
    }
}

fn rect<W: WidgetExt>(w: &W) -> (i32, i32, i32, i32) {
    (w.x(), w.y(), w.w(), w.h())
}
fn title(s: &Snapshot) -> String {
    match s.mode {
        Mode::File => "Crear archivo".into(),
        Mode::Folder => "Crear carpeta".into(),
        Mode::Rename if s.selected_count == 1 => "Renombrar elemento".into(),
        Mode::Rename => format!("Renombrar {} elementos", s.selected_count),
    }
}

impl Active {
    fn new(anchor: &Frame, snapshot: &Snapshot, inbox: Inbox) -> Self {
        let published = Rc::new(RefCell::new(Some(snapshot.clone())));
        let stamp = Rc::new(Cell::new(Some(snapshot.stamp)));
        let rejected_edit = Rc::new(Cell::new(false));
        let controls = Rc::new(RefCell::new(None));
        let builds = Rc::new(Cell::new(0u64));
        let title = State::new(title(snapshot));
        let dismissible = State::new(snapshot.can_dismiss);
        let (p, st, rejected, output, count) = (
            published.clone(),
            stamp.clone(),
            rejected_edit.clone(),
            controls.clone(),
            builds.clone(),
        );
        let dismiss_inbox = inbox.clone();
        let dismiss_stamp = stamp.clone();
        let rename = snapshot.mode == Mode::Rename;
        let sheet = Sheet::new(
            anchor,
            "",
            if rename { 780 } else { 500 },
            if rename { 610 } else { 300 },
            move |col, _| {
                count.set(count.get().saturating_add(1));
                let directory = Label::new("").tone(Tone::Dim);
                col.fit(&*directory);
                let mut scroll = Scroll::default();
                scroll.set_type(ScrollType::Vertical);
                scroll.set_frame(FrameType::NoBox);
                scroll.set_scrollbar_size(12);
                let mut form = Group::default();
                form.set_frame(FrameType::NoBox);
                let mut fields = Vec::new();
                let mut labels = Vec::new();
                for (i, label) in if rename {
                    vec![
                        "Patrón · {name} {dotext} {ext} {n:03}",
                        "Inicio",
                        "Paso",
                        "Buscar",
                        "Reemplazar por",
                    ]
                } else {
                    vec!["Nombre"]
                }
                .into_iter()
                .enumerate()
                {
                    labels.push(Label::new(label));
                    let (q, stamp, bad) = (inbox.clone(), st.clone(), rejected.clone());
                    let (submit, submit_stamp, submit_bad) =
                        (inbox.clone(), st.clone(), rejected.clone());
                    let field = TextField::new()
                        .placeholder(label)
                        .on_change(move |value| {
                            let accepted = stamp.get().is_some_and(|s| q.edit(s, i, &value));
                            bad.set(!accepted);
                        })
                        .on_submit(move |_| {
                            if !submit_bad.get()
                                && let Some(stamp) = submit_stamp.get()
                            {
                                submit.request(stamp, Action::Apply);
                            }
                        });
                    // A defensive allocation bound; the model enforces UTF-8 bytes.
                    field.input().set_maximum_size(4096);
                    fields.push(field);
                }
                let case = rename.then(|| {
                    let (q, stamp) = (inbox.clone(), st.clone());
                    Button::new("Caso: conservar").bordered().on_click(move || {
                        if let Some(s) = stamp.get() {
                            q.request(s, Action::CycleCase);
                        }
                    })
                });
                let regex = rename.then(|| {
                    let (q, stamp) = (inbox.clone(), st.clone());
                    Button::new("Literal").bordered().on_click(move || {
                        if let Some(s) = stamp.get() {
                            q.request(s, Action::ToggleRegex);
                        }
                    })
                });
                let preview = rename.then(|| {
                    let mut frame = Frame::default();
                    frame.set_visible_focus();
                    frame.set_tooltip("Vista previa · flechas, RePág/AvPág, Inicio/Fin o rueda");
                    let source = p.clone();
                    frame.draw(move |f| draw_preview(f, source.borrow().as_ref()));
                    let (q, stamp) = (inbox.clone(), st.clone());
                    frame.handle(move |f, ev| {
                        if matches!(ev, Event::Focus | Event::Unfocus) {
                            f.redraw();
                            return true;
                        }
                        if ev == Event::Push && app::event_mouse_button() == app::MouseButton::Left
                        {
                            let _ = f.take_focus();
                            f.redraw();
                            return true;
                        }
                        let delta = if ev == Event::MouseWheel {
                            app::event_dy_value() as isize * 3
                        } else if ev == Event::KeyDown {
                            match app::event_key() {
                                Key::Up => -1,
                                Key::Down => 1,
                                Key::PageUp => -5,
                                Key::PageDown => 5,
                                Key::Home => isize::MIN,
                                Key::End => isize::MAX,
                                _ => return false,
                            }
                        } else {
                            return false;
                        };
                        if let Some(s) = stamp.get() {
                            q.request(s, Action::ScrollBy(delta));
                        }
                        true
                    });
                    frame
                });
                form.end();
                form.make_resizable(false);
                scroll.end();
                let status = Label::new("").tone(Tone::Dim);
                col.fit(&*status);
                let mut footer = layout::row();
                let _space = layout::spacer();
                let (q, stamp) = (inbox.clone(), st.clone());
                let cancel = Button::new("Cancelar").bordered().on_click(move || {
                    if let Some(s) = stamp.get() {
                        q.request(s, Action::Cancel);
                    }
                });
                footer.fit(&*cancel);
                let (q, stamp, bad) = (inbox.clone(), st.clone(), rejected.clone());
                let apply = Button::new(if rename { "Renombrar" } else { "Crear" })
                    .primary()
                    .on_click(move || {
                        if !bad.get()
                            && let Some(s) = stamp.get()
                        {
                            q.request(s, Action::Apply);
                        }
                    });
                footer.fit(&*apply);
                footer.close_fitted();
                let c = Controls {
                    fields,
                    labels,
                    form,
                    scroll: scroll.clone(),
                    directory,
                    status,
                    case,
                    regex,
                    preview,
                    apply,
                    cancel,
                };
                let resize = c.clone();
                scroll.resize_callback(move |_, _, _, _, _| resize.layout(rename));
                c.layout(rename);
                *output.borrow_mut() = Some(c);
            },
        )
        .bind_title(&title)
        .bind_dismissible(&dismissible)
        .on_dismiss(move |reason| {
            if matches!(
                reason,
                SheetDismiss::Escape | SheetDismiss::ClickOutside | SheetDismiss::CloseButton
            ) && let Some(s) = dismiss_stamp.get()
            {
                dismiss_inbox.request(s, Action::Dismiss);
            }
        });
        Self {
            epoch: snapshot.stamp.dialog,
            sheet,
            title,
            dismissible,
            published,
            stamp,
            rejected_edit,
            controls,
            builds,
            focus: snapshot.focus,
            positions: [(0, 0); 5],
            mounted_build: 0,
            native_focus: None,
            viewport: (0, 0),
        }
    }
}

impl Controls {
    fn sync(&self, s: &Snapshot, rejected: bool) {
        for (i, field) in self.fields.iter().enumerate() {
            let mut input = field.input();
            if input.value() != s.fields[i] {
                // Roll back a rejected paste even if the accepted State did not change.
                // Leave matching native values alone, preserving IME/caret/undo.
                let position = boundary(&s.fields[i], input.position());
                let mark = boundary(&s.fields[i], input.mark());
                input.set_value(&s.fields[i]);
                let _ = input.set_position(position);
                let _ = input.set_mark(mark);
            }
            enabled(&mut *field.clone(), s.can_edit);
            enabled(&mut input, s.can_edit);
        }
        self.directory.clone().set_text(&s.directory);
        let status = if rejected {
            format!("Campo demasiado largo o edición pendiente (máximo {FIELD_BYTES} bytes)")
        } else if !s.error.is_empty() {
            s.error.clone()
        } else if s.applying {
            if s.cancel_requested {
                "Cancelación solicitada… Esperando confirmación".into()
            } else {
                "Aplicando cambios… Esperando confirmación".into()
            }
        } else if s.planning {
            "Calculando vista previa… Puedes seguir editando".into()
        } else if s.mode == Mode::Rename {
            format!(
                "{} cambios · revisa los nombres antes de aplicar",
                s.preview_total
            )
        } else {
            "Escribe un nombre y pulsa Crear".into()
        };
        self.status
            .clone()
            .tone(if rejected || !s.error.is_empty() {
                Tone::Urgent
            } else {
                Tone::Dim
            })
            .set_text(&status);
        self.status.clone().set_tooltip(&status);
        let mut apply = self.apply.clone();
        enabled(&mut *apply, s.can_apply);
        let mut cancel = self.cancel.clone();
        cancel.set_text(if s.cancel_requested {
            "Cancelación solicitada"
        } else if s.applying {
            "Solicitar cancelación"
        } else {
            "Cancelar"
        });
        enabled(&mut *cancel, !s.closing && !s.cancel_requested);
        if let Some(mut case) = self.case.clone() {
            case.set_text(match s.case {
                Case::Preserve => "Caso: conservar",
                Case::Lower => "Caso: minúsculas",
                Case::Upper => "Caso: MAYÚSCULAS",
            });
            enabled(&mut *case, s.can_edit);
        }
        if let Some(mut regex) = self.regex.clone() {
            regex.set_text(if s.regex {
                "Regex · $1 ${nombre} $$"
            } else {
                "Reemplazo literal"
            });
            enabled(&mut *regex, s.can_edit);
        }
        if let Some(mut preview) = self.preview.clone() {
            preview.redraw();
        }
    }

    fn layout(&self, rename: bool) {
        if self.scroll.was_deleted() {
            return;
        }
        let (x, y, w) = (
            self.scroll.x(),
            self.scroll.y() - self.scroll.yposition(),
            (self.scroll.w() - 16).max(1),
        );
        let narrow = w < 520;
        let field_h = theme::with(|t| t.font.body + 6 + 2 * t.spacing.input_padding_y);
        let line = field_h + 28;
        let mut height = line;
        let positions = if !rename {
            vec![(x, y, w)]
        } else if narrow {
            height = line * 5;
            (0..5).map(|i| (x, y + i * line, w)).collect()
        } else {
            height = line * 3;
            let half = (w - 12) / 2;
            vec![
                (x, y, w),
                (x, y + line, half),
                (x + half + 12, y + line, half),
                (x, y + 2 * line, half),
                (x + half + 12, y + 2 * line, half),
            ]
        };
        let mut form = self.form.clone();
        form.resize(x, y, w, height + if rename { 224 } else { 8 });
        for (i, (fx, fy, fw)) in positions.into_iter().enumerate() {
            self.labels[i].clone().resize(fx, fy, fw, 20);
            self.fields[i].clone().resize(fx, fy + 22, fw, field_h);
        }
        if let (Some(mut case), Some(mut regex), Some(mut preview)) =
            (self.case.clone(), self.regex.clone(), self.preview.clone())
        {
            let half = (w - 12) / 2;
            case.resize(x, y + height, half, 32);
            regex.resize(x + half + 12, y + height, half, 32);
            preview.resize(x, y + height + 44, w, 174);
        }
        form.redraw();
    }

    fn reveal<W: WidgetExt>(&self, w: &W) {
        if w.y() < self.scroll.y() || w.y() + w.h() > self.scroll.y() + self.scroll.h() {
            self.scroll.clone().scroll_to(
                0,
                (self.scroll.yposition() + w.y() - self.scroll.y() - 8).max(0),
            );
        }
    }
}

fn enabled<W: WidgetExt>(w: &mut W, on: bool) {
    if w.active() != on {
        if on { w.activate() } else { w.deactivate() }
    }
}
fn boundary(s: &str, position: i32) -> i32 {
    let mut p = (position.max(0) as usize).min(s.len());
    while !s.is_char_boundary(p) {
        p -= 1;
    }
    p as i32
}

fn draw_preview(frame: &mut Frame, s: Option<&Snapshot>) {
    let Some(s) = s else {
        return;
    };
    theme::with(|t| {
        let (x, y, w, h) = rect(frame);
        draw::push_clip(x, y, w, h);
        draw::set_draw_color(t.palette.background.fl());
        draw::draw_rectf(x, y, w, h);
        let half = w / 2;
        for (text, px) in [("ANTES", x + 8), ("DESPUÉS", x + half + 8)] {
            paint::text_elided(
                text,
                px,
                y,
                half - 16,
                26,
                t.font.body,
                false,
                t.palette.foreground.darker(1.6),
                Align::Left,
            );
        }
        for (i, row) in s
            .preview
            .iter()
            .take(((h - 26).max(0) / 28) as usize + 1)
            .enumerate()
        {
            let ry = y + 26 + i as i32 * 28;
            if i % 2 == 0 {
                draw::set_draw_color(t.palette.foreground.darker(7.0).fl());
                draw::draw_rectf(x, ry, w, 28);
            }
            paint::text_elided(
                &row.before,
                x + 8,
                ry,
                half - 16,
                28,
                t.font.body,
                false,
                t.palette.foreground,
                Align::Left,
            );
            paint::text_elided(
                &row.after,
                x + half + 8,
                ry,
                half - 16,
                28,
                t.font.body,
                false,
                t.palette.accent,
                Align::Left,
            );
        }
        if s.preview.is_empty() {
            paint::text_elided(
                if s.planning {
                    "Preparando vista previa…"
                } else {
                    "El patrón no cambia ningún nombre."
                },
                x + 8,
                y + 32,
                w - 16,
                30,
                t.font.body,
                false,
                t.palette.foreground.darker(1.6),
                Align::Left,
            );
        }
        if app::focus().is_some_and(|w| w.as_widget_ptr() == frame.as_widget_ptr()) {
            draw::set_draw_color(t.palette.accent.fl());
            draw::draw_rect(x, y, w, h);
        }
        draw::pop_clip();
    });
}

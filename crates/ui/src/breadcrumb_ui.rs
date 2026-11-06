//! Painted path chips and lossless overflow targets; no filesystem probes.
use crate::{
    breadcrumbs::Crumb,
    model::{Action, Hit, Model},
    painter::{Painter, Rect},
};
use fltk_omarchy::popups::PopupItem;
#[derive(Default)]
pub struct State {
    pub hidden: Vec<Crumb>,
    pub visible: Vec<(Crumb, Rect)>,
    pub overflow: Option<Rect>,
    pub area: Rect,
    pub open: bool,
}
impl State {
    pub fn clear_geometry(&mut self) {
        self.hidden.clear();
        self.visible.clear();
        self.overflow = None;
        self.area = Rect::default();
    }
}
pub fn available(m: &Model) -> bool {
    !crate::places_sidebar::covered(m)
        && !m.editing
        && !m.filter_edit.active
        && m.query.is_empty()
        && !m.settings
        && !m.actions_menu
        && m.context_menu.is_none()
        && !m.breadcrumbs.hidden.is_empty()
        && m.breadcrumbs.overflow.is_some()
}
pub fn open(m: &mut Model) {
    if available(m) {
        m.breadcrumbs.open = true;
    }
}

pub fn paint(p: &mut dyn Painter, m: &mut Model, area: Rect) {
    let all = crate::breadcrumbs::build(&m.path, &files_core::theme::home());
    let layout = crate::breadcrumbs::layout(&all, area.w, |s| p.text_width(s, 12, false));
    m.breadcrumbs.area = area;
    m.breadcrumbs.hidden = layout.hidden;
    let mut x = area.x;
    if layout.overflow_width > 0 {
        let rect = Rect::new(x, 17, layout.overflow_width, 24);
        p.rounded_rect(rect, 7 * m.radius, m.palette.lighter_background);
        p.text(rect, "…", 12, m.palette.foreground, false, true);
        m.hits.push(Hit {
            generation: m.generation,
            rect,
            action: Action::Breadcrumbs,
        });
        m.breadcrumbs.overflow = Some(rect);
        x += rect.w + layout.gap;
    }
    for (crumb, width) in layout.visible {
        let rect = Rect::new(x, 17, width, 24);
        let current = crumb.path == m.path;
        p.rounded_rect(
            rect,
            7 * m.radius,
            if current {
                m.palette.accent
            } else {
                m.palette.lighter_background
            },
        );
        p.text(
            Rect::new(x + 8, 17, (width - 16).max(0), 24),
            &crumb.label,
            12,
            if current {
                m.palette.darker_background
            } else {
                m.palette.foreground
            },
            false,
            false,
        );
        m.hits.push(Hit {
            generation: m.generation,
            rect,
            action: Action::Navigate(crumb.path.clone()),
        });
        m.breadcrumbs.visible.push((crumb, rect));
        x += width + layout.gap;
    }
    if x + 90 < area.x + area.w {
        p.text(
            Rect::new(x + 4, 11, (area.x + area.w - x - 8).max(0), 36),
            "escribe para filtrar",
            12,
            m.palette.dark_foreground,
            false,
            false,
        );
    }
}
fn caption(c: &Crumb, home: &std::path::Path) -> String {
    let Some(parent) = c.path.parent() else {
        return c.label.clone();
    };
    let context = match parent.strip_prefix(home) {
        Ok(relative) if relative.as_os_str().is_empty() => "~".into(),
        Ok(relative) => format!("~/{}", files_core::name_label::label(relative.as_os_str())),
        Err(_) => files_core::name_label::label(parent.as_os_str()).into_owned(),
    };
    format!("{} · {context}", c.label)
}
pub fn menu(m: &Model) -> (Vec<PopupItem>, Vec<Option<Action>>) {
    let mut actions = Vec::new();
    let mut items = Vec::new();
    let home = files_core::theme::home();
    for c in &m.breadcrumbs.hidden {
        let label = caption(c, &home);
        let mut row = PopupItem::new("", &label)
            .id(&actions.len().to_string())
            .checked(c.path == m.path);
        if c.path == m.path {
            row = row.disabled();
        }
        items.push(row);
        actions.push(Some(Action::Navigate(c.path.clone())));
    }
    if let Some(row) = items.last_mut() {
        row.divider = true;
    }
    let mut background = Vec::new();
    for c in &m.breadcrumbs.hidden {
        background.push(
            PopupItem::new("", &files_core::name_label::label(c.path.as_os_str()))
                .id(&actions.len().to_string()),
        );
        actions.push(Some(Action::BackgroundNavigate(c.path.clone())));
    }
    items.push(PopupItem::submenu(
        "",
        "Abrir en pestaña de fondo",
        background,
    ));
    (items, actions)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ancestor_caption_starts_with_name_and_abbreviates_home_context() {
        let home = std::path::Path::new("/home/person");
        let c = crate::breadcrumbs::build(std::path::Path::new("/home/person/parent/child"), home)
            .pop()
            .unwrap();
        assert_eq!(caption(&c, home), "child · ~/parent");
        let root = crate::breadcrumbs::build(std::path::Path::new("/"), home)
            .pop()
            .unwrap();
        assert_eq!(caption(&root, home), "/");
        assert_eq!(c.path, std::path::Path::new("/home/person/parent/child"));
    }
}

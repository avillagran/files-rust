//! App-owned tab actions. Toolkit widgets request operations by stable identity.
use crate::{
    model::{Action, Model},
    tabs::Command,
};
use fltk_omarchy::popups::PopupItem;
#[derive(Clone)]
pub struct Menu {
    pub id: u64,
    pub owner: (u64, u64),
    pub generation: u64,
    pub anchor: (i32, i32),
}
fn clear_surface(m: &Model) -> bool {
    !crate::places_sidebar::covered(m)
        && !m.settings
        && !m.actions_menu
        && m.context_menu.is_none()
        && !m.editing
        && !m.filter_edit.active
        && !m.mutations.busy
}
pub fn available(m: &Model) -> bool {
    clear_surface(m)
        && m.tab_context.as_ref().is_some_and(|t| {
            t.owner == m.pane_owner()
                && t.generation == m.generation
                && m.tabs.index(t.id).is_some()
        })
}
pub fn open(m: &mut Model, id: u64, x: i32, y: i32) {
    if m.tab_context.is_some() {
        m.tab_context = None;
        return;
    }
    if clear_surface(m) && m.tabs.index(id).is_some() {
        // This handler may run before the canvas event router (or from a
        // native widget): late startup restoration must not replace user intent.
        m.session.restore_allowed = false;
        m.tab_context = Some(Menu {
            id,
            owner: m.pane_owner(),
            generation: m.generation,
            anchor: (x, y),
        });
    }
}
pub fn pointer(m: &mut Model, x: i32, y: i32) -> bool {
    let id = m.hits.iter().rev().find_map(|h| {
        if h.generation != m.generation || !h.rect.contains(x, y) {
            return None;
        }
        match h.action {
            Action::Tab(Command::Select(id) | Command::Close(id)) => Some(id),
            _ => None,
        }
    });
    if let Some(id) = id {
        open(m, id, x, y);
        true
    } else {
        false
    }
}
pub fn keyboard(m: &mut Model) {
    if let Some(r) = m.hits.iter().find_map(|h| {
        matches!(h.action,Action::Tab(Command::Select(id)) if id==m.tabs.active)
            .then_some(h.rect)
            .filter(|_| h.generation == m.generation)
    }) {
        open(m, m.tabs.active, r.x, r.y + r.h);
    }
}
pub fn move_command(m: &Model, id: u64, right: bool) -> Option<Command> {
    let at = m.tabs.index(id)?;
    if right {
        (at + 1 < m.tabs.rows.len()).then(|| Command::Move {
            id,
            before: m.tabs.rows.get(at + 2).map(|t| t.id),
        })
    } else {
        at.checked_sub(1).map(|to| Command::Move {
            id,
            before: Some(m.tabs.rows[to].id),
        })
    }
}
pub fn menu(m: &Model) -> (Vec<PopupItem>, Vec<Option<Action>>) {
    let t = m.tab_context.as_ref().expect("tab menu request");
    let at = m.tabs.index(t.id).expect("validated tab id");
    let entries = [
        ("Cerrar pestaña", Some(Command::Close(t.id))),
        (
            "Cerrar otras pestañas",
            (m.tabs.rows.len() > 1).then_some(Command::CloseOthers(t.id)),
        ),
        (
            "Cerrar pestañas a la derecha",
            (at + 1 < m.tabs.rows.len()).then_some(Command::CloseRight(t.id)),
        ),
        ("Mover a la izquierda", move_command(m, t.id, false)),
        ("Mover a la derecha", move_command(m, t.id, true)),
    ];
    entries
        .into_iter()
        .enumerate()
        .map(|(i, (label, command))| {
            let mut row = PopupItem::new("", label).id(&i.to_string());
            if command.is_none() {
                row = row.disabled();
            }
            if i == 2 {
                row.divider = true;
            }
            (row, command.map(Action::Tab))
        })
        .unzip()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tab_context_owns_user_intent_and_rejects_a_changed_browser() {
        let mut m = Model::new("/fixture-tab-context-intent".into());
        m.session.restore_allowed = true;
        let active = m.tabs.active;
        let path = m.path.clone();
        open(&mut m, active + 99, 20, 70);
        assert!(m.tab_context.is_none() && m.session.restore_allowed);
        open(&mut m, active, 20, 70);
        assert!(!m.session.restore_allowed && available(&m));
        assert_eq!(m.tabs.active, active);
        assert_eq!(m.path, path);
        m.generation = m.generation.wrapping_add(1);
        assert!(!available(&m));
    }
}

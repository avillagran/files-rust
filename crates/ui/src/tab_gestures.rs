//! Auxiliary pointer gestures never fall through to primary button actions.
use crate::{
    model::{Action, Model},
    tabs::Command,
};

pub fn blocked(m: &Model) -> bool {
    m.breadcrumbs.open
        || crate::places_sidebar::covered(m)
        || m.settings
        || m.actions_menu
        || m.context_menu.is_some()
        || m.editing
        || m.filter_edit.active
        || m.mutations.busy
}

pub fn hit(m: &Model, x: i32, y: i32) -> Option<Action> {
    m.hits
        .iter()
        .rev()
        .find(|h| h.generation == m.generation && h.rect.contains(x, y))
        .map(|h| h.action.clone())
}

/// Uses the painted target, never changes selection to discover a folder.
pub fn middle(m: &mut Model, target: Option<Action>) {
    if blocked(m) {
        return;
    }
    let Some(action) = target else { return };
    match action {
        Action::Tab(Command::Select(id) | Command::Close(id)) => {
            m.tab_command(Command::Close(id));
            m.session.restore_allowed = false;
        }
        action => match m.background_folder_tab(&action) {
            Ok(Some(_)) => {
                m.session.restore_allowed = false;
                m.tab_feedback = Some(std::time::Instant::now());
            }
            Ok(None) => {}
            Err(error) => m.operation_message = Some(error.into()),
        },
    }
}

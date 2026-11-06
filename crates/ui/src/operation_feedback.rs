//! One bounded, app-owned feedback record. Cards never retain worker results
//! or grant mutation authority; the caller supplies current admission guards.
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    sync::Arc,
};

pub const MAX_BODY_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Info,
    Mutation(u64),
    Copy(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub revision: u64,
    pub source: Source,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Details,
    Undo,
    Redo,
    Dismiss,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Control {
    pub action: Action,
    pub enabled: bool,
}

/// Visibility does not discard the record. `available` governs commands that
/// open a report or submit history work; dismissal changes presentation only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Context {
    pub visible: bool,
    pub available: bool,
    pub copy_result: Option<u64>,
    pub can_undo: bool,
    pub can_redo: bool,
}

/// Cloning a snapshot retains at most one bounded message, not a report/tree.
/// It can leave a Model borrow before toolkit callbacks are connected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub stamp: Stamp,
    pub body: Arc<str>,
    pub controls: [Option<Control>; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Fingerprint {
    bytes: usize,
    digest: u64,
}
impl Fingerprint {
    fn new(message: &str) -> Self {
        let mut hash = DefaultHasher::new();
        message.hash(&mut hash);
        Self {
            bytes: message.len(),
            digest: hash.finish(),
        }
    }
}

struct Record {
    stamp: Stamp,
    body: Arc<str>,
    history: bool,
    dismissed: bool,
}

#[derive(Default)]
pub struct State {
    revision: u64,
    observed: Option<Fingerprint>,
    record: Option<Record>,
}

fn bounded(message: &str) -> Arc<str> {
    if message.len() <= MAX_BODY_BYTES {
        return Arc::from(message);
    }
    let mut end = MAX_BODY_BYTES - '…'.len_utf8();
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    Arc::from(format!("{}…", &message[..end]))
}

impl State {
    fn publish(&mut self, source: Source, message: &str, history: bool) -> Stamp {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("operation feedback revision exhausted");
        let stamp = Stamp {
            revision: self.revision,
            source,
        };
        self.observed = Some(Fingerprint::new(message));
        self.record = Some(Record {
            stamp,
            body: bounded(message),
            history,
            dismissed: false,
        });
        stamp
    }

    /// Every typed completion is a new event, even when its text is identical.
    /// Failure may retain an older worker history; it must not advertise that
    /// history as belonging to this failed result.
    pub fn publish_mutation(&mut self, id: u64, message: &str, success: bool) -> Stamp {
        self.publish(Source::Mutation(id), message, success)
    }

    pub fn publish_copy(&mut self, id: u64, message: &str) -> Stamp {
        self.publish(Source::Copy(id), message, false)
    }

    /// Bridge legacy informational assignments without republishing an
    /// unchanged (or manually dismissed) card. Matching typed publication text
    /// keeps its provenance. Fingerprints deduplicate display text only; action
    /// admission always uses the current exact stamp/source and capabilities.
    pub fn observe_info(&mut self, message: Option<&str>) -> bool {
        let message = message.filter(|text| !text.trim().is_empty());
        let observed = message.map(Fingerprint::new);
        if self.observed == observed {
            return false;
        }
        if let Some(message) = message {
            self.publish(Source::Info, message, false);
        } else {
            self.observed = None;
            self.record = None;
        }
        true
    }

    pub fn stamp(&self) -> Option<Stamp> {
        self.record.as_ref().map(|record| record.stamp)
    }

    fn controls(record: &Record, context: Context) -> [Option<Control>; 2] {
        let primary = match record.stamp.source {
            Source::Copy(id) => Some(Control {
                action: Action::Details,
                enabled: context.available && context.copy_result == Some(id),
            }),
            Source::Mutation(_) if record.history && context.can_redo => Some(Control {
                action: Action::Redo,
                enabled: context.available,
            }),
            Source::Mutation(_) if record.history && context.can_undo => Some(Control {
                action: Action::Undo,
                enabled: context.available,
            }),
            _ => None,
        };
        [primary, None]
    }

    pub fn snapshot(&self, context: Context) -> Option<Snapshot> {
        let record = self.record.as_ref()?;
        if !context.visible || record.dismissed {
            return None;
        }
        Some(Snapshot {
            stamp: record.stamp,
            body: Arc::clone(&record.body),
            controls: Self::controls(record, context),
        })
    }

    pub fn permits(&self, stamp: Stamp, action: Action, context: Context) -> bool {
        let Some(record) = self.record.as_ref() else {
            return false;
        };
        record.stamp == stamp
            && !record.dismissed
            && context.visible
            && (action == Action::Dismiss
                || Self::controls(record, context)
                    .iter()
                    .flatten()
                    .any(|control| control.action == action && control.enabled))
    }

    /// Does not modify legacy messages, retained reports, modal review, queues
    /// or worker state. Covered/temporarily closed widgets must not call this.
    pub fn dismiss(&mut self, stamp: Stamp) -> bool {
        let Some(record) = self.record.as_mut() else {
            return false;
        };
        if record.stamp != stamp || record.dismissed {
            return false;
        }
        record.dismissed = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready() -> Context {
        Context {
            visible: true,
            available: true,
            ..Context::default()
        }
    }

    #[test]
    fn stale_callback_cannot_target_a_replacement_with_identical_text() {
        let mut state = State::default();
        let old = state.publish_copy(41, "Copia terminada");
        let new = state.publish_copy(42, "Copia terminada");
        let context = Context {
            copy_result: Some(42),
            ..ready()
        };
        assert_ne!(old.revision, new.revision);
        assert!(!state.permits(old, Action::Details, context));
        assert!(!state.dismiss(old));
        assert!(state.permits(new, Action::Details, context));
    }

    #[test]
    fn unrelated_information_never_inherits_history_or_retained_copy_actions() {
        let mut state = State::default();
        let context = Context {
            can_undo: true,
            copy_result: Some(12),
            ..ready()
        };
        let mutation = state.publish_mutation(7, "Nombres actualizados", true);
        assert!(state.permits(mutation, Action::Undo, context));
        assert!(state.observe_info(Some("Selecciona un archivo")));
        let info = state.stamp().unwrap();
        assert_eq!(info.source, Source::Info);
        assert!(!state.permits(info, Action::Undo, context));
        assert!(!state.permits(info, Action::Details, context));
        state.publish_copy(12, "Copia terminada");
        assert!(state.observe_info(Some("Otro resultado")));
        assert!(!state.permits(state.stamp().unwrap(), Action::Details, context));
    }

    #[test]
    fn dismiss_stays_dismissed_until_a_new_publication_or_changed_information() {
        let mut state = State::default();
        let stamp = state.publish_mutation(7, "Nombres actualizados", true);
        assert!(!state.observe_info(Some("Nombres actualizados")));
        assert!(state.dismiss(stamp));
        assert!(!state.observe_info(Some("Nombres actualizados")));
        assert!(state.snapshot(ready()).is_none());
        let new = state.publish_mutation(8, "Nombres actualizados", true);
        assert_ne!(stamp, new);
        assert!(state.snapshot(ready()).is_some());
        assert!(state.dismiss(new));
        assert!(state.observe_info(Some("Otro mensaje")));
        assert!(state.snapshot(ready()).is_some());
    }

    #[test]
    fn failed_mutation_does_not_offer_history_retained_from_an_earlier_success() {
        let mut state = State::default();
        let failed = state.publish_mutation(8, "No se pudo completar", false);
        for context in [
            Context {
                can_undo: true,
                ..ready()
            },
            Context {
                can_redo: true,
                ..ready()
            },
        ] {
            assert!(!state.permits(failed, Action::Undo, context));
            assert!(!state.permits(failed, Action::Redo, context));
        }
    }

    #[test]
    fn covered_or_busy_surfaces_cannot_run_report_or_history_commands() {
        let mut state = State::default();
        let copy = state.publish_copy(9, "Copia terminada");
        let context = Context {
            copy_result: Some(9),
            ..ready()
        };
        assert!(state.permits(copy, Action::Details, context));
        assert!(!state.permits(
            copy,
            Action::Details,
            Context {
                available: false,
                ..context
            }
        ));
        assert!(!state.permits(
            copy,
            Action::Dismiss,
            Context {
                visible: false,
                ..context
            }
        ));
        assert!(
            state
                .snapshot(Context {
                    visible: false,
                    ..context
                })
                .is_none()
        );
        assert_eq!(state.snapshot(context).unwrap().stamp, copy);
        assert!(state.permits(copy, Action::Details, context));
    }

    #[test]
    fn details_revalidate_the_single_retained_result_and_history_uses_current_flags() {
        let mut state = State::default();
        let copy = state.publish_copy(9, "Copia terminada");
        for retained in [None, Some(8), Some(10)] {
            assert!(!state.permits(
                copy,
                Action::Details,
                Context {
                    copy_result: retained,
                    ..ready()
                }
            ));
        }
        let mutation = state.publish_mutation(51, "Nombres actualizados", true);
        assert!(state.permits(
            mutation,
            Action::Undo,
            Context {
                can_undo: true,
                ..ready()
            }
        ));
        assert!(!state.permits(mutation, Action::Undo, ready()));
        assert!(state.permits(
            mutation,
            Action::Redo,
            Context {
                can_redo: true,
                ..ready()
            }
        ));
    }

    #[test]
    fn text_is_utf8_bounded_and_hidden_tail_changes_demote_typed_provenance() {
        let mut state = State::default();
        let mut original = "界".repeat(600);
        original.push('a');
        let stamp = state.publish_mutation(4, &original, true);
        let snapshot = state.snapshot(ready()).unwrap();
        assert!(snapshot.body.len() <= MAX_BODY_BYTES);
        assert!(snapshot.body.ends_with('…'));
        assert!(!state.observe_info(Some(&original)));
        original.pop();
        original.push('b');
        assert!(state.observe_info(Some(&original)));
        let updated = state.snapshot(ready()).unwrap();
        assert_eq!(snapshot.body, updated.body);
        assert_ne!(updated.stamp, stamp);
        assert_eq!(updated.stamp.source, Source::Info);
    }

    #[test]
    fn clearing_invalidates_callbacks_and_a_later_identical_message_is_new() {
        let mut state = State::default();
        let old = state.publish_copy(9, "Copia terminada");
        assert!(state.observe_info(None));
        assert!(state.snapshot(ready()).is_none());
        assert!(!state.dismiss(old));
        assert!(state.observe_info(Some("Copia terminada")));
        assert_ne!(state.stamp().unwrap(), old);
        assert!(state.observe_info(Some(" \n\t")));
        assert!(state.snapshot(ready()).is_none());
    }
}

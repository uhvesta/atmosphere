//! What a session's row shows, kept up to date as events come, so listing
//! sessions doesn't read every session's whole history.

use humata_workspace::Snapshot;
use serde::{Deserialize, Serialize};

use crate::event::{Event, EventKind, PermissionKind, SessionState};

/// How much of the last message a row shows.
const MAX_CHARS: usize = 300;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    /// Turns that ended.
    pub turns: u32,
    /// The running turn's requests for permission that aren't answered yet.
    pub pending: Vec<Pending>,
    /// Milliseconds since 1970 of the last event; 0 with none.
    pub last_active_unix_ms: i64,
    /// The latest failure or reply, cut short.
    pub last_message: String,
    /// Checkpoints taken at turns' ends, and saves part way through turns.
    pub checkpoints: usize,
    pub saves: usize,
    /// The code as last checkpointed or saved; empty before it first is.
    pub code: Vec<Snapshot>,
    /// The state as of the last turn's start or end.
    settled: SessionState,
    failed: bool,
    /// Requests less answers in the running turn, as [`SessionState::after`] counts them.
    open: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pending {
    pub request_id: String,
    pub kind: PermissionKind,
    pub summary: String,
}

impl Default for Summary {
    fn default() -> Self {
        Self {
            turns: 0,
            pending: Vec::new(),
            last_active_unix_ms: 0,
            last_message: String::new(),
            checkpoints: 0,
            saves: 0,
            code: Vec::new(),
            settled: SessionState::Idle,
            failed: false,
            open: 0,
        }
    }
}

impl Summary {
    pub fn of(events: &[Event]) -> Self {
        let mut summary = Self::default();
        for event in events {
            summary.add(event);
        }
        summary
    }

    pub fn add(&mut self, event: &Event) {
        self.last_active_unix_ms = event.time_unix_ms;
        match &event.kind {
            EventKind::TurnStarted => {
                self.settled = SessionState::Working;
                self.failed = false;
                self.open = 0;
                self.pending.clear();
            }
            EventKind::PermissionRequested {
                request_id,
                kind,
                summary,
            } => {
                self.open += 1;
                self.pending.push(Pending {
                    request_id: request_id.clone(),
                    kind: *kind,
                    summary: summary.clone(),
                });
            }
            EventKind::PermissionCompleted { request_id, .. } => {
                self.open = self.open.saturating_sub(1);
                self.pending
                    .retain(|request| &request.request_id != request_id);
            }
            EventKind::Failed { message } => {
                self.failed = true;
                self.last_message = cut(message);
            }
            EventKind::AssistantMessage { text } => self.last_message = cut(text),
            EventKind::Checkpoint { snapshots } => {
                self.checkpoints += 1;
                self.code.clone_from(snapshots);
            }
            EventKind::Saved { snapshots } => {
                self.saves += 1;
                self.code.clone_from(snapshots);
            }
            EventKind::TurnEnded { .. } => {
                self.settled = if self.failed {
                    SessionState::Failed
                } else {
                    SessionState::Idle
                };
                self.open = 0;
                self.pending.clear();
                self.turns = self.turns.saturating_add(1);
            }
            _ => {}
        }
    }

    /// The same as [`SessionState::after`] the events added.
    pub fn state(&self) -> SessionState {
        if self.settled == SessionState::Working && self.open > 0 {
            SessionState::WaitingForApproval
        } else {
            self.settled
        }
    }
}

fn cut(text: &str) -> String {
    let text = text.trim();
    match text.char_indices().nth(MAX_CHARS) {
        None => text.to_owned(),
        Some((end, _)) => format!("{}…", text[..end].trim_end()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: EventKind) -> Event {
        Event::now(kind)
    }

    fn asked(id: &str) -> Event {
        event(EventKind::PermissionRequested {
            request_id: id.to_owned(),
            kind: PermissionKind::Shell,
            summary: format!("run {id}"),
        })
    }

    fn answered(id: &str) -> Event {
        event(EventKind::PermissionCompleted {
            request_id: id.to_owned(),
            approved: true,
            automatic: false,
            judged: false,
        })
    }

    fn ids(events: &[Event]) -> Vec<String> {
        Summary::of(events)
            .pending
            .into_iter()
            .map(|request| request.request_id)
            .collect()
    }

    #[test]
    fn only_the_running_turns_unanswered_requests_are_pending() {
        let first = [
            event(EventKind::TurnStarted),
            asked("a"),
            asked("b"),
            answered("a"),
        ];
        assert_eq!(ids(&first), ["b"]);
        let mut ended = first.to_vec();
        ended.push(event(EventKind::TurnEnded { aborted: true }));
        assert!(ids(&ended).is_empty());
        ended.extend([event(EventKind::TurnStarted), asked("c")]);
        assert_eq!(ids(&ended), ["c"]);
    }

    #[test]
    fn adding_events_one_by_one_matches_reading_them_all() {
        let events = [
            event(EventKind::UserMessage {
                text: "go".to_owned(),
                images: Vec::new(),
            }),
            event(EventKind::TurnStarted),
            asked("a"),
            answered("a"),
            event(EventKind::AssistantMessage {
                text: "  done  ".to_owned(),
            }),
            event(EventKind::TurnEnded { aborted: false }),
            event(EventKind::TurnStarted),
            asked("b"),
            event(EventKind::Failed {
                message: "x".repeat(MAX_CHARS + 10),
            }),
            event(EventKind::TurnEnded { aborted: false }),
            event(EventKind::Failed {
                message: "after".to_owned(),
            }),
            event(EventKind::TurnStarted),
            asked("c"),
        ];
        let mut summary = Summary::default();
        for (count, event) in events.iter().enumerate() {
            summary.add(event);
            let prefix = &events[..=count];
            assert_eq!(summary, Summary::of(prefix));
            assert_eq!(summary.state(), SessionState::after(prefix));
            assert_eq!(summary.last_active_unix_ms, event.time_unix_ms);
        }
        assert_eq!(summary.turns, 2);
        assert_eq!(summary.last_message, "after");
    }

    #[test]
    fn a_long_message_is_cut_short() {
        let summary = Summary::of(&[event(EventKind::AssistantMessage {
            text: "y".repeat(MAX_CHARS * 2),
        })]);
        assert!(summary.last_message.chars().count() <= MAX_CHARS + 1);
        assert!(summary.last_message.starts_with('y'));
    }
}

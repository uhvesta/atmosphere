//! What ties one of our sessions to Copilot's: its Copilot session, and which
//! of our events are which of Copilot's, so a fork before one of our events
//! forks Copilot's session at the same place.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AgentError;
use crate::event::{Event, EventId};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Links {
    /// None until the session's first turn.
    pub(super) copilot: Option<String>,
    /// For a fork that hasn't run a turn: the Copilot session to fork, made
    /// at its first turn.
    pub(super) fork: Option<PendingFork>,
    /// Our event and Copilot's, in the order they happened.
    pub(super) events: Vec<(EventId, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct PendingFork {
    pub(super) source: String,
}

impl Links {
    pub(super) fn path(root: &Path, session: &str) -> PathBuf {
        root.join("links").join(format!("{session}.json"))
    }

    /// The links in `path`; none if there is no file.
    pub(super) fn load(path: &Path) -> Result<Self, AgentError> {
        match std::fs::read(path) {
            Ok(json) => {
                serde_json::from_slice(&json).map_err(|source| AgentError::CorruptBackendState {
                    path: path.to_owned(),
                    source,
                })
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(AgentError::Read {
                path: path.to_owned(),
                source,
            }),
        }
    }

    /// Writes the links to `path`, all or nothing.
    pub(super) fn save(&self, path: &Path) -> Result<(), AgentError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|source| AgentError::CreateDirectory {
                path: dir.to_owned(),
                source,
            })?;
        }
        let json = serde_json::to_vec(self).map_err(|source| AgentError::Encode { source })?;
        let partial = path.with_extension("json.partial");
        std::fs::write(&partial, json).map_err(|source| AgentError::Write {
            path: partial.clone(),
            source,
        })?;
        std::fs::rename(&partial, path).map_err(|source| AgentError::Write {
            path: path.to_owned(),
            source,
        })
    }

    pub(super) fn is_linked(&self, event: &EventId) -> bool {
        self.events.iter().any(|(ours, _)| ours == event)
    }

    /// The links of a fork of these that keeps `history`. A fork keeps the
    /// Copilot events of the events it keeps; with none, it starts afresh.
    pub(super) fn fork(&self, history: &[Event]) -> Self {
        let kept: HashSet<&EventId> = history.iter().map(|event| &event.id).collect();
        let events: Vec<_> = self
            .events
            .iter()
            .filter(|(ours, _)| kept.contains(ours))
            .cloned()
            .collect();
        let source = self
            .copilot
            .clone()
            .or_else(|| self.fork.as_ref().map(|fork| fork.source.clone()));
        match source {
            Some(source) if !events.is_empty() => Self {
                copilot: None,
                fork: Some(PendingFork { source }),
                events,
            },
            _ => Self::default(),
        }
    }

    /// Where to cut `source`'s events, by their ids in order, to keep only
    /// what these links keep: before the first event after the last one kept
    /// that `source` has.
    pub(super) fn cut(&self, source: &[String]) -> Cut {
        let last = self
            .events
            .iter()
            .rev()
            .find_map(|(_, theirs)| source.iter().position(|id| id == theirs));
        match last {
            None => Cut::Nothing,
            Some(last) => source
                .get(last + 1)
                .map_or(Cut::All, |next| Cut::Before(next.clone())),
        }
    }
}

/// What of a Copilot session a fork keeps.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Cut {
    All,
    Before(String),
    Nothing,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::EventKind;

    fn event(id: &str) -> Event {
        Event {
            id: EventId::new(id),
            time_unix_ms: 0,
            kind: EventKind::Idle,
        }
    }

    fn links(copilot: &str, pairs: &[(&str, &str)]) -> Links {
        Links {
            copilot: Some(copilot.into()),
            fork: None,
            events: pairs
                .iter()
                .map(|(ours, theirs)| (EventId::new(*ours), (*theirs).to_owned()))
                .collect(),
        }
    }

    #[test]
    fn a_fork_keeps_the_links_of_the_events_it_keeps() {
        let parent = links("c", &[("a", "1"), ("b", "2"), ("c", "3")]);
        let fork = parent.fork(&[event("a"), event("x"), event("b")]);
        assert_eq!(fork.copilot, None);
        assert_eq!(fork.fork, Some(PendingFork { source: "c".into() }));
        assert_eq!(
            fork.events,
            vec![
                (EventId::new("a"), "1".to_owned()),
                (EventId::new("b"), "2".to_owned())
            ]
        );
    }

    #[test]
    fn a_fork_that_keeps_no_linked_event_starts_afresh() {
        let parent = links("c", &[("a", "1")]);
        assert_eq!(parent.fork(&[event("x")]), Links::default());
        assert_eq!(Links::default().fork(&[event("a")]), Links::default());
    }

    #[test]
    fn a_fork_of_a_fork_forks_the_same_copilot_session() {
        let parent = links("c", &[("a", "1"), ("b", "2")]);
        let child = parent.fork(&[event("a"), event("b")]);
        let grandchild = child.fork(&[event("a")]);
        assert_eq!(grandchild.fork, Some(PendingFork { source: "c".into() }));
        assert_eq!(grandchild.events.len(), 1);
    }

    #[test]
    fn the_cut_is_the_event_after_the_last_one_kept() {
        let source: Vec<String> = ["0", "1", "x", "2", "3"].map(String::from).into();
        let kept = links("c", &[("a", "1"), ("b", "2")]);
        assert_eq!(kept.cut(&source), Cut::Before("3".to_owned()));
        let all = links("c", &[("a", "1"), ("b", "3")]);
        assert_eq!(all.cut(&source), Cut::All);
        // A kept event Copilot no longer has is skipped.
        let gone = links("c", &[("a", "1"), ("b", "9")]);
        assert_eq!(gone.cut(&source), Cut::Before("x".to_owned()));
        let none = links("c", &[("a", "9")]);
        assert_eq!(none.cut(&source), Cut::Nothing);
    }
}

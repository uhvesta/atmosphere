//! What ties one of our sessions to a backend's that keeps its own, such as
//! Copilot's or Claude Code's: the backend's session, and which of our events
//! are which of its, so a fork before one of our events forks the backend's
//! session at the same place.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AgentError;
use crate::event::{Event, EventId, SessionId};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Links {
    /// The backend's session; None until the session's first turn.
    #[serde(alias = "copilot")]
    pub(crate) session: Option<String>,
    /// For a fork that hasn't run a turn: the backend's session to fork,
    /// made at its first turn.
    pub(crate) fork: Option<PendingFork>,
    /// Our event and the backend's, in the order they happened.
    pub(crate) events: Vec<(EventId, String)>,
    /// The backend's session this was imported from, started outside
    /// Humata, so it is never deleted with ours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) imported: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PendingFork {
    pub(crate) source: String,
}

/// Said when a session with earlier turns has no backend session to carry
/// on, so its agent starts with none of what came before.
/// Under the links folder: the links each session had before each of its
/// rewinds.
const REWOUND_FOLDER: &str = "rewound";

pub(crate) const STARTED_OVER: &str = "The agent started a new conversation: its earlier one in this session couldn't be found, so it doesn't know what was said before. Tell it what it needs from before to carry on.";

/// Whether `history` holds a finished turn before the one starting.
pub(crate) fn had_turns(history: &[Event]) -> bool {
    history
        .iter()
        .any(|event| matches!(event.kind, crate::event::EventKind::TurnEnded { .. }))
}

impl Links {
    pub(crate) fn path(root: &Path, session: &str) -> PathBuf {
        root.join("links").join(format!("{session}.json"))
    }

    /// Which of our sessions under `root` carries on each backend session
    /// linked there.
    pub(crate) fn linked(root: &Path) -> Result<Vec<(String, SessionId)>, AgentError> {
        let dir = root.join("links");
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(AgentError::ReadDirectory { path: dir, source }),
        };
        let mut linked = Vec::new();
        for path in entries.filter_map(Result::ok).map(|entry| entry.path()) {
            let Some(ours) = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".json"))
            else {
                continue;
            };
            if let Some(theirs) = Self::load(&path)?.session {
                linked.push((theirs, SessionId::new(ours)));
            }
        }
        Ok(linked)
    }

    /// The links in `path`; none if there is no file. Pairs linked since it
    /// was last written whole follow its own.
    pub(crate) fn load(path: &Path) -> Result<Self, AgentError> {
        let mut links: Self = match std::fs::read(path) {
            Ok(json) => {
                serde_json::from_slice(&json).map_err(|source| AgentError::CorruptBackendState {
                    path: path.to_owned(),
                    source,
                })?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(source) => {
                return Err(AgentError::Read {
                    path: path.to_owned(),
                    source,
                });
            }
        };
        let appended = pairs_path(path);
        let bytes = match std::fs::read(&appended) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(links),
            Err(source) => {
                return Err(AgentError::Read {
                    path: appended,
                    source,
                });
            }
        };
        // A crash after writing them whole but before removing the appended
        // pairs leaves some in both: each is read once.
        let mut known: HashSet<EventId> =
            links.events.iter().map(|(ours, _)| ours.clone()).collect();
        let mut lines = bytes.split(|byte| *byte == b'\n').peekable();
        while let Some(line) = lines.next() {
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            match serde_json::from_slice::<(EventId, String)>(line) {
                Ok(pair) => {
                    if known.insert(pair.0.clone()) {
                        links.events.push(pair);
                    }
                }
                // The last line, cut short by a crash mid-write.
                Err(_) if lines.peek().is_none_or(|rest| rest.is_empty()) => break,
                Err(source) => {
                    return Err(AgentError::CorruptBackendState {
                        path: appended,
                        source,
                    });
                }
            }
        }
        Ok(links)
    }

    /// Links our event `ours` to the backend's `theirs`, appending the pair
    /// to the file instead of writing every link again, which a long
    /// session would make slow.
    pub(crate) fn link(
        &mut self,
        path: &Path,
        ours: EventId,
        theirs: String,
    ) -> Result<(), AgentError> {
        use std::io::Write as _;
        let mut line = serde_json::to_vec(&(&ours, &theirs))
            .map_err(|source| AgentError::Encode { source })?;
        line.push(b'\n');
        self.events.push((ours, theirs));
        if !path.exists() {
            return self.save(path);
        }
        let appended = pairs_path(path);
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&appended)
            .and_then(|mut file| file.write_all(&line))
            .map_err(|source| AgentError::Write {
                path: appended,
                source,
            })
    }

    /// Writes the links to `path`, all or nothing, the pairs appended since
    /// it was last written included.
    pub(crate) fn save(&self, path: &Path) -> Result<(), AgentError> {
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
        })?;
        remove_appended(path)
    }

    /// Where the links a session had before its rewind `rewind` are kept,
    /// for undoing it.
    fn rewound_path(root: &Path, session: &str, rewind: usize) -> PathBuf {
        root.join("links")
            .join(REWOUND_FOLDER)
            .join(session)
            .join(format!("{rewind}.json"))
    }

    /// Saves the session's links for undoing rewind `rewind`, then links
    /// it as a fork that keeps `history`: its next turn forks the backend's
    /// session where the rewind cut ours.
    pub(crate) fn rewind(
        root: &Path,
        session: &str,
        history: &[Event],
        rewind: usize,
    ) -> Result<(), AgentError> {
        let path = Self::path(root, session);
        let links = Self::load(&path)?;
        links.save(&Self::rewound_path(root, session, rewind))?;
        links.fork(history).save_or_remove(&path)
    }

    /// Puts back the links the session had before rewind `rewind`.
    pub(crate) fn undo_rewind(root: &Path, session: &str, rewind: usize) -> Result<(), AgentError> {
        let saved = Self::load(&Self::rewound_path(root, session, rewind))?;
        saved.save_or_remove(&Self::path(root, session))
    }

    /// The backend sessions kept for undoing a rewind of `session`, and the
    /// folder they're kept in.
    pub(crate) fn rewound(
        root: &Path,
        session: &str,
    ) -> Result<(Vec<String>, PathBuf), AgentError> {
        let dir = root.join("links").join(REWOUND_FOLDER).join(session);
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok((Vec::new(), dir));
            }
            Err(source) => return Err(AgentError::ReadDirectory { path: dir, source }),
        };
        let mut sessions = Vec::new();
        for path in entries.filter_map(Result::ok).map(|entry| entry.path()) {
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                sessions.extend(Self::load(&path)?.session);
            }
        }
        Ok((sessions, dir))
    }

    /// Writes the links to `path`, or removes the file when there are none.
    fn save_or_remove(&self, path: &Path) -> Result<(), AgentError> {
        if *self != Self::default() {
            return self.save(path);
        }
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(AgentError::Write {
                    path: path.to_owned(),
                    source,
                });
            }
        }
        remove_appended(path)
    }

    /// The backend's event of the last event linked: where a fork that
    /// resumes up to an event resumes to.
    pub(crate) fn last(&self) -> Option<&str> {
        self.events.last().map(|(_, theirs)| theirs.as_str())
    }

    pub(crate) fn is_linked(&self, event: &EventId) -> bool {
        self.events.iter().any(|(ours, _)| ours == event)
    }

    /// The links of a fork of these that keeps `history`. A fork keeps the
    /// backend's events of the events it keeps; with none, it starts afresh.
    pub(crate) fn fork(&self, history: &[Event]) -> Self {
        let kept: HashSet<&EventId> = history.iter().map(|event| &event.id).collect();
        let events: Vec<_> = self
            .events
            .iter()
            .filter(|(ours, _)| kept.contains(ours))
            .cloned()
            .collect();
        let source = self
            .session
            .clone()
            .or_else(|| self.fork.as_ref().map(|fork| fork.source.clone()));
        match source {
            Some(source) if !events.is_empty() => Self {
                session: None,
                fork: Some(PendingFork { source }),
                events,
                imported: self.imported.clone(),
            },
            _ => Self::default(),
        }
    }

    /// Where to cut `source`'s events, by their ids in order, to keep only
    /// what these links keep: before the first event after the last one kept
    /// that `source` has.
    pub(crate) fn cut(&self, source: &[String]) -> Cut {
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

/// Where the pairs linked since the links at `path` were last written whole
/// are appended, one JSON line each.
pub(crate) fn pairs_path(path: &Path) -> PathBuf {
    path.with_extension("jsonl")
}

/// Removes the pairs appended to the links at `path`, once they are in it.
pub(crate) fn remove_appended(path: &Path) -> Result<(), AgentError> {
    let appended = pairs_path(path);
    match std::fs::remove_file(&appended) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(AgentError::Write {
            path: appended,
            source,
        }),
    }
}

/// What of a backend's session a fork keeps, for a backend that forks
/// before an event.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Cut {
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

    fn links(session: &str, pairs: &[(&str, &str)]) -> Links {
        Links {
            session: Some(session.into()),
            fork: None,
            imported: None,
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
        assert_eq!(fork.session, None);
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
    fn a_fork_of_a_fork_forks_the_same_backend_session() {
        let parent = links("c", &[("a", "1"), ("b", "2")]);
        let child = parent.fork(&[event("a"), event("b")]);
        let grandchild = child.fork(&[event("a")]);
        assert_eq!(grandchild.fork, Some(PendingFork { source: "c".into() }));
        assert_eq!(grandchild.events.len(), 1);
    }

    #[test]
    fn linked_pairs_are_appended_and_read_back_in_order() {
        let dir = tempfile::tempdir().expect("a folder");
        let path = Links::path(dir.path(), "s");
        let mut written = links("c", &[("a", "1")]);
        written.save(&path).expect("saved");
        written
            .link(&path, EventId::new("b"), "2".into())
            .expect("linked");
        written
            .link(&path, EventId::new("c"), "3".into())
            .expect("linked");
        let read = Links::load(&path).expect("read");
        assert_eq!(read, written);
        assert_eq!(read.last(), Some("3"));

        // Written whole again, the appended pairs are in it once.
        written.save(&path).expect("saved");
        assert_eq!(Links::load(&path).expect("read"), written);
    }

    #[test]
    fn a_pair_cut_short_or_already_written_whole_is_read_once() {
        let dir = tempfile::tempdir().expect("a folder");
        let path = Links::path(dir.path(), "s");
        links("c", &[("a", "1"), ("b", "2")])
            .save(&path)
            .expect("saved");
        // A crash between writing them whole and removing the appended ones,
        // and another mid-append.
        std::fs::write(
            pairs_path(&path),
            "[\"b\",\"2\"]\n[\"c\",\"3\"]\n[\"d\",\"4",
        )
        .expect("written");
        let read = Links::load(&path).expect("read");
        assert_eq!(read, links("c", &[("a", "1"), ("b", "2"), ("c", "3")]));
    }

    #[test]
    fn the_cut_is_the_event_after_the_last_one_kept() {
        let source: Vec<String> = ["0", "1", "x", "2", "3"].map(String::from).into();
        let kept = links("c", &[("a", "1"), ("b", "2")]);
        assert_eq!(kept.cut(&source), Cut::Before("3".to_owned()));
        let all = links("c", &[("a", "1"), ("b", "3")]);
        assert_eq!(all.cut(&source), Cut::All);
        // A kept event the backend no longer has is skipped.
        let gone = links("c", &[("a", "1"), ("b", "9")]);
        assert_eq!(gone.cut(&source), Cut::Before("x".to_owned()));
        let none = links("c", &[("a", "9")]);
        assert_eq!(none.cut(&source), Cut::Nothing);
    }
}

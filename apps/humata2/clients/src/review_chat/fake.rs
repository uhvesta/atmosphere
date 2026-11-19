//! A scripted [`ReviewChatClient`]: it keeps a review's view, switches, notes
//! and change feed the way the daemon does (PR-22 to PR-26), and lets a test
//! play the agent (leaving notes, moving the view, the code moving lines) with
//! [`FakeReviewChat::add_note`] and [`FakeReviewChat::diff_changed`]. No timers
//! and no real IO.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::mpsc;

use super::{
    AgentSwitches, ChangeEvents, FocusRequest, Highlight, Note, Place, ReviewChange,
    ReviewChangeKind, ReviewChatClient, ReviewContext, ReviewView,
};
use crate::daemon::{DaemonError, Reply};
use crate::model::SessionId;

/// A call a test can read back, one per trait method.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    /// `context`.
    Context(SessionId),
    /// `report_view`.
    ReportView(SessionId),
    /// `focus`, with the file as `repo/path`.
    Focus(SessionId, String),
    /// `undo`, with the undo id.
    Undo(SessionId, String),
    /// `watch_changes`, with the sequence number it resumes after.
    Watch(SessionId, u64),
    /// `set_switches`.
    SetSwitches(SessionId, AgentSwitches),
}

#[derive(Default)]
struct Session {
    view: ReviewView,
    switches: AgentSwitches,
    notes: Vec<Note>,
    changes: Vec<ReviewChange>,
    watchers: Vec<mpsc::UnboundedSender<ReviewChange>>,
    last_seq: u64,
}

impl Session {
    /// Appends a change, numbers it, and pushes it to every watcher.
    fn record(
        &mut self,
        tool: &str,
        summary: String,
        change: Option<ReviewChangeKind>,
        highlights: Vec<Highlight>,
        undoable: bool,
        undid: String,
    ) -> ReviewChange {
        self.last_seq += 1;
        let seq = self.last_seq;
        let undo_id = if undoable { format!("u-{seq}") } else { String::new() };
        let recorded = ReviewChange {
            undo_id,
            seq,
            tool: tool.to_owned(),
            summary,
            change,
            highlights,
            undid,
            at_unix_ms: 0,
        };
        self.changes.push(recorded.clone());
        self.watchers.retain(|watcher| watcher.send(recorded.clone()).is_ok());
        recorded
    }
}

#[derive(Default)]
struct Inner {
    sessions: HashMap<SessionId, Session>,
    failures: HashMap<&'static str, DaemonError>,
    calls: Vec<Call>,
}

impl Inner {
    fn session_mut(&mut self, session: &SessionId) -> &mut Session {
        self.sessions.entry(session.clone()).or_default()
    }
}

fn refused(what: &str, reason: &str) -> DaemonError {
    DaemonError::Refused {
        what: what.to_owned(),
        reason: reason.to_owned(),
    }
}

/// The last name of a path, as the chat line shows it.
fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// A review chat client that does what a test scripts, and that tests can
/// play the agent against.
#[derive(Clone, Default)]
pub struct FakeReviewChat {
    shared: Arc<Mutex<Inner>>,
}

impl FakeReviewChat {
    /// A fake with no notes or changes, every switch on.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn call<T: Send + 'static>(
        &self,
        method: &'static str,
        call: Call,
        run: impl FnOnce(&mut Inner) -> Result<T, DaemonError>,
    ) -> Reply<T> {
        let mut inner = self.inner();
        inner.calls.push(call);
        let result = match inner.failures.get(method) {
            Some(error) => Err(error.clone()),
            None => run(&mut inner),
        };
        Box::pin(std::future::ready(result))
    }

    /// The agent leaves a note: it is kept with the notes, and the feed hears
    /// it as a `review_note` change with an undo.
    pub fn add_note(&self, session: &SessionId, note: Note) -> ReviewChange {
        let mut inner = self.inner();
        let state = inner.session_mut(session);
        state.notes.push(note.clone());
        let summary = format!("{}:{}", file_name(&note.anchor.path), note.anchor.first);
        state.record("review_note", summary, Some(ReviewChangeKind::Note(note)), Vec::new(), true, String::new())
    }

    /// The agent moves the view to a stack layer (D800), `None` for Whole
    /// stack: the view keeps it and the feed hears a `review_layer` change.
    pub fn move_to_layer(&self, session: &SessionId, layer: Option<String>) -> ReviewChange {
        let mut inner = self.inner();
        let state = inner.session_mut(session);
        state.view.layer = layer.clone();
        let summary = layer.clone().unwrap_or_else(|| "Whole stack".to_owned());
        state.record("review_layer", summary, Some(ReviewChangeKind::Layer(layer)), Vec::new(), true, String::new())
    }

    /// The code moved the lines a diff is about: the feed hears a `diff` change,
    /// which has no undo (D88).
    pub fn diff_changed(&self, session: &SessionId, summary: &str) -> ReviewChange {
        let mut inner = self.inner();
        inner.session_mut(session).record(
            "diff",
            summary.to_owned(),
            None,
            Vec::new(),
            false,
            String::new(),
        )
    }

    /// Makes calls of `method` (the trait's method name, such as `"focus"`)
    /// fail with `error` until cleared.
    pub fn fail(&self, method: &'static str, error: DaemonError) {
        self.inner().failures.insert(method, error);
    }

    /// Stops failing calls of `method`.
    pub fn clear_failure(&self, method: &'static str) {
        self.inner().failures.remove(method);
    }

    /// The calls made so far, oldest first.
    #[must_use]
    pub fn calls(&self) -> Vec<Call> {
        self.inner().calls.clone()
    }
}

impl ReviewChatClient for FakeReviewChat {
    fn context(&self, session: &SessionId) -> Reply<ReviewContext> {
        self.call("context", Call::Context(session.clone()), |inner| {
            let state = inner.session_mut(session);
            Ok(ReviewContext {
                view: state.view.clone(),
                selected_text: Vec::new(),
                notes: state.notes.clone(),
                switches: state.switches,
            })
        })
    }

    fn report_view(&self, session: &SessionId, view: ReviewView) -> Reply<()> {
        self.call("report_view", Call::ReportView(session.clone()), |inner| {
            inner.session_mut(session).view = view;
            Ok(())
        })
    }

    fn focus(&self, session: &SessionId, request: FocusRequest) -> Reply<ReviewChange> {
        let call = Call::Focus(session.clone(), format!("{}/{}", request.repo, request.path));
        self.call("focus", call, move |inner| {
            let state = inner.session_mut(session);
            if !state.switches.focus {
                return Err(refused(
                    "The agent's focus",
                    "Agent may: Focus is off for this review, so the agent can't move the view. Turn it on in the Info panel to let the agent focus lines again",
                ));
            }
            let summary = match request.lines {
                Some(lines) => format!("{}:{}", file_name(&request.path), lines.first),
                None => file_name(&request.path).to_owned(),
            };
            let place = Place {
                repo: request.repo,
                path: request.path,
                visible: request.lines,
                selection: None,
                side: request.side,
            };
            Ok(state.record(
                "review_focus",
                summary,
                Some(ReviewChangeKind::Focus(place)),
                request.highlights,
                true,
                String::new(),
            ))
        })
    }

    fn undo(&self, session: &SessionId, undo_id: &str) -> Reply<ReviewChange> {
        self.call("undo", Call::Undo(session.clone(), undo_id.to_owned()), |inner| {
            let state = inner.session_mut(session);
            let target = state
                .changes
                .iter()
                .find(|change| !change.undo_id.is_empty() && change.undo_id == undo_id)
                .cloned()
                .ok_or_else(|| {
                    refused(
                        "This change",
                        "it isn't in this review's chat, so there is nothing to undo; reload the chat to see what is there now",
                    )
                })?;
            let undone = state
                .changes
                .iter()
                .any(|change| change.tool == "undo" && change.undid == target.undo_id);
            if undone {
                return Err(refused("This change", "it was undone already"));
            }
            Ok(state.record(
                "undo",
                format!("undid {}", target.summary),
                None,
                Vec::new(),
                false,
                target.undo_id,
            ))
        })
    }

    fn watch_changes(&self, session: &SessionId, after_seq: u64) -> Reply<ChangeEvents> {
        self.call("watch_changes", Call::Watch(session.clone(), after_seq), |inner| {
            let state = inner.session_mut(session);
            let (sender, receiver) = mpsc::unbounded_channel();
            for change in state.changes.iter().filter(|change| change.seq > after_seq) {
                let _ = sender.send(change.clone());
            }
            state.watchers.push(sender);
            Ok(receiver)
        })
    }

    fn set_switches(&self, session: &SessionId, switches: AgentSwitches) -> Reply<AgentSwitches> {
        let call = Call::SetSwitches(session.clone(), switches);
        self.call("set_switches", call, move |inner| {
            inner.session_mut(session).switches = switches;
            Ok(switches)
        })
    }
}

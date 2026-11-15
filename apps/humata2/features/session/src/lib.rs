//! A session's page (SE-6–23, FK-1–13): its terminal tabs, the side panel,
//! and the composer, a box for messages that wait for the agent's turn to
//! end (SE-21–23). Every session keeps its own [`SessionView`]: tab, panel,
//! draft and queue survive switching away and a restart (PER-25).
//!
//! Live terminal output arrives as [`SessionAction::Output`] from the
//! daemon's event stream and is appended to [`SessionState::live`], so a
//! line doesn't copy the workspace.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use humata2_clients::model::{ForkKind, Line, SessionId, Skill, Speaker, Turn};
use humata2_clients::{Clients, DaemonError, Workspace};
use serde::{Deserialize, Serialize};
use tca::{Effect, Reducer};

/// How many live lines a session keeps; older ones are in the daemon's
/// scrollback.
pub const LIVE_LINES: usize = 2_000;

/// The side panel beside the terminal (SE-10).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Panel {
    /// The session's changes, to review.
    Changes,
    /// Its Inbox requests.
    Inbox,
    /// Its files.
    Files,
    /// Its board items.
    Board,
    /// Events and prepared views for the session.
    Journal,
    /// What it is and where it runs.
    Info,
}

impl Panel {
    /// Every panel, in tab order.
    pub const ALL: [Self; 6] = [
        Self::Changes,
        Self::Inbox,
        Self::Files,
        Self::Board,
        Self::Journal,
        Self::Info,
    ];

    /// Its tab's title.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Changes => "Changes",
            Self::Inbox => "Inbox",
            Self::Files => "Files",
            Self::Board => "Board",
            Self::Journal => "Journal",
            Self::Info => "Info",
        }
    }
}

/// One session's page as the person left it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionView {
    /// The terminal tab: 0 is the agent, then its shells.
    pub tab: usize,
    /// The side panel, if open.
    pub panel: Option<Panel>,
    /// Whether the panel takes the whole page (RV-2).
    pub widened: bool,
    /// What the composer holds.
    pub draft: String,
    /// Messages waiting for the turn to end, first to go first.
    pub queue: Vec<String>,
}

/// A fork being named, before it is made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForkDraft {
    /// How it shares the parent's work.
    pub kind: ForkKind,
    /// Its title.
    pub title: String,
}

/// The session pages' state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionState {
    /// What the pages show.
    pub workspace: Arc<Workspace>,
    /// The session on screen.
    pub open: Option<SessionId>,
    /// Each session's page, kept while the app runs and saved.
    pub views: BTreeMap<SessionId, SessionView>,
    /// Output that arrived after the session's last snapshot, per session
    /// and terminal tab.
    pub live: HashMap<(SessionId, usize), Arc<Vec<Line>>>,
    /// Whether a message is on its way.
    pub sending: bool,
    /// The fork dialog, if open (FK-1).
    pub fork: Option<ForkDraft>,
    /// What went wrong last, for the page's error banner.
    pub error: Option<String>,
}

impl SessionState {
    /// The page of the open session.
    #[must_use]
    pub fn view(&self) -> Option<&SessionView> {
        self.open.as_ref().and_then(|id| self.views.get(id))
    }

    fn view_mut(&mut self) -> Option<&mut SessionView> {
        let id = self.open.clone()?;
        Some(self.views.entry(id).or_default())
    }

    /// The skills the `/` menu offers for the open draft, if it is a
    /// command being typed.
    #[must_use]
    pub fn suggestions(&self) -> Vec<&Skill> {
        let Some(draft) = self.view().map(|view| view.draft.as_str()) else {
            return Vec::new();
        };
        let Some(typed) = draft.strip_prefix('/') else {
            return Vec::new();
        };
        if typed.contains(char::is_whitespace) {
            return Vec::new();
        }
        self.workspace
            .skills
            .iter()
            .filter(|skill| skill.name.starts_with(typed))
            .collect()
    }
}

/// What the session pages ask their parent to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delegate {
    /// Open this session, such as a new fork.
    OpenSession(SessionId),
    /// Say this in a toast.
    Toast(String),
    /// The open session was archived: leave its page.
    Left,
}

/// What happens on a session's page.
#[derive(Clone, Debug, PartialEq)]
pub enum SessionAction {
    /// The data changed.
    Show(Arc<Workspace>),
    /// Saved pages came back at launch.
    Restore(BTreeMap<SessionId, SessionView>),
    /// Show this session.
    Open(SessionId),
    /// A line arrived for a session's tab.
    Output {
        /// The session.
        session: SessionId,
        /// The terminal tab.
        tab: usize,
        /// The line.
        line: Line,
    },
    /// A new snapshot of the session arrived, holding the agent tab's
    /// output so far.
    Snapshot(SessionId),
    /// A terminal tab was picked.
    SelectTab(usize),
    /// A panel tab was picked, or the open one closed with `None`.
    SetPanel(Option<Panel>),
    /// The panel was widened or narrowed again.
    ToggleWiden,
    /// The composer's text changed.
    Draft(String),
    /// A `/` suggestion was picked.
    PickSkill(String),
    /// Enter in the composer: send now if the turn ended, else queue.
    Submit,
    /// Send the first queued message even though the turn may not have
    /// ended (SE-23).
    SendNow,
    /// Take a queued message back into the composer to edit it.
    EditQueued(usize),
    /// Drop a queued message.
    RemoveQueued(usize),
    /// The daemon took a message, or didn't.
    Sent(SessionId, Result<(), DaemonError>),
    /// Start an exited session again on its conversation (SE-14).
    Resume,
    /// Open the fork dialog.
    StartFork(ForkKind),
    /// The fork dialog's title changed.
    ForkTitle(String),
    /// Make the fork.
    ConfirmFork,
    /// Close the fork dialog.
    CancelFork,
    /// The daemon made the fork, or didn't.
    Forked(Result<SessionId, DaemonError>),
    /// Archive the open session.
    Archive,
    /// Pin or unpin the open session.
    TogglePin,
    /// Mark or unmark the open session very important.
    ToggleVip,
    /// A request finished; on failure its message is shown.
    Done(Result<(), DaemonError>),
    /// Hide the error banner.
    DismissError,
    /// For the parent.
    Delegate(Delegate),
}

/// The session pages' reducer.
#[derive(Clone)]
pub struct Sessions {
    clients: Clients,
}

impl Sessions {
    /// Session pages that talk to `clients`.
    #[must_use]
    pub const fn new(clients: Clients) -> Self {
        Self { clients }
    }

    fn send(&self, session: SessionId, text: String) -> Effect<SessionAction> {
        let daemon = self.clients.daemon.clone();
        Effect::future(async move {
            let result = daemon.send(&session, &text).await;
            SessionAction::Sent(session, result)
        })
    }

    fn done(
        &self,
        work: impl FnOnce(&dyn humata2_clients::Daemon) -> humata2_clients::Reply<()>,
    ) -> Effect<SessionAction> {
        let future = work(self.clients.daemon.as_ref());
        Effect::future(async move { SessionAction::Done(future.await) })
    }

    /// Sends the first queued message of `session` if its turn ended.
    fn flush(&self, state: &mut SessionState, session: &SessionId) -> Effect<SessionAction> {
        let ended = state
            .workspace
            .session(session)
            .is_some_and(|session| session.turn == Turn::Ended);
        if !ended || state.sending {
            return Effect::none();
        }
        let Some(view) = state.views.get_mut(session) else {
            return Effect::none();
        };
        if view.queue.is_empty() {
            return Effect::none();
        }
        let text = view.queue.remove(0);
        state.sending = true;
        self.send(session.clone(), text)
    }
}

impl Reducer for Sessions {
    type State = SessionState;
    type Action = SessionAction;

    #[allow(clippy::too_many_lines)]
    fn reduce(&self, state: &mut SessionState, action: SessionAction) -> Effect<SessionAction> {
        match action {
            SessionAction::Show(workspace) => {
                state.workspace = workspace;
                let waiting: Vec<_> = state
                    .views
                    .iter()
                    .filter(|(_, view)| !view.queue.is_empty())
                    .map(|(id, _)| id.clone())
                    .collect();
                let effects: Vec<_> = waiting
                    .iter()
                    .map(|id| self.flush(state, id))
                    .collect();
                return Effect::merge(effects);
            }
            SessionAction::Restore(views) => state.views = views,
            SessionAction::Open(id) => {
                state.views.entry(id.clone()).or_default();
                state.open = Some(id.clone());
                state.fork = None;
                state.error = None;
                let unread = state
                    .workspace
                    .session(&id)
                    .is_some_and(|session| session.unread > 0 || session.recovery.is_some());
                if unread {
                    return self.done(|daemon| daemon.mark_read(&id));
                }
            }
            SessionAction::Output { session, tab, line } => {
                let lines = state.live.entry((session, tab)).or_default();
                let lines = Arc::make_mut(lines);
                lines.push(line);
                if lines.len() > LIVE_LINES {
                    let extra = lines.len() - LIVE_LINES;
                    lines.drain(..extra);
                }
            }
            SessionAction::Snapshot(session) => {
                state.live.remove(&(session, 0));
            }
            SessionAction::SelectTab(tab) => {
                let tabs = state
                    .open
                    .as_ref()
                    .and_then(|id| state.workspace.session(id))
                    .map_or(1, |session| session.shells.len() + 1);
                if let Some(view) = state.view_mut() {
                    view.tab = tab.min(tabs - 1);
                }
            }
            SessionAction::SetPanel(panel) => {
                if let Some(view) = state.view_mut() {
                    view.panel = panel;
                    if panel.is_none() {
                        view.widened = false;
                    }
                }
            }
            SessionAction::ToggleWiden => {
                if let Some(view) = state.view_mut()
                    && view.panel.is_some()
                {
                    view.widened = !view.widened;
                }
            }
            SessionAction::Draft(text) => {
                if let Some(view) = state.view_mut() {
                    view.draft = text;
                }
            }
            SessionAction::PickSkill(name) => {
                if let Some(view) = state.view_mut() {
                    view.draft = format!("/{name} ");
                }
            }
            SessionAction::Submit => {
                let Some(id) = state.open.clone() else {
                    return Effect::none();
                };
                let Some(session) = state.workspace.session(&id) else {
                    return Effect::none();
                };
                if !session.status.accepts_input() {
                    state.error = Some(format!(
                        "{} isn't running, so it can't take a message. Resume it first, then send again.",
                        session.title
                    ));
                    return Effect::none();
                }
                let Some(view) = state.view_mut() else {
                    return Effect::none();
                };
                let text = view.draft.trim().to_owned();
                if text.is_empty() {
                    return Effect::none();
                }
                view.draft.clear();
                view.queue.push(text);
                return self.flush(state, &id);
            }
            SessionAction::SendNow => {
                let Some(id) = state.open.clone() else {
                    return Effect::none();
                };
                if state.sending {
                    return Effect::none();
                }
                let Some(view) = state.view_mut() else {
                    return Effect::none();
                };
                if view.queue.is_empty() {
                    return Effect::none();
                }
                let text = view.queue.remove(0);
                state.sending = true;
                return self.send(id, text);
            }
            SessionAction::EditQueued(index) => {
                if let Some(view) = state.view_mut()
                    && index < view.queue.len()
                {
                    let text = view.queue.remove(index);
                    if view.draft.trim().is_empty() {
                        view.draft = text;
                    } else {
                        view.draft = format!("{text}\n{}", view.draft);
                    }
                }
            }
            SessionAction::RemoveQueued(index) => {
                if let Some(view) = state.view_mut()
                    && index < view.queue.len()
                {
                    view.queue.remove(index);
                }
            }
            SessionAction::Sent(session, result) => {
                state.sending = false;
                match result {
                    // The daemon echoes the message as output.
                    Ok(()) => return self.flush(state, &session),
                    Err(error) => state.error = Some(error.to_string()),
                }
            }
            SessionAction::Resume => {
                if let Some(id) = state.open.clone() {
                    return self.done(|daemon| daemon.resume(&id));
                }
            }
            SessionAction::StartFork(kind) => {
                let title = state
                    .open
                    .as_ref()
                    .and_then(|id| state.workspace.session(id))
                    .map(|session| format!("{} (fork)", session.title))
                    .unwrap_or_default();
                state.fork = Some(ForkDraft { kind, title });
            }
            SessionAction::ForkTitle(title) => {
                if let Some(fork) = &mut state.fork {
                    fork.title = title;
                }
            }
            SessionAction::ConfirmFork => {
                let (Some(id), Some(fork)) = (state.open.clone(), state.fork.take()) else {
                    return Effect::none();
                };
                let daemon = self.clients.daemon.clone();
                return Effect::future(async move {
                    SessionAction::Forked(daemon.fork(&id, fork.kind, fork.title.trim()).await)
                });
            }
            SessionAction::CancelFork => state.fork = None,
            SessionAction::Forked(result) => match result {
                Ok(id) => return Effect::send(SessionAction::Delegate(Delegate::OpenSession(id))),
                Err(error) => state.error = Some(error.to_string()),
            },
            SessionAction::Archive => {
                if let Some(id) = state.open.take() {
                    let title = state
                        .workspace
                        .session(&id)
                        .map(|session| session.title.clone())
                        .unwrap_or_default();
                    return self.done(|daemon| daemon.archive(&id)).and(Effect::concat([
                        Effect::send(SessionAction::Delegate(Delegate::Left)),
                        Effect::send(SessionAction::Delegate(Delegate::Toast(format!(
                            "Archived {title}. It stays in Trash for 30 days."
                        )))),
                    ]));
                }
            }
            SessionAction::TogglePin => {
                if let Some(session) = open_session(state) {
                    let (id, pinned) = (session.id.clone(), !session.pinned);
                    return self.done(|daemon| daemon.set_pinned(&id, pinned));
                }
            }
            SessionAction::ToggleVip => {
                if let Some(session) = open_session(state) {
                    let (id, vip) = (session.id.clone(), !session.vip);
                    return self.done(|daemon| daemon.set_vip(&id, vip));
                }
            }
            SessionAction::Done(result) => {
                if let Err(error) = result {
                    state.error = Some(error.to_string());
                }
            }
            SessionAction::DismissError => state.error = None,
            SessionAction::Delegate(_) => {}
        }
        Effect::none()
    }
}

fn open_session(state: &SessionState) -> Option<&humata2_clients::model::Session> {
    state
        .open
        .as_ref()
        .and_then(|id| state.workspace.session(id))
}

/// The lines a terminal tab shows: for the agent's tab the last snapshot's
/// transcript, then live output.
#[must_use]
pub fn lines(state: &SessionState, session: &SessionId, tab: usize) -> Vec<Line> {
    let mut lines = if tab == 0 {
        state
            .workspace
            .session(session)
            .map(|session| session.transcript.clone())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    if let Some(live) = state.live.get(&(session.clone(), tab)) {
        lines.extend(live.iter().cloned());
    }
    lines
}

/// A line from the person, as the terminal echoes it.
#[must_use]
pub fn echo(text: &str) -> Line {
    Line {
        from: Speaker::You,
        text: text.to_owned(),
    }
}

//! The Desk (DK-1 to DK-5, DK-10 to DK-14, SE-26): sessions that belong to
//! no channel. The page lists the open ones, the documents in their folders
//! and the closed ones; New desk session, Close, Reopen, Keep, Clean up…
//! and Copy to a channel… are changes it asks a daemon for.

mod channel;
mod clean_up;
mod close;
mod copy;
mod new;

use std::collections::BTreeSet;
use std::sync::Arc;

use humata2_clients::desk::{Access, DeskClient, DeskSession, Document, Switch};
use humata2_clients::model::{
    Agent, ChannelId, Machine, MachineId, Session, SessionId,
};
use humata2_clients::{DaemonError, Workspace};
use serde::{Deserialize, Serialize};
use tca::{Effect, Reducer};

pub use channel::{ChannelCloseSheet, ChannelForm, ChannelNewAction, ChannelRow};
pub use clean_up::CleanUpDialog;
pub use close::CloseDialog;
pub use copy::CopyDialog;
pub use humata2_clients::desk::{CleanUp, CleanUpRow};
pub use new::{NewAction, NewForm};

/// How the Open list is ordered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sort {
    /// Most recently used first (SE-26).
    #[default]
    LastUsed,
    /// By title.
    Title,
}

/// A pop-up open on the Desk page or on a Desk session's folder bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Menu {
    /// The Sort pop-up in the Desk page's header.
    Sort,
    /// The machine picker in this session's folder bar (DK-3).
    Machine(SessionId),
    /// The Add repo field in this session's folder bar (DK-3).
    AddRepo(SessionId),
    /// The new-target picker in this channel Desk session's folder bar
    /// (CD-5).
    Target(SessionId),
}

/// A Desk session with what the Desk knows about it.
#[derive(Clone, Copy, Debug)]
pub struct Row<'a> {
    /// The session.
    pub session: &'a Session,
    /// Its folder, repos, access and when it was last used.
    pub desk: &'a DeskSession,
}

/// The Desk page's state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeskState {
    /// Where the sessions and documents come from.
    pub workspace: Arc<Workspace>,
    /// How the Open list is ordered.
    pub sort: Sort,
    /// The New desk session form.
    pub new: Option<NewForm>,
    /// The Clean up… dialog.
    pub clean_up: Option<CleanUpDialog>,
    /// The Copy to a channel… dialog.
    pub copy: Option<CopyDialog>,
    /// The New desk session dialog on a channel's Desk (CD-6).
    pub channel_new: Option<ChannelForm>,
    /// The Close session confirmation of a channel Desk session (CD-4).
    pub channel_close: Option<ChannelCloseSheet>,
    /// The Close session question (DV-4).
    pub close: Option<CloseDialog>,
    /// The pop-up that is open, if one is.
    pub menu: Option<Menu>,
    /// Sessions with a change in flight.
    pub pending: BTreeSet<SessionId>,
    /// What went wrong last, outside a dialog.
    pub error: Option<String>,
}

impl DeskState {
    fn rows(&self, closed: bool) -> Vec<Row<'_>> {
        self.workspace
            .sessions
            .iter()
            .filter_map(|session| {
                let desk = self.workspace.desk.sessions.get(&session.id)?;
                (desk.closed.is_some() == closed).then_some(Row { session, desk })
            })
            .collect()
    }

    /// The open Desk sessions, in the order of [`DeskState::sort`].
    #[must_use]
    pub fn open(&self) -> Vec<Row<'_>> {
        let mut rows = self.rows(false);
        match self.sort {
            Sort::LastUsed => rows.sort_by(|a, b| {
                b.desk
                    .last_used
                    .order
                    .cmp(&a.desk.last_used.order)
                    .then_with(|| a.session.id.cmp(&b.session.id))
            }),
            Sort::Title => rows.sort_by(|a, b| {
                a.session
                    .title
                    .to_lowercase()
                    .cmp(&b.session.title.to_lowercase())
                    .then_with(|| a.session.id.cmp(&b.session.id))
            }),
        }
        rows
    }

    /// The closed Desk sessions, most recently used first.
    #[must_use]
    pub fn closed(&self) -> Vec<Row<'_>> {
        let mut rows = self.rows(true);
        rows.sort_by(|a, b| {
            b.desk
                .last_used
                .order
                .cmp(&a.desk.last_used.order)
                .then_with(|| a.session.id.cmp(&b.session.id))
        });
        rows
    }

    /// The documents in Desk folders, each with its session, in the Desk's
    /// order. A document whose session is gone is left out.
    #[must_use]
    pub fn documents(&self) -> Vec<(&Document, &Session)> {
        self.workspace
            .desk
            .documents
            .iter()
            .filter_map(|document| {
                let session = self
                    .workspace
                    .sessions
                    .iter()
                    .find(|session| session.id == document.session)?;
                Some((document, session))
            })
            .collect()
    }

    /// What the Desk knows about a session.
    #[must_use]
    pub fn record(&self, id: &SessionId) -> Option<&DeskSession> {
        self.workspace.desk.sessions.get(id)
    }

    /// Whether the New desk session form, Clean up… or Copy is open.
    #[must_use]
    pub const fn sheet_open(&self) -> bool {
        if self.channel_new.is_some() || self.channel_close.is_some() {
            return true;
        }
        self.new.is_some()
            || self.clean_up.is_some()
            || self.copy.is_some()
            || self.close.is_some()
    }

    /// Whether anything Escape closes is open: one of the three dialogs or
    /// a pop-up.
    #[must_use]
    pub const fn dialog_open(&self) -> bool {
        self.sheet_open() || self.menu.is_some()
    }

    /// The machines of the copy dialog's channel, for its picker.
    #[must_use]
    pub fn copy_machines(&self) -> Vec<&Machine> {
        let Some(channel) = self.copy.as_ref().and_then(|copy| copy.channel.as_ref()) else {
            return Vec::new();
        };
        let Some(channel) = self.workspace.channels.iter().find(|c| &c.id == channel) else {
            return Vec::new();
        };
        channel
            .machines
            .iter()
            .filter_map(|id| self.workspace.machines.iter().find(|m| &m.id == id))
            .collect()
    }
}

/// What the Desk asks its parent to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delegate {
    /// Open a session.
    OpenSession(SessionId),
    /// Open a session's own folder in the file manager (D382).
    OpenFolder(SessionId),
    /// Say this in a toast.
    Toast(String),
}

/// Which change to a session came back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Close session.
    Close,
    /// Reopen.
    Reopen,
    /// Keep folder.
    Keep,
    /// Add repo.
    AddRepo,
    /// Remove a repo.
    RemoveRepo,
    /// Move to another machine.
    Move,
    /// Agent access.
    Access,
    /// A channel Desk session's switches (CD-3).
    Switches,
    /// What a channel Desk session looks after (CD-5).
    Target,
}

/// What happens on the Desk page.
#[derive(Clone, Debug, PartialEq)]
pub enum DeskAction {
    /// The data changed.
    Show(Arc<Workspace>),
    /// The sort was picked.
    SetSort(Sort),
    /// A row was clicked.
    Open(SessionId),
    /// Open folder was clicked on a session's folder bar.
    OpenFolder(SessionId),
    /// New desk session was clicked, with the agent to start from.
    StartNew {
        /// The agent the form starts with.
        agent: Agent,
    },
    /// A change to the New desk session form.
    New(NewAction),
    /// The daemon started the session, or didn't.
    Started(Result<SessionId, DaemonError>),
    /// Close session was asked for, from the menu, the palette or a
    /// shortcut: closes at once, or asks first (DV-4).
    AskClose(SessionId),
    /// Close session, without asking; also Close in the question.
    Close(SessionId),
    /// Save version and close was clicked in the question.
    SaveAndClose(SessionId),
    /// The version saved before closing was saved, or wasn't.
    SavedBeforeClose {
        /// The session.
        session: SessionId,
        /// How it went.
        result: Result<(), DaemonError>,
    },
    /// Reopen was clicked.
    Reopen(SessionId),
    /// Keep was clicked.
    Keep(SessionId),
    /// A repo was added to a session.
    AddRepo(SessionId, String),
    /// A repo was removed from a session.
    RemoveRepo(SessionId, String),
    /// A session was moved to a machine.
    Move(SessionId, MachineId),
    /// A session's access switches were set.
    SetAccess(SessionId, Access),
    /// A change to a session finished.
    Changed {
        /// The session.
        session: SessionId,
        /// What was asked.
        change: Change,
        /// How it went.
        result: Result<(), DaemonError>,
    },
    /// Clean up… was clicked.
    OpenCleanUp,
    /// The closed sessions were listed.
    CleanUpListed(Result<Vec<CleanUpRow>, DaemonError>),
    /// A row's tick was clicked.
    ToggleCleanUp(SessionId),
    /// Delete was clicked.
    ConfirmCleanUp,
    /// The clean-up finished.
    CleanedUp(Result<CleanUp, DaemonError>),
    /// Copy to a channel… was clicked on a document.
    StartCopy(Document),
    /// A channel was picked.
    CopyChannel(ChannelId),
    /// A machine was picked.
    CopyMachine(MachineId),
    /// Copy was clicked.
    ConfirmCopy,
    /// The copy finished.
    Copied(Result<(), DaemonError>),
    /// New desk session on a channel's Desk tab (CD-6), or for one of its
    /// sessions, with the agent to start from.
    StartChannelNew {
        /// The channel.
        channel: ChannelId,
        /// The session to look after; `None` for the whole channel.
        target: Option<SessionId>,
        /// The agent the form starts with.
        agent: Agent,
    },
    /// A change to the channel's New desk session dialog.
    ChannelNew(ChannelNewAction),
    /// Close session was confirmed on a channel Desk session (CD-4).
    ConfirmCloseChannel,
    /// Copy to shared/ was clicked on a Markdown file in the close sheet.
    CopyToShared(String),
    /// The copy to shared/ finished.
    SharedCopied {
        /// The file.
        path: String,
        /// How it went.
        result: Result<(), DaemonError>,
    },
    /// A channel Desk session's switch was turned on or off (CD-3).
    SetSwitch(SessionId, Switch, bool),
    /// A channel Desk session was pointed at a session of its channel, or
    /// with `None` at the whole channel (CD-5).
    SetTarget(SessionId, Option<SessionId>),
    /// Cancel or Escape: closes the innermost thing that is open: a
    /// pop-up first, then the dialog.
    CancelDialog,
    /// A pop-up was opened (`Some`) or closed (`None`).
    Menu(Option<Menu>),
    /// The error was dismissed.
    DismissError,
    /// For the parent.
    Delegate(Delegate),
}

/// The Desk page's reducer.
#[derive(Clone)]
pub struct Desk {
    client: Arc<dyn DeskClient>,
}

impl Desk {
    /// A Desk that changes things with `client`.
    #[must_use]
    pub fn new(client: Arc<dyn DeskClient>) -> Self {
        Self { client }
    }

    fn change<F>(
        &self,
        state: &mut DeskState,
        session: SessionId,
        change: Change,
        call: F,
    ) -> Effect<DeskAction>
    where
        F: FnOnce(&dyn DeskClient, &SessionId) -> humata2_clients::Reply<()> + Send + 'static,
    {
        if !state.pending.insert(session.clone()) {
            return Effect::none();
        }
        let client = self.client.clone();
        Effect::future(async move {
            let result = call(&*client, &session).await;
            DeskAction::Changed {
                session,
                change,
                result,
            }
        })
    }

    fn changed(
        state: &mut DeskState,
        session: SessionId,
        change: Change,
        result: Result<(), DaemonError>,
    ) -> Effect<DeskAction> {
        state.pending.remove(&session);
        if change == Change::Close {
            Self::close_sheet_answered(state, &session, &result);
        }
        if let Err(error) = result {
            state.error = Some(error.to_string());
            return Effect::none();
        }
        let toast = |text: String| Effect::send(DeskAction::Delegate(Delegate::Toast(text)));
        match change {
            Change::Reopen => Effect::send(DeskAction::Delegate(Delegate::OpenSession(session))),
            Change::Close if state.workspace.desk.channel_sessions.contains_key(&session) => {
                let title = state
                    .workspace
                    .sessions
                    .iter()
                    .find(|s| s.id == session)
                    .map_or_else(|| session.to_string(), |s| s.title.clone());
                toast(format!(
                    "Closed {title}. Its folder is in the trash; restore it from there within 30 days."
                ))
            }
            Change::Close => {
                let title = state
                    .workspace
                    .sessions
                    .iter()
                    .find(|s| s.id == session)
                    .map_or_else(|| session.to_string(), |s| s.title.clone());
                let expires = state
                    .record(&session)
                    .is_some_and(|record| record.folder.expires() && record.unpushed.is_none());
                toast(if expires {
                    format!("Closed {title}. Its folder is deleted in 7 days unless you keep it.")
                } else {
                    format!("Closed {title}. Reopen it from the Closed list.")
                })
            }
            Change::Keep => toast("The folder stays after the session closes.".to_owned()),
            Change::AddRepo
            | Change::RemoveRepo
            | Change::Move
            | Change::Access
            | Change::Switches
            | Change::Target => Effect::none(),
        }
    }
}

impl Reducer for Desk {
    type State = DeskState;
    type Action = DeskAction;

    fn reduce(&self, state: &mut DeskState, action: DeskAction) -> Effect<DeskAction> {
        match action {
            DeskAction::Show(workspace) => {
                state.workspace = workspace;
                let gone = state.copy.as_ref().is_some_and(|copy| {
                    !state.workspace.desk.documents.contains(&copy.document)
                });
                if gone {
                    state.copy = None;
                }
                Effect::none()
            }
            DeskAction::SetSort(sort) => {
                state.sort = sort;
                state.menu = None;
                Effect::none()
            }
            DeskAction::Menu(menu) => {
                // A pop-up never opens over a dialog.
                if menu.is_none() || !state.sheet_open() {
                    state.menu = menu;
                }
                Effect::none()
            }
            DeskAction::Open(session) => {
                Effect::send(DeskAction::Delegate(Delegate::OpenSession(session)))
            }
            DeskAction::OpenFolder(session) => {
                Effect::send(DeskAction::Delegate(Delegate::OpenFolder(session)))
            }
            DeskAction::StartNew { agent } => {
                Self::start_new(state, agent);
                Effect::none()
            }
            DeskAction::New(action) => self.reduce_new(state, action),
            DeskAction::Started(result) => match result {
                Ok(id) => {
                    state.new = None;
                    state.channel_new = None;
                    Effect::send(DeskAction::Delegate(Delegate::OpenSession(id)))
                }
                Err(error) => {
                    if let Some(form) = state.new.as_mut() {
                        form.starting = false;
                        form.error = Some(error.to_string());
                    }
                    if let Some(form) = state.channel_new.as_mut() {
                        form.starting = false;
                        form.error = Some(error.to_string());
                    }
                    Effect::none()
                }
            },
            DeskAction::AskClose(id) => self.ask_close(state, id),
            DeskAction::Close(id) => self.close_now(state, id),
            DeskAction::SaveAndClose(id) => self.save_and_close(state, id),
            DeskAction::SavedBeforeClose { session, result } => {
                self.saved_before_close(state, session, result)
            }
            DeskAction::Reopen(id) => self.change(state, id, Change::Reopen, |c, s| c.reopen(s)),
            DeskAction::Keep(id) => self.change(state, id, Change::Keep, |c, s| c.keep(s)),
            DeskAction::AddRepo(id, repo) => {
                state.menu = None;
                self.change(state, id, Change::AddRepo, move |c, s| c.add_repo(s, &repo))
            }
            DeskAction::RemoveRepo(id, repo) => self.change(state, id, Change::RemoveRepo, move |c, s| {
                c.remove_repo(s, &repo)
            }),
            DeskAction::Move(id, machine) => {
                state.menu = None;
                self.change(state, id, Change::Move, move |c, s| c.move_to(s, &machine))
            }
            DeskAction::SetAccess(id, access) => {
                self.change(state, id, Change::Access, move |c, s| c.set_access(s, access))
            }
            DeskAction::Changed {
                session,
                change,
                result,
            } => Self::changed(state, session, change, result),
            DeskAction::OpenCleanUp => self.open_clean_up(state),
            DeskAction::CleanUpListed(result) => {
                Self::listed(state, result);
                Effect::none()
            }
            DeskAction::ToggleCleanUp(id) => {
                Self::toggle_clean_up(state, id);
                Effect::none()
            }
            DeskAction::ConfirmCleanUp => self.confirm_clean_up(state),
            DeskAction::CleanedUp(result) => Self::cleaned_up(state, result),
            DeskAction::StartCopy(document) => {
                Self::start_copy(state, document);
                Effect::none()
            }
            DeskAction::CopyChannel(id) => {
                Self::copy_channel(state, id);
                Effect::none()
            }
            DeskAction::CopyMachine(id) => {
                Self::copy_machine(state, id);
                Effect::none()
            }
            DeskAction::ConfirmCopy => self.confirm_copy(state),
            DeskAction::Copied(result) => Self::copied(state, result),
            DeskAction::CancelDialog => {
                if state.menu.take().is_some() {
                    return Effect::none();
                }
                if let Some(form) = state.new.as_mut()
                    && (form.machines_open || form.adding_repo)
                {
                    form.machines_open = false;
                    form.adding_repo = false;
                    return Effect::none();
                }
                if let Some(form) = state.channel_new.as_mut()
                    && (form.machines_open || form.sessions_open)
                {
                    form.machines_open = false;
                    form.sessions_open = false;
                    return Effect::none();
                }
                state.new = None;
                state.clean_up = None;
                state.copy = None;
                state.channel_new = None;
                // A close in flight can't be taken back; the sheet stays until it answers.
                if state.channel_close.as_ref().is_some_and(|sheet| !sheet.closing) {
                    state.channel_close = None;
                }
                // A version being saved stays saved; the session stays open.
                state.close = None;
                Effect::none()
            }
            DeskAction::StartChannelNew {
                channel,
                target,
                agent,
            } => {
                Self::start_channel_new(state, channel, target, agent);
                Effect::none()
            }
            DeskAction::ChannelNew(action) => self.reduce_channel_new(state, action),
            DeskAction::ConfirmCloseChannel => self.confirm_close_channel(state),
            DeskAction::CopyToShared(path) => self.copy_to_shared(state, path),
            DeskAction::SharedCopied { path, result } => Self::shared_copied(state, path, result),
            DeskAction::SetSwitch(id, switch, on) => self.set_switch(state, id, switch, on),
            DeskAction::SetTarget(id, target) => self.set_target(state, id, target),
            DeskAction::DismissError => {
                state.error = None;
                Effect::none()
            }
            DeskAction::Delegate(_) => Effect::none(),
        }
    }
}

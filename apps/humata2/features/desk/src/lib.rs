//! The Desk (DK-1 to DK-5, DK-10 to DK-14, SE-26): sessions that belong to
//! no channel. The page lists the open ones, the documents in their folders
//! and the closed ones; New desk session, Close, Reopen, Keep, Clean up…
//! and Copy to a channel… are changes it asks a daemon for.

mod clean_up;
mod copy;
mod new;

use std::collections::BTreeSet;
use std::sync::Arc;

use humata2_clients::desk::{Access, DeskClient, DeskSession, Document};
use humata2_clients::model::{
    Agent, ChannelId, Item, ItemStatus, Machine, MachineId, Session, SessionId,
};
use humata2_clients::{DaemonError, Workspace};
use serde::{Deserialize, Serialize};
use tca::{Effect, Reducer};

pub use clean_up::CleanUpDialog;
pub use copy::CopyDialog;
pub use humata2_clients::desk::{CleanUp, CleanUpRow};
pub use new::{FolderPick, NewAction, NewForm};

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
}

/// A Desk session with what the Desk knows about it.
#[derive(Clone, Copy, Debug)]
pub struct Row<'a> {
    /// The session.
    pub session: &'a Session,
    /// Its folder, repos, access and when it was last used.
    pub desk: &'a DeskSession,
}

/// An item in a Desk session's Board panel (DK-4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaneItem<'a> {
    /// The item.
    pub item: &'a Item,
    /// Whether the session whose panel this is added it.
    pub added: bool,
}

/// One lane of a Desk session's Board panel (DK-4): a status and every
/// board's items in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lane<'a> {
    /// The lane's status.
    pub status: ItemStatus,
    /// Its items: the ones the session added first, then the rest, each in
    /// the order the workspace holds them.
    pub items: Vec<LaneItem<'a>>,
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

    /// The lanes of `session`'s Board panel (DK-4, BD-7): every board's
    /// items by status, in the compact board's order (active lanes first), leaving out the lanes
    /// with nothing in them. A Desk session has no board of its own
    /// (DK-12), so this is every item there is.
    #[must_use]
    pub fn board(&self, session: &SessionId) -> Vec<Lane<'_>> {
        let workspace: &Workspace = &self.workspace;
        let added = |item: &Item| {
            workspace
                .item_details
                .get(&item.id)
                .is_some_and(|details| details.created_by.as_ref() == Some(session))
        };
        ItemStatus::COMPACT
            .into_iter()
            .filter_map(|status| {
                let mut items: Vec<LaneItem<'_>> = workspace
                    .items
                    .iter()
                    .filter(|item| item.status == status)
                    .map(|item| LaneItem {
                        item,
                        added: added(item),
                    })
                    .collect();
                // A stable sort: each half keeps the workspace's order.
                items.sort_by_key(|entry| !entry.added);
                (!items.is_empty()).then_some(Lane { status, items })
            })
            .collect()
    }

    /// Whether the New desk session form, Clean up… or Copy is open.
    #[must_use]
    pub const fn sheet_open(&self) -> bool {
        self.new.is_some() || self.clean_up.is_some() || self.copy.is_some()
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
    /// New desk session was clicked, with the agent to start from.
    StartNew {
        /// The agent the form starts with.
        agent: Agent,
    },
    /// A change to the New desk session form.
    New(NewAction),
    /// The daemon started the session, or didn't.
    Started(Result<SessionId, DaemonError>),
    /// Close session was clicked.
    Close(SessionId),
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
        if let Err(error) = result {
            state.error = Some(error.to_string());
            return Effect::none();
        }
        let toast = |text: String| Effect::send(DeskAction::Delegate(Delegate::Toast(text)));
        match change {
            Change::Reopen => Effect::send(DeskAction::Delegate(Delegate::OpenSession(session))),
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
            Change::AddRepo | Change::RemoveRepo | Change::Move | Change::Access => Effect::none(),
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
            DeskAction::StartNew { agent } => {
                Self::start_new(state, agent);
                Effect::none()
            }
            DeskAction::New(action) => self.reduce_new(state, action),
            DeskAction::Started(result) => match result {
                Ok(id) => {
                    state.new = None;
                    Effect::send(DeskAction::Delegate(Delegate::OpenSession(id)))
                }
                Err(error) => {
                    if let Some(form) = state.new.as_mut() {
                        form.starting = false;
                        form.error = Some(error.to_string());
                    }
                    Effect::none()
                }
            },
            DeskAction::Close(id) => self.change(state, id, Change::Close, |c, s| c.close(s)),
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
                state.new = None;
                state.clean_up = None;
                state.copy = None;
                Effect::none()
            }
            DeskAction::DismissError => {
                state.error = None;
                Effect::none()
            }
            DeskAction::Delegate(_) => Effect::none(),
        }
    }
}

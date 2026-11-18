//! Boards (BD-1–10): the global board and each channel's, with columns by
//! status. Items move between columns, open in a drawer, gain dependencies
//! (a cycle is refused, BD-7) and are added with a title. A channel's board
//! can also show one session's board, or a session and its forks (BD-5–BD-8,
//! D216), and the session strip's Board panel lists a session's items.

use std::sync::Arc;

use humata2_clients::model::{
    Agent, FieldChange, Item, ItemChange, ItemId, ItemStatus, Scope, Session, SessionId,
};
use humata2_clients::{Clients, DaemonError, Workspace};
use serde::{Deserialize, Serialize};
use tca::{Effect, Reducer};

/// The approved ways of viewing a board.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Kanban,
    Table,
    Dashboard,
}
/// A dashboard drill-down, applied alongside scope and text search.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Filter {
    #[default]
    All,
    Status(ItemStatus),
    Waiting,
    Unassigned,
    DoneDay(u8),
    DoneWeek,
}
/// Which board a channel's page shows (BD-5–BD-8, D216).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelView {
    /// The channel's own items, not those of its sessions.
    #[default]
    Channel,
    /// The channel's items and every session's items in it.
    WithSessions,
    /// One session's items and those of its forks, at any depth.
    SessionAndForks(SessionId),
    /// One session's own items.
    OneSession(SessionId),
}

/// Presentation retained across restarts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct View {
    pub mode: Mode,
    pub query: String,
    pub filter: Filter,
    pub global_only: bool,
    /// The board a channel's page shows. Kept even while it names a session
    /// that is gone, so the session coming back restores it.
    pub channel_view: ChannelView,
}

/// What a drawer picker adds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerKind {
    /// Sessions working on the item.
    Sessions,
    /// Items it depends on.
    Dependencies,
}

/// The drawer's open picker: a filter over a list with a highlighted row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picker {
    /// What it adds.
    pub kind: PickerKind,
    /// The filter text.
    pub query: String,
    /// The highlighted row.
    pub cursor: usize,
}

/// One row of a picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    /// A session to add.
    Session(SessionId),
    /// An item to depend on.
    Item(ItemId),
}

/// What the agent behind a waiting request asks of the drawer's item (BD-15).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ask {
    /// Move the item.
    Move {
        /// The agent asking.
        by: Option<Agent>,
        /// The column it asks for.
        to: ItemStatus,
        /// The column the item is in.
        keep: ItemStatus,
    },
    /// Change fields.
    Edit {
        /// The agent asking.
        by: Option<Agent>,
        /// Each field, before and after.
        changes: Vec<FieldChange>,
    },
    /// Delete the item.
    Delete {
        /// The agent asking.
        by: Option<Agent>,
    },
}

/// The board's state.
#[derive(Clone, Debug, PartialEq)]
pub struct BoardState {
    /// Where items come from.
    pub workspace: Arc<Workspace>,
    /// Which board shows.
    pub scope: Scope,
    /// Saved presentation.
    pub view: View,
    /// Whether the scope menu is open.
    pub scope_menu: bool,
    /// Whether the status picker is open.
    pub status_menu: bool,
    /// Whether the drawer fills the board area.
    pub expanded: bool,
    /// The column for the next new item.
    pub new_status: ItemStatus,
    /// The item whose drawer is open.
    pub drawer: Option<ItemId>,
    /// A new item's title, while it is being written.
    pub new_title: Option<String>,
    /// The open picker under the drawer's Sessions or Depends on row.
    pub picker: Option<Picker>,
    /// What the drawer's item has been through, read when it opened.
    pub history: Vec<String>,
    /// What went wrong last.
    pub error: Option<String>,
}

impl Default for BoardState {
    fn default() -> Self {
        Self {
            workspace: Arc::default(),
            scope: Scope::Global,
            view: View::default(),
            scope_menu: false,
            status_menu: false,
            expanded: false,
            new_status: ItemStatus::Todo,
            drawer: None,
            new_title: None,
            picker: None,
            history: Vec::new(),
            error: None,
        }
    }
}

impl BoardState {
    /// The items in `status`'s column on the shown board.
    #[must_use]
    pub fn column(&self, status: ItemStatus) -> Vec<&Item> {
        self.workspace
            .items
            .iter()
            .filter(|item| self.matches(item) && item.status == status)
            .collect()
    }

    /// The board a channel's page shows, as saved. It reads as
    /// [`ChannelView::Channel`] when the saved session is gone, archived, or
    /// of another channel, and on any page that isn't a channel's.
    #[must_use]
    pub fn channel_view(&self) -> ChannelView {
        let Scope::Channel(channel) = &self.scope else {
            return ChannelView::Channel;
        };
        let session = match &self.view.channel_view {
            ChannelView::Channel | ChannelView::WithSessions => return self.view.channel_view.clone(),
            ChannelView::SessionAndForks(id) | ChannelView::OneSession(id) => id,
        };
        let live = self
            .workspace
            .session(session)
            .is_some_and(|s| !s.archived && &s.channel == channel);
        if live {
            self.view.channel_view.clone()
        } else {
            ChannelView::Channel
        }
    }

    /// The boards this channel's page offers: the channel's, with its
    /// sessions, each session that has forks with its forks, and each live
    /// session alone. Empty on any page that isn't a channel's.
    #[must_use]
    pub fn channel_views(&self) -> Vec<ChannelView> {
        let Scope::Channel(channel) = &self.scope else {
            return Vec::new();
        };
        let live: Vec<&Session> = self
            .workspace
            .sessions
            .iter()
            .filter(|s| &s.channel == channel && !s.archived)
            .collect();
        let mut views = vec![ChannelView::Channel, ChannelView::WithSessions];
        for session in &live {
            let has_forks = self
                .workspace
                .sessions
                .iter()
                .any(|s| !s.archived && s.parent.as_ref() == Some(&session.id));
            if has_forks {
                views.push(ChannelView::SessionAndForks(session.id.clone()));
            }
        }
        views.extend(live.iter().map(|s| ChannelView::OneSession(s.id.clone())));
        views
    }

    /// `root` and every session whose parent chain reaches it, at any depth.
    fn family(&self, root: &SessionId) -> Vec<SessionId> {
        let mut family = vec![root.clone()];
        let mut at = 0;
        while at < family.len() {
            let parent = family[at].clone();
            at += 1;
            for session in &self.workspace.sessions {
                if session.parent.as_ref() == Some(&parent) && !family.contains(&session.id) {
                    family.push(session.id.clone());
                }
            }
        }
        family
    }

    /// Whether this item is on the shown board, ignoring text and drill-down.
    #[must_use]
    pub fn in_scope(&self, item: &Item) -> bool {
        match &self.scope {
            Scope::Global => match &item.scope {
                Scope::Session { .. } => false,
                Scope::Global => true,
                Scope::Channel(_) => !self.view.global_only,
            },
            Scope::Channel(channel) => match self.channel_view() {
                ChannelView::Channel => item.scope == Scope::Channel(channel.clone()),
                ChannelView::WithSessions => item.scope.channel() == Some(channel),
                ChannelView::SessionAndForks(id) => {
                    let family = self.family(&id);
                    matches!(&item.scope, Scope::Session { session, .. } if family.contains(session))
                }
                ChannelView::OneSession(id) => {
                    matches!(&item.scope, Scope::Session { session, .. } if *session == id)
                }
            },
            Scope::Session { .. } => item.scope == self.scope,
        }
    }

    /// Whether this item belongs to the current scope and drill-down.
    #[must_use]
    pub fn matches(&self, item: &Item) -> bool {
        let scope = self.in_scope(item);
        let query = self.view.query.trim().to_lowercase();
        let text = query.is_empty()
            || item.title.to_lowercase().contains(&query)
            || item.id.as_str().to_lowercase().contains(&query)
            || item
                .session
                .as_ref()
                .and_then(|id| self.workspace.session(id))
                .is_some_and(|s| s.title.to_lowercase().contains(&query));
        scope
            && text
            && match self.view.filter {
                Filter::All => true,
                Filter::Status(status) => item.status == status,
                Filter::Waiting => self.request(item).is_some(),
                Filter::Unassigned => {
                    item.session.is_none()
                        && !matches!(item.status, ItemStatus::Done | ItemStatus::WontDo)
                }
                Filter::DoneWeek => {
                    item.status == ItemStatus::Done
                        && self
                            .workspace
                            .item_details
                            .get(&item.id)
                            .and_then(|d| d.done_day)
                            .is_some_and(|d| (7..14).contains(&d))
                }
                Filter::DoneDay(day) => {
                    item.status == ItemStatus::Done
                        && self
                            .workspace
                            .item_details
                            .get(&item.id)
                            .is_some_and(|d| d.done_day == Some(day))
                }
            }
    }
    /// The unresolved item-specific approval, if any.
    #[must_use]
    pub fn request(&self, item: &Item) -> Option<&humata2_clients::model::Request> {
        let request = self
            .workspace
            .item_details
            .get(&item.id)?
            .request
            .as_ref()?;
        self.workspace.waiting().find(|r| &r.id == request)
    }

    /// What a waiting request asks of `item`, if one still waits.
    #[must_use]
    pub fn ask(&self, item: &Item) -> Option<Ask> {
        let details = self.workspace.item_details.get(&item.id)?;
        let request = self.request(item)?;
        let by = self.workspace.session(&request.session).map(|s| s.agent);
        match &details.requested_change {
            Some(ItemChange::Edit(changes)) => Some(Ask::Edit {
                by,
                changes: changes.clone(),
            }),
            Some(ItemChange::Delete) => Some(Ask::Delete { by }),
            None => details.requested_status.map(|to| Ask::Move {
                by,
                to,
                keep: item.status,
            }),
        }
    }

    /// The rows the open picker offers for the drawer's item.
    #[must_use]
    pub fn choices(&self) -> Vec<Choice> {
        let (Some(picker), Some(item)) = (&self.picker, self.opened()) else {
            return Vec::new();
        };
        let query = picker.query.trim().to_lowercase();
        let details = self.workspace.item_details.get(&item.id);
        match picker.kind {
            PickerKind::Sessions => {
                let working = details.map(|d| d.sessions.as_slice()).unwrap_or_default();
                let allowed = |s: &&humata2_clients::model::Session| {
                    !s.archived
                        && !working.contains(&s.id)
                        && item.session.as_ref() != Some(&s.id)
                        && (query.is_empty() || s.title.to_lowercase().contains(&query))
                };
                let channels: Vec<_> = match &item.scope {
                    Scope::Channel(id) | Scope::Session { channel: id, .. } => vec![id],
                    Scope::Global => self.workspace.channels.iter().map(|c| &c.id).collect(),
                };
                let mut rows: Vec<Choice> = channels
                    .iter()
                    .flat_map(|channel| {
                        self.workspace
                            .sessions
                            .iter()
                            .filter(move |s| &s.channel == *channel)
                    })
                    .filter(allowed)
                    .map(|s| Choice::Session(s.id.clone()))
                    .collect();
                if item.scope == Scope::Global {
                    // Sessions of channels missing from the list still show.
                    rows.extend(
                        self.workspace
                            .sessions
                            .iter()
                            .filter(|s| !self.workspace.channels.iter().any(|c| c.id == s.channel))
                            .filter(allowed)
                            .map(|s| Choice::Session(s.id.clone())),
                    );
                }
                rows
            }
            PickerKind::Dependencies => {
                let mut blocked = vec![item.id.clone()];
                blocked.extend(item.depends_on.iter().cloned());
                // Anything that already waits on this item, however far.
                let mut at = 0;
                let mut dependents = vec![item.id.clone()];
                while at < dependents.len() {
                    let on = dependents[at].clone();
                    at += 1;
                    for other in &self.workspace.items {
                        if other.depends_on.contains(&on) && !dependents.contains(&other.id) {
                            dependents.push(other.id.clone());
                        }
                    }
                }
                blocked.extend(dependents);
                self.workspace
                    .items
                    .iter()
                    .filter(|other| !blocked.contains(&other.id))
                    .filter(|other| {
                        query.is_empty()
                            || other.id.as_str().to_lowercase().contains(&query)
                            || other.title.to_lowercase().contains(&query)
                    })
                    .map(|other| Choice::Item(other.id.clone()))
                    .collect()
            }
        }
    }

    /// The item whose drawer is open.
    #[must_use]
    pub fn opened(&self) -> Option<&Item> {
        let id = self.drawer.as_ref()?;
        self.workspace.items.iter().find(|item| &item.id == id)
    }
}

/// What the board asks its parent to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delegate {
    /// Open the session working on an item.
    OpenSession(SessionId),
    /// Say this in a toast.
    Toast(String),
    /// Read the workspace again.
    Refresh,
}

/// What happens on a board.
#[derive(Clone, Debug, PartialEq)]
pub enum BoardAction {
    /// The data changed.
    Show(Arc<Workspace>),
    /// Show this board.
    SetScope(Scope),
    /// Show this board of the channel's page, and close the scope menu.
    SetChannelView(ChannelView),
    /// Change the board's representation.
    Mode(Mode),
    /// Search titles, keys and assigned sessions.
    FilterText(String),
    /// Drill into items counted on the dashboard, showing the table.
    Drill(Filter),
    /// Clear the drill-down.
    ClearFilter,
    /// Show global items without channel items.
    GlobalOnly(bool),
    /// Open or close the scope picker.
    ScopeMenu,
    /// Open or close the drawer status picker.
    StatusMenu,
    /// Expand or restore the drawer.
    Expand,
    /// Start an item in a particular column.
    StartNewIn(ItemStatus),
    /// Select the new item's column.
    NewStatus(ItemStatus),
    /// Open an item's drawer.
    Open(ItemId),
    /// The drawer item's history arrived.
    History(ItemId, Result<Vec<String>, DaemonError>),
    /// Close the drawer.
    Close,
    /// Open a picker under the Sessions or Depends on row.
    OpenPicker(PickerKind),
    /// The picker's filter changed.
    PickerQuery(String),
    /// Highlight the row above.
    PickerUp,
    /// Highlight the row below.
    PickerDown,
    /// Add the highlighted row.
    PickerEnter,
    /// Add this row.
    Pick(Choice),
    /// Close the picker.
    ClosePicker,
    /// Take a session off the drawer's item.
    RemoveSession(SessionId),
    /// Drop a dependency of the drawer's item.
    RemoveDependency(ItemId),
    /// Move an item to a column.
    Move(ItemId, ItemStatus),
    /// Start writing a new item's title.
    StartNew,
    /// The title changed.
    NewTitle(String),
    /// Add the item.
    CreateNew,
    /// Drop the new item.
    CancelNew,
    /// Open the session working on the drawer's item.
    OpenSession,
    /// A request finished; on failure its message is shown.
    Done(Result<(), DaemonError>),
    /// For the parent.
    Delegate(Delegate),
}

/// The board's reducer.
#[derive(Clone)]
pub struct Board {
    clients: Clients,
}

impl Board {
    /// A board that changes items with `clients`.
    #[must_use]
    pub const fn new(clients: Clients) -> Self {
        Self { clients }
    }
}

impl Reducer for Board {
    type State = BoardState;
    type Action = BoardAction;

    fn reduce(&self, state: &mut BoardState, action: BoardAction) -> Effect<BoardAction> {
        let daemon = self.clients.daemon.clone();
        match action {
            BoardAction::Show(workspace) => state.workspace = workspace,
            BoardAction::Mode(mode) => state.view.mode = mode,
            BoardAction::FilterText(query) => state.view.query = query,
            BoardAction::Drill(filter) => {
                state.view.filter = filter;
                state.view.mode = Mode::Table;
                state.drawer = None;
            }
            BoardAction::ClearFilter => state.view.filter = Filter::All,
            BoardAction::GlobalOnly(only) => {
                state.scope = Scope::Global;
                state.view.global_only = only;
                state.scope_menu = false;
                state.drawer = None;
            }
            BoardAction::ScopeMenu => state.scope_menu = !state.scope_menu,
            BoardAction::StatusMenu => state.status_menu = !state.status_menu,
            BoardAction::Expand => state.expanded = !state.expanded,
            BoardAction::StartNewIn(status) => {
                state.new_title = Some(String::new());
                state.new_status = status;
            }
            BoardAction::NewStatus(status) => state.new_status = status,
            BoardAction::SetScope(scope) => {
                // Picking the scope already shown changes nothing but the
                // menu: the open drawer and its picker stay.
                if state.scope != scope {
                    state.view.global_only = false;
                    state.view.filter = Filter::All;
                    state.drawer = None;
                    state.picker = None;
                }
                state.scope = scope;
                state.scope_menu = false;
            }
            BoardAction::SetChannelView(view) => {
                state.view.channel_view = view;
                state.view.filter = Filter::All;
                state.scope_menu = false;
                state.drawer = None;
                state.picker = None;
            }
            BoardAction::Open(id) => {
                state.drawer = state
                    .workspace
                    .items
                    .iter()
                    .any(|i| i.id == id)
                    .then_some(id);
                state.status_menu = false;
                state.picker = None;
                state.history = Vec::new();
                if let Some(open) = state.drawer.clone() {
                    return Effect::future(async move {
                        let history = daemon.item_history(&open).await;
                        BoardAction::History(open, history)
                    });
                }
            }
            BoardAction::History(id, result) => {
                if state.drawer.as_ref() == Some(&id) {
                    match result {
                        Ok(history) => state.history = history,
                        Err(error) => state.error = Some(error.to_string()),
                    }
                }
            }
            BoardAction::Close => {
                state.drawer = None;
                state.picker = None;
                state.history = Vec::new();
                state.expanded = false;
            }
            BoardAction::OpenPicker(kind) => {
                if state.drawer.is_some() {
                    state.picker = Some(Picker {
                        kind,
                        query: String::new(),
                        cursor: 0,
                    });
                }
            }
            BoardAction::PickerQuery(text) => {
                if let Some(picker) = &mut state.picker {
                    picker.query = text;
                    picker.cursor = 0;
                }
            }
            BoardAction::PickerUp => {
                if let Some(picker) = &mut state.picker {
                    picker.cursor = picker.cursor.saturating_sub(1);
                }
            }
            BoardAction::PickerDown => {
                let last = state.choices().len().saturating_sub(1);
                if let Some(picker) = &mut state.picker {
                    picker.cursor = (picker.cursor + 1).min(last);
                }
            }
            BoardAction::PickerEnter => {
                let Some(picker) = &state.picker else {
                    return Effect::none();
                };
                let Some(choice) = state.choices().into_iter().nth(picker.cursor) else {
                    return Effect::none();
                };
                return add(state, &daemon, choice);
            }
            BoardAction::Pick(choice) => return add(state, &daemon, choice),
            BoardAction::ClosePicker => state.picker = None,
            BoardAction::RemoveSession(session) => {
                let Some(item) = state.drawer.clone() else {
                    return Effect::none();
                };
                return Effect::future(async move {
                    BoardAction::Done(daemon.set_item_session(&item, &session, false).await)
                });
            }
            BoardAction::RemoveDependency(on) => {
                let Some(item) = state.drawer.clone() else {
                    return Effect::none();
                };
                return Effect::future(async move {
                    BoardAction::Done(daemon.remove_dependency(&item, &on).await)
                });
            }
            BoardAction::Move(id, status) => {
                state.status_menu = false;
                // The move names the revision this screen shows, so a move
                // an agent made meanwhile is refused, not overwritten; the
                // refusal refreshes the board (`Done`) and the drawer stays.
                let seen = state
                    .workspace
                    .items
                    .iter()
                    .find(|item| item.id == id)
                    .map(|item| item.revision);
                return Effect::future(async move {
                    BoardAction::Done(daemon.move_item(&id, status, seen).await)
                });
            }
            BoardAction::StartNew => {
                state.new_title = Some(String::new());
                state.new_status = ItemStatus::Todo;
            }
            BoardAction::NewTitle(title) => {
                if let Some(new) = &mut state.new_title {
                    *new = title;
                }
            }
            BoardAction::CreateNew => {
                let Some(title) = state.new_title.take() else {
                    return Effect::none();
                };
                let title = title.trim().to_owned();
                if title.is_empty() {
                    return Effect::none();
                }
                // On one session's board, a new item goes on that board.
                let scope = match (&state.scope, state.channel_view()) {
                    (Scope::Channel(channel), ChannelView::OneSession(session)) => {
                        Scope::Session {
                            channel: channel.clone(),
                            session,
                        }
                    }
                    _ => state.scope.clone(),
                };
                let status = state.new_status;
                return Effect::future(async move {
                    BoardAction::Done(
                        daemon
                            .create_item_in(scope, &title, status)
                            .await
                            .map(|_| ()),
                    )
                });
            }
            BoardAction::CancelNew => state.new_title = None,
            BoardAction::OpenSession => {
                if let Some(session) = state.opened().and_then(|item| item.session.clone()) {
                    return Effect::send(BoardAction::Delegate(Delegate::OpenSession(session)));
                }
            }
            BoardAction::Done(result) => match result {
                Ok(()) => state.error = None,
                Err(error) => {
                    state.error = Some(error.to_string());
                    return Effect::send(BoardAction::Delegate(Delegate::Refresh));
                }
            },
            BoardAction::Delegate(_) => {}
        }
        Effect::none()
    }
}

/// Adds `choice` to the drawer's item, finishing with `Done`.
fn add(
    state: &BoardState,
    daemon: &Arc<dyn humata2_clients::Daemon>,
    choice: Choice,
) -> Effect<BoardAction> {
    let Some(item) = state.drawer.clone() else {
        return Effect::none();
    };
    let daemon = daemon.clone();
    Effect::future(async move {
        BoardAction::Done(match choice {
            Choice::Session(session) => daemon.set_item_session(&item, &session, true).await,
            Choice::Item(on) => daemon.add_dependency(&item, &on).await,
        })
    })
}

/// The session strip's Board panel (BD-8, D216): its channel's items with the
/// session's own board items first.
#[must_use]
pub fn session_panel<'w>(workspace: &'w Workspace, session: &Session) -> Vec<&'w Item> {
    let own = Scope::Session {
        channel: session.channel.clone(),
        session: session.id.clone(),
    };
    let channel = Scope::Channel(session.channel.clone());
    let mut panel: Vec<&Item> = workspace.items.iter().filter(|item| item.scope == own).collect();
    panel.extend(workspace.items.iter().filter(|item| item.scope == channel));
    panel
}

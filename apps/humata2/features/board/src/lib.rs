//! Boards (BD-1–10): the global board and each channel's, with columns by
//! status. Items move between columns, open in a drawer, gain dependencies
//! (a cycle is refused, BD-7) and are added with a title.

use std::sync::Arc;

use humata2_clients::model::{Item, ItemId, ItemStatus, Scope, SessionId};
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
/// Presentation retained across restarts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct View {
    pub mode: Mode,
    pub query: String,
    pub filter: Filter,
    pub global_only: bool,
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
    /// The item picking what it depends on.
    pub linking: Option<ItemId>,
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
            linking: None,
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

    /// Whether this item belongs to the current scope and drill-down.
    #[must_use]
    pub fn matches(&self, item: &Item) -> bool {
        let scope = match &self.scope {
            Scope::Global => !self.view.global_only || item.scope == Scope::Global,
            scope => &item.scope == scope,
        };
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
}

/// What happens on a board.
#[derive(Clone, Debug, PartialEq)]
pub enum BoardAction {
    /// The data changed.
    Show(Arc<Workspace>),
    /// Show this board.
    SetScope(Scope),
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
    /// Close the drawer.
    Close,
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
    /// Pick what the drawer's item depends on.
    StartLink,
    /// It depends on this.
    Link(ItemId),
    /// Stop picking.
    CancelLink,
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
                if state.scope != scope {
                    state.view.global_only = false;
                    state.view.filter = Filter::All;
                }
                state.scope = scope;
                state.scope_menu = false;
                state.drawer = None;
                state.linking = None;
            }
            BoardAction::Open(id) => {
                state.drawer = state
                    .workspace
                    .items
                    .iter()
                    .any(|i| i.id == id)
                    .then_some(id);
                state.status_menu = false;
                state.linking = None;
            }
            BoardAction::Close => {
                state.drawer = None;
                state.linking = None;
                state.expanded = false;
            }
            BoardAction::Move(id, status) => {
                state.status_menu = false;
                return Effect::future(async move {
                    BoardAction::Done(daemon.move_item(&id, status).await)
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
                let scope = state.scope.clone();
                let status = state.new_status;
                return Effect::future(async move {
                    let result = match daemon.create_item(scope, &title).await {
                        Ok(id) if status != ItemStatus::Todo => daemon.move_item(&id, status).await,
                        Ok(_) => Ok(()),
                        Err(error) => Err(error),
                    };
                    BoardAction::Done(result)
                });
            }
            BoardAction::CancelNew => state.new_title = None,
            BoardAction::StartLink => state.linking.clone_from(&state.drawer),
            BoardAction::Link(on) => {
                if let Some(item) = state.linking.take() {
                    return Effect::future(async move {
                        BoardAction::Done(daemon.add_dependency(&item, &on).await)
                    });
                }
            }
            BoardAction::CancelLink => state.linking = None,
            BoardAction::OpenSession => {
                if let Some(session) = state.opened().and_then(|item| item.session.clone()) {
                    return Effect::send(BoardAction::Delegate(Delegate::OpenSession(session)));
                }
            }
            BoardAction::Done(result) => match result {
                Ok(()) => state.error = None,
                Err(error) => state.error = Some(error.to_string()),
            },
            BoardAction::Delegate(_) => {}
        }
        Effect::none()
    }
}

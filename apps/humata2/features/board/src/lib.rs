//! Boards (BD-1–10): the global board and each channel's, with columns by
//! status. Items move between columns, open in a drawer, gain dependencies
//! (a cycle is refused, BD-7) and are added with a title.

use std::sync::Arc;

use humata2_clients::model::{Item, ItemId, ItemStatus, Scope, SessionId};
use humata2_clients::{Clients, DaemonError, Workspace};
use tca::{Effect, Reducer};

/// The board's state.
#[derive(Clone, Debug, PartialEq)]
pub struct BoardState {
    /// Where items come from.
    pub workspace: Arc<Workspace>,
    /// Which board shows.
    pub scope: Scope,
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
            .filter(|item| item.scope == self.scope && item.status == status)
            .collect()
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
            BoardAction::SetScope(scope) => {
                state.scope = scope;
                state.drawer = None;
                state.linking = None;
            }
            BoardAction::Open(id) => {
                state.drawer = Some(id);
                state.linking = None;
            }
            BoardAction::Close => {
                state.drawer = None;
                state.linking = None;
            }
            BoardAction::Move(id, status) => {
                return Effect::future(async move {
                    BoardAction::Done(daemon.move_item(&id, status).await)
                });
            }
            BoardAction::StartNew => state.new_title = Some(String::new()),
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
                return Effect::future(async move {
                    BoardAction::Done(daemon.create_item(scope, &title).await.map(|_| ()))
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

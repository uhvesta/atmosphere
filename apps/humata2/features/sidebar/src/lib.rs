//! The sidebar (NV-2, CH-16, NV-18–23): pinned sessions, then each channel
//! with its session tree. Channels and sessions with forks fold; Expand all
//! and Collapse all open or close every one. The tree is flattened into
//! [`Row`]s, recomputed only when the data or folding changes, so the view
//! can draw just the rows on screen however many there are.
//!
//! The keyboard cursor moves over the rows (↑ ↓ Home End, ← folds or goes to
//! the parent, → unfolds or goes to the first child, Enter opens). Opening
//! is the parent's business: the sidebar only says what to open.
//!
//! The Desk row lists Desk sessions under it on every page (DK-1, D793): the
//! session you are on and the ones used last, up to a cap, drawn in the
//! order they were started, then a "more" row for the rest. It folds like a
//! channel.

use std::collections::BTreeSet;
use std::sync::Arc;

use humata2_clients::Workspace;
use humata2_clients::model::{Agent, ChannelId, SessionId, Status};
use serde::{Deserialize, Serialize};
use tca::{Effect, Reducer};

/// Which row a key or click is about. The host's pins are not rows here:
/// the Pinned section draws them from the host's list.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RowKey {
    /// A channel.
    Channel(ChannelId),
    /// A session in its channel's tree.
    Session(SessionId),
    /// The Desk row under a channel that has open Desk sessions (CD-2): a
    /// stop of its own for the keyboard, between the channel and its Desk
    /// sessions.
    ChannelDesk(ChannelId),
    /// The Desk row under Board, a stop while there are open Desk sessions
    /// (D793): it folds them, and Enter opens the Desk page.
    Desk,
    /// The last row under the Desk, counting the Desk sessions not listed;
    /// it opens the Desk page (D793).
    DeskMore,
}

/// What a row shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowKind {
    /// A section title: "Channels".
    Section(&'static str),
    /// A channel.
    Channel {
        /// Its name, without `#`.
        name: String,
        /// How many sessions it has.
        sessions: usize,
        /// How many of them need the person.
        needs_you: usize,
    },
    /// A session.
    Session {
        /// Its title.
        title: String,
        /// What it is doing.
        status: Status,
        /// Its agent.
        agent: Agent,
        /// Lines not read yet.
        unread: u32,
        /// Marked very important.
        vip: bool,
        /// Whether it was just reattached, resumed or needs recovering.
        recovered: bool,
    },
}

/// One line of the flattened tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// Its key; sections have none and can't be selected.
    pub key: Option<RowKey>,
    /// How deep it sits: channels 0, their sessions 1, forks deeper.
    pub depth: u16,
    /// What it shows.
    pub kind: RowKind,
    /// Whether it is open, when it can fold.
    pub expanded: Option<bool>,
}

/// What the person folded, saved between launches.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folding {
    /// Channels that are folded.
    pub channels: BTreeSet<ChannelId>,
    /// Sessions whose forks are folded.
    pub sessions: BTreeSet<SessionId>,
    /// Whether the Desk is folded (D793). Folding saved before the Desk
    /// could fold reads as open.
    #[serde(default)]
    pub desk: bool,
}

/// The sidebar's state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SidebarState {
    /// What it lists.
    pub workspace: Arc<Workspace>,
    /// What is folded.
    pub folding: Folding,
    /// The rows to draw, top to bottom.
    pub rows: Arc<Vec<Row>>,
    /// The Desk sessions to draw under the Desk row, on every page (DK-1,
    /// D793): the one the main area shows and the ones used last, up to
    /// `desk_limit`, in the order they were started, so a row doesn't move
    /// when its session is used (D148). Empty while the Desk is folded. Each
    /// is a session row at depth 1 with no forks to fold (DK-12). ↑ and ↓
    /// walk the Desk row, these and the "more" row before the channels.
    pub desk: Arc<Vec<Row>>,
    /// How many open Desk sessions `desk` leaves out, for the "more" row;
    /// 0 while the Desk is folded.
    pub desk_more: usize,
    /// How many Desk sessions are open. The Desk row folds, and is a stop
    /// for the keyboard, only while there are some.
    pub desk_open: usize,
    /// At most how many Desk sessions `desk` lists; `None` lists them all.
    /// The app sets it from Settings.
    pub desk_limit: Option<usize>,
    /// Each channel's open Desk sessions, in the order they were started
    /// (CD-2): the view draws a Desk row under the channel, above its
    /// sessions, while the channel has any. Each is a session row at depth
    /// 1.
    pub channel_desks: Arc<std::collections::BTreeMap<ChannelId, Vec<Row>>>,
    /// The keyboard cursor.
    pub cursor: Option<RowKey>,
    /// The session or channel the main area shows, highlighted.
    pub current: Option<RowKey>,
}

/// A keyboard move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    /// ↑
    Up,
    /// ↓
    Down,
    /// Home
    First,
    /// End
    Last,
    /// ←: fold, or go to the parent.
    Out,
    /// →: unfold, or go to the first child.
    In,
}

/// What the sidebar asks its parent to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delegate {
    /// Open a channel's page.
    OpenChannel(ChannelId),
    /// Open a session.
    OpenSession(SessionId),
    /// Open a channel's page on its Desk tab.
    OpenChannelDesk(ChannelId),
    /// Open the Desk page.
    OpenDesk,
}

/// What happens in the sidebar.
#[derive(Clone, Debug, PartialEq)]
pub enum SidebarAction {
    /// The data changed.
    Show(Arc<Workspace>),
    /// Saved folding came back at launch.
    Restore(Folding),
    /// The main area now shows this.
    SetCurrent(Option<RowKey>),
    /// At most this many Desk sessions under the Desk row; `None` for all.
    SetDeskLimit(Option<usize>),
    /// A row's chevron was clicked.
    Toggle(RowKey),
    /// Open every channel and fork tree (⌥⌘→).
    ExpandAll,
    /// Fold every channel and fork tree (⌥⌘←).
    CollapseAll,
    /// A row was clicked: it gets the cursor and opens.
    Activate(RowKey),
    /// A key moved the cursor.
    Move(Move),
    /// Enter: open the row under the cursor.
    OpenCursor,
    /// For the parent.
    Delegate(Delegate),
}

/// The sidebar's reducer.
#[derive(Clone, Copy, Debug, Default)]
pub struct Sidebar;

impl Reducer for Sidebar {
    type State = SidebarState;
    type Action = SidebarAction;

    fn reduce(&self, state: &mut SidebarState, action: SidebarAction) -> Effect<SidebarAction> {
        match action {
            SidebarAction::Show(workspace) => {
                state.workspace = workspace;
                state.rebuild();
            }
            SidebarAction::Restore(folding) => {
                state.folding = folding;
                state.rebuild();
            }
            SidebarAction::SetCurrent(current) => {
                // Opening a channel from its Desk row leaves the cursor there.
                let from_desk_row = matches!(
                    (&state.cursor, &current),
                    (Some(RowKey::ChannelDesk(from)), Some(RowKey::Channel(to))) if from == to
                );
                if current.is_some() && !from_desk_row {
                    state.cursor.clone_from(&current);
                }
                state.current = current;
                // The session shown always has a row under the Desk.
                state.rebuild_desk();
            }
            SidebarAction::SetDeskLimit(limit) => {
                state.desk_limit = limit;
                state.rebuild_desk();
            }
            SidebarAction::Toggle(key) => {
                state.toggle(&key);
                state.rebuild();
            }
            SidebarAction::ExpandAll => {
                state.folding = Folding::default();
                state.rebuild();
            }
            SidebarAction::CollapseAll => {
                state.folding.channels = state
                    .workspace
                    .channels
                    .iter()
                    .map(|channel| channel.id.clone())
                    .collect();
                state.folding.sessions = state
                    .workspace
                    .sessions
                    .iter()
                    .filter(|session| state.workspace.forks(&session.id).next().is_some())
                    .map(|session| session.id.clone())
                    .collect();
                state.fold_desk();
                state.rebuild();
            }
            SidebarAction::Activate(key) => {
                state.cursor = Some(key.clone());
                return open(&key);
            }
            SidebarAction::Move(movement) => state.step(movement),
            SidebarAction::OpenCursor => {
                if let Some(key) = state.cursor.clone() {
                    return open(&key);
                }
            }
            SidebarAction::Delegate(_) => {}
        }
        Effect::none()
    }
}

fn open(key: &RowKey) -> Effect<SidebarAction> {
    Effect::send(SidebarAction::Delegate(match key {
        RowKey::Channel(id) => Delegate::OpenChannel(id.clone()),
        RowKey::Session(id) => Delegate::OpenSession(id.clone()),
        RowKey::ChannelDesk(id) => Delegate::OpenChannelDesk(id.clone()),
        RowKey::Desk | RowKey::DeskMore => Delegate::OpenDesk,
    }))
}

impl SidebarState {
    /// A sidebar listing `workspace`, everything unfolded.
    #[must_use]
    pub fn new(workspace: Arc<Workspace>) -> Self {
        let mut state = Self {
            workspace,
            ..Self::default()
        };
        state.rebuild();
        state
    }

    /// The index of the row with `key`.
    #[must_use]
    pub fn index_of(&self, key: &RowKey) -> Option<usize> {
        self.rows
            .iter()
            .position(|row| row.key.as_ref() == Some(key))
    }

    fn toggle(&mut self, key: &RowKey) {
        match key {
            RowKey::Channel(id) => {
                if !self.folding.channels.remove(id) {
                    self.folding.channels.insert(id.clone());
                }
            }
            RowKey::Session(id) => {
                if !self.folding.sessions.remove(id) {
                    self.folding.sessions.insert(id.clone());
                }
            }
            // The Desk row folds with its channel.
            RowKey::ChannelDesk(_) | RowKey::DeskMore => {}
            RowKey::Desk => {
                if self.folding.desk {
                    self.folding.desk = false;
                } else {
                    self.fold_desk();
                }
            }
        }
    }

    /// Folds the Desk; a cursor on a row under it goes to the Desk row.
    fn fold_desk(&mut self) {
        self.folding.desk = true;
        if self
            .cursor
            .as_ref()
            .is_some_and(|key| self.under_desk(key))
        {
            self.cursor = Some(RowKey::Desk);
        }
    }

    /// Whether `key` is a row drawn under the Desk row.
    fn under_desk(&self, key: &RowKey) -> bool {
        *key == RowKey::DeskMore || self.desk.iter().any(|row| row.key.as_ref() == Some(key))
    }

    fn selectable(&self) -> Vec<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.key.is_some())
            .map(|(index, _)| index)
            .collect()
    }

    /// Where ↑ or ↓ goes when there are Desk rows and the cursor is on a
    /// row: the neighbour in drawing order (the Desk row, its sessions and
    /// its "more" row, then the channels, each open channel's own Desk
    /// sessions right under it and above its sessions, CD-2). `None` leaves
    /// the move to the tree alone.
    fn walk_with_desk(&self, movement: Move) -> Option<RowKey> {
        let desk = self.desk_open > 0;
        if !desk && self.channel_desks.is_empty() {
            return None;
        }
        let mut order: Vec<RowKey> = Vec::new();
        for row in self.rows.iter() {
            if desk && matches!(row.kind, RowKind::Section("Channels")) {
                order.push(RowKey::Desk);
                order.extend(self.desk.iter().filter_map(|desk| desk.key.clone()));
                if self.desk_more > 0 {
                    order.push(RowKey::DeskMore);
                }
            }
            order.extend(row.key.clone());
            if let Some(RowKey::Channel(id)) = &row.key
                && !self.folding.channels.contains(id)
                && let Some(desks) = self.channel_desks.get(id)
                && !desks.is_empty()
            {
                order.push(RowKey::ChannelDesk(id.clone()));
                order.extend(desks.iter().filter_map(|desk| desk.key.clone()));
            }
        }
        let at = order
            .iter()
            .position(|key| Some(key) == self.cursor.as_ref())?;
        let next = match movement {
            Move::Down => (at + 1).min(order.len() - 1),
            _ => at.saturating_sub(1),
        };
        order.get(next).cloned()
    }

    fn step(&mut self, movement: Move) {
        if matches!(movement, Move::Up | Move::Down)
            && let Some(key) = self.walk_with_desk(movement)
        {
            self.cursor = Some(key);
            return;
        }
        // On the Desk row, ← folds and → unfolds or goes to its first row;
        // under it, ← goes back to the Desk row.
        if self.cursor == Some(RowKey::Desk) {
            match movement {
                Move::Out => {
                    if !self.folding.desk {
                        self.fold_desk();
                        self.rebuild_desk();
                    }
                    return;
                }
                Move::In => {
                    if self.folding.desk {
                        self.folding.desk = false;
                        self.rebuild_desk();
                    } else if let Some(first) = self
                        .desk
                        .first()
                        .and_then(|row| row.key.clone())
                        .or_else(|| (self.desk_more > 0).then_some(RowKey::DeskMore))
                    {
                        self.cursor = Some(first);
                    }
                    return;
                }
                _ => {}
            }
        }
        if let Some(key) = &self.cursor
            && self.under_desk(key)
            && matches!(movement, Move::Out | Move::In)
        {
            if movement == Move::Out {
                self.cursor = Some(RowKey::Desk);
            }
            return;
        }
        // On a channel's Desk row, → goes into its Desk sessions and ← back
        // to the channel.
        if let Some(RowKey::ChannelDesk(id)) = &self.cursor {
            match movement {
                Move::Out => {
                    self.cursor = Some(RowKey::Channel(id.clone()));
                    return;
                }
                Move::In => {
                    if let Some(first) = self
                        .channel_desks
                        .get(id)
                        .and_then(|desks| desks.first())
                        .and_then(|desk| desk.key.clone())
                    {
                        self.cursor = Some(first);
                    }
                    return;
                }
                _ => {}
            }
        }
        let selectable = self.selectable();
        let Some(&first) = selectable.first() else {
            return;
        };
        let at = self
            .cursor
            .as_ref()
            .and_then(|key| self.index_of(key))
            .and_then(|index| selectable.iter().position(|&row| row == index));
        let target = match (movement, at) {
            (Move::First, _) | (Move::Down | Move::Up | Move::In | Move::Out, None) => Some(first),
            (Move::Last, _) => selectable.last().copied(),
            (Move::Down, Some(at)) => selectable.get(at + 1).copied().or(Some(selectable[at])),
            (Move::Up, Some(at)) => Some(selectable[at.saturating_sub(1)]),
            (Move::In, Some(at)) => {
                let index = selectable[at];
                let row = &self.rows[index];
                match row.expanded {
                    Some(false) => {
                        if let Some(key) = row.key.clone() {
                            self.toggle(&key);
                            self.rebuild();
                        }
                        self.index_of_cursor()
                    }
                    Some(true) => self
                        .rows
                        .get(index + 1)
                        .filter(|next| next.depth > row.depth)
                        .map(|_| index + 1)
                        .or(Some(index)),
                    None => Some(index),
                }
            }
            (Move::Out, Some(at)) => {
                let index = selectable[at];
                let row = &self.rows[index];
                if row.expanded == Some(true) {
                    if let Some(key) = row.key.clone() {
                        self.toggle(&key);
                        self.rebuild();
                    }
                    self.index_of_cursor()
                } else {
                    let depth = row.depth;
                    (0..index)
                        .rev()
                        .find(|&before| {
                            self.rows[before].depth < depth && self.rows[before].key.is_some()
                        })
                        .or(Some(index))
                }
            }
        };
        if let Some(target) = target {
            self.cursor = self.rows[target].key.clone();
        }
    }

    /// Whether a row with `key` is listed, in the tree or under the Desk.
    fn lists(&self, key: &RowKey) -> bool {
        match key {
            RowKey::ChannelDesk(id) => {
                return self
                    .channel_desks
                    .get(id)
                    .is_some_and(|desks| !desks.is_empty());
            }
            RowKey::Desk => return self.desk_open > 0,
            RowKey::DeskMore => return self.desk_more > 0,
            RowKey::Channel(_) | RowKey::Session(_) => {}
        }
        self.index_of(key).is_some()
            || self
                .desk
                .iter()
                .any(|row| row.key.as_ref() == Some(key))
            || self
                .channel_desks
                .values()
                .flatten()
                .any(|row| row.key.as_ref() == Some(key))
    }

    fn index_of_cursor(&self) -> Option<usize> {
        self.cursor.as_ref().and_then(|key| self.index_of(key))
    }

    /// Flattens the tree again. Called only when data or folding changed.
    fn rebuild(&mut self) {
        let workspace = &self.workspace;
        let mut rows = Vec::new();
        rows.push(Row {
            key: None,
            depth: 0,
            kind: RowKind::Section("Channels"),
            expanded: None,
        });
        // Children by parent, once, so deep trees stay linear.
        let mut children: std::collections::HashMap<&SessionId, Vec<usize>> =
            std::collections::HashMap::new();
        let mut roots: std::collections::HashMap<&ChannelId, Vec<usize>> =
            std::collections::HashMap::new();
        for (index, session) in workspace.sessions.iter().enumerate() {
            if session.archived {
                continue;
            }
            match &session.parent {
                Some(parent)
                    if workspace
                        .session(parent)
                        .is_some_and(|parent| !parent.archived) =>
                {
                    children.entry(parent).or_default().push(index);
                }
                _ => roots.entry(&session.channel).or_default().push(index),
            }
        }
        for channel in &workspace.channels {
            let members: Vec<_> = workspace
                .sessions
                .iter()
                .filter(|session| session.channel == channel.id && !session.archived)
                .collect();
            let folded = self.folding.channels.contains(&channel.id);
            rows.push(Row {
                key: Some(RowKey::Channel(channel.id.clone())),
                depth: 0,
                kind: RowKind::Channel {
                    name: channel.name.clone(),
                    sessions: members.len(),
                    needs_you: members
                        .iter()
                        .filter(|session| session.status == Status::NeedsYou)
                        .count(),
                },
                expanded: (!members.is_empty()).then_some(!folded),
            });
            if folded {
                continue;
            }
            let mut stack: Vec<(usize, u16)> = roots
                .get(&channel.id)
                .map(|roots| roots.iter().rev().map(|&index| (index, 1)).collect())
                .unwrap_or_default();
            while let Some((index, depth)) = stack.pop() {
                let session = &workspace.sessions[index];
                let kids = children.get(&session.id);
                let has_kids = kids.is_some_and(|kids| !kids.is_empty());
                let open = !self.folding.sessions.contains(&session.id);
                rows.push(session_row(
                    RowKey::Session(session.id.clone()),
                    depth,
                    session,
                    has_kids.then_some(open),
                ));
                if has_kids
                    && open
                    && let Some(kids) = kids
                {
                    stack.extend(kids.iter().rev().map(|&kid| (kid, depth + 1)));
                }
            }
        }
        self.rows = Arc::new(rows);
        let mut channel_desks: std::collections::BTreeMap<ChannelId, Vec<Row>> =
            std::collections::BTreeMap::new();
        for session in &workspace.sessions {
            let Some(record) = workspace.desk.channel_sessions.get(&session.id) else {
                continue;
            };
            if session.archived || record.desk.closed.is_some() {
                continue;
            }
            channel_desks
                .entry(record.channel.clone())
                .or_default()
                .push(session_row(RowKey::Session(session.id.clone()), 1, session, None));
        }
        self.channel_desks = Arc::new(channel_desks);
        self.rebuild_desk();
    }

    /// Picks the rows under the Desk again (D793), then drops a cursor on a
    /// row that is gone. Called when the data, the fold, the cap or the
    /// session shown changed.
    fn rebuild_desk(&mut self) {
        let workspace = &self.workspace;
        let open: Vec<(&humata2_clients::model::Session, u64)> = workspace
            .sessions
            .iter()
            .filter(|session| !session.archived)
            .filter_map(|session| {
                let record = workspace.desk.sessions.get(&session.id)?;
                record
                    .closed
                    .is_none()
                    .then_some((session, record.last_used.order))
            })
            .collect();
        self.desk_open = open.len();
        if self.folding.desk {
            self.desk = Arc::new(Vec::new());
            self.desk_more = 0;
        } else {
            let current = match &self.current {
                Some(RowKey::Session(id)) => Some(id),
                _ => None,
            };
            let keep = desk_pick(&open, current, self.desk_limit);
            let rows: Vec<Row> = open
                .iter()
                .zip(&keep)
                .filter(|(_, kept)| **kept)
                .map(|((session, _), _)| {
                    session_row(RowKey::Session(session.id.clone()), 1, session, None)
                })
                .collect();
            self.desk_more = open.len() - rows.len();
            self.desk = Arc::new(rows);
        }
        if self
            .cursor
            .as_ref()
            .is_some_and(|key| !self.lists(key))
        {
            self.cursor = None;
        }
    }
}

/// Which of the open Desk sessions, in start order with how recently each
/// was used, get a row under a cap of `limit` (D793): the one `current`
/// names, then the most recently used. `None` keeps them all.
fn desk_pick(
    open: &[(&humata2_clients::model::Session, u64)],
    current: Option<&SessionId>,
    limit: Option<usize>,
) -> Vec<bool> {
    let Some(limit) = limit.filter(|limit| *limit < open.len()) else {
        return vec![true; open.len()];
    };
    let is_current = |index: usize| Some(&open[index].0.id) == current;
    let mut by_use: Vec<usize> = (0..open.len()).collect();
    by_use.sort_by(|&a, &b| {
        is_current(b)
            .cmp(&is_current(a))
            .then_with(|| open[b].1.cmp(&open[a].1))
            .then_with(|| open[a].0.id.cmp(&open[b].0.id))
    });
    let mut keep = vec![false; open.len()];
    for index in by_use.into_iter().take(limit.max(1)) {
        keep[index] = true;
    }
    keep
}

fn session_row(
    key: RowKey,
    depth: u16,
    session: &humata2_clients::model::Session,
    expanded: Option<bool>,
) -> Row {
    Row {
        key: Some(key),
        depth,
        kind: RowKind::Session {
            title: session.shown_title(),
            status: session.status,
            agent: session.agent,
            unread: session.unread,
            vip: session.vip,
            recovered: session.recovery.is_some(),
        },
        expanded,
    }
}

//! The sidebar (NV-2, CH-16, NV-18–23): pinned sessions, then each channel
//! with its session tree. Channels and sessions with forks fold; Expand all
//! and Collapse all open or close every one. The tree is flattened into
//! [`Row`]s, recomputed only when the data or folding changes, so the view
//! can draw just the rows on screen however many there are.
//!
//! The keyboard cursor moves over the rows (↑ ↓ Home End, ← folds or goes to
//! the parent, → unfolds or goes to the first child, Enter opens). Opening
//! is the parent's business: the sidebar only says what to open.

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
    /// The open Desk sessions, in the order they were started, for the view
    /// to draw under its Desk row (DK-1). A row doesn't move when its session
    /// is used (D148). Each is a session row at depth 1 with no forks to fold
    /// (DK-12).
    pub desk: Arc<Vec<Row>>,
    /// Whether the view is drawing the Desk rows right now (only while the
    /// Desk page or a Desk session shows). The app sets it; while it is
    /// set, ↑ and ↓ walk through `desk` too, before the channels, in the
    /// order they are drawn.
    pub desk_shown: bool,
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
            RowKey::ChannelDesk(_) => {}
        }
    }

    fn selectable(&self) -> Vec<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.key.is_some())
            .map(|(index, _)| index)
            .collect()
    }

    /// Where ↑ or ↓ goes when Desk rows are showing and the cursor is on a
    /// row: the neighbour in drawing order (the Desk rows, then the
    /// channels, each open channel's own Desk sessions right under it and
    /// above its sessions, CD-2). `None` leaves the move to the tree alone.
    fn walk_with_desk(&self, movement: Move) -> Option<RowKey> {
        let desk_shown = self.desk_shown && !self.desk.is_empty();
        if !desk_shown && self.channel_desks.is_empty() {
            return None;
        }
        let mut order: Vec<RowKey> = Vec::new();
        for row in self.rows.iter() {
            if desk_shown && matches!(row.kind, RowKind::Section("Channels")) {
                order.extend(self.desk.iter().filter_map(|desk| desk.key.clone()));
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
        if let RowKey::ChannelDesk(id) = key {
            return self
                .channel_desks
                .get(id)
                .is_some_and(|desks| !desks.is_empty());
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
        self.desk = Arc::new(
            workspace
                .sessions
                .iter()
                .filter(|session| {
                    !session.archived
                        && workspace
                            .desk
                            .sessions
                            .get(&session.id)
                            .is_some_and(|record| record.closed.is_none())
                })
                .map(|session| session_row(RowKey::Session(session.id.clone()), 1, session, None))
                .collect(),
        );
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
        if self
            .cursor
            .as_ref()
            .is_some_and(|key| !self.lists(key))
        {
            self.cursor = None;
        }
    }
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

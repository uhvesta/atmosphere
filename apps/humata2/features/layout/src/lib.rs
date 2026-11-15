//! The pane workspace (NV-28–NV-35, D74). Each screen is a tree of panes,
//! each showing one thing. Panes split side by side or stacked, move by
//! dropping one on another's edge, swap by dropping on the middle, and
//! close; every change can be undone. What the person arranged is kept per
//! screen and saved across restarts; Reset goes back to the screen's
//! default layout without closing anything, and Undo reset puts the exact
//! previous layout back.
//!
//! The layout policy (Fixed, Simple, Free) is checked in one place,
//! [`Policy::check`], and a layout that doesn't fit a narrower policy shows
//! as the default until the policy widens again, so narrowing never closes
//! anything.

mod tree;

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use humata2_clients::Workspace;
use humata2_clients::model::{ChannelId, Scope, SessionId};
use serde::{Deserialize, Serialize};
use tca::{Effect, Reducer};

pub use crate::tree::{Axis, Direction, Edge, Pane, PaneId, PaneKind, Rect, Shape, Split, Tree};

/// How many layout changes Undo remembers per screen.
pub const UNDO: usize = 50;

/// The most panes a Simple layout holds.
pub const SIMPLE_PANES: usize = 4;

/// How deep a Simple layout splits.
pub const SIMPLE_DEPTH: usize = 2;

/// User-made panes get ids from here up, clear of the defaults' small ids.
const FIRST_ID: u64 = 1_000;

/// A screen with its own layout.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Context {
    /// Home.
    Home,
    /// The Inbox.
    Inbox,
    /// A board.
    Board(Scope),
    /// A channel's page.
    Channel(ChannelId),
    /// A session's page.
    Session(SessionId),
    /// Hosts.
    Hosts,
    /// Files and transfers.
    Files,
    /// Prepared views.
    Views,
    /// Settings.
    Settings,
}

/// How freely panes can be arranged (NV-35, Settings › Layout › Panes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Policy {
    /// Default layouts only: panes show other things but don't split or
    /// move.
    Fixed,
    /// Up to four panes, two levels deep.
    #[default]
    Simple,
    /// Anything.
    Free,
}

impl Policy {
    /// Every policy, for the setting's picker.
    pub const ALL: [Self; 3] = [Self::Fixed, Self::Simple, Self::Free];

    /// Its name in Settings.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Fixed => "Fixed",
            Self::Simple => "Simple",
            Self::Free => "Free",
        }
    }

    /// What it allows, for the setting's description.
    #[must_use]
    pub const fn summary(self) -> &'static str {
        match self {
            Self::Fixed => "Each screen keeps its default panes. Panes can show other things but don't split or move.",
            Self::Simple => "Up to four panes, split two levels deep.",
            Self::Free => "Split and move panes however you like.",
        }
    }

    /// Whether `tree` fits, with `default` the screen's default layout.
    #[must_use]
    pub fn fits(self, tree: &Tree, default: &Tree) -> bool {
        self.limit(tree, default).is_none()
    }

    fn limit(self, tree: &Tree, default: &Tree) -> Option<Refusal> {
        match self {
            Self::Fixed => (tree.shape() != default.shape()).then_some(Refusal::Fixed),
            Self::Simple if tree.count() > SIMPLE_PANES => Some(Refusal::TooMany),
            Self::Simple if tree.depth() > SIMPLE_DEPTH => Some(Refusal::TooDeep),
            Self::Simple | Self::Free => None,
        }
    }

    /// The one place the policy is enforced: whether `change`, giving
    /// `tree`, is allowed on a screen whose default is `default`.
    ///
    /// # Errors
    ///
    /// The [`Refusal`] saying why not and what to do instead.
    pub fn check(self, change: Change, tree: &Tree, default: &Tree) -> Result<(), Refusal> {
        if self == Self::Fixed && change.rearranges() {
            return Err(Refusal::Fixed);
        }
        self.limit(tree, default).map_or(Ok(()), Err)
    }
}

/// What kind of layout change was asked for, for [`Policy::check`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// A pane shows something else.
    Open,
    /// A pane split.
    Split,
    /// A pane moved beside another.
    Move,
    /// Two panes swapped.
    Swap,
    /// A pane closed.
    Close,
    /// A split's shares changed.
    Resize,
}

impl Change {
    const fn rearranges(self) -> bool {
        matches!(self, Self::Split | Self::Move | Self::Swap | Self::Close)
    }
}

/// Why a layout change was refused, said so the person knows what to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The policy is Fixed.
    Fixed,
    /// A Simple layout is full.
    TooMany,
    /// A Simple layout is as deep as it goes.
    TooDeep,
    /// The screen's only pane can't close.
    LastPane,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Fixed => {
                "Panes are set to Fixed, so this screen keeps its default layout. To split or move panes, choose Simple or Free in Settings › Layout › Panes."
            }
            Self::TooMany => {
                "A Simple layout holds four panes. Close one first, or choose Free in Settings › Layout › Panes."
            }
            Self::TooDeep => {
                "A Simple layout splits two levels deep. Drop the pane on the edge of a pane in a shallower split, or choose Free in Settings › Layout › Panes."
            }
            Self::LastPane => {
                "This is the screen's only pane, so it stays open. To show something else here, pick it in the Panes sidebar."
            }
        })
    }
}

/// What is being dropped on a pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A pane's title, dragged.
    Pane(PaneId),
    /// An entry dragged from the Panes sidebar.
    Kind(PaneKind),
}

/// A section of the Panes sidebar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    /// Its title, such as "This session".
    pub title: String,
    /// What it lists.
    pub kinds: Vec<PaneKind>,
}

/// An undo step.
#[derive(Clone, Debug, PartialEq)]
struct Step {
    /// The arrangement before; `None` is the default.
    before: Option<Tree>,
    /// The focus before.
    focus: Option<PaneId>,
    /// Whether it was a Reset, so Undo reset knows it is still there.
    reset: bool,
}

/// One screen's layout.
#[derive(Clone, Debug, Default, PartialEq)]
struct Entry {
    /// What the person arranged; `None` follows the default.
    user: Option<Tree>,
    /// The focused pane.
    focus: Option<PaneId>,
    /// Changes to undo, oldest first.
    undo: Vec<Step>,
}

/// One screen's saved layout.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedContext {
    /// The screen.
    pub context: Context,
    /// What the person arranged; `None` follows the default.
    pub tree: Option<Tree>,
    /// The focused pane.
    pub focus: Option<PaneId>,
}

/// What is saved across restarts. Undo history isn't.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    /// The policy.
    pub policy: Policy,
    /// The next id for a new pane.
    pub next_id: u64,
    /// Each screen arranged or focused.
    pub contexts: Vec<SavedContext>,
}

/// The pane workspace's state.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutState {
    /// The policy.
    pub policy: Policy,
    /// The screen showing.
    pub current: Context,
    workspace: Arc<Workspace>,
    entries: BTreeMap<Context, Entry>,
    next_id: u64,
}

impl Default for LayoutState {
    fn default() -> Self {
        Self {
            policy: Policy::default(),
            current: Context::Home,
            workspace: Arc::default(),
            entries: BTreeMap::new(),
            next_id: FIRST_ID,
        }
    }
}

impl LayoutState {
    /// The default layout for `context` with what the workspace holds now:
    /// a session's agent terminal, with its Diff beside once it has
    /// changes; a channel's sessions; otherwise the screen's own pane.
    #[must_use]
    pub fn default_tree(&self, context: &Context) -> Tree {
        let only = |kind| Tree::pane(PaneId(1), kind);
        match context {
            Context::Session(id) => {
                let agent = only(PaneKind::Agent(id.clone()));
                let changed = self
                    .workspace
                    .diffs
                    .iter()
                    .any(|diff| &diff.session == id && !diff.files.is_empty());
                if changed {
                    let mut tree = Tree::split(
                        Axis::Horizontal,
                        vec![agent, Tree::pane(PaneId(2), PaneKind::Diff(id.clone()))],
                    );
                    tree.resize(&[], &[0.6, 0.4]);
                    tree
                } else {
                    agent
                }
            }
            Context::Channel(id) => only(PaneKind::Sessions(id.clone())),
            Context::Home => only(PaneKind::Home),
            Context::Inbox => only(PaneKind::Inbox),
            Context::Board(scope) => only(PaneKind::Board(scope.clone())),
            Context::Hosts => only(PaneKind::Hosts),
            Context::Files => only(PaneKind::Files(None)),
            Context::Views => only(PaneKind::Views),
            Context::Settings => only(PaneKind::Settings),
        }
    }

    /// The layout showing on `context`: the person's, if it fits the
    /// policy, else the default.
    #[must_use]
    pub fn tree_for(&self, context: &Context) -> Tree {
        let default = self.default_tree(context);
        match self.entries.get(context).and_then(|entry| entry.user.as_ref()) {
            Some(user) if self.policy.fits(user, &default) => user.clone(),
            _ => default,
        }
    }

    /// The layout showing now.
    #[must_use]
    pub fn tree(&self) -> Tree {
        self.tree_for(&self.current)
    }

    /// Whether the current screen shows the person's own arrangement rather
    /// than its default.
    #[must_use]
    pub fn arranged(&self) -> bool {
        let default = self.default_tree(&self.current);
        self.entries
            .get(&self.current)
            .and_then(|entry| entry.user.as_ref())
            .is_some_and(|user| self.policy.fits(user, &default))
    }

    /// Whether the current screen has a layout change to undo.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        self.entries
            .get(&self.current)
            .is_some_and(|entry| !entry.undo.is_empty())
    }

    /// Whether the last change on the current screen was a Reset that can
    /// still be undone.
    #[must_use]
    pub fn can_undo_reset(&self) -> bool {
        self.entries
            .get(&self.current)
            .and_then(|entry| entry.undo.last())
            .is_some_and(|step| step.reset)
    }

    /// The focused pane on the current screen.
    #[must_use]
    pub fn focus(&self) -> Option<PaneId> {
        let tree = self.tree();
        self.entries
            .get(&self.current)
            .and_then(|entry| entry.focus)
            .filter(|id| tree.find(*id).is_some())
            .or_else(|| tree.panes().first().map(|pane| pane.id))
    }

    /// What the Panes sidebar lists for the current screen (NV-29): the
    /// session's own panes first on a session, then what every screen can
    /// open, then each channel's sessions.
    #[must_use]
    pub fn catalogue(&self) -> Vec<Group> {
        let workspace = &self.workspace;
        let mut groups = Vec::new();
        if let Context::Session(id) = &self.current {
            let mut kinds = vec![PaneKind::Agent(id.clone())];
            if let Some(session) = workspace.session(id) {
                for n in 0..session.shells.len() {
                    kinds.push(PaneKind::Shell(id.clone(), u32::try_from(n).unwrap_or(u32::MAX)));
                }
                kinds.push(PaneKind::Diff(id.clone()));
                kinds.push(PaneKind::Files(Some(id.clone())));
                kinds.push(PaneKind::Board(Scope::Channel(session.channel.clone())));
                kinds.push(PaneKind::Info(id.clone()));
            }
            groups.push(Group {
                title: "This session".to_owned(),
                kinds,
            });
        }
        groups.push(Group {
            title: "Everywhere".to_owned(),
            kinds: vec![
                PaneKind::Home,
                PaneKind::Inbox,
                PaneKind::Board(Scope::Global),
                PaneKind::Journal,
                PaneKind::Hosts,
                PaneKind::Files(None),
            ],
        });
        for channel in &workspace.channels {
            let kinds: Vec<PaneKind> = workspace
                .sessions
                .iter()
                .filter(|session| session.channel == channel.id && !session.archived)
                .map(|session| PaneKind::Agent(session.id.clone()))
                .collect();
            groups.push(Group {
                title: format!("#{}", channel.name),
                kinds: std::iter::once(PaneKind::Sessions(channel.id.clone()))
                    .chain(kinds)
                    .collect(),
            });
        }
        groups
    }

    /// What is saved now.
    #[must_use]
    pub fn saved(&self) -> Saved {
        Saved {
            policy: self.policy,
            next_id: self.next_id,
            contexts: self
                .entries
                .iter()
                .filter(|(_, entry)| entry.user.is_some() || entry.focus.is_some())
                .map(|(context, entry)| SavedContext {
                    context: context.clone(),
                    tree: entry.user.clone(),
                    focus: entry.focus,
                })
                .collect(),
        }
    }

    fn restore(&mut self, saved: Saved) {
        self.policy = saved.policy;
        let highest = saved
            .contexts
            .iter()
            .filter_map(|context| context.tree.as_ref().map(Tree::max_id))
            .max()
            .unwrap_or(0);
        self.next_id = saved.next_id.max(FIRST_ID).max(highest + 1);
        self.entries = saved
            .contexts
            .into_iter()
            .map(|saved| {
                (
                    saved.context,
                    Entry {
                        user: saved.tree,
                        focus: saved.focus,
                        undo: Vec::new(),
                    },
                )
            })
            .collect();
    }

    fn new_id(&mut self) -> PaneId {
        let id = PaneId(self.next_id);
        self.next_id += 1;
        id
    }

    fn entry(&mut self) -> &mut Entry {
        self.entries.entry(self.current.clone()).or_default()
    }

    fn set_focus(&mut self, id: PaneId) {
        if self.tree().find(id).is_some() {
            self.entry().focus = Some(id);
        }
    }

    /// Checks `change` giving `tree` against the policy and, if allowed,
    /// makes it the current screen's arrangement, undoable.
    fn commit(&mut self, change: Change, mut tree: Tree, focus: Option<PaneId>) -> Result<(), Refusal> {
        tree.normalize();
        let default = self.default_tree(&self.current);
        self.policy.check(change, &tree, &default)?;
        let shown = self.tree();
        if tree == shown {
            if let Some(focus) = focus {
                self.set_focus(focus);
            }
            return Ok(());
        }
        let user = (tree != default).then_some(tree);
        let entry = self.entry();
        let step = Step {
            before: entry.user.clone(),
            focus: entry.focus,
            reset: false,
        };
        entry.undo.push(step);
        if entry.undo.len() > UNDO {
            entry.undo.remove(0);
        }
        entry.user = user;
        if let Some(focus) = focus {
            entry.focus = Some(focus);
        }
        Ok(())
    }

    /// A new pane for splitting `pane`: a new shell for a terminal, the
    /// same thing again otherwise.
    fn copy_of(&mut self, tree: &Tree, pane: &Pane) -> Pane {
        let kind = match &pane.kind {
            PaneKind::Agent(id) | PaneKind::Shell(id, _) => {
                let open = self
                    .workspace
                    .session(id)
                    .map_or(0, |session| session.shells.len());
                let shown = tree
                    .panes()
                    .iter()
                    .filter_map(|pane| match &pane.kind {
                        PaneKind::Shell(session, n) if session == id => Some(*n + 1),
                        _ => None,
                    })
                    .max()
                    .unwrap_or(0);
                PaneKind::Shell(id.clone(), u32::try_from(open).unwrap_or(u32::MAX).max(shown))
            }
            kind => kind.clone(),
        };
        Pane {
            id: self.new_id(),
            kind,
        }
    }

    fn split_focused(&mut self, axis: Axis) -> Result<(), Refusal> {
        let mut tree = self.tree();
        let Some(focus) = self.focus().and_then(|id| tree.find(id).cloned()) else {
            return Ok(());
        };
        let new = self.copy_of(&tree, &focus);
        let id = new.id;
        let edge = match axis {
            Axis::Horizontal => Edge::Right,
            Axis::Vertical => Edge::Bottom,
        };
        tree.insert(focus.id, new, edge);
        self.commit(Change::Split, tree, Some(id))
    }

    fn open(&mut self, kind: PaneKind) -> Result<(), Refusal> {
        let mut tree = self.tree();
        if let Some(id) = tree.showing(&kind) {
            self.set_focus(id);
            return Ok(());
        }
        let Some(focus) = self.focus() else {
            return Ok(());
        };
        let new = Pane {
            id: self.new_id(),
            kind,
        };
        let id = new.id;
        tree.replace(focus, new);
        self.commit(Change::Open, tree, Some(id))
    }

    fn drop_on(&mut self, source: Source, target: PaneId, edge: Edge) -> Result<(), Refusal> {
        let mut tree = self.tree();
        if tree.find(target).is_none() {
            return Ok(());
        }
        let moving = match source {
            Source::Pane(id) => tree.find(id).cloned(),
            Source::Kind(kind) => match tree.showing(&kind) {
                Some(id) => tree.find(id).cloned(),
                None => {
                    let new = Pane {
                        id: self.new_id(),
                        kind,
                    };
                    if edge == Edge::Centre {
                        let id = new.id;
                        tree.replace(target, new);
                        return self.commit(Change::Open, tree, Some(id));
                    }
                    let id = new.id;
                    tree.insert(target, new, edge);
                    return self.commit(Change::Split, tree, Some(id));
                }
            },
        };
        let Some(moving) = moving else {
            return Ok(());
        };
        if moving.id == target {
            return Ok(());
        }
        let id = moving.id;
        if edge == Edge::Centre {
            tree.swap(id, target);
            return self.commit(Change::Swap, tree, Some(id));
        }
        if tree.remove(id).is_none() {
            return Ok(());
        }
        tree.insert(target, moving, edge);
        self.commit(Change::Move, tree, Some(id))
    }

    fn close(&mut self, id: PaneId) -> Result<(), Refusal> {
        let mut tree = self.tree();
        if tree.find(id).is_none() {
            return Ok(());
        }
        let next = tree
            .neighbour(id, Direction::Left)
            .or_else(|| tree.neighbour(id, Direction::Up))
            .or_else(|| tree.neighbour(id, Direction::Right))
            .or_else(|| tree.neighbour(id, Direction::Down));
        if tree.remove(id).is_none() {
            return Err(Refusal::LastPane);
        }
        self.commit(Change::Close, tree, next)
    }

    fn resize(&mut self, path: &[usize], sizes: &[f32]) -> Result<(), Refusal> {
        let mut tree = self.tree();
        if !tree.resize(path, sizes) {
            return Ok(());
        }
        let default = self.default_tree(&self.current);
        self.policy.check(Change::Resize, &tree, &default)?;
        // Dragging a divider isn't an undo step of its own: it lands on the
        // arrangement as it is.
        let user = (tree != default).then_some(tree);
        self.entry().user = user;
        Ok(())
    }

    fn undo(&mut self) -> bool {
        let entry = self.entry();
        let Some(step) = entry.undo.pop() else {
            return false;
        };
        entry.user = step.before;
        entry.focus = step.focus;
        true
    }

    fn reset(&mut self) -> bool {
        let entry = self.entry();
        if entry.user.is_none() {
            return false;
        }
        let step = Step {
            before: entry.user.take(),
            focus: entry.focus.take(),
            reset: true,
        };
        entry.undo.push(step);
        if entry.undo.len() > UNDO {
            entry.undo.remove(0);
        }
        true
    }
}

/// Something for the parent to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delegate {
    /// A change was refused: say why.
    Refused(Refusal),
    /// The layout was reset: offer Undo reset.
    WasReset,
}

/// What can happen to the pane workspace.
#[derive(Clone, Debug, PartialEq)]
pub enum LayoutAction {
    /// The data changed.
    Show(Arc<Workspace>),
    /// What the last run saved came back.
    Restore(Saved),
    /// This screen shows now.
    Enter(Context),
    /// A pane was clicked or tabbed to.
    Focus(PaneId),
    /// ⌘⌥ and an arrow.
    FocusToward(Direction),
    /// ⌘\ (side by side) or ⇧⌘\ (stacked): split the focused pane.
    SplitFocused(Axis),
    /// A Panes sidebar entry was clicked: show it in the focused pane's
    /// place, or focus the pane already showing it.
    Open(PaneKind),
    /// Something was dropped on a pane.
    Drop {
        /// What.
        source: Source,
        /// On which pane.
        target: PaneId,
        /// Where on it.
        edge: Edge,
    },
    /// A pane's ✕.
    Close(PaneId),
    /// ⌘W.
    CloseFocused,
    /// A divider was dragged: the split at `path` gets these shares.
    Resize {
        /// Child indices from the root.
        path: Vec<usize>,
        /// Each child's share.
        sizes: Vec<f32>,
    },
    /// ⌘Z outside a text field.
    Undo,
    /// Reset layout.
    Reset,
    /// The toast's Undo reset.
    UndoReset,
    /// Settings › Layout › Panes.
    SetPolicy(Policy),
    /// For the parent.
    Delegate(Delegate),
}

/// The pane workspace's reducer.
#[derive(Clone, Copy, Debug, Default)]
pub struct Layout;

impl Reducer for Layout {
    type State = LayoutState;
    type Action = LayoutAction;

    fn reduce(&self, state: &mut LayoutState, action: LayoutAction) -> Effect<LayoutAction> {
        let outcome = match action {
            LayoutAction::Show(workspace) => {
                state.workspace = workspace;
                Ok(())
            }
            LayoutAction::Restore(saved) => {
                state.restore(saved);
                Ok(())
            }
            LayoutAction::Enter(context) => {
                state.current = context;
                Ok(())
            }
            LayoutAction::Focus(id) => {
                state.set_focus(id);
                Ok(())
            }
            LayoutAction::FocusToward(direction) => {
                if let Some(next) = state
                    .focus()
                    .and_then(|focus| state.tree().neighbour(focus, direction))
                {
                    state.set_focus(next);
                }
                Ok(())
            }
            LayoutAction::SplitFocused(axis) => state.split_focused(axis),
            LayoutAction::Open(kind) => state.open(kind),
            LayoutAction::Drop { source, target, edge } => state.drop_on(source, target, edge),
            LayoutAction::Close(id) => state.close(id),
            LayoutAction::CloseFocused => match state.focus() {
                Some(id) => state.close(id),
                None => Ok(()),
            },
            LayoutAction::Resize { path, sizes } => state.resize(&path, &sizes),
            LayoutAction::Undo => {
                state.undo();
                Ok(())
            }
            LayoutAction::Reset => {
                if state.reset() {
                    return Effect::send(LayoutAction::Delegate(Delegate::WasReset));
                }
                Ok(())
            }
            LayoutAction::UndoReset => {
                if state.can_undo_reset() {
                    state.undo();
                }
                Ok(())
            }
            LayoutAction::SetPolicy(policy) => {
                state.policy = policy;
                Ok(())
            }
            LayoutAction::Delegate(_) => Ok(()),
        };
        match outcome {
            Ok(()) => Effect::none(),
            Err(refusal) => Effect::send(LayoutAction::Delegate(Delegate::Refused(refusal))),
        }
    }
}

//! A screen's panes as a tree: splits side by side or stacked, any number
//! of children each, with a pane at every leaf. Edits keep the tree in
//! normal form: no split with one child, and no split directly inside a
//! split along the same axis, so dropping three panes in a row gives one
//! split of three rather than a staircase.

use humata2_clients::Workspace;
use humata2_clients::model::{ChannelId, Scope, SessionId};
use serde::{Deserialize, Serialize};

/// A pane's id, stable while it stays open, so its view and what it shows
/// survive splits, moves and restarts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PaneId(pub u64);

/// What a pane shows (NV-28).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PaneKind {
    /// A session's agent terminal.
    Agent(SessionId),
    /// One of a session's shells, counted from 0.
    Shell(SessionId, u32),
    /// A session's changes.
    Diff(SessionId),
    /// Files: a session's, or every host's with transfers.
    Files(Option<SessionId>),
    /// What a session is: its host, branch, agent and recovery.
    Info(SessionId),
    /// A board.
    Board(Scope),
    /// The Inbox.
    Inbox,
    /// The journal.
    Journal,
    /// A channel's sessions.
    Sessions(ChannelId),
    /// What needs the person across channels.
    Home,
    /// Hosts.
    Hosts,
    /// Views agents prepared.
    Views,
    /// Settings.
    Settings,
}

impl PaneKind {
    /// Whether it is a terminal, so splitting it opens a new shell rather
    /// than a second copy.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Agent(_) | Self::Shell(..))
    }

    /// The session it belongs to, if any.
    #[must_use]
    pub const fn session(&self) -> Option<&SessionId> {
        match self {
            Self::Agent(id) | Self::Shell(id, _) | Self::Diff(id) | Self::Info(id) => Some(id),
            Self::Files(id) => id.as_ref(),
            _ => None,
        }
    }

    /// Its title, from what the workspace holds.
    #[must_use]
    pub fn title(&self, workspace: &Workspace) -> String {
        match self {
            Self::Agent(id) => workspace
                .session(id)
                .map_or_else(|| id.to_string(), |session| session.title.clone()),
            Self::Shell(id, n) => workspace
                .session(id)
                .and_then(|session| session.shells.get(*n as usize).cloned())
                .unwrap_or_else(|| format!("Shell {}", n + 1)),
            Self::Diff(_) => "Diff".to_owned(),
            Self::Files(_) => "Files".to_owned(),
            Self::Info(_) => "Info".to_owned(),
            Self::Board(Scope::Global) => "Board".to_owned(),
            Self::Board(Scope::Channel(id)) => workspace
                .channel(id)
                .map_or_else(|| "Board".to_owned(), |channel| format!("#{} board", channel.name)),
            Self::Inbox => "Inbox".to_owned(),
            Self::Journal => "Journal".to_owned(),
            Self::Sessions(id) => workspace
                .channel(id)
                .map_or_else(|| id.to_string(), |channel| format!("#{}", channel.name)),
            Self::Home => "Home".to_owned(),
            Self::Hosts => "Hosts".to_owned(),
            Self::Views => "Views".to_owned(),
            Self::Settings => "Settings".to_owned(),
        }
    }
}

/// Which way a split lays out its children.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Axis {
    /// Side by side.
    Horizontal,
    /// Stacked.
    Vertical,
}

/// Where a dragged pane landed on another (NV-30).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Edge {
    /// Its left edge: split, dropped pane on the left.
    Left,
    /// Its right edge.
    Right,
    /// Its top edge.
    Top,
    /// Its bottom edge.
    Bottom,
    /// Its middle: the two swap places.
    Centre,
}

impl Edge {
    /// The axis a split at this edge makes, and whether the new pane goes
    /// first; `None` for the centre.
    #[must_use]
    pub const fn split(self) -> Option<(Axis, bool)> {
        match self {
            Self::Left => Some((Axis::Horizontal, true)),
            Self::Right => Some((Axis::Horizontal, false)),
            Self::Top => Some((Axis::Vertical, true)),
            Self::Bottom => Some((Axis::Vertical, false)),
            Self::Centre => None,
        }
    }
}

/// A direction to move focus in (⌘⌥ and an arrow).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Direction {
    /// ←
    Left,
    /// →
    Right,
    /// ↑
    Up,
    /// ↓
    Down,
}

/// A leaf: one pane.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pane {
    /// Its id.
    pub id: PaneId,
    /// What it shows.
    pub kind: PaneKind,
}

/// A split: children side by side or stacked, with each one's share of the
/// room, adding up to 1.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Split {
    /// Which way.
    pub axis: Axis,
    /// Its children, two or more.
    pub children: Vec<Tree>,
    /// Each child's share.
    pub sizes: Vec<f32>,
}

/// A screen's panes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Tree {
    /// One pane.
    Pane(Pane),
    /// A split.
    Split(Split),
}

/// The tree's structure without what the panes show, to compare layouts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shape {
    /// A pane.
    Pane,
    /// A split of these.
    Split(Axis, Vec<Self>),
}

/// A rectangle in a unit square, for finding neighbours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    /// Left.
    pub x: f32,
    /// Top.
    pub y: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

const EPSILON: f32 = 1e-4;

impl Tree {
    /// One pane.
    #[must_use]
    pub const fn pane(id: PaneId, kind: PaneKind) -> Self {
        Self::Pane(Pane { id, kind })
    }

    /// A split with equal shares.
    #[must_use]
    pub fn split(axis: Axis, children: Vec<Self>) -> Self {
        #[allow(clippy::cast_precision_loss)]
        let share = 1.0 / children.len().max(1) as f32;
        let sizes = vec![share; children.len()];
        let mut tree = Self::Split(Split { axis, children, sizes });
        tree.normalize();
        tree
    }

    /// Every pane, left to right and top to bottom.
    #[must_use]
    pub fn panes(&self) -> Vec<&Pane> {
        let mut panes = Vec::new();
        self.collect(&mut panes);
        panes
    }

    fn collect<'a>(&'a self, panes: &mut Vec<&'a Pane>) {
        match self {
            Self::Pane(pane) => panes.push(pane),
            Self::Split(split) => {
                for child in &split.children {
                    child.collect(panes);
                }
            }
        }
    }

    /// How many panes.
    #[must_use]
    pub fn count(&self) -> usize {
        match self {
            Self::Pane(_) => 1,
            Self::Split(split) => split.children.iter().map(Self::count).sum(),
        }
    }

    /// How many splits deep it goes: a lone pane is 0, two side by side 1.
    #[must_use]
    pub fn depth(&self) -> usize {
        match self {
            Self::Pane(_) => 0,
            Self::Split(split) => 1 + split.children.iter().map(Self::depth).max().unwrap_or(0),
        }
    }

    /// Its structure.
    #[must_use]
    pub fn shape(&self) -> Shape {
        match self {
            Self::Pane(_) => Shape::Pane,
            Self::Split(split) => Shape::Split(split.axis, split.children.iter().map(Self::shape).collect()),
        }
    }

    /// The pane with `id`.
    #[must_use]
    pub fn find(&self, id: PaneId) -> Option<&Pane> {
        self.panes().into_iter().find(|pane| pane.id == id)
    }

    /// The pane showing `kind`, if one does.
    #[must_use]
    pub fn showing(&self, kind: &PaneKind) -> Option<PaneId> {
        self.panes().into_iter().find(|pane| &pane.kind == kind).map(|pane| pane.id)
    }

    /// The highest pane id in it.
    #[must_use]
    pub fn max_id(&self) -> u64 {
        self.panes().iter().map(|pane| pane.id.0).max().unwrap_or(0)
    }

    fn find_mut(&mut self, id: PaneId) -> Option<&mut Pane> {
        match self {
            Self::Pane(pane) => (pane.id == id).then_some(pane),
            Self::Split(split) => split.children.iter_mut().find_map(|child| child.find_mut(id)),
        }
    }

    /// Shows `kind` in pane `id`, under the id `new`.
    pub fn replace(&mut self, id: PaneId, new: Pane) -> bool {
        match self.find_mut(id) {
            Some(pane) => {
                *pane = new;
                true
            }
            None => false,
        }
    }

    /// Swaps what panes `a` and `b` are, ids included, so each view moves
    /// with its pane.
    pub fn swap(&mut self, a: PaneId, b: PaneId) -> bool {
        let (Some(first), Some(second)) = (self.find(a).cloned(), self.find(b).cloned()) else {
            return false;
        };
        let placeholder = PaneId(u64::MAX);
        let mut done = self.replace(a, Pane { id: placeholder, kind: first.kind.clone() });
        done &= self.replace(b, first);
        done &= self.replace(placeholder, second);
        done
    }

    /// Takes pane `id` out; its share goes to its neighbours. Refused, so
    /// `None`, when it is the only pane or isn't there.
    pub fn remove(&mut self, id: PaneId) -> Option<Pane> {
        let Self::Split(split) = self else {
            return None;
        };
        let removed = Self::remove_from(split, id)?;
        self.normalize();
        Some(removed)
    }

    fn remove_from(split: &mut Split, id: PaneId) -> Option<Pane> {
        let at = split
            .children
            .iter()
            .position(|child| matches!(child, Self::Pane(pane) if pane.id == id));
        if let Some(at) = at {
            let Self::Pane(pane) = split.children.remove(at) else {
                return None;
            };
            split.sizes.remove(at);
            renormalize(&mut split.sizes);
            return Some(pane);
        }
        split.children.iter_mut().find_map(|child| match child {
            Self::Split(inner) => Self::remove_from(inner, id),
            Self::Pane(_) => None,
        })
    }

    /// Puts `new` beside pane `target` at `edge`, taking half of the
    /// target's room. `false` when the target isn't there or `edge` is the
    /// centre.
    pub fn insert(&mut self, target: PaneId, new: Pane, edge: Edge) -> bool {
        let Some((axis, before)) = edge.split() else {
            return false;
        };
        let done = self.insert_at(target, new, axis, before);
        self.normalize();
        done
    }

    fn insert_at(&mut self, target: PaneId, new: Pane, axis: Axis, before: bool) -> bool {
        match self {
            Self::Pane(pane) if pane.id == target => {
                let old = Self::Pane(pane.clone());
                let new = Self::Pane(new);
                let children = if before { vec![new, old] } else { vec![old, new] };
                *self = Self::Split(Split {
                    axis,
                    children,
                    sizes: vec![0.5, 0.5],
                });
                true
            }
            Self::Pane(_) => false,
            Self::Split(split) => {
                let direct = split
                    .children
                    .iter()
                    .position(|child| matches!(child, Self::Pane(pane) if pane.id == target));
                match direct {
                    Some(at) if split.axis == axis => {
                        let half = split.sizes[at] / 2.0;
                        split.sizes[at] = half;
                        let at = if before { at } else { at + 1 };
                        split.children.insert(at, Self::Pane(new));
                        split.sizes.insert(at, half);
                        true
                    }
                    _ => {
                        let mut new = Some(new);
                        for child in &mut split.children {
                            if child.find(target).is_some() {
                                if let Some(new) = new.take() {
                                    return child.insert_at(target, new, axis, before);
                                }
                            }
                        }
                        false
                    }
                }
            }
        }
    }

    /// Sets the shares of the split at `path` (child indices from the
    /// root). `false` when there is no such split or the count is wrong.
    pub fn resize(&mut self, path: &[usize], sizes: &[f32]) -> bool {
        let mut node = self;
        for &at in path {
            let Self::Split(split) = node else {
                return false;
            };
            let Some(child) = split.children.get_mut(at) else {
                return false;
            };
            node = child;
        }
        match node {
            Self::Split(split) if split.sizes.len() == sizes.len() && sizes.iter().all(|size| *size > 0.0) => {
                split.sizes = sizes.to_vec();
                renormalize(&mut split.sizes);
                true
            }
            _ => false,
        }
    }

    /// Puts the tree in normal form: lone children replace their split, and
    /// a split inside a split along the same axis merges into it.
    pub fn normalize(&mut self) {
        let Self::Split(split) = self else {
            return;
        };
        for child in &mut split.children {
            child.normalize();
        }
        let mut children = Vec::with_capacity(split.children.len());
        let mut sizes = Vec::with_capacity(split.sizes.len());
        for (child, size) in split.children.drain(..).zip(split.sizes.drain(..)) {
            match child {
                Self::Split(inner) if inner.axis == split.axis => {
                    for (grandchild, share) in inner.children.into_iter().zip(inner.sizes) {
                        children.push(grandchild);
                        sizes.push(size * share);
                    }
                }
                child => {
                    children.push(child);
                    sizes.push(size);
                }
            }
        }
        renormalize(&mut sizes);
        split.children = children;
        split.sizes = sizes;
        if split.children.len() == 1 {
            if let Some(only) = split.children.pop() {
                *self = only;
            }
        }
    }

    /// Where each pane sits in a unit square.
    #[must_use]
    pub fn rects(&self) -> Vec<(PaneId, Rect)> {
        let mut rects = Vec::new();
        self.place(
            Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            &mut rects,
        );
        rects
    }

    fn place(&self, rect: Rect, rects: &mut Vec<(PaneId, Rect)>) {
        match self {
            Self::Pane(pane) => rects.push((pane.id, rect)),
            Self::Split(split) => {
                let mut offset = 0.0;
                for (child, share) in split.children.iter().zip(&split.sizes) {
                    let part = match split.axis {
                        Axis::Horizontal => Rect {
                            x: rect.x + offset * rect.width,
                            width: share * rect.width,
                            ..rect
                        },
                        Axis::Vertical => Rect {
                            y: rect.y + offset * rect.height,
                            height: share * rect.height,
                            ..rect
                        },
                    };
                    child.place(part, rects);
                    offset += share;
                }
            }
        }
    }

    /// The pane next to `id` toward `direction`: the closest one on that
    /// side that overlaps it the most.
    #[must_use]
    pub fn neighbour(&self, id: PaneId, direction: Direction) -> Option<PaneId> {
        let rects = self.rects();
        let (_, from) = rects.iter().find(|(pane, _)| *pane == id)?;
        let overlap = |a0: f32, a1: f32, b0: f32, b1: f32| (a1.min(b1) - a0.max(b0)).max(0.0);
        rects
            .iter()
            .filter(|(pane, _)| *pane != id)
            .filter_map(|(pane, to)| {
                let (gap, shared) = match direction {
                    Direction::Left => (
                        from.x - (to.x + to.width),
                        overlap(from.y, from.y + from.height, to.y, to.y + to.height),
                    ),
                    Direction::Right => (
                        to.x - (from.x + from.width),
                        overlap(from.y, from.y + from.height, to.y, to.y + to.height),
                    ),
                    Direction::Up => (
                        from.y - (to.y + to.height),
                        overlap(from.x, from.x + from.width, to.x, to.x + to.width),
                    ),
                    Direction::Down => (
                        to.y - (from.y + from.height),
                        overlap(from.x, from.x + from.width, to.x, to.x + to.width),
                    ),
                };
                (gap > -EPSILON && shared > EPSILON).then_some((*pane, gap, shared))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1).then(b.2.total_cmp(&a.2)))
            .map(|(pane, _, _)| pane)
    }
}

fn renormalize(sizes: &mut [f32]) {
    let total: f32 = sizes.iter().sum();
    if total <= 0.0 {
        #[allow(clippy::cast_precision_loss)]
        let share = 1.0 / sizes.len().max(1) as f32;
        sizes.iter_mut().for_each(|size| *size = share);
        return;
    }
    sizes.iter_mut().for_each(|size| *size /= total);
}

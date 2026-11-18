//! In-memory arrangement of session-bound panes. Closing a pane keeps its state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Agent,
    Shell,
    Tool(&'static str),
}
impl Kind {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Agent => "Agent",
            Self::Shell => "Shell",
            Self::Tool(name) => name,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Edge {
    Left,
    Right,
    Above,
    Below,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Node {
    Pane(usize),
    Split {
        id: usize,
        vertical: bool,
        first: Box<Node>,
        second: Box<Node>,
    },
}
impl Node {
    fn contains(&self, pane: usize) -> bool {
        match self {
            Self::Pane(id) => *id == pane,
            Self::Split { first, second, .. } => first.contains(pane) || second.contains(pane),
        }
    }
    fn first(&self) -> usize {
        match self {
            Self::Pane(id) => *id,
            Self::Split { first, .. } => first.first(),
        }
    }
    fn insert(&mut self, target: usize, incoming: usize, edge: Edge, split: usize) {
        match self {
            Self::Pane(id) if *id == target => {
                let previous = self.clone();
                let new = Self::Pane(incoming);
                let (first, second) = if matches!(edge, Edge::Left | Edge::Above) {
                    (new, previous)
                } else {
                    (previous, new)
                };
                *self = Self::Split {
                    id: split,
                    vertical: matches!(edge, Edge::Above | Edge::Below),
                    first: Box::new(first),
                    second: Box::new(second),
                };
            }
            Self::Split { first, second, .. } => {
                if first.contains(target) {
                    first.insert(target, incoming, edge, split);
                } else {
                    second.insert(target, incoming, edge, split);
                }
            }
            _ => {}
        }
    }
    fn remove(self, pane: usize) -> Option<Self> {
        match self {
            Self::Pane(id) => (id != pane).then_some(Self::Pane(id)),
            Self::Split {
                id,
                vertical,
                first,
                second,
            } => match (first.remove(pane), second.remove(pane)) {
                (Some(first), Some(second)) => Some(Self::Split {
                    id,
                    vertical,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (first, second) => first.or(second),
            },
        }
    }
}
#[derive(Clone, Debug)]
pub(super) struct Pane {
    pub(super) session: usize,
    pub(super) kind: Kind,
}
pub(super) struct Layout {
    pub(super) root: Option<Node>,
    pub(super) panes: Vec<Pane>,
    pub(super) focused: usize,
    pub(super) maximized: Option<usize>,
    pub(super) closed: Vec<usize>,
    next_split: usize,
}
impl Layout {
    pub(super) fn new() -> Self {
        Self {
            root: Some(Node::Pane(0)),
            panes: vec![Pane {
                session: 0,
                kind: Kind::Agent,
            }],
            focused: 0,
            maximized: None,
            closed: vec![],
            next_split: 0,
        }
    }
    pub(super) fn visible(&self, pane: usize) -> bool {
        self.root.as_ref().is_some_and(|root| root.contains(pane))
    }
    pub(super) fn find(&self, session: usize, kind: Kind) -> Option<usize> {
        self.panes
            .iter()
            .enumerate()
            .find(|(id, p)| p.session == session && p.kind == kind && self.visible(*id))
            .map(|(id, _)| id)
    }
    pub(super) fn add(&mut self, session: usize, kind: Kind, target: usize, edge: Edge) -> usize {
        let id = self.panes.len();
        self.panes.push(Pane { session, kind });
        self.attach(id, target, edge);
        id
    }
    fn attach(&mut self, id: usize, target: usize, edge: Edge) {
        if let Some(root) = &mut self.root {
            let target = if root.contains(target) {
                target
            } else {
                root.first()
            };
            root.insert(target, id, edge, self.next_split);
            self.next_split += 1;
        } else {
            self.root = Some(Node::Pane(id));
        }
        self.focused = id;
        self.maximized = None;
    }
    pub(super) fn close(&mut self, id: usize) {
        if !self.visible(id) {
            return;
        }
        self.root = self.root.take().and_then(|root| root.remove(id));
        self.closed.push(id);
        if self.focused == id {
            self.focused = self.root.as_ref().map(Node::first).unwrap_or(0);
        }
        if self.maximized == Some(id) {
            self.maximized = None;
        }
    }
    pub(super) fn reopen(&mut self) -> Option<usize> {
        let id = self.closed.pop()?;
        self.attach(id, self.focused, Edge::Right);
        Some(id)
    }
    pub(super) fn move_to(&mut self, id: usize, target: usize, edge: Edge) {
        if id == target || !self.visible(id) || !self.visible(target) {
            return;
        }
        self.root = self.root.take().and_then(|root| root.remove(id));
        self.attach(id, target, edge);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn split_move_close_and_restore_keep_session_identity() {
        let mut layout = Layout::new();
        let shell = layout.add(0, Kind::Shell, 0, Edge::Right);
        let other = layout.add(1, Kind::Agent, shell, Edge::Below);
        layout.move_to(other, 0, Edge::Left);
        assert_eq!(layout.panes[other].session, 1);
        assert!(layout.visible(shell) && layout.visible(other) && layout.visible(0));
        layout.maximized = Some(other);
        layout.close(other);
        assert_eq!(layout.maximized, None);
        assert!(!layout.visible(other));
        assert_eq!(layout.reopen(), Some(other));
        assert_eq!(layout.panes[other].session, 1);
        layout.move_to(0, 0, Edge::Above);
        assert!(layout.visible(0));
        layout.close(0);
        layout.close(shell);
        layout.close(other);
        assert_eq!(layout.root, None);
        assert_eq!(layout.reopen(), Some(other));
        assert_eq!(layout.root, Some(Node::Pane(other)));
    }
}

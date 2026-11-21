//! Per-channel page state (CH-9–15), independent of any GUI framework.
use humata2_clients::{
    Workspace,
    model::{ChannelId, MachineId, Session, SessionId},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// A channel's page tabs, in their approved order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tab {
    #[default]
    Sessions,
    Desk,
    Inbox,
    Board,
    Repos,
    Access,
    Activity,
    Settings,
}
/// Which sessions the tree table includes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum History {
    #[default]
    Open,
    Archived,
    All,
}
/// The channel page as the person left it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct View {
    pub tab: Tab,
    pub history: History,
    pub query: String,
    pub folded: BTreeSet<SessionId>,
    pub clones: Option<(MachineId, String)>,
}
/// State of the "Delete channel…" confirmation modal (CH-24).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteState {
    pub channel: ChannelId,
    pub channel_name: String,
    pub loading: bool,
    pub plan: Option<humata2_clients::DeleteChannelPlan>,
    pub delete_work: bool,
    pub end_live: bool,
    pub deleting: bool,
    pub error: Option<String>,
}

/// Channel page interactions, scoped by the current route.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Tab(Tab),
    History(History),
    Filter(String),
    Fold(SessionId),
    ExpandAll,
    CollapseAll,
    Clones { machine: MachineId, repo: String },
    ClearClones,
    /// Toggle the channel header `…` menu.
    ToggleMenu,
    /// Close the channel header `…` menu.
    CloseMenu,
    /// Open the "Delete channel…" confirmation modal and plan the delete.
    StartDelete,
    /// The daemon answered `plan_delete_channel`.
    DeletePlanned {
        channel: ChannelId,
        result: Result<humata2_clients::DeleteChannelPlan, humata2_clients::DaemonError>,
    },
    /// Toggle "Delete uncommitted or unpushed work".
    ToggleDeleteWork,
    /// Toggle ending live sessions vs moving them to Desk.
    ToggleEndLive,
    /// Confirm and execute `delete_channel`.
    ConfirmDelete,
    /// The daemon answered `delete_channel`.
    DeleteFinished {
        channel: ChannelId,
        result: Result<humata2_clients::DeleteChannelResponse, humata2_clients::DaemonError>,
    },
    /// Dismiss the "Delete channel…" modal.
    CancelDelete,
}
impl View {
    pub(super) fn change(&mut self, action: Action, workspace: &Workspace, channel: &ChannelId) {
        match action {
            Action::Tab(tab) => self.tab = tab,
            Action::History(history) => self.history = history,
            Action::Filter(query) => self.query = query,
            Action::Fold(id) => {
                if !self.folded.remove(&id) {
                    self.folded.insert(id);
                }
            }
            Action::ExpandAll => self.folded.clear(),
            Action::CollapseAll => {
                self.folded = workspace
                    .sessions
                    .iter()
                    .filter(|s| s.channel == *channel)
                    .map(|s| s.id.clone())
                    .collect()
            }
            Action::Clones { machine, repo } => {
                self.clones = Some((machine, repo));
                self.tab = Tab::Sessions;
                self.folded.clear();
                self.query.clear();
                self.history = History::Open;
            }
            Action::ClearClones => self.clones = None,
            Action::ToggleMenu
            | Action::CloseMenu
            | Action::StartDelete
            | Action::DeletePlanned { .. }
            | Action::ToggleDeleteWork
            | Action::ToggleEndLive
            | Action::ConfirmDelete
            | Action::DeleteFinished { .. }
            | Action::CancelDelete => {}
        }
    }
}

/// A row in the channel tree table, including connectors and descendant requests.
pub struct Row<'a> {
    pub session: &'a Session,
    pub prefix: String,
    pub has_children: bool,
    pub expanded: bool,
    pub pending: usize,
}

/// The session and all descendants, bounded even for a malformed parent cycle.
#[must_use]
pub fn family(workspace: &Workspace, id: &SessionId) -> BTreeSet<SessionId> {
    let mut ids = BTreeSet::from([id.clone()]);
    loop {
        let before = ids.len();
        for session in &workspace.sessions {
            if session.parent.as_ref().is_some_and(|id| ids.contains(id)) {
                ids.insert(session.id.clone());
            }
        }
        if ids.len() == before {
            return ids;
        }
    }
}

/// Whether the host's clone listing contains this repository for the session.
#[must_use]
pub fn has_repo(workspace: &Workspace, session: &Session, repo: &str) -> bool {
    let same = |name: &str| name == repo || name.rsplit('/').next() == repo.rsplit('/').next();
    same(&session.repo)
        || workspace
            .session_files
            .get(&session.id)
            .is_some_and(|files| files.iter().any(|f| same(&f.repo)))
}

/// Visible rows preserve ancestors when filtering, while ordinary folding hides descendants.
#[must_use]
pub fn rows<'a>(workspace: &'a Workspace, channel: &ChannelId, view: &View) -> Vec<Row<'a>> {
    let sessions: Vec<_> = workspace
        .sessions
        .iter()
        .filter(|s| s.channel == *channel)
        .collect();
    let query = view.query.to_lowercase();
    let mut visible: BTreeSet<_> = sessions
        .iter()
        .filter(|s| {
            (match view.history {
                History::Open => !s.archived,
                History::Archived => s.archived,
                History::All => true,
            }) && (query.is_empty() || s.title.to_lowercase().contains(&query))
                && view
                    .clones
                    .as_ref()
                    .is_none_or(|(host, repo)| s.machine == *host && has_repo(workspace, s, repo))
        })
        .map(|s| s.id.clone())
        .collect();
    let filtered = !query.is_empty() || view.clones.is_some() || view.history == History::Archived;
    if filtered {
        for id in visible.clone() {
            let mut parent = workspace.session(&id).and_then(|s| s.parent.clone());
            let mut seen = BTreeSet::new();
            while let Some(id) = parent {
                if !seen.insert(id.clone()) {
                    break;
                }
                let Some(s) = workspace.session(&id).filter(|s| s.channel == *channel) else {
                    break;
                };
                visible.insert(id);
                parent = s.parent.clone();
            }
        }
    }
    struct Tree<'a> {
        workspace: &'a Workspace,
        sessions: Vec<&'a Session>,
        visible: BTreeSet<SessionId>,
        seen: BTreeSet<SessionId>,
        rows: Vec<Row<'a>>,
    }
    fn visit<'a>(
        tree: &mut Tree<'a>,
        session: &'a Session,
        prefix: String,
        last: bool,
        root: bool,
        view: &View,
        filtered: bool,
    ) {
        if !tree.seen.insert(session.id.clone()) || !tree.visible.contains(&session.id) {
            return;
        }
        let children: Vec<_> = tree
            .sessions
            .iter()
            .copied()
            .filter(|s| s.parent.as_ref() == Some(&session.id) && tree.visible.contains(&s.id))
            .collect();
        let expanded = filtered || !view.folded.contains(&session.id);
        let family = family(tree.workspace, &session.id);
        tree.rows.push(Row {
            session,
            prefix: format!(
                "{}{}",
                prefix,
                if root {
                    ""
                } else if last {
                    "└─ "
                } else {
                    "├─ "
                }
            ),
            has_children: !children.is_empty(),
            expanded,
            pending: tree
                .workspace
                .waiting()
                .filter(|r| family.contains(&r.session))
                .count(),
        });
        if expanded {
            let next = if root {
                prefix
            } else {
                format!("{prefix}{}", if last { "   " } else { "│  " })
            };
            for (i, child) in children.iter().enumerate() {
                visit(
                    tree,
                    child,
                    next.clone(),
                    i + 1 == children.len(),
                    false,
                    view,
                    filtered,
                );
            }
        }
    }
    let mut tree = Tree {
        workspace,
        sessions: sessions.clone(),
        visible,
        seen: BTreeSet::new(),
        rows: Vec::new(),
    };
    for session in sessions.iter().copied().filter(|s| {
        s.parent
            .as_ref()
            .is_none_or(|id| !sessions.iter().any(|s| s.id == *id))
    }) {
        visit(
            &mut tree,
            session,
            String::new(),
            true,
            true,
            view,
            filtered,
        );
    }
    tree.rows
}

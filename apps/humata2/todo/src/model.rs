use crate::day::Day;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Effort {
    S,
    M,
    L,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Impact {
    Low,
    Medium,
    High,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum State {
    Open,
    InProgress,
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub text: String,
    pub done: bool,
}

/// A pull request review request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    /// "acme/api#483"
    pub repo_prs: Vec<String>,
    pub author: String,
    pub waiting_days: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AskKind {
    Close,
    Edit {
        field: String,
        before: String,
        after: String,
    },
    Add,
}

/// An agent's pending change on an item.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ask {
    pub kind: AskKind,
    /// "claude in Ledger entries"
    pub by: String,
    pub reason: String,
}

impl Ask {
    /// The short tag shown on the row.
    pub fn label(&self) -> &'static str {
        match &self.kind {
            AskKind::Close => "close?",
            AskKind::Edit { field, .. } if field == "due" => "new due date?",
            AskKind::Edit { .. } => "edit?",
            AskKind::Add => "add?",
        }
    }
}

/// A fact an agent offers for its ask: "3 commits", with where to read it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    /// What it is, as a link or plain text: "3 commits".
    pub text: String,
    /// Where to read it, if anywhere.
    pub link: Option<String>,
}

/// What an ask carries beyond its kind, who and why: what backs it, what
/// else it touches, how long ago it was made and whether the agent can
/// still be asked. Kept beside [`Ask`] so existing asks need no change.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AskDetail {
    /// What backs the ask: commits, a pull request, tests.
    pub evidence: Vec<Evidence>,
    /// What else changes if the answer is yes.
    pub affects: Vec<String>,
    /// The change as before and after, when it is not the ask's own kind.
    pub change: Option<(String, String)>,
    /// Minutes since the agent asked.
    pub asked_minutes_ago: u32,
    /// The agent's session has ended, so it can't answer "Ask why" until
    /// it is resumed.
    pub session_ended: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum OldColumn {
    Backlog,
    ToDo,
    InProgress,
    InReview,
    Done,
}

impl OldColumn {
    /// Order used to break ties: In progress, In review, To do, Backlog, Done.
    pub(crate) fn tie_order(self) -> u8 {
        match self {
            OldColumn::InProgress => 0,
            OldColumn::InReview => 1,
            OldColumn::ToDo => 2,
            OldColumn::Backlog => 3,
            OldColumn::Done => 4,
        }
    }
}

/// Where a migrated item was on the old boards.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Start {
    pub column: OldColumn,
    /// 1-based within its board and column.
    pub position: u32,
    /// None is the global board.
    pub board: Option<String>,
    pub vip: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Todo {
    /// "PAY-3"; may be empty for a suggested item.
    pub key: String,
    pub title: String,
    pub channel: Option<String>,
    pub sessions: Vec<String>,
    pub state: State,
    pub due: Option<Day>,
    pub effort: Option<Effort>,
    pub impact: Option<Impact>,
    pub tags: Vec<String>,
    pub steps: Vec<Step>,
    pub hide_until: Option<Day>,
    /// Pinned, with the order among pinned items; smaller first.
    pub pin: Option<u32>,
    /// Keys of items this one waits on.
    pub depends_on: Vec<String>,
    pub suggested: bool,
    pub ask: Option<Ask>,
    /// What backs the ask, when there is one.
    #[serde(default)]
    pub ask_detail: AskDetail,
    /// The item's longer text.
    #[serde(default)]
    pub description: String,
    pub review: Option<Review>,
    pub start: Option<Start>,
    pub created: Day,
    pub closed: Option<Day>,
}

impl Todo {
    /// An open item with only a title.
    pub fn new(key: &str, title: &str, created: Day) -> Todo {
        Todo {
            key: key.to_owned(),
            title: title.to_owned(),
            channel: None,
            sessions: Vec::new(),
            state: State::Open,
            due: None,
            effort: None,
            impact: None,
            tags: Vec::new(),
            steps: Vec::new(),
            hide_until: None,
            pin: None,
            depends_on: Vec::new(),
            suggested: false,
            ask: None,
            ask_detail: AskDetail::default(),
            description: String::new(),
            review: None,
            start: None,
            created,
            closed: None,
        }
    }
}

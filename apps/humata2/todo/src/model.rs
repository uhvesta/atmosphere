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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Need {
    Idea,
    NiceToHave,
    Expected,
    Critical,
}

impl Need {
    pub fn label(self) -> &'static str {
        match self {
            Need::Idea => "Idea",
            Need::NiceToHave => "Nice to have",
            Need::Expected => "Expected",
            Need::Critical => "Critical",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum FuzzyDue {
    InAFewDays,
    NextWeek,
    NextMonth,
}

impl FuzzyDue {
    pub fn label(self) -> &'static str {
        match self {
            FuzzyDue::InAFewDays => "In a few days",
            FuzzyDue::NextWeek => "Next week",
            FuzzyDue::NextMonth => "Next month",
        }
    }

    /// Approximate day horizon for ranking (never treated as overdue).
    pub fn approx_days(self) -> i32 {
        match self {
            FuzzyDue::InAFewDays => 3,
            FuzzyDue::NextWeek => 7,
            FuzzyDue::NextMonth => 30,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum State {
    Open,
    InProgress,
    Closed,
    WontDo,
    Became,
    WatchingPr,
}

impl State {
    pub fn is_terminal(self) -> bool {
        matches!(self, State::Closed | State::WontDo | State::Became)
    }

    pub fn is_active(self) -> bool {
        !self.is_terminal()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CheckpointState {
    WithContext,
    Defined,
    Done,
}

impl CheckpointState {
    pub fn label(self) -> &'static str {
        match self {
            CheckpointState::WithContext => "With context",
            CheckpointState::Defined => "Defined",
            CheckpointState::Done => "Done",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SourceAnchor {
    pub repo: String,
    pub path: String,
    pub line: u32,
    pub end_line: Option<u32>,
    pub comment_id: Option<String>,
}

impl SourceAnchor {
    pub fn display(&self) -> String {
        match self.end_line {
            Some(end) if end > self.line => format!("{}:{}-{}", self.path, self.line, end),
            _ if self.line > 0 => format!("{}:{}", self.path, self.line),
            _ => self.path.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExternalBlock {
    pub reason: String,
    pub since: Option<Day>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TodoComment {
    pub id: String,
    pub author: String,
    pub body: String,
    pub created: Option<Day>,
    pub step_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OwnerKind {
    User,
    Agent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoOwner {
    pub kind: OwnerKind,
    pub label: String,
    #[serde(default)]
    pub session: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StuckAgent {
    pub session: String,
    pub agent_label: String,
    pub reason: String,
    pub since_minutes: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockerGone {
    pub blocker_key: String,
    pub blocker_state: State,
    #[serde(default)]
    pub replacement_keys: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WatchedPr {
    pub repo_pr: String,
    pub title: String,
    pub ci_state: String,
    pub unresolved_comments: u32,
    pub has_conflicts: bool,
    pub upstream_merged: bool,
    pub merged: bool,
    pub closed_without_merge: bool,
    pub stack_key: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CloseReadiness {
    pub steps_done: u32,
    pub steps_total: u32,
    pub unresolved_comments: u32,
    pub ci_status: String,
    pub commits: u32,
}

impl CloseReadiness {
    /// Compact summary line shown in the drawer and close requests:
    /// `Steps X/Y · Z unresolved comments · CI … · N commits`.
    pub fn summary_line(&self) -> String {
        let mut parts = Vec::new();
        if self.steps_total > 0 {
            parts.push(format!("Steps {}/{}", self.steps_done, self.steps_total));
        }
        let comment_word = if self.unresolved_comments == 1 {
            "comment"
        } else {
            "comments"
        };
        parts.push(format!("{} unresolved {}", self.unresolved_comments, comment_word));
        if !self.ci_status.is_empty() {
            parts.push(format!("CI {}", self.ci_status));
        }
        let commit_word = if self.commits == 1 { "commit" } else { "commits" };
        parts.push(format!("{} {}", self.commits, commit_word));
        parts.join(" · ")
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionSignal {
    pub session: String,
    pub label: String,
    pub status: String,
    pub idle_days: u32,
    pub active_turn: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LifecyclePrompt {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub detail: String,
    pub choices: Vec<String>,
    pub pr_ref: Option<String>,
    pub session: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunLink {
    pub label: String,
    pub target: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunEntry {
    pub id: String,
    pub tool: String,
    pub model: String,
    pub outcome: String,
    pub cost_micros: u64,
    pub started: Option<Day>,
    pub links: Vec<RunLink>,
    pub session: Option<String>,
}

/// Formats micro-dollars as `$X.XX`.
pub fn format_cost_usd(cost_micros: u64) -> String {
    let cents = (cost_micros + 5_000) / 10_000;
    format!("${}.{:02}", cents / 100, cents % 100)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortBy {
    #[default]
    Priority,
    DueSoonest,
    MostNeeded,
    SmallestEffort,
    LargestImpact,
}

impl SortBy {
    pub fn label(self) -> &'static str {
        match self {
            SortBy::Priority => "Priority",
            SortBy::DueSoonest => "Due soonest",
            SortBy::MostNeeded => "Most needed",
            SortBy::SmallestEffort => "Smallest effort",
            SortBy::LargestImpact => "Largest impact",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HeldBackBy {
    pub summary: String,
    pub factors: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WordDiffKind {
    Equal,
    Added,
    Removed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WordDiffSpan {
    pub kind: WordDiffKind,
    pub text: String,
}

/// Computes a pure word-level diff between `before` and `after` using LCS,
/// coalescing consecutive spans of the same kind.
pub fn word_diff(before: &str, after: &str) -> Vec<WordDiffSpan> {
    let a: Vec<&str> = before.split_whitespace().collect();
    let b: Vec<&str> = after.split_whitespace().collect();
    let n = a.len();
    let m = b.len();
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }

    let mut raw: Vec<(WordDiffKind, &str)> = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[i] == b[j] {
            raw.push((WordDiffKind::Equal, a[i]));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            raw.push((WordDiffKind::Removed, a[i]));
            i += 1;
        } else {
            raw.push((WordDiffKind::Added, b[j]));
            j += 1;
        }
    }
    while i < n {
        raw.push((WordDiffKind::Removed, a[i]));
        i += 1;
    }
    while j < m {
        raw.push((WordDiffKind::Added, b[j]));
        j += 1;
    }

    let mut spans: Vec<WordDiffSpan> = Vec::new();
    for (kind, word) in raw {
        if let Some(last) = spans.last_mut()
            && last.kind == kind
        {
            last.text.push(' ');
            last.text.push_str(word);
        } else {
            spans.push(WordDiffSpan {
                kind,
                text: word.to_owned(),
            });
        }
    }
    spans
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Step {
    pub id: String,
    pub text: String,
    pub done: bool,
    pub markdown_context: Option<String>,
    pub dropped: bool,
    pub from_key: Option<String>,
    pub source_anchor: Option<SourceAnchor>,
}

impl Step {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }
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
    Question {
        question: String,
        #[serde(default)]
        choices: Vec<String>,
        #[serde(default)]
        multi_select: bool,
        #[serde(default)]
        step_id: Option<String>,
        #[serde(default)]
        details: Option<String>,
        #[serde(default)]
        editable: bool,
    },
    ActionRequest {
        action: String,
        #[serde(default)]
        step_id: Option<String>,
        #[serde(default)]
        details: Option<String>,
    },
    ReviewRequest {
        summary: String,
        #[serde(default)]
        round: u32,
        #[serde(default)]
        step_id: Option<String>,
        #[serde(default)]
        details: Option<String>,
        #[serde(default)]
        checkpoint: Option<CheckpointState>,
    },
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
            AskKind::Question { .. } => "question?",
            AskKind::ActionRequest { .. } => "action?",
            AskKind::ReviewRequest { .. } => "review?",
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
    #[serde(default)]
    pub need: Option<Need>,
    #[serde(default)]
    pub fuzzy_due: Option<FuzzyDue>,
    #[serde(default)]
    pub external_block: Option<ExternalBlock>,
    #[serde(default)]
    pub from_keys: Vec<String>,
    #[serde(default)]
    pub became_keys: Vec<String>,
    #[serde(default)]
    pub source_anchor: Option<SourceAnchor>,
    #[serde(default)]
    pub comments: Vec<TodoComment>,
    #[serde(default)]
    pub owner: Option<TodoOwner>,
    #[serde(default)]
    pub checkpoint: Option<CheckpointState>,
    #[serde(default)]
    pub stuck_agent: Option<StuckAgent>,
    #[serde(default)]
    pub blocker_gone: Vec<BlockerGone>,
    #[serde(default)]
    pub watched_prs: Vec<WatchedPr>,
    #[serde(default)]
    pub close_readiness: Option<CloseReadiness>,
    #[serde(default)]
    pub session_signals: Vec<SessionSignal>,
    #[serde(default)]
    pub lifecycle_prompts: Vec<LifecyclePrompt>,
    #[serde(default)]
    pub run_count: u32,
    #[serde(default)]
    pub total_cost_micros: u64,
    #[serde(default)]
    pub wont_do_reason: Option<String>,
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
            need: None,
            fuzzy_due: None,
            external_block: None,
            from_keys: Vec::new(),
            became_keys: Vec::new(),
            source_anchor: None,
            comments: Vec::new(),
            owner: None,
            checkpoint: None,
            stuck_agent: None,
            blocker_gone: Vec::new(),
            watched_prs: Vec::new(),
            close_readiness: None,
            session_signals: Vec::new(),
            lifecycle_prompts: Vec::new(),
            run_count: 0,
            total_cost_micros: 0,
            wont_do_reason: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_diff_coalesces_equal_removed_and_added_spans() {
        let spans = word_diff("Fix login redirect loop", "Fix auth redirect bug now");
        assert_eq!(
            spans,
            vec![
                WordDiffSpan { kind: WordDiffKind::Equal, text: "Fix".to_owned() },
                WordDiffSpan { kind: WordDiffKind::Removed, text: "login".to_owned() },
                WordDiffSpan { kind: WordDiffKind::Added, text: "auth".to_owned() },
                WordDiffSpan { kind: WordDiffKind::Equal, text: "redirect".to_owned() },
                WordDiffSpan { kind: WordDiffKind::Removed, text: "loop".to_owned() },
                WordDiffSpan { kind: WordDiffKind::Added, text: "bug now".to_owned() },
            ]
        );
    }

    #[test]
    fn close_readiness_summary_line_formats_compactly() {
        let cr = CloseReadiness {
            steps_done: 4,
            steps_total: 5,
            unresolved_comments: 2,
            ci_status: "passing".to_owned(),
            commits: 3,
        };
        assert_eq!(
            cr.summary_line(),
            "Steps 4/5 · 2 unresolved comments · CI passing · 3 commits"
        );
    }

    #[test]
    fn format_cost_usd_rounds_to_cents() {
        assert_eq!(format_cost_usd(0), "$0.00");
        assert_eq!(format_cost_usd(1_250_000), "$1.25");
        assert_eq!(format_cost_usd(45_000), "$0.05");
    }
}

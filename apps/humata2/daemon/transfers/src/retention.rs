//! Retention rules (LD-16, LD-17, LD-18): the decision half. A pure function
//! from the rules, facts about sessions and the time now to what to archive,
//! what to skip and why, and when to look again (TR-9: one timer, no
//! polling). The daemon gathers the facts and carries the decisions out.

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// The user's retention rules; 0 turns a rule off (LD-16: both are off by
/// default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rules {
    /// Archive PR reviews this many days after the PR merges or closes.
    pub archive_pr_reviews_after_days: u32,
    /// Archive sessions with no activity for this many days.
    pub archive_idle_sessions_after_days: u32,
}

/// The rule that made a decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// Archive PR reviews `days` after the PR merged or closed.
    PrReviews { days: u32 },
    /// Archive sessions inactive for `days`.
    Inactive { days: u32 },
}

/// Whether a session's work is safe on a remote (LD-17).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Work {
    /// Everything is committed and pushed.
    Safe,
    /// Files with uncommitted changes.
    Uncommitted { files: u32 },
    /// Branches with commits that are not pushed.
    Unpushed { branches: Vec<String> },
    /// Never reported, so not known to be safe.
    Unknown,
}

/// What the daemon knows about one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionFacts {
    /// The session's id.
    pub id: String,
    /// The session's title, for the skip list.
    pub title: String,
    /// Already archived: no rule touches it.
    pub archived: bool,
    /// The session is a PR review.
    pub is_pr_review: bool,
    /// When its PR merged or closed; `None` while open or unknown.
    pub pr_closed_unix_ms: Option<i64>,
    /// When the session was created.
    pub created_unix_ms: i64,
    /// When a terminal of the session last printed.
    pub last_output_unix_ms: Option<i64>,
    /// When a terminal of the session last took input.
    pub last_input_unix_ms: Option<i64>,
    /// When the user last opened it.
    pub last_opened_unix_ms: Option<i64>,
    /// Whether its work is safe on a remote.
    pub work: Work,
}

/// A session a rule archives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Archive {
    /// The session's id.
    pub session: String,
    /// The rule that archives it.
    pub rule: Rule,
}

/// Why a rule left a due session alone (LD-17).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// It has uncommitted files.
    Uncommitted { files: u32 },
    /// It has unpushed branches.
    Unpushed { branches: Vec<String> },
    /// Its work was never reported.
    WorkUnknown,
}

/// A due session a rule did not archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skip {
    /// The session's id.
    pub session: String,
    /// The session's title.
    pub title: String,
    /// The rule that would have archived it.
    pub rule: Rule,
    /// What holds it back.
    pub reason: SkipReason,
}

/// What the rules decide at one moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    /// Sessions to archive, in the order given.
    pub archive: Vec<Archive>,
    /// Due sessions left alone, in the order given.
    pub skips: Vec<Skip>,
    /// The earliest future moment a session not due now becomes due; `None`
    /// when nothing can become due without a new fact.
    pub next_due_unix_ms: Option<i64>,
}

impl SkipReason {
    /// What holds the session back and what to do about it.
    pub fn sentence(&self) -> String {
        match self {
            SkipReason::Uncommitted { files } => format!(
                "{files} file{} with uncommitted changes; commit and push them, then the rule can archive this session",
                if *files == 1 { "" } else { "s" }
            ),
            SkipReason::Unpushed { branches } => format!(
                "unpushed commits on {}; push them, then the rule can archive this session",
                branches.join(", ")
            ),
            SkipReason::WorkUnknown => "Humata has not seen whether its work is committed and pushed; open it so the state is read, then the rule can archive it".to_string(),
        }
    }
}

impl Rule {
    /// Which rule it was, with its days, for the Activity entry (LD-18).
    pub fn sentence(&self) -> String {
        match self {
            Rule::PrReviews { days } => {
                format!("Archive PR reviews {days} days after the PR merges or closes")
            }
            Rule::Inactive { days } => format!("Archive sessions inactive for {days} days"),
        }
    }
}

fn due_at(base: i64, days: u32) -> i64 {
    base.saturating_add(i64::from(days).saturating_mul(DAY_MS))
}

fn last_activity(s: &SessionFacts) -> i64 {
    [s.last_output_unix_ms, s.last_input_unix_ms, s.last_opened_unix_ms]
        .into_iter()
        .flatten()
        .fold(s.created_unix_ms, i64::max)
}

/// Decides what the rules archive and skip at `now_unix_ms`, and when the
/// next session becomes due.
pub fn decide(rules: &Rules, sessions: &[SessionFacts], now_unix_ms: i64) -> Decision {
    let mut decision = Decision {
        archive: Vec::new(),
        skips: Vec::new(),
        next_due_unix_ms: None,
    };
    for s in sessions.iter().filter(|s| !s.archived) {
        let mut moments = Vec::new();
        if rules.archive_pr_reviews_after_days > 0 && s.is_pr_review {
            if let Some(closed) = s.pr_closed_unix_ms {
                let days = rules.archive_pr_reviews_after_days;
                moments.push((Rule::PrReviews { days }, due_at(closed, days)));
            }
        }
        if rules.archive_idle_sessions_after_days > 0 {
            let days = rules.archive_idle_sessions_after_days;
            moments.push((Rule::Inactive { days }, due_at(last_activity(s), days)));
        }
        if let Some((rule, _)) = moments.iter().find(|(_, at)| *at <= now_unix_ms) {
            match &s.work {
                Work::Safe => decision.archive.push(Archive {
                    session: s.id.clone(),
                    rule: *rule,
                }),
                work => decision.skips.push(Skip {
                    session: s.id.clone(),
                    title: s.title.clone(),
                    rule: *rule,
                    reason: match work {
                        Work::Uncommitted { files } => SkipReason::Uncommitted { files: *files },
                        Work::Unpushed { branches } => SkipReason::Unpushed {
                            branches: branches.clone(),
                        },
                        _ => SkipReason::WorkUnknown,
                    },
                }),
            }
        } else if let Some(at) = moments.iter().map(|(_, at)| *at).min() {
            decision.next_due_unix_ms =
                Some(decision.next_due_unix_ms.map_or(at, |cur| cur.min(at)));
        }
    }
    decision
}

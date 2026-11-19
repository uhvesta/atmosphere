use crate::day::Day;
use crate::model::{Effort, Impact, OldColumn, State, Todo};
use std::cmp::Ordering;

/// Why a row sits where it does, beyond what the row already shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    Pinned,
    PinnedWasVip,
    HighImpactSmallJob,
    HighImpact,
    Blocks(String),
    WaitingOnYou(u32),
    WasAt {
        position: u32,
        column: OldColumn,
        board: Option<String>,
    },
    Added(u32),
    WaitsOn(String),
    ComesBack(Day),
    NoLongerWaiting,
}

fn ordinal(n: u32) -> String {
    let suffix = match (n % 100, n % 10) {
        (11..=13, _) => "th",
        (_, 1) => "st",
        (_, 2) => "nd",
        (_, 3) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

fn column_name(c: OldColumn) -> &'static str {
    match c {
        OldColumn::Backlog => "Backlog",
        OldColumn::ToDo => "To do",
        OldColumn::InProgress => "In progress",
        OldColumn::InReview => "In review",
        OldColumn::Done => "Done",
    }
}

impl Reason {
    /// The product wording shown on the row.
    pub fn text(&self) -> String {
        match self {
            Reason::Pinned => "Pinned".to_owned(),
            Reason::PinnedWasVip => "Pinned · was VIP".to_owned(),
            Reason::HighImpactSmallJob => "High impact for a small job".to_owned(),
            Reason::HighImpact => "High impact".to_owned(),
            Reason::Blocks(k) => format!("{k} waits on it"),
            Reason::WaitingOnYou(1) => "Waiting on you for 1 day".to_owned(),
            Reason::WaitingOnYou(n) => format!("Waiting on you for {n} days"),
            Reason::WasAt {
                position,
                column,
                board,
            } => {
                let mut s = format!("Was {} in {}", ordinal(*position), column_name(*column));
                if let Some(b) = board {
                    s.push_str(" on ");
                    s.push_str(b);
                }
                s
            }
            Reason::Added(0) => "Added today".to_owned(),
            Reason::Added(1) => "Added 1 day ago".to_owned(),
            Reason::Added(n) => format!("Added {n} days ago"),
            Reason::WaitsOn(k) => format!("Waits on {k}"),
            Reason::ComesBack(d) => format!("Comes back {}", d.short()),
            Reason::NoLongerWaiting => "No longer waiting".to_owned(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub key: String,
    /// 1-based rank; Next rows only.
    pub position: Option<u32>,
    pub reasons: Vec<Reason>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ranked {
    pub needs_attention: Vec<Row>,
    pub in_progress: Vec<Row>,
    pub next: Vec<Row>,
    pub waiting: Vec<Row>,
    pub hidden: Vec<Row>,
    pub closed: Vec<Row>,
    /// Key of the first Next row.
    pub do_next: Option<String>,
}

fn is_overdue(t: &Todo, today: Day) -> bool {
    t.due.is_some_and(|d| d < today)
}

fn is_hidden(t: &Todo, today: Day) -> bool {
    !is_overdue(t, today) && t.hide_until.is_some_and(|h| h > today)
}

fn score(t: &Todo, today: Day) -> i64 {
    let due = match t.due.map(|d| today.days_until(d)) {
        Some(d) if d < 0 => 100,
        Some(0) => 90,
        Some(d @ 1..=7) => 80 - 5 * i64::from(d),
        Some(d @ 8..=30) => 30 - i64::from(d),
        _ => 0,
    };
    let value = match t.impact {
        None => 0,
        Some(i) => {
            let impact = match i {
                Impact::High => 3,
                Impact::Medium => 2,
                Impact::Low => 1,
            };
            let effort = match t.effort {
                Some(Effort::S) => 1,
                Some(Effort::M) | None => 2,
                Some(Effort::L) => 3,
            };
            10 * impact / effort
        }
    };
    let review = t
        .review
        .as_ref()
        .map_or(0, |r| (60 + 10 * i64::from(r.waiting_days)).min(100));
    due + value + review
}

type StartOrder<'a> = (u8, u32, Option<&'a str>);

fn start_order(t: &Todo) -> StartOrder<'_> {
    match &t.start {
        Some(s) => (s.column.tie_order(), s.position, s.board.as_deref()),
        None => (u8::MAX, u32::MAX, None),
    }
}

fn by_created(a: &Todo, b: &Todo) -> Ordering {
    a.created.cmp(&b.created).then_with(|| a.key.cmp(&b.key))
}

fn compare_next(a: &(&Todo, i64), b: &(&Todo, i64)) -> Ordering {
    let (ta, sa) = a;
    let (tb, sb) = b;
    match (ta.pin, tb.pin) {
        (Some(x), Some(y)) if x != y => return x.cmp(&y),
        (Some(_), None) => return Ordering::Less,
        (None, Some(_)) => return Ordering::Greater,
        _ => {}
    }
    sb.cmp(sa)
        .then_with(|| start_order(ta).cmp(&start_order(tb)))
        .then_with(|| by_created(ta, tb))
}

fn first_open_dependency<'a>(t: &'a Todo, items: &[Todo]) -> Option<&'a str> {
    t.depends_on
        .iter()
        .find(|k| items.iter().any(|o| &o.key == *k && o.state != State::Closed))
        .map(String::as_str)
}

fn next_reasons(t: &Todo, items: &[Todo], today: Day) -> Vec<Reason> {
    let mut reasons = Vec::new();
    if t.pin.is_some() {
        let vip = t.start.as_ref().is_some_and(|s| s.vip);
        reasons.push(if vip {
            Reason::PinnedWasVip
        } else {
            Reason::Pinned
        });
    }
    match (t.impact, t.effort) {
        (Some(Impact::High), Some(Effort::S)) => reasons.push(Reason::HighImpactSmallJob),
        (Some(Impact::High), _) => reasons.push(Reason::HighImpact),
        _ => {}
    }
    if !t.key.is_empty() {
        for o in items {
            if o.state != State::Closed && o.depends_on.contains(&t.key) {
                reasons.push(Reason::Blocks(o.key.clone()));
            }
        }
    }
    if let Some(r) = &t.review {
        reasons.push(Reason::WaitingOnYou(r.waiting_days));
    } else if t.due.is_none() && t.effort.is_none() && t.impact.is_none() && t.pin.is_none() {
        match &t.start {
            Some(s) => reasons.push(Reason::WasAt {
                position: s.position,
                column: s.column,
                board: s.board.clone(),
            }),
            None => reasons.push(Reason::Added(
                u32::try_from(t.created.days_until(today).max(0)).unwrap_or(0),
            )),
        }
    }
    reasons
}

fn row(t: &Todo, reasons: Vec<Reason>) -> Row {
    Row {
        key: t.key.clone(),
        position: None,
        reasons,
    }
}

/// Groups and orders the list. See the module tests for the rules in action.
pub fn rank(items: &[Todo], today: Day) -> Ranked {
    let mut needs: Vec<&Todo> = Vec::new();
    let mut progress: Vec<&Todo> = Vec::new();
    let mut closed: Vec<&Todo> = Vec::new();
    let mut hidden: Vec<&Todo> = Vec::new();
    let mut waiting: Vec<(&Todo, &str)> = Vec::new();
    let mut next: Vec<(&Todo, i64)> = Vec::new();
    let window_start = today.add_days(-6);

    for t in items {
        if t.state != State::Closed && (t.ask.is_some() || t.suggested) {
            needs.push(t);
            continue;
        }
        match t.state {
            State::Closed => {
                if t.closed.is_none_or(|c| c >= window_start && c <= today) {
                    closed.push(t);
                }
                continue;
            }
            State::InProgress => {
                progress.push(t);
                continue;
            }
            State::Open => {}
        }
        if is_overdue(t, today) {
            next.push((t, score(t, today)));
        } else if is_hidden(t, today) {
            hidden.push(t);
        } else if let Some(k) = first_open_dependency(t, items) {
            waiting.push((t, k));
        } else {
            next.push((t, score(t, today)));
        }
    }

    needs.sort_by(|a, b| by_created(a, b));
    progress.sort_by(|a, b| by_created(a, b));
    hidden.sort_by(|a, b| a.hide_until.cmp(&b.hide_until).then_with(|| by_created(a, b)));
    waiting.sort_by(|a, b| by_created(a.0, b.0));
    closed.sort_by(|a, b| b.closed.cmp(&a.closed).then_with(|| a.key.cmp(&b.key)));
    next.sort_by(compare_next);

    let next_rows: Vec<Row> = next
        .iter()
        .enumerate()
        .map(|(i, (t, _))| Row {
            position: Some(u32::try_from(i + 1).unwrap_or(u32::MAX)),
            ..row(t, next_reasons(t, items, today))
        })
        .collect();

    Ranked {
        needs_attention: needs.into_iter().map(|t| row(t, Vec::new())).collect(),
        in_progress: progress.into_iter().map(|t| row(t, Vec::new())).collect(),
        do_next: next_rows.first().map(|r| r.key.clone()),
        next: next_rows,
        waiting: waiting
            .into_iter()
            .map(|(t, k)| row(t, vec![Reason::WaitsOn(k.to_owned())]))
            .collect(),
        hidden: hidden
            .into_iter()
            .map(|t| {
                let reasons = t.hide_until.map(Reason::ComesBack).into_iter().collect();
                row(t, reasons)
            })
            .collect(),
        closed: closed.into_iter().map(|t| row(t, Vec::new())).collect(),
    }
}

/// Whether the item counts in the header: open, not suggested, not hidden.
pub(crate) fn counts_as_open(t: &Todo, today: Day) -> bool {
    t.state != State::Closed && !t.suggested && !is_hidden(t, today)
}

/// "10 open across 4 channels": (open items, distinct channels among them).
pub fn header(items: &[Todo], today: Day) -> (usize, usize) {
    let mut channels: Vec<&str> = Vec::new();
    let mut open = 0;
    for t in items.iter().filter(|t| counts_as_open(t, today)) {
        open += 1;
        if let Some(c) = t.channel.as_deref()
            && !channels.contains(&c)
        {
            channels.push(c);
        }
    }
    (open, channels.len())
}

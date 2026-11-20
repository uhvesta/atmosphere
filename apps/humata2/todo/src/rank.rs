use crate::day::Day;
use crate::model::{Effort, FuzzyDue, HeldBackBy, Impact, Need, OldColumn, SortBy, State, Todo};
use std::cmp::Ordering;
use std::collections::HashMap;

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
    ExternalBlock(String),
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
            Reason::ExternalBlock(reason) => {
                if reason.is_empty() {
                    "Blocked externally".to_owned()
                } else {
                    format!("Waiting: {reason}")
                }
            }
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
    /// Why this item is not ranked higher (unset fields, blockers, watching PR).
    pub held_back: Option<HeldBackBy>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ranked {
    pub needs_attention: Vec<Row>,
    pub in_progress: Vec<Row>,
    pub watching_pr: Vec<Row>,
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

fn need_score(t: &Todo, today: Day) -> i64 {
    let Some(need) = t.need else {
        return 0;
    };
    let age_days = i64::from(t.created.days_until(today).max(0));
    match need {
        Need::Critical => 40 + (age_days * 2).min(40),
        Need::Expected => 24 + age_days.min(24),
        Need::NiceToHave => 10 + (age_days / 2).min(12),
        Need::Idea => 2 + (age_days / 7).min(4),
    }
}

fn base_score(t: &Todo, today: Day) -> i64 {
    let due = match t.due.map(|d| today.days_until(d)) {
        Some(d) if d < 0 => 100,
        Some(0) => 90,
        Some(d @ 1..=7) => 80 - 5 * i64::from(d),
        Some(d @ 8..=30) => 30 - i64::from(d),
        Some(_) => 0,
        None => match t.fuzzy_due {
            Some(FuzzyDue::InAFewDays) => 25,
            Some(FuzzyDue::NextWeek) => 15,
            Some(FuzzyDue::NextMonth) => 5,
            None => 0,
        },
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
    due + need_score(t, today) + value + review
}

/// Computes `ceil(1.1 * s)` for non-negative scores.
fn inherit_boost(s: i64) -> i64 {
    if s <= 0 {
        0
    } else {
        (s.saturating_mul(11).saturating_add(9)) / 10
    }
}

/// Computes cycle-safe transitive 1.1x blocker priority inheritance:
/// `effective(u) = max(base(u), max_{v depends on u} ceil(1.1 * effective(v)))`.
fn effective_scores(items: &[Todo], today: Day) -> Vec<i64> {
    let n = items.len();
    let bases: Vec<i64> = items.iter().map(|t| base_score(t, today)).collect();
    let mut key_to_idx: HashMap<&str, usize> = HashMap::with_capacity(n);
    for (i, t) in items.iter().enumerate() {
        if !t.key.is_empty() {
            key_to_idx.insert(t.key.as_str(), i);
        }
    }

    // Reverse edges: dependents[u] = active items v that depend on u.
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (v_idx, v) in items.iter().enumerate() {
        if !v.state.is_active() {
            continue;
        }
        for dep_key in &v.depends_on {
            if let Some(&u_idx) = key_to_idx.get(dep_key.as_str()) {
                if u_idx != v_idx && items[u_idx].state.is_active() {
                    dependents[u_idx].push(v_idx);
                }
            }
        }
    }

    fn dfs(
        u: usize,
        bases: &[i64],
        dependents: &[Vec<usize>],
        memo: &mut [Option<i64>],
        on_stack: &mut [bool],
    ) -> (i64, bool) {
        if let Some(val) = memo[u] {
            return (val, false);
        }
        on_stack[u] = true;
        let mut best = bases[u];
        let mut hit_cycle = false;
        for &v in &dependents[u] {
            if on_stack[v] {
                hit_cycle = true;
                continue;
            }
            let (v_eff, v_cycle) = dfs(v, bases, dependents, memo, on_stack);
            if v_cycle {
                hit_cycle = true;
            }
            best = best.max(inherit_boost(v_eff));
        }
        on_stack[u] = false;
        if !hit_cycle {
            memo[u] = Some(best);
        }
        (best, hit_cycle)
    }

    let mut memo: Vec<Option<i64>> = vec![None; n];
    let mut on_stack = vec![false; n];
    let mut out = vec![0i64; n];
    for u in 0..n {
        out[u] = dfs(u, &bases, &dependents, &mut memo, &mut on_stack).0;
    }
    out
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

fn compare_priority(a: &(&Todo, i64), b: &(&Todo, i64), today: Day) -> Ordering {
    let (ta, sa) = a;
    let (tb, sb) = b;
    match (ta.pin, tb.pin) {
        (Some(x), Some(y)) if x != y => return x.cmp(&y),
        (Some(_), None) => return Ordering::Less,
        (None, Some(_)) => return Ordering::Greater,
        _ => {}
    }
    match (is_overdue(ta, today), is_overdue(tb, today)) {
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        _ => {}
    }
    sb.cmp(sa)
        .then_with(|| start_order(ta).cmp(&start_order(tb)))
        .then_with(|| by_created(ta, tb))
}

fn due_sort_key(t: &Todo, today: Day) -> (u8, i32) {
    if let Some(d) = t.due {
        (0, today.days_until(d))
    } else if let Some(fd) = t.fuzzy_due {
        (1, fd.approx_days())
    } else {
        (2, i32::MAX)
    }
}

fn effort_sort_key(e: Option<Effort>) -> u8 {
    match e {
        Some(Effort::S) => 0,
        Some(Effort::M) => 1,
        Some(Effort::L) => 2,
        None => 3,
    }
}

fn impact_sort_key(i: Option<Impact>) -> u8 {
    match i {
        Some(Impact::High) => 0,
        Some(Impact::Medium) => 1,
        Some(Impact::Low) => 2,
        None => 3,
    }
}

fn need_sort_key(n: Option<Need>) -> u8 {
    match n {
        Some(Need::Critical) => 0,
        Some(Need::Expected) => 1,
        Some(Need::NiceToHave) => 2,
        Some(Need::Idea) => 3,
        None => 4,
    }
}

fn compare_next(a: &(&Todo, i64), b: &(&Todo, i64), sort: SortBy, today: Day) -> Ordering {
    match sort {
        SortBy::Priority => compare_priority(a, b, today),
        SortBy::DueSoonest => due_sort_key(a.0, today)
            .cmp(&due_sort_key(b.0, today))
            .then_with(|| compare_priority(a, b, today)),
        SortBy::MostNeeded => need_sort_key(a.0.need)
            .cmp(&need_sort_key(b.0.need))
            .then_with(|| compare_priority(a, b, today)),
        SortBy::SmallestEffort => effort_sort_key(a.0.effort)
            .cmp(&effort_sort_key(b.0.effort))
            .then_with(|| compare_priority(a, b, today)),
        SortBy::LargestImpact => impact_sort_key(a.0.impact)
            .cmp(&impact_sort_key(b.0.impact))
            .then_with(|| compare_priority(a, b, today)),
    }
}

fn first_open_dependency<'a>(t: &'a Todo, items: &[Todo]) -> Option<&'a str> {
    t.depends_on
        .iter()
        .find(|k| items.iter().any(|o| &o.key == *k && o.state.is_active()))
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
            if o.state.is_active() && o.depends_on.contains(&t.key) {
                reasons.push(Reason::Blocks(o.key.clone()));
            }
        }
    }
    if let Some(r) = &t.review {
        reasons.push(Reason::WaitingOnYou(r.waiting_days));
    } else if t.due.is_none()
        && t.fuzzy_due.is_none()
        && t.need.is_none()
        && t.effort.is_none()
        && t.impact.is_none()
        && t.pin.is_none()
    {
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

fn held_back_for_next(t: &Todo) -> Option<HeldBackBy> {
    if t.pin.is_some() {
        return None;
    }
    let mut factors = Vec::new();
    if t.due.is_none() && t.fuzzy_due.is_none() {
        factors.push("No due date".to_owned());
    }
    if t.need.is_none() {
        factors.push("No need set".to_owned());
    }
    if t.impact.is_none() {
        factors.push("No impact set".to_owned());
    }
    if t.effort.is_none() {
        factors.push("No effort set".to_owned());
    }
    if factors.is_empty() {
        None
    } else {
        Some(HeldBackBy {
            summary: format!("Held back by: {}", factors.join(", ")),
            factors,
        })
    }
}

fn row(t: &Todo, reasons: Vec<Reason>, held_back: Option<HeldBackBy>) -> Row {
    Row {
        key: t.key.clone(),
        position: None,
        reasons,
        held_back,
    }
}

fn needs_attention_item(t: &Todo) -> bool {
    t.state.is_active()
        && (t.ask.is_some()
            || t.suggested
            || !t.blocker_gone.is_empty()
            || !t.lifecycle_prompts.is_empty()
            || t.stuck_agent.is_some())
}

/// Groups and orders the list using default Priority sorting.
pub fn rank(items: &[Todo], today: Day) -> Ranked {
    rank_with(items, SortBy::Priority, today)
}

/// Groups and orders the list with the requested `SortBy` mode.
pub fn rank_with(items: &[Todo], sort: SortBy, today: Day) -> Ranked {
    let eff_scores = effective_scores(items, today);

    let mut needs: Vec<&Todo> = Vec::new();
    let mut progress: Vec<&Todo> = Vec::new();
    let mut watching: Vec<&Todo> = Vec::new();
    let mut closed: Vec<&Todo> = Vec::new();
    let mut hidden: Vec<&Todo> = Vec::new();
    let mut waiting: Vec<(&Todo, Reason, HeldBackBy)> = Vec::new();
    let mut next: Vec<(&Todo, i64)> = Vec::new();
    let window_start = today.add_days(-6);

    for (idx, t) in items.iter().enumerate() {
        if needs_attention_item(t) {
            needs.push(t);
            continue;
        }
        match t.state {
            State::Closed | State::WontDo | State::Became => {
                if t.closed.is_none_or(|c| c >= window_start && c <= today) {
                    closed.push(t);
                }
                continue;
            }
            State::InProgress => {
                progress.push(t);
                continue;
            }
            State::WatchingPr => {
                watching.push(t);
                continue;
            }
            State::Open => {}
        }
        if is_overdue(t, today) {
            next.push((t, eff_scores[idx]));
        } else if is_hidden(t, today) {
            hidden.push(t);
        } else if let Some(k) = first_open_dependency(t, items) {
            let reason = Reason::WaitsOn(k.to_owned());
            let hb = HeldBackBy {
                summary: format!("Blocked by {k}"),
                factors: vec![format!("Waits on {k}")],
            };
            waiting.push((t, reason, hb));
        } else if let Some(ext) = &t.external_block {
            let reason = Reason::ExternalBlock(ext.reason.clone());
            let factor = if ext.reason.is_empty() {
                "Blocked externally".to_owned()
            } else {
                ext.reason.clone()
            };
            let hb = HeldBackBy {
                summary: format!("Blocked: {factor}"),
                factors: vec![factor],
            };
            waiting.push((t, reason, hb));
        } else {
            next.push((t, eff_scores[idx]));
        }
    }

    needs.sort_by(|a, b| by_created(a, b));
    progress.sort_by(|a, b| by_created(a, b));
    watching.sort_by(|a, b| by_created(a, b));
    hidden.sort_by(|a, b| a.hide_until.cmp(&b.hide_until).then_with(|| by_created(a, b)));
    waiting.sort_by(|a, b| by_created(a.0, b.0));
    closed.sort_by(|a, b| b.closed.cmp(&a.closed).then_with(|| a.key.cmp(&b.key)));
    next.sort_by(|a, b| compare_next(a, b, sort, today));

    let next_rows: Vec<Row> = next
        .iter()
        .enumerate()
        .map(|(i, (t, _))| Row {
            position: Some(u32::try_from(i + 1).unwrap_or(u32::MAX)),
            ..row(t, next_reasons(t, items, today), held_back_for_next(t))
        })
        .collect();

    Ranked {
        needs_attention: needs.into_iter().map(|t| row(t, Vec::new(), None)).collect(),
        in_progress: progress
            .into_iter()
            .map(|t| row(t, Vec::new(), None))
            .collect(),
        watching_pr: watching
            .into_iter()
            .map(|t| {
                row(
                    t,
                    Vec::new(),
                    Some(HeldBackBy {
                        summary: "Watching PR".to_owned(),
                        factors: vec!["Waiting on PR activity".to_owned()],
                    }),
                )
            })
            .collect(),
        do_next: next_rows.first().map(|r| r.key.clone()),
        next: next_rows,
        waiting: waiting
            .into_iter()
            .map(|(t, reason, hb)| row(t, vec![reason], Some(hb)))
            .collect(),
        hidden: hidden
            .into_iter()
            .map(|t| {
                let reasons = t.hide_until.map(Reason::ComesBack).into_iter().collect();
                row(t, reasons, None)
            })
            .collect(),
        closed: closed
            .into_iter()
            .map(|t| row(t, Vec::new(), None))
            .collect(),
    }
}

/// Whether the item counts in the header: active, not suggested, not hidden.
pub(crate) fn counts_as_open(t: &Todo, today: Day) -> bool {
    t.state.is_active() && !t.suggested && !is_hidden(t, today)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BlockerGone, ExternalBlock, LifecyclePrompt};

    fn d(m: u32, day: u32) -> Day {
        Day::from_ymd(2026, m, day).unwrap()
    }

    fn today() -> Day {
        d(11, 19)
    }

    fn keys(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|r| r.key.as_str()).collect()
    }

    #[test]
    fn need_order_and_need_scaled_aging() {
        let mut crit = Todo::new("C-1", "crit", d(11, 10));
        crit.need = Some(Need::Critical);
        let mut exp = Todo::new("E-1", "exp", d(11, 10));
        exp.need = Some(Need::Expected);
        let mut nice = Todo::new("N-1", "nice", d(11, 10));
        nice.need = Some(Need::NiceToHave);
        let mut idea = Todo::new("I-1", "idea", d(11, 10));
        idea.need = Some(Need::Idea);

        let r = rank(&[idea, nice, exp, crit], today());
        assert_eq!(keys(&r.next), vec!["C-1", "E-1", "N-1", "I-1"]);

        // Aging raises an older Critical above a brand-new Critical.
        let mut old_crit = Todo::new("C-OLD", "old", d(11, 1));
        old_crit.need = Some(Need::Critical);
        let mut new_crit = Todo::new("C-NEW", "new", d(11, 19));
        new_crit.need = Some(Need::Critical);
        assert!(base_score(&old_crit, today()) > base_score(&new_crit, today()));
    }

    #[test]
    fn fuzzy_due_gives_soft_bonus_and_is_never_overdue() {
        let mut few = Todo::new("F-1", "few", d(11, 1));
        few.fuzzy_due = Some(FuzzyDue::InAFewDays);
        let mut week = Todo::new("F-2", "week", d(11, 1));
        week.fuzzy_due = Some(FuzzyDue::NextWeek);
        let mut month = Todo::new("F-3", "month", d(11, 1));
        month.fuzzy_due = Some(FuzzyDue::NextMonth);
        let bare = Todo::new("F-4", "bare", d(11, 1));

        assert!(!is_overdue(&few, today()));
        let r = rank(&[bare, month, week, few], today());
        assert_eq!(keys(&r.next), vec!["F-1", "F-2", "F-3", "F-4"]);
    }

    #[test]
    fn transitive_blocker_priority_inheritance_and_cycle_safety() {
        // C depends on B, B depends on A. C is high-priority; A should inherit ceil(1.1 * ceil(1.1 * base(C))).
        let a = Todo::new("A-1", "root blocker", d(11, 18));
        let mut b = Todo::new("B-1", "mid", d(11, 18));
        b.depends_on = vec!["A-1".to_owned()];
        let mut c = Todo::new("C-1", "urgent leaf", d(11, 18));
        c.depends_on = vec!["B-1".to_owned()];
        c.due = Some(d(11, 20)); // tomorrow: 75
        let mut unrelated = Todo::new("U-1", "unrelated", d(11, 18));
        unrelated.due = Some(d(11, 20)); // 75

        let items = vec![a, b, c, unrelated];
        let eff = effective_scores(&items, today());
        assert_eq!(eff[2], 75);
        assert_eq!(eff[1], 83); // ceil(1.1 * 75) = 82.5 -> 83
        assert_eq!(eff[0], 92); // ceil(1.1 * 83) = 91.3 -> 92

        let r = rank(&items, today());
        // A-1 beats U-1 because it transitively unblocks C-1.
        assert_eq!(keys(&r.next), vec!["A-1", "U-1"]);
        assert_eq!(keys(&r.waiting), vec!["B-1", "C-1"]);

        // Cycle safety: X <-> Y must not infinite-loop.
        let mut x = Todo::new("X-1", "x", d(11, 18));
        x.depends_on = vec!["Y-1".to_owned()];
        x.need = Some(Need::Expected);
        let mut y = Todo::new("Y-1", "y", d(11, 18));
        y.depends_on = vec!["X-1".to_owned()];
        let cyc = effective_scores(&[x, y], today());
        assert!(cyc[0] > 0 && cyc[1] >= cyc[0]);
    }

    #[test]
    fn sort_by_modes_reorder_next() {
        let mut due_soon = Todo::new("D-1", "due", d(11, 10));
        due_soon.due = Some(d(11, 21));
        due_soon.effort = Some(Effort::L);
        due_soon.impact = Some(Impact::Low);

        let mut tiny = Todo::new("S-1", "small", d(11, 10));
        tiny.effort = Some(Effort::S);
        tiny.need = Some(Need::NiceToHave);

        let mut big_impact = Todo::new("I-1", "impact", d(11, 10));
        big_impact.impact = Some(Impact::High);
        big_impact.need = Some(Need::Critical);

        let items = vec![due_soon, tiny, big_impact];
        assert_eq!(
            keys(&rank_with(&items, SortBy::DueSoonest, today()).next)[0],
            "D-1"
        );
        assert_eq!(
            keys(&rank_with(&items, SortBy::SmallestEffort, today()).next)[0],
            "S-1"
        );
        assert_eq!(
            keys(&rank_with(&items, SortBy::LargestImpact, today()).next)[0],
            "I-1"
        );
        assert_eq!(
            keys(&rank_with(&items, SortBy::MostNeeded, today()).next)[0],
            "I-1"
        );
    }

    #[test]
    fn terminal_watching_pr_external_block_and_needs_attention_groupings() {
        let mut wont = Todo::new("W-1", "wont", d(11, 10));
        wont.state = State::WontDo;
        wont.closed = Some(today());

        let mut became = Todo::new("B-1", "became", d(11, 10));
        became.state = State::Became;
        became.closed = Some(today());

        let mut watch = Todo::new("P-1", "pr", d(11, 10));
        watch.state = State::WatchingPr;

        let mut ext = Todo::new("E-1", "ext", d(11, 10));
        ext.external_block = Some(ExternalBlock {
            reason: "Waiting on vendor".to_owned(),
            since: Some(d(11, 15)),
        });

        let mut bg = Todo::new("BG-1", "bg", d(11, 10));
        bg.blocker_gone.push(BlockerGone {
            blocker_key: "W-1".to_owned(),
            blocker_state: State::WontDo,
            replacement_keys: Vec::new(),
        });

        let mut lp = Todo::new("LP-1", "lp", d(11, 10));
        lp.lifecycle_prompts.push(LifecyclePrompt {
            id: "p1".to_owned(),
            kind: "pr_merged".to_owned(),
            title: "PR merged — close?".to_owned(),
            ..LifecyclePrompt::default()
        });

        let bare = Todo::new("O-1", "open", d(11, 10));

        let r = rank(&[wont, became, watch, ext, bg, lp, bare], today());
        assert_eq!(keys(&r.closed), vec!["B-1", "W-1"]);
        assert_eq!(keys(&r.watching_pr), vec!["P-1"]);
        assert!(r.watching_pr[0].held_back.is_some());
        assert_eq!(keys(&r.waiting), vec!["E-1"]);
        assert_eq!(
            r.waiting[0].reasons,
            vec![Reason::ExternalBlock("Waiting on vendor".to_owned())]
        );
        assert!(r.waiting[0].held_back.is_some());
        assert_eq!(keys(&r.needs_attention), vec!["BG-1", "LP-1"]);
        assert_eq!(keys(&r.next), vec!["O-1"]);
        let hb = r.next[0].held_back.as_ref().unwrap();
        assert_eq!(hb.factors.len(), 4);
    }
}

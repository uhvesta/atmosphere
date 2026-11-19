use crate::day::Day;
use crate::model::{OldColumn, Start, State, Todo};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OldItem {
    pub key: String,
    pub title: String,
    pub column: OldColumn,
    /// None is the global board.
    pub board: Option<String>,
    /// Old lexical rank within its column.
    pub rank: String,
    pub vip: bool,
    pub pinned: bool,
    pub channel: Option<String>,
    pub sessions: Vec<String>,
    pub session_archived: bool,
    pub created: Day,
    pub done_at: Option<Day>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub items: usize,
    pub open: usize,
    pub in_progress: usize,
    pub closed: usize,
    /// Distinct old boards (the global board counts as one).
    pub boards: usize,
}

/// Moves old board items into the list, keeping where each one was.
pub fn migrate(old: Vec<OldItem>) -> Vec<Todo> {
    migrate_with_summary(old).0
}

pub fn migrate_with_summary(old: Vec<OldItem>) -> (Vec<Todo>, Summary) {
    let mut boards: Vec<Option<&str>> = Vec::new();
    for o in &old {
        if !boards.contains(&o.board.as_deref()) {
            boards.push(o.board.as_deref());
        }
    }
    let board_count = boards.len();

    // Position within (board, column) by old rank.
    let mut order: Vec<usize> = (0..old.len()).collect();
    order.sort_by(|&a, &b| {
        let (x, y) = (&old[a], &old[b]);
        x.board
            .cmp(&y.board)
            .then(x.column.cmp(&y.column))
            .then_with(|| x.rank.cmp(&y.rank))
            .then_with(|| x.key.cmp(&y.key))
    });
    let mut positions = vec![0u32; old.len()];
    let mut last: Option<(&Option<String>, OldColumn)> = None;
    let mut n = 0u32;
    for &i in &order {
        let o = &old[i];
        if last != Some((&o.board, o.column)) {
            n = 0;
            last = Some((&o.board, o.column));
        }
        n += 1;
        if let Some(p) = positions.get_mut(i) {
            *p = n;
        }
    }

    let mut todos: Vec<(Todo, u32, bool)> = old
        .iter()
        .zip(positions)
        .map(|(o, position)| {
            let state = if o.session_archived || o.column == OldColumn::Done {
                State::Closed
            } else if matches!(o.column, OldColumn::InProgress | OldColumn::InReview) {
                State::InProgress
            } else {
                State::Open
            };
            let mut t = Todo::new(&o.key, &o.title, o.created);
            t.state = state;
            t.channel = o.channel.clone();
            t.sessions = o.sessions.clone();
            t.closed = (state == State::Closed)
                .then(|| o.done_at.unwrap_or(o.created));
            t.start = Some(Start {
                column: o.column,
                position,
                board: o.board.clone(),
                vip: o.vip,
            });
            (t, position, o.pinned)
        })
        .collect();

    // Start order: column (In progress first), position, global board before channels.
    todos.sort_by(|(a, pa, _), (b, pb, _)| {
        let (sa, sb) = (a.start.as_ref(), b.start.as_ref());
        let col = |s: Option<&Start>| s.map_or(u8::MAX, |s| s.column.tie_order());
        col(sa)
            .cmp(&col(sb))
            .then(pa.cmp(pb))
            .then_with(|| sa.map(|s| &s.board).cmp(&sb.map(|s| &s.board)))
            .then_with(|| a.key.cmp(&b.key))
    });

    // Pins: VIP first, then pinned, each in start order. Closed items are not pinned.
    let mut next_pin = 0u32;
    for vip_pass in [true, false] {
        for (t, _, pinned) in todos.iter_mut() {
            let is_vip = t.start.as_ref().is_some_and(|s| s.vip);
            if t.state != State::Closed && (is_vip || *pinned) && is_vip == vip_pass {
                t.pin = Some(next_pin);
                next_pin += 1;
            }
        }
    }

    let out: Vec<Todo> = todos.into_iter().map(|(t, _, _)| t).collect();
    let count = |s: State| out.iter().filter(|t| t.state == s).count();
    let summary = Summary {
        items: out.len(),
        open: count(State::Open),
        in_progress: count(State::InProgress),
        closed: count(State::Closed),
        boards: board_count,
    };
    (out, summary)
}

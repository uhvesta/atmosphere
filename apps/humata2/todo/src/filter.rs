use crate::day::Day;
use crate::model::{Effort, Impact, Todo};
use crate::rank::counts_as_open;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChannelFilter {
    Channel(String),
    NoChannel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionFilter {
    Session(String),
    /// Items not in any session yet.
    NoneYet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DueFilter {
    Overdue,
    Next7Days,
    /// Due more than 7 days from today.
    AfterThisWeek,
    /// No due date.
    NoDate,
}

/// What an item may be, for the "Only" filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Only {
    /// An agent asks about it, or it is suggested.
    NeedsAttention,
    /// A pull request waiting for review.
    PullRequests,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    pub channel: Option<ChannelFilter>,
    pub session: Option<SessionFilter>,
    pub due: Vec<DueFilter>,
    pub tags: Vec<String>,
    pub search: String,
    /// Any of these efforts.
    pub effort: Vec<Effort>,
    /// Any of these impacts.
    pub impact: Vec<Impact>,
    /// Any of these kinds of item.
    pub only: Vec<Only>,
}

fn matches_due(t: &Todo, f: &Filter, today: Day) -> bool {
    if f.due.is_empty() {
        return true;
    }
    let days = t.due.map(|due| today.days_until(due));
    f.due.iter().any(|k| match (k, days) {
        (DueFilter::NoDate, d) => d.is_none(),
        (_, None) => false,
        (DueFilter::Overdue, Some(d)) => d < 0,
        (DueFilter::Next7Days, Some(d)) => (0..=7).contains(&d),
        (DueFilter::AfterThisWeek, Some(d)) => d > 7,
    })
}

fn matches_rest(t: &Todo, f: &Filter) -> bool {
    let channel_ok = match &f.channel {
        None => true,
        Some(ChannelFilter::NoChannel) => t.channel.is_none(),
        Some(ChannelFilter::Channel(c)) => t.channel.as_deref() == Some(c.as_str()),
    };
    let session_ok = match &f.session {
        None => true,
        Some(SessionFilter::NoneYet) => t.sessions.is_empty(),
        Some(SessionFilter::Session(s)) => t.sessions.contains(s),
    };
    let tags_ok = f.tags.is_empty()
        || f.tags
            .iter()
            .any(|w| t.tags.iter().any(|x| x.eq_ignore_ascii_case(w)));
    let needle = f.search.trim().to_lowercase();
    let search_ok = needle.is_empty()
        || t.title.to_lowercase().contains(&needle)
        || t.key.to_lowercase().contains(&needle)
        || t.tags.iter().any(|x| x.to_lowercase().contains(&needle));
    let effort_ok = f.effort.is_empty() || t.effort.is_some_and(|e| f.effort.contains(&e));
    let impact_ok = f.impact.is_empty() || t.impact.is_some_and(|i| f.impact.contains(&i));
    let only_ok = f.only.is_empty()
        || f.only.iter().any(|o| match o {
            Only::NeedsAttention => t.ask.is_some() || t.suggested,
            Only::PullRequests => t.review.is_some(),
        });
    channel_ok && session_ok && tags_ok && search_ok && effort_ok && impact_ok && only_ok
}

pub fn apply(items: &[Todo], f: &Filter, today: Day) -> Vec<Todo> {
    items
        .iter()
        .filter(|t| matches_rest(t, f) && matches_due(t, f, today))
        .cloned()
        .collect()
}

/// Items every filter but the due filter lets through, which the due filter dropped.
pub fn left_out(items: &[Todo], f: &Filter, today: Day) -> Vec<Todo> {
    items
        .iter()
        .filter(|t| matches_rest(t, f) && !matches_due(t, f, today))
        .cloned()
        .collect()
}

/// Open items per channel for the chip row, largest first; None is "no channel".
pub fn channel_counts(items: &[Todo], today: Day) -> Vec<(Option<String>, usize)> {
    let mut counts: Vec<(Option<String>, usize)> = Vec::new();
    for t in items.iter().filter(|t| counts_as_open(t, today)) {
        match counts.iter_mut().find(|(c, _)| *c == t.channel) {
            Some((_, n)) => *n += 1,
            None => counts.push((t.channel.clone(), 1)),
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    counts
}

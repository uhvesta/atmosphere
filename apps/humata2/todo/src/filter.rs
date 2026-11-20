use crate::day::Day;
use crate::model::{Effort, FuzzyDue, Impact, Need, State, Todo};
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
    /// Any of these need levels.
    pub need: Vec<Need>,
    /// Any of these kinds of item.
    pub only: Vec<Only>,
    /// Include terminal `Won't do` and `Became` items.
    pub include_terminal: bool,
}

impl Filter {
    /// Returns true when no filter criterion is active.
    pub fn is_empty(&self) -> bool {
        self.channel.is_none()
            && self.session.is_none()
            && self.due.is_empty()
            && self.tags.is_empty()
            && self.search.trim().is_empty()
            && self.effort.is_empty()
            && self.impact.is_empty()
            && self.need.is_empty()
            && self.only.is_empty()
            && !self.include_terminal
    }

    /// Returns `(kind, label)` pairs for every active filter chip.
    pub fn active_chips(&self) -> Vec<(String, String)> {
        let mut chips = Vec::new();
        if let Some(ch) = &self.channel {
            let label = match ch {
                ChannelFilter::Channel(name) => name.clone(),
                ChannelFilter::NoChannel => "No channel".to_owned(),
            };
            chips.push(("channel".to_owned(), label));
        }
        if let Some(sess) = &self.session {
            let label = match sess {
                SessionFilter::Session(id) => format!("Session: {id}"),
                SessionFilter::NoneYet => "No session yet".to_owned(),
            };
            chips.push(("session".to_owned(), label));
        }
        for d in &self.due {
            let label = match d {
                DueFilter::Overdue => "Overdue",
                DueFilter::Next7Days => "Next 7 days",
                DueFilter::AfterThisWeek => "After this week",
                DueFilter::NoDate => "No date",
            };
            chips.push(("due".to_owned(), label.to_owned()));
        }
        for tag in &self.tags {
            chips.push(("tag".to_owned(), format!("#{tag}")));
        }
        for e in &self.effort {
            let label = match e {
                Effort::S => "Effort: S",
                Effort::M => "Effort: M",
                Effort::L => "Effort: L",
            };
            chips.push(("effort".to_owned(), label.to_owned()));
        }
        for i in &self.impact {
            let label = match i {
                Impact::Low => "Impact: Low",
                Impact::Medium => "Impact: Medium",
                Impact::High => "Impact: High",
            };
            chips.push(("impact".to_owned(), label.to_owned()));
        }
        for n in &self.need {
            chips.push(("need".to_owned(), format!("Need: {}", n.label())));
        }
        for o in &self.only {
            let label = match o {
                Only::NeedsAttention => "Needs attention",
                Only::PullRequests => "Pull requests",
            };
            chips.push(("only".to_owned(), label.to_owned()));
        }
        if self.include_terminal {
            chips.push(("terminal".to_owned(), "Won't do / Became".to_owned()));
        }
        let search = self.search.trim();
        if !search.is_empty() {
            chips.push(("search".to_owned(), format!("\"{search}\"")));
        }
        chips
    }
}

fn matches_due(t: &Todo, f: &Filter, today: Day) -> bool {
    if f.due.is_empty() {
        return true;
    }
    let firm = t.due.map(|due| today.days_until(due));
    let fuzzy = t.fuzzy_due.map(FuzzyDue::approx_days);
    f.due.iter().any(|k| match k {
        DueFilter::NoDate => firm.is_none() && fuzzy.is_none(),
        // Fuzzy due dates are never overdue.
        DueFilter::Overdue => firm.is_some_and(|d| d < 0),
        DueFilter::Next7Days => {
            firm.is_some_and(|d| (0..=7).contains(&d))
                || (firm.is_none() && fuzzy.is_some_and(|d| (0..=7).contains(&d)))
        }
        DueFilter::AfterThisWeek => {
            firm.is_some_and(|d| d > 7)
                || (firm.is_none() && fuzzy.is_some_and(|d| d > 7))
        }
    })
}

fn matches_search(t: &Todo, needle: &str) -> bool {
    needle.is_empty()
        || t.title.to_lowercase().contains(needle)
        || t.key.to_lowercase().contains(needle)
        || t.tags.iter().any(|x| x.to_lowercase().contains(needle))
        || t.external_block
            .as_ref()
            .is_some_and(|b| b.reason.to_lowercase().contains(needle))
}

fn matches_terminal(t: &Todo, f: &Filter) -> bool {
    f.include_terminal || !matches!(t.state, State::WontDo | State::Became)
}

fn matches_rest(t: &Todo, f: &Filter) -> bool {
    if !matches_terminal(t, f) {
        return false;
    }
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
    let search_ok = matches_search(t, &needle);
    let effort_ok = f.effort.is_empty() || t.effort.is_some_and(|e| f.effort.contains(&e));
    let impact_ok = f.impact.is_empty() || t.impact.is_some_and(|i| f.impact.contains(&i));
    let need_ok = f.need.is_empty() || t.need.is_some_and(|n| f.need.contains(&n));
    let only_ok = f.only.is_empty()
        || f.only.iter().any(|o| match o {
            Only::NeedsAttention => {
                t.ask.is_some()
                    || t.suggested
                    || t.stuck_agent.is_some()
                    || !t.blocker_gone.is_empty()
                    || !t.lifecycle_prompts.is_empty()
            }
            Only::PullRequests => t.review.is_some() || !t.watched_prs.is_empty(),
        });
    channel_ok
        && session_ok
        && tags_ok
        && search_ok
        && effort_ok
        && impact_ok
        && need_ok
        && only_ok
}

pub fn apply(items: &[Todo], f: &Filter, today: Day) -> Vec<Todo> {
    let needle = f.search.trim().to_lowercase();
    items
        .iter()
        .filter(|t| {
            if !matches_terminal(t, f) {
                return false;
            }
            // When searching, include snoozed (`hide_until > today`) items matching
            // the query so they can be found and rescheduled without toggling filters.
            if !needle.is_empty()
                && t.hide_until.is_some_and(|h| h > today)
                && matches_search(t, &needle)
            {
                return true;
            }
            matches_rest(t, f) && matches_due(t, f, today)
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn day(m: u32, d: u32) -> Day {
        Day::from_ymd(2026, m, d).unwrap()
    }

    fn today() -> Day {
        day(11, 19)
    }

    #[test]
    fn need_filter_matches_selected_need_levels() {
        let mut a = Todo::new("A-1", "Critical fix", day(11, 1));
        a.need = Some(Need::Critical);
        let mut b = Todo::new("A-2", "Nice polish", day(11, 1));
        b.need = Some(Need::NiceToHave);
        let c = Todo::new("A-3", "Unset need", day(11, 1));

        let f = Filter {
            need: vec![Need::Critical],
            ..Filter::default()
        };
        let got: Vec<String> = apply(&[a, b, c], &f, today())
            .into_iter()
            .map(|t| t.key)
            .collect();
        assert_eq!(got, vec!["A-1"]);
    }

    #[test]
    fn include_terminal_gates_wont_do_and_became() {
        let open = Todo::new("T-1", "Open task", day(11, 1));
        let mut wont = Todo::new("T-2", "Skipped task", day(11, 1));
        wont.state = State::WontDo;
        let mut became = Todo::new("T-3", "Split task", day(11, 1));
        became.state = State::Became;
        let items = vec![open, wont, became];

        let default_keys: Vec<String> = apply(&items, &Filter::default(), today())
            .into_iter()
            .map(|t| t.key)
            .collect();
        assert_eq!(default_keys, vec!["T-1"]);

        let f = Filter {
            include_terminal: true,
            ..Filter::default()
        };
        let all_keys: Vec<String> = apply(&items, &f, today())
            .into_iter()
            .map(|t| t.key)
            .collect();
        assert_eq!(all_keys, vec!["T-1", "T-2", "T-3"]);
    }

    #[test]
    fn search_includes_snoozed_items_matching_query_without_toggling_filters() {
        let mut snoozed = Todo::new("SNZ-1", "Migrate billing webhooks", day(11, 1));
        snoozed.hide_until = Some(day(12, 1));
        snoozed.channel = Some("infra".to_owned());

        let mut other_snoozed = Todo::new("SNZ-2", "Cleanup docs", day(11, 1));
        other_snoozed.hide_until = Some(day(12, 1));

        let f = Filter {
            channel: Some(ChannelFilter::Channel("payments".to_owned())),
            due: vec![DueFilter::Overdue],
            search: "webhooks".to_owned(),
            ..Filter::default()
        };
        let got: Vec<String> = apply(&[snoozed, other_snoozed], &f, today())
            .into_iter()
            .map(|t| t.key)
            .collect();
        assert_eq!(got, vec!["SNZ-1"]);
    }

    #[test]
    fn active_chips_lists_every_set_criterion() {
        assert!(Filter::default().is_empty());
        assert!(Filter::default().active_chips().is_empty());

        let f = Filter {
            channel: Some(ChannelFilter::Channel("payments".to_owned())),
            due: vec![DueFilter::Overdue],
            tags: vec!["bug".to_owned()],
            need: vec![Need::Critical],
            include_terminal: true,
            search: "retry".to_owned(),
            ..Filter::default()
        };
        assert!(!f.is_empty());
        let chips = f.active_chips();
        assert_eq!(
            chips,
            vec![
                ("channel".to_owned(), "payments".to_owned()),
                ("due".to_owned(), "Overdue".to_owned()),
                ("tag".to_owned(), "#bug".to_owned()),
                ("need".to_owned(), "Need: Critical".to_owned()),
                ("terminal".to_owned(), "Won't do / Became".to_owned()),
                ("search".to_owned(), "\"retry\"".to_owned()),
            ]
        );
    }

    #[test]
    fn fuzzy_due_never_matches_overdue_and_matches_horizon() {
        let mut soon = Todo::new("F-1", "Soon", day(11, 1));
        soon.fuzzy_due = Some(FuzzyDue::InAFewDays);
        let mut month = Todo::new("F-2", "Later", day(11, 1));
        month.fuzzy_due = Some(FuzzyDue::NextMonth);
        let items = vec![soon, month];

        let overdue = Filter {
            due: vec![DueFilter::Overdue],
            ..Filter::default()
        };
        assert!(apply(&items, &overdue, today()).is_empty());

        let next7 = Filter {
            due: vec![DueFilter::Next7Days],
            ..Filter::default()
        };
        let got: Vec<String> = apply(&items, &next7, today())
            .into_iter()
            .map(|t| t.key)
            .collect();
        assert_eq!(got, vec!["F-1"]);

        let after = Filter {
            due: vec![DueFilter::AfterThisWeek],
            ..Filter::default()
        };
        let got: Vec<String> = apply(&items, &after, today())
            .into_iter()
            .map(|t| t.key)
            .collect();
        assert_eq!(got, vec!["F-2"]);
    }
}

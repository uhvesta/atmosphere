use crate::day::{Day, Weekday};
use crate::model::{Effort, FuzzyDue, Impact, Need};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    pub title: String,
    pub due: Option<Day>,
    pub fuzzy_due: Option<FuzzyDue>,
    pub tags: Vec<String>,
    pub channel: Option<String>,
    pub effort: Option<Effort>,
    pub impact: Option<Impact>,
    pub need: Option<Need>,
}

fn weekday_word(w: &str) -> Option<Weekday> {
    Some(match w {
        "mon" | "monday" => Weekday::Mon,
        "tue" | "tuesday" => Weekday::Tue,
        "wed" | "wednesday" => Weekday::Wed,
        "thu" | "thursday" => Weekday::Thu,
        "fri" | "friday" => Weekday::Fri,
        "sat" | "saturday" => Weekday::Sat,
        "sun" | "sunday" => Weekday::Sun,
        _ => return None,
    })
}

fn day_word(w: &str, today: Day) -> Option<Day> {
    match w {
        "today" => Some(today),
        "tomorrow" => Some(today.add_days(1)),
        _ => weekday_word(w).map(|wd| today.next_weekday(wd)),
    }
}

fn need_word(w: &str) -> Option<Need> {
    match w {
        "!critical" => Some(Need::Critical),
        "!expected" => Some(Need::Expected),
        "!nice" | "!nice-to-have" => Some(Need::NiceToHave),
        "!idea" => Some(Need::Idea),
        _ => None,
    }
}

fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_dash = true;
    for c in s.chars() {
        if c.is_alphanumeric() {
            for lc in c.to_lowercase() {
                out.push(lc);
            }
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    if out.ends_with('-') {
        out.pop();
    }
    out
}

fn match_channel<'a>(lower: &str, channels: &'a [(String, String)]) -> Option<&'a str> {
    channels
        .iter()
        .find(|(id, name)| {
            name.to_lowercase() == lower
                || slugify(name) == lower
                || id.to_lowercase() == lower
        })
        .map(|(id, _)| id.as_str())
}

/// Reads trailing tokens (tag, channel, effort, impact, need, fuzzy/firm day) off a typed line.
/// Returns None when no title is left.
pub fn parse(line: &str, today: Day, channels: &[String]) -> Option<Draft> {
    let pairs: Vec<(String, String)> =
        channels.iter().map(|c| (c.clone(), c.clone())).collect();
    parse_with_channels(line, today, &pairs)
}

/// Reads trailing tokens off a typed line, matching channel tokens case-insensitively
/// against each `(id, name)` pair's display name, slugified display name, or ID, and
/// storing the resolved channel ID in `Draft.channel`.
/// Returns None when no title is left.
pub fn parse_with_channels(
    line: &str,
    today: Day,
    channels: &[(String, String)],
) -> Option<Draft> {
    let mut tokens: Vec<&str> = line.split_whitespace().collect();
    let mut draft = Draft {
        title: String::new(),
        due: None,
        fuzzy_due: None,
        tags: Vec::new(),
        channel: None,
        effort: None,
        impact: None,
        need: None,
    };
    while let Some(&last) = tokens.last() {
        let lower = last.to_lowercase();
        if let Some(tag) = last.strip_prefix('#').filter(|t| !t.is_empty()) {
            draft.tags.insert(0, tag.to_owned());
        } else if draft.need.is_none()
            && let Some(need) = need_word(&lower)
        {
            draft.need = Some(need);
        } else if draft.channel.is_none()
            && let Some(id) = match_channel(&lower, channels)
        {
            draft.channel = Some(id.to_owned());
        } else if draft.effort.is_none() && matches!(lower.as_str(), "s" | "m" | "l") {
            draft.effort = Some(match lower.as_str() {
                "s" => Effort::S,
                "m" => Effort::M,
                _ => Effort::L,
            });
        } else if draft.impact.is_none() && matches!(lower.as_str(), "low" | "medium" | "high") {
            draft.impact = Some(match lower.as_str() {
                "low" => Impact::Low,
                "medium" => Impact::Medium,
                _ => Impact::High,
            });
        } else if draft.due.is_none()
            && draft.fuzzy_due.is_none()
            && lower == "days"
            && tokens
                .iter()
                .rev()
                .nth(1)
                .is_some_and(|t| t.eq_ignore_ascii_case("few"))
        {
            draft.fuzzy_due = Some(FuzzyDue::InAFewDays);
            tokens.pop();
            // Optionally strip leading "in a" before "few days".
            if tokens
                .iter()
                .rev()
                .nth(1)
                .is_some_and(|t| t.eq_ignore_ascii_case("a"))
                && tokens
                    .iter()
                    .rev()
                    .nth(2)
                    .is_some_and(|t| t.eq_ignore_ascii_case("in"))
            {
                tokens.pop();
                tokens.pop();
            }
        } else if draft.due.is_none()
            && draft.fuzzy_due.is_none()
            && lower == "month"
            && tokens
                .iter()
                .rev()
                .nth(1)
                .is_some_and(|t| t.eq_ignore_ascii_case("next"))
        {
            draft.fuzzy_due = Some(FuzzyDue::NextMonth);
            tokens.pop();
        } else if draft.due.is_none()
            && draft.fuzzy_due.is_none()
            && lower == "week"
            && tokens
                .iter()
                .rev()
                .nth(1)
                .is_some_and(|t| t.eq_ignore_ascii_case("next"))
        {
            draft.due = Some(today.next_weekday(Weekday::Mon));
            draft.fuzzy_due = Some(FuzzyDue::NextWeek);
            tokens.pop();
        } else if draft.due.is_none()
            && draft.fuzzy_due.is_none()
            && let Some(d) = day_word(&lower, today)
        {
            draft.due = Some(d);
        } else {
            break;
        }
        tokens.pop();
    }
    let joined = tokens.join(" ");
    let mut chars = joined.chars();
    let title = match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect::<String>(),
        None => return None,
    };
    draft.title = title;
    Some(draft)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_need_and_fuzzy_due_tokens() {
        let today = Day::from_ymd(2026, 11, 19).unwrap();
        let d1 = parse("ship billing alerts !critical few days", today, &[]).unwrap();
        assert_eq!(d1.title, "Ship billing alerts");
        assert_eq!(d1.need, Some(Need::Critical));
        assert_eq!(d1.fuzzy_due, Some(FuzzyDue::InAFewDays));
        assert_eq!(d1.due, None);

        let d2 = parse("clean up old flags !nice next month", today, &[]).unwrap();
        assert_eq!(d2.title, "Clean up old flags");
        assert_eq!(d2.need, Some(Need::NiceToHave));
        assert_eq!(d2.fuzzy_due, Some(FuzzyDue::NextMonth));

        let d3 = parse("try new layout !idea in a few days", today, &[]).unwrap();
        assert_eq!(d3.title, "Try new layout");
        assert_eq!(d3.need, Some(Need::Idea));
        assert_eq!(d3.fuzzy_due, Some(FuzzyDue::InAFewDays));

        let d4 = parse("migrate db !expected next week", today, &[]).unwrap();
        assert_eq!(d4.title, "Migrate db");
        assert_eq!(d4.need, Some(Need::Expected));
        assert_eq!(d4.fuzzy_due, Some(FuzzyDue::NextWeek));
    }

    #[test]
    fn parse_with_channels_resolves_name_slug_and_id_to_channel_id() {
        let today = Day::from_ymd(2026, 11, 19).unwrap();
        let channels = vec![
            ("ch_pay".to_owned(), "Payments".to_owned()),
            ("ch_rew".to_owned(), "Payments Rewrite".to_owned()),
        ];

        let d1 = parse_with_channels("fix retries Payments fri", today, &channels).unwrap();
        assert_eq!(d1.title, "Fix retries");
        assert_eq!(d1.channel.as_deref(), Some("ch_pay"));

        let d2 = parse_with_channels("fix retries payments", today, &channels).unwrap();
        assert_eq!(d2.title, "Fix retries");
        assert_eq!(d2.channel.as_deref(), Some("ch_pay"));

        let d3 = parse_with_channels("migrate ledger payments-rewrite", today, &channels).unwrap();
        assert_eq!(d3.title, "Migrate ledger");
        assert_eq!(d3.channel.as_deref(), Some("ch_rew"));

        let d4 = parse_with_channels("audit ch_pay", today, &channels).unwrap();
        assert_eq!(d4.title, "Audit");
        assert_eq!(d4.channel.as_deref(), Some("ch_pay"));
    }
}

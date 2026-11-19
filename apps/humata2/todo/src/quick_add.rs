use crate::day::{Day, Weekday};
use crate::model::{Effort, Impact};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    pub title: String,
    pub due: Option<Day>,
    pub tags: Vec<String>,
    pub channel: Option<String>,
    pub effort: Option<Effort>,
    pub impact: Option<Impact>,
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

/// Reads trailing tokens (tag, channel, effort, impact, day) off a typed line.
/// Returns None when no title is left.
pub fn parse(line: &str, today: Day, channels: &[String]) -> Option<Draft> {
    let mut tokens: Vec<&str> = line.split_whitespace().collect();
    let mut draft = Draft {
        title: String::new(),
        due: None,
        tags: Vec::new(),
        channel: None,
        effort: None,
        impact: None,
    };
    while let Some(&last) = tokens.last() {
        let lower = last.to_lowercase();
        if let Some(tag) = last.strip_prefix('#').filter(|t| !t.is_empty()) {
            draft.tags.insert(0, tag.to_owned());
        } else if draft.channel.is_none()
            && let Some(c) = channels.iter().find(|c| c.to_lowercase() == lower)
        {
            draft.channel = Some(c.clone());
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
        } else if draft.due.is_none() && lower == "week" && tokens
                .iter()
                .rev()
                .nth(1)
                .is_some_and(|t| t.eq_ignore_ascii_case("next"))
        {
            draft.due = Some(today.next_weekday(Weekday::Mon));
            tokens.pop();
        } else if draft.due.is_none()
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

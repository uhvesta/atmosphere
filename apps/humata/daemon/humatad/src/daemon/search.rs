//! Searching every session's history, on every machine: the index is
//! caught up from each session that changed since it was last read, then
//! asked. A host that's offline is searched as last indexed.

use std::collections::{HashMap, HashSet};

use humata_wire::{Reply, pb};

use super::Daemon;
use crate::error::DaemonError;
use crate::store::{Entry, EntryKind, Found, Indexed, Query, Whereabouts};

/// The most of a tool's output indexed; the start says what it was.
const OUTPUT_INDEXED: usize = 4_000;
/// Hits when the request doesn't say.
const DEFAULT_LIMIT: usize = 50;

impl Daemon {
    /// The hits for `search`. `scope`, for an agent on a host, is the
    /// projects it may search; None for any.
    pub(super) async fn search(
        &self,
        search: pb::Search,
        scope: Option<Vec<String>>,
    ) -> Result<pb::SearchResults, DaemonError> {
        let (words, range) = time_range(&search.query);
        let matching = all_of(&words);
        if matching.is_empty() {
            return Err(DaemonError::NothingToSearch);
        }
        let projects = match (scope, search.project_id.is_empty()) {
            (None, true) => Vec::new(),
            (None, false) => vec![search.project_id.clone()],
            // An agent on a host with no projects there has nothing to search.
            (Some(scope), true) if scope.is_empty() => return Ok(pb::SearchResults::default()),
            (Some(scope), true) => scope,
            // The caller says when it's outside, naming who asked.
            (Some(scope), false) if !scope.contains(&search.project_id) => {
                return Ok(pb::SearchResults::default());
            }
            (Some(_), false) => vec![search.project_id.clone()],
        };
        let sessions = self.catch_up().await?;
        let (since, until) = match range {
            Some(range) => {
                let since = self.store.local_time(&range.since)?;
                let until = match &range.until {
                    Some(until) => Some(self.store.local_time(until)?),
                    None => None,
                };
                (Some(since), until)
            }
            None => (
                (search.since_unix_ms > 0).then_some(search.since_unix_ms),
                (search.until_unix_ms > 0).then_some(search.until_unix_ms),
            ),
        };
        let limit = match usize::try_from(search.limit) {
            Ok(0) | Err(_) => DEFAULT_LIMIT,
            Ok(limit) => limit,
        };
        let mut query = Query {
            matching,
            since_unix_ms: since,
            until_unix_ms: until,
            projects,
            item: Some(search.item_id.clone()).filter(|item| !item.is_empty()),
            session: Some(search.session_id.clone()).filter(|session| !session.is_empty()),
            // Hits in sessions since removed are left out after.
            limit: limit * 2,
        };
        let mut hits: Vec<pb::SearchHit> = Vec::new();
        let mut seen = HashSet::new();
        let mut take = |found: Vec<Found>, related: bool, hits: &mut Vec<pb::SearchHit>| {
            for found in found {
                if hits.len() >= limit {
                    return;
                }
                let name = if found.session.is_empty() {
                    String::new()
                } else {
                    match sessions.get(&found.session) {
                        Some(name) => name.clone(),
                        None => continue,
                    }
                };
                let key = (
                    found.session.clone(),
                    found.event.clone(),
                    found.thread,
                    found.item.clone(),
                    found.kind == EntryKind::Spec,
                );
                if seen.insert(key) {
                    hits.push(hit(found, name, related));
                }
            }
        };
        take(self.store.search(&query)?, false, &mut hits);
        let related = any_of(&search.related);
        if hits.len() < limit && !related.is_empty() {
            query.matching = related;
            take(self.store.search(&query)?, true, &mut hits);
        }
        Ok(pb::SearchResults {
            hits,
            words: words.join(" "),
            since_unix_ms: since.unwrap_or_default(),
            until_unix_ms: until.unwrap_or_default(),
        })
    }

    /// Indexes what every session did since it was last indexed, and
    /// returns each session's name by its id.
    async fn catch_up(&self) -> Result<HashMap<String, String>, DaemonError> {
        let _indexing = self.indexing.lock().await;
        let sessions = self.list("").await?.sessions;
        for session in &sessions {
            let whereabouts = Whereabouts {
                host: session.host.clone(),
                project: session.project_id.clone(),
                item: session.item_id.clone(),
            };
            let indexed = self.store.indexed(&session.id)?;
            let from = indexed.as_ref().map_or(0, |indexed| indexed.events);
            let settled = session.state() == pb::SessionState::Idle
                || session.state() == pb::SessionState::Failed;
            if indexed
                .as_ref()
                .is_some_and(|indexed| indexed.turns == session.turns && settled)
            {
                let unchanged = Indexed {
                    events: from,
                    turns: session.turns,
                };
                self.store
                    .index(&session.id, &whereabouts, from, &unchanged, &[])?;
                continue;
            }
            let read = pb::ReadHistory {
                session_id: session.id.clone(),
            };
            // A session whose history can't be read now is searched as
            // last indexed.
            let Ok(Reply::Response(pb::response::Kind::History(history))) =
                self.history(&session.host, read).await
            else {
                continue;
            };
            let events = history.events;
            if events.len() < from {
                continue;
            }
            let indexed = Indexed {
                events: events.len(),
                turns: session.turns,
            };
            self.store.index(
                &session.id,
                &whereabouts,
                from,
                &indexed,
                &entries(&events, from),
            )?;
        }
        self.store.index_comments_and_specs()?;
        Ok(sessions
            .into_iter()
            .map(|session| (session.id, session.name))
            .collect())
    }
}

fn hit(found: Found, session_name: String, related: bool) -> pb::SearchHit {
    let kind = match found.kind {
        EntryKind::Message => pb::SearchHitKind::Message,
        EntryKind::Reply => pb::SearchHitKind::Reply,
        EntryKind::ToolCall => pb::SearchHitKind::ToolCall,
        EntryKind::ToolOutput => pb::SearchHitKind::ToolOutput,
        EntryKind::Comment => pb::SearchHitKind::Comment,
        EntryKind::Spec => pb::SearchHitKind::Spec,
    };
    pb::SearchHit {
        kind: kind.into(),
        session_id: found.session,
        session_name,
        event_id: found.event,
        turn: found.turn,
        time_unix_ms: found.time_unix_ms,
        project_id: found.project,
        item_id: found.item,
        host: found.host,
        thread_id: found.thread,
        snippet: found.snippet,
        related,
    }
}

/// The entries for `events` from the `from`th on.
fn entries(events: &[pb::SessionEvent], from: usize) -> Vec<Entry> {
    use pb::session_event::Kind;
    let mut turn = 1;
    let mut entries = Vec::new();
    for (index, event) in events.iter().enumerate() {
        let found = match &event.kind {
            Some(Kind::UserMessage(message)) => Some((EntryKind::Message, message.text.clone())),
            Some(Kind::AssistantMessage(message)) => Some((EntryKind::Reply, message.text.clone())),
            Some(Kind::ToolStarted(started)) => Some((EntryKind::ToolCall, tool_call(started))),
            Some(Kind::ToolCompleted(completed)) => {
                Some((EntryKind::ToolOutput, start_of(&completed.output)))
            }
            _ => None,
        };
        if index >= from
            && let Some((kind, text)) = found
            && !text.trim().is_empty()
        {
            entries.push(Entry {
                kind,
                event: event.id.clone(),
                turn,
                time_unix_ms: event.time_unix_ms,
                text,
            });
        }
        if matches!(event.kind, Some(Kind::TurnEnded(_))) {
            turn += 1;
        }
    }
    entries
}

/// A tool call as it's searched: a shell call by its command, any other by
/// its name and arguments.
fn tool_call(started: &pb::ToolStarted) -> String {
    let arguments: serde_json::Value =
        serde_json::from_str(&started.arguments_json).unwrap_or(serde_json::Value::Null);
    match arguments["command"].as_str() {
        Some(command) => command.to_owned(),
        None => format!("{} {}", started.tool, started.arguments_json),
    }
}

fn start_of(output: &str) -> String {
    let mut end = output.len().min(OUTPUT_INDEXED);
    while !output.is_char_boundary(end) {
        end -= 1;
    }
    output[..end].to_owned()
}

/// A time named in a query, as SQLite date modifiers from now in this
/// Mac's time zone.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Range {
    since: Vec<String>,
    until: Option<Vec<String>>,
}

/// The query's words, without a time it names, and that time.
fn time_range(query: &str) -> (Vec<String>, Option<Range>) {
    let words: Vec<String> = query.split_whitespace().map(str::to_owned).collect();
    let lower: Vec<String> = words.iter().map(|word| word.to_lowercase()).collect();
    for start in 0..lower.len() {
        let rest: Vec<&str> = lower[start..].iter().map(String::as_str).collect();
        let Some((length, range)) = named_time(&rest) else {
            continue;
        };
        let mut words = words;
        words.drain(start..start + length);
        return (words, Some(range));
    }
    (words, None)
}

/// The time `words` start with, and how many words name it.
fn named_time(words: &[&str]) -> Option<(usize, Range)> {
    let modifiers = |names: &[&str]| names.iter().map(|name| (*name).to_owned()).collect();
    let week = ["start of day", "-6 days", "weekday 1"];
    let since = |length, names: &[&str]| {
        Some((
            length,
            Range {
                since: modifiers(names),
                until: None,
            },
        ))
    };
    let between = |length, from: &[&str], to: &[&str]| {
        Some((
            length,
            Range {
                since: modifiers(from),
                until: Some(modifiers(to)),
            },
        ))
    };
    match words {
        ["today", ..] => since(1, &["start of day"]),
        ["yesterday", ..] => between(1, &["start of day", "-1 day"], &["start of day"]),
        ["this", "week", ..] => since(2, &week),
        ["last", "week", ..] => between(2, &[&week[..], &["-7 days"]].concat(), &week),
        ["this", "month", ..] => since(2, &["start of month"]),
        ["last", "month", ..] => between(2, &["start of month", "-1 month"], &["start of month"]),
        ["this", "year", ..] => since(2, &["start of year"]),
        ["last", "year", ..] => between(2, &["start of year", "-1 year"], &["start of year"]),
        ["last" | "past", count, unit, ..] => {
            let count: u32 = count.parse().ok().filter(|count| *count > 0)?;
            let (count, unit) = match *unit {
                "hour" | "hours" => (count, "hours"),
                "day" | "days" => (count, "days"),
                "week" | "weeks" => (count.checked_mul(7)?, "days"),
                "month" | "months" => (count, "months"),
                "year" | "years" => (count, "years"),
                _ => return None,
            };
            Some((
                3,
                Range {
                    since: vec![format!("-{count} {unit}")],
                    until: None,
                },
            ))
        }
        _ => None,
    }
}

/// An FTS5 query finding entries with every one of `words`, the last as
/// the start of a word, since it may be still being typed.
fn all_of(words: &[String]) -> String {
    let quoted: Vec<String> = words.iter().filter_map(|word| quoted(word)).collect();
    match quoted.split_last() {
        Some((last, rest)) => {
            let mut query = rest.to_vec();
            query.push(format!("{last}*"));
            query.join(" ")
        }
        None => String::new(),
    }
}

/// An FTS5 query finding entries with any of `words`.
fn any_of(words: &[String]) -> String {
    words
        .iter()
        .filter_map(|word| quoted(word))
        .collect::<Vec<_>>()
        .join(" OR ")
}

/// `word` as an FTS5 string, so nothing in it is read as syntax; None if
/// it has nothing to find.
fn quoted(word: &str) -> Option<String> {
    word.chars()
        .any(char::is_alphanumeric)
        .then(|| format!("\"{}\"", word.replace('"', "\"\"")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_time_named_in_a_query_is_taken_out_of_its_words() {
        let (words, range) = time_range("cargo test Last Week failing");
        assert_eq!(words, ["cargo", "test", "failing"]);
        let range = range.expect("a range");
        assert!(range.until.is_some());

        let (words, range) = time_range("migration past 3 weeks");
        assert_eq!(words, ["migration"]);
        assert_eq!(
            range.map(|range| range.since),
            Some(vec!["-21 days".to_owned()])
        );

        let (words, range) = time_range("the last thing");
        assert_eq!(words, ["the", "last", "thing"]);
        assert_eq!(range, None);
    }

    #[test]
    fn query_words_are_never_read_as_syntax() {
        let words = ["cargo".to_owned(), "\"NEAR(".to_owned(), "--".to_owned()];
        assert_eq!(all_of(&words), "\"cargo\" \"\"\"NEAR(\"*");
        assert_eq!(any_of(&words), "\"cargo\" OR \"\"\"NEAR(\"");
        assert_eq!(all_of(&["--".to_owned()]), "");
    }
}

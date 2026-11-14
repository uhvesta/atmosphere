//! Searching every session's history, on every machine: the index is
//! caught up from each session that changed since it was last read, then
//! asked. A host that's offline is searched as last indexed.

use std::collections::{HashMap, HashSet};
use std::sync::PoisonError;
use std::time::{Duration, Instant};

use humata_wire::{Reply, pb};

use super::Daemon;
use crate::error::DaemonError;
use crate::store::{Entry, EntryKind, Found, Indexed, Query, Whereabouts};

/// The most of a tool's output indexed; the start says what it was.
const OUTPUT_INDEXED: usize = 4_000;
/// Hits when the request doesn't say.
const DEFAULT_LIMIT: usize = 50;
/// How long a search waits for the index to catch up before searching what
/// was indexed.
const CATCH_UP_WAIT: Duration = Duration::from_millis(1_500);
/// How long a session at work, or waiting for an approval, is left unread
/// once its history was read for the index, whatever it says meanwhile.
const READ_BUSY_EVERY: Duration = Duration::from_secs(30);

/// A session at work or waiting, as it was listed when its history was
/// last read for the index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LastRead {
    turns: u32,
    last_active_unix_ms: i64,
    at: Instant,
}

impl LastRead {
    fn of(session: &pb::SessionInfo, at: Instant) -> Self {
        Self {
            turns: session.turns,
            last_active_unix_ms: session.last_active_unix_ms,
            at,
        }
    }

    /// Whether `session`, still at work or waiting at `now`, needn't be
    /// read again yet: it's listed as it was when read, and every event
    /// moves when it was last active, so it has said nothing since; or it
    /// was read too lately to read again.
    fn still_holds(&self, session: &pb::SessionInfo, now: Instant) -> bool {
        (self.turns == session.turns && self.last_active_unix_ms == session.last_active_unix_ms)
            || now.saturating_duration_since(self.at) < READ_BUSY_EVERY
    }
}

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
        self.catch_up_soon().await;
        let sessions: HashMap<String, String> = self
            .list("")
            .await?
            .sessions
            .into_iter()
            .map(|session| (session.id, session.name))
            .collect();
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
        // Off the executor: the query scores every match before its limit.
        let found = {
            let query = query.clone();
            self.store.apart(move |store| store.search(&query)).await?
        };
        take(found, false, &mut hits);
        let related = any_of(&search.related);
        if hits.len() < limit && !related.is_empty() {
            query.matching = related;
            let found = self.store.apart(move |store| store.search(&query)).await?;
            take(found, true, &mut hits);
        }
        Ok(pb::SearchResults {
            hits,
            words: words.join(" "),
            since_unix_ms: since.unwrap_or_default(),
            until_unix_ms: until.unwrap_or_default(),
        })
    }

    /// Catches the index up in the background, waiting a little for it: a
    /// search then finds what was just said, yet a host slow to give a
    /// history can't hold it up, and searches what was indexed before.
    async fn catch_up_soon(&self) {
        let Some(daemon) = self.me.upgrade() else {
            return;
        };
        // One catching up already is waited on instead of starting another.
        let mut done = {
            let mut catching_up = self
                .catching_up
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            match catching_up.as_ref() {
                Some(running) if !*running.borrow() => running.clone(),
                _ => {
                    let (finished, done) = tokio::sync::watch::channel(false);
                    tokio::spawn(async move {
                        if let Err(error) = daemon.catch_up().await {
                            eprintln!(
                                "humatad: couldn't bring the search index up to date: {}",
                                humata_wire::describe(&error)
                            );
                        }
                        finished.send_replace(true);
                    });
                    *catching_up = Some(done.clone());
                    done
                }
            }
        };
        // Given up, it goes on, and the next search may find what it adds.
        let _ = tokio::time::timeout(CATCH_UP_WAIT, done.wait_for(|done| *done)).await;
    }

    /// Indexes what every session did since it was last indexed. A session
    /// whose turns and whereabouts are as indexed isn't read or written,
    /// and neither is one at work or waiting that has said nothing since
    /// it was read, or was read a moment ago.
    async fn catch_up(&self) -> Result<(), DaemonError> {
        let mut reads = self.indexing.lock().await;
        let sessions = self.list("").await?.sessions;
        {
            let listed: HashSet<&str> =
                sessions.iter().map(|session| session.id.as_str()).collect();
            reads.retain(|id, _| listed.contains(id.as_str()));
        }
        // What the index has of each, read in one go.
        let indexed = {
            let ids: Vec<String> = sessions.iter().map(|session| session.id.clone()).collect();
            self.store
                .apart(move |store| {
                    ids.iter()
                        .map(|id| store.indexed(id))
                        .collect::<Result<Vec<_>, _>>()
                })
                .await?
        };
        for (session, indexed) in sessions.iter().zip(indexed) {
            let whereabouts = Whereabouts {
                host: session.host.clone(),
                project: session.project_id.clone(),
                item: session.item_id.clone(),
            };
            let from = indexed.as_ref().map_or(0, |indexed| indexed.indexed.events);
            let settled = session.state() == pb::SessionState::Idle
                || session.state() == pb::SessionState::Failed;
            if settled {
                // Read as soon as it changes, to find what it just said.
                reads.remove(&session.id);
            }
            let read_lately = reads
                .get(&session.id)
                .is_some_and(|read| read.still_holds(session, Instant::now()));
            if let Some(indexed) = &indexed
                && ((settled && indexed.indexed.turns == session.turns) || read_lately)
            {
                if indexed.whereabouts != whereabouts {
                    let unchanged = indexed.indexed.clone();
                    let id = session.id.clone();
                    self.store
                        .apart(move |store| store.index(&id, &whereabouts, from, &unchanged, &[]))
                        .await?;
                }
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
            // Counted without the reply's streamed parts, which a later
            // read may not hold once the reply has come whole.
            let events: Vec<_> = history
                .events
                .into_iter()
                .filter(|event| {
                    !matches!(event.kind, Some(pb::session_event::Kind::AssistantDelta(_)))
                })
                .collect();
            let now = Indexed {
                events: events.len(),
                turns: session.turns,
                last_event: events
                    .last()
                    .map(|event| event.id.clone())
                    .unwrap_or_default(),
            };
            let had = indexed.as_ref().map(|indexed| &indexed.indexed);
            let id = session.id.clone();
            if follows_on(&events, had) {
                let entries = entries(&events, from);
                self.store
                    .apart(move |store| store.index(&id, &whereabouts, from, &now, &entries))
                    .await?;
            } else {
                // Cut back since, as by a rewind: what was indexed is no
                // longer its history, so its entries are made again.
                let entries = entries(&events, 0);
                self.store
                    .apart(move |store| store.index_afresh(&id, &whereabouts, &now, &entries))
                    .await?;
            }
            if !settled {
                reads.insert(session.id.clone(), LastRead::of(session, Instant::now()));
            }
        }
        let store = self.store.clone();
        store
            .apart(|store| store.index_comments_and_specs().map(drop))
            .await?;
        Ok(())
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

/// Whether `events` start with the events indexed as `had`, so only those
/// after need indexing: the last one indexed is still where it was. A
/// history cut back, as by a rewind, is shorter or has another event there.
fn follows_on(events: &[pb::SessionEvent], had: Option<&Indexed>) -> bool {
    let Some(had) = had else {
        return true;
    };
    match had.events.checked_sub(1) {
        None => true,
        Some(last) => events
            .get(last)
            .is_some_and(|event| event.id == had.last_event),
    }
}

/// The entries for `events` from the `from`th on.
fn entries(events: &[pb::SessionEvent], from: usize) -> Vec<Entry> {
    use pb::session_event::Kind;
    let mut turn = 1;
    let mut entries = Vec::new();
    for (index, event) in events.iter().enumerate() {
        // A hit opens the chat at it, where a subagent's steps aren't.
        if !event.parent_call_id.is_empty() {
            continue;
        }
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
    fn a_history_cut_back_doesnt_follow_on_from_what_was_indexed() {
        let events = |ids: &[&str]| -> Vec<pb::SessionEvent> {
            ids.iter()
                .map(|id| pb::SessionEvent {
                    id: (*id).to_owned(),
                    ..Default::default()
                })
                .collect()
        };
        let had = Indexed {
            events: 2,
            turns: 1,
            last_event: "b".to_owned(),
        };
        // Grown, or as it was.
        assert!(follows_on(&events(&["a", "b", "c"]), Some(&had)));
        assert!(follows_on(&events(&["a", "b"]), Some(&had)));
        // Never indexed.
        assert!(follows_on(&events(&["a"]), None));
        // Cut back, and cut back then grown past where it was.
        assert!(!follows_on(&events(&["a"]), Some(&had)));
        assert!(!follows_on(&events(&["a", "x", "y"]), Some(&had)));
    }

    #[test]
    fn a_busy_session_is_read_again_only_once_it_says_more_and_not_at_once() {
        let listed = |turns: u32, active: i64| pb::SessionInfo {
            turns,
            last_active_unix_ms: active,
            ..Default::default()
        };
        let read_at = Instant::now();
        let read = LastRead::of(&listed(2, 1_000), read_at);
        let much_later = read_at + READ_BUSY_EVERY * 10;
        // Waiting as it was, however long ago it was read.
        assert!(read.still_holds(&listed(2, 1_000), much_later));
        // It said more, or ended a turn: left a moment, then read.
        let just_after = read_at + READ_BUSY_EVERY / 2;
        for now in [listed(2, 2_000), listed(3, 1_000)] {
            assert!(read.still_holds(&now, just_after));
            assert!(!read.still_holds(&now, much_later));
        }
    }

    #[test]
    fn query_words_are_never_read_as_syntax() {
        let words = ["cargo".to_owned(), "\"NEAR(".to_owned(), "--".to_owned()];
        assert_eq!(all_of(&words), "\"cargo\" \"\"\"NEAR(\"*");
        assert_eq!(any_of(&words), "\"cargo\" OR \"\"\"NEAR(\"");
        assert_eq!(all_of(&["--".to_owned()]), "");
    }
}

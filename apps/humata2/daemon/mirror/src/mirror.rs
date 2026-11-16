//! The durable copy of each session's conversation and the index over it.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::index::{Index, Search};
use crate::session::{self, SavedMeta, State};
use crate::{MirrorError, Role, Scrubber, read_from};

/// The index's format. It is kept in `index.db` as `PRAGMA user_version`;
/// an index with another number is deleted and rebuilt from the mirror
/// files (LD-23, D62.5). Bump it with every change to what the index holds.
pub const FORMAT_VERSION: i64 = 1;

/// What a hit says about its session, besides the turn (LD-24).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionMeta {
    /// The session's id.
    pub session: String,
    /// Its channel's id; what [`Query::channels`] filters by.
    pub channel: String,
    pub channel_name: String,
    /// The session's title.
    pub title: String,
    /// Its machine's id; what [`Query::machine`] filters by.
    pub machine: String,
    pub machine_name: String,
    /// The agent that runs in it; what [`Query::agent`] filters by.
    pub agent: String,
    /// Archived sessions stay searchable (LD-23).
    pub archived: bool,
}

/// One mirrored entry of a session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MirrorEntry {
    /// Its place in the session, from 0, without gaps.
    pub seq: u64,
    /// The turn it belongs to: 1 from the first user prompt, one more with
    /// each prompt after it; 0 for what came before the first prompt.
    pub turn: u32,
    pub role: Role,
    /// The text, already scrubbed (LD-21).
    pub text: String,
    /// When the agent wrote it (RFC 3339), if its file said.
    pub at: Option<String>,
}

/// What to search for.
///
/// A word is a run of letters and digits; everything else only separates
/// words. Every word of `text` must be in one entry, in any case; the last
/// word also matches as the start of a longer one ("ledg" finds "ledger").
/// Nothing has a special meaning and nothing is an error: quotes, brackets,
/// `*`, `:`, `-` and a NUL separate words like a space does, and `AND`,
/// `OR`, `NOT` and `NEAR` are words to look for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    /// Channel ids to search in; empty searches all.
    pub channels: Vec<String>,
    /// Only sessions on the machine with this id.
    pub machine: Option<String>,
    /// Only sessions run by this agent.
    pub agent: Option<String>,
    /// Only entries written at or after this time (RFC 3339, UTC as the
    /// agent writes it). An entry without a time matches no time range.
    pub since: Option<String>,
    /// Only entries written before this time.
    pub until: Option<String>,
    pub page: PageAsk,
}

/// Which page of an answer to return.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageAsk {
    /// The most items to return; 0 means 50.
    pub size: u32,
    /// The `next_token` of the page before; empty for the first page.
    pub token: String,
}

/// One page of an answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page<T> {
    pub items: Vec<T>,
    /// What to ask the next page with; empty on the page that holds the
    /// last item, so a caller never fetches an empty page.
    pub next_token: String,
}

/// One search result: the best matching entry of one turn of one session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub session: String,
    pub channel: String,
    pub channel_name: String,
    pub title: String,
    pub machine: String,
    pub machine_name: String,
    pub agent: String,
    /// The turn to open.
    pub turn: u32,
    /// The matching entry's place in the session.
    pub seq: u64,
    /// Plain text around the first match, at most about 160 characters,
    /// with `…` at each end that was cut. A shorter entry is given whole.
    pub snippet: String,
    /// When the matching entry was written, if known.
    pub at: Option<String>,
    /// When this session's mirror last caught up (RFC 3339), for "as of
    /// its last sync". Saved with the mirror, so rebuilding the index
    /// doesn't change it.
    pub as_of: String,
}

/// The host's mirror of every session's conversation and its search index.
/// Clones share one mirror; it is safe to use from several threads.
#[derive(Clone)]
pub struct Mirror {
    inner: Arc<Inner>,
}

struct Inner {
    mirror_dir: PathBuf,
    index: Mutex<Index>,
}

/// Page size when the caller asks for 0.
const DEFAULT_PAGE: u32 = 50;
/// How much of an entry a snippet shows, in characters.
const SNIPPET_CHARS: usize = 160;
/// How much of the text before the first match a snippet keeps.
const SNIPPET_LEAD: usize = 40;

impl Mirror {
    /// Opens the mirror under `<root>/mirror/` and its index at
    /// `<root>/search/index.db`, creating what is missing. An index that is
    /// missing, damaged or of another [`FORMAT_VERSION`] is rebuilt from
    /// the mirror files. A cut-off append at the end of a session's
    /// `entries.log` is dropped (see the crate docs for the layout).
    pub fn open(root: &Path) -> Result<Mirror, MirrorError> {
        let mirror_dir = root.join("mirror");
        let search_dir = root.join("search");
        for dir in [&mirror_dir, &search_dir] {
            fs::create_dir_all(dir).map_err(|source| MirrorError::Io {
                path: dir.clone(),
                source,
            })?;
        }
        let mut index = Index::open(&search_dir.join("index.db"))?;
        let sessions = session::sessions(&mirror_dir)?;
        for session in &sessions {
            let dir = session::dir_of(&mirror_dir, session);
            let state = session::load_state(&dir)?;
            session::trim_log(&dir, state.committed_len)?;
            let meta = session::load_meta(&dir)?.unwrap_or_default();
            index.sync_session(session, &dir, &state, &meta)?;
        }
        for gone in index.sessions()? {
            if !sessions.contains(&gone) {
                index.remove(&gone)?;
            }
        }
        index.finish()?;
        Ok(Mirror {
            inner: Arc::new(Inner {
                mirror_dir,
                index: Mutex::new(index),
            }),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Index> {
        self.inner
            .index
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn dir(&self, session: &str) -> PathBuf {
        session::dir_of(&self.inner.mirror_dir, session)
    }

    /// Names the session's channel, machine, agent and title for its hits,
    /// before or after its entries arrive. Call it again when one of them
    /// changes: hits and filters use the latest.
    pub fn describe(&self, meta: &SessionMeta) -> Result<(), MirrorError> {
        let index = self.lock();
        let dir = self.dir(&meta.session);
        let saved = SavedMeta::from(meta);
        session::save_meta(&dir, &saved)?;
        let state = session::load_state(&dir)?;
        index.describe(&meta.session, &saved, &state.as_of)
    }

    /// Reads `file`, the agent's conversation file, from where this
    /// session's mirror stopped, scrubs the new entries, appends them and
    /// indexes them. Returns how many entries were added; calling it again
    /// with nothing new in the file adds 0. A cut-off append is never read
    /// back twice or in part.
    pub fn catch_up(
        &self,
        session: &str,
        file: &Path,
        scrubber: &Scrubber,
    ) -> Result<usize, MirrorError> {
        let mut index = self.lock();
        let dir = self.dir(session);
        let mut state = session::load_state(&dir)?;
        session::trim_log(&dir, state.committed_len)?;
        let meta = session::load_meta(&dir)?.unwrap_or_default();
        // The index may have missed an append that was saved before a crash.
        index.sync_session(session, &dir, &state, &meta)?;

        let mut own_tools = state.own_tools.clone();
        let read = read_from(file, state.offset, &mut own_tools)?;
        let mut turn = state.turn;
        let mut seq = state.next_seq;
        let entries: Vec<MirrorEntry> = read
            .entries
            .into_iter()
            .map(|entry| {
                if entry.new_turn {
                    turn += 1;
                }
                seq += 1;
                MirrorEntry {
                    seq: seq - 1,
                    turn,
                    role: entry.role,
                    text: scrubber.scrub(&entry.text),
                    at: entry.at,
                }
            })
            .collect();
        // Nothing new: the saved state already says all of it, so don't
        // pay for another fsync'd save just to move "as of".
        if entries.is_empty() && read.next_offset == state.offset && own_tools == state.own_tools {
            return Ok(0);
        }

        let written = session::append(&dir, state.committed_len, &entries)?;
        state = State {
            committed_len: state.committed_len + written,
            offset: read.next_offset,
            next_seq: seq,
            turn,
            own_tools,
            as_of: session::now(),
        };
        session::save_state(&dir, &state)?;
        index.add(session, &meta, &state.as_of, &entries)?;
        Ok(entries.len())
    }

    /// Searches every mirrored session, archived ones included (LD-23,
    /// LD-24). One hit per session and turn, the best match first and the
    /// newer first among equals. An empty `text`, or one of only white
    /// space, is [`MirrorError::EmptyQuery`]; a token this search didn't
    /// hand out is [`MirrorError::BadPageToken`].
    pub fn search(&self, query: &Query) -> Result<Page<Hit>, MirrorError> {
        if query.text.trim().is_empty() {
            return Err(MirrorError::EmptyQuery);
        }
        let words = words_of(&query.text);
        let fingerprint = fingerprint(&(
            &words,
            &query.channels,
            &query.machine,
            &query.agent,
            &query.since,
            &query.until,
        ));
        let offset = match query.page.token.as_str() {
            "" => 0,
            token => open_token(token, fingerprint)?,
        };
        let size = page_size(&query.page);
        let Some(expression) = match_expression(&words) else {
            return Ok(Page {
                items: Vec::new(),
                next_token: String::new(),
            });
        };
        let found = self.lock().search(&Search {
            expression: &expression,
            channels: &query.channels,
            machine: query.machine.as_deref(),
            agent: query.agent.as_deref(),
            since: query.since.as_deref(),
            until: query.until.as_deref(),
            limit: size.saturating_add(1),
            offset,
        })?;
        let more = found.len() > size as usize;
        let items = found
            .into_iter()
            .take(size as usize)
            .map(|(mut hit, text)| {
                hit.snippet = snippet(&text, &words);
                hit
            })
            .collect::<Vec<_>>();
        let next_token = if more {
            make_token(offset + u64::from(size), fingerprint)
        } else {
            String::new()
        };
        Ok(Page { items, next_token })
    }

    /// A session's entries in turns `first_turn..=last_turn` (`None`: from
    /// the start, to the end), in `seq` order, a page at a time (LD-25). A
    /// session the mirror doesn't know has none.
    pub fn read(
        &self,
        session: &str,
        first_turn: Option<u32>,
        last_turn: Option<u32>,
        page: &PageAsk,
    ) -> Result<Page<MirrorEntry>, MirrorError> {
        let fingerprint = fingerprint(&(session, first_turn, last_turn));
        let after = match page.token.as_str() {
            "" => None,
            token => Some(open_token(token, fingerprint)?),
        };
        let size = page_size(page);
        let mut items = self.lock().read(
            session,
            after,
            first_turn.unwrap_or(0),
            last_turn.unwrap_or(u32::MAX),
            size.saturating_add(1),
        )?;
        let more = items.len() > size as usize;
        items.truncate(size as usize);
        let next_token = match items.last() {
            Some(last) if more => make_token(last.seq, fingerprint),
            _ => String::new(),
        };
        Ok(Page { items, next_token })
    }

    /// The session is gone for good (LD-22): removes its mirror folder and
    /// its hits. Forgetting a session the mirror doesn't know is fine.
    pub fn forget(&self, session: &str) -> Result<(), MirrorError> {
        let mut index = self.lock();
        // The folder first: a session whose index rows outlive a failure
        // here is removed at the next open, while one whose folder
        // outlives its rows would come back.
        session::remove_dir(&self.dir(session))?;
        index.remove(session)
    }
}

fn page_size(page: &PageAsk) -> u32 {
    if page.size == 0 {
        DEFAULT_PAGE
    } else {
        page.size.min(u32::MAX - 1)
    }
}

/// The words of a text: runs of letters and digits, lower-cased.
fn words_of(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// An FTS5 expression for `words`: each one quoted, so none is syntax, and
/// the last one also a prefix. `None` when there is nothing to look for.
fn match_expression(words: &[String]) -> Option<String> {
    let (last, before) = words.split_last()?;
    let mut expression = String::new();
    for word in before {
        expression.push_str(&format!("\"{word}\" "));
    }
    expression.push_str(&format!("\"{last}\"*"));
    Some(expression)
}

fn fingerprint<T: Hash>(request: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    request.hash(&mut hasher);
    hasher.finish()
}

fn make_token(position: u64, fingerprint: u64) -> String {
    format!("{position:x}-{fingerprint:016x}")
}

/// The position in a token this mirror made for the same request.
fn open_token(token: &str, fingerprint: u64) -> Result<u64, MirrorError> {
    let (position, tag) = token.split_once('-').ok_or(MirrorError::BadPageToken)?;
    let position = u64::from_str_radix(position, 16).map_err(|_| MirrorError::BadPageToken)?;
    let tag = u64::from_str_radix(tag, 16).map_err(|_| MirrorError::BadPageToken)?;
    if tag == fingerprint {
        Ok(position)
    } else {
        Err(MirrorError::BadPageToken)
    }
}

/// Plain text around the first word of `text` that matches one of `words`.
fn snippet(text: &str, words: &[String]) -> String {
    let chars: Vec<char> = text
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect();
    if chars.len() <= SNIPPET_CHARS {
        return chars.into_iter().collect();
    }
    let mut first_match = 0;
    let mut start = 0;
    while start < chars.len() {
        if !chars[start].is_alphanumeric() {
            start += 1;
            continue;
        }
        let end = chars[start..]
            .iter()
            .position(|c| !c.is_alphanumeric())
            .map_or(chars.len(), |len| start + len);
        let word = chars[start..end].iter().collect::<String>().to_lowercase();
        let is_last = |candidate: &String| candidate == words.last().unwrap_or(candidate);
        if words
            .iter()
            .any(|candidate| *candidate == word || (is_last(candidate) && word.starts_with(candidate.as_str())))
        {
            first_match = start;
            break;
        }
        start = end;
    }
    let from = first_match.saturating_sub(SNIPPET_LEAD);
    let to = (from + SNIPPET_CHARS).min(chars.len());
    let mut out = String::new();
    if from > 0 {
        out.push('…');
    }
    out.extend(&chars[from..to]);
    if to < chars.len() {
        out.push('…');
    }
    out
}

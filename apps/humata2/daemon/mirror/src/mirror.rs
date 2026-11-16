//! The durable copy of each session's conversation and the index over it.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::index::{Index, Search};
use crate::raw::{self, Cursor, Raw};
use crate::session::{self, SavedMeta, State};
use crate::{Entry, MirrorError, OwnTools, Role, Scrubber, parse_lines, read_from};

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

/// The raw lines a session on another machine keeps (D195): the scrubbed
/// lines as they came from the machine's conversation file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawConversation {
    /// The agent that runs the session, from its meta. The lines are in
    /// that agent's format.
    pub agent: String,
    /// Complete scrubbed lines, each ended by `\n`. While a backfill runs,
    /// only the lines read so far.
    pub data: Vec<u8>,
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
            raw::trim(&dir, &state.raw)?;
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
        let state = session::load_state(&dir)?;
        session::trim_log(&dir, state.committed_len)?;
        let meta = session::load_meta(&dir)?.unwrap_or_default();
        // The index may have missed an append that was saved before a crash.
        index.sync_session(session, &dir, &state, &meta)?;

        let mut own_tools = state.own_tools.clone();
        let read = read_from(file, state.offset, &mut own_tools)?;
        let (entries, next_seq, turn) = number(&state, read.entries, scrubber);
        // Nothing new: the saved state already says all of it, so don't
        // pay for another fsync'd save just to move "as of".
        if entries.is_empty() && read.next_offset == state.offset && own_tools == state.own_tools {
            return Ok(0);
        }
        let next = Progress {
            offset: read.next_offset,
            source: state.source.clone(),
            next_seq,
            turn,
            own_tools,
            backfill: state.raw.backfill.clone(),
            raw: RawChange::Keep,
        };
        commit(&mut index, session, &dir, &meta, &state, next, &entries)?;
        Ok(entries.len())
    }

    /// Where reading session `session`'s conversation file on another
    /// machine goes on, in that file's bytes, and what the machine said the
    /// file was at the last read (empty before the first). While a backfill
    /// runs (see [`Mirror::begin_raw_backfill`]) that is the backfill's own
    /// place, which starts at the file's beginning.
    pub fn source_offset(&self, session: &str) -> Result<(u64, String), MirrorError> {
        let _index = self.lock();
        let state = session::load_state(&self.dir(session))?;
        Ok(match state.raw.backfill {
            Some(cursor) => (cursor.offset, cursor.source),
            None => (state.offset, state.source),
        })
    }

    /// Whether the session keeps raw lines yet. A session mirrored before
    /// the host kept them doesn't until [`Mirror::begin_raw_backfill`].
    pub fn keeps_raw_lines(&self, session: &str) -> Result<bool, MirrorError> {
        let _index = self.lock();
        Ok(session::load_state(&self.dir(session))?.raw.kept)
    }

    /// Starts keeping raw lines for a session whose entries were mirrored
    /// before that: from now on [`Mirror::source_offset`] answers `(0, "")`,
    /// and the machine's file is read again from its start for the raw lines
    /// alone. The raw lines start over, so a session that already keeps them
    /// gets them again in full. Until the read reaches the offset the entries
    /// already hold, takes add raw lines and no entries; the entries are then
    /// read as usual from that offset on. A session with no entries yet
    /// starts with none. Survives a reopen.
    pub fn begin_raw_backfill(&self, session: &str) -> Result<(), MirrorError> {
        let _index = self.lock();
        let dir = self.dir(session);
        let state = session::load_state(&dir)?;
        let raw = Raw {
            kept: true,
            generation: state.raw.generation + 1,
            committed_len: 0,
            backfill: (state.offset > 0).then(Cursor::default),
        };
        session::save_state(&dir, &State { raw, ..state })
    }

    /// While a backfill runs, the offset where the mirror's entries already
    /// stop. The backfill's reads must end there: a chunk that runs past it
    /// is refused by [`Mirror::take`]. `None` when no backfill runs.
    pub fn raw_backfill_stop(&self, session: &str) -> Result<Option<u64>, MirrorError> {
        let _index = self.lock();
        let state = session::load_state(&self.dir(session))?;
        Ok(state.raw.backfill.is_some().then_some(state.offset))
    }

    /// The raw lines a session on another machine keeps, as they came. `None`
    /// for a session that never had a take (this host's own sessions, which
    /// resume from their own files). During a backfill, the lines read so far.
    pub fn raw_conversation(&self, session: &str) -> Result<Option<RawConversation>, MirrorError> {
        let _index = self.lock();
        let dir = self.dir(session);
        let state = session::load_state(&dir)?;
        if !state.raw.kept {
            return Ok(None);
        }
        let data = raw::read(&dir, &state.raw)?;
        let agent = session::load_meta(&dir)?
            .map(|meta| meta.agent)
            .unwrap_or_default();
        Ok(Some(RawConversation { agent, data }))
    }

    /// Adds what a machine read for the host's mirror (D189): `chunk.data`
    /// is complete scrubbed lines as [`read_lines`](crate::read_lines) made
    /// them, read from byte `chunk.from` of the machine's file.
    ///
    /// A chunk that doesn't start where the mirror stopped is stale (a
    /// repeated trigger read the same bytes twice) and is dropped: `None`.
    /// A `rewritten` chunk starts a new file: its entries are appended after
    /// the old ones (nothing mirrored is lost, LD-9), `seq` and turns go on,
    /// and the source offset, the new source and the entries are saved in
    /// one write. Every taken chunk, even an empty one, moves "as of".
    ///
    /// The lines are also kept as they came, in the session's raw file (D195);
    /// a `rewritten` chunk starts that file over. During a backfill a chunk
    /// adds raw lines only, and one that runs past the offset the entries
    /// hold is [`MirrorError::BackfillPast`], which saves nothing: the host
    /// reads a backfill only up to [`Mirror::raw_backfill_stop`].
    /// Returns how many entries were added.
    pub fn take(&self, session: &str, chunk: &Chunk<'_>) -> Result<Option<usize>, MirrorError> {
        let mut index = self.lock();
        let dir = self.dir(session);
        let state = session::load_state(&dir)?;
        session::trim_log(&dir, state.committed_len)?;
        raw::trim(&dir, &state.raw)?;
        let meta = session::load_meta(&dir)?.unwrap_or_default();
        index.sync_session(session, &dir, &state, &meta)?;
        let (entries, next) = match &state.raw.backfill {
            Some(cursor) if !chunk.rewritten => {
                if chunk.from != cursor.offset {
                    return Ok(None);
                }
                if chunk.next_offset > state.offset {
                    return Err(MirrorError::BackfillPast {
                        at: state.offset,
                        read_to: chunk.next_offset,
                    });
                }
                // The entries already hold these lines: only the raw lines are new.
                let next = Progress {
                    offset: state.offset,
                    source: state.source.clone(),
                    next_seq: state.next_seq,
                    turn: state.turn,
                    own_tools: state.own_tools.clone(),
                    backfill: (chunk.next_offset < state.offset).then(|| Cursor {
                        offset: chunk.next_offset,
                        source: chunk.source.to_owned(),
                    }),
                    raw: RawChange::Append(chunk.data.to_vec()),
                };
                (Vec::new(), next)
            }
            _ => {
                if !chunk.rewritten && chunk.from != state.offset {
                    return Ok(None);
                }
                let mut own_tools = state.own_tools.clone();
                let read = parse_lines(chunk.data, &mut own_tools);
                // The lines were scrubbed on the machine.
                let (entries, next_seq, turn) = number(&state, read, &Scrubber::default());
                let raw = if chunk.rewritten {
                    RawChange::Restart(chunk.data.to_vec())
                } else {
                    RawChange::Append(chunk.data.to_vec())
                };
                let next = Progress {
                    offset: chunk.next_offset,
                    source: chunk.source.to_owned(),
                    next_seq,
                    turn,
                    own_tools,
                    backfill: None,
                    raw,
                };
                (entries, next)
            }
        };
        commit(&mut index, session, &dir, &meta, &state, next, &entries)?;
        Ok(Some(entries.len()))
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
        ))?;
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
        let fingerprint = fingerprint(&(session, first_turn, last_turn))?;
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

/// What the mirror's state becomes with an append.
struct Progress {
    offset: u64,
    source: String,
    next_seq: u64,
    turn: u32,
    own_tools: OwnTools,
    /// The backfill's place after this append; `None` once it is done.
    backfill: Option<Cursor>,
    raw: RawChange,
}

/// How an append changes the session's raw file.
enum RawChange {
    /// Nothing: the session's lines are not kept by this append (catch-up).
    Keep,
    /// The bytes go after the ones kept.
    Append(Vec<u8>),
    /// The bytes are the whole of a new file, which replaces the kept ones.
    Restart(Vec<u8>),
}

/// A part of another machine's conversation file, for [`Mirror::take`].
#[derive(Clone, Copy, Debug)]
pub struct Chunk<'a> {
    /// The byte of the machine's file `data` was read from; 0 when `rewritten`.
    pub from: u64,
    /// Complete scrubbed lines.
    pub data: &'a [u8],
    /// Where the next read starts.
    pub next_offset: u64,
    /// The file's identity as the machine answered.
    pub source: &'a str,
    /// The machine's file is a new one: `from` is not checked.
    pub rewritten: bool,
}

/// Numbers `read` after the state's last entry, scrubbed; also returns the
/// next `seq` and the turn of the last entry.
fn number(state: &State, read: Vec<Entry>, scrubber: &Scrubber) -> (Vec<MirrorEntry>, u64, u32) {
    let mut turn = state.turn;
    let mut seq = state.next_seq;
    let entries = read
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
    (entries, seq, turn)
}

/// Appends the raw bytes and `entries`, saves the state they lead to and
/// indexes the entries.
fn commit(
    index: &mut Index,
    session: &str,
    dir: &Path,
    meta: &SavedMeta,
    state: &State,
    next: Progress,
    entries: &[MirrorEntry],
) -> Result<(), MirrorError> {
    let mut raw = match next.raw {
        RawChange::Keep => state.raw.clone(),
        RawChange::Append(bytes) => Raw {
            kept: true,
            committed_len: state.raw.committed_len + raw::append(dir, &state.raw, &bytes)?,
            ..state.raw.clone()
        },
        RawChange::Restart(bytes) => {
            let generation = state.raw.generation + 1;
            Raw {
                kept: true,
                generation,
                committed_len: raw::start(dir, generation, &bytes)?,
                ..Raw::default()
            }
        }
    };
    raw.backfill = next.backfill;
    let written = session::append(dir, state.committed_len, entries)?;
    let state = State {
        committed_len: state.committed_len + written,
        offset: next.offset,
        next_seq: next.next_seq,
        turn: next.turn,
        own_tools: next.own_tools,
        as_of: session::now(),
        source: next.source,
        raw,
    };
    session::save_state(dir, &state)?;
    index.add(session, meta, &state.as_of, entries)
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

/// A request's tag in its page tokens: FNV-1a 64 over the request's JSON.
/// Both are fixed, so a token keeps opening across builds and toolchains
/// (a std hasher may change between Rust releases).
fn fingerprint<T: serde::Serialize>(request: &T) -> Result<u64, MirrorError> {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let bytes = serde_json::to_vec(request).map_err(|source| MirrorError::Request { source })?;
    Ok(bytes.iter().fold(OFFSET_BASIS, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(PRIME)
    }))
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

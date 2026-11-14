//! A session's history: events in memory, appended to a JSON-lines file and
//! broadcast to whoever watches.
//!
//! The parts of a reply as it streams are only for watchers, and for one
//! that starts watching part way through it. They're never written to the
//! file and are dropped from memory once the reply they make up is whole,
//! so a history read again doesn't carry every reply twice.
//!
//! A session at rest keeps only its summary in memory, written beside the
//! history, and reads its events again when something needs them. An old
//! history can be sealed: what's in the plain file then is compressed onto
//! the end of `events.sealed.zst`, one length-prefixed zstd frame each
//! time, and the plain file starts again empty, so the live tail stays
//! plain and appends never touch the sealed part.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::error::AgentError;
use crate::event::{Event, EventKind};
use crate::summary::Summary;

/// How many events a watcher can fall behind before it lags.
const LIVE_CAPACITY: usize = 4096;

pub(crate) struct EventLog {
    path: PathBuf,
    inner: Mutex<Inner>,
    /// Kept up to date with the events, loaded or not. Apart from `inner`,
    /// which is held while a history at rest is read from disk, so reading
    /// the summary never waits for that. Locked only for a moment, and
    /// when both are held, after `inner`.
    summary: Mutex<Summary>,
    /// Whether `inner` has the events loaded, kept beside it for the same
    /// reason.
    is_loaded: AtomicBool,
    live: broadcast::Sender<Event>,
}

struct Inner {
    /// None while the log is at rest.
    loaded: Option<Loaded>,
}

struct Loaded {
    events: Vec<Event>,
    /// Where the parts of the reply streaming now start in `events`.
    streaming: Option<usize>,
    file: File,
}

/// What a log at rest keeps beside its history, so opening it reads none
/// of it. Kept only while the files are as long as it says.
#[derive(Serialize, Deserialize)]
struct Resting {
    summary: Summary,
    plain: u64,
    sealed: u64,
}

/// What [`EventLog::rest`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rested {
    /// It's at rest, sealed if it was asked to be.
    Resting { sealed: bool },
    /// Someone watches it, a reply streams, or its turn isn't closed.
    Busy,
}

impl EventLog {
    /// Creates the file with `events` in it, replacing any there.
    pub(crate) fn create(path: PathBuf, events: Vec<Event>) -> Result<Self, AgentError> {
        let mut file = File::create(&path).map_err(|source| AgentError::Write {
            path: path.clone(),
            source,
        })?;
        let events: Vec<Event> = events
            .into_iter()
            .filter(|event| !streamed(event))
            .collect();
        let mut text = String::new();
        for event in &events {
            text.push_str(&encode(event)?);
        }
        file.write_all(text.as_bytes())
            .and_then(|()| file.sync_data())
            .map_err(|source| AgentError::Write {
                path: path.clone(),
                source,
            })?;
        remove(&sealed_path(&path))?;
        remove(&resting_path(&path))?;
        Ok(Self::with(path, events, file))
    }

    /// Opens the file and reads the events in it. A last line cut short,
    /// as a crash mid-write leaves it, is dropped from the file; a damaged
    /// line before it is an error, as dropping it would lose the rest.
    pub(crate) fn open(path: PathBuf) -> Result<Self, AgentError> {
        let loaded = read_all(&path, &mut |_, _| {})?;
        let (live, _) = broadcast::channel(LIVE_CAPACITY);
        Ok(Self {
            path,
            summary: Mutex::new(Summary::of(&loaded.events)),
            inner: Mutex::new(Inner {
                loaded: Some(loaded),
            }),
            is_loaded: AtomicBool::new(true),
            live,
        })
    }

    /// Opens the log at rest if it was left at rest and its files haven't
    /// changed since, reading only its summary; otherwise as [`Self::open`].
    pub(crate) fn open_resting(path: PathBuf) -> Result<Self, AgentError> {
        if let Some(resting) = still_resting(&path) {
            let (live, _) = broadcast::channel(LIVE_CAPACITY);
            return Ok(Self {
                path,
                inner: Mutex::new(Inner { loaded: None }),
                summary: Mutex::new(resting.summary),
                is_loaded: AtomicBool::new(false),
                live,
            });
        }
        Self::open(path)
    }

    fn with(path: PathBuf, events: Vec<Event>, file: File) -> Self {
        let (live, _) = broadcast::channel(LIVE_CAPACITY);
        Self {
            path,
            summary: Mutex::new(Summary::of(&events)),
            inner: Mutex::new(Inner {
                loaded: Some(Loaded {
                    events,
                    streaming: None,
                    file,
                }),
            }),
            is_loaded: AtomicBool::new(true),
            live,
        }
    }

    /// Whether the events are in memory, as they are unless the log is at
    /// rest. Doesn't wait for a history being read.
    pub(crate) fn is_loaded(&self) -> bool {
        self.is_loaded.load(Ordering::Acquire)
    }

    fn loaded<'a>(
        &self,
        inner: &'a mut Inner,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<&'a mut Loaded, AgentError> {
        if inner.loaded.is_none() {
            inner.loaded = Some(read_all(&self.path, progress)?);
            self.is_loaded.store(true, Ordering::Release);
        }
        inner.loaded.as_mut().ok_or_else(|| AgentError::Read {
            path: self.path.clone(),
            source: std::io::Error::other("the history wasn't loaded"),
        })
    }

    /// Writes the event, then tells watchers. Part of a reply as it
    /// streams is kept only until the reply is whole, or its turn ends.
    pub(crate) fn append(&self, event: Event) -> Result<Event, AgentError> {
        let mut inner = self.lock();
        let loaded = self.loaded(&mut inner, &mut |_, _| {})?;
        if streamed(&event) {
            if loaded.streaming.is_none() {
                loaded.streaming = Some(loaded.events.len());
            }
        } else {
            let line = encode(&event)?;
            loaded
                .file
                .write_all(line.as_bytes())
                .map_err(|source| AgentError::Write {
                    path: self.path.clone(),
                    source,
                })?;
            // A subagent's reply never streams, so it leaves the main
            // agent's parts alone.
            if matches!(
                event.kind,
                EventKind::AssistantMessage { .. } | EventKind::TurnEnded { .. }
            ) && event.is_main()
                && let Some(from) = loaded.streaming.take()
            {
                let tail = loaded.events.split_off(from);
                loaded
                    .events
                    .extend(tail.into_iter().filter(|kept| !streamed(kept)));
            }
        }
        loaded.events.push(event.clone());
        self.lock_summary().add(&event);
        // No watchers is not an error.
        let _ = self.live.send(event.clone());
        Ok(event)
    }

    /// Puts `events` in place of the whole history, all or nothing, then
    /// tells watchers the last of them, which says why: a watcher starts
    /// over from the new history when it sees it.
    pub(crate) fn replace(&self, events: Vec<Event>) -> Result<(), AgentError> {
        let events: Vec<Event> = events
            .into_iter()
            .filter(|event| !streamed(event))
            .collect();
        let mut text = String::new();
        for event in &events {
            text.push_str(&encode(event)?);
        }
        let mut inner = self.lock();
        let partial = self.path.with_extension("jsonl.partial");
        let written = File::create(&partial)
            .and_then(|mut file| {
                file.write_all(text.as_bytes())
                    .and_then(|()| file.sync_data())
            })
            .and_then(|()| std::fs::rename(&partial, &self.path));
        written.map_err(|source| AgentError::Write {
            path: self.path.clone(),
            source,
        })?;
        // The plain file is whole now. Were this cut short here, opening
        // it again tells so by its first event, which the sealed part
        // starts with too.
        remove(&sealed_path(&self.path))?;
        remove(&resting_path(&self.path))?;
        let file = append_to(&self.path)?;
        let summary = Summary::of(&events);
        let last = events.last().cloned();
        inner.loaded = Some(Loaded {
            events,
            streaming: None,
            file,
        });
        self.is_loaded.store(true, Ordering::Release);
        *self.lock_summary() = summary;
        if let Some(last) = last {
            let _ = self.live.send(last);
        }
        Ok(())
    }

    /// A copy of the events so far. A log at rest stays at rest: see
    /// [`Self::peek`].
    pub(crate) fn events(&self) -> Result<Vec<Event>, AgentError> {
        self.peek(<[Event]>::to_vec)
    }

    /// `read` given the events so far, as [`Self::with_events`], but a log
    /// at rest reads them from disk for this alone and stays at rest, so
    /// looking at an old history doesn't keep it in memory.
    pub(crate) fn peek<T>(&self, read: impl FnOnce(&[Event]) -> T) -> Result<T, AgentError> {
        let inner = self.lock();
        match &inner.loaded {
            Some(loaded) => Ok(read(&loaded.events)),
            None => Ok(read(&read_all(&self.path, &mut |_, _| {})?.events)),
        }
    }

    /// Reads the events into memory if the log is at rest, so what needs
    /// them next doesn't read the disk. It reads the disk itself, so it's
    /// for a thread that may block.
    pub(crate) fn load(&self) -> Result<(), AgentError> {
        let mut inner = self.lock();
        self.loaded(&mut inner, &mut |_, _| {}).map(|_| ())
    }

    /// `read` given the events so far, without copying them; the log stays
    /// locked meanwhile, so `read` mustn't wait.
    pub(crate) fn with_events<T>(&self, read: impl FnOnce(&[Event]) -> T) -> Result<T, AgentError> {
        let mut inner = self.lock();
        Ok(read(&self.loaded(&mut inner, &mut |_, _| {})?.events))
    }

    /// The summary of the events so far. Doesn't wait for a history being
    /// read.
    pub(crate) fn summary(&self) -> Summary {
        self.lock_summary().clone()
    }

    /// The events so far and a receiver for the rest, with none missed or
    /// repeated between them. A log at rest is read first, telling
    /// `progress` the bytes read of the total.
    pub(crate) fn watch(
        &self,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(Vec<Event>, broadcast::Receiver<Event>), AgentError> {
        let mut inner = self.lock();
        let events = self.loaded(&mut inner, progress)?.events.clone();
        Ok((events, self.live.subscribe()))
    }

    /// Puts the log at rest, its events dropped from memory and its summary
    /// kept beside it, if no one watches, no reply streams and `closed`
    /// says its turn is closed. With `seal`, what's in the plain file is
    /// sealed first.
    pub(crate) fn rest(
        &self,
        seal: bool,
        closed: impl FnOnce(&[Event]) -> bool,
    ) -> Result<Rested, AgentError> {
        let mut inner = self.lock();
        if self.live.receiver_count() > 0 {
            return Ok(Rested::Busy);
        }
        if let Some(loaded) = &inner.loaded
            && (loaded.streaming.is_some() || !closed(&loaded.events))
        {
            return Ok(Rested::Busy);
        }
        let sealed = seal && self.seal(&mut inner)?;
        let resting = Resting {
            summary: self.summary(),
            plain: length(&self.path)?,
            sealed: length(&sealed_path(&self.path))?,
        };
        let json = serde_json::to_vec(&resting).map_err(|source| AgentError::Encode { source })?;
        let kept = resting_path(&self.path);
        let partial = kept.with_extension("json.partial");
        std::fs::write(&partial, json)
            .and_then(|()| std::fs::rename(&partial, &kept))
            .map_err(|source| AgentError::Write {
                path: kept.clone(),
                source,
            })?;
        inner.loaded = None;
        self.is_loaded.store(false, Ordering::Release);
        Ok(Rested::Resting { sealed })
    }

    /// Compresses what's in the plain file onto the end of the sealed one
    /// and empties the plain one, unless it's empty or ends cut short.
    /// With the lock held, so nothing appends meanwhile.
    fn seal(&self, inner: &mut Inner) -> Result<bool, AgentError> {
        let plain = std::fs::read(&self.path).map_err(|source| AgentError::Read {
            path: self.path.clone(),
            source,
        })?;
        if plain.iter().all(u8::is_ascii_whitespace) || plain.last() != Some(&b'\n') {
            return Ok(false);
        }
        let frame = ruzstd::encoding::compress_to_vec(
            plain.as_slice(),
            ruzstd::encoding::CompressionLevel::Fastest,
        );
        let sealed = sealed_path(&self.path);
        let mut bytes = match std::fs::read(&sealed) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(source) => {
                return Err(AgentError::Read {
                    path: sealed,
                    source,
                });
            }
        };
        bytes.extend_from_slice(&(frame.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&frame);
        let partial = sealed.with_extension("zst.partial");
        let written = File::create(&partial)
            .and_then(|mut file| file.write_all(&bytes).and_then(|()| file.sync_data()))
            .and_then(|()| std::fs::rename(&partial, &sealed));
        written.map_err(|source| AgentError::Write {
            path: sealed.clone(),
            source,
        })?;
        // Were this cut short here, opening it again tells so: the plain
        // file starts as the last sealed frame does.
        let emptied = self.path.with_extension("jsonl.partial");
        File::create(&emptied)
            .and_then(|file| file.sync_data())
            .and_then(|()| std::fs::rename(&emptied, &self.path))
            .map_err(|source| AgentError::Write {
                path: self.path.clone(),
                source,
            })?;
        // The loaded log, if it is, appends to the new, empty file.
        if let Some(loaded) = inner.loaded.as_mut() {
            loaded.file = append_to(&self.path)?;
        }
        Ok(true)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // A panic while holding the lock leaves the events as they were: each
        // change is one push after a successful write.
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// For a moment only: never held while taking `inner` or reading the
    /// disk.
    fn lock_summary(&self) -> std::sync::MutexGuard<'_, Summary> {
        self.summary
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Where the sealed part of the history at `plain` is.
fn sealed_path(plain: &Path) -> PathBuf {
    plain.with_extension("sealed.zst")
}

/// Where a log at rest keeps its summary.
fn resting_path(plain: &Path) -> PathBuf {
    plain.with_extension("resting.json")
}

/// Removes the file at `path`, if there is one.
fn remove(path: &Path) -> Result<(), AgentError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(AgentError::Write {
            path: path.to_owned(),
            source,
        }),
    }
}

/// How long the file at `path` is; 0 if it isn't there.
fn length(path: &Path) -> Result<u64, AgentError> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(source) => Err(AgentError::Read {
            path: path.to_owned(),
            source,
        }),
    }
}

fn append_to(path: &Path) -> Result<File, AgentError> {
    OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(|source| AgentError::Write {
            path: path.to_owned(),
            source,
        })
}

/// What the log at `plain` kept when it was put at rest, if its files are
/// still as they were then.
fn still_resting(plain: &Path) -> Option<Resting> {
    let json = std::fs::read(resting_path(plain)).ok()?;
    let resting: Resting = serde_json::from_slice(&json).ok()?;
    let unchanged =
        length(plain).ok()? == resting.plain && length(&sealed_path(plain)).ok()? == resting.sealed;
    unchanged.then_some(resting)
}

/// Reads the whole history at `plain`, its sealed part first, telling
/// `progress` the bytes read of the total, and repairs what a crash left.
fn read_all(plain: &Path, progress: &mut dyn FnMut(u64, u64)) -> Result<Loaded, AgentError> {
    let sealed_at = sealed_path(plain);
    let total = length(plain)? + length(&sealed_at)?;
    progress(0, total);
    let (mut events, last_frame) = match std::fs::read(&sealed_at) {
        Ok(bytes) => unseal(&sealed_at, &bytes, &mut |read| progress(read, total))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (Vec::new(), None),
        Err(source) => {
            return Err(AgentError::Read {
                path: sealed_at,
                source,
            });
        }
    };
    let bytes = std::fs::read(plain).map_err(|source| AgentError::Read {
        path: plain.to_owned(),
        source,
    })?;
    let (mut recent, kept) = parse(plain, &bytes)?;
    let mut file = append_to(plain)?;
    let repair = if kept < bytes.len() {
        file.set_len(kept as u64)
    } else if bytes.last().is_some_and(|byte| *byte != b'\n') {
        file.write_all(b"\n")
    } else {
        Ok(())
    };
    repair.map_err(|source| AgentError::Write {
        path: plain.to_owned(),
        source,
    })?;
    if let Some(first) = recent.first() {
        if events.first().is_some_and(|sealed| sealed.id == first.id) {
            // A replace was cut short after it wrote the plain file whole.
            events.clear();
            remove(&sealed_at)?;
        } else if last_frame
            .and_then(|at| events.get(at))
            .is_some_and(|sealed| sealed.id == first.id)
        {
            // A seal was cut short before it emptied the plain file.
            recent.clear();
            file.set_len(0).map_err(|source| AgentError::Write {
                path: plain.to_owned(),
                source,
            })?;
        }
    }
    events.append(&mut recent);
    progress(total, total);
    Ok(Loaded {
        events,
        streaming: None,
        file,
    })
}

/// The events in the sealed file at `path`, and where its last frame's
/// start in them, telling `progress` the bytes read so far.
fn unseal(
    path: &Path,
    bytes: &[u8],
    progress: &mut dyn FnMut(u64),
) -> Result<(Vec<Event>, Option<usize>), AgentError> {
    let damaged = |source: std::io::Error| AgentError::CorruptSealedHistory {
        path: path.to_owned(),
        source,
    };
    let cut = || damaged(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
    let mut events = Vec::new();
    let mut last = None;
    let mut at = 0;
    while at < bytes.len() {
        let header: [u8; 8] = bytes
            .get(at..at + 8)
            .and_then(|header| header.try_into().ok())
            .ok_or_else(cut)?;
        let start = at + 8;
        let end = usize::try_from(u64::from_le_bytes(header))
            .ok()
            .and_then(|length| start.checked_add(length))
            .ok_or_else(cut)?;
        let frame = bytes.get(start..end).ok_or_else(cut)?;
        let mut decoder = ruzstd::decoding::StreamingDecoder::new(frame)
            .map_err(|error| damaged(std::io::Error::other(error.to_string())))?;
        let mut text = Vec::new();
        decoder.read_to_end(&mut text).map_err(damaged)?;
        let (kept, parsed) = parse(path, &text)?;
        if parsed < text.len() {
            return Err(cut());
        }
        last = Some(events.len());
        events.extend(kept);
        at = end;
        progress(at as u64);
    }
    Ok((events, last))
}

/// The events in the JSON lines `bytes`, and how many of the bytes they
/// take: a last line cut short isn't counted. A damaged line before it is
/// an error.
fn parse(path: &Path, bytes: &[u8]) -> Result<(Vec<Event>, usize), AgentError> {
    let mut events = Vec::new();
    let mut kept = 0;
    let mut start = 0;
    let mut number = 0;
    while start < bytes.len() {
        number += 1;
        let (end, complete) = match bytes[start..].iter().position(|byte| *byte == b'\n') {
            Some(at) => (start + at, true),
            None => (bytes.len(), false),
        };
        let line = &bytes[start..end];
        start = end + 1;
        if line.iter().all(u8::is_ascii_whitespace) {
            kept = start.min(bytes.len());
            continue;
        }
        // Written before parts of replies were left out; they're
        // dropped as they're read.
        match serde_json::from_slice::<Event>(line) {
            Ok(event) => {
                if !streamed(&event) {
                    events.push(event);
                }
                // Whole but for its newline: the write was cut after it.
                kept = start.min(bytes.len());
            }
            Err(_) if !complete => break,
            Err(source) => {
                return Err(AgentError::CorruptHistory {
                    path: path.to_owned(),
                    line: number,
                    source,
                });
            }
        }
    }
    Ok((events, kept))
}

/// Whether `event` is part of a reply as it streams.
fn streamed(event: &Event) -> bool {
    matches!(event.kind, EventKind::AssistantDelta { .. })
}

fn encode(event: &Event) -> Result<String, AgentError> {
    let mut line = serde_json::to_string(event).map_err(|source| AgentError::Encode { source })?;
    line.push('\n');
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn said(text: &str) -> Event {
        Event::now(EventKind::UserMessage {
            text: text.into(),
            images: Vec::new(),
        })
    }

    fn texts(log: &EventLog) -> Vec<String> {
        log.events()
            .unwrap_or_else(|error| panic!("{error}"))
            .into_iter()
            .filter_map(|event| match event.kind {
                EventKind::UserMessage { text, .. } => Some(text),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn events_stored_before_subagents_were_recorded_load_as_the_main_agents() {
        let folder = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let path = folder.path().join("events.jsonl");
        std::fs::write(
            &path,
            "{\"id\":\"a\",\"time_unix_ms\":1,\"kind\":{\"type\":\"assistant_message\",\"text\":\"hi\"}}\n",
        )
        .unwrap_or_else(|error| panic!("{error}"));

        let log = EventLog::open(path).unwrap_or_else(|error| panic!("{error}"));
        let events = log.events().unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(events.len(), 1);
        assert!(events[0].is_main());
        assert_eq!(
            events[0].kind,
            EventKind::AssistantMessage { text: "hi".into() }
        );
    }

    #[test]
    fn a_subagents_steps_are_kept_apart_from_the_main_agents() {
        let folder = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let path = folder.path().join("events.jsonl");
        let log =
            EventLog::create(path.clone(), Vec::new()).unwrap_or_else(|error| panic!("{error}"));
        let mut step = Event::now(EventKind::AssistantMessage {
            text: "found it".into(),
        });
        step.parent_call_id = Some("call-1".into());
        log.append(step).unwrap_or_else(|error| panic!("{error}"));
        drop(log);

        let log = EventLog::open(path).unwrap_or_else(|error| panic!("{error}"));
        let events = log.events().unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].parent_call_id.as_deref(), Some("call-1"));
    }

    #[test]
    fn a_line_cut_short_is_dropped_and_the_log_carries_on() {
        let folder = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let path = folder.path().join("events.jsonl");
        drop(EventLog::create(path.clone(), vec![said("one")]));
        let mut file = OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap_or_else(|e| panic!("{e}"));
        file.write_all(b"{\"id\":\"cut")
            .unwrap_or_else(|e| panic!("{e}"));
        drop(file);

        let log = EventLog::open(path.clone()).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(texts(&log), ["one"]);
        log.append(said("two"))
            .unwrap_or_else(|error| panic!("{error}"));
        drop(log);
        let log = EventLog::open(path).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(texts(&log), ["one", "two"]);
    }

    #[test]
    fn a_damaged_line_before_the_last_is_an_error() {
        let folder = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let path = folder.path().join("events.jsonl");
        drop(EventLog::create(path.clone(), vec![said("one")]));
        let good = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(&path, format!("not json\n{good}")).unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(
            EventLog::open(path),
            Err(AgentError::CorruptHistory { line: 1, .. })
        ));
    }

    #[test]
    fn the_parts_of_a_reply_are_kept_only_until_it_is_whole() {
        let folder = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let path = folder.path().join("events.jsonl");
        let log = EventLog::create(path.clone(), vec![said("hi")])
            .unwrap_or_else(|error| panic!("{error}"));
        let (_, mut live) = log
            .watch(&mut |_, _| {})
            .unwrap_or_else(|error| panic!("{error}"));
        let append = |kind| {
            log.append(Event::now(kind))
                .unwrap_or_else(|error| panic!("{error}"))
        };
        append(EventKind::AssistantDelta { text: "hel".into() });
        append(EventKind::AssistantDelta { text: "lo".into() });
        // One who starts watching now sees the reply so far.
        assert_eq!(log.events().map(|events| events.len()).ok(), Some(3));
        append(EventKind::AssistantMessage {
            text: "hello".into(),
        });
        let kinds = |events: Vec<Event>| -> Vec<EventKind> {
            events.into_iter().map(|event| event.kind).collect()
        };
        let whole = vec![
            EventKind::UserMessage {
                text: "hi".into(),
                images: Vec::new(),
            },
            EventKind::AssistantMessage {
                text: "hello".into(),
            },
        ];
        assert_eq!(
            kinds(log.events().unwrap_or_else(|error| panic!("{error}"))),
            whole
        );
        // Those watching got every part.
        let mut streamed = 0;
        while let Ok(event) = live.try_recv() {
            streamed += usize::from(matches!(event.kind, EventKind::AssistantDelta { .. }));
        }
        assert_eq!(streamed, 2);
        drop(log);
        let reopened = EventLog::open(path).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            kinds(reopened.events().unwrap_or_else(|error| panic!("{error}"))),
            whole
        );
    }

    fn ok<T>(result: Result<T, AgentError>) -> T {
        result.unwrap_or_else(|error| panic!("{error}"))
    }

    fn sealed_log(path: &Path) -> EventLog {
        let log = ok(EventLog::create(
            path.to_owned(),
            vec![said("one"), said("two")],
        ));
        assert_eq!(
            ok(log.rest(true, |_| true)),
            Rested::Resting { sealed: true }
        );
        log
    }

    #[test]
    fn a_sealed_history_reads_as_it_was_and_goes_on_in_the_plain_file() {
        let folder = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let path = folder.path().join("events.jsonl");
        let log = sealed_log(&path);
        assert_eq!(texts(&log), ["one", "two"]);
        ok(log.append(said("three")));
        assert_eq!(texts(&log), ["one", "two", "three"]);
        drop(log);
        let log = ok(EventLog::open(path.clone()));
        assert_eq!(texts(&log), ["one", "two", "three"]);
        // Sealed again, the new frame follows the first.
        assert_eq!(
            ok(log.rest(true, |_| true)),
            Rested::Resting { sealed: true }
        );
        ok(log.append(said("four")));
        drop(log);
        let log = ok(EventLog::open(path));
        assert_eq!(texts(&log), ["one", "two", "three", "four"]);
    }

    #[test]
    fn a_log_left_at_rest_opens_with_its_summary_and_reads_its_events_when_asked() {
        let folder = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let path = folder.path().join("events.jsonl");
        let log = ok(EventLog::create(path.clone(), vec![said("one")]));
        let summary = log.summary();
        assert_eq!(
            ok(log.rest(false, |_| true)),
            Rested::Resting { sealed: false }
        );
        drop(log);
        let log = ok(EventLog::open_resting(path.clone()));
        assert!(!log.is_loaded());
        assert_eq!(log.summary(), summary);
        let mut told = Vec::new();
        drop(ok(log.watch(&mut |read, total| told.push((read, total)))));
        assert!(log.is_loaded());
        assert_eq!(told.last().map(|(read, total)| read == total), Some(true));
        assert_eq!(texts(&log), ["one"]);
    }

    #[test]
    fn the_summary_follows_what_is_appended_replaced_and_put_at_rest() {
        let folder = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let path = folder.path().join("events.jsonl");
        let log = ok(EventLog::create(path, vec![said("one")]));
        let of_events = |log: &EventLog| Summary::of(&ok(log.events()));
        assert_eq!(log.summary(), of_events(&log));

        ok(log.append(said("two")));
        assert_eq!(log.summary(), of_events(&log));

        ok(log.replace(vec![said("three")]));
        assert_eq!(log.summary(), of_events(&log));
        assert!(log.is_loaded());

        assert_eq!(
            ok(log.rest(false, |_| true)),
            Rested::Resting { sealed: false }
        );
        assert!(!log.is_loaded());
        assert_eq!(log.summary(), of_events(&log));

        // Appending to a log at rest loads it, and is counted.
        ok(log.append(said("four")));
        assert!(log.is_loaded());
        assert_eq!(log.summary(), of_events(&log));
        assert_eq!(texts(&log), ["three", "four"]);
    }

    #[test]
    fn a_log_at_rest_stays_at_rest_when_its_events_are_only_read() {
        let folder = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let path = folder.path().join("events.jsonl");
        let log = sealed_log(&path);
        assert_eq!(texts(&log), ["one", "two"]);
        assert_eq!(ok(log.peek(<[Event]>::len)), 2);
        assert!(!log.is_loaded());
        // Loaded when asked to be, and what's appended then is read too.
        ok(log.load());
        assert!(log.is_loaded());
        ok(log.append(said("three")));
        assert_eq!(texts(&log), ["one", "two", "three"]);
    }

    #[test]
    fn a_log_changed_after_it_was_put_at_rest_is_read_whole() {
        let folder = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let path = folder.path().join("events.jsonl");
        let log = ok(EventLog::create(path.clone(), vec![said("one")]));
        ok(log.rest(false, |_| true));
        ok(log.append(said("two")));
        drop(log);
        let log = ok(EventLog::open_resting(path));
        assert!(log.is_loaded());
        assert_eq!(texts(&log), ["one", "two"]);
    }

    #[test]
    fn a_log_watched_or_mid_turn_isnt_put_at_rest() {
        let folder = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let path = folder.path().join("events.jsonl");
        let log = ok(EventLog::create(path, vec![said("one")]));
        assert_eq!(ok(log.rest(true, |_| false)), Rested::Busy);
        let watching = ok(log.watch(&mut |_, _| {}));
        assert_eq!(ok(log.rest(true, |_| true)), Rested::Busy);
        drop(watching);
        assert_eq!(
            ok(log.rest(true, |_| true)),
            Rested::Resting { sealed: true }
        );
    }

    #[test]
    fn a_seal_cut_short_before_the_plain_file_was_emptied_repeats_nothing() {
        let folder = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let path = folder.path().join("events.jsonl");
        let log = sealed_log(&path);
        ok(log.append(said("three")));
        ok(log.rest(true, |_| true));
        drop(log);
        // As if the second seal stopped after writing its frame.
        let frame_text = {
            let log = ok(EventLog::open(path.clone()));
            let events = ok(log.events());
            ok(encode(&events[2]))
        };
        std::fs::write(&path, &frame_text).unwrap_or_else(|e| panic!("{e}"));
        let log = ok(EventLog::open(path));
        assert_eq!(texts(&log), ["one", "two", "three"]);
    }

    #[test]
    fn a_replace_cut_short_before_the_sealed_part_was_removed_keeps_the_new_history() {
        let folder = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let path = folder.path().join("events.jsonl");
        let log = sealed_log(&path);
        let first = ok(log.events()).remove(0);
        drop(log);
        // As if a rewind to after the first message wrote the plain file
        // and stopped.
        std::fs::write(&path, ok(encode(&first))).unwrap_or_else(|e| panic!("{e}"));
        let log = ok(EventLog::open(path));
        assert_eq!(texts(&log), ["one"]);
    }

    #[test]
    fn a_damaged_sealed_part_is_an_error() {
        let folder = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let path = folder.path().join("events.jsonl");
        drop(sealed_log(&path));
        let sealed = sealed_path(&path);
        let bytes = std::fs::read(&sealed).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(&sealed, &bytes[..bytes.len() - 3]).unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(
            EventLog::open(path),
            Err(AgentError::CorruptSealedHistory { .. })
        ));
    }
}

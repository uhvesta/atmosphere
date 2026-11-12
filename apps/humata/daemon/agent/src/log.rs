//! A session's history: events in memory, appended to a JSON-lines file and
//! broadcast to whoever watches.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use tokio::sync::broadcast;

use crate::error::AgentError;
use crate::event::Event;

/// How many events a watcher can fall behind before it lags.
const LIVE_CAPACITY: usize = 4096;

pub(crate) struct EventLog {
    path: PathBuf,
    inner: Mutex<Inner>,
    live: broadcast::Sender<Event>,
}

struct Inner {
    events: Vec<Event>,
    file: File,
}

impl EventLog {
    /// Creates the file with `events` in it, replacing any there.
    pub(crate) fn create(path: PathBuf, events: Vec<Event>) -> Result<Self, AgentError> {
        let mut file = File::create(&path).map_err(|source| AgentError::Write {
            path: path.clone(),
            source,
        })?;
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
        Ok(Self::with(path, events, file))
    }

    /// Opens the file and reads the events in it. A last line cut short,
    /// as a crash mid-write leaves it, is dropped from the file; a damaged
    /// line before it is an error, as dropping it would lose the rest.
    pub(crate) fn open(path: PathBuf) -> Result<Self, AgentError> {
        let bytes = std::fs::read(&path).map_err(|source| AgentError::Read {
            path: path.clone(),
            source,
        })?;
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
            match serde_json::from_slice(line) {
                Ok(event) if complete => {
                    events.push(event);
                    kept = start;
                }
                // Whole but for its newline: the write was cut after it.
                Ok(event) => {
                    events.push(event);
                    kept = bytes.len();
                }
                Err(_) if !complete => break,
                Err(source) => {
                    return Err(AgentError::CorruptHistory {
                        path: path.clone(),
                        line: number,
                        source,
                    });
                }
            }
        }
        let mut file = OpenOptions::new()
            .append(true)
            .open(&path)
            .map_err(|source| AgentError::Write {
                path: path.clone(),
                source,
            })?;
        let repair = if kept < bytes.len() {
            file.set_len(kept as u64)
        } else if bytes.last().is_some_and(|byte| *byte != b'\n') {
            file.write_all(b"\n")
        } else {
            Ok(())
        };
        repair.map_err(|source| AgentError::Write {
            path: path.clone(),
            source,
        })?;
        Ok(Self::with(path, events, file))
    }

    fn with(path: PathBuf, events: Vec<Event>, file: File) -> Self {
        let (live, _) = broadcast::channel(LIVE_CAPACITY);
        Self {
            path,
            inner: Mutex::new(Inner { events, file }),
            live,
        }
    }

    /// Writes the event, then tells watchers.
    pub(crate) fn append(&self, event: Event) -> Result<Event, AgentError> {
        let line = encode(&event)?;
        let mut inner = self.lock();
        inner
            .file
            .write_all(line.as_bytes())
            .map_err(|source| AgentError::Write {
                path: self.path.clone(),
                source,
            })?;
        inner.events.push(event.clone());
        // No watchers is not an error.
        let _ = self.live.send(event.clone());
        Ok(event)
    }

    pub(crate) fn events(&self) -> Vec<Event> {
        self.lock().events.clone()
    }

    /// The events so far and a receiver for the rest, with none missed or
    /// repeated between them.
    pub(crate) fn watch(&self) -> (Vec<Event>, broadcast::Receiver<Event>) {
        let inner = self.lock();
        (inner.events.clone(), self.live.subscribe())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // A panic while holding the lock leaves the events as they were: each
        // change is one push after a successful write.
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn encode(event: &Event) -> Result<String, AgentError> {
    let mut line = serde_json::to_string(event).map_err(|source| AgentError::Encode { source })?;
    line.push('\n');
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::EventKind;

    fn said(text: &str) -> Event {
        Event::now(EventKind::UserMessage { text: text.into() })
    }

    fn texts(log: &EventLog) -> Vec<String> {
        log.events()
            .into_iter()
            .filter_map(|event| match event.kind {
                EventKind::UserMessage { text } => Some(text),
                _ => None,
            })
            .collect()
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
}

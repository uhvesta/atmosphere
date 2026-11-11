//! A session's history: events in memory, appended to a JSON-lines file and
//! broadcast to whoever watches.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
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

    /// Opens the file and reads the events in it.
    pub(crate) fn open(path: PathBuf) -> Result<Self, AgentError> {
        let reader = File::open(&path).map_err(|source| AgentError::Read {
            path: path.clone(),
            source,
        })?;
        let mut events = Vec::new();
        for (index, line) in BufReader::new(reader).lines().enumerate() {
            let line = line.map_err(|source| AgentError::Read {
                path: path.clone(),
                source,
            })?;
            if line.trim().is_empty() {
                continue;
            }
            let event =
                serde_json::from_str(&line).map_err(|source| AgentError::CorruptHistory {
                    path: path.clone(),
                    line: index + 1,
                    source,
                })?;
            events.push(event);
        }
        let file = OpenOptions::new()
            .append(true)
            .open(&path)
            .map_err(|source| AgentError::Write {
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

//! The operations a start found cut short, and how each ended (PER-29,
//! D241). Each start-time recovery records its operation as `pending` when
//! it reads the journal, and again with the outcome when it settles. The
//! lines go to `operations.jsonl` in this start's restart report folder,
//! appended and synced one at a time, so a crash mid-recovery loses at most
//! the line being written. The folder is only known once the reconcile has
//! written the report, so earlier lines wait in memory until then.

use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};

/// The file in the report folder.
const FILE: &str = "operations.jsonl";

/// How far an operation a start found has got.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum State {
    /// Found in a journal and not settled yet, and what it waits for.
    Pending { waits_for: String },
    /// Carried on to its end.
    Finished,
    /// Rolled back, and why that was the way out.
    Undone { reason: String },
    /// Neither finished nor undone, and why; the next start tries again.
    Stuck { reason: String },
}

/// One operation as the report names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) id: String,
    /// What it was: `move`, `fork`, ...
    pub(crate) kind: String,
    /// The last step it reached before the restart.
    pub(crate) step: String,
    pub(crate) state: State,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum OperationsError {
    #[error(
        "couldn't open {path} for the restart's interrupted operations: {source}; the report lacks them, so check the data folder's disk and permissions"
    )]
    Open {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "couldn't write to {path} for the restart's interrupted operations: {source}; the report lacks them, so check that the data folder's disk isn't full"
    )]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("couldn't read {path}: {source}; check its permissions")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "line {line} of {path} isn't an operation record ({detail}); the file was edited or damaged, so restore it from a backup or ignore it"
    )]
    Malformed {
        path: PathBuf,
        line: usize,
        detail: String,
    },
}

#[derive(Default)]
struct Inner {
    /// Lines written before the folder was known.
    waiting: Vec<String>,
    folder: Option<PathBuf>,
    file: Option<File>,
}

/// Records the operations a start found, shared by every recovery of it.
#[derive(Clone, Default)]
pub(crate) struct Operations {
    inner: Arc<Mutex<Inner>>,
}

impl Operations {
    /// Notes `id`'s `step` and `state`: logged now, and written to the
    /// report once its folder is known.
    pub(crate) fn record(&self, id: &str, kind: &str, step: &str, state: State) {
        let line = line_of(&Entry {
            id: id.to_owned(),
            kind: kind.to_owned(),
            step: step.to_owned(),
            state,
        });
        crate::log::say!("humatad2: restart operation {line}");
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        inner.waiting.push(line);
        if inner.folder.is_some() {
            Self::flush(&mut inner);
        }
    }

    /// Says where this start's report is; what waited is written now.
    pub(crate) fn set_folder(&self, folder: &Path) {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        inner.folder = Some(folder.to_owned());
        Self::flush(&mut inner);
    }

    fn flush(inner: &mut Inner) {
        let Some(folder) = inner.folder.clone() else {
            return;
        };
        let lines = std::mem::take(&mut inner.waiting);
        if let Err(error) = append(inner, &folder.join(FILE), &lines) {
            crate::log::say!("humatad2: {error}");
        }
    }
}

fn append(inner: &mut Inner, path: &Path, lines: &[String]) -> Result<(), OperationsError> {
    if lines.is_empty() {
        return Ok(());
    }
    if inner.file.is_none() {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)
            .map_err(|source| OperationsError::Open {
                path: path.to_owned(),
                source,
            })?;
        inner.file = Some(file);
    }
    let Some(file) = inner.file.as_mut() else {
        return Ok(());
    };
    for line in lines {
        let write = file
            .write_all(format!("{line}\n").as_bytes())
            .and_then(|()| file.sync_data());
        write.map_err(|source| OperationsError::Write {
            path: path.to_owned(),
            source,
        })?;
    }
    Ok(())
}

fn line_of(entry: &Entry) -> String {
    let mut value = json!({
        "id": entry.id,
        "kind": entry.kind,
        "step": entry.step,
    });
    let (state, detail) = match &entry.state {
        State::Pending { waits_for } => ("pending", Some(("waits_for", waits_for))),
        State::Finished => ("finished", None),
        State::Undone { reason } => ("undone", Some(("reason", reason))),
        State::Stuck { reason } => ("stuck", Some(("reason", reason))),
    };
    value["state"] = json!(state);
    if let Some((key, text)) = detail {
        value[key] = json!(text);
    }
    value.to_string()
}

fn entry_of(value: &Value) -> Option<Entry> {
    let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
    let state = match value.get("state")?.as_str()? {
        "pending" => State::Pending {
            waits_for: text("waits_for")?,
        },
        "finished" => State::Finished,
        "undone" => State::Undone { reason: text("reason")? },
        "stuck" => State::Stuck { reason: text("reason")? },
        _ => return None,
    };
    Some(Entry {
        id: text("id")?,
        kind: text("kind")?,
        step: text("step")?,
        state,
    })
}

/// The operations an `operations.jsonl` names, each as its last line says,
/// in the order they first appear. A last line a crash cut short is
/// ignored. Read by the restart doctor (PER-30) and the tests.
#[allow(dead_code)]
pub(crate) fn read(path: &Path) -> Result<Vec<Entry>, OperationsError> {
    let text = std::fs::read_to_string(path).map_err(|source| OperationsError::Read {
        path: path.to_owned(),
        source,
    })?;
    let lines: Vec<&str> = text.lines().collect();
    let mut entries: Vec<Entry> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let parsed = serde_json::from_str::<Value>(line)
            .map_err(|error| error.to_string())
            .and_then(|value| entry_of(&value).ok_or_else(|| "a field is missing".to_owned()));
        let entry = match parsed {
            Ok(entry) => entry,
            Err(_) if index + 1 == lines.len() => continue,
            Err(detail) => {
                return Err(OperationsError::Malformed {
                    path: path.to_owned(),
                    line: index + 1,
                    detail,
                });
            }
        };
        match entries.iter_mut().find(|known| known.id == entry.id) {
            Some(known) => *known = entry,
            None => entries.push(entry),
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, state: State) -> Entry {
        Entry {
            id: id.to_owned(),
            kind: "move".to_owned(),
            step: "copied".to_owned(),
            state,
        }
    }

    #[test]
    fn the_latest_line_of_each_operation_wins_in_first_seen_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        let lines = [
            entry("a", State::Pending { waits_for: "link".into() }),
            entry("b", State::Pending { waits_for: "link".into() }),
            entry("a", State::Undone { reason: "agent still ran there".into() }),
            entry("b", State::Finished),
            entry("b", State::Stuck { reason: "disk".into() }),
        ];
        let text: String = lines.iter().map(|entry| line_of(entry) + "\n").collect();
        std::fs::write(&path, text).unwrap();

        let read = read(&path).unwrap();
        assert_eq!(
            read,
            vec![
                entry("a", State::Undone { reason: "agent still ran there".into() }),
                entry("b", State::Stuck { reason: "disk".into() }),
            ]
        );
    }

    #[test]
    fn a_torn_last_line_is_ignored_and_a_torn_middle_one_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        let whole = line_of(&entry("a", State::Finished));
        std::fs::write(&path, format!("{whole}\n{{\"id\":\"b\",\"ki")).unwrap();
        assert_eq!(read(&path).unwrap(), vec![entry("a", State::Finished)]);

        std::fs::write(&path, format!("{{\"id\":\"b\",\"ki\n{whole}\n")).unwrap();
        assert!(matches!(read(&path), Err(OperationsError::Malformed { line: 1, .. })));
    }

    #[test]
    fn lines_recorded_before_the_folder_is_known_are_written_when_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let operations = Operations::default();
        operations.record("a", "move", "copied", State::Pending { waits_for: "link".into() });
        assert!(!dir.path().join(FILE).exists());
        operations.set_folder(dir.path());
        operations.record("a", "move", "copied", State::Finished);

        let read = read(&dir.path().join(FILE)).unwrap();
        assert_eq!(read, vec![entry("a", State::Finished)]);
    }
}

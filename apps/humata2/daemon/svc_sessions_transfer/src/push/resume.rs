//! A push on another machine that the host was following when it stopped
//! (the restart's follow-up of F3). While the host follows the machine's
//! operation it keeps one JSON file per push in the state directory's `push`
//! folder: the host's operation as it started, whose it is, the session, the
//! machine, and the machine's own operation once it answered.

use std::path::{Path, PathBuf};

use humata2_store::StoreError;
use humata2_svc_sessions_common::pb;
use prost::Message;
use serde_json::{Value, json};

use crate::SessionTransferSvc;
use crate::ports::blocking;

/// Origin metadata for a tracked operation (owner + optional `(channel, session)` scope).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin {
    pub owner: String,
    pub scope: Option<(String, String)>,
}

/// Why a push couldn't be kept on disk.
#[derive(Debug, thiserror::Error)]
pub enum KeptError {
    #[error("couldn't make the folder {path} for the pushes that wait on another machine")]
    Folder {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("couldn't write {path}, a push that waits on another machine")]
    Write {
        path: PathBuf,
        #[source]
        source: StoreError,
    },
}

/// What a restart needs to go on following a push on another machine.
#[derive(Clone, Debug)]
pub struct Kept {
    /// The host's operation as it started.
    pub operation: pb::Operation,
    pub origin: Origin,
    pub session: String,
    pub machine: String,
    /// The machine's own operation, once it answered.
    pub asked: Option<String>,
}

impl Kept {
    pub fn new(operation: pb::Operation, origin: Origin, session: &str, machine: &str) -> Self {
        Self {
            operation,
            origin,
            session: session.to_owned(),
            machine: machine.to_owned(),
            asked: None,
        }
    }

    pub fn id(&self) -> &str {
        self.operation.id.as_ref().map_or("", |id| id.value.as_str())
    }

    fn value(&self) -> Value {
        json!({
            "operation": self.operation.encode_to_vec(),
            "owner": self.origin.owner,
            "scope": self.origin.scope,
            "session": self.session,
            "machine": self.machine,
            "asked": self.asked,
        })
    }

    fn from_value(value: &Value) -> Option<Self> {
        let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
        let operation = pb::Operation::decode(bytes(value.get("operation")?)?.as_slice()).ok()?;
        let scope = match value.get("scope")? {
            Value::Null => None,
            Value::Array(pair) => match pair.as_slice() {
                [channel, session] => Some((channel.as_str()?.to_owned(), session.as_str()?.to_owned())),
                _ => return None,
            },
            _ => return None,
        };
        Some(Self {
            operation,
            origin: Origin {
                owner: text("owner")?,
                scope,
            },
            session: text("session")?,
            machine: text("machine")?,
            asked: text("asked"),
        })
    }
}

/// Bytes kept as a JSON array of numbers.
fn bytes(value: &Value) -> Option<Vec<u8>> {
    value
        .as_array()?
        .iter()
        .map(|byte| byte.as_u64().and_then(|byte| u8::try_from(byte).ok()))
        .collect()
}

/// Where the kept pushes are, in `root`.
pub fn folder_of(root: &Path) -> PathBuf {
    root.join("push")
}

fn path_of(folder: &Path, kept: &Kept) -> PathBuf {
    folder.join(format!("{}.json", kept.id()))
}

/// Keeps `kept` on disk in `folder`. Blocking.
pub fn write(folder: &Path, kept: &Kept) -> Result<(), KeptError> {
    std::fs::create_dir_all(folder).map_err(|source| KeptError::Folder {
        path: folder.to_owned(),
        source,
    })?;
    let path = path_of(folder, kept);
    let bytes = serde_json::to_vec_pretty(&kept.value()).unwrap_or_default();
    humata2_store::write_atomic(&path, &bytes).map_err(|source| KeptError::Write { path, source })
}

/// `kept` is done or moot; its file goes. Blocking.
pub fn remove(folder: &Path, kept: &Kept) {
    let _ = std::fs::remove_file(path_of(folder, kept));
}

/// Every push the last run left. A file that can't be read is left in place. Blocking.
pub fn read_all(folder: &Path) -> Vec<Kept> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") || !path.is_file() {
            continue;
        }
        let read = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|value| Kept::from_value(&value));
        if let Some(kept) = read {
            found.push(kept);
        }
    }
    found
}

impl SessionTransferSvc {
    /// Keeps `kept` on disk, so a restart goes on following the push. Best effort.
    pub async fn keep_push(&self, kept: &Kept) {
        let folder = folder_of(&self.env.state_dir());
        let copy = kept.clone();
        let _ = blocking(move || write(&folder, &copy)).await;
    }

    /// The host's operation ended, or the push is moot; its file goes.
    pub async fn drop_kept(&self, kept: &Kept) {
        let folder = folder_of(&self.env.state_dir());
        let done = kept.clone();
        let _ = blocking(move || remove(&folder, &done)).await;
    }

    /// Reads all kept pushes from the state directory.
    pub fn kept_pushes(&self) -> Vec<Kept> {
        read_all(&folder_of(&self.env.state_dir()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kept(asked: Option<&str>, scope: Option<(&str, &str)>) -> Kept {
        Kept {
            operation: pb::Operation {
                id: Some(pb::OperationId {
                    value: "op_1".to_owned(),
                }),
                state: pb::OperationState::Running as i32,
                version: 1,
                ..Default::default()
            },
            origin: Origin {
                owner: "session:se_1".to_owned(),
                scope: scope.map(|(channel, session)| (channel.to_owned(), session.to_owned())),
            },
            session: "se_1".to_owned(),
            machine: "box-1".to_owned(),
            asked: asked.map(str::to_owned),
        }
    }

    #[test]
    fn a_kept_push_reads_back_as_written_and_goes_when_removed() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("push");
        let one = kept(Some("op_9"), Some(("ch_1", "se_1")));
        write(&folder, &one).unwrap();
        let read = read_all(&folder);
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].operation, one.operation);
        assert_eq!(read[0].origin.owner, one.origin.owner);
        assert_eq!(read[0].origin.scope, one.origin.scope);
        assert_eq!(
            (read[0].session.as_str(), read[0].machine.as_str()),
            ("se_1", "box-1")
        );
        assert_eq!(read[0].asked.as_deref(), Some("op_9"));
        remove(&folder, &one);
        assert!(read_all(&folder).is_empty());
    }

    #[test]
    fn a_push_the_machine_never_confirmed_reads_back_without_its_operation() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("push");
        write(&folder, &kept(None, None)).unwrap();
        let read = read_all(&folder);
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].asked, None);
        assert_eq!(read[0].origin.scope, None);
    }

    #[test]
    fn a_file_that_is_not_a_kept_push_is_left_and_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("push");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("op_2.json"), "not json").unwrap();
        assert!(read_all(&folder).is_empty());
        assert!(folder.join("op_2.json").exists());
    }
}

//! The host's journal of the session starts it asks another machine for
//! (D432). Kept from before the host saves the session's record until the
//! machine answers the start with the session or refuses it, so a power cut
//! on either side never leaves a record on the host that nothing runs.

use std::path::{Path, PathBuf};

use humata2_store::StoreError;
use humata2_svc_sessions_common::pb;
use prost::Message;
use serde_json::{Value, json};
use tonic::Status;

use crate::SessionTransferSvc;
use crate::ports::{blocking, chain, internal};
use crate::push::resume::Origin;

/// How the restart report names these operations.
pub const KIND: &str = "remote_start";
pub const STEP: &str = "asked";

/// Why a start's journal couldn't be kept on disk.
#[derive(Debug, thiserror::Error)]
pub enum AskedError {
    #[error("couldn't make the folder {path} for the session starts that wait on another machine")]
    Folder {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("couldn't write {path}, a session start that waits on another machine")]
    Write {
        path: PathBuf,
        #[source]
        source: StoreError,
    },
}

/// What a restart needs to ask a machine again for a session's start.
#[derive(Clone, Debug)]
pub struct Asked {
    /// The host's operation as it started.
    pub operation: pb::Operation,
    pub origin: Origin,
    pub session: String,
    pub machine: String,
    /// The start as the caller asked it, without its idempotency key.
    pub request: pb::StartSessionRequest,
}

impl Asked {
    pub fn new(
        operation: pb::Operation,
        origin: Origin,
        session: &str,
        machine: &str,
        request: &pb::StartSessionRequest,
    ) -> Self {
        Self {
            operation,
            origin,
            session: session.to_owned(),
            machine: machine.to_owned(),
            request: pb::StartSessionRequest {
                idempotency_key: None,
                ..request.clone()
            },
        }
    }

    fn value(&self) -> Value {
        json!({
            "operation": self.operation.encode_to_vec(),
            "owner": self.origin.owner,
            "scope": self.origin.scope,
            "session": self.session,
            "machine": self.machine,
            "request": self.request.encode_to_vec(),
        })
    }

    fn from_value(value: &Value) -> Option<Self> {
        let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
        let operation = pb::Operation::decode(bytes(value.get("operation")?)?.as_slice()).ok()?;
        let request =
            pb::StartSessionRequest::decode(bytes(value.get("request")?)?.as_slice()).ok()?;
        let scope = match value.get("scope")? {
            Value::Null => None,
            Value::Array(pair) => match pair.as_slice() {
                [channel, session] => {
                    Some((channel.as_str()?.to_owned(), session.as_str()?.to_owned()))
                }
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
            request,
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

/// Where the starts wait, in `root`.
pub fn folder_of(root: &Path) -> PathBuf {
    root.join("start")
}

pub fn path_of(folder: &Path, session: &str) -> PathBuf {
    folder.join(format!("{session}.json"))
}

/// Keeps `asked` on disk in `folder`. Blocking.
pub fn write(folder: &Path, asked: &Asked) -> Result<(), AskedError> {
    std::fs::create_dir_all(folder).map_err(|source| AskedError::Folder {
        path: folder.to_owned(),
        source,
    })?;
    let path = path_of(folder, &asked.session);
    let bytes = serde_json::to_vec_pretty(&asked.value()).unwrap_or_default();
    humata2_store::write_atomic(&path, &bytes).map_err(|source| AskedError::Write { path, source })
}

/// Every start the last run left unanswered. A file that can't be read is
/// left in place. Blocking.
pub fn read_all(folder: &Path) -> Vec<Asked> {
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
            .and_then(|value| Asked::from_value(&value));
        if let Some(asked) = read {
            found.push(asked);
        }
    }
    found
}

impl SessionTransferSvc {
    /// Keeps `asked` on disk before the host saves the session's record.
    pub async fn keep_asked(&self, asked: &Asked) -> Result<(), Status> {
        let folder = folder_of(&self.env.state_dir());
        let copy = asked.clone();
        match blocking(move || write(&folder, &copy)).await? {
            Ok(()) => Ok(()),
            Err(error) => Err(internal(
                format!(
                    "{}; the session wasn't started on {}, so check that the data folder's disk isn't full and start it again",
                    chain(&error),
                    asked.machine
                ),
                "",
            )),
        }
    }

    /// The start of `session` is answered, refused or moot; its file goes.
    pub async fn drop_asked(&self, session: &str) {
        let path = path_of(&folder_of(&self.env.state_dir()), session);
        let _ = blocking(move || std::fs::remove_file(path)).await;
    }

    /// The start of `session` failed before the machine took it: the host's
    /// record goes, then its journal.
    pub async fn remove_placed_then_asked(&self, session: &str) {
        let (store, removing) = (self.core.store.clone(), session.to_owned());
        if let Ok(Ok(())) = blocking(move || store.remove_session(&removing)).await {
            self.drop_asked(session).await;
        }
    }

    /// Reads all unanswered starts from the state directory.
    pub fn kept_starts(&self) -> Vec<Asked> {
        read_all(&folder_of(&self.env.state_dir()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asked(session: &str) -> Asked {
        Asked {
            operation: pb::Operation {
                id: Some(pb::OperationId {
                    value: "op_1".to_owned(),
                }),
                ..pb::Operation::default()
            },
            origin: Origin {
                owner: "app".to_owned(),
                scope: Some(("ch_1".to_owned(), session.to_owned())),
            },
            session: session.to_owned(),
            machine: "devbox".to_owned(),
            request: pb::StartSessionRequest {
                title: "Fix the login".to_owned(),
                branch: "fix-login".to_owned(),
                first_message: "look at the login form".to_owned(),
                ..pb::StartSessionRequest::default()
            },
        }
    }

    /// A start kept before a power cut is there to ask again after it, as
    /// it was asked; one whose answer came is not.
    #[test]
    fn a_kept_start_reads_back_until_it_is_answered() {
        let root = tempfile::tempdir().unwrap();
        let folder = folder_of(root.path());
        write(&folder, &asked("se_1")).unwrap();
        write(&folder, &asked("se_2")).unwrap();
        std::fs::remove_file(path_of(&folder, "se_2")).unwrap();

        let back = read_all(&folder);
        assert_eq!(back.len(), 1);
        let kept = &back[0];
        let want = asked("se_1");
        assert_eq!(kept.session, want.session);
        assert_eq!(kept.machine, want.machine);
        assert_eq!(kept.request, want.request);
        assert_eq!(kept.operation, want.operation);
        assert_eq!(kept.origin.owner, want.origin.owner);
        assert_eq!(kept.origin.scope, want.origin.scope);
    }

    /// A file that isn't a kept start is left alone and asks nothing.
    #[test]
    fn an_unreadable_start_is_skipped_and_kept() {
        let root = tempfile::tempdir().unwrap();
        let folder = folder_of(root.path());
        std::fs::create_dir_all(&folder).unwrap();
        let damaged = folder.join("se_9.json");
        std::fs::write(&damaged, b"{not json").unwrap();

        assert!(read_all(&folder).is_empty());
        assert!(damaged.exists());
    }
}

//! A push on another machine that the host was following when it stopped
//! (the restart's follow-up of F3). While the host follows the machine's
//! operation it keeps one JSON file per push in the carry's
//! `remote_archive/push` folder: the host's operation as it started, whose
//! it is, the session, the machine, and the machine's own operation once it
//! answered. Each is written before the machine is asked, written again
//! with the machine's operation, and removed once the host's operation has
//! ended. At the next start the host brings each operation back under its
//! own id, waits for the machine's link, follows the machine's operation to
//! its end and ends the host's with the machine's pull requests or error,
//! the way `archive_remote` goes on with what waited for a link (D222).
//!
//! Nothing here waits on a timer: a machine that is away keeps the push
//! waiting, and each wait is the link's own change.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use humata2_grpc::humata::v1 as pb;
use humata2_store::StoreError;
use prost::Message;
use serde_json::{Value, json};
use tonic::{Code, Status};

use super::super::archive_remote::{follow_to_end, next_link_up};
use super::{RemotePushError, SessionSvc, end_push};
use crate::carry::Carry;
use crate::channel_service::{blocking, failed};
use crate::host_relay::Links;
use crate::operations::{Origin, Progress};
use crate::restart::{OperationState, Started};
use crate::status;

/// Why a push couldn't be kept on disk.
#[derive(Debug, thiserror::Error)]
enum KeptError {
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
pub(super) struct Kept {
    /// The host's operation as it started.
    operation: pb::Operation,
    origin: Origin,
    session: String,
    machine: String,
    /// The machine's own operation, once it answered.
    pub(super) asked: Option<String>,
}

impl Kept {
    pub(super) fn new(progress: &Progress, session: &str, machine: &str) -> Self {
        Self {
            operation: progress.operation(),
            origin: progress.origin(),
            session: session.to_owned(),
            machine: machine.to_owned(),
            asked: None,
        }
    }

    fn id(&self) -> &str {
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

/// Where the kept pushes are, in the carry's `remote_archive` folder `root`.
fn folder_of(root: &Path) -> PathBuf {
    root.join("push")
}

fn path_of(folder: &Path, kept: &Kept) -> PathBuf {
    folder.join(format!("{}.json", kept.id()))
}

/// Keeps `kept` on disk in `folder`. Blocking.
fn write(folder: &Path, kept: &Kept) -> Result<(), KeptError> {
    std::fs::create_dir_all(folder).map_err(|source| KeptError::Folder {
        path: folder.to_owned(),
        source,
    })?;
    let path = path_of(folder, kept);
    let bytes = serde_json::to_vec_pretty(&kept.value()).unwrap_or_default();
    humata2_store::write_atomic(&path, &bytes).map_err(|source| KeptError::Write { path, source })
}

/// `kept` is done or moot; its file goes. Blocking.
fn remove(folder: &Path, kept: &Kept) {
    let _ = std::fs::remove_file(path_of(folder, kept));
}

/// Every push the last run left. A file that can't be read is logged and
/// left in place. Blocking.
fn read_all(folder: &Path) -> Vec<Kept> {
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
        match read {
            Some(kept) => found.push(kept),
            None => crate::log::say!(
                "humatad2: couldn't read {}, a push on another machine that Humata was following, so it isn't followed. Nothing was deleted; look at the session's branches and pull requests on its machine, then delete the file",
                path.display()
            ),
        }
    }
    found
}

impl SessionSvc {
    /// Keeps `kept` on disk, so a restart goes on following the push. Best
    /// effort: a push that can't be kept still runs, and the log says so.
    pub(super) async fn keep_push(&self, kept: &Kept) {
        let Some(root) = self.carry.get().map(Carry::remote_archive_folder) else {
            return;
        };
        let (folder, copy) = (folder_of(&root), kept.clone());
        if let Ok(Err(error)) = blocking(move || write(&folder, &copy)).await {
            crate::log::say!(
                "humatad2: {}; if Humata restarts before {} finishes the push, look at session {} on it and push again",
                status::chain(&error),
                kept.machine,
                kept.session
            );
        }
    }

    /// The host's operation ended, or the push is moot; its file goes.
    pub(super) async fn drop_kept(&self, kept: &Kept) {
        let Some(root) = self.carry.get().map(Carry::remote_archive_folder) else {
            return;
        };
        let (folder, done) = (folder_of(&root), kept.clone());
        let _ = blocking(move || remove(&folder, &done)).await;
    }

    /// Brings back the host's operation of each push the last run left, at
    /// once and under its own id, and follows each to its end once the start
    /// is done. Call it once the carry is set, before anything is served.
    pub(in crate::session_service) fn recover_remote_pushes_after(&self, started: Arc<Started>) {
        let Some(root) = self.carry.get().map(Carry::remote_archive_folder) else {
            return;
        };
        for kept in read_all(&folder_of(&root)) {
            let id = kept.id().to_owned();
            let Some(progress) = self.operations.restore(kept.operation.clone(), &kept.origin) else {
                crate::log::say!(
                    "humatad2: couldn't bring back push {id} on {}, so it isn't followed. Nothing was deleted; look at session {} on {}, then delete its file",
                    kept.machine,
                    kept.session,
                    kept.machine
                );
                continue;
            };
            started.operations().record(
                &id,
                "push",
                "following",
                OperationState::Pending {
                    waits_for: format!("the start to finish, then the link of {}", kept.machine),
                },
            );
            let (svc, started) = (self.clone(), started.clone());
            tokio::spawn(async move {
                started.wait().await;
                svc.follow_kept(&started, progress, kept).await;
            });
        }
    }

    /// Follows the machine's operation of `kept` to its end, ends the host's
    /// `progress` with how it went and drops the record.
    async fn follow_kept(self, started: &Started, progress: Progress, kept: Kept) {
        let ended = self.follow_until_ended(&kept, &progress).await;
        let state = match &ended {
            Ok(_) => OperationState::Finished,
            Err(refused) => OperationState::Undone {
                reason: format!("the push couldn't be followed to its end: {}", refused.message()),
            },
        };
        end_push(progress, &kept.machine, ended);
        self.drop_kept(&kept).await;
        started.operations().record(kept.id(), "push", "following", state);
        self.tell
            .session(&kept.session, pb::Change::Updated, kept.operation.started_by.clone())
            .await;
    }

    /// The machine's operation, once it ends. Each time the link comes up it
    /// follows again, until the machine answers or is forgotten.
    async fn follow_until_ended(&self, kept: &Kept, progress: &Progress) -> Result<pb::Operation, Status> {
        let machine = kept.machine.as_str();
        let Some(asked) = kept.asked.as_deref() else {
            return Err(failed(
                Code::FailedPrecondition,
                &RemotePushError::NotConfirmed {
                    machine: machine.to_owned(),
                },
                &format!(
                    "look at the session's branches and pull requests on {machine}, then push again: what was pushed stays pushed"
                ),
                None,
            ));
        };
        let Some(links) = self.carry.get().map(|carry| carry.links().clone()) else {
            return Err(failed(
                Code::Internal,
                &RemotePushError::NoLinks {
                    machine: machine.to_owned(),
                },
                "restart Humata; the push is still on the machine",
                None,
            ));
        };
        let mut stale = None;
        while let Some(generation) = next_link_up(&links, machine, stale).await {
            match follow_once(&links, machine, asked, progress).await {
                Err(refused) if refused.code() == Code::Unavailable => stale = Some(generation),
                ended => return ended,
            }
        }
        Err(failed(
            Code::NotFound,
            &RemotePushError::Forgotten {
                machine: machine.to_owned(),
            },
            "what it pushed stays pushed; look at the session's pull requests on GitHub, then push again from wherever the session runs now",
            None,
        ))
    }
}

/// Follows the machine's operation `asked` on the link as it is now. A link
/// that breaks first answers `Unavailable`.
async fn follow_once(
    links: &Arc<dyn humata2_svc_transfers::ports::LinkProvider>,
    machine: &str,
    asked: &str,
    progress: &Progress,
) -> Result<pb::Operation, Status> {
    let relayed = super::super::move_recover::relayed_provider(links, machine).map_err(Status::from)?;
    // Only its id and that it runs: the machine's watch tells the rest.
    let running = pb::Operation {
        id: Some(pb::OperationId {
            value: asked.to_owned(),
        }),
        state: pb::OperationState::Running as i32,
        ..Default::default()
    };
    follow_to_end(&relayed, machine, running, progress, || {
        failed(
            Code::Unavailable,
            &RemotePushError::LinkLost {
                machine: machine.to_owned(),
            },
            "Humata goes on following the push when the link is back",
            None,
        )
    })
    .await
    .map_err(|refused| {
        if refused.code() == Code::NotFound {
            failed(
                Code::NotFound,
                &RemotePushError::Unknown {
                    machine: machine.to_owned(),
                    operation: asked.to_owned(),
                },
                &format!(
                    "look at the session's branches and pull requests on {machine}, then push again: what was pushed stays pushed"
                ),
                None,
            )
        } else {
            refused
        }
    })
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
        assert_eq!((read[0].session.as_str(), read[0].machine.as_str()), ("se_1", "box-1"));
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

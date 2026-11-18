//! The host's journal of the session starts it asks another machine for
//! (D432). Kept from before the host saves the session's record until the
//! machine answers the start with the session or refuses it, so a power cut
//! on either side never leaves a record on the host that nothing runs: the
//! next start asks the machine again under the same id once it links, and
//! the machine answers with the one session it kept or starts it (D150).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use humata2_grpc::humata::v1 as pb;
use humata2_store::StoreError;
use prost::Message;
use serde_json::{Value, json};
use tonic::Status;

use super::SessionSvc;
use crate::carry::Carry;
use crate::channel_service::blocking;
use crate::host_relay::RelayError;
use crate::operations::{Origin, Progress};
use crate::remote_work::{Answered, HostOp, RemoteError};
use crate::restart::{OperationState, Started};
use crate::status;

/// How the restart report names these operations.
const KIND: &str = "remote_start";
const STEP: &str = "asked";

/// Why a start's journal couldn't be kept on disk.
#[derive(Debug, thiserror::Error)]
enum AskedError {
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
pub(super) struct Asked {
    /// The host's operation as it started.
    operation: pb::Operation,
    origin: Origin,
    session: String,
    machine: String,
    /// The start as the caller asked it, without its idempotency key.
    request: pb::StartSessionRequest,
}

impl Asked {
    pub(super) fn new(progress: &Progress, session: &str, machine: &str, request: &pb::StartSessionRequest) -> Self {
        Self {
            operation: progress.operation(),
            origin: progress.origin(),
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
        let request = pb::StartSessionRequest::decode(bytes(value.get("request")?)?.as_slice()).ok()?;
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

/// Where the starts wait, in the carry's `remote_archive` folder `root`.
fn folder_of(root: &Path) -> PathBuf {
    root.join("start")
}

fn path_of(folder: &Path, session: &str) -> PathBuf {
    folder.join(format!("{session}.json"))
}

/// Keeps `asked` on disk in `folder`, the folder's entry included. Blocking.
fn write(folder: &Path, asked: &Asked) -> Result<(), AskedError> {
    // Every folder this makes, `carry/remote_archive` too, stays only once
    // its own parent is synced: a power cut takes an unsynced one, and the
    // journal inside it, with it.
    let missing: Vec<PathBuf> = folder
        .ancestors()
        .take_while(|found| !found.as_os_str().is_empty() && std::fs::symlink_metadata(found).is_err())
        .map(Path::to_owned)
        .collect();
    humata2_durable::create_dir_all(folder, 0o700).map_err(|source| AskedError::Folder {
        path: folder.to_owned(),
        source,
    })?;
    for made in &missing {
        if let Some(parent) = made.parent().filter(|parent| !parent.as_os_str().is_empty()) {
            humata2_durable::sync_dir(parent).map_err(|source| AskedError::Folder {
                path: made.clone(),
                source,
            })?;
        }
    }
    let path = path_of(folder, &asked.session);
    let bytes = serde_json::to_vec_pretty(&asked.value()).unwrap_or_default();
    humata2_store::write_atomic(&path, &bytes).map_err(|source| AskedError::Write { path, source })
}

/// Every start the last run left unanswered. A file that can't be read is
/// logged and left in place. Blocking.
fn read_all(folder: &Path) -> Vec<Asked> {
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
        match read {
            Some(asked) => found.push(asked),
            None => crate::log::say!(
                "humatad2: couldn't read {}, a session start Humata asked another machine for, so it isn't asked again. Nothing was deleted; look for the session on that machine, start it again if it isn't there, then delete the file",
                path.display()
            ),
        }
    }
    found
}

impl SessionSvc {
    /// Keeps `asked` on disk before the host saves the session's record.
    /// A start that can't be kept isn't made: a power cut could leave a
    /// record nothing runs.
    pub(super) async fn keep_asked(&self, asked: &Asked) -> Result<(), Status> {
        let Some(root) = self.carry.get().map(Carry::remote_archive_folder) else {
            return Ok(());
        };
        let (folder, copy) = (folder_of(&root), asked.clone());
        match blocking(move || write(&folder, &copy)).await? {
            Ok(()) => Ok(()),
            Err(error) => Err(status::internal(
                format!(
                    "{}; the session wasn't started on {}, so check that the data folder's disk isn't full and start it again",
                    status::chain(&error),
                    asked.machine
                ),
                "",
            )),
        }
    }

    /// The start of `session` is answered, refused or moot; its file goes.
    pub(super) async fn drop_asked(&self, session: &str) {
        let Some(root) = self.carry.get().map(Carry::remote_archive_folder) else {
            return;
        };
        let path = path_of(&folder_of(&root), session);
        let _ = blocking(move || std::fs::remove_file(path)).await;
    }

    /// What the remote side tells once the machine answered the start of
    /// `session`: its file goes.
    pub(super) fn answered_hook(&self, session: &str) -> Answered {
        let path = self
            .carry
            .get()
            .map(|carry| path_of(&folder_of(&carry.remote_archive_folder()), session));
        Arc::new(move || {
            if let Some(path) = path.clone() {
                tokio::task::spawn_blocking(move || {
                    let _ = std::fs::remove_file(path);
                });
            }
        })
    }

    /// Brings back the host's operation of each start the last run asked a
    /// machine for and heard no answer to, at once and under its own id, and
    /// asks again once the start is done and the machine links. Call it once
    /// the carry is set, before anything is served.
    pub(in crate::session_service) fn recover_remote_starts_after(&self, started: Arc<Started>) {
        let Some(root) = self.carry.get().map(Carry::remote_archive_folder) else {
            return;
        };
        for asked in read_all(&folder_of(&root)) {
            let Some(progress) = self.operations.restore(asked.operation.clone(), &asked.origin) else {
                crate::log::say!(
                    "humatad2: couldn't bring back the start of session {} on {}, so it isn't asked again. Nothing was deleted; look for the session on {}, start it again if it isn't there, then delete its file",
                    asked.session,
                    asked.machine,
                    asked.machine
                );
                continue;
            };
            started.operations().record(
                &asked.session,
                KIND,
                STEP,
                OperationState::Pending {
                    waits_for: format!("the start to finish, then the link of {}", asked.machine),
                },
            );
            let (svc, started) = (self.clone(), started.clone());
            tokio::spawn(async move {
                started.wait().await;
                let state = svc.ask_again(progress, &asked).await;
                started.operations().record(&asked.session, KIND, STEP, state);
            });
        }
    }

    /// Asks `asked.machine` again for the start, under the same id, once it
    /// links. The host's record names the branch the machine makes.
    async fn ask_again(&self, progress: Progress, asked: &Asked) -> OperationState {
        let (Some(carry), Some(remote)) = (self.carry.get().cloned(), self.remote.clone()) else {
            progress.end_cancelled();
            return OperationState::Stuck {
                reason: "this daemon doesn't work on other machines now".to_owned(),
            };
        };
        let (store, id) = (self.starter.store.clone(), asked.session.clone());
        let read = blocking(move || -> Result<_, StoreError> {
            Ok((store.session(&id)?, store.archive(&id)?, store.session_detail(&id)?))
        })
        .await;
        let detail = match read {
            Ok(Ok((Some(_), None, detail))) => detail,
            Ok(Ok(_)) => {
                // Never saved, or archived or removed since: nothing to start.
                progress.end_cancelled();
                self.drop_asked(&asked.session).await;
                return OperationState::Undone {
                    reason: format!("the host has no live session {} to start on {}", asked.session, asked.machine),
                };
            }
            Ok(Err(error)) => {
                progress.end_cancelled();
                return OperationState::Stuck {
                    reason: format!("couldn't read session {}: {}", asked.session, status::chain(&error)),
                };
            }
            Err(status) => {
                progress.end_cancelled();
                return OperationState::Stuck {
                    reason: status.message().to_owned(),
                };
            }
        };
        let branch = detail
            .and_then(|detail| detail.repos.into_iter().next())
            .map_or_else(|| asked.request.branch.clone(), |repo| repo.branch);
        let request = pb::StartSessionRequest {
            branch,
            ..asked.request.clone()
        };
        let mut progress = progress;
        loop {
            if super::move_recover::wait_up(carry.links(), &asked.machine).await.is_err() {
                progress.end_cancelled();
                return OperationState::Stuck {
                    reason: format!("{} was removed, or this daemon stopped, before it linked", asked.machine),
                };
            }
            let host = Arc::new(HostOp::new(progress));
            let hook = self.answered_hook(&asked.session);
            match remote
                .start_session_answered(&asked.machine, &asked.session, request.clone(), host.clone(), Some(hook))
                .await
            {
                Ok(_) => {
                    self.tell.session(&asked.session, pb::Change::Updated, None).await;
                    return OperationState::Finished;
                }
                Err(RemoteError::Relay(RelayError::Offline { .. })) => {
                    let Some(back) = host.take() else {
                        return OperationState::Stuck {
                            reason: format!("the start of session {} ended while {} was offline", asked.session, asked.machine),
                        };
                    };
                    progress = back;
                    // The link went down between its state and the call.
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                Err(error) => {
                    let refusal = Status::from(error);
                    if let Some(progress) = host.take() {
                        progress.fail(super::archive::failure_of(&refusal));
                    }
                    // The machine won't run it: the host's record would be a
                    // session nothing runs.
                    self.tell.session(&asked.session, pb::Change::Deleted, None).await;
                    let (store, removing) = (self.starter.store.clone(), asked.session.clone());
                    let _ = blocking(move || store.remove_session(&removing)).await;
                    self.drop_asked(&asked.session).await;
                    return OperationState::Undone {
                        reason: format!(
                            "{} refused the start ({}); start the session again",
                            asked.machine,
                            refusal.message()
                        ),
                    };
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asked(session: &str) -> Asked {
        Asked {
            operation: pb::Operation {
                id: Some(pb::OperationId { value: "op_1".to_owned() }),
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

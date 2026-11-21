//! The host's journal of the Desk starts it asks another machine for
//! (D731), as D432 keeps one for a channel session's start. Kept from
//! before the host saves the session's records until the machine answers
//! the start or refuses it. The host's record is saved as started, so a
//! restart's recovery never undoes it as a start cut short here: a host that
//! dies between the ask and the answer asks the machine again under the same
//! id once it links, and the machine answers with the session it kept, or
//! starts it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::pb;
use humata2_store::StoreError;
use prost::Message;
use serde_json::{Value, json};
use tonic::Status;

use super::DeskSvc;
use super::remote::{failure_of, is_definitive_refusal};
use crate::ports::blocking;
use crate::ports::{Origin, Progress};
use crate::ports::HostOp;
use crate::ports::OperationState;
use crate::ports as status;

/// The folder in the state folder the starts wait in.
const DIR: &str = "desk_remote_start";
/// How the restart report names these operations.
const KIND: &str = "remote_desk_start";
const STEP: &str = "asked";

/// Why a start's journal couldn't be kept on disk.
#[derive(Debug, thiserror::Error)]
enum AskedError {
    #[error("couldn't make the folder {path} for the Desk starts that wait on another machine")]
    Folder {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("couldn't write {path}, a Desk start that waits on another machine")]
    Write {
        path: PathBuf,
        #[source]
        source: StoreError,
    },
}

/// What a restart needs to ask a machine again for a Desk session's start.
#[derive(Clone, Debug)]
pub(super) struct Asked {
    /// The host's operation as it started.
    operation: pb::Operation,
    origin: Origin,
    session: String,
    machine: String,
    /// The start as the machine is asked it: title and agent set, with no
    /// machine and no idempotency key.
    request: pb::StartDeskSessionRequest,
    /// A channel's folder name, for a channel's Desk session.
    channel_folder: Option<String>,
}

impl Asked {
    pub(super) fn new(
        progress: &Progress,
        session: &str,
        machine: &str,
        request: &pb::StartDeskSessionRequest,
        channel_folder: Option<&str>,
    ) -> Self {
        Self {
            operation: progress.operation(),
            origin: progress.origin(),
            session: session.to_owned(),
            machine: machine.to_owned(),
            request: pb::StartDeskSessionRequest {
                idempotency_key: None,
                machine: None,
                ..request.clone()
            },
            channel_folder: channel_folder.map(str::to_owned),
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
            "channel_folder": self.channel_folder,
        })
    }

    fn from_value(value: &Value) -> Option<Self> {
        let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
        let operation = pb::Operation::decode(bytes(value.get("operation")?)?.as_slice()).ok()?;
        let request = pb::StartDeskSessionRequest::decode(bytes(value.get("request")?)?.as_slice()).ok()?;
        let scope = match value.get("scope")? {
            Value::Null => None,
            Value::Array(pair) => match pair.as_slice() {
                [channel, session] => Some((channel.as_str()?.to_owned(), session.as_str()?.to_owned())),
                _ => return None,
            },
            _ => return None,
        };
        let channel_folder = match value.get("channel_folder") {
            None | Some(Value::Null) => None,
            Some(Value::String(folder)) => Some(folder.clone()),
            Some(_) => return None,
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
            channel_folder,
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

fn path_of(folder: &Path, session: &str) -> PathBuf {
    folder.join(format!("{session}.json"))
}

/// Keeps `asked` on disk in `folder`, the folder's entry included. Blocking.
fn write(folder: &Path, asked: &Asked) -> Result<(), AskedError> {
    let made = std::fs::symlink_metadata(folder).is_err();
    humata2_durable::create_dir_all(folder, 0o700).map_err(|source| AskedError::Folder {
        path: folder.to_owned(),
        source,
    })?;
    // A folder made now stays only once its parent is synced.
    if made && let Some(parent) = folder.parent() {
        humata2_durable::sync_dir(parent).map_err(|source| AskedError::Folder {
            path: folder.to_owned(),
            source,
        })?;
    }
    let path = path_of(folder, &asked.session);
    let bytes = serde_json::to_vec_pretty(&asked.value()).unwrap_or_default();
    humata2_store::write_atomic(&path, &bytes).map_err(|source| AskedError::Write { path, source })
}

/// Every start the last run left unanswered. A file that can't be read is
/// logged and left in place. Blocking.
pub(super) fn read_all(folder: &Path) -> Vec<Asked> {
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
            None => humata2_rpc_kit::log::say!(
                "humatad2: couldn't read {}, a Desk session start Humata asked another machine for, so it isn't asked again. Nothing was deleted; look for the session on that machine, start it again if it isn't there, then delete the file",
                path.display()
            ),
        }
    }
    found
}

impl DeskSvc {
    pub(super) fn asked_folder(&self) -> PathBuf {
        self.state.join(DIR)
    }

    /// Keeps `asked` on disk before the host saves the session's records.
    /// A start that can't be kept isn't made: a host that died after asking
    /// would have nothing to ask the machine again with.
    pub(super) async fn keep_asked(&self, asked: &Asked) -> Result<(), Status> {
        let (folder, copy) = (self.asked_folder(), asked.clone());
        match blocking(move || write(&folder, &copy)).await? {
            Ok(()) => Ok(()),
            Err(error) => Err(status::internal(
                format!(
                    "{}; the Desk session wasn't started on {}",
                    status::chain(&error),
                    asked.machine
                ),
                "check that Humata's state folder can be written to and its disk isn't full, then start the session again",
            )),
        }
    }

    /// The start of `session` is answered, refused or moot; its file goes.
    pub(super) async fn drop_asked(&self, session: &str) {
        let path = path_of(&self.asked_folder(), session);
        let _ = blocking(move || std::fs::remove_file(path)).await;
    }

    /// Asks `asked.machine` again for the start, under the same id, once it
    /// links. The machine answers with the session it kept, or starts it.
    pub(super) async fn ask_again(&self, progress: Progress, asked: &Asked) -> OperationState {
        let Some(remote) = self.remote.clone() else {
            progress.end_cancelled();
            return OperationState::Stuck {
                reason: "this daemon doesn't work on other machines now".to_owned(),
            };
        };
        let (store, id) = (self.starter.store.clone(), asked.session.clone());
        let record = match blocking(move || store.session(&id)).await {
            Ok(Ok(Some(record))) => record,
            Ok(Ok(None)) => {
                // Never saved, or removed since: nothing to start.
                progress.end_cancelled();
                self.drop_asked(&asked.session).await;
                return OperationState::Undone {
                    reason: format!("the host has no Desk session {} to start on {}", asked.session, asked.machine),
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
        let mut progress = progress;
        loop {
            if remote.wait_up(&asked.machine).await.is_err() {
                progress.end_cancelled();
                return OperationState::Stuck {
                    reason: format!("{} was removed, or this daemon stopped, before it linked", asked.machine),
                };
            }
            let host = Arc::new(HostOp::new(progress));
            match remote
                .start_desk_on(
                    &asked.machine,
                    &asked.session,
                    asked.request.clone(),
                    asked.channel_folder.as_deref(),
                    host.clone(),
                )
                .await
            {
                Ok(response) => {
                    let folder = response
                        .desk
                        .as_ref()
                        .and_then(|desk| desk.folder.as_ref())
                        .map(|folder| PathBuf::from(&folder.path));
                    if let Some(folder) = folder {
                        let mut saving = record;
                        saving.folder = folder;
                        let store = self.starter.store.clone();
                        if let Ok(Err(error)) = blocking(move || store.put_session(&saving)).await {
                            humata2_rpc_kit::log::say!(
                                "humatad2: couldn't save where session {} works on {}: {}; its folder shows empty until the session moves or is started again",
                                asked.session,
                                asked.machine,
                                status::chain(&error)
                            );
                        }
                    }
                    self.drop_asked(&asked.session).await;
                    self.tell.session(&asked.session, pb::Change::Updated, None).await;
                    return OperationState::Finished;
                }
                Err(error) if !is_definitive_refusal(&error) => {
                    let Some(back) = host.take() else {
                        return OperationState::Stuck {
                            reason: format!(
                                "the start of Desk session {} ended while {} was unreachable",
                                asked.session, asked.machine
                            ),
                        };
                    };
                    progress = back;
                    // The link went down or the call ended ambiguously; ask again once linked.
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                Err(error) => {
                    let refusal = Status::from(error);
                    if let Some(progress) = host.take() {
                        progress.fail(failure_of(&refusal));
                    }
                    // The machine won't run it: the host's record would be a
                    // session nothing runs. Only the host's records go; no
                    // folder is touched.
                    self.tell.session(&asked.session, pb::Change::Deleted, None).await;
                    let (store, removing) = (self.starter.store.clone(), asked.session.clone());
                    let _ = blocking(move || store.remove_session(&removing)).await;
                    self.drop_asked(&asked.session).await;
                    return OperationState::Undone {
                        reason: format!(
                            "{} refused the start ({}); start the Desk session again",
                            asked.machine,
                            refusal.message()
                        ),
                    };
                }
            }
        }
    }
}

/// Brings back the host's operation of each Desk start the last run asked a
/// machine for and heard no answer to, at once and under its own id, and
/// asks again once the start is done and the machine links. Call it before
/// anything is served.
pub(super) fn recover_after(svc: DeskSvc, started: Arc<dyn crate::ports::DeskRestartPort>) {
    for asked in read_all(&svc.asked_folder()) {
        let Some(progress) = svc.operations.restore(asked.operation.clone(), &asked.origin) else {
            humata2_rpc_kit::log::say!(
                "humatad2: couldn't bring back the start of Desk session {} on {}, so it isn't asked again. Nothing was deleted; look for the session on {}, start it again if it isn't there, then delete its file",
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
        let (svc, started) = (svc.clone(), started.clone());
        tokio::spawn(async move {
            started.wait().await;
            let state = svc.ask_again(progress, &asked).await;
            started.operations().record(&asked.session, KIND, STEP, state);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asked(session: &str, channel_folder: Option<&str>) -> Asked {
        Asked {
            operation: pb::Operation {
                id: Some(pb::OperationId { value: format!("op_{session}") }),
                ..pb::Operation::default()
            },
            origin: Origin {
                owner: "user".to_owned(),
                scope: None,
            },
            session: session.to_owned(),
            machine: "box-1".to_owned(),
            request: pb::StartDeskSessionRequest {
                title: "Notes".to_owned(),
                ..pb::StartDeskSessionRequest::default()
            },
            channel_folder: channel_folder.map(str::to_owned),
        }
    }

    #[test]
    fn a_kept_start_reads_back_until_it_is_answered() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join(DIR);
        write(&folder, &asked("se_1", None)).unwrap();
        write(&folder, &asked("se_2", Some("payments"))).unwrap();

        let mut found = read_all(&folder);
        found.sort_by(|a, b| a.session.cmp(&b.session));

        assert_eq!(found.len(), 2);
        assert_eq!(found[0].channel_folder, None);
        assert_eq!(found[0].request.title, "Notes");
        assert_eq!(found[1].channel_folder.as_deref(), Some("payments"));
        assert_eq!(found[1].operation.id, asked("se_2", None).operation.id);
        std::fs::remove_file(path_of(&folder, "se_1")).unwrap();
        assert_eq!(read_all(&folder).len(), 1);
    }

    #[test]
    fn an_unreadable_start_is_skipped_and_kept() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join(DIR);
        write(&folder, &asked("se_1", None)).unwrap();
        let broken = path_of(&folder, "se_bad");
        std::fs::write(&broken, b"{ not json").unwrap();

        assert_eq!(read_all(&folder).len(), 1);
        assert!(broken.exists(), "a file that can't be read is left in place");
    }
}

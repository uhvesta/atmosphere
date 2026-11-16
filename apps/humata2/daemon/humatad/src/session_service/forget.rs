//! The sessions' part of Forget machine (LD-11, LD-12, flow 26.23): which
//! sessions were on a machine that is gone and what is lost with each, from
//! the host's last pointers (SL-3), and archiving them from the host's own
//! records. Nothing here dials the machine: the host's records, its last
//! pointers and its mirror are all it has, and all it needs (SL-4).

use std::collections::HashMap;

use humata2_grpc::humata::v1 as pb;
use humata2_store::{ArchiveRow, ArchiveState, SessionDetail, SessionRecord};
use prost::Message;
use tonic::Status;

use super::lineage::{Lineage, publish_family};
use super::{SessionSvc, now_ms};
use crate::channel_service::{blocking, timestamp_ms};
use crate::machine_service::Forgetting;
use crate::remote_work::RemoteView;
use crate::status;

/// A session on the gone machine and what the host last knew of it.
struct Gone {
    record: SessionRecord,
    detail: Option<SessionDetail>,
    view: Option<RemoteView>,
}

impl Gone {
    fn pointers(&self) -> &[pb::Pointer] {
        self.view.as_ref().map_or(&[], |view| view.pointers.as_slice())
    }

    /// The work the session's last pointers say is lost.
    fn work(&self) -> pb::SessionWork {
        let pointers = self.pointers();
        pb::SessionWork {
            session: Some(pb::SessionId {
                value: self.record.id.clone(),
            }),
            title: self.record.title.clone(),
            dirty_files: pointers.iter().map(|pointer| pointer.dirty_files).sum(),
            unpushed_commits: pointers.iter().map(|pointer| pointer.unpushed_commits).sum(),
        }
    }

    /// When the host last heard the pointers: the report's time, else the
    /// newest look a pointer carries.
    fn pointers_at(&self) -> Option<humata2_grpc::Timestamp> {
        self.view
            .as_ref()
            .and_then(|view| view.pointers_at.clone())
            .or_else(|| {
                self.pointers()
                    .iter()
                    .filter_map(|pointer| pointer.reported_at.clone())
                    .max_by_key(|at| (at.seconds, at.nanos))
            })
    }

    fn lost(&self) -> pb::LostWork {
        let work = self.work();
        pb::LostWork {
            session: work.session,
            title: work.title,
            dirty_files: work.dirty_files,
            unpushed_commits: work.unpushed_commits,
            pointers_at: self.pointers_at(),
        }
    }

    /// What the archive keeps of each repo: its branch, the last commit
    /// known to be pushed, and the files that were dirty.
    fn summary(&self, archived_unix_ms: i64) -> pb::ArchiveSummary {
        let repos = self
            .detail
            .iter()
            .flat_map(|detail| detail.repos.iter())
            .map(|repo| {
                let pointer = self.pointers().iter().find(|pointer| {
                    pointer
                        .repo
                        .as_ref()
                        .is_some_and(|named| named.owner == repo.owner && named.name == repo.name)
                });
                pb::archive_summary::Repo {
                    repo: Some(pb::RepoRef {
                        owner: repo.owner.clone(),
                        name: repo.name.clone(),
                    }),
                    branch: pointer
                        .map(|pointer| pointer.branch.clone())
                        .filter(|branch| !branch.is_empty())
                        .unwrap_or_else(|| repo.branch.clone()),
                    last_pushed_sha: pointer
                        .filter(|pointer| pointer.pushed)
                        .map(|pointer| pointer.head_sha.clone())
                        .unwrap_or_default(),
                    files: pointer
                        .iter()
                        .flat_map(|pointer| pointer.dirty_paths.iter())
                        .map(|path| pb::FileChange {
                            path: path.clone(),
                            added: 0,
                            removed: 0,
                        })
                        .collect(),
                    unpushed_subjects: pointer
                        .map(|pointer| pointer.unpushed_subjects.clone())
                        .unwrap_or_default(),
                }
            })
            .collect();
        pb::ArchiveSummary {
            archived_at: Some(timestamp_ms(archived_unix_ms)),
            repos,
            machine_lost: true,
            lost: vec![self.work()],
        }
    }
}

impl SessionSvc {
    /// The sessions recorded on `machine` that aren't archived, with what
    /// the host last heard of each. Reads only.
    async fn gone_on(&self, machine: &str) -> Result<Vec<Gone>, Status> {
        let Some(remote) = self.remote.clone() else {
            return Err(status::internal(
                "only the host forgets machines, and this daemon has no other machines",
                "",
            ));
        };
        let (store, name) = (self.starter.store.clone(), machine.to_owned());
        let (records, details, lineage) = blocking(move || {
            let lineage = Lineage::read(&store)?;
            Ok::<_, humata2_store::StoreError>((
                store
                    .sessions()?
                    .into_iter()
                    .filter(|record| record.machine == name && !lineage.is_archived(&record.id))
                    .collect::<Vec<_>>(),
                store.session_details()?,
                lineage,
            ))
        })
        .await?
        .map_err(|error| status::store(error, &self.machine, "read the sessions on the machine"))?;
        drop(lineage);
        let mut details: HashMap<String, SessionDetail> = details
            .into_iter()
            .map(|detail| (detail.session.clone(), detail))
            .collect();
        let mut views: HashMap<String, RemoteView> = remote
            .views_on(machine)
            .into_iter()
            .map(|view| (view.session.clone(), view))
            .collect();
        let mut gone: Vec<Gone> = records
            .into_iter()
            .map(|record| Gone {
                detail: details.remove(&record.id),
                view: views.remove(&record.id),
                record,
            })
            .collect();
        gone.sort_by(|a, b| (a.record.created_unix_ms, &a.record.id).cmp(&(b.record.created_unix_ms, &b.record.id)));
        Ok(gone)
    }

    /// Takes `machine` out of every channel that has it.
    async fn leave_channels(&self, machine: &str) -> Result<(), Status> {
        let (store, feed, name) = (self.starter.store.clone(), self.tell.feed.clone(), machine.to_owned());
        blocking(move || -> Result<(), humata2_store::StoreError> {
            for mut channel in store.channels()? {
                if !channel.machines.contains(&name) {
                    continue;
                }
                channel.machines.retain(|machine| machine != &name);
                channel.revision += 1;
                store.put_channel(&channel)?;
                crate::channel_service::tell_channel(&feed, &store, &channel.id, pb::Change::Updated, None);
            }
            Ok(())
        })
        .await?
        .map_err(|error| status::store(error, &self.machine, "take the machine out of its channels"))
    }
}

#[tonic::async_trait]
impl Forgetting for SessionSvc {
    async fn lost_on(&self, machine: &str) -> Result<Vec<pb::LostWork>, Status> {
        Ok(self.gone_on(machine).await?.iter().map(Gone::lost).collect())
    }

    async fn archive_lost(&self, machine: &str) -> Result<Vec<pb::SessionId>, Status> {
        let gone = self.gone_on(machine).await?;
        let archived_at = now_ms();
        // The sessions' clones were on the gone machine, so there is nothing
        // here to free: the rows are written as freed and never touch a folder.
        for session in &gone {
            let id = session.record.id.clone();
            let guard = self.lifecycle.lock_of(&id).lock_owned().await;
            let row = ArchiveRow {
                session_id: id,
                archived_unix_ms: archived_at,
                state: ArchiveState::Freed,
                summary: session.summary(archived_at).encode_to_vec(),
                free_after_unix_ms: archived_at,
            };
            let store = self.starter.store.clone();
            blocking(move || store.put_archive(&row))
                .await?
                .map_err(|error| status::store(error, &self.machine, "save that the session was lost"))?;
            drop(guard);
        }
        self.leave_channels(machine).await?;
        let remote = self.remote.clone();
        let mut done = Vec::new();
        for session in &gone {
            let id = &session.record.id;
            // The session no longer waits on anyone (IN-22, TP-15).
            if let Err(error) = self.end_grants_of(id).await {
                crate::log::say!("humatad2: ending lost session {id}'s grants: {}", error.message());
            }
            if let Err(error) = self.withdraw_requests_of(id, pb::WithdrawReason::SessionArchived).await {
                crate::log::say!("humatad2: withdrawing lost session {id}'s requests: {}", error.message());
            }
            if let Some(remote) = &remote {
                remote.forget_session(machine, id).await;
            }
            self.tell.session(id, pb::Change::Updated, None).await;
            publish_family(&self.tell, id, None).await;
            done.push(pb::SessionId { value: id.clone() });
        }
        Ok(done)
    }
}

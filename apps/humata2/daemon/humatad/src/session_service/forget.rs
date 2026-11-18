//! The sessions' part of Forget machine (LD-11, LD-12, flow 26.23): which
//! sessions were on a machine that is gone and what is lost with each, from
//! the host's last pointers (SL-3), and archiving them from the host's own
//! records. Nothing here dials the machine: the host's records, its last
//! pointers and its mirror are all it has, and all it needs (SL-4).

use std::collections::{BTreeSet, HashMap, HashSet};

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

/// How long Remove machine waits for the machine to end each session (D784).
const END_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);

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
            // D783: never relayed to a machine later added under the name.
            machine_removed: true,
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

    async fn take_out_of_channels(&self, machine: &str) -> Result<(), Status> {
        self.leave_channels(machine).await
    }

    async fn let_go_on(&self, machine: &str) -> Result<pb::LetGoOfHostRequest, Status> {
        let (store, name) = (self.starter.store.clone(), machine.to_owned());
        blocking(move || -> Result<pb::LetGoOfHostRequest, humata2_store::StoreError> {
            let mut request = pb::LetGoOfHostRequest::default();
            let mut channels = BTreeSet::new();
            for record in store.sessions()?.into_iter().filter(|record| record.machine == name) {
                let removed = store
                    .archive(&record.id)?
                    .and_then(|row| pb::ArchiveSummary::decode(row.summary.as_slice()).ok())
                    .is_some_and(|summary| summary.machine_removed);
                if !removed {
                    continue;
                }
                if record.channel != humata2_sessions::DESK_CHANNEL
                    && store
                        .channel(&record.channel)?
                        .is_some_and(|channel| !channel.machines.contains(&name))
                {
                    channels.insert(record.channel.clone());
                }
                request.sessions.push(pb::SessionId { value: record.id });
            }
            request.channels = channels.into_iter().map(|value| pb::ChannelId { value }).collect();
            Ok(request)
        })
        .await?
        .map_err(|error| status::store(error, &self.machine, "read which sessions the machine should let go of"))
    }

    async fn end_sessions_on(&self, machine: &str) {
        let Some(remote) = self.remote.clone() else {
            return;
        };
        let gone = match self.gone_on(machine).await {
            Ok(gone) => gone,
            Err(error) => {
                crate::log::say!(
                    "humatad2: {machine} is being removed, but its sessions couldn't be read to end them there: {}. They may still run on {machine}; stop them there before you reuse that machine",
                    error.message()
                );
                return;
            }
        };
        for session in &gone {
            let id = &session.record.id;
            let ended = tokio::time::timeout(END_DEADLINE, remote.stop_session_on(machine, id)).await;
            let failed = match ended {
                Ok(Ok(_)) => continue,
                Ok(Err(error)) => Status::from(error).message().to_owned(),
                Err(_) => format!("{machine} didn't answer within {} seconds", END_DEADLINE.as_secs()),
            };
            crate::log::say!(
                "humatad2: {machine} is being removed, but session {id} couldn't be ended there: {failed}. It may still run on {machine}; stop it there before you reuse that machine. The removal goes on and archives the session here"
            );
        }
    }

    async fn archive_lost(&self, machine: &str, reason: pb::WithdrawReason) -> Result<Vec<pb::SessionId>, Status> {
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
        // D783: the sessions archived before are marked too, so none of
        // them is relayed to a machine later added under the name.
        let marked = self.mark_machine_removed(machine).await?;
        self.drop_follow_ups_of(machine).await;
        self.leave_channels(machine).await?;
        let remote = self.remote.clone();
        let mut done = Vec::new();
        for session in &gone {
            let id = &session.record.id;
            // The session no longer waits on anyone (IN-22, TP-15).
            if let Err(error) = self.end_grants_of(id).await {
                crate::log::say!("humatad2: ending lost session {id}'s grants: {}", error.message());
            }
            if let Err(error) = self.withdraw_requests_of(id, reason).await {
                crate::log::say!("humatad2: withdrawing lost session {id}'s requests: {}", error.message());
            }
            if let Some(remote) = &remote {
                remote.forget_session(machine, id).await;
            }
            self.tell.session(id, pb::Change::Updated, None).await;
            publish_family(&self.tell, id, None).await;
            done.push(pb::SessionId { value: id.clone() });
        }
        // The ones archived before: their archive now says so.
        for id in &marked {
            self.tell.session(id, pb::Change::Updated, None).await;
        }
        let told: HashSet<String> = gone.iter().map(|session| session.record.id.clone()).chain(marked).collect();
        self.tell_channels_of(told).await;
        Ok(done)
    }
}

impl SessionSvc {
    /// Tells each channel that holds one of `sessions`, after the sessions
    /// themselves, so a channel view is told on the Channels feed too, even
    /// one that no longer lists their machine (D783). One that can't be
    /// read is logged: the Sessions feed has told of each session already.
    async fn tell_channels_of(&self, sessions: HashSet<String>) {
        if sessions.is_empty() {
            return;
        }
        let (store, feed) = (self.starter.store.clone(), self.tell.feed.clone());
        let told = blocking(move || -> Result<(), humata2_store::StoreError> {
            let channels: BTreeSet<String> = store
                .sessions()?
                .into_iter()
                .filter(|record| sessions.contains(&record.id))
                .map(|record| record.channel)
                .collect();
            for channel in channels {
                crate::channel_service::tell_channel(&feed, &store, &channel, pb::Change::Updated, None);
            }
            Ok(())
        })
        .await;
        let problem = match told {
            Ok(Ok(())) => return,
            Ok(Err(error)) => error.to_string(),
            Err(status) => status.message().to_owned(),
        };
        crate::log::say!(
            "humatad2: the channels of a removed machine's sessions couldn't be told of them: {problem}. Their sessions show archived; reopen a channel to see it updated"
        );
    }

    /// Marks every archived session that ran on `machine` as its machine
    /// removed (D783), each under its lifecycle lock so an archive step in
    /// flight isn't overwritten. The sessions marked now; one marked before
    /// is left as it is.
    async fn mark_machine_removed(&self, machine: &str) -> Result<Vec<String>, Status> {
        let (store, name) = (self.starter.store.clone(), machine.to_owned());
        let ran_there = blocking(move || {
            store
                .sessions()
                .map(|records| records.into_iter().filter(|record| record.machine == name).map(|record| record.id).collect::<Vec<_>>())
        })
        .await?
        .map_err(|error| status::store(error, &self.machine, "read the sessions on the machine"))?;
        let mut marked = Vec::new();
        for id in ran_there {
            let guard = self.lifecycle.lock_of(&id).lock_owned().await;
            let (store, key) = (self.starter.store.clone(), id.clone());
            let changed = blocking(move || -> Result<bool, humata2_store::StoreError> {
                let Some(mut row) = store.archive(&key)? else {
                    return Ok(false);
                };
                // A summary this daemon can't read is left whole rather
                // than replaced by one that holds the mark alone.
                let Ok(mut summary) = pb::ArchiveSummary::decode(row.summary.as_slice()) else {
                    return Ok(false);
                };
                if summary.machine_removed {
                    return Ok(false);
                }
                summary.machine_removed = true;
                row.summary = summary.encode_to_vec();
                store.put_archive(&row).map(|()| true)
            })
            .await?
            .map_err(|error| status::store(error, &self.machine, "mark the session as its machine removed"))?;
            drop(guard);
            if changed {
                marked.push(id);
            }
        }
        Ok(marked)
    }
}

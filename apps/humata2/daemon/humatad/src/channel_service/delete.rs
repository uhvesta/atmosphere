//! PlanDeleteChannel and DeleteChannel (CH-24, CH-25, NV-27, LD-5 to LD-7,
//! D194): the plan lists every session, board item and pending request of
//! the channel and changes nothing; the delete stops the agents, withdraws
//! the requests, copies the channel's records into a side file in its
//! folder, removes them from the state and sends the folder to the trash.
//! Restoring the trash item brings the channel back with its sessions
//! archived ([`finish_restore`]). Only the user, present, can delete.
//!
//! Every step can run again, so a restart finishes a delete it cut short:
//! the intent is written outside the folder, under `<root>/.deleting`,
//! before the first step and removed after the last.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use humata2_access::{Call, Capability, ChannelId, Refusal, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_sessions::repo_work;
use humata2_store::{AgentState, ChannelRecord, Placement, SIDE_FILE, Store, StoreError};
use humata2_transfers::NewItem;
use serde::{Deserialize, Serialize};
use tonic::{Code, Status};

use super::move_folder::{self, MoveError};
use super::{ChannelSvc, blocking, error_of};
use crate::guard;
use crate::holder_end;
use crate::identity::{self, Caller};
use crate::operations::Started;
use crate::status;
use crate::trash_service::{Bin, caller_data, now_ms};

/// Where unfinished deletes are written, in the channels folder.
const INTENTS: &str = ".deleting";

/// How long a plan's token stays good.
const PLAN_LIFETIME: Duration = Duration::from_secs(10 * 60);

/// What a delete and its plans share.
#[derive(Default)]
pub(super) struct State {
    bin: OnceLock<Arc<Bin>>,
    plans: Mutex<HashMap<String, (String, Listing, Instant)>>,
}

impl State {
    pub(super) fn use_trash(&self, bin: Arc<Bin>) {
        let _ = self.bin.set(bin);
    }

    fn make(&self, channel: &str, listing: &Listing) -> String {
        let token = format!("pl_{}", uuid::Uuid::new_v4().simple());
        let mut plans = self.plans.lock().unwrap_or_else(PoisonError::into_inner);
        plans.retain(|_, (_, _, made)| made.elapsed() < PLAN_LIFETIME);
        plans.insert(
            token.clone(),
            (channel.to_owned(), listing.clone(), Instant::now()),
        );
        token
    }

    /// The listing a live token was made for, left for its one use.
    fn peek(&self, token: &str, channel: &str) -> Option<Listing> {
        let plans = self.plans.lock().unwrap_or_else(PoisonError::into_inner);
        plans
            .get(token)
            .filter(|(of, _, made)| of == channel && made.elapsed() < PLAN_LIFETIME)
            .map(|(_, listing, _)| listing.clone())
    }

    fn take(&self, token: &str) {
        self.plans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(token);
    }
}

/// One repository of a session and the work in its clone.
#[derive(Clone, Debug, PartialEq, Eq)]
struct RepoWorkListed {
    owner: String,
    name: String,
    branch: String,
    dirty_files: u32,
    unpushed_commits: u32,
    never_pushed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SessionListed {
    title: String,
    repos: Vec<RepoWorkListed>,
}

impl SessionListed {
    fn dirty(&self) -> u32 {
        self.repos.iter().map(|repo| repo.dirty_files).sum()
    }

    fn unpushed(&self) -> u32 {
        self.repos.iter().map(|repo| repo.unpushed_commits).sum()
    }

    fn has_work(&self) -> bool {
        self.dirty() > 0 || self.unpushed() > 0
    }
}

/// What a plan listed, and what the delete lists again to compare.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
struct Listing {
    live: BTreeMap<String, SessionListed>,
    /// Sessions of the channel that live on another machine.
    elsewhere: BTreeSet<String>,
    archived: BTreeMap<String, String>,
    items: BTreeSet<String>,
    requests: BTreeSet<String>,
}

impl Listing {
    /// What differs from `before`, in words for the person who planned.
    fn changes_since(&self, before: &Self) -> Vec<String> {
        let mut changes = Vec::new();
        for (id, session) in &self.live {
            match before.live.get(id) {
                None => changes.push(format!("session \"{}\" started or came back", session.title)),
                Some(then) if then != session => changes.push(format!(
                    "session \"{}\" now has {} uncommitted files and {} unpushed commits (it had {} and {})",
                    session.title,
                    session.dirty(),
                    session.unpushed(),
                    then.dirty(),
                    then.unpushed()
                )),
                Some(_) => {}
            }
        }
        for (id, session) in &before.live {
            if !self.live.contains_key(id) {
                changes.push(format!("session \"{}\" is no longer running here", session.title));
            }
        }
        for (id, title) in &self.archived {
            if !before.archived.contains_key(id) {
                changes.push(format!("session \"{title}\" was archived"));
            }
        }
        for key in self.items.difference(&before.items) {
            changes.push(format!("board item {key} was added"));
        }
        for key in before.items.difference(&self.items) {
            changes.push(format!("board item {key} was removed"));
        }
        for id in self.requests.difference(&before.requests) {
            changes.push(format!("request {id} is waiting in the Inbox"));
        }
        for id in before.requests.difference(&self.requests) {
            changes.push(format!("request {id} is no longer pending"));
        }
        changes
    }
}

/// What listing a channel can fail on.
#[derive(Debug, thiserror::Error)]
enum ListError {
    #[error("the store could not be read")]
    Store(#[from] StoreError),
    #[error("git could not read a clone of session {session}")]
    Git {
        session: String,
        #[source]
        source: humata2_sessions::GitError,
    },
}

fn list(store: &Store, channel: &str, machine: &str) -> Result<Listing, ListError> {
    let mut listing = Listing::default();
    for session in store.sessions()?.into_iter().filter(|s| s.channel == channel) {
        if store.archive(&session.id)?.is_some() {
            listing.archived.insert(session.id, session.title);
            continue;
        }
        if !session.machine.is_empty() && session.machine != machine {
            listing.elsewhere.insert(session.id);
            continue;
        }
        let mut repos = Vec::new();
        for repo in store
            .session_detail(&session.id)?
            .into_iter()
            .flat_map(|detail| detail.repos)
        {
            // A clone whose folder is gone was freed and counts nothing.
            if !repo.path.exists() {
                continue;
            }
            // Reading the work fetches, which writes FETCH_HEAD; a plan
            // changes nothing, so a file it made is taken away again.
            let fetched = repo.path.join(".git").join("FETCH_HEAD");
            let had_fetched = fetched.exists();
            let work = repo_work(&repo.path);
            if !had_fetched {
                let _ = fs::remove_file(&fetched);
            }
            let work = work.map_err(|source| ListError::Git {
                session: session.id.clone(),
                source,
            })?;
            repos.push(RepoWorkListed {
                owner: repo.owner,
                name: repo.name,
                branch: work.branch,
                dirty_files: work.dirty_files,
                unpushed_commits: work.unpushed_commits,
                never_pushed: work.never_pushed,
            });
        }
        listing.live.insert(
            session.id,
            SessionListed {
                title: session.title,
                repos,
            },
        );
    }
    listing.items = store
        .board_items_in_channel(channel)?
        .into_iter()
        .map(|item| item.key)
        .collect();
    Ok(listing)
}

/// What stops a delete part way.
#[derive(Debug, thiserror::Error)]
enum DeleteError {
    #[error("the channel's records could not be read or written")]
    Store(#[source] StoreError),
    #[error("session {session}'s agent could not be stopped")]
    Stop {
        session: String,
        #[source]
        source: Status,
    },
    #[error("the requests of the channel's sessions could not be withdrawn")]
    Withdraw(#[source] Status),
    #[error("the channel's Desk sessions could not be closed into the trash")]
    DeskSessions(#[source] Status),
    #[error("the trash could not take the channel's folder {}", folder.display())]
    Trash {
        folder: PathBuf,
        #[source]
        source: Status,
    },
    #[error("the delete's note at {} could not be saved", path.display())]
    Intent {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// What a delete in progress is written down as.
fn intent_path(root: &Path, channel: &str) -> PathBuf {
    root.join(INTENTS).join(channel)
}

/// Makes `folder` and its missing parents, and puts each new folder's entry
/// in its parent on disk, so a power cut can't take the folder back.
fn make_dirs(folder: &Path) -> std::io::Result<()> {
    let missing: Vec<&Path> = folder
        .ancestors()
        .take_while(|found| !found.as_os_str().is_empty() && fs::symlink_metadata(found).is_err())
        .collect();
    humata2_durable::create_dir_all(folder, 0o700)?;
    for found in missing {
        match found.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => humata2_durable::sync_dir(parent)?,
            _ => {}
        }
    }
    Ok(())
}

/// Removes `path` and puts the removal on disk, so a note that finished its
/// delete can't come back after a power cut and delete a folder again.
fn remove_durably(path: &Path) -> std::io::Result<()> {
    humata2_durable::remove_file(path)?;
    match path.parent() {
        Some(parent) => humata2_durable::sync_dir(parent),
        None => Ok(()),
    }
}

impl ChannelSvc {
    /// Only the user, present, deletes a channel: an agent is refused and
    /// nothing is asked of the Inbox (CH-25).
    fn user_deletes(&self, caller: &Caller, channel: &str, present: bool) -> Result<(), Status> {
        if matches!(caller, Caller::Peer) {
            return Err(status::refused(&Refusal::PeerCannotCall));
        }
        self.guard.require(
            caller,
            &Call::Use {
                capability: Capability::ChannelDelete,
                target: Target::Channel(ChannelId::new(channel)),
            },
        )?;
        if present {
            let principal = guard::principal(caller);
            if self
                .guard
                .with_policy(|_, context| principal.present_user(context))
                .is_none()
            {
                return Err(status::refused(&Refusal::UserOnly {
                    capability: Some(Capability::ChannelDelete),
                }));
            }
        }
        Ok(())
    }

    async fn listing(&self, channel: &str) -> Result<Listing, Status> {
        let (store, id, machine) = (
            self.starter.store.clone(),
            channel.to_owned(),
            self.machine.clone(),
        );
        let mut listing = blocking(move || list(&store, &id, &machine))
            .await?
            .map_err(|error| match error {
                ListError::Store(error) => {
                    status::store(error, &self.machine, "list the channel's sessions")
                }
                git @ ListError::Git { .. } => status::failure(
                    Code::Unavailable,
                    pb::Error {
                        message: format!(
                            "could not read a session's uncommitted and unpushed work: {}",
                            status::chain(&git)
                        ),
                        fix: "check that git runs and the session's clones are readable, then plan again; nothing was changed"
                            .to_owned(),
                        retry: None,
                        kind: None,
                    },
                ),
            })?;
        let sessions: Vec<String> = {
            let (store, id) = (self.starter.store.clone(), channel.to_owned());
            blocking(move || store.sessions())
                .await?
                .map_err(|error| status::store(error, &self.machine, "list the channel's sessions"))?
                .into_iter()
                .filter(|session| session.channel == id)
                .map(|session| session.id)
                .collect()
        };
        listing.requests = self.inbox.pending_ids_of(&sessions).await.into_iter().collect();
        Ok(listing)
    }

    pub(super) async fn plan_delete_channel_of(
        &self,
        caller: &Caller,
        request: pb::PlanDeleteChannelRequest,
    ) -> Result<pb::DeleteChannelPlan, Status> {
        let id = request
            .channel
            .map(|id| id.value)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| status::missing("channel"))?;
        self.user_deletes(caller, &id, false)?;
        let channel = self.find(&id).await?;
        let listing = self.listing(&id).await?;
        let work = |id: &String, session: &SessionListed| pb::SessionWork {
            session: Some(pb::SessionId { value: id.clone() }),
            title: session.title.clone(),
            dirty_files: session.dirty(),
            unpushed_commits: session.unpushed(),
        };
        let machine = |id: &String| pb::MachineId { value: id.clone() };
        let others: Vec<&String> = channel.machines.iter().filter(|m| **m != self.machine).collect();
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        let desk_sessions = match self.desk.get().and_then(|handle| handle.get()) {
            Some(desk) => desk.desks_of_channel(&id).await?,
            None => Vec::new(),
        };
        Ok(pb::DeleteChannelPlan {
            channel: Some(pb::ChannelId { value: id.clone() }),
            live_sessions: listing.live.iter().map(|(id, s)| work(id, s)).collect(),
            archived_sessions: count(listing.archived.len()),
            board_items: count(listing.items.len()),
            pending_requests: count(listing.requests.len()),
            token: Some(pb::PlanToken {
                value: self.deletes.make(&id, &listing),
            }),
            archived: listing
                .archived
                .iter()
                .map(|(id, title)| pb::SessionWork {
                    session: Some(pb::SessionId { value: id.clone() }),
                    title: title.clone(),
                    dirty_files: 0,
                    unpushed_commits: 0,
                })
                .collect(),
            board_item_ids: listing
                .items
                .iter()
                .map(|key| pb::BoardItemId { value: key.clone() })
                .collect(),
            pending_request_ids: listing
                .requests
                .iter()
                .map(|id| pb::RequestId { value: id.clone() })
                .collect(),
            machines: vec![machine(&self.machine)],
            // Other machines' folders are not deleted yet (D194): the plan
            // names them so the person knows the delete will say so.
            offline_machines: others.into_iter().map(machine).collect(),
            desk_sessions,
        })
    }

    pub(super) async fn delete_channel_of(
        &self,
        caller: &Caller,
        request: pb::DeleteChannelRequest,
    ) -> Result<pb::DeleteChannelResponse, Status> {
        let id = request
            .channel
            .map(|id| id.value)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| status::missing("channel"))?;
        self.user_deletes(caller, &id, true)?;
        let token = request
            .plan
            .map(|plan| plan.value)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| status::missing("plan"))?;
        let before = self.deletes.peek(&token, &id).ok_or_else(|| {
            stale(
                "the plan to delete the channel is unknown, used or older than ten minutes",
                Vec::new(),
            )
        })?;
        let channel = self.find(&id).await?;
        if channel.machines.iter().any(|machine| *machine != self.machine) {
            return Err(status::not_built(
                "deleting a channel that other machines hold: bring each of them online and remove it from the channel first",
            ));
        }
        let now = self.listing(&id).await?;
        if !now.elsewhere.is_empty() {
            return Err(status::not_built("deleting a channel with live sessions on other machines"));
        }
        let changes = now.changes_since(&before);
        if !changes.is_empty() {
            self.deletes.take(&token);
            return Err(stale(
                "the channel changed since the plan was made; nothing was deleted",
                changes,
            ));
        }
        if !request.delete_unpushed_work
            && let Some((session, listed)) = now.live.iter().find(|(_, listed)| listed.has_work())
        {
            return Err(unpushed(session, listed));
        }
        self.deletes.take(&token);
        let bin = self
            .deletes
            .bin
            .get()
            .cloned()
            .ok_or_else(|| status::internal("this humatad2 was started without a trash", ""))?;

        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Channel(pb::ChannelId { value: id.clone() })),
        };
        let steps = [
            ("stop".to_owned(), "Stopping the agents".to_owned()),
            ("withdraw".to_owned(), "Withdrawing the requests".to_owned()),
            ("snapshot".to_owned(), "Saving the channel's records".to_owned()),
            ("trash".to_owned(), "Moving the folder to the trash".to_owned()),
        ];
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::DeleteChannel,
            target.clone(),
            &steps,
            false,
            request.idempotency_key,
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => {
                return Ok(pb::DeleteChannelResponse {
                    trash_item: None,
                    operation: Some(operation),
                });
            }
        };
        let by = identity::actor(caller);
        let data = caller_data(&by, &target);
        let operation_id = progress.operation().id.map(|id| id.value).unwrap_or_default();
        let run = self.run_delete(channel.clone(), data, bin, Some(&progress)).await;
        match run {
            Ok(item) => {
                let _ = remove_durably(&intent_path(&self.starter.layout.root(), &channel.id));
                progress.succeed(None);
                let feed = self.operations.feed();
                let (record, machine_scope) = (channel.clone(), channel.id.clone());
                let _ = blocking(move || {
                    feed.publish_in(
                        pb::Change::Deleted,
                        pb::event::Payload::Channel(super::channel_pb(&record, 0, &record.default_agent)),
                        crate::events::Scope {
                            channel: machine_scope,
                            session: String::new(),
                        },
                        Some(by),
                    );
                })
                .await;
                let operation = self
                    .operations
                    .progress_of(caller, &operation_id)
                    .ok()
                    .map(|progress| progress.operation());
                Ok(pb::DeleteChannelResponse {
                    trash_item: item.map(|value| pb::TrashItemId { value }),
                    operation,
                })
            }
            Err(error) => {
                let failure = error_of(
                    &error,
                    "fix the cause the message names and delete the channel again; restarting Humata on this machine also finishes the delete. Work in the channel's folder is untouched until its last step",
                    None,
                );
                progress.fail(failure.clone());
                Err(status::failure(Code::Unavailable, failure))
            }
        }
    }

    /// The steps of a delete, each safe to run again. Returns the trash
    /// item's id, or none when the folder was already in the trash.
    async fn run_delete(
        &self,
        channel: ChannelRecord,
        data: String,
        bin: Arc<Bin>,
        progress: Option<&crate::operations::Progress>,
    ) -> Result<Option<String>, DeleteError> {
        let step = |key: &str, state: pb::StepState| {
            if let Some(progress) = progress {
                progress.set_step(key, state, None);
            }
        };
        let root = self.starter.layout.root();
        let folder = self.starter.layout.channel(&channel.folder);
        let note = intent_path(&root, &channel.id);
        {
            let (at, text) = (note.clone(), format!("{}\n{data}\n{}\n", channel.folder, channel.name));
            blocking(move || -> Result<(), std::io::Error> {
                if let Some(parent) = at.parent() {
                    make_dirs(parent)?;
                }
                humata2_store::write_atomic(&at, text.as_bytes()).map_err(std::io::Error::other)
            })
            .await
            .map_err(|source| DeleteError::Stop { session: String::new(), source })?
            .map_err(|source| DeleteError::Intent { path: note.clone(), source })?;
        }

        // A kill here leaves only the note: the restart runs every step.
        crate::fault::point("delete_channel.intent");
        step("stop", pb::StepState::Running);
        let (store, id) = (self.starter.store.clone(), channel.id.clone());
        let sessions: Vec<String> = blocking(move || store.sessions())
            .await
            .map_err(|source| DeleteError::Stop { session: String::new(), source })?
            .map_err(DeleteError::Store)?
            .into_iter()
            .filter(|session| session.channel == id)
            .map(|session| session.id)
            .collect();
        self.stop_all(&sessions).await?;
        crate::fault::point("delete_channel.stopped");
        step("stop", pb::StepState::Done);

        step("withdraw", pb::StepState::Running);
        for session in &sessions {
            self.inbox
                .withdraw_pending_of(session, pb::WithdrawReason::ChannelDeleted)
                .await
                .map_err(DeleteError::Withdraw)?;
        }
        step("withdraw", pb::StepState::Done);
        // CH-24: each Desk session's folder is inside the channel's, so it
        // goes to its own trash item before the channel's folder moves.
        if let Some(desk) = self.desk.get().and_then(|handle| handle.get()) {
            desk.close_for_channel(&channel.id)
                .await
                .map_err(DeleteError::DeskSessions)?;
        }
        crate::fault::point("delete_channel.agents_stopped");

        step("snapshot", pb::StepState::Running);
        let (store, id, side) = (self.starter.store.clone(), channel.id.clone(), folder.join(SIDE_FILE));
        let copying = folder.clone();
        blocking(move || -> Result<(), StoreError> {
            // A folder already in the trash took its side file with it.
            if copying.exists() && store.channel(&id)?.is_some() {
                store.export_channel(&id, &side)?;
            }
            Ok(())
        })
        .await
        .map_err(|source| DeleteError::Stop { session: String::new(), source })?
        .map_err(DeleteError::Store)?;
        // A kill here leaves the side file in the folder: the restart
        // writes it again.
        crate::fault::point("delete_channel.snapshotted");
        step("snapshot", pb::StepState::Done);

        step("trash", pb::StepState::Running);
        let (moving, text) = (folder.clone(), data);
        let item = blocking(move || -> Result<Option<String>, Status> {
            let trash = bin.get()?;
            let item = if moving.exists() {
                let put = trash
                    .put(
                        &moving,
                        NewItem {
                            description: format!("Channel {}", channel.name),
                            caller_data: text,
                            now_unix_ms: now_ms(),
                        },
                    )
                    .map_err(|error| {
                        status::internal(
                            format!("couldn't move the channel's folder to the trash: {}", status::chain(&error)),
                            "",
                        )
                    })?;
                Some(put.id)
            } else {
                None
            };
            Ok(item)
        })
        .await
        .map_err(|source| DeleteError::Trash { folder: folder.clone(), source })?
        .map_err(|source| DeleteError::Trash { folder, source })?;
        // A kill here leaves the folder in the trash and the records: the
        // restart finds no folder to move and forgets the records.
        crate::fault::point("delete_channel.trashed");
        step("trash", pb::StepState::Done);

        // The records go last: a channel still listed with no folder is a
        // delete a restart finishes, and the folder is already safe.
        let (store, id) = (self.starter.store.clone(), channel.id.clone());
        blocking(move || -> Result<(), StoreError> {
            if store.channel(&id)?.is_some() {
                store.forget_channel(&id)?;
            }
            Ok(())
        })
        .await
        .map_err(|source| DeleteError::Stop { session: String::new(), source })?
        .map_err(DeleteError::Store)?;
        // A kill here leaves only the note: the restart finds nothing left
        // to do and removes it. The caller removes the note once it has
        // said how the delete ended, so a restart report's line comes first.
        crate::fault::point("delete_channel.forgotten");
        Ok(item)
    }

    /// Stops every terminal of the sessions, shells and agents, and saves
    /// that their agents stopped. A holder already gone is as stopped as one
    /// that was asked to.
    async fn stop_all(&self, sessions: &[String]) -> Result<(), DeleteError> {
        let store = self.starter.store.clone();
        let wanted: BTreeSet<String> = sessions.iter().cloned().collect();
        let tabs = blocking(move || store.tabs())
            .await
            .map_err(|source| DeleteError::Stop { session: String::new(), source })?
            .map_err(DeleteError::Store)?;
        for tab in tabs
            .into_iter()
            .filter(|tab| tab.session.as_ref().is_some_and(|session| wanted.contains(session)))
        {
            if let Err(error) = holder_end::shut_or_end(
                &tab.holder_socket,
                tab.holder_pid,
                &format!("tab {}", tab.id),
            )
            .await
            {
                crate::log::say!("humatad2: {error}");
            }
        }
        for session in sessions {
            let (store, key) = (self.starter.store.clone(), session.clone());
            let stopped = |source: Status| DeleteError::Stop { session: session.clone(), source };
            let agent = blocking(move || store.agent(&key))
                .await
                .map_err(&stopped)?
                .map_err(DeleteError::Store)?;
            let Some(mut agent) = agent else { continue };
            let AgentState::Running { holder_pid } = agent.state else {
                continue;
            };
            if let Err(error) = holder_end::shut_or_end(
                &agent.launch.holder_socket,
                holder_pid,
                &format!("session {session}'s terminal"),
            )
            .await
            {
                crate::log::say!("humatad2: {error}");
            }
            agent.state = AgentState::Stopped;
            agent.updated_unix_ms = now_ms();
            let store = self.starter.store.clone();
            blocking(move || store.put_agent(&agent))
                .await
                .map_err(&stopped)?
                .map_err(DeleteError::Store)?;
        }
        Ok(())
    }

    /// Takes up the deletes a restart cut short and the restores one cut
    /// short, once the start has brought the sessions back. Never blocks
    /// the start.
    pub(crate) fn recover_deletes_after(
        &self,
        bin: Arc<Bin>,
        started: Arc<crate::restart::Started>,
    ) {
        self.deletes.use_trash(bin.clone());
        note_owed_restores(&self.starter.store, &self.starter.layout.root(), started.operations());
        let me = self.clone();
        // D241: each owed delete is named pending now, before the report.
        let owed = owed_deletes(&self.starter.layout.root(), started.operations());
        tokio::spawn(async move {
            started.wait().await;
            me.recover_deletes(bin, started.operations(), owed).await;
            // A machine that was away for a rename is renamed as it links.
            me.follow_machines().await;
        });
    }

    async fn recover_deletes(
        &self,
        bin: Arc<Bin>,
        operations: &crate::restart::Operations,
        notes: Vec<(PathBuf, String)>,
    ) {
        let root = self.starter.layout.root();
        for (note, text) in notes {
            let mut lines = text.lines();
            let (Some(id), Some(folder), Some(data), Some(name)) =
                (lines.next(), lines.next(), lines.next(), lines.next())
            else {
                continue;
            };
            let (store, key) = (self.starter.store.clone(), id.to_owned());
            let known = blocking(move || store.channel(&key)).await.ok().and_then(Result::ok).flatten();
            let channel = known.unwrap_or_else(|| ChannelRecord {
                id: id.to_owned(),
                name: name.to_owned(),
                folder: folder.to_owned(),
                branch_prefix: None,
                default_agent: String::new(),
                repos: Vec::new(),
                machines: Vec::new(),
                revision: 0,
                created_unix_ms: 0,
            });
            let operation = format!("delete_channel:{id}");
            let name = channel.name.clone();
            let record = |state: crate::restart::OperationState| {
                operations.record(&operation, "delete_channel", "deleting", state);
            };
            match self.run_delete(channel, data.to_owned(), bin.clone(), None).await {
                Ok(_) => record(crate::restart::OperationState::Finished),
                Err(error) => {
                    let reason = format!(
                        "{}; the channel \"{name}\" may be partly deleted, so check its folder and delete the channel again",
                        status::chain(&error)
                    );
                    crate::log::say!("humatad2: couldn't finish deleting channel {id}: {reason}");
                    record(crate::restart::OperationState::Stuck { reason });
                }
            }
            let _ = remove_durably(&note);
        }
        // A rename cut short leaves its note: finish it, and resume the
        // sessions it stopped.
        self.recover_renames().await;
        // A restore cut short leaves a folder with its side file and no
        // channel: bring the channel back from it.
        let (store, root) = (
            Restores { records: self.starter.store.clone(), operations: operations.clone() },
            root,
        );
        let _ = blocking(move || {
            finish_restores(&store, &root);
            finish_restore_notes(&store, &root);
        })
        .await;
    }
}

/// The notes of the deletes the last run left, each as `<id>\n<note text>`
/// with its path, read at once (before anything is served) and each named
/// pending in the restart report, which is written after this (D241). A
/// note that doesn't name a channel is left alone, as before.
fn owed_deletes(root: &Path, operations: &crate::restart::Operations) -> Vec<(PathBuf, String)> {
    let Ok(entries) = fs::read_dir(root.join(INTENTS)) else {
        return Vec::new();
    };
    let notes: Vec<(PathBuf, String)> = entries
        .flatten()
        .filter_map(|entry| {
            let id = entry.file_name().to_string_lossy().into_owned();
            fs::read_to_string(entry.path()).ok().map(|text| (entry.path(), format!("{id}\n{text}")))
        })
        .collect();
    for (_, text) in &notes {
        let mut lines = text.lines();
        if let (Some(id), Some(_), Some(_), Some(_)) = (lines.next(), lines.next(), lines.next(), lines.next()) {
            operations.record(
                &format!("delete_channel:{id}"),
                "delete_channel",
                "deleting",
                crate::restart::OperationState::Pending { waits_for: "the start to finish".to_owned() },
            );
        }
    }
    notes
}

/// A plan that no longer matches (NV-27).
fn stale(message: &str, changes: Vec<String>) -> Status {
    status::failure(
        Code::FailedPrecondition,
        status::error(
            message,
            "plan the delete again, check what it lists now, then delete with the new plan's token",
            pb::error::Kind::PlanStale(pb::PlanStale { changes }),
        ),
    )
}

/// Work that deleting would lose (LD-5): the fix never says to tick the box
/// alone.
fn unpushed(session: &str, listed: &SessionListed) -> Status {
    let repos = listed
        .repos
        .iter()
        .filter(|repo| repo.dirty_files > 0 || repo.unpushed_commits > 0)
        .map(|repo| pb::unpushed_work::Repo {
            repo: Some(pb::RepoRef {
                owner: repo.owner.clone(),
                name: repo.name.clone(),
            }),
            branch: repo.branch.clone(),
            dirty_files: repo.dirty_files,
            unpushed_commits: repo.unpushed_commits,
            never_pushed: repo.never_pushed,
        })
        .collect();
    status::failure(
        Code::FailedPrecondition,
        status::error(
            format!(
                "session \"{}\" has {} uncommitted files and {} unpushed commits that exist only on this machine; nothing was deleted",
                listed.title,
                listed.dirty(),
                listed.unpushed()
            ),
            "commit, push and open a pull request from the session first, then delete the channel. Deleting with \"Delete that work\" ticked loses that work unless it is pushed",
            pb::error::Kind::UnpushedWork(pb::UnpushedWork {
                session: Some(pb::SessionId { value: session.to_owned() }),
                repos,
            }),
        ),
    )
}

/// Where a channel folder that was in the trash goes: its old place, or
/// the first free `<folder>-2`, `-3`, ... when something took it.
pub(crate) fn pick_place(store: &Store, original: &Path) -> Result<PathBuf, StoreError> {
    let taken = store.channels()?;
    let free = |path: &Path| {
        fs::symlink_metadata(path).is_err()
            && !path
                .file_name()
                .is_some_and(|name| taken.iter().any(|channel| channel.folder.as_str() == name))
    };
    if free(original) {
        return Ok(original.to_owned());
    }
    let base = original.as_os_str().to_owned();
    for n in 2u32.. {
        let mut next = base.clone();
        next.push(format!("-{n}"));
        let next = PathBuf::from(next);
        if free(&next) {
            return Ok(next);
        }
    }
    Ok(original.to_owned())
}

/// What bringing a channel back can fail on.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RestoreError {
    #[error("the channel's records in {} could not be brought back", folder.display())]
    Store {
        folder: PathBuf,
        #[source]
        source: StoreError,
    },
    #[error("{} has no name to put the channel's records under", folder.display())]
    NoName { folder: PathBuf },
    #[error("the files in {} still name the folder the channel was deleted from", folder.display())]
    Files {
        folder: PathBuf,
        #[source]
        source: MoveError,
    },
    #[error("couldn't write the note of the restore at {}", path.display())]
    Note {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "a test asked for a failure at {point} (HUMATA2_FAIL_AT); only test daemons read that variable, so start the daemon again without it"
    )]
    Injected { point: &'static str },
}

/// Where restores under another folder are written down, in the channels
/// folder, before their records come back and until their files are done.
const RESTORING: &str = ".restoring";

/// A restore under another folder in progress: what is needed, after its
/// records came back, to finish moving what names the old folder.
#[derive(Debug, Serialize, Deserialize)]
struct RestoreNote {
    channel: String,
    old_path: PathBuf,
    new_path: PathBuf,
}

fn write_restore_note(path: &Path, note: &RestoreNote) -> Result<(), RestoreError> {
    let failed = |source| RestoreError::Note {
        path: path.to_owned(),
        source,
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(failed)?;
    }
    let text = serde_json::to_vec(note).map_err(|error| failed(std::io::Error::other(error)))?;
    humata2_store::write_atomic(path, &text).map_err(|error| failed(std::io::Error::other(error)))
}

/// Moves what names the old folder to the new, for a channel whose records
/// are in place under the new one. The conversations to move are found by
/// asking where the records, put back the way they were, would have them.
fn finish_files(store: &Store, note: &RestoreNote) -> Result<(), RestoreError> {
    // A fault build can be told to fail here (HUMATA2_FAIL_AT); in any other
    // build this is false and compiled away.
    if humata2_store::fault::fails("restore_channel.finish_files") {
        return Err(RestoreError::Injected { point: "restore_channel.finish_files" });
    }
    let forward = Placement {
        name: String::new(),
        folder: String::new(),
        old_path: note.old_path.clone(),
        new_path: note.new_path.clone(),
        now_unix_ms: now_ms(),
    };
    let back = Placement {
        old_path: note.new_path.clone(),
        new_path: note.old_path.clone(),
        ..forward.clone()
    };
    let moves: Vec<(PathBuf, PathBuf)> = store
        .conversation_moves(&note.channel, None, &back)
        .map_err(|source| RestoreError::Store {
            folder: note.new_path.clone(),
            source,
        })?
        .into_iter()
        .map(|(now, was)| (was, now))
        .collect();
    move_folder::rewrite_folders(&forward)
        .and_then(|()| move_folder::carry_conversations(&forward, &moves))
        .map_err(|source| RestoreError::Files {
            folder: note.new_path.clone(),
            source,
        })
}

/// The channel whose folder was put back at `folder` comes back with its
/// sessions archived. Blocking.
pub(crate) fn finish_restore(store: &Store, folder: &Path) -> Result<(), RestoreError> {
    let side = folder.join(SIDE_FILE);
    let failed = |source| RestoreError::Store {
        folder: folder.to_owned(),
        source,
    };
    let head = Store::deleted_channel(&side).map_err(failed)?;
    let (root, name) = match (folder.parent(), folder.file_name()) {
        (Some(root), Some(name)) => (root, name.to_string_lossy().into_owned()),
        _ => {
            return Err(RestoreError::NoName {
                folder: folder.to_owned(),
            });
        }
    };
    // Put back under a `-N` folder: the name takes the same suffix.
    let shown = match name.strip_prefix(&format!("{}-", head.folder)).and_then(|n| n.parse::<u32>().ok()) {
        Some(n) => format!("{}-{n}", head.name),
        None => head.name.clone(),
    };
    let placement = Placement {
        name: shown,
        folder: name,
        old_path: root.join(&head.folder),
        new_path: folder.to_owned(),
        now_unix_ms: now_ms(),
    };
    // Put back under another name, what the folder holds still names the
    // place it was deleted from, and its agents' conversations are kept
    // under that place's keys (MV-16). The note lets a restart finish that
    // when the daemon is cut down after the records came back.
    let moved = placement.old_path != placement.new_path;
    let note = root.join(RESTORING).join(&head.id);
    if moved {
        let written = RestoreNote {
            channel: head.id.clone(),
            old_path: placement.old_path.clone(),
            new_path: placement.new_path.clone(),
        };
        write_restore_note(&note, &written)?;
    }
    // Its sessions come back archived with the undo window an archive
    // gets: their folders are not freed the moment they are back.
    let free_after = placement
        .now_unix_ms
        .saturating_add(crate::session_service::archive_undo_window_ms());
    // A fault build can be told to fail the import here (HUMATA2_FAIL_AT); in
    // any other build this is false and compiled away.
    if humata2_store::fault::fails("restore_channel.import_record") {
        return Err(RestoreError::Injected { point: "restore_channel.import_record" });
    }
    let moves = store.import_channel(&side, &placement, free_after).map_err(failed)?;
    // A kill here leaves the records back and the files not: the restart
    // finishes them from the note.
    crate::fault::point("restore_channel.imported");
    if moved {
        move_folder::rewrite_folders(&placement)
            .and_then(|()| move_folder::carry_conversations(&placement, &moves))
            .map_err(|source| RestoreError::Files {
                folder: folder.to_owned(),
                source,
            })?;
        let _ = fs::remove_file(&note);
    }
    Ok(())
}

/// Finishes the restores a restart cut short after their records came back:
/// the files and conversations that still name the old folder.
fn finish_restore_notes(restores: &Restores, root: &Path) {
    let Restores { records: store, operations } = restores;
    let Ok(entries) = fs::read_dir(root.join(RESTORING)) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let read = fs::read(&path).ok().and_then(|bytes| serde_json::from_slice::<RestoreNote>(&bytes).ok());
        let Some(note) = read else {
            crate::log::say!(
                "humatad2: {} is not a note of a restore in progress; remove it if no restore is waiting",
                path.display()
            );
            continue;
        };
        match store.channel(&note.channel) {
            Ok(Some(_)) => match finish_files(store, &note) {
                Ok(()) => {
                    // The end is named before the note goes (D241).
                    operations.record(
                        &restore_operation(&note.channel),
                        "restore_channel",
                        "moving its files",
                        crate::restart::OperationState::Finished,
                    );
                    let _ = fs::remove_file(&path);
                }
                Err(error) => {
                    let reason = format!(
                        "{}; check that the channel's folder can be written to, then restart humatad2 to try again",
                        status::chain(&error)
                    );
                    crate::log::say!("humatad2: couldn't finish bringing back channel {}: {reason}", note.channel);
                    operations.record(
                        &restore_operation(&note.channel),
                        "restore_channel",
                        "moving its files",
                        crate::restart::OperationState::Stuck { reason },
                    );
                }
            },
            // Its records never came back, and no side file is left to bring
            // them from: nothing is waiting on the note.
            Ok(None) => {
                let _ = fs::remove_file(&path);
            }
            Err(_) => {}
        }
    }
}

/// What the restart report calls the restore of channel `id`.
fn restore_operation(id: &str) -> String {
    format!("restore_channel:{id}")
}

/// Names pending, in the restart report, each restore the last run left to
/// finish: a folder with a side file whose channel is not in the state, and
/// a note of a restore whose records came back but whose files didn't
/// move. Read at once, before anything is served, because the report is
/// written after this (D241).
fn note_owed_restores(store: &Store, root: &Path, operations: &crate::restart::Operations) {
    let mut owed = BTreeSet::new();
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let side = entry.path().join(SIDE_FILE);
            if entry.file_name().to_string_lossy().starts_with('.') || !side.exists() {
                continue;
            }
            if let Ok(head) = Store::deleted_channel(&side)
                && matches!(store.channel(&head.id), Ok(None))
            {
                owed.insert(head.id);
            }
        }
    }
    if let Ok(entries) = fs::read_dir(root.join(RESTORING)) {
        for entry in entries.flatten() {
            let read = fs::read(entry.path()).ok().and_then(|bytes| serde_json::from_slice::<RestoreNote>(&bytes).ok());
            if let Some(note) = read
                && matches!(store.channel(&note.channel), Ok(Some(_)))
            {
                owed.insert(note.channel);
            }
        }
    }
    for id in owed {
        operations.record(
            &restore_operation(&id),
            "restore_channel",
            "bringing the channel back",
            crate::restart::OperationState::Pending { waits_for: "the start to finish".to_owned() },
        );
    }
}

/// What finishing the restores a restart cut short needs: the records, and
/// the restart report each one's end is named in (D241).
struct Restores {
    records: Arc<Store>,
    operations: crate::restart::Operations,
}

/// Finishes the restores a restart cut short: a folder with a side file
/// whose channel is not in the state. Each is named in the restart report
/// as it ends (D241).
fn finish_restores(restores: &Restores, root: &Path) {
    let Restores { records: store, operations } = restores;
    let Ok(entries) = fs::read_dir(root) else { return };
    for entry in entries.flatten() {
        let folder = entry.path();
        if entry.file_name().to_string_lossy().starts_with('.') || !folder.join(SIDE_FILE).exists() {
            continue;
        }
        let Ok(head) = Store::deleted_channel(&folder.join(SIDE_FILE)) else { continue };
        match store.channel(&head.id) {
            Ok(None) => {
                let operation = restore_operation(&head.id);
                match finish_restore(store, &folder) {
                    Ok(()) => operations.record(
                        &operation,
                        "restore_channel",
                        "bringing the channel back",
                        crate::restart::OperationState::Finished,
                    ),
                    Err(error) => {
                        let reason = format!(
                            "{}; check the channel's folder and the data folder's disk, then restart humatad2 to try again",
                            status::chain(&error)
                        );
                        crate::log::say!("humatad2: couldn't bring back a channel: {reason}");
                        operations.record(
                            &operation,
                            "restore_channel",
                            "bringing the channel back",
                            crate::restart::OperationState::Stuck { reason },
                        );
                    }
                }
            }
            // Imported already; only the used-up file is left.
            Ok(Some(_)) => {
                let _ = fs::remove_file(folder.join(SIDE_FILE));
            }
            Err(_) => {}
        }
    }
}

#[cfg(test)]
mod restore_owed_tests {
    use super::*;
    use crate::restart::{Operations, pending_in};

    fn record(id: &str, folder: &str) -> ChannelRecord {
        ChannelRecord {
            id: id.to_owned(),
            name: "Payments".to_owned(),
            folder: folder.to_owned(),
            branch_prefix: None,
            default_agent: String::new(),
            repos: Vec::new(),
            machines: Vec::new(),
            revision: 1,
            created_unix_ms: 0,
        }
    }

    /// The last state the report names for `id`.
    fn last_state(report: &Path, id: &str) -> Option<String> {
        let text = fs::read_to_string(report.join("operations.jsonl")).ok()?;
        text.lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter(|value| value["id"] == id)
            .filter_map(|value| value["state"].as_str().map(str::to_owned))
            .next_back()
    }

    /// A store whose channel `ch_1` was deleted: its side file is in the
    /// channel's folder under `root` and the state no longer has it.
    fn deleted(state: &Path, root: &Path) -> Store {
        let store = Store::open(state.join("store")).unwrap();
        store.put_channel(&record("ch_1", "payments")).unwrap();
        let folder = root.join("payments");
        fs::create_dir_all(&folder).unwrap();
        store.export_channel("ch_1", &folder.join(SIDE_FILE)).unwrap();
        store.forget_channel("ch_1").unwrap();
        store
    }

    /// A restore the last run left is named pending when it is read, and
    /// finished once the channel is back.
    #[test]
    fn a_restore_the_last_run_left_is_named_pending_then_finished() {
        let (state, root, report) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let store = Arc::new(deleted(state.path(), root.path()));
        let operations = Operations::default();
        operations.set_folder(report.path());
        let restores = Restores { records: store.clone(), operations: operations.clone() };

        note_owed_restores(&store, root.path(), &operations);
        assert_eq!(pending_in(report.path()), vec!["restore_channel:ch_1".to_owned()]);
        assert!(store.channel("ch_1").unwrap().is_none(), "nothing is restored by noting it");

        finish_restores(&restores, root.path());
        assert!(store.channel("ch_1").unwrap().is_some());
        assert_eq!(last_state(report.path(), "restore_channel:ch_1").as_deref(), Some("finished"));
    }

    /// A side file whose channel is back already is only left over: nothing
    /// is owed, nothing is named, and the file goes.
    #[test]
    fn a_leftover_side_file_is_not_a_restore_owed() {
        let (state, root, report) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let store = Arc::new(deleted(state.path(), root.path()));
        store.put_channel(&record("ch_1", "payments")).unwrap();
        let operations = Operations::default();
        operations.set_folder(report.path());
        let restores = Restores { records: store.clone(), operations: operations.clone() };

        note_owed_restores(&store, root.path(), &operations);
        finish_restores(&restores, root.path());

        assert!(pending_in(report.path()).is_empty());
        assert_eq!(last_state(report.path(), "restore_channel:ch_1"), None);
        assert!(!root.path().join("payments").join(SIDE_FILE).exists());
    }

    /// A note of a restore whose records are back, but whose files are not
    /// moved, is owed too.
    #[test]
    fn a_restore_note_of_a_channel_that_is_back_is_named_pending() {
        let (state, root, report) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let store = Store::open(state.path().join("store")).unwrap();
        store.put_channel(&record("ch_1", "payments-2")).unwrap();
        let note = RestoreNote {
            channel: "ch_1".to_owned(),
            old_path: root.path().join("payments"),
            new_path: root.path().join("payments-2"),
        };
        write_restore_note(&root.path().join(RESTORING).join("ch_1"), &note).unwrap();
        let operations = Operations::default();
        operations.set_folder(report.path());

        note_owed_restores(&store, root.path(), &operations);

        assert_eq!(pending_in(report.path()), vec!["restore_channel:ch_1".to_owned()]);
    }
}

#[cfg(test)]
mod owed_tests {
    use super::*;

    /// A delete the last run left is named pending when its note is read,
    /// and a note that names no channel is not.
    #[test]
    fn a_delete_the_last_run_left_is_named_pending_when_read() {
        let (root, report) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        fs::create_dir_all(root.path().join(INTENTS)).unwrap();
        fs::write(root.path().join(INTENTS).join("ch_1"), "payments\n/data/payments\nPayments\n").unwrap();
        fs::write(root.path().join(INTENTS).join("ch_2"), "payments\n").unwrap();
        let operations = crate::restart::Operations::default();
        operations.set_folder(report.path());

        let owed = owed_deletes(root.path(), &operations);

        assert_eq!(owed.len(), 2, "both notes are kept for the recovery");
        assert_eq!(crate::restart::pending_in(report.path()), vec!["delete_channel:ch_1".to_owned()]);
    }
}

//! Closing a channel's Desk session (CD-4, CD-5, CH-24, D306): unlike a
//! plain Desk session, whose closed folder waits 7 days for the sweep, a
//! closed channel Desk session's folder goes to this machine's trash at
//! once, as one item that names the session. Restoring that item reopens
//! the session. The watch closes a session on its own when its target or
//! its channel is gone, or stops it and marks its target gone when its
//! folder holds Markdown the user may want. Deleting a channel closes each
//! of its Desk sessions into its own trash item before the channel's
//! folder moves.
//!
//! The trash and channel services reach the Desk service through a
//! [`DeskHandle`], made in serve.rs before either and set once the Desk
//! service exists.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use humata2_grpc::humata::v1 as pb;
use humata2_sessions::scratch_child;
use humata2_store::{AgentState, ChannelDeskRow, Placement, SessionRecord, Store, StoreError, TabKind, TabRecord};
use humata2_transfers::NewItem;
use tokio::sync::broadcast::error::RecvError;
use tonic::Status;

use super::{DeskSvc, desk_status, failed_precondition, now_ms};
use crate::ports::{blocking, carry_conversations};
use crate::ports::Feed;
use humata2_daemon_ports::Caller;
use crate::ports as identity;
use crate::ports::Started;
use crate::ports as status;
use crate::ports::caller_data;

/// The Desk service as the trash and channel services see it: empty until
/// `desk_service::server` sets it, which serve.rs does before it serves.
#[derive(Clone)]
pub struct DeskHandle {
    svc: Arc<OnceLock<DeskSvc>>,
    /// The event feed the watch follows (CD-5).
    feed: Arc<Feed>,
}

impl DeskHandle {
    pub fn new(feed: Arc<Feed>) -> Self {
        Self {
            svc: Arc::default(),
            feed,
        }
    }

    /// Sets the Desk service. Once; a second call is ignored.
    pub(super) fn set(&self, svc: DeskSvc) {
        let _ = self.svc.set(svc);
    }

    /// The Desk service, once it is set.
    pub fn get(&self) -> Option<&DeskSvc> {
        self.svc.get()
    }

    pub(super) fn feed(&self) -> Arc<Feed> {
        self.feed.clone()
    }
}

impl DeskSvc {
    /// CloseDeskSession on a channel Desk session (CD-4). `Ok(None)` when
    /// `request.session` names no channel Desk session, so the plain Close
    /// runs instead. Otherwise, holding the session's lock:
    ///
    /// - Already closed: answers the session as it is (a retry is no new
    ///   trash item).
    /// - `stop_agent`, then `stop_shells` (close.rs).
    /// - Moves the folder to `self.bin` as one item, after checking with
    ///   `humata2_sessions::scratch_child` that it is a direct child of
    ///   `<channel folder>/desk` and not a symlink. The item's description
    ///   names the session's title; its `caller_data` is
    ///   `trash_service::caller_data(actor of caller, was)` with `was` the
    ///   `EntityRef` of the session, so the Trash shows and restores it as
    ///   the session. A folder already gone is no error: no item.
    /// - Saves the row with `closed_unix_ms` and `last_used_unix_ms` now and
    ///   `trash_item` the item's id, then `end_grants` and tells the session
    ///   Updated.
    ///
    /// Answers `channel_desk::channel_desk_pb` of the closed session. A
    /// failure to move the folder leaves the session open but stopped, and
    /// says how to free the trash's folder.
    pub(super) async fn close_channel_desk(
        &self,
        caller: &Caller,
        request: &pb::CloseDeskSessionRequest,
    ) -> Result<Option<pb::DeskSession>, Status> {
        let Some(id) = request.session.as_ref().map(|id| id.value.clone()) else {
            return Ok(None);
        };
        if self.channel_desk_row(&id).await?.is_none() {
            return Ok(None);
        }
        let _held = self.locks.hold(&id).await;
        let (session, row) = self.load_channel_desk(&id).await?;
        let row = if row.closed_unix_ms.is_some() {
            row
        } else {
            // A plain close keeps a folder that holds a version; the caller says when it should go too.
            self.close_locked(Some(caller), &session, &row, !request.trash_versioned).await?
        };
        Ok(Some(self.channel_desk_view(&session, &row)))
    }

    /// ReopenDeskSession on a channel Desk session closed in place (DV-13,
    /// D452). `Ok(None)` when `request.session` names no channel Desk
    /// session, so the plain Reopen runs instead. Otherwise, holding the
    /// session's lock, as Reopen does for a plain one: its folder must still
    /// be there and not in the trash (a trashed one is restored from the
    /// Trash), and its channel and target must still be there; then the row
    /// is saved open and the agent resumes in the same folder.
    pub(super) async fn reopen_channel_desk(
        &self,
        caller: &Caller,
        request: &pb::ReopenDeskSessionRequest,
    ) -> Result<Option<pb::Operation>, Status> {
        let Some(id) = request.session.as_ref().map(|id| id.value.clone()) else {
            return Ok(None);
        };
        if self.channel_desk_row(&id).await?.is_none() {
            return Ok(None);
        }
        let _held = self.locks.hold(&id).await;
        let (session, mut row) = self.load_channel_desk(&id).await?;
        if let Some(operation) = self
            .operations
            .existing(caller, request.idempotency_key.as_ref())?
        {
            return Ok(Some(operation));
        }
        if row.closed_unix_ms.is_none() {
            return Err(failed_precondition(
                format!("Desk session {id} is open, so there is nothing to reopen"),
                "open its terminal from the channel's Desk tab",
            ));
        }
        if row.trash_item.is_some() {
            return Err(failed_precondition(
                format!("Desk session {id}'s folder is in the trash, so it can't be reopened from the Desk"),
                "restore the Desk session from the Trash",
            ));
        }
        if let Some(machine) = self.elsewhere(&session) {
            return Err(failed_precondition(
                format!("Desk session {id} runs on {machine}, so it can't be reopened from here"),
                format!("reopen it from {machine}"),
            ));
        }
        if !session.folder.is_dir() {
            return Err(failed_precondition(
                format!(
                    "the folder {} of Desk session {id} is gone, so the session can't be reopened",
                    session.folder.display()
                ),
                "put the folder back where it was; a Desk session needs its folder to run, so it stays closed until then",
            ));
        }
        if self.remote.is_some() {
            let (store, channel, target) = (
                self.starter.store.clone(),
                row.channel.clone(),
                row.target_session.clone(),
            );
            let (channel_there, target_gone) = blocking(move || -> Result<_, StoreError> {
                let gone = match &target {
                    Some(target) => target_is_gone(&store, target, &channel, now_ms())?,
                    None => false,
                };
                Ok((store.channel(&channel)?.is_some(), gone))
            })
            .await?
            .map_err(|error| crate::ports::store_error(error, &self.machine, "check the Desk session's channel and target"))?;
            if !channel_there {
                return Err(failed_precondition(
                    format!("Desk session {id} belongs to channel {}, which is gone, so it can't be reopened", row.channel),
                    "restore the channel from the trash first, then reopen the Desk session",
                ));
            }
            if target_gone || row.target_gone {
                return Err(failed_precondition(
                    format!("Desk session {id} looked after a session that is deleted or archived for good, so it can't be reopened"),
                    "copy what you need from its folder instead; a Desk session can't come back without the session it looks after",
                ));
            }
        }
        let steps: Vec<(String, String)> = vec![
            ("launch".to_owned(), "Starting the agent again".to_owned()),
            (
                "verify".to_owned(),
                "Checking it took up its conversation".to_owned(),
            ),
        ];
        let entity = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Session(pb::SessionId {
                value: id.clone(),
            })),
        };
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::StartSession,
            entity,
            &steps,
            false,
            request.idempotency_key.clone(),
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(Some(operation)),
        };
        let operation = progress.operation();
        row.closed_unix_ms = None;
        row.last_used_unix_ms = now_ms();
        let saved = self.save_channel_desk_row(&row).await;
        self.resume_saved(caller, &id, saved, progress).await?;
        self.tell
            .session(&id, pb::Change::Updated, Some(identity::actor(caller)))
            .await;
        Ok(Some(operation))
    }

    /// DeleteChannel's step before the channel's folder moves (CH-24): each
    /// open Desk session of `channel` is closed as Close does, by the
    /// system, each into its own trash item. Answers those items' ids. A
    /// channel with none answers an empty list and does nothing. Safe to run
    /// again: a closed one is skipped.
    pub async fn close_for_channel(&self, channel: &str) -> Result<Vec<String>, Status> {
        let open = self.open_desks_of(channel).await?;
        if open.is_empty() {
            return Ok(Vec::new());
        }
        let mut items = Vec::new();
        for row in open {
            let _held = self.locks.hold(&row.session_id).await;
            // Read again under the lock: a close that ran meanwhile is done.
            let (session, row) = self.load_channel_desk(&row.session_id).await?;
            if row.closed_unix_ms.is_some() {
                continue;
            }
            let closed = self.close_locked(None, &session, &row, false).await?;
            items.extend(closed.trash_item);
        }
        Ok(items)
    }

    /// The open Desk sessions of `channel`, for the delete's plan to list
    /// (CH-24), oldest id first.
    pub async fn desks_of_channel(&self, channel: &str) -> Result<Vec<pb::SessionId>, Status> {
        Ok(self
            .open_desks_of(channel)
            .await?
            .into_iter()
            .map(|row| pb::SessionId { value: row.session_id })
            .collect())
    }

    /// The channel of the closed channel Desk session whose folder is trash
    /// item `item`, if it is one's: whose rule judges its restore.
    pub async fn closed_channel_of(&self, item: &str) -> Result<Option<String>, Status> {
        Ok(self.closed_into(item).await?.map(|row| row.channel))
    }

    /// RestoreFromTrash's check before it moves anything (CD-4, CD-5).
    /// `Ok(())` when trash item `item` is no channel Desk session's. For
    /// one, FailedPrecondition while its channel is gone ("restore the
    /// channel first") or while its target is gone (deleted, archived past
    /// its undo window, or `target_gone`): "a Desk session can't come back
    /// without the session it looks after; copy what you need from the
    /// trash instead". The item stays in the trash either way.
    pub async fn before_restore(&self, item: &str) -> Result<(), Status> {
        let Some(row) = self.closed_into(item).await? else {
            return Ok(());
        };
        // SL-1, MV-17: on a machine's own daemon the host judged the channel
        // and the target before it relayed the restore; this machine may
        // hold neither.
        if self.remote.is_none() {
            return Ok(());
        }
        let (store, channel, target) = (
            self.starter.store.clone(),
            row.channel.clone(),
            row.target_session.clone(),
        );
        let (channel_there, target_gone) = blocking(move || -> Result<_, StoreError> {
            let gone = match &target {
                Some(target) => target_is_gone(&store, target, &channel, now_ms())?,
                None => false,
            };
            Ok((store.channel(&channel)?.is_some(), gone))
        })
        .await?
        .map_err(|error| crate::ports::store_error(error, &self.machine, "check the Desk session's channel and target"))?;
        let id = &row.session_id;
        if !channel_there {
            return Err(failed_precondition(
                format!("Desk session {id} belongs to channel {}, which is gone, so it can't be restored yet; it is still in the trash", row.channel),
                "restore the channel from the trash first, then restore the Desk session",
            ));
        }
        if target_gone || row.target_gone {
            return Err(failed_precondition(
                format!("Desk session {id} looked after a session that is deleted or archived for good, so it can't come back; it is still in the trash"),
                "copy what you need from the trash instead; a Desk session can't come back without the session it looks after",
            ));
        }
        Ok(())
    }

    /// RestoreFromTrash's step once trash item `item` is at `placed`. Does
    /// nothing when it is no channel Desk session's. For one, holding the
    /// session's lock: clears `closed_unix_ms` and `trash_item`, moves
    /// `last_used_unix_ms`, saves `placed` as the session's folder when the
    /// trash put it elsewhere, points its agent's launch and conversation at
    /// `placed` when they name another folder, then resumes its agent as
    /// Reopen does
    /// (close.rs's resume: an operation with the launch and verify steps,
    /// then `run_resume`) and tells the session Updated. A failure after
    /// the folder is back leaves it there and says to reopen it.
    pub async fn after_restore(
        &self,
        caller: &Caller,
        item: &str,
        placed: &Path,
    ) -> Result<(), Status> {
        let Some(found) = self.closed_into(item).await? else {
            return Ok(());
        };
        let id = found.session_id;
        let _held = self.locks.hold(&id).await;
        let (mut session, mut row) = self.load_channel_desk(&id).await?;
        if row.trash_item.as_deref() != Some(item) {
            // Another restore of this item got here first.
            return Ok(());
        }
        let steps: Vec<(String, String)> = vec![
            ("launch".to_owned(), "Starting the agent again".to_owned()),
            (
                "verify".to_owned(),
                "Checking it took up its conversation".to_owned(),
            ),
        ];
        let entity = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Session(pb::SessionId {
                value: id.clone(),
            })),
        };
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::StartSession,
            entity,
            &steps,
            false,
            None,
        )? {
            Started::New(progress) => progress,
            Started::Existing(_) => return Ok(()),
        };
        row.closed_unix_ms = None;
        row.trash_item = None;
        row.last_used_unix_ms = now_ms();
        let moved = session.folder.as_path() != placed;
        session.folder = placed.to_path_buf();
        let saved = match self.refit_agent(&session, placed).await {
            Ok(()) => self.save_restored(&session, &row, moved).await,
            Err(error) => Err(error),
        };
        self.resume_saved(caller, &id, saved, progress)
            .await
            .map_err(|error| {
                failed_precondition(
                    format!(
                        "Desk session {id}'s folder is back at {}, but its agent could not be resumed: {}",
                        placed.display(),
                        error.message()
                    ),
                    "the folder is safe where it is; check humatad2's log for the cause, then start a new Desk session for the same target to carry on",
                )
            })?;
        self.tell
            .session(&id, pb::Change::Updated, Some(identity::actor(caller)))
            .await;
        Ok(())
    }

    /// Where trash item `item` is put back when it is a channel Desk
    /// session's (MV-16): its channel's Desk folder as the channel stands
    /// now, under the folder's own name. `None` for any other item.
    pub async fn restore_place(&self, item: &str) -> Result<Option<PathBuf>, Status> {
        let Some(row) = self.closed_into(item).await? else {
            return Ok(None);
        };
        let (store, layout) = (self.starter.store.clone(), self.starter.layout.clone());
        blocking(move || -> Result<Option<PathBuf>, StoreError> {
            let (Some(session), Some(channel)) = (store.session(&row.session_id)?, store.channel(&row.channel)?) else {
                return Ok(None);
            };
            Ok(session
                .folder
                .file_name()
                .map(|name| layout.channel(&channel.folder).join("desk").join(name)))
        })
        .await?
        .map_err(|error| crate::ports::store_error(error, &self.machine, "find where the Desk session's folder goes"))
    }

    /// Points the session's agent at `placed` when its launch names another
    /// folder (its channel moved while it sat in the trash, or came back
    /// elsewhere), as `Starter` fills a launch in for a folder, and moves
    /// the conversation to the key `placed` has.
    async fn refit_agent(&self, session: &SessionRecord, placed: &Path) -> Result<(), Status> {
        let (store, id) = (self.starter.store.clone(), session.id.clone());
        let found = blocking(move || store.agent(&id))
            .await?
            .map_err(|error| crate::ports::store_error(error, &self.machine, "read the Desk session's agent"))?;
        let Some(mut agent) = found else {
            return Ok(());
        };
        if agent.launch.folder == placed {
            return Ok(());
        }
        let kind = self.starter.agents.get(&session.agent).map_err(|error| {
            failed_precondition(
                format!(
                    "Desk session {}'s agent can't be started in {}: {}",
                    session.id,
                    placed.display(),
                    status::chain(&error)
                ),
                "set the agent up again in Settings, then reopen the session",
            )
        })?;
        let old = agent.launch.clone();
        let plan = kind.launch(&self.starter.home, placed, &old.conversation_id);
        agent.launch.folder = placed.to_path_buf();
        agent.launch.argv = if old.argv == old.resume_argv {
            plan.resume_argv.clone()
        } else {
            plan.argv
        };
        agent.launch.resume_argv = plan.resume_argv;
        agent.launch.conversation_file = plan.conversation_file;
        let placement = Placement {
            name: String::new(),
            folder: String::new(),
            old_path: old.folder,
            new_path: placed.to_path_buf(),
            now_unix_ms: now_ms(),
        };
        let moves = vec![(old.conversation_file, agent.launch.conversation_file.clone())];
        let (store, id, machine) = (self.starter.store.clone(), session.id.clone(), self.machine.clone());
        blocking(move || -> Result<(), Status> {
            carry_conversations(&placement, &moves).map_err(|error| {
                failed_precondition(
                    format!(
                        "Desk session {id}'s conversation could not be moved to its new folder: {}",
                        status::chain(&error)
                    ),
                    "fix the cause the message names, then restore the Desk session again",
                )
            })?;
            store
                .put_agent(&agent)
                .map_err(|error| crate::ports::store_error(error, &machine, "save the Desk session's agent"))
        })
        .await?
    }

    /// Saves the reopened row, and the session's record when the trash put
    /// its folder somewhere else.
    async fn save_restored(
        &self,
        session: &SessionRecord,
        row: &ChannelDeskRow,
        folder_moved: bool,
    ) -> Result<(), Status> {
        if folder_moved {
            let (store, session) = (self.starter.store.clone(), session.clone());
            blocking(move || store.put_session(&session))
                .await?
                .map_err(|error| {
                    crate::ports::store_error(error, &self.machine, "save the restored Desk session's folder")
                })?;
        }
        self.save_channel_desk_row(row).await
    }

    /// Closes a session whose lock the caller holds (CD-4): stops its agent
    /// and shells, sends its folder to the trash as one item, saves the row
    /// closed with that item, ends its grants and tells it Updated. `by` is
    /// the user or agent that asked, or `None` when the system closes it.
    /// With `keep_versions`, a folder that holds a version (DV-4) is not
    /// trashed but left in place (D452): the row is closed with no item.
    /// Deleting the channel passes false, as the folder's lifetime ends with it.
    /// Answers the row as it is saved.
    async fn close_locked(
        &self,
        by: Option<&Caller>,
        session: &SessionRecord,
        row: &ChannelDeskRow,
        keep_versions: bool,
    ) -> Result<ChannelDeskRow, Status> {
        // A session on another machine is closed there: its folder and its
        // Bin are that machine's (MV-17).
        if let Some(machine) = self.elsewhere(session) {
            return self.close_channel_desk_there(by, &machine, session, row, keep_versions).await;
        }
        let id = row.session_id.as_str();
        self.stop_agent(id).await?;
        self.stop_shells(id).await?;
        let item = if keep_versions && self.holds_version(&session.folder).await {
            None
        } else {
            self.move_to_trash(by, session, row).await?
        };
        let now = now_ms();
        let mut closed = row.clone();
        closed.closed_unix_ms = Some(now);
        closed.last_used_unix_ms = now;
        closed.trash_item = item;
        self.save_channel_desk_row(&closed).await?;
        self.end_grants(id).await?;
        self.tell
            .session(id, pb::Change::Updated, by.map(identity::actor))
            .await;
        Ok(closed)
    }

    /// D801 I: detaches `session`'s running terminals instead of ending
    /// them, for the watch's automatic closes. Each running holder (the
    /// agent's and every shell's) keeps running, its tab no longer names the
    /// session, so the running-holders listing shows it under "Not in a
    /// channel" for the user to attach, adopt or end (D784). The agent is
    /// saved as stopped, so nothing resumes a second copy of it. A session
    /// on another machine, and one with no running terminal, are left to
    /// the caller. An agent still starting is no error here: it is left as
    /// it is, and `close_locked` refuses as before.
    async fn detach_live(&self, session: &SessionRecord) -> Result<(), Status> {
        if self.elsewhere(session).is_some() {
            return Ok(());
        }
        let (store, id) = (self.starter.store.clone(), session.id.clone());
        let detached = blocking(move || -> Result<Vec<String>, StoreError> {
            let mut forgotten = Vec::new();
            let tabs = store.tabs()?;
            let agent = store.agent(&id)?;
            if let Some(mut agent) = agent
                && let AgentState::Running { holder_pid } = agent.state
            {
                let mut tab = match store.tab(&agent.tab)? {
                    Some(tab) => tab,
                    None => TabRecord {
                        id: agent.tab.clone(),
                        session: None,
                        kind: TabKind::Shell,
                        title: agent.launch.agent.clone(),
                        holder_socket: agent.launch.holder_socket.clone(),
                        holder_pid,
                        created_unix_ms: agent.updated_unix_ms,
                        holder_generation: 0,
                    },
                };
                tab.session = None;
                tab.kind = TabKind::Shell;
                store.put_tab(&tab)?;
                agent.state = AgentState::Stopped;
                agent.updated_unix_ms = now_ms();
                store.put_agent(&agent)?;
                forgotten.push(tab.id);
            }
            for mut tab in tabs
                .into_iter()
                .filter(|tab| tab.session.as_deref() == Some(id.as_str()) && tab.kind == TabKind::Shell)
            {
                tab.session = None;
                store.put_tab(&tab)?;
                forgotten.push(tab.id);
            }
            Ok(forgotten)
        })
        .await?
        .map_err(|error| crate::ports::store_error(error, &self.machine, "detach the session's running terminals"))?;
        for tab in detached {
            self.terminals.forget_tab(&tab);
        }
        Ok(())
    }

    /// Moves the session's folder to this machine's trash as one item that
    /// names the session, after checking it is a direct child of its
    /// channel's `desk` folder and not a symlink. A folder already gone is
    /// no error: no item.
    async fn move_to_trash(
        &self,
        by: Option<&Caller>,
        session: &SessionRecord,
        row: &ChannelDeskRow,
    ) -> Result<Option<String>, Status> {
        if std::fs::symlink_metadata(&session.folder).is_err() {
            return Ok(None);
        }
        let (store, channel) = (self.starter.store.clone(), row.channel.clone());
        let channel = blocking(move || store.channel(&channel))
            .await?
            .map_err(|error| crate::ports::store_error(error, &self.machine, "look up the Desk session's channel"))?;
        let desk_root = match channel {
            Some(channel) => self.starter.layout.channel(&channel.folder).join("desk"),
            // The channel is gone: its folder name is the folder's parent's.
            None => session
                .folder
                .parent()
                .filter(|parent| parent.file_name().is_some_and(|name| name == "desk"))
                .map(Path::to_path_buf)
                .ok_or_else(|| {
                    failed_precondition(
                        format!("the folder {} of Desk session {} isn't in a channel's desk folder, so it was not moved", session.folder.display(), session.id),
                        "leave the folder where it is; remove it yourself once you have what you need",
                    )
                })?,
        };
        let by_actor = by.map_or_else(pb::Actor::default, identity::actor);
        let was = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Session(pb::SessionId {
                value: session.id.clone(),
            })),
        };
        let item = NewItem {
            description: format!("Desk session {}", session.title),
            caller_data: caller_data(&by_actor, &was),
            now_unix_ms: now_ms(),
        };
        let (bin, folder) = (self.bin.clone(), session.folder.clone());
        let id = session.id.clone();
        let item = blocking(move || -> Result<humata2_transfers::TrashItem, Status> {
            let folder = scratch_child(&desk_root, &folder).map_err(|error| desk_status(&error))?;
            let trash = bin.get()?;
            trash.put(&folder, item).map_err(|error| {
                failed_precondition(
                    format!(
                        "couldn't move the folder {} of Desk session {id} to the trash: {}; the session is stopped but still open",
                        folder.display(),
                        status::chain(&error)
                    ),
                    "check that the trash folder can be written to and the disk has free space, then close the session again",
                )
            })
        })
        .await??;
        Ok(Some(item.id))
    }

    /// The channel Desk row of `session`, if it is one.
    async fn channel_desk_row(&self, session: &str) -> Result<Option<ChannelDeskRow>, Status> {
        let (store, key) = (self.starter.store.clone(), session.to_owned());
        blocking(move || store.channel_desk(&key))
            .await?
            .map_err(|error| crate::ports::store_error(error, &self.machine, "read the channel Desk session's row"))
    }

    /// Every channel Desk row, closed ones included.
    async fn channel_desk_rows(&self) -> Result<Vec<ChannelDeskRow>, Status> {
        let store = self.starter.store.clone();
        blocking(move || store.channel_desks())
            .await?
            .map_err(|error| crate::ports::store_error(error, &self.machine, "read the channel Desk rows"))
    }

    async fn open_desks_of(&self, channel: &str) -> Result<Vec<ChannelDeskRow>, Status> {
        Ok(self
            .channel_desk_rows()
            .await?
            .into_iter()
            .filter(|row| row.channel == channel && row.closed_unix_ms.is_none())
            .collect())
    }

    /// The closed channel Desk session whose folder is trash item `item`.
    pub(super) async fn closed_into(&self, item: &str) -> Result<Option<ChannelDeskRow>, Status> {
        Ok(self
            .channel_desk_rows()
            .await?
            .into_iter()
            .find(|row| row.trash_item.as_deref() == Some(item)))
    }
}

/// Starts the watch (CD-5): one pass now, then one after each event on
/// `feed` that changes or removes a session or a channel, and when the
/// earliest archived target's `free_after_unix_ms` comes due. No timer
/// runs while no open channel Desk session has an archived target. Each
/// pass, per open channel Desk session, under its lock:
///
/// - its channel is gone (`store.channel` is none): close it, as Close does;
/// - its target is gone (no record) or archived with its undo window over
///   (`store.archive(target)` with `free_after_unix_ms <= now`): if its
///   folder holds a Markdown file (`documents_under` is not empty) stop its
///   agent and shells and save `target_gone`, keeping the folder and
///   staying listed; otherwise close it.
///
/// A target archived inside its undo window, or restored, leaves the
/// session as it is. A failed pass logs and waits for the next one.
///
/// Only the host watches (SL-1, MV-17, D316): it holds every channel and
/// every target, and closes or stops a session that works on another
/// machine there. A machine's own daemon may hold neither the channel nor
/// the target of a Desk session it runs, so it would take them for gone and
/// close a session the host still shows open.
pub(super) fn spawn_watch(svc: DeskSvc, feed: Arc<Feed>) {
    if svc.remote.is_none() {
        return;
    }
    tokio::spawn(async move {
        // Subscribed before the first pass, so nothing in between is missed.
        let (_, mut events) = feed.subscribe(None);
        loop {
            let due = match svc.watch_pass().await {
                Ok(due) => due,
                Err(error) => {
                    humata2_rpc_kit::log::say!(
                        "humatad2: the channel Desk watch couldn't finish a pass: {}",
                        error.message()
                    );
                    None
                }
            };
            let timer = async {
                match due {
                    Some(at) => {
                        let wait = u64::try_from(at.saturating_sub(now_ms())).unwrap_or(0);
                        tokio::time::sleep(Duration::from_millis(wait)).await;
                    }
                    None => std::future::pending::<()>().await,
                }
            };
            tokio::pin!(timer);
            loop {
                tokio::select! {
                    () = &mut timer => break,
                    event = events.recv() => match event {
                        Ok(event) if changes_session_or_channel(&event) => break,
                        Ok(_) => {}
                        // Missed some: look at everything again.
                        Err(RecvError::Lagged(_)) => break,
                        Err(RecvError::Closed) => return,
                    },
                }
            }
        }
    });
}

/// True for an event that tells of a session or a channel, the two things
/// the watch follows.
fn changes_session_or_channel(event: &pb::Event) -> bool {
    matches!(
        event.payload,
        Some(pb::event::Payload::Session(_) | pb::event::Payload::Channel(_))
    )
}

/// What the watch finds a channel Desk session's target to be.
enum Target {
    /// There, or archived with its undo window open until `until`.
    Here { until: Option<i64> },
    /// Deleted, or archived with its undo window over.
    Gone,
}

/// True when session `target` is deleted, or archived with its undo window
/// over at `now` (CD-5).
fn target_is_gone(
    store: &Store,
    target: &str,
    channel: &str,
    now: i64,
) -> Result<bool, StoreError> {
    Ok(matches!(target_in(store, target, channel, now)?, Target::Gone))
}

/// As [`target_of`], and a target that sits in another channel than the Desk
/// session's is gone too (CD-5): it can no longer be looked after from there.
fn target_in(
    store: &Store,
    target: &str,
    channel: &str,
    now: i64,
) -> Result<Target, StoreError> {
    if store
        .session(target)?
        .is_some_and(|record| record.channel != channel)
    {
        return Ok(Target::Gone);
    }
    target_of(store, target, now)
}

fn target_of(store: &Store, target: &str, now: i64) -> Result<Target, StoreError> {
    if store.session(target)?.is_none() {
        return Ok(Target::Gone);
    }
    Ok(match store.archive(target)? {
        Some(archive) if archive.free_after_unix_ms <= now => Target::Gone,
        Some(archive) => Target::Here {
            until: Some(archive.free_after_unix_ms),
        },
        None => Target::Here { until: None },
    })
}

impl DeskSvc {
    /// One pass of the watch over every open channel Desk session. Answers
    /// when the earliest archived target's undo window ends, if any.
    async fn watch_pass(&self) -> Result<Option<i64>, Status> {
        let open: Vec<String> = self
            .channel_desk_rows()
            .await?
            .into_iter()
            .filter(|row| row.closed_unix_ms.is_none())
            .map(|row| row.session_id)
            .collect();
        let mut next: Option<i64> = None;
        for id in open {
            match self.watch_one(&id).await {
                Ok(Some(at)) => next = Some(next.map_or(at, |earliest| earliest.min(at))),
                Ok(None) => {}
                Err(error) => humata2_rpc_kit::log::say!(
                    "humatad2: the channel Desk watch couldn't look after session {id}: {}",
                    error.message()
                ),
            }
        }
        Ok(next)
    }

    /// The watch's work on one session, under its lock. Answers the time
    /// its target's undo window ends, when it is archived inside it.
    async fn watch_one(&self, id: &str) -> Result<Option<i64>, Status> {
        let _held = self.locks.hold(id).await;
        let (session, mut row) = self.load_channel_desk(id).await?;
        if row.closed_unix_ms.is_some() {
            return Ok(None);
        }
        let (store, channel) = (self.starter.store.clone(), row.channel.clone());
        let target = row.target_session.clone();
        let (channel_there, target) = blocking(move || -> Result<_, StoreError> {
            let target = match &target {
                Some(target) => target_in(&store, target, &channel, now_ms())?,
                None => Target::Here { until: None },
            };
            Ok((store.channel(&channel)?.is_some(), target))
        })
        .await?
        .map_err(|error| crate::ports::store_error(error, &self.machine, "look up the Desk session's channel and target"))?;
        if !channel_there {
            self.detach_live(&session).await?;
            self.close_locked(None, &session, &row, false).await?;
            return Ok(None);
        }
        match target {
            Target::Here { until } => Ok(until),
            Target::Gone if row.target_gone => Ok(None),
            Target::Gone => {
                if self.documents_of(&session, Some(&row.channel)).await?.is_empty() {
                    self.detach_live(&session).await?;
                    self.close_locked(None, &session, &row, true).await?;
                } else {
                    // The folder holds Markdown the user may want: stop, keep
                    // the folder, and show the target gone.
                    // D801 I: the watch never ends a live holder. A session
                    // on this machine detaches it; one on another machine
                    // keeps running there under its session, still listed
                    // and attachable, until the owner ends it.
                    self.detach_live(&session).await?;
                    row.target_gone = true;
                    row.last_used_unix_ms = now_ms();
                    self.save_channel_desk_row(&row).await?;
                    self.tell.session(id, pb::Change::Updated, None).await;
                }
                Ok(None)
            }
        }
    }
}

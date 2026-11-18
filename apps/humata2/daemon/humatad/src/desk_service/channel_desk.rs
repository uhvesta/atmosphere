//! A channel's own Desk (CD-1..CD-6, D306) on DeskService: short-lived
//! managerial sessions of one channel, each in a scratch folder
//! `<channel folder>/desk/<name>` with no clones, branch, fork or board of
//! its own. The record is filed under the Desk's channel id, so it is in no
//! channel's Sessions tree and no plain Desk list; its channel Desk row says
//! which channel owns it, the session it looks after and its six management
//! switches. Its agent uses the channel's rules with the channel as its own;
//! the switches allow more on top, after Never (D182's grant model).
//!
//! The existing RPCs carry it: StartDeskSession with `channel` set,
//! ListDeskSessions with `channel` or `target` set, SetDeskAccess with
//! `switches` set. Every call is the user's; no switch answers Inbox
//! requests (IN-23). This machine only, until other machines are built
//! (MACH).

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use humata2_access::{ChannelDeskAccess, ChannelDeskSwitch, ChannelDeskSwitches, ChannelId, SessionId};
use humata2_grpc::humata::v1 as pb;
use humata2_sessions::{ChannelDeskStart, DESK_CHANNEL};
use humata2_store::{ChannelDeskRow, ChannelRecord, SessionRecord, Store, StoreError, TabRecord};
use tonic::{Code, Status};

use super::remote::{Assigned, Kind, Plan};
use super::{DeskSvc, failed_precondition, not_desk, now_ms, session_id, start_failure};
use crate::channel_service::{blocking, failed, timestamp_ms};
use crate::host_relay::CHANNEL_FOLDER;
use crate::identity::{self, Caller};
use crate::operations::{Progress, Started};
use crate::remote_work::HostOp;
use crate::session_service::{clean_title, watch_agent};
use crate::status;

/// The steps of a start, in order, as the operation shows them.
const STEPS: [(&str, &str); 3] = [
    ("write_record", "Saving the session"),
    ("launch", "Starting the agent"),
    ("verify", "Checking it started"),
];

/// The terminal size a Desk agent starts with, until a client attaches.
const COLS: u16 = 80;
const ROWS: u16 = 24;

impl DeskSvc {
    /// StartDeskSession with `channel` set (CD-1, CD-6).
    ///
    /// - The channel must exist; `target`, when set, must be a session of
    ///   that channel, not archived, deleted or itself on a Desk. Either
    ///   refusal is FailedPrecondition or InvalidArgument (`target`) and
    ///   leaves no folder and no record behind.
    /// - `machine` unset or this machine: anything else is refused as the
    ///   plain Desk refuses it (MACH not built). `picked_path` and `repos`
    ///   are refused (InvalidArgument): a channel Desk folder is always a
    ///   scratch folder with no clones (CD-1).
    /// - `switches` unset takes `ChannelDeskSwitches::default()` (read and
    ///   notes on); `access` is ignored.
    /// - Then as `start` does (desk_service/start.rs): clean title, agent,
    ///   the idempotency key's earlier answer, the three steps, then
    ///   `Starter::start_channel_desk` with `layout.channel(<channel
    ///   folder>)`, `tokens.env`, `watch_agent` and `tell.session(Created)`.
    /// - The response's `desk` is [`channel_desk_pb`], `operation` the start.
    pub(super) async fn start_channel_desk(
        &self,
        caller: &Caller,
        request: pb::StartDeskSessionRequest,
        assigned: Option<Assigned>,
    ) -> Result<pb::StartDeskSessionResponse, Status> {
        // MV-17, D316: another machine takes the start, the host's relay
        // carrying it.
        let remote = self.remote_machine(request.machine.as_ref())?;
        let original = request.clone();
        let channel_id = request
            .channel
            .as_ref()
            .map(|channel| channel.value.clone())
            .filter(|channel| !channel.is_empty())
            .ok_or_else(|| status::missing("channel"))?;
        // A channel's Desk session starts on any linked machine, the channel's or not.
        if !request.picked_path.trim().is_empty() {
            return Err(status::invalid_field(
                "picked_path",
                pb::invalid_field::Problem::NotAllowedHere,
                "a channel's Desk session works in a scratch folder of the channel, so it can't be started in a picked folder",
                "leave picked_path empty; Humata places the session in the channel's own folder",
            ));
        }
        if !request.repos.is_empty() {
            return Err(status::invalid_field(
                "repos",
                pb::invalid_field::Problem::NotAllowedHere,
                "a channel's Desk session has no clones, so it can't be started with repositories",
                "leave repos empty; the session reads the channel's sessions through its switches instead",
            ));
        }
        let target = request
            .target
            .as_ref()
            .map(|target| target.value.clone())
            .filter(|target| !target.is_empty());
        // A machine the host asked holds no record of the channel, maybe: the
        // host checked it, and named the channel's folder.
        let (channel, channel_folder) = match &assigned {
            Some(assigned) => (
                None,
                assigned
                    .channel_folder
                    .clone()
                    .ok_or_else(|| status::missing(CHANNEL_FOLDER))?,
            ),
            None => {
                let channel = self.channel_to_watch(&channel_id, target.as_deref()).await?;
                let folder = channel.folder.clone();
                (Some(channel), folder)
            }
        };
        let title = clean_title(&request.title)?;
        let title = if title.is_empty() {
            "Session".to_owned()
        } else {
            title
        };
        let agent = self.channel_agent(request.agent.as_ref())?;
        let switches = switches_from(request.switches);
        if let Some(assigned) = &assigned
            && let Some(known) = self.known_start(&assigned.id).await?
        {
            return Ok(known);
        }
        if let (Some((machine, work)), Some(channel)) = (remote, channel) {
            let plan = Plan {
                machine,
                work,
                kind: Kind::Channel {
                    channel: Box::new(channel),
                    target,
                    switches,
                },
                title,
                agent,
            };
            return self.start_remote(caller, plan, original).await;
        }

        if let Some(operation) = self
            .operations
            .existing(caller, request.idempotency_key.as_ref())?
        {
            return self.answered_channel_desk(operation).await;
        }
        // AGENTSET (bug 8): the agent's CLI is found, or installed when that
        // is allowed, before anything is made; a missing one is refused as
        // its own error naming the agent, never as UNAVAILABLE, which reads
        // as this machine not answering.
        crate::agent_gate::ready(&self.starter.agents, &agent).await?;
        let id = assigned.as_ref().map_or_else(
            || format!("se_{}", uuid::Uuid::new_v4().simple()),
            |assigned| assigned.id.clone(),
        );
        let steps: Vec<(String, String)> = STEPS
            .iter()
            .map(|(key, title)| ((*key).to_owned(), (*title).to_owned()))
            .collect();
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
            request.idempotency_key,
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return self.answered_channel_desk(operation).await,
        };
        let operation = progress.operation();
        let start = ChannelDeskStart {
            id: id.clone(),
            title,
            agent,
            model: Some(request.model).filter(|model| !model.is_empty()),
            first_message: Some(request.first_message).filter(|message| !message.is_empty()),
            machine: self.machine.clone(),
            channel: channel_id,
            channel_folder,
            target,
            read_files: switches.read_files,
            send_notes: switches.send_notes,
            type_into_terminal: switches.type_into_terminal,
            start_and_fork: switches.start_and_fork,
            stop_and_archive: switches.stop_and_archive,
            change_board_items: switches.change_board_items,
            operation: operation.id.as_ref().map(|id| id.value.clone()),
            cols: COLS,
            rows: ROWS,
        };
        match self.run_channel_desk(start, &progress).await {
            Ok(()) => {
                progress.set_step("verify", pb::StepState::Done, None);
                progress.succeed(Some(pb::operation::Result::Session(pb::SessionId {
                    value: id.clone(),
                })));
                watch_agent(self.tell.clone(), id.clone());
                self.tell
                    .session(&id, pb::Change::Created, Some(identity::actor(caller)))
                    .await;
                self.answered_channel_desk_with(operation, &id).await
            }
            Err((code, error)) => {
                let step = if code == Code::Internal { "write_record" } else { "launch" };
                progress.set_step(step, pb::StepState::Failed, Some(error.message.clone()));
                progress.fail(error.clone());
                Err(status::failure(code, error))
            }
        }
    }

    /// ListDeskSessions with `channel` or `target` set (CD-2): the open
    /// channel Desk sessions of that channel, or of the channel whose
    /// session `target` is, looking after `target` (or after the whole
    /// channel when `target` is unset), oldest first. Closed ones are left
    /// out: they are in the trash (CD-4). Runs no git.
    pub(super) async fn list_channel_desk(
        &self,
        channel: Option<pb::ChannelId>,
        target: Option<pb::SessionId>,
    ) -> Result<Vec<pb::DeskSession>, Status> {
        let channel = channel.map(|channel| channel.value);
        let target = target.map(|target| target.value);
        let store = self.starter.store.clone();
        let (rows, sessions) = blocking(move || -> Result<_, StoreError> {
            Ok((store.channel_desks()?, store.sessions()?))
        })
        .await?
        .map_err(|error| status::store(error, &self.machine, "list the channel's Desk sessions"))?;
        let mut records: HashMap<String, SessionRecord> = sessions
            .into_iter()
            .map(|session| (session.id.clone(), session))
            .collect();
        let mut open: Vec<(SessionRecord, ChannelDeskRow)> = rows
            .into_iter()
            // A session closed in place (its folder kept for a version, D452)
            // is listed as closed; one in the trash is the Trash's.
            .filter(|row| row.closed_unix_ms.is_none() || row.trash_item.is_none())
            .filter(|row| channel.as_ref().is_none_or(|channel| &row.channel == channel))
            .filter(|row| {
                target
                    .as_ref()
                    .is_none_or(|target| row.target_session.as_ref() == Some(target))
            })
            .filter_map(|row| records.remove(&row.session_id).map(|session| (session, row)))
            .filter(|(session, row)| row.closed_unix_ms.is_none() || session.folder.is_dir())
            .collect();
        open.sort_by(|(a, _), (b, _)| (a.created_unix_ms, &a.id).cmp(&(b.created_unix_ms, &b.id)));
        let asked = pb::ListDeskSessionsRequest {
            channel: channel.clone().map(|value| pb::ChannelId { value }),
            target: target.clone().map(|value| pb::SessionId { value }),
        };
        let shown: Vec<_> = open
            .iter()
            .map(|(session, row)| (self.elsewhere(session), self.channel_desk_view(session, row)))
            .collect();
        // A session on another machine shows what that machine holds (MV-17).
        Ok(self.fill_remote(shown, asked).await)
    }

    /// SetDeskAccess with `switches` set (CD-3, IN-23): saves the six
    /// switches on the session's channel Desk row and moves its
    /// `last_used_unix_ms`. The agent's next call is judged by them:
    /// [`access_of`] reads the row on every call. A session that is not a
    /// channel Desk session is NotFound; a closed one FailedPrecondition.
    /// Takes the session's lock, as `set_access` does. Answers
    /// [`channel_desk_pb`] after the change.
    pub(super) async fn set_channel_desk_switches(
        &self,
        caller: &Caller,
        request: pb::SetDeskAccessRequest,
    ) -> Result<pb::DeskSession, Status> {
        let id = session_id(request.session)?;
        let switches = switches_from(request.switches);
        let _held = self.locks.hold(&id).await;
        let (session, mut row) = self.load_channel_desk(&id).await?;
        if row.closed_unix_ms.is_some() {
            return Err(failed_precondition(
                format!("Desk session {id} is closed, so its switches can't be changed"),
                "restore it from the trash first; its switches come back as they were",
            ));
        }
        row.read_files = switches.read_files;
        row.send_notes = switches.send_notes;
        row.type_into_terminal = switches.type_into_terminal;
        row.start_and_fork = switches.start_and_fork;
        row.stop_and_archive = switches.stop_and_archive;
        row.change_board_items = switches.change_board_items;
        row.last_used_unix_ms = now_ms();
        self.save_channel_desk_row(&row).await?;
        self.tell
            .session(&id, pb::Change::Updated, Some(identity::actor(caller)))
            .await;
        Ok(self.channel_desk_view(&session, &row))
    }

    /// SetDeskTarget (CD-5): the session looks after `target` from now on,
    /// or the whole channel when it is unset. The target must be one a start
    /// could have named (a session of the channel that is there, not
    /// archived, not a Desk session), else the call is refused and nothing
    /// changes. A closed session is FailedPrecondition. The row's
    /// `target_gone` is cleared; when it was set the watch had stopped the
    /// session for it, so its agent resumes the way Reopen's does (a session
    /// on another machine is not resumed from here). The agent's next call
    /// is judged against the new target: [`access_of`] reads the row on every
    /// call. Takes the session's lock.
    pub(super) async fn set_channel_desk_target(
        &self,
        caller: &Caller,
        request: pb::SetDeskTargetRequest,
    ) -> Result<pb::DeskSession, Status> {
        let id = session_id(request.session)?;
        let target = request
            .target
            .map(|target| target.value)
            .filter(|target| !target.is_empty());
        let _held = self.locks.hold(&id).await;
        let (session, mut row) = self.load_channel_desk(&id).await?;
        if row.closed_unix_ms.is_some() {
            return Err(failed_precondition(
                format!("Desk session {id} is closed, so what it looks after can't be changed"),
                "restore it from the trash first; it keeps the target it had",
            ));
        }
        self.channel_to_watch(&row.channel, target.as_deref()).await?;
        let stopped_for_it = row.target_gone;
        // A session on another machine is resumed there: refuse before
        // changing anything when that machine can't be reached.
        if stopped_for_it
            && let (Some(machine), Some(remote)) = (self.elsewhere(&session), &self.remote)
        {
            remote.require_link(&machine).map_err(Status::from)?;
        }
        row.target_session = target;
        row.target_gone = false;
        row.last_used_unix_ms = now_ms();
        self.save_channel_desk_row(&row).await?;
        if stopped_for_it {
            self.resume_after_retarget(caller, &session).await?;
        }
        self.tell
            .session(&id, pb::Change::Updated, Some(identity::actor(caller)))
            .await;
        Ok(self.channel_desk_view(&session, &row))
    }

    /// Resumes a channel Desk session the watch stopped for its target,
    /// through the same hand-off Reopen uses; the operation follows the
    /// resume. A session on another machine is resumed there, as
    /// ResumeSession does for any session that lives elsewhere (a [`HostOp`]
    /// mirrors the machine's operation).
    async fn resume_after_retarget(
        &self,
        caller: &Caller,
        session: &SessionRecord,
    ) -> Result<(), Status> {
        let id = session.id.as_str();
        let steps: Vec<(String, String)> = vec![
            ("launch".to_owned(), "Starting the agent again".to_owned()),
            (
                "verify".to_owned(),
                "Checking it took up its conversation".to_owned(),
            ),
        ];
        let entity = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Session(pb::SessionId {
                value: id.to_owned(),
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
        let (Some(machine), Some(remote)) = (self.elsewhere(session), &self.remote) else {
            return self.resume_saved(caller, id, Ok(()), progress).await;
        };
        let host = Arc::new(HostOp::new(progress));
        match remote
            .resume_session_on(&machine, id, pb::ResumeSessionRequest::default(), host.clone())
            .await
        {
            Ok(_) => Ok(()),
            Err(error) => {
                let status = Status::from(error);
                if let Some(progress) = host.take() {
                    progress.fail(status::error_of(&status).unwrap_or_default());
                }
                Err(status)
            }
        }
    }

    /// The `DeskSession` the app shows, with the switches the channel's
    /// rules block (CD-3).
    pub(super) fn channel_desk_view(
        &self,
        session: &SessionRecord,
        row: &ChannelDeskRow,
    ) -> pb::DeskSession {
        let blocked = self.guard.with_policy(|policy, _| {
            policy
                .channel_rules(&ChannelId::new(row.channel.as_str()))
                .map(ChannelDeskSwitch::blocked_by)
                .unwrap_or_default()
        });
        channel_desk_pb(session, row, &blocked)
    }

    /// The session's record and channel Desk row, or the status that says
    /// why it isn't a channel Desk session.
    pub(super) async fn load_channel_desk(
        &self,
        id: &str,
    ) -> Result<(SessionRecord, ChannelDeskRow), Status> {
        let (store, key) = (self.starter.store.clone(), id.to_owned());
        let found = blocking(move || -> Result<_, StoreError> {
            Ok((store.session(&key)?, store.channel_desk(&key)?))
        })
        .await?;
        match found {
            Ok((Some(session), Some(row))) => Ok((session, row)),
            Ok(_) | Err(StoreError::InvalidId { .. }) => Err(not_desk(id)),
            Err(error) => Err(status::store(
                error,
                &self.machine,
                "look up the channel's Desk session",
            )),
        }
    }

    /// Saves a channel Desk row, in place of the one it had.
    pub(super) async fn save_channel_desk_row(&self, row: &ChannelDeskRow) -> Result<(), Status> {
        let (store, row) = (self.starter.store.clone(), row.clone());
        blocking(move || store.put_channel_desk(&row))
            .await?
            .map_err(|error| {
                status::store(error, &self.machine, "save the channel's Desk session")
            })
    }

    /// The channel a start is for, once the target is known to be a session
    /// of it that a Desk session can look after (CD-1).
    async fn channel_to_watch(
        &self,
        channel: &str,
        target: Option<&str>,
    ) -> Result<ChannelRecord, Status> {
        let (store, channel_key, target_key) = (
            self.starter.store.clone(),
            channel.to_owned(),
            target.map(str::to_owned),
        );
        let found = blocking(move || -> Result<_, StoreError> {
            let channel = store.channel(&channel_key)?;
            let target = match &target_key {
                Some(target) => Some((store.session(target)?, store.archive(target)?)),
                None => None,
            };
            Ok((channel, target))
        })
        .await?;
        let (channel_record, target_found) = match found {
            Ok(found) => found,
            Err(StoreError::InvalidId { .. }) => (None, None),
            Err(error) => {
                return Err(status::store(
                    error,
                    &self.machine,
                    "look up the channel and the session its Desk session would look after",
                ));
            }
        };
        let record = channel_record.ok_or_else(|| {
            status::not_found(
                pb::EntityKind::Channel,
                channel,
                format!("no channel {channel} is on this machine, so no Desk session was started"),
                "list the channels to see their ids, then start the Desk session in one of them",
            )
        })?;
        let (Some(target), Some(found)) = (target, target_found) else {
            return Ok(record);
        };
        let Some(session) = found.0 else {
            return Err(status::not_found(
                pb::EntityKind::Session,
                target,
                format!("session {target} doesn't exist any more, so no Desk session was started to look after it"),
                "pick a session of the channel that is still there, or start the Desk session for the whole channel",
            ));
        };
        if session.channel == DESK_CHANNEL {
            return Err(failed_precondition(
                format!("session {target} is itself a Desk session, and a Desk session can't look after another, so none was started"),
                "pick a session of the channel, or start the Desk session for the whole channel",
            ));
        }
        if session.channel != channel {
            return Err(failed_precondition(
                format!("session {target} is not a session of channel {channel}, so no Desk session was started to look after it"),
                "pick a session of this channel, or start the Desk session in the channel that session belongs to",
            ));
        }
        if found.1.is_some() {
            return Err(failed_precondition(
                format!("session {target} is archived, so no Desk session was started to look after it"),
                "restore the session first, or start the Desk session for the whole channel",
            ));
        }
        Ok(record)
    }

    /// The agent a start runs: the one named, else `--default-agent`, which
    /// must be one Settings › Agents knows.
    fn channel_agent(&self, wanted: Option<&pb::AgentId>) -> Result<String, Status> {
        let name = wanted
            .map(|agent| agent.value.clone())
            .filter(|agent| !agent.is_empty())
            .unwrap_or_else(|| self.default_agent.clone());
        // SEAM: AGENTSET: a Desk start of a kind that is off is refused.
        crate::agent_gate::enabled(&self.starter.agents, &name)?;
        self.starter.agents.get(&name).map(|_| name.clone()).map_err(|error| {
            failed(
                Code::InvalidArgument,
                &error,
                "pick one of the agents named, or add the agent in Settings",
                Some(pb::error::Kind::InvalidField(pb::InvalidField {
                    field: "agent".to_owned(),
                    problem: pb::invalid_field::Problem::UnknownValue as i32,
                })),
            )
        })
    }

    /// Runs the start on the starter, moving the operation's steps as it
    /// goes. The error is what the app shows.
    async fn run_channel_desk(
        &self,
        start: ChannelDeskStart,
        progress: &Progress,
    ) -> Result<(), (Code, pb::Error)> {
        let tokens = self.tokens.clone();
        let env = move |tab: &TabRecord| tokens.env(tab);
        let mut note = |event: &str| {
            for (key, _) in STEPS {
                if key == event {
                    break;
                }
                progress.set_step(key, pb::StepState::Done, None);
            }
            progress.set_step(event, pb::StepState::Running, None);
        };
        progress.set_step("write_record", pb::StepState::Running, None);
        let started = self.starter.start_channel_desk(start, &env, &mut note).await;
        started.map(|_| ()).map_err(|error| start_failure(&error))
    }

    /// The response for a start that already happened.
    pub(super) async fn answered_channel_desk(
        &self,
        operation: pb::Operation,
    ) -> Result<pb::StartDeskSessionResponse, Status> {
        let key = operation
            .id
            .as_ref()
            .map(|id| id.value.clone())
            .unwrap_or_default();
        let store = self.starter.store.clone();
        let session = blocking(move || -> Result<Option<String>, StoreError> {
            Ok(store
                .session_details()?
                .into_iter()
                .find(|detail| detail.operation.as_deref() == Some(key.as_str()))
                .map(|detail| detail.session))
        })
        .await?
        .map_err(|error| {
            status::store(error, &self.machine, "find the Desk session being started")
        })?;
        let session = session.ok_or_else(|| {
            status::internal(
                "the operation that started a Desk session has no session recorded",
                "",
            )
        })?;
        self.answered_channel_desk_with(operation, &session).await
    }

    async fn answered_channel_desk_with(
        &self,
        operation: pb::Operation,
        session: &str,
    ) -> Result<pb::StartDeskSessionResponse, Status> {
        let (record, row) = self.load_channel_desk(session).await?;
        Ok(pb::StartDeskSessionResponse {
            session: Some(pb::SessionId {
                value: session.to_owned(),
            }),
            operation: Some(operation),
            desk: Some(self.channel_desk_view(&record, &row)),
        })
    }
}

/// The channel a channel Desk session belongs to and what its switches
/// allow, read from the store on every call so a switch the user changes,
/// or a fork made after the session started, applies from the agent's next
/// call. `None` for any other session.
pub(crate) fn access_of(
    store: &Store,
    session: &str,
) -> Result<Option<(ChannelId, ChannelDeskAccess)>, StoreError> {
    let Some(row) = store.channel_desk(session)? else {
        return Ok(None);
    };
    let covers = match &row.target_session {
        None => None,
        Some(target) => Some(covered_by(&store.fork_origins()?, target)),
    };
    let access = ChannelDeskAccess {
        target: row.target_session.as_deref().map(SessionId::new),
        covers,
        switches: switches_of(&row),
    };
    Ok(Some((ChannelId::new(row.channel.as_str()), access)))
}

/// `target` and every fork of it, transitively, from `origins`.
fn covered_by(origins: &[humata2_store::ForkOrigin], target: &str) -> BTreeSet<SessionId> {
    let mut covered: BTreeSet<String> = BTreeSet::from([target.to_owned()]);
    loop {
        let before = covered.len();
        for origin in origins {
            if covered.contains(&origin.parent) {
                covered.insert(origin.session_id.clone());
            }
        }
        if covered.len() == before {
            break;
        }
    }
    covered.into_iter().map(SessionId::new).collect()
}

/// The six switches a row holds.
pub(crate) fn switches_of(row: &ChannelDeskRow) -> ChannelDeskSwitches {
    ChannelDeskSwitches {
        read_files: row.read_files,
        send_notes: row.send_notes,
        type_into_terminal: row.type_into_terminal,
        start_and_fork: row.start_and_fork,
        stop_and_archive: row.stop_and_archive,
        change_board_items: row.change_board_items,
    }
}

/// The switches a request names, the New desk session dialog's defaults
/// when unset.
pub(crate) fn switches_from(switches: Option<pb::ChannelDeskSwitches>) -> ChannelDeskSwitches {
    match switches {
        None => ChannelDeskSwitches::default(),
        Some(switches) => ChannelDeskSwitches {
            read_files: switches.read_files,
            send_notes: switches.send_notes,
            type_into_terminal: switches.type_into_terminal,
            start_and_fork: switches.start_and_fork,
            stop_and_archive: switches.stop_and_archive,
            change_board_items: switches.change_board_items,
        },
    }
}

fn switch_pb(switch: ChannelDeskSwitch) -> pb::ChannelDeskSwitch {
    match switch {
        ChannelDeskSwitch::ReadFiles => pb::ChannelDeskSwitch::ReadFiles,
        ChannelDeskSwitch::SendNotes => pb::ChannelDeskSwitch::SendNotes,
        ChannelDeskSwitch::TypeIntoTerminal => pb::ChannelDeskSwitch::TypeIntoTerminal,
        ChannelDeskSwitch::StartAndFork => pb::ChannelDeskSwitch::StartAndFork,
        ChannelDeskSwitch::StopAndArchive => pb::ChannelDeskSwitch::StopAndArchive,
        ChannelDeskSwitch::ChangeBoardItems => pb::ChannelDeskSwitch::ChangeBoardItems,
    }
}

/// The `DeskSession` the app shows for a channel Desk session: its folder
/// (always scratch; kept once it is closed in place), its times, its Markdown files, and the
/// channel Desk part. `blocked` lists the switches the channel's rules set
/// to Never (`ChannelDeskSwitch::blocked_by` on the guard's policy), shown
/// as "Blocked by the channel rule: Never" (CD-3).
pub(crate) fn channel_desk_pb(
    session: &SessionRecord,
    row: &ChannelDeskRow,
    blocked: &[ChannelDeskSwitch],
) -> pb::DeskSession {
    let switches = switches_of(row);
    let (has_versions, doc_head) = super::view::doc_state(&session.folder);
    let (version_count, holds_files) = super::view::folder_signals(&session.folder);
    pb::DeskSession {
        session: Some(pb::SessionId {
            value: session.id.clone(),
        }),
        folder: Some(pb::DeskFolder {
            path: session.folder.to_string_lossy().into_owned(),
            kind: Some(pb::desk_folder::Kind::Scratch(pb::desk_folder::Scratch {
                // Closed with its folder left where it is (D452).
                kept: row.closed_unix_ms.is_some() && row.trash_item.is_none(),
            })),
        }),
        access: None,
        last_used_at: Some(timestamp_ms(row.last_used_unix_ms)),
        closed_at: row.closed_unix_ms.map(timestamp_ms),
        delete_at: None,
        held_work: String::new(),
        repos: Vec::new(),
        documents: humata2_sessions::documents_under(&session.folder),
        document_lines: humata2_sessions::document_lines_under(&session.folder)
            .into_iter()
            .collect(),
        channel_desk: Some(pb::ChannelDesk {
            channel: Some(pb::ChannelId {
                value: row.channel.clone(),
            }),
            target: row
                .target_session
                .clone()
                .map(|value| pb::SessionId { value }),
            switches: Some(pb::ChannelDeskSwitches {
                read_files: switches.read_files,
                send_notes: switches.send_notes,
                type_into_terminal: switches.type_into_terminal,
                start_and_fork: switches.start_and_fork,
                stop_and_archive: switches.stop_and_archive,
                change_board_items: switches.change_board_items,
            }),
            blocked: blocked.iter().map(|switch| switch_pb(*switch) as i32).collect(),
            target_gone: row.target_gone,
            trash_item: row
                .trash_item
                .clone()
                .map(|value| pb::TrashItemId { value }),
        }),
        doc_head,
        has_versions,
        version_count,
        holds_files,
    }
}

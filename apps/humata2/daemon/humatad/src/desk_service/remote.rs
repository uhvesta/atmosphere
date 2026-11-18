//! Desk sessions on other machines (MV-17, D316): a plain Desk session or a
//! channel's own can start on any linked machine, as a channel session does
//! (SL-1, PER-11). The host writes its own record first, with an id it mints,
//! then makes the same start on the machine through the relay, carrying the
//! id. The machine runs its ordinary start in its own `<root>/desk/<name>`
//! (a channel's in `<root>/<channel folder>/desk/<name>`, even when the
//! machine holds no record of the channel: the host names the folder) and
//! answers where; the host's record then names that folder. Nothing polls:
//! the machine's operation is mirrored into the host's as it runs.

use std::path::PathBuf;
use std::sync::Arc;

use humata2_access::ChannelDeskSwitches;
use humata2_grpc::humata::v1 as pb;
use humata2_sessions::DESK_CHANNEL;
use humata2_store::{
    ChannelDeskRow, ChannelRecord, DeskFolderKind, DeskRow, SessionDetail, SessionRecord, StartStep, StoreError,
};
use tonic::{Request, Status};

use super::{DeskSvc, failed_precondition, now_ms, view};
use crate::channel_service::blocking;
use crate::host_relay::CHANNEL_FOLDER;
use crate::identity::{self, Caller};
use crate::operations::Started;
use crate::remote_work::{HostOp, RemoteWork};
use crate::session_service::stuck_start;
use crate::status;

/// The steps of a start, as the operation shows them: the machine's own.
const STEPS: [(&str, &str); 3] = [
    ("write_record", "Saving the session"),
    ("launch", "Starting the agent"),
    ("verify", "Checking it started"),
];

/// What the host told this machine about the session it is asked to start.
pub(super) struct Assigned {
    /// The session's id, which the host minted.
    pub(super) id: String,
    /// A channel's folder name, for a channel's Desk session: this machine
    /// may hold no record of the channel.
    pub(super) channel_folder: Option<String>,
}

/// What a relayed start carries beyond the request: the id the host chose
/// and, for a channel's Desk session, the channel's folder.
pub(super) fn assigned_of<T>(request: &Request<T>, caller: &Caller) -> Result<Option<Assigned>, Status> {
    let Some(id) = identity::host_assigned_id(request, caller)? else {
        return Ok(None);
    };
    let channel_folder = match request.metadata().get(CHANNEL_FOLDER) {
        None => None,
        Some(value) => {
            let folder = value.to_str().map_err(|_| status::missing(CHANNEL_FOLDER))?;
            let plain = folder
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
            if folder.is_empty() || folder == DESK_CHANNEL || !plain {
                return Err(status::missing(CHANNEL_FOLDER));
            }
            Some(folder.to_owned())
        }
    };
    Ok(Some(Assigned { id, channel_folder }))
}

/// Which Desk a remote start is for.
pub(super) enum Kind {
    Plain {
        access: pb::DeskAccess,
    },
    Channel {
        channel: Box<ChannelRecord>,
        target: Option<String>,
        switches: ChannelDeskSwitches,
    },
}

/// A start that goes to another machine.
pub(super) struct Plan {
    pub(super) machine: String,
    pub(super) work: RemoteWork,
    pub(super) kind: Kind,
    /// The cleaned title.
    pub(super) title: String,
    /// The agent's name: the one asked for, else the host's default.
    pub(super) agent: String,
}

/// The rows a start writes on the host.
#[derive(Clone)]
enum Rows {
    Plain(DeskRow),
    Channel(ChannelDeskRow),
}

/// The failure an operation records for `status`.
pub(super) fn failure_of(status: &Status) -> pb::Error {
    status::error_of(status).unwrap_or_else(|| pb::Error {
        message: status.message().to_owned(),
        fix: String::new(),
        retry: None,
        kind: None,
    })
}

impl DeskSvc {
    /// The machine a start names when it isn't this one: the host's relay
    /// to it, once it is known to be at the host's release and linked.
    /// None for this machine. A daemon that works with no other machine
    /// refuses a start elsewhere, and nothing is made (MV-17).
    pub(super) fn remote_machine(
        &self,
        wanted: Option<&pb::MachineId>,
    ) -> Result<Option<(String, RemoteWork)>, Status> {
        let Some(named) = wanted.filter(|machine| !machine.value.is_empty() && machine.value != self.machine) else {
            return Ok(None);
        };
        let Some(remote) = &self.remote else {
            return Err(failed_precondition(
                format!(
                    "machine {} isn't this one, and this machine starts Desk sessions only on itself, so nothing was started",
                    named.value
                ),
                "start the Desk session from the host, naming the machine, or leave machine unset to start it here",
            ));
        };
        // UP-1: nothing starts on a machine at another release.
        crate::upgrade::gate::machine_ready(&self.migrations, named)?;
        // D331: a machine this host has never linked is an unknown value of
        // `machine`, as it is for a fork.
        if !remote.knows(&named.value) {
            return Err(status::invalid_field(
                "machine",
                pb::invalid_field::Problem::UnknownValue,
                format!("{} isn't a machine Humata knows, so no Desk session was started", named.value),
                "list the machines to see their names, or add it on the Machines page first",
            ));
        }
        remote.require_link(&named.value).map_err(Status::from)?;
        Ok(Some((named.value.clone(), remote.clone())))
    }

    /// Starts the Desk session on another machine: the host's records first,
    /// then the machine's own start under the same id, mirrored into the
    /// host's operation. A machine that refuses takes the records back.
    pub(super) async fn start_remote(
        &self,
        caller: &Caller,
        plan: Plan,
        mut asked: pb::StartDeskSessionRequest,
    ) -> Result<pb::StartDeskSessionResponse, Status> {
        let Plan { machine, work, kind, title, agent } = plan;
        let key = asked.idempotency_key.take();
        if let Some(operation) = self.operations.existing(caller, key.as_ref())? {
            return self.answered_remote(&kind, operation).await;
        }
        let id = format!("se_{}", uuid::Uuid::new_v4().simple());
        let steps: Vec<(String, String)> = STEPS
            .iter()
            .map(|(step, shown)| ((*step).to_owned(), (*shown).to_owned()))
            .collect();
        let entity = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Session(pb::SessionId { value: id.clone() })),
        };
        let progress = match self
            .operations
            .start_steps(caller, pb::OperationKind::StartSession, entity, &steps, false, key)?
        {
            Started::New(progress) => progress,
            Started::Existing(operation) => return self.answered_remote(&kind, operation).await,
        };
        let now = now_ms();
        let record = SessionRecord {
            id: id.clone(),
            channel: DESK_CHANNEL.to_owned(),
            machine: machine.clone(),
            title: title.clone(),
            agent: agent.clone(),
            // The machine's own path, which it answers with.
            folder: PathBuf::new(),
            created_unix_ms: now,
        };
        let detail = SessionDetail {
            session: id.clone(),
            repos: Vec::new(),
            model: Some(asked.model.clone()).filter(|model| !model.is_empty()),
            first_message: Some(asked.first_message.clone()).filter(|message| !message.is_empty()),
            // D731: the machine owns the start, so the host's record is
            // saved as started. A host restart must never finish or undo it
            // here as a start cut short on this machine; the journal kept
            // below asks the machine again instead.
            start: StartStep::Done,
            operation: progress.operation().id.map(|operation| operation.value),
            revision: 1,
        };
        let (rows, channel_folder) = match &kind {
            Kind::Plain { access } => (
                Rows::Plain(DeskRow {
                    session_id: id.clone(),
                    kind: DeskFolderKind::Scratch { kept: false },
                    boards: access.boards,
                    channels_and_machines: access.channels_and_machines,
                    last_used_unix_ms: now,
                    closed_unix_ms: None,
                    deleted_unix_ms: None,
                    held_work: String::new(),
                }),
                None,
            ),
            Kind::Channel { channel, target, switches } => (
                Rows::Channel(ChannelDeskRow {
                    session_id: id.clone(),
                    channel: channel.id.clone(),
                    target_session: target.clone(),
                    read_files: switches.read_files,
                    send_notes: switches.send_notes,
                    type_into_terminal: switches.type_into_terminal,
                    start_and_fork: switches.start_and_fork,
                    stop_and_archive: switches.stop_and_archive,
                    change_board_items: switches.change_board_items,
                    last_used_unix_ms: now,
                    closed_unix_ms: None,
                    target_gone: false,
                    trash_item: None,
                }),
                Some(channel.folder.clone()),
            ),
        };
        asked.title = title;
        asked.agent = Some(pb::AgentId { value: agent });
        asked.machine = None;
        // D731: kept before the host's records, so a host that dies before
        // the machine's answer asks it again on its restart.
        let journal = super::remote_asked::Asked::new(&progress, &id, &machine, &asked, channel_folder.as_deref());
        if let Err(shown) = self.keep_asked(&journal).await {
            progress.fail(failure_of(&shown));
            return Err(shown);
        }
        let (store, saving, detailed, filed) = (self.starter.store.clone(), record.clone(), detail, rows.clone());
        let written = blocking(move || -> Result<(), StoreError> {
            store.put_session(&saving)?;
            store.put_session_detail(&detailed)?;
            match &filed {
                Rows::Plain(row) => store.put_desk(row),
                Rows::Channel(row) => store.put_channel_desk(row),
            }
        })
        .await?;
        if let Err(error) = written {
            let shown = status::store(error, &self.machine, "save the Desk session");
            progress.fail(failure_of(&shown));
            self.drop_asked(&id).await;
            return Err(shown);
        }
        progress.set_step("write_record", pb::StepState::Done, None);
        let host = Arc::new(HostOp::new(progress));
        let answer = work
            .start_desk_on(&machine, &id, asked, channel_folder.as_deref(), host.clone())
            .await;
        // The machine has answered, and the host hasn't kept the answer yet.
        crate::fault::point("desk.remote.answered");
        match answer {
            Ok(response) => {
                let mut record = record;
                let folder = response
                    .desk
                    .as_ref()
                    .and_then(|desk| desk.folder.as_ref())
                    .map(|folder| PathBuf::from(&folder.path));
                if let Some(folder) = folder {
                    record.folder = folder;
                    let (store, saving) = (self.starter.store.clone(), record.clone());
                    if let Err(error) = blocking(move || store.put_session(&saving)).await? {
                        crate::log::say!(
                            "humatad2: couldn't save where session {id} works on {machine}: {}; its folder shows empty until the session moves or is started again",
                            status::chain(&error)
                        );
                    }
                }
                self.drop_asked(&id).await;
                self.tell
                    .session(&id, pb::Change::Created, Some(identity::actor(caller)))
                    .await;
                let desk = match &rows {
                    Rows::Plain(row) => view::desk_pb(&record, row),
                    Rows::Channel(row) => self.channel_desk_view(&record, row),
                };
                Ok(pb::StartDeskSessionResponse {
                    session: Some(pb::SessionId { value: id }),
                    operation: response.operation,
                    desk: Some(desk),
                })
            }
            Err(error) => {
                let shown = Status::from(error);
                if let Some(progress) = host.take() {
                    progress.fail(failure_of(&shown));
                }
                let (store, removing) = (self.starter.store.clone(), id.clone());
                let _ = blocking(move || store.remove_session(&removing)).await;
                self.drop_asked(&id).await;
                Err(shown)
            }
        }
    }

    /// The response for a remote start that already happened.
    async fn answered_remote(
        &self,
        kind: &Kind,
        operation: pb::Operation,
    ) -> Result<pb::StartDeskSessionResponse, Status> {
        match kind {
            Kind::Plain { .. } => self.answered(operation).await,
            Kind::Channel { .. } => self.answered_channel_desk(operation).await,
        }
    }

    /// The answer for a start that names a session this machine already has:
    /// the host asking again after a link dropped. None when it has none.
    pub(super) async fn known_start(&self, id: &str) -> Result<Option<pb::StartDeskSessionResponse>, Status> {
        // A start asked again across this machine's restart is answered once
        // recovery has finished, undone or named it stuck, never from a record
        // recovery is about to name stuck as still running.
        self.restart.wait_for(crate::restart::Awaited::DeskStart).await?;
        let named = stuck_after_restart(&self.restart, id).await;
        let (store, key) = (self.starter.store.clone(), id.to_owned());
        let found = blocking(move || -> Result<_, StoreError> {
            Ok((
                store.session(&key)?,
                store.session_detail(&key)?,
                store.desk(&key)?,
                store.channel_desk(&key)?,
            ))
        })
        .await?
        .map_err(|error| status::store(error, &self.machine, "look up the Desk session"))?;
        let (Some(record), detail, plain, own) = found else {
            return Ok(None);
        };
        let desk = match (own, plain) {
            (Some(row), _) => self.channel_desk_view(&record, &row),
            (None, Some(row)) => view::desk_pb(&record, &row),
            (None, None) => return Ok(None),
        };
        let done = detail.as_ref().is_some_and(|detail| detail.start == StartStep::Done);
        // A start the restart couldn't finish or remove stays as it was
        // left, so the host is told it failed, with the restart's words,
        // instead of waiting for it forever.
        let stuck = named.filter(|_| !done);
        let operation = start_operation(id, detail.and_then(|detail| detail.operation), done, stuck.as_deref());
        Ok(Some(pb::StartDeskSessionResponse {
            session: Some(pb::SessionId { value: id.to_owned() }),
            operation: Some(operation),
            desk: Some(desk),
        }))
    }
}

/// What the restart says of the start of session `id`: the problem it named
/// when it couldn't finish or remove it. Waits for the restart to be done,
/// so a start asked during recovery is never answered from a record recovery
/// is about to name stuck.
async fn stuck_after_restart(restart: &crate::restart::Started, id: &str) -> Option<String> {
    restart.wait().await;
    restart.get().and_then(|last| stuck_start(last, id)).map(str::to_owned)
}

/// A Desk start's operation as the host is answered: done, failed because
/// the restart named it stuck (`stuck` is what it says), or still running.
fn start_operation(id: &str, operation: Option<String>, done: bool, stuck: Option<&str>) -> pb::Operation {
    let state = match (done, stuck) {
        (true, _) => pb::OperationState::Succeeded,
        (false, Some(_)) => pb::OperationState::Failed,
        (false, None) => pb::OperationState::Running,
    };
    let error = stuck.filter(|_| !done).map(|problem| pb::Error {
        message: problem.to_owned(),
        fix: "remove the Desk session's folder on this machine as the message says, then start the session again".to_owned(),
        retry: None,
        kind: None,
    });
    pb::Operation {
        id: operation.map(|value| pb::OperationId { value }),
        kind: pb::OperationKind::StartSession as i32,
        target: Some(pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Session(pb::SessionId { value: id.to_owned() })),
        }),
        state: state as i32,
        error,
        result: Some(pb::operation::Result::Session(pb::SessionId { value: id.to_owned() })),
        ..pb::Operation::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A start asked while the restart is still recovering is answered once
    /// the restart says whether it is stuck, never before: paused time never
    /// moves, so nothing polled.
    #[tokio::test(start_paused = true)]
    async fn a_start_asked_during_recovery_waits_for_the_restart_to_say_if_it_is_stuck() {
        let restart = std::sync::Arc::new(crate::restart::Started::default());
        let asked = tokio::spawn({
            let restart = restart.clone();
            async move { stuck_after_restart(&restart, "se_1").await }
        });
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert!(!asked.is_finished(), "nothing is answered while the restart runs");

        restart.set(pb::LastStart {
            not_brought_back: vec![pb::NotBroughtBack {
                session: "se_1".to_owned(),
                tab: String::new(),
                problem: "delete its folder by hand".to_owned(),
            }],
            ..pb::LastStart::default()
        });

        assert_eq!(asked.await.unwrap().as_deref(), Some("delete its folder by hand"));
        assert_eq!(stuck_after_restart(&restart, "se_2").await, None, "a start the restart didn't name is not stuck");
    }

    /// A start the restart named stuck is failed, with its words, so the
    /// host stops waiting.
    #[test]
    fn a_stuck_desk_start_is_answered_failed_with_the_restarts_words() {
        let operation = start_operation("se_1", Some("op_1".to_owned()), false, Some("delete its folder by hand"));
        assert_eq!(operation.state, pb::OperationState::Failed as i32);
        assert_eq!(operation.error.expect("a failure names what went wrong").message, "delete its folder by hand");
    }

    /// A start still in progress is running; one that finished succeeded,
    /// even if a stale line of the report names it.
    #[test]
    fn a_desk_start_not_named_stuck_is_running_or_succeeded() {
        let running = start_operation("se_1", None, false, None);
        assert_eq!(running.state, pb::OperationState::Running as i32);
        assert!(running.error.is_none());
        let done = start_operation("se_1", None, true, Some("stale"));
        assert_eq!(done.state, pb::OperationState::Succeeded as i32);
        assert!(done.error.is_none());
    }
}

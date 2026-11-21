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
use std::time::Duration;

use humata2_access::ChannelDeskSwitches;
use crate::pb;
use humata2_sessions::DESK_CHANNEL;
use humata2_store::{
    ChannelDeskRow, ChannelRecord, DeskFolderKind, DeskRow, SessionDetail, SessionRecord, StartStep, StoreError,
};
use tonic::{Code, Request, Status};

use super::{DeskSvc, failed_precondition, now_ms, view};
use crate::ports::blocking;
use crate::ports::CHANNEL_FOLDER;
use humata2_daemon_ports::Caller;
use crate::ports as identity;
use crate::ports::Started;
use crate::ports::{HostOp, RelayError, RemoteError, RemoteWork};
use crate::ports::stuck_start;
use crate::ports as status;

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
    humata2_rpc_kit::helpers::status_error_of(status).unwrap_or_else(|| pb::Error {
        message: status.message().to_owned(),
        fix: String::new(),
        retry: None,
        kind: None,
    })
}

/// Whether `error` is an explicit pre-start refusal from the machine, meaning
/// the machine never started the session and the host can safely delete its
/// session row and drop the `Asked` journal. Transient or ambiguous errors
/// (`Offline`, `Unavailable`, `Unknown`, `DeadlineExceeded`, `Cancelled`, etc.)
/// return `false` so the host keeps the row and journal and re-asks under the
/// same session id.
pub(super) fn is_definitive_refusal(error: &RemoteError) -> bool {
    let code = match error {
        RemoteError::Relay(RelayError::Offline { .. }) => return false,
        RemoteError::Relay(RelayError::Refused { answer }) | RemoteError::Failed { answer } => answer.code(),
    };
    matches!(
        code,
        Code::InvalidArgument
            | Code::FailedPrecondition
            | Code::PermissionDenied
            | Code::Unimplemented
            | Code::AlreadyExists
            | Code::NotFound
            | Code::Unauthenticated
            | Code::OutOfRange
    )
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
        self.migrations.machine_ready( named)?;
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
            let shown = crate::ports::store_error(error, &self.machine, "save the Desk session");
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
        humata2_daemon_runtime::fault::point("desk.remote.answered");
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
                        humata2_rpc_kit::log::say!(
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
                let definitive = is_definitive_refusal(&error);
                let shown = Status::from(error);
                if definitive {
                    if let Some(progress) = host.take() {
                        progress.fail(failure_of(&shown));
                    }
                    let (store, removing) = (self.starter.store.clone(), id.clone());
                    let _ = blocking(move || store.remove_session(&removing)).await;
                    self.drop_asked(&id).await;
                } else if let Some(progress) = host.take() {
                    // The call ended ambiguously (transport drop, timeout, etc.):
                    // the machine may already have started the session. Keep the
                    // host row and `Asked` journal and re-ask under the same id.
                    let svc = self.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        let _ = svc.ask_again(progress, &journal).await;
                    });
                }
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
        self.restart.wait_for(crate::ports::Awaited::DeskStart).await?;
        let named = stuck_after_restart(&*self.restart, id).await;
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
        .map_err(|error| crate::ports::store_error(error, &self.machine, "look up the Desk session"))?;
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
async fn stuck_after_restart(restart: &dyn crate::ports::DeskRestartPort, id: &str) -> Option<String> {
    restart.wait().await;
    restart.get().as_ref().and_then(|last| stuck_start(last, id)).map(str::to_owned)
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
        let restart = std::sync::Arc::new(crate::ports::TestRestart::default());
        let asked = tokio::spawn({
            let restart = restart.clone();
            async move { stuck_after_restart(restart.as_ref(), "se_1").await }
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
        assert_eq!(stuck_after_restart(restart.as_ref(), "se_2").await, None, "a start the restart didn't name is not stuck");
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

    #[test]
    fn definitive_refusal_classification_keeps_transient_and_ambiguous_codes() {
        for transient in [
            RemoteError::Relay(RelayError::Offline {
                machine: "box-1".to_owned(),
                answer: Status::unavailable("offline"),
            }),
            RemoteError::Failed { answer: Status::unavailable("link dropped") },
            RemoteError::Failed { answer: Status::unknown("transport reset") },
            RemoteError::Failed { answer: Status::deadline_exceeded("timed out") },
            RemoteError::Failed { answer: Status::cancelled("cancelled") },
        ] {
            assert!(!is_definitive_refusal(&transient), "should not clean up on {transient:?}");
        }
        for refusal in [
            RemoteError::Failed { answer: Status::invalid_argument("bad agent") },
            RemoteError::Failed { answer: Status::failed_precondition("not ready") },
            RemoteError::Failed { answer: Status::permission_denied("denied") },
            RemoteError::Failed { answer: Status::unimplemented("old daemon") },
            RemoteError::Failed { answer: Status::already_exists("duplicate") },
        ] {
            assert!(is_definitive_refusal(&refusal), "should clean up on {refusal:?}");
        }
    }

    struct FakeProgress {
        op: pb::Operation,
        origin: crate::ports::Origin,
        tx: tokio::sync::watch::Sender<pb::Operation>,
    }

    impl FakeProgress {
        fn new(op: pb::Operation, origin: crate::ports::Origin) -> Arc<Self> {
            let (tx, _) = tokio::sync::watch::channel(op.clone());
            Arc::new(Self { op, origin, tx })
        }
    }

    impl crate::ports::ProgressPort for FakeProgress {
        fn id(&self) -> String {
            self.op.id.as_ref().map_or_default(|id| id.value.clone())
        }
        fn operation(&self) -> pb::Operation {
            self.op.clone()
        }
        fn origin(&self) -> crate::ports::Origin {
            self.origin.clone()
        }
        fn set_step(&self, _key: &str, _state: pb::StepState, _output_tail: Option<String>) {}
        fn succeed(&self, _result: Option<pb::operation::Result>) {}
        fn fail(&self, _error: pb::Error) {}
        fn end_cancelled(&self) {}
        fn watch(&self) -> tokio::sync::watch::Receiver<pb::Operation> {
            self.tx.subscribe()
        }
    }

    struct FakeOps;

    #[tonic::async_trait]
    impl crate::ports::DeskOperationsPort for FakeOps {
        fn existing(
            &self,
            _caller: &Caller,
            _key: Option<&pb::IdempotencyKey>,
        ) -> Result<Option<pb::Operation>, Status> {
            Ok(None)
        }
        fn progress_of(&self, _caller: &Caller, _id: &str) -> Result<crate::ports::Progress, Status> {
            Err(Status::not_found("none"))
        }
        fn restore(&self, op: pb::Operation, origin: &crate::ports::Origin) -> Option<crate::ports::Progress> {
            Some(crate::ports::Progress::new(FakeProgress::new(op, origin.clone())))
        }
        fn start_steps(
            &self,
            _caller: &Caller,
            kind: pb::OperationKind,
            target: pb::EntityRef,
            _steps: &[(String, String)],
            _cancellable: bool,
            _key: Option<pb::IdempotencyKey>,
        ) -> Result<Started, Status> {
            let op = pb::Operation {
                id: Some(pb::OperationId { value: "op_1".to_owned() }),
                kind: kind as i32,
                target: Some(target),
                ..pb::Operation::default()
            };
            let origin = crate::ports::Origin {
                owner: "user".to_owned(),
                scope: None,
            };
            Ok(Started::New(crate::ports::Progress::new(FakeProgress::new(op, origin))))
        }
    }

    struct NoopGuard;
    impl crate::ports::DeskGuardPort for NoopGuard {
        fn check(&self, _caller: &Caller, _call: &humata2_access::Call) -> Result<crate::ports::Checked, Status> {
            Ok(crate::ports::Checked::Allowed)
        }
        fn require(&self, _caller: &Caller, _call: &humata2_access::Call) -> Result<(), Status> {
            Ok(())
        }
        fn end_grants_of(&self, _session: &humata2_access::SessionId) -> Vec<humata2_access::Grant> {
            Vec::new()
        }
        fn channel_decision(
            &self,
            _channel: &humata2_access::ChannelId,
            _capability: humata2_access::Capability,
        ) -> Option<humata2_access::Decision> {
            None
        }
        fn channel_rules(&self, _channel: &humata2_access::ChannelId) -> Option<humata2_access::ChannelRules> {
            None
        }
        fn policy(&self) -> humata2_access::Policy {
            humata2_access::Policy::default()
        }
    }

    struct NoopIdentities;
    impl crate::ports::DeskIdentityPort for NoopIdentities {
        fn revoke(&self, _tab_id: &str) -> Result<(), StoreError> {
            Ok(())
        }
    }

    struct NoopTokens;
    impl crate::ports::DeskTokensPort for NoopTokens {
        fn env(&self, _tab: &humata2_store::TabRecord) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
            Vec::new()
        }
    }

    struct NoopInbox;
    #[tonic::async_trait]
    impl crate::ports::DeskInboxPort for NoopInbox {
        async fn ask_read(
            &self,
            _caller: &Caller,
            _asked: humata2_access::AccessRequest,
            _operation: &str,
            _run: Option<crate::ports::Run>,
        ) -> Result<(), Status> {
            Ok(())
        }
        async fn withdraw_pending_of(&self, _session: &str, _reason: pb::WithdrawReason) -> Result<(), Status> {
            Ok(())
        }
        async fn revoked(&self, _id: &str, _at: std::time::SystemTime) -> Result<(), Status> {
            Ok(())
        }
        async fn defer(
            &self,
            _caller: &Caller,
            _asked: humata2_access::AccessRequest,
            _key: Option<String>,
            _run: crate::ports::Run,
        ) -> Status {
            Status::permission_denied("deferred")
        }
    }

    struct NoopTell;
    #[tonic::async_trait]
    impl crate::ports::DeskTellPort for NoopTell {
        async fn session(&self, _session_id: &str, _change: pb::Change, _by: Option<pb::Actor>) {}
    }

    struct NoopBin;
    impl crate::ports::DeskTrashPort for NoopBin {
        fn get(&self) -> Result<Arc<dyn crate::ports::TrashOps>, Status> {
            Err(Status::unimplemented("unused"))
        }
    }

    struct NoopTerminals;
    impl crate::ports::DeskTerminalPort for NoopTerminals {
        fn forget_tab(&self, _tab_id: &str) {}
    }

    struct NoopMigrations;
    #[tonic::async_trait]
    impl crate::ports::DeskMigrationsPort for NoopMigrations {
        fn machine_ready(&self, _machine: &pb::MachineId) -> Result<(), Status> {
            Ok(())
        }
        async fn observe(
            &self,
            _thing: pb::MigrateThing,
            _machine: &pb::MachineId,
            _host: pb::BuildInfo,
            _peer: pb::BuildInfo,
        ) -> Result<(), Status> {
            Ok(())
        }
        fn set_desk_folders(&self, _migrator: Arc<dyn crate::ports::FolderMigrator>) {}
        async fn resolved(&self, _thing: pb::MigrateThing, _machine: &pb::MachineId) -> Result<(), Status> {
            Ok(())
        }
    }

    struct NoopChannels;
    #[tonic::async_trait]
    impl crate::ports::DeskChannelPort for NoopChannels {
        async fn add_repo_to_channel(
            &self,
            _caller: &Caller,
            _req: pb::AddRepoRequest,
            _approved: bool,
        ) -> Result<crate::ports::Gated<pb::Operation>, Status> {
            Err(Status::unimplemented("unused"))
        }
        fn record_added(&self, _owner: &str, _name: &str, _place: &humata2_repo_library::Place) {}
        fn watch_aliases(&self) {}
    }

    struct NoopLiveEvents;
    #[tonic::async_trait]
    impl crate::ports::LiveEvents for NoopLiveEvents {
        async fn recv(&mut self) -> Result<pb::Event, tokio::sync::broadcast::error::RecvError> {
            Err(tokio::sync::broadcast::error::RecvError::Closed)
        }
        fn try_recv(&mut self) -> Result<pb::Event, tokio::sync::broadcast::error::TryRecvError> {
            Err(tokio::sync::broadcast::error::TryRecvError::Closed)
        }
    }

    struct NoopFeed;
    impl crate::ports::DeskFeedPort for NoopFeed {
        fn publish_in(
            &self,
            _change: pb::Change,
            _payload: pb::event::Payload,
            _scope: crate::ports::Scope,
            _by: Option<pb::Actor>,
        ) {
        }
        fn subscribe(&self, _after: Option<u64>) -> (u64, crate::ports::Events) {
            (0, crate::ports::Events::new(Vec::new(), Box::new(NoopLiveEvents)))
        }
    }

    struct NoopSessions;
    #[tonic::async_trait]
    impl crate::ports::DeskSessionPort for NoopSessions {
        async fn agent_ready(&self, _agents: &humata2_sessions::Agents, _agent: &str) -> Result<(), Status> {
            Ok(())
        }
        fn agent_enabled(&self, _agents: &humata2_sessions::Agents, _agent: &str) -> Result<(), Status> {
            Ok(())
        }
        fn pick_agent(
            &self,
            _agents: &humata2_sessions::Agents,
            _preferred: &[String],
        ) -> Result<String, Status> {
            Ok("claude".to_owned())
        }
        fn watch_agent(&self, _session: String) {}
        fn running_turn(&self, _session: &str, _holder_pid: u32) -> pb::SessionStatus {
            pb::SessionStatus::Working
        }
        fn moving(&self, _session: &str, _to: &str, _operation: &str) -> crate::ports::Moving {
            crate::ports::Moving::new(Box::new(()))
        }
        fn session_follows(
            &self,
            _store: &humata2_store::Store,
            _agents: &humata2_sessions::Agents,
            _home: &std::path::Path,
            _channels: &std::path::Path,
            _session: &str,
            _from: &std::path::Path,
            _to: &std::path::Path,
        ) -> Result<Vec<humata2_revive::FollowStep>, StoreError> {
            Ok(Vec::new())
        }
        async fn run_resume(
            &self,
            _by: pb::Actor,
            _loaded: crate::ports::Loaded,
            _progress: crate::ports::Progress,
        ) {
        }
    }

    struct ScriptedRemote {
        outcomes: std::sync::Mutex<std::collections::VecDeque<Result<pb::StartDeskSessionResponse, RemoteError>>>,
        seen_ids: std::sync::Mutex<Vec<String>>,
    }

    impl ScriptedRemote {
        fn new(outcomes: Vec<Result<pb::StartDeskSessionResponse, RemoteError>>) -> Arc<Self> {
            Arc::new(Self {
                outcomes: std::sync::Mutex::new(outcomes.into()),
                seen_ids: std::sync::Mutex::new(Vec::new()),
            })
        }
    }

    #[tonic::async_trait]
    impl crate::ports::DeskRemotePort for ScriptedRemote {
        fn knows(&self, _machine: &str) -> bool {
            true
        }
        fn machines_up(&self) -> Vec<String> {
            vec!["box-1".to_owned()]
        }
        fn relayed(&self, _machine: &str) -> Result<Arc<dyn crate::ports::RelayedClients>, RelayError> {
            Err(RelayError::Refused { answer: Status::unimplemented("unused") })
        }
        fn require_link(&self, _machine: &str) -> Result<Arc<dyn crate::ports::RelayedClients>, Status> {
            Err(Status::unimplemented("unused"))
        }
        async fn start_desk_on(
            &self,
            _machine: &str,
            id: &str,
            _req: pb::StartDeskSessionRequest,
            _channel_folder: Option<&str>,
            _host: Arc<HostOp>,
        ) -> Result<pb::StartDeskSessionResponse, RemoteError> {
            self.seen_ids.lock().unwrap().push(id.to_owned());
            self.outcomes
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Err(RemoteError::Failed { answer: Status::internal("exhausted") }))
        }
        async fn resume_session_on(
            &self,
            _machine: &str,
            _id: &str,
            _req: pb::ResumeSessionRequest,
            _host: Arc<HostOp>,
        ) -> Result<pb::Operation, Status> {
            Err(Status::unimplemented("unused"))
        }
        async fn wait_up(&self, _machine: &str) -> Result<(), RelayError> {
            Ok(())
        }
        fn watch_links(&self) -> crate::ports::LinkWatch {
            let (_tx, rx) = tokio::sync::mpsc::channel(1);
            crate::ports::LinkWatch::new(rx)
        }
    }

    fn test_svc(dir: &std::path::Path, remote: Arc<dyn crate::ports::DeskRemotePort>) -> (DeskSvc, Arc<humata2_store::Store>) {
        let store = Arc::new(humata2_store::Store::open(dir).unwrap());
        let starter = humata2_sessions::Starter {
            store: store.clone(),
            layout: humata2_sessions::Layout::new(dir.join("channels")),
            remote_template: String::new(),
            agents: humata2_sessions::Agents::builtin(),
            home: dir.to_path_buf(),
            revive: humata2_revive::ReviveConfig {
                holder: PathBuf::new(),
                logs: dir.join("logs"),
                scrollback_bytes: 1024,
                shell: PathBuf::from("/bin/sh"),
                shell_args: Vec::new(),
                home: dir.to_path_buf(),
                resume_timeout: Duration::from_secs(1),
                settle: Duration::from_secs(1),
                reports: humata2_revive::RestartReports {
                    folder: dir.join("restarts"),
                    version: String::new(),
                    machine: "ma_host".to_owned(),
                    os: String::new(),
                    daemon_log: None,
                },
                contain: None,
                builds: None,
                versions: None,
                gate: None,
            },
            holders: dir.join("holders"),
            first_message: humata2_sessions::FirstMessage::default(),
        };
        let svc = DeskSvc {
            guard: Arc::new(NoopGuard),
            operations: Arc::new(FakeOps),
            starter,
            identities: Arc::new(NoopIdentities),
            tokens: Arc::new(NoopTokens),
            inbox: Arc::new(NoopInbox),
            tell: Arc::new(NoopTell),
            machine: "ma_host".to_owned(),
            default_agent: "claude".to_owned(),
            bin: Arc::new(NoopBin),
            terminals: Arc::new(NoopTerminals),
            state: dir.to_path_buf(),
            locks: Arc::default(),
            wake: Arc::new(tokio::sync::Notify::new()),
            cleaned: Arc::default(),
            copied: Arc::default(),
            remote: Some(remote),
            migrations: Arc::new(NoopMigrations),
            channels: Arc::new(NoopChannels),
            restart: Arc::new(crate::ports::TestRestart::default()),
            feed: Arc::new(NoopFeed),
            sessions: Arc::new(NoopSessions),
        };
        (svc, store)
    }

    fn remote_plan(work: RemoteWork) -> Plan {
        Plan {
            machine: "box-1".to_owned(),
            work,
            kind: Kind::Plain {
                access: pb::DeskAccess::default(),
            },
            title: "Notes".to_owned(),
            agent: "claude".to_owned(),
        }
    }

    /// SMB-051: a transient Unavailable error from the remote start must keep
    /// the host session row and `Asked` journal so the retry under the same id
    /// settles the session instead of losing it.
    #[tokio::test(start_paused = true)]
    async fn a_transient_remote_start_error_keeps_the_session_and_journal_and_recovers_on_retry() {
        let dir = tempfile::tempdir().unwrap();
        let remote = ScriptedRemote::new(vec![
            Err(RemoteError::Failed {
                answer: Status::unavailable("link dropped after start"),
            }),
            Ok(pb::StartDeskSessionResponse {
                desk: Some(pb::DeskSession {
                    folder: Some(pb::DeskFolder {
                        path: "/remote/desk/notes".to_owned(),
                        kind: None,
                    }),
                    ..pb::DeskSession::default()
                }),
                ..pb::StartDeskSessionResponse::default()
            }),
        ]);
        let (svc, store) = test_svc(dir.path(), remote.clone());
        let caller = Caller::User(humata2_daemon_ports::UserVia::App);

        let err = svc
            .start_remote(&caller, remote_plan(remote.clone()), pb::StartDeskSessionRequest::default())
            .await
            .expect_err("first attempt returned Unavailable");
        assert_eq!(err.code(), Code::Unavailable);

        // Host row and Asked journal survive the transient failure.
        let sessions = store.sessions().unwrap();
        assert_eq!(sessions.len(), 1, "host session row must survive transient RPC error");
        let id = sessions[0].id.clone();
        let kept = super::super::remote_asked::read_all(&svc.asked_folder());
        assert_eq!(kept.len(), 1, "Asked journal must survive transient RPC error");

        // Let the spawned retry task register its timer, then advance paused
        // time so `ask_again` wakes and settles immediately.
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(2)).await;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !super::super::remote_asked::read_all(&svc.asked_folder()).is_empty() {
            assert!(
                std::time::Instant::now() < deadline,
                "background retry did not finish and drop the Asked journal in time"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
            std::thread::sleep(Duration::from_millis(2));
        }

        let seen = remote.seen_ids.lock().unwrap().clone();
        assert_eq!(seen, vec![id.clone(), id.clone()], "retry must re-ask under the same session id");
        let saved = store.session(&id).unwrap().expect("session row exists after retry");
        assert_eq!(saved.folder, PathBuf::from("/remote/desk/notes"));
        assert!(
            super::super::remote_asked::read_all(&svc.asked_folder()).is_empty(),
            "journal is dropped once the retry succeeds"
        );
    }

    /// SMB-051: an explicit pre-start refusal cleans up the host session row
    /// and drops the `Asked` journal immediately.
    #[tokio::test]
    async fn a_definitive_remote_start_refusal_removes_the_session_and_drops_the_journal() {
        let dir = tempfile::tempdir().unwrap();
        let remote = ScriptedRemote::new(vec![Err(RemoteError::Failed {
            answer: Status::invalid_argument("unknown agent"),
        })]);
        let (svc, store) = test_svc(dir.path(), remote.clone());
        let caller = Caller::User(humata2_daemon_ports::UserVia::App);

        let err = svc
            .start_remote(&caller, remote_plan(remote), pb::StartDeskSessionRequest::default())
            .await
            .expect_err("definitive refusal returns error");
        assert_eq!(err.code(), Code::InvalidArgument);
        assert!(store.sessions().unwrap().is_empty(), "host session row is removed on definitive refusal");
        assert!(
            super::super::remote_asked::read_all(&svc.asked_folder()).is_empty(),
            "Asked journal is dropped on definitive refusal"
        );
    }
}

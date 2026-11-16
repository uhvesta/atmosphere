//! Sessions on other machines, as the host's SessionService handles them (SL-1, SL-3,
//! PER-11, CH-4, HO-5; D139, D144, D150). The host writes its own record first, under an
//! id it chose, then makes the same call on the machine (`RemoteWork`); what the machine
//! reports later is merged into how the session shows. Every branch here runs after the
//! handler's own access check, because the machine honours a relayed call as the user's
//! and checks nothing more. On the machine, a start that names a session it already has
//! answers with that one.

use std::sync::Arc;

use humata2_grpc::humata::v1 as pb;
use humata2_sessions::StartRequest;
use humata2_store::{ChannelRecord, ChannelRepo, MainClone, SessionRecord, StartStep};
use tonic::{Code, Status};

use super::{COLS, Gated, Loaded, ROWS, SessionSvc, Tell, choose_repos, clean_title, start_error};
use crate::channel_service::blocking;
use crate::identity::{self, Caller};
use crate::operations::Started;
use crate::remote_work::{HostOp, RemoteView, RemoteWork, Sink};
use crate::status;

/// Whether `record` says its session lives on a machine other than `here`.
pub(super) fn elsewhere(record: &SessionRecord, here: &str) -> bool {
    !record.machine.is_empty() && record.machine != here
}

/// Publishes a remote session's changes on the host's event feed the way local ones are.
pub(crate) struct TellSink(pub(crate) Tell);

#[tonic::async_trait]
impl Sink for TellSink {
    async fn changed(&self, view: &RemoteView) {
        self.0.session(&view.session, pb::Change::Updated, None).await;
    }

    /// The host's record decides whether the session still exists; nothing to publish.
    async fn gone(&self, _machine: &str, _session: &str) {}
}

/// The failure an operation records for `status`.
fn failure_of(status: &Status) -> pb::Error {
    status::error_of(status).unwrap_or_else(|| pb::Error {
        message: status.message().to_owned(),
        fix: String::new(),
        retry: None,
        kind: None,
    })
}

impl SessionSvc {
    /// The machine a start names when it isn't this one and this daemon works with other
    /// machines (the host); none for this machine. A daemon without them answers as it
    /// always did.
    pub(super) fn remote_machine(
        &self,
        wanted: Option<&pb::MachineId>,
    ) -> Result<Option<(String, RemoteWork)>, Status> {
        let named = wanted
            .map(|machine| machine.value.as_str())
            .filter(|machine| !machine.is_empty() && *machine != self.machine);
        match (named, &self.remote) {
            (Some(machine), Some(remote)) => Ok(Some((machine.to_owned(), remote.clone()))),
            _ => self.this_machine(wanted).map(|()| None),
        }
    }

    /// The machine `record`'s session lives on, when it isn't this one and this daemon
    /// works with other machines.
    pub(super) fn lives_elsewhere(&self, record: &SessionRecord) -> Option<(String, RemoteWork)> {
        let remote = self.remote.as_ref()?;
        elsewhere(record, &self.machine).then(|| (record.machine.clone(), remote.clone()))
    }

    /// Puts what the machine last reported on a remote session as shown here (SL-3).
    pub(super) fn merge_remote(&self, record: &SessionRecord, shown: &mut pb::Session) {
        if let Some(remote) = &self.remote
            && elsewhere(record, &self.machine)
        {
            remote.merge(shown);
        }
    }

    /// StartSession on another machine: the record first (SL-1), then the same start
    /// there. An offline machine answers at once and leaves no record (CH-4); a start the
    /// machine refuses takes the record back.
    pub(super) async fn begin_remote(
        &self,
        caller: &Caller,
        request: pb::StartSessionRequest,
        channel: ChannelRecord,
        agent: String,
        machine: String,
        remote: RemoteWork,
    ) -> Result<Gated<pb::StartSessionResponse>, Status> {
        if let Some(operation) = self
            .operations
            .existing(caller, request.idempotency_key.as_ref())?
        {
            return Ok(Gated::Done(self.started_by(operation).await?));
        }
        let title = clean_title(&request.title)?;
        // The machine checks its own clones; here only the names matter.
        let named = ChannelRecord {
            repos: channel
                .repos
                .iter()
                .map(|repo| ChannelRepo {
                    main: MainClone::Ready,
                    ..repo.clone()
                })
                .collect(),
            ..channel.clone()
        };
        let repos = choose_repos(&named, &request.repos)?;
        remote.require_link(&machine).map_err(Status::from)?;

        let mut steps = vec![("write_record".to_owned(), "Saving the session".to_owned())];
        for (_, name) in &repos {
            steps.push((format!("clone:{name}"), format!("Cloning {name}")));
        }
        steps.push(("launch".to_owned(), "Starting the agent".to_owned()));
        steps.push(("verify".to_owned(), "Checking it started".to_owned()));
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Channel(pb::ChannelId {
                value: channel.id.clone(),
            })),
        };
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::StartSession,
            target,
            &steps,
            false,
            request.idempotency_key.clone(),
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => {
                return Ok(Gated::Done(self.started_by(operation).await?));
            }
        };
        let operation = progress.operation();
        let id = format!("se_{}", uuid::Uuid::new_v4().simple());
        let start = StartRequest {
            id: Some(id.clone()),
            channel: channel.id.clone(),
            machine: machine.clone(),
            title: if title.is_empty() {
                "Session".to_owned()
            } else {
                title
            },
            agent: Some(agent),
            model: Some(request.model.clone()).filter(|model| !model.is_empty()),
            repos: Some(repos),
            first_message: Some(request.first_message.clone()).filter(|message| !message.is_empty()),
            branch: Some(request.branch.clone()).filter(|branch| !branch.is_empty()),
            operation: operation.id.as_ref().map(|id| id.value.clone()),
            cols: COLS,
            rows: ROWS,
        };
        if let Err(error) = self.starter.place_session(&start).await {
            let (code, failure) = start_error(&error);
            progress.set_step("write_record", pb::StepState::Failed, Some(status::chain(&error)));
            progress.fail(failure.clone());
            return Err(status::failure(code, failure));
        }
        progress.set_step("write_record", pb::StepState::Done, None);
        let host = Arc::new(HostOp::new(progress));
        let asked = pb::StartSessionRequest {
            idempotency_key: None,
            ..request
        };
        match remote.start_session_on(&machine, &id, asked, host.clone()).await {
            Ok(response) => {
                self.tell
                    .session(&id, pb::Change::Created, Some(identity::actor(caller)))
                    .await;
                Ok(Gated::Done(response))
            }
            Err(error) => {
                let status = Status::from(error);
                if let Some(progress) = host.take() {
                    progress.fail(failure_of(&status));
                }
                let (store, removing) = (self.starter.store.clone(), id);
                let _ = blocking(move || store.remove_session(&removing)).await;
                Err(status)
            }
        }
    }

    /// The answer for a start that names a session this machine already has: the session,
    /// and its start's operation as it stands. None when it has no such session.
    pub(super) async fn known_start(&self, id: &str) -> Result<Option<pb::StartSessionResponse>, Status> {
        let loaded = match self.load(id).await {
            Ok(loaded) => loaded,
            Err(status) if status.code() == Code::NotFound => return Ok(None),
            Err(status) => return Err(status),
        };
        let done = loaded
            .detail
            .as_ref()
            .is_some_and(|detail| detail.start == StartStep::Done);
        let operation = pb::Operation {
            id: loaded
                .detail
                .as_ref()
                .and_then(|detail| detail.operation.clone())
                .map(|value| pb::OperationId { value }),
            kind: pb::OperationKind::StartSession as i32,
            target: Some(pb::EntityRef {
                entity: Some(pb::entity_ref::Entity::Channel(pb::ChannelId {
                    value: loaded.record.channel.clone(),
                })),
            }),
            state: if done {
                pb::OperationState::Succeeded
            } else {
                pb::OperationState::Running
            } as i32,
            result: Some(pb::operation::Result::Session(pb::SessionId {
                value: id.to_owned(),
            })),
            ..pb::Operation::default()
        };
        Ok(Some(pb::StartSessionResponse {
            session: Some(pb::SessionId { value: id.to_owned() }),
            operation: Some(operation),
        }))
    }

    /// StopSession for a session on another machine: the machine stops it and the answer
    /// is the session as the host shows it, with the machine's status.
    pub(super) async fn stop_remote(
        &self,
        id: &str,
        loaded: &Loaded,
        machine: String,
        remote: RemoteWork,
    ) -> Result<Gated<pb::Session>, Status> {
        let stopped = remote
            .stop_session_on(&machine, id)
            .await
            .map_err(Status::from)?;
        let mut shown = self.show(loaded);
        shown.status = stopped.status;
        shown.exit_code = stopped.exit_code;
        self.withdraw_requests_of(id, pb::WithdrawReason::SessionStopped)
            .await?;
        Ok(Gated::Done(shown))
    }

    /// ResumeSession for a session on another machine: the machine's operation is shown
    /// as the host's (launch, verify). An offline machine answers at once, queueing
    /// nothing (CH-4).
    pub(super) async fn resume_remote(
        &self,
        caller: &Caller,
        request: pb::ResumeSessionRequest,
        id: String,
        machine: String,
        remote: RemoteWork,
    ) -> Result<Gated<pb::Operation>, Status> {
        if let Some(operation) = self
            .operations
            .existing(caller, request.idempotency_key.as_ref())?
        {
            return Ok(Gated::Done(operation));
        }
        remote.require_link(&machine).map_err(Status::from)?;
        let steps = [
            ("launch".to_owned(), "Starting the agent again".to_owned()),
            (
                "verify".to_owned(),
                "Checking it took up its conversation".to_owned(),
            ),
        ];
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Session(pb::SessionId {
                value: id.clone(),
            })),
        };
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::StartSession,
            target,
            &steps,
            false,
            request.idempotency_key.clone(),
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(Gated::Done(operation)),
        };
        let started = progress.operation();
        let host = Arc::new(HostOp::new(progress));
        match remote
            .resume_session_on(&machine, &id, request, host.clone())
            .await
        {
            Ok(shown) => Ok(Gated::Done(shown.unwrap_or(started))),
            Err(error) => {
                let status = Status::from(error);
                if let Some(progress) = host.take() {
                    progress.fail(failure_of(&status));
                }
                Err(status)
            }
        }
    }
}

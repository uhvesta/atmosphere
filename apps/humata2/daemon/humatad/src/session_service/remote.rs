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
use crate::remote_work::{HostOp, RemoteError, RemoteView, RemoteWork, Sink};
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

/// The start the restart's report names as one it couldn't finish or remove:
/// what it says about `id`, which is what the person has to do. Only the
/// session's own line counts, not a shell tab of it.
pub(crate) fn stuck_start<'a>(last: &'a pb::LastStart, id: &str) -> Option<&'a str> {
    last.not_brought_back
        .iter()
        .find(|line| line.session == id && line.tab.is_empty())
        .map(|line| line.problem.as_str())
}

/// The status a machine's refusal of a start answers with. A machine that
/// made the session under another id than the host's has that session still:
/// nothing on the host points to it, so the answer names it and the machine
/// for the person to remove there, as a stop of it here could end someone
/// else's session of that id.
fn refusal(error: RemoteError) -> Status {
    match &error {
        RemoteError::IdNotHonoured { machine, got, .. } if !got.is_empty() => {
            let message = format!(
                "{error}; the session it made, {got:?}, is still on {machine} and nothing here points to it, so remove it on {machine} before you start again"
            );
            status::internal(message, "")
        }
        _ => Status::from(error),
    }
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
        // D432: the start's journal goes first, so a power cut anywhere
        // from the host's record to the machine's answer leaves a start to
        // ask again, never a record on the host that nothing runs.
        if let Err(status) = self
            .keep_asked(&super::remote_start::Asked::new(&progress, &id, &machine, &request))
            .await
        {
            progress.set_step("write_record", pb::StepState::Failed, Some(status.message().to_owned()));
            progress.fail(failure_of(&status));
            return Err(status);
        }
        if let Err(error) = self.starter.place_session(&start).await {
            self.drop_asked(&id).await;
            let (code, failure) = start_error(&error);
            progress.set_step("write_record", pb::StepState::Failed, Some(status::chain(&error)));
            progress.fail(failure.clone());
            return Err(status::failure(code, failure));
        }
        progress.set_step("write_record", pb::StepState::Done, None);
        // The machine makes the branch this record names, not one of its own
        // picking: its prefix may differ, and a restore here fetches the name
        // recorded here.
        let (store, placed, named) = (self.starter.store.clone(), id.clone(), id.clone());
        let read = blocking(move || store.session_detail(&placed)).await.and_then(|read| {
            read.map_err(|error| {
                status::internal(
                    format!("couldn't read the branch just recorded for session {named}: {}", status::chain(&error)),
                    "",
                )
            })
        });
        let branch = match read {
            Ok(detail) => detail
                .and_then(|detail| detail.repos.into_iter().next())
                .map(|repo| repo.branch),
            Err(status) => {
                progress.fail(failure_of(&status));
                self.drop_asked(&id).await;
                let (store, removing) = (self.starter.store.clone(), id);
                let _ = blocking(move || store.remove_session(&removing)).await;
                return Err(status);
            }
        };
        let host = Arc::new(HostOp::new(progress));
        let asked = pb::StartSessionRequest {
            idempotency_key: None,
            branch: branch.unwrap_or(request.branch.clone()),
            ..request
        };
        match remote
            .start_session_answered(&machine, &id, asked, host.clone(), Some(self.answered_hook(&id)))
            .await
        {
            Ok(response) => {
                self.tell
                    .session(&id, pb::Change::Created, Some(identity::actor(caller)))
                    .await;
                Ok(Gated::Done(response))
            }
            Err(error) => {
                let status = refusal(error);
                if let Some(progress) = host.take() {
                    progress.fail(failure_of(&status));
                }
                self.drop_asked(&id).await;
                let (store, removing) = (self.starter.store.clone(), id);
                let _ = blocking(move || store.remove_session(&removing)).await;
                Err(status)
            }
        }
    }

    /// Loads session `id` for a reader that may be the host following a start
    /// it asked for: a session whose start isn't recorded as done is the
    /// restart's recovery to finish or undo, so it is read once that is over.
    /// Answering earlier lets the host settle on a session that recovery then
    /// removes, leaving its record claiming a clone that is gone.
    pub(super) async fn load_after_recovery(&self, id: &str) -> Result<Loaded, Status> {
        let loaded = self.load(id).await?;
        let started = loaded
            .detail
            .as_ref()
            .is_some_and(|detail| detail.start == StartStep::Done);
        let Some(restart) = self.restart.get() else {
            return Ok(loaded);
        };
        if started {
            return Ok(loaded);
        }
        restart.wait_for(crate::restart::Awaited::Read).await?;
        self.load(id).await
    }

    /// The answer for a start that names a session this machine already has: the session,
    /// and its start's operation as it stands. None when it has no such session.
    pub(super) async fn known_start(&self, id: &str) -> Result<Option<pb::StartSessionResponse>, Status> {
        // A start asked again across this machine's restart (its answer was
        // lost with the link) is answered once recovery has finished or
        // undone it, never from a record recovery is about to remove.
        if let Some(started) = self.restart.get() {
            started.wait_for(crate::restart::Awaited::Start).await?;
        }
        let loaded = match self.load(id).await {
            Ok(loaded) => loaded,
            Err(status) if status.code() == Code::NotFound => return Ok(None),
            Err(status) => return Err(status),
        };
        let done = loaded
            .detail
            .as_ref()
            .is_some_and(|detail| detail.start == StartStep::Done);
        // A start the restart couldn't finish or remove stays as it was
        // saved; answered as still running, the host would wait on it
        // forever. It is answered as failed, with the restart's words.
        let stuck = if done {
            None
        } else {
            self.restart
                .get()
                .and_then(|started| started.get())
                .and_then(|last| stuck_start(last, id))
                .map(str::to_owned)
        };
        let state = match (done, &stuck) {
            (true, _) => pb::OperationState::Succeeded,
            (false, Some(_)) => pb::OperationState::Failed,
            (false, None) => pb::OperationState::Running,
        };
        let error = stuck.map(|problem| pb::Error {
            message: problem,
            fix: "remove the session's folder on this machine as the message says, then start the session again".to_owned(),
            retry: None,
            kind: None,
        });
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
            state: state as i32,
            error,
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
        // D757: the machine trusts an after-move resume from the host, so a
        // caller's own is passed on as the plain resume. The move asks the
        // machine itself, not through here.
        let mut request = request;
        if request.mode == pb::resume_session_request::Mode::AfterMove as i32 {
            request.mode = pb::resume_session_request::Mode::Resume as i32;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn line(session: &str, tab: &str, problem: &str) -> pb::NotBroughtBack {
        pb::NotBroughtBack {
            session: session.to_owned(),
            tab: tab.to_owned(),
            problem: problem.to_owned(),
        }
    }

    /// A session the restart couldn't finish or remove is named stuck, with
    /// the restart's own words; one it didn't name, or no restart, is not.
    #[test]
    fn a_start_the_restart_left_stuck_is_found_by_its_session() {
        let last = pb::LastStart {
            not_brought_back: vec![
                line("", "", "the store is damaged"),
                line("se_other", "", "other problem"),
                line("se_1", "", "delete its folder by hand"),
            ],
            ..pb::LastStart::default()
        };
        assert_eq!(stuck_start(&last, "se_1"), Some("delete its folder by hand"));
        assert_eq!(stuck_start(&last, "se_2"), None);
        assert_eq!(stuck_start(&pb::LastStart::default(), "se_1"), None);
    }

    /// A shell tab that didn't come back is not its session's stuck start.
    #[test]
    fn a_shell_tab_that_did_not_come_back_is_not_a_stuck_start() {
        let last = pb::LastStart {
            not_brought_back: vec![line("se_1", "tab_1", "shell gone")],
            ..pb::LastStart::default()
        };
        assert_eq!(stuck_start(&last, "se_1"), None);
    }

    /// A machine that made the session under another id leaves that session
    /// on it, so the refusal names it and the machine for the person to remove.
    #[test]
    fn a_machine_that_ignored_the_id_is_refused_naming_the_session_it_made() {
        let error = RemoteError::IdNotHonoured {
            machine: "devbox".to_owned(),
            asked: "se_1".to_owned(),
            got: "se_other".to_owned(),
        };
        let status = refusal(error);
        assert_eq!(status.code(), Code::Internal);
        assert!(status.message().contains("se_other"));
        assert!(status.message().contains("devbox"));
    }

    /// Any other refusal goes to the caller as the machine said it.
    #[test]
    fn another_refusal_is_the_machines_own_answer() {
        let source = Status::failed_precondition("no clone");
        let error = RemoteError::Machine {
            machine: "devbox".to_owned(),
            doing: "start the session",
            reason: "no clone".to_owned(),
            source,
        };
        assert_eq!(refusal(error).code(), Code::FailedPrecondition);
    }
}

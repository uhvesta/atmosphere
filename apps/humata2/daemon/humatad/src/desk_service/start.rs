//! StartDeskSession (DK-14, D182): checks the request, claims or checks the
//! folder, records the session and starts its agent. Refuses repos and
//! another machine before anything is made, and answers the same session
//! to a retry with the same idempotency key (DK-14).

use humata2_grpc::humata::v1 as pb;
use humata2_sessions::{DeskFolder, DeskStart};
use humata2_store::TabRecord;
use tonic::{Code, Status};

use super::view;
use super::{DeskSvc, failed_precondition, load, start_failure};
use crate::channel_service::{blocking, failed};
use crate::identity::{self, Caller};
use crate::operations::{Progress, Started};
use crate::session_service::{clean_title, watch_agent};
use crate::status;

/// The terminal size a Desk agent starts with, until a client attaches.
const COLS: u16 = 80;
const ROWS: u16 = 24;

/// The steps of a start, in order, as the operation shows them.
const STEPS: [(&str, &str); 3] = [
    ("write_record", "Saving the session"),
    ("launch", "Starting the agent"),
    ("verify", "Checking it started"),
];

impl DeskSvc {
    pub(super) async fn start(
        &self,
        caller: &Caller,
        request: pb::StartDeskSessionRequest,
    ) -> Result<pb::StartDeskSessionResponse, Status> {
        if let Some(machine) = request
            .machine
            .as_ref()
            .map(|machine| machine.value.as_str())
            .filter(|machine| !machine.is_empty())
            && machine != self.machine
        {
            return Err(failed_precondition(
                format!("machine {machine} isn't this one, and Desk sessions can only start on this machine until other machines are built (MACH), so nothing was started"),
                "leave machine unset, or name this machine, and start the Desk session again",
            ));
        }
        let title = clean_title(&request.title)?;
        let title = if title.is_empty() {
            "Session".to_owned()
        } else {
            title
        };
        let agent = self.agent(request.agent.as_ref())?;
        let folder = match request.picked_path.trim() {
            "" => DeskFolder::Scratch,
            path => DeskFolder::Picked(path.to_owned()),
        };
        let access = request.access.unwrap_or_default();

        if let Some(operation) = self
            .operations
            .existing(caller, request.idempotency_key.as_ref())?
        {
            return self.answered(operation).await;
        }
        let id = format!("se_{}", uuid::Uuid::new_v4().simple());
        let steps: Vec<(String, String)> = STEPS
            .iter()
            .map(|(key, title)| ((*key).to_owned(), (*title).to_owned()))
            .collect();
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
            request.idempotency_key,
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return self.answered(operation).await,
        };
        let operation = progress.operation();
        let start = DeskStart {
            id: id.clone(),
            title,
            agent,
            model: Some(request.model).filter(|model| !model.is_empty()),
            first_message: Some(request.first_message).filter(|message| !message.is_empty()),
            machine: self.machine.clone(),
            folder,
            state: self.state.clone(),
            repos: request
                .repos
                .iter()
                .map(|repo| (repo.owner.clone(), repo.name.clone()))
                .collect(),
            boards: access.boards,
            channels_and_machines: access.channels_and_machines,
            operation: operation.id.as_ref().map(|id| id.value.clone()),
            cols: COLS,
            rows: ROWS,
        };
        match self.run(start, &progress).await {
            Ok(()) => {
                progress.set_step("verify", pb::StepState::Done, None);
                progress.succeed(Some(pb::operation::Result::Session(pb::SessionId {
                    value: id.clone(),
                })));
                watch_agent(self.tell.clone(), id.clone());
                self.tell
                    .session(&id, pb::Change::Created, Some(identity::actor(caller)))
                    .await;
                self.answered_with(operation, &id).await
            }
            Err((code, error)) => {
                let step = if code == Code::Internal { "write_record" } else { "launch" };
                progress.set_step(step, pb::StepState::Failed, Some(error.message.clone()));
                progress.fail(error.clone());
                Err(status::failure(code, error))
            }
        }
    }

    /// Runs the start on the starter, moving the operation's steps as it
    /// goes. The error is what the app shows.
    async fn run(&self, start: DeskStart, progress: &Progress) -> Result<(), (Code, pb::Error)> {
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
        let started = self.starter.start_desk(start, &env, &mut note).await;
        started.map(|_| ()).map_err(|error| start_failure(&error))
    }

    /// The response for a start that already happened: its session, its
    /// operation and its Desk row.
    async fn answered(&self, operation: pb::Operation) -> Result<pb::StartDeskSessionResponse, Status> {
        let key = operation
            .id
            .as_ref()
            .map(|id| id.value.clone())
            .unwrap_or_default();
        let store = self.starter.store.clone();
        let session = blocking(move || -> Result<Option<String>, humata2_store::StoreError> {
            Ok(store
                .session_details()?
                .into_iter()
                .find(|detail| detail.operation.as_deref() == Some(key.as_str()))
                .map(|detail| detail.session))
        })
        .await?
        .map_err(|error| status::store(error, &self.machine, "find the Desk session being started"))?;
        let session = session.ok_or_else(|| {
            status::internal(
                "the operation that started a Desk session has no session recorded",
                "",
            )
        })?;
        self.answered_with(operation, &session).await
    }

    async fn answered_with(
        &self,
        operation: pb::Operation,
        session: &str,
    ) -> Result<pb::StartDeskSessionResponse, Status> {
        let (record, row) = load(&self.starter.store, &self.machine, session).await?;
        Ok(pb::StartDeskSessionResponse {
            session: Some(pb::SessionId {
                value: session.to_owned(),
            }),
            operation: Some(operation),
            desk: Some(view::desk_pb(&record, &row)),
        })
    }

    /// The agent a start runs: the one named, else `--default-agent`, which
    /// must be one Settings › Agents knows.
    fn agent(&self, wanted: Option<&pb::AgentId>) -> Result<String, Status> {
        let name = wanted
            .map(|agent| agent.value.clone())
            .filter(|agent| !agent.is_empty())
            .unwrap_or_else(|| self.default_agent.clone());
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
}



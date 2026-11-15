//! OperationService (GRPC.md §7): the long work this daemon runs, each an
//! `Operation` that changes as its steps do. A watcher holds one stream and
//! gets every change as it happens; nothing asks again (TR-7). Operations
//! live in memory: after a restart the work they did is finished or undone
//! by its own owner (machines' `recover`, PER-27), not resumed from here.
//! Cancel and Retry aren't built: a failed add was already undone, and
//! Try again is a fresh add.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use humata2_grpc::humata::v1 as pb;
use humata2_access::Refusal;
use humata2_machines::{OpStep, StepEvent};
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use crate::attach::now;
use crate::identity::{self, Caller};
use crate::status;

/// Who may see an operation: the user sees all, an agent its session's.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Owner {
    User,
    Session(String),
}

impl Owner {
    fn of(caller: &Caller) -> Result<Self, Status> {
        match caller {
            Caller::User(_) => Ok(Self::User),
            Caller::Agent(agent) => Ok(Self::Session(agent.session.id.as_str().to_owned())),
            Caller::Peer => Err(status::refused(&Refusal::PeerCannotCall)),
        }
    }

    fn sees(&self, owner: &Self) -> bool {
        *self == Self::User || self == owner
    }
}

struct Tracked {
    owner: Owner,
    operation: watch::Sender<pb::Operation>,
}

#[derive(Default)]
struct Table {
    by_id: BTreeMap<String, Arc<Tracked>>,
    by_key: HashMap<String, String>,
}

/// Every operation since this daemon started.
#[derive(Default)]
pub(crate) struct Operations {
    table: Mutex<Table>,
}

/// What [`Operations::start`] found.
pub(crate) enum Started {
    New(Progress),
    /// A call with this idempotency key already started one.
    Existing(pb::Operation),
}

impl Operations {
    /// Starts an operation of `kind` on `target` with `steps`, all waiting,
    /// for `caller`. A key already used returns that operation instead.
    pub(crate) fn start(
        &self,
        caller: &Caller,
        kind: pb::OperationKind,
        target: pb::EntityRef,
        steps: &[OpStep],
        key: Option<pb::IdempotencyKey>,
    ) -> Result<Started, Status> {
        let owner = Owner::of(caller)?;
        let key = key.filter(|key| !key.value.is_empty());
        let mut table = lock(&self.table);
        if let Some(key) = &key
            && let Some(id) = table.by_key.get(&key.value)
            && let Some(tracked) = table.by_id.get(id)
            && tracked.owner == owner
        {
            return Ok(Started::Existing(tracked.operation.borrow().clone()));
        }
        let id = format!("op_{}", uuid::Uuid::new_v4().simple());
        let operation = pb::Operation {
            id: Some(pb::OperationId { value: id.clone() }),
            kind: kind as i32,
            target: Some(target),
            state: pb::OperationState::Running as i32,
            steps: steps.iter().map(|&step| waiting(step)).collect(),
            error: None,
            cancellable: false,
            idempotency_key: key.clone(),
            started_at: Some(now()),
            finished_at: None,
            started_by: Some(actor(caller)),
            version: 1,
            result: None,
        };
        let (sender, _) = watch::channel(operation);
        let tracked = Arc::new(Tracked {
            owner,
            operation: sender,
        });
        table.by_id.insert(id.clone(), tracked.clone());
        if let Some(key) = key {
            table.by_key.insert(key.value, id);
        }
        Ok(Started::New(Progress { tracked }))
    }

    /// The operation a call with `key` from this caller already started.
    pub(crate) fn existing(
        &self,
        caller: &Caller,
        key: Option<&pb::IdempotencyKey>,
    ) -> Result<Option<pb::Operation>, Status> {
        let owner = Owner::of(caller)?;
        let Some(key) = key.filter(|key| !key.value.is_empty()) else {
            return Ok(None);
        };
        let table = lock(&self.table);
        Ok(table
            .by_key
            .get(&key.value)
            .and_then(|id| table.by_id.get(id))
            .filter(|tracked| tracked.owner == owner)
            .map(|tracked| tracked.operation.borrow().clone()))
    }

    fn find(&self, caller: &Caller, id: &str) -> Result<Arc<Tracked>, Status> {
        let owner = Owner::of(caller)?;
        lock(&self.table)
            .by_id
            .get(id)
            .filter(|tracked| owner.sees(&tracked.owner))
            .cloned()
            .ok_or_else(|| no_operation(id))
    }
}

/// The handle the work updates its operation through.
pub(crate) struct Progress {
    tracked: Arc<Tracked>,
}

impl Progress {
    pub(crate) fn operation(&self) -> pb::Operation {
        self.tracked.operation.borrow().clone()
    }

    /// Applies a step's progress. A step the kind doesn't list (an undo's)
    /// is added at the end, so the list says what was done.
    pub(crate) fn step(&self, event: &StepEvent) {
        let (step, state, tail) = match event {
            StepEvent::Running(step) | StepEvent::Retrying { step, .. } => {
                (*step, pb::StepState::Running, None)
            }
            StepEvent::Done(step) => (*step, pb::StepState::Done, None),
            StepEvent::Skipped(step) => (*step, pb::StepState::Skipped, None),
            StepEvent::Failed { step, output_tail } => {
                (*step, pb::StepState::Failed, Some(output_tail.clone()))
            }
            StepEvent::Undone(step) => (*step, pb::StepState::Undone, None),
        };
        self.change(|operation| {
            let index = match operation.steps.iter().position(|shown| shown.key == step.key()) {
                Some(index) => index,
                None => {
                    operation.steps.push(waiting(step));
                    operation.steps.len() - 1
                }
            };
            let shown = &mut operation.steps[index];
            shown.state = state as i32;
            if let Some(tail) = tail {
                shown.output_tail = tail;
            }
        });
    }

    pub(crate) fn succeed(self, result: Option<pb::operation::Result>) {
        self.change(|operation| {
            operation.state = pb::OperationState::Succeeded as i32;
            operation.finished_at = Some(now());
            operation.result = result;
        });
    }

    pub(crate) fn fail(self, error: pb::Error) {
        self.change(|operation| {
            operation.state = pb::OperationState::Failed as i32;
            operation.finished_at = Some(now());
            operation.error = Some(error);
        });
    }

    fn change(&self, edit: impl FnOnce(&mut pb::Operation)) {
        self.tracked.operation.send_modify(|operation| {
            edit(operation);
            operation.version += 1;
        });
    }
}

fn waiting(step: OpStep) -> pb::OperationStep {
    pb::OperationStep {
        key: step.key().to_owned(),
        title: step.title().to_owned(),
        state: pb::StepState::Waiting as i32,
        done_units: 0,
        total_units: 0,
        output_tail: String::new(),
    }
}

fn actor(caller: &Caller) -> pb::Actor {
    use pb::actor::Actor;
    let actor = match caller {
        Caller::User(_) => Actor::User(pb::UserActor {
            via: pb::user_actor::Via::Unspecified as i32,
            machine: None,
        }),
        Caller::Agent(agent) => Actor::Agent(pb::AgentActor {
            session: Some(pb::SessionId {
                value: agent.session.id.as_str().to_owned(),
            }),
            machine: Some(pb::MachineId {
                value: agent.session.machine.as_str().to_owned(),
            }),
            agent: Some(pb::AgentId {
                value: agent.agent.clone(),
            }),
            session_title: agent.title.clone(),
        }),
        Caller::Peer => Actor::System(pb::SystemActor { reason: 0 }),
    };
    pb::Actor { actor: Some(actor) }
}

fn finished(operation: &pb::Operation) -> bool {
    operation.state != pb::OperationState::Running as i32
}

fn no_operation(id: &str) -> Status {
    status::not_found(
        pb::EntityKind::Operation,
        id,
        format!("no operation {id} on this daemon"),
        "operations are kept only until humatad2 restarts; list the machines or sessions to see how the work ended",
    )
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) struct OperationSvc {
    operations: Arc<Operations>,
}

impl OperationSvc {
    pub(crate) fn new(operations: Arc<Operations>) -> Self {
        Self { operations }
    }
}

fn id_of(id: Option<pb::OperationId>) -> Result<String, Status> {
    id.map(|id| id.value)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| status::missing("id"))
}

#[tonic::async_trait]
impl pb::operation_service_server::OperationService for OperationSvc {
    async fn get_operation(
        &self,
        request: Request<pb::GetOperationRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = identity::caller(&request)?;
        let id = id_of(request.into_inner().id)?;
        let tracked = self.operations.find(&caller, &id)?;
        Ok(Response::new(tracked.operation.borrow().clone()))
    }

    async fn find_operation(
        &self,
        request: Request<pb::FindOperationRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = identity::caller(&request)?;
        let key = request
            .into_inner()
            .idempotency_key
            .map(|key| key.value)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| status::missing("idempotency_key"))?;
        let id = lock(&self.operations.table).by_key.get(&key).cloned();
        let missing = || {
            status::not_found(
                pb::EntityKind::Operation,
                &key,
                format!("no operation was started with idempotency key {key} on this daemon"),
                "the call may not have reached humatad2, or it restarted since; send the call again with the same key",
            )
        };
        let id = id.ok_or_else(missing)?;
        let tracked = self.operations.find(&caller, &id).map_err(|_| missing())?;
        Ok(Response::new(tracked.operation.borrow().clone()))
    }

    async fn list_operations(
        &self,
        request: Request<pb::ListOperationsRequest>,
    ) -> Result<Response<pb::ListOperationsResponse>, Status> {
        let caller = identity::caller(&request)?;
        let owner = Owner::of(&caller)?;
        let request = request.into_inner();
        let operations = lock(&self.operations.table)
            .by_id
            .values()
            .filter(|tracked| owner.sees(&tracked.owner))
            .map(|tracked| tracked.operation.borrow().clone())
            .filter(|operation| !request.running_only || !finished(operation))
            .filter(|operation| request.target.as_ref().is_none_or(|target| operation.target.as_ref() == Some(target)))
            .collect();
        Ok(Response::new(pb::ListOperationsResponse {
            operations,
            page: None,
        }))
    }

    type WatchOperationStream = ReceiverStream<Result<pb::Operation, Status>>;

    /// The operation as it is (when newer than `from_version`), then each
    /// change as it is made, then the stream ends with the operation. A
    /// watcher that falls behind gets the latest state, not every step
    /// between.
    async fn watch_operation(
        &self,
        request: Request<pb::WatchOperationRequest>,
    ) -> Result<Response<Self::WatchOperationStream>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let tracked = self.operations.find(&caller, &id_of(request.id)?)?;
        let mut changes = tracked.operation.subscribe();
        let (sender, receiver) = mpsc::channel(4);
        let mut from = request.from_version;
        tokio::spawn(async move {
            loop {
                let operation = changes.borrow_and_update().clone();
                if operation.version > from {
                    from = operation.version;
                    let done = finished(&operation);
                    if sender.send(Ok(operation)).await.is_err() || done {
                        return;
                    }
                } else if finished(&operation) {
                    return;
                }
                tokio::select! {
                    changed = changes.changed() => if changed.is_err() { return },
                    () = sender.closed() => return,
                }
            }
        });
        Ok(Response::new(ReceiverStream::new(receiver)))
    }

    async fn cancel_operation(
        &self,
        _request: Request<pb::CancelOperationRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        Err(status::not_built("humata.v1.OperationService.CancelOperation"))
    }

    async fn retry_operation(
        &self,
        _request: Request<pb::RetryOperationRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        Err(status::not_built("humata.v1.OperationService.RetryOperation"))
    }
}

//! OperationService (GRPC.md §7): the long work this daemon runs, each an
//! `Operation` that changes as its steps do. A watcher holds one stream and
//! gets every change as it happens; nothing asks again (TR-7). Operations
//! live in memory: after a restart the work they did is finished or undone
//! by its own owner (machines' `recover`, PER-27), not resumed from here.
//! Cancel asks the work to stop at its next step boundary; the work
//! undoes what it did and ends the operation cancelled. Retry runs a
//! failed operation's work again under the same id (D81). Each start and
//! each change goes on the event feed too (D94).

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use humata2_access::Refusal;
use humata2_grpc::humata::v1 as pb;
use humata2_machines::{Facts, OpStep, StepEvent};
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Code, Request, Response, Status};

use crate::attach::now;
use crate::events::{Feed, Scope};
use crate::identity::{self, Caller};
use crate::status;

/// Who may see an operation: the user sees all, an agent its session's,
/// a script those its token started (SEC-14).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Owner {
    User,
    Session(String),
    Script(String),
}

impl Owner {
    fn of(caller: &Caller) -> Result<Self, Status> {
        match caller {
            Caller::User(_) => Ok(Self::User),
            Caller::Agent(agent) => Ok(Self::Session(agent.session.id.as_str().to_owned())),
            Caller::Script(script) => Ok(Self::Script(script.id.clone())),
            Caller::Peer => Err(status::refused(&Refusal::PeerCannotCall)),
        }
    }

    fn sees(&self, owner: &Self) -> bool {
        *self == Self::User || self == owner
    }
}

impl Owner {
    /// The owner as kept on disk for [`Operations::restore`].
    fn label(&self) -> String {
        match self {
            Self::User => "user".to_owned(),
            Self::Session(id) => format!("session:{id}"),
            Self::Script(id) => format!("script:{id}"),
        }
    }

    fn from_label(label: &str) -> Option<Self> {
        match label.split_once(':') {
            None if label == "user" => Some(Self::User),
            Some(("session", id)) => Some(Self::Session(id.to_owned())),
            Some(("script", id)) => Some(Self::Script(id.to_owned())),
            _ => None,
        }
    }
}

/// Whose an operation is and where the feed files it, as work keeps them on
/// disk to bring the operation back after a restart ([`Operations::restore`]).
#[derive(Clone, Debug)]
pub(crate) struct Origin {
    pub(crate) owner: String,
    pub(crate) scope: Option<(String, String)>,
}

/// Runs an operation's work again with the handle given (Retry).
pub(crate) type Rerun = Arc<dyn Fn(Progress) + Send + Sync>;

struct Tracked {
    owner: Owner,
    /// Where the feed files it: the starting agent's channel and session;
    /// the user's belong nowhere.
    scope: Option<(String, String)>,
    feed: Arc<Feed>,
    operation: watch::Sender<pb::Operation>,
    /// The steps the kind lists, for a rerun to start from.
    steps: Vec<OpStep>,
    /// Whether the kind can be cancelled while it runs (GRPC.md §7).
    cancellable: bool,
    cancel: Mutex<Cancel>,
    rerun: Mutex<Option<Rerun>>,
    /// The key of the retry that started the current run.
    retry_key: Mutex<Option<String>>,
}

/// Whether CancelOperation asked the run to stop, and whether the run is
/// past its point of no return; one lock, so a cancel never slips in
/// between the run's last check and that point.
#[derive(Default)]
struct Cancel {
    asked: bool,
    past_no_return: bool,
}

#[derive(Default)]
struct Table {
    by_id: BTreeMap<String, Arc<Tracked>>,
    by_key: HashMap<String, String>,
}

/// Every operation since this daemon started.
pub(crate) struct Operations {
    table: Mutex<Table>,
    feed: Arc<Feed>,
}

/// What [`Operations::start`] found.
pub(crate) enum Started {
    New(Progress),
    /// A call with this idempotency key already started one.
    Existing(pb::Operation),
}

impl Operations {
    /// The event feed operations publish on, for the services that hold
    /// `Operations` to publish their own changes (sessions, channels).
    pub(crate) fn feed(&self) -> Arc<Feed> {
        self.feed.clone()
    }

    pub(crate) fn new(feed: Arc<Feed>) -> Self {
        Self {
            table: Mutex::new(Table::default()),
            feed,
        }
    }

    /// Starts an operation of `kind` on `target` with `steps`, all waiting,
    /// for `caller`. A key already used returns that operation instead.
    /// `cancellable` says whether the work stops when asked to; work that
    /// is cancellable checks [`Progress::cancelled`] between its steps.
    pub(crate) fn start(
        &self,
        caller: &Caller,
        kind: pb::OperationKind,
        target: pb::EntityRef,
        steps: &[OpStep],
        cancellable: bool,
        key: Option<pb::IdempotencyKey>,
    ) -> Result<Started, Status> {
        let shown = steps.iter().map(|&step| waiting(step)).collect();
        self.start_with(
            caller,
            kind,
            target,
            shown,
            steps.to_vec(),
            cancellable,
            key,
        )
    }

    /// As [`Operations::start`], for work whose steps are named by the
    /// work itself, as `(key, title)`. Such an operation has no steps for
    /// a rerun to start from, so Retry answers that it can't be tried
    /// again. [`Progress::set_step`] changes them.
    pub(crate) fn start_steps(
        &self,
        caller: &Caller,
        kind: pb::OperationKind,
        target: pb::EntityRef,
        steps: &[(String, String)],
        cancellable: bool,
        key: Option<pb::IdempotencyKey>,
    ) -> Result<Started, Status> {
        let shown = steps
            .iter()
            .map(|(key, title)| waiting_named(key, title))
            .collect();
        self.start_with(caller, kind, target, shown, Vec::new(), cancellable, key)
    }

    #[allow(clippy::too_many_arguments)]
    fn start_with(
        &self,
        caller: &Caller,
        kind: pb::OperationKind,
        target: pb::EntityRef,
        steps: Vec<pb::OperationStep>,
        rerun_steps: Vec<OpStep>,
        cancellable: bool,
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
            steps,
            error: None,
            cancellable,
            idempotency_key: key.clone(),
            started_at: Some(now()),
            finished_at: None,
            started_by: Some(identity::actor(caller)),
            version: 1,
            result: None,
            pull_requests: Vec::new(),
        };
        let scope = match caller {
            Caller::Agent(agent) => Some((
                agent.session.channel.to_string(),
                agent.session.id.to_string(),
            )),
            _ => None,
        };
        let (sender, _) = watch::channel(operation.clone());
        let tracked = Arc::new(Tracked {
            owner,
            scope,
            feed: self.feed.clone(),
            operation: sender,
            steps: rerun_steps,
            cancellable,
            cancel: Mutex::default(),
            rerun: Mutex::new(None),
            retry_key: Mutex::new(None),
        });
        table.by_id.insert(id.clone(), tracked.clone());
        if let Some(key) = key {
            table.by_key.insert(key.value, id);
        }
        let by = operation.started_by.clone();
        tracked.tell(pb::Change::Created, operation, by);
        Ok(Started::New(Progress { tracked }))
    }

    /// Brings back an operation a restart lost, under its own id, as it was
    /// kept (running, with the steps it had): the work that kept it goes on
    /// and ends it through the returned handle. One this daemon already has
    /// is returned as it is. `None` when `operation` has no id or `origin`
    /// names no owner.
    pub(crate) fn restore(&self, operation: pb::Operation, origin: &Origin) -> Option<Progress> {
        let id = operation.id.as_ref().map(|id| id.value.clone())?;
        let owner = Owner::from_label(&origin.owner)?;
        let mut table = lock(&self.table);
        if let Some(known) = table.by_id.get(&id) {
            return Some(Progress {
                tracked: known.clone(),
            });
        }
        let (sender, _) = watch::channel(operation.clone());
        let tracked = Arc::new(Tracked {
            owner,
            scope: origin.scope.clone(),
            feed: self.feed.clone(),
            operation: sender,
            steps: Vec::new(),
            cancellable: operation.cancellable,
            cancel: Mutex::default(),
            rerun: Mutex::new(None),
            retry_key: Mutex::new(None),
        });
        table.by_id.insert(id.clone(), tracked.clone());
        if let Some(key) = operation.idempotency_key.as_ref().filter(|key| !key.value.is_empty()) {
            table.by_key.insert(key.value.clone(), id);
        }
        let by = operation.started_by.clone();
        tracked.tell(pb::Change::Created, operation, by);
        Some(Progress { tracked })
    }

    /// Whether operation `id` is known here and still running. One a
    /// restart lost, or that has ended, is not.
    pub(crate) fn running(&self, id: &str) -> bool {
        let table = lock(&self.table);
        let Some(tracked) = table.by_id.get(id) else {
            return false;
        };
        tracked.operation.borrow().state == pb::OperationState::Running as i32
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

    /// The handle to an operation `caller` may see, for work that finishes
    /// it from elsewhere (the interactive login ends a create that stopped
    /// needing one).
    pub(crate) fn progress_of(&self, caller: &Caller, id: &str) -> Result<Progress, Status> {
        Ok(Progress {
            tracked: self.find(caller, id)?,
        })
    }
}

/// The handle the work updates its operation through.
pub(crate) struct Progress {
    tracked: Arc<Tracked>,
}

/// Reports a step's progress in units and facts, for work that runs on other
/// threads than the one that owns the [`Progress`].
#[derive(Clone)]
pub(crate) struct Units(Arc<Tracked>);

impl Units {
    pub(crate) fn set(&self, key: &str, done: u64, total: u64) {
        Progress { tracked: self.0.clone() }.set_units(key, done, total);
    }

    pub(crate) fn add_fact(&self, key: &str, name: &str, more: u64) {
        Progress { tracked: self.0.clone() }.add_fact(key, name, more);
    }
}

impl Progress {
    pub(crate) fn operation(&self) -> pb::Operation {
        self.tracked.operation.borrow().clone()
    }

    /// Whose the operation is, for work that keeps it on disk to bring it
    /// back after a restart.
    pub(crate) fn origin(&self) -> Origin {
        Origin {
            owner: self.tracked.owner.label(),
            scope: self.tracked.scope.clone(),
        }
    }

    /// The operation's changes, for the work that hands this handle to
    /// another task and waits for the operation to end.
    pub(crate) fn watch(&self) -> watch::Receiver<pb::Operation> {
        self.tracked.operation.subscribe()
    }

    /// True once CancelOperation asked this run to stop.
    pub(crate) fn cancelled(&self) -> bool {
        lock(&self.tracked.cancel).asked
    }

    /// The run is at its point of no return (GRPC.md §7): from here
    /// CancelOperation answers `not_cancellable`. False when a cancel was
    /// asked first; the run then stops and undoes instead.
    pub(crate) fn past_no_return(&self) -> bool {
        let mut cancel = lock(&self.tracked.cancel);
        if cancel.asked {
            return false;
        }
        cancel.past_no_return = true;
        drop(cancel);
        self.change(|operation| operation.cancellable = false);
        true
    }

    /// How RetryOperation runs this operation's work again.
    pub(crate) fn rerun_with(&self, rerun: Rerun) {
        *lock(&self.tracked.rerun) = Some(rerun);
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
            StepEvent::Checked(facts) => {
                self.set_facts(OpStep::Check, check_facts(facts));
                return;
            }
        };
        self.change(|operation| {
            let index = match operation
                .steps
                .iter()
                .position(|shown| shown.key == step.key())
            {
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

    /// Sets what `step` read, adding the step at the end when the
    /// operation doesn't list it yet.
    fn set_facts(&self, step: OpStep, facts: HashMap<String, String>) {
        self.change(|operation| {
            let index = match operation.steps.iter().position(|shown| shown.key == step.key()) {
                Some(index) => index,
                None => {
                    operation.steps.push(waiting(step));
                    operation.steps.len() - 1
                }
            };
            operation.steps[index].facts = facts;
        });
    }

    /// Sets the step `key`, adding it at the end when the operation
    /// doesn't list it yet (its title is then the key). `output_tail`
    /// replaces what the step showed when given.
    pub(crate) fn set_step(&self, key: &str, state: pb::StepState, output_tail: Option<String>) {
        self.change(|operation| {
            let index = match operation.steps.iter().position(|shown| shown.key == key) {
                Some(index) => index,
                None => {
                    operation.steps.push(waiting_named(key, key));
                    operation.steps.len() - 1
                }
            };
            let shown = &mut operation.steps[index];
            shown.state = state as i32;
            if let Some(tail) = output_tail {
                shown.output_tail = tail;
            }
        });
    }

    /// How far step `key` is, in units: neither count goes down, and done
    /// never passes total (GRPC.md section 7).
    fn set_units(&self, key: &str, done: u64, total: u64) {
        self.change(|operation| {
            if let Some(shown) = operation.steps.iter_mut().find(|shown| shown.key == key) {
                shown.total_units = shown.total_units.max(total);
                shown.done_units = shown.done_units.max(done).min(shown.total_units);
            }
        });
    }

    /// Adds `more` to the decimal fact `name` of step `key`.
    fn add_fact(&self, key: &str, name: &str, more: u64) {
        self.change(|operation| {
            if let Some(shown) = operation.steps.iter_mut().find(|shown| shown.key == key) {
                let before = shown.facts.get(name).and_then(|fact| fact.parse::<u64>().ok()).unwrap_or(0);
                shown.facts.insert(name.to_owned(), before.saturating_add(more).to_string());
            }
        });
    }

    /// A handle that reports a step's units from the threads doing its work.
    pub(crate) fn units(&self) -> Units {
        Units(self.tracked.clone())
    }

    pub(crate) fn succeed(self, result: Option<pb::operation::Result>) {
        self.change(|operation| {
            operation.state = pb::OperationState::Succeeded as i32;
            operation.cancellable = false;
            operation.finished_at = Some(now());
            operation.result = result;
        });
    }

    /// Ends the operation as succeeded with the pull requests a push opened
    /// or found, one for each repository.
    pub(crate) fn succeed_with_pull_requests(self, pull_requests: Vec<pb::PullRequestRef>) {
        self.change(|operation| {
            operation.state = pb::OperationState::Succeeded as i32;
            operation.cancellable = false;
            operation.finished_at = Some(now());
            operation.pull_requests = pull_requests;
        });
    }

    pub(crate) fn fail(self, error: pb::Error) {
        self.change(|operation| {
            operation.state = pb::OperationState::Failed as i32;
            operation.cancellable = false;
            operation.finished_at = Some(now());
            operation.error = Some(error);
        });
    }

    /// The work stopped as asked and undid what it had done. A step it was
    /// in the middle of is shown as not run.
    pub(crate) fn end_cancelled(self) {
        self.change(|operation| {
            operation.state = pb::OperationState::Cancelled as i32;
            operation.cancellable = false;
            operation.finished_at = Some(now());
            for step in &mut operation.steps {
                if step.state == pb::StepState::Running as i32 {
                    step.state = pb::StepState::Waiting as i32;
                }
            }
        });
    }

    fn change(&self, edit: impl FnOnce(&mut pb::Operation)) {
        // Told while the operation is held, so the feed has its changes
        // in the order they were made.
        self.tracked.operation.send_modify(|operation| {
            edit(operation);
            operation.version += 1;
            self.tracked
                .tell(pb::Change::Updated, operation.clone(), None);
        });
    }
}

impl Tracked {
    /// `by` is the caller that started it; the work's own steps are the
    /// daemon's.
    fn tell(&self, change: pb::Change, operation: pb::Operation, by: Option<pb::Actor>) {
        let payload = pb::event::Payload::Operation(operation);
        match &self.scope {
            Some((channel, session)) => self.feed.publish_in(
                change,
                payload,
                Scope {
                    channel: channel.clone(),
                    session: session.clone(),
                },
                by,
            ),
            None => self.feed.publish(change, payload, by),
        }
    }
}

/// What the check read, as the `check` step's facts (HO-1): the names
/// GRPC.md lists; a fact that isn't known is left out.
fn check_facts(facts: &Facts) -> HashMap<String, String> {
    let mut named = HashMap::new();
    named.insert("os".to_owned(), facts.platform.os.to_string());
    named.insert("arch".to_owned(), facts.platform.arch.to_string());
    for (name, version) in [("git", facts.git_version()), ("gh", facts.gh_version())] {
        if !version.is_empty() {
            named.insert(name.to_owned(), version.to_owned());
        }
    }
    if let Some(kib) = facts.disk_free_kib {
        named.insert("disk_free_bytes".to_owned(), kib.saturating_mul(1024).to_string());
    }
    named
}

fn waiting(step: OpStep) -> pb::OperationStep {
    waiting_named(step.key(), step.title())
}

fn waiting_named(key: &str, title: &str) -> pb::OperationStep {
    pb::OperationStep {
        key: key.to_owned(),
        title: title.to_owned(),
        state: pb::StepState::Waiting as i32,
        done_units: 0,
        total_units: 0,
        output_tail: String::new(),
        facts: HashMap::new(),
    }
}

fn state(operation: &pb::Operation) -> pb::OperationState {
    pb::OperationState::try_from(operation.state).unwrap_or(pb::OperationState::Unspecified)
}

/// The title of the step the operation is in, for `not_cancellable`.
fn current_step(operation: &pb::Operation) -> String {
    operation
        .steps
        .iter()
        .find(|step| step.state == pb::StepState::Running as i32)
        .or_else(|| {
            operation
                .steps
                .iter()
                .find(|step| step.state == pb::StepState::Waiting as i32)
        })
        .map(|step| step.title.clone())
        .unwrap_or_default()
}

fn not_cancellable(id: &str, operation: &pb::Operation) -> Status {
    status::failure(
        Code::FailedPrecondition,
        status::error(
            format!("operation {id} can't be stopped: what it does can't be undone half way"),
            "let it finish, then undo it with the reverse action (add the machine again after a removal)",
            pb::error::Kind::NotCancellable(pb::NotCancellable {
                operation: Some(pb::OperationId {
                    value: id.to_owned(),
                }),
                step: current_step(operation),
            }),
        ),
    )
}

fn not_retryable(id: &str, operation: &pb::Operation) -> Status {
    let (what, fix) = match state(operation) {
        pb::OperationState::Running => (
            "is still running",
            "watch it to see how it ends; only a failed operation can be tried again",
        ),
        pb::OperationState::Succeeded => ("already succeeded", "nothing is left to do"),
        pb::OperationState::Cancelled => (
            "was cancelled",
            "start the work again as a new call (a cancelled operation was undone)",
        ),
        _ => ("can't run again", "start the work again as a new call"),
    };
    status::failure(
        Code::FailedPrecondition,
        status::error(
            format!("operation {id} {what}, so it can't be tried again"),
            fix,
            pb::error::Kind::InvalidField(pb::InvalidField {
                field: "id".to_owned(),
                problem: pb::invalid_field::Problem::NotAllowedHere as i32,
            }),
        ),
    )
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
            .filter(|operation| {
                request
                    .target
                    .as_ref()
                    .is_none_or(|target| operation.target.as_ref() == Some(target))
            })
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

    /// Asks a running operation to stop at its next step boundary and
    /// answers with it as it is; its watchers see it end cancelled. One
    /// that already ended is answered as it ended.
    async fn cancel_operation(
        &self,
        request: Request<pb::CancelOperationRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = identity::caller(&request)?;
        let id = id_of(request.into_inner().id)?;
        let tracked = self.operations.find(&caller, &id)?;
        let operation = tracked.operation.borrow().clone();
        if finished(&operation) {
            return Ok(Response::new(operation));
        }
        let mut cancel = lock(&tracked.cancel);
        if !tracked.cancellable || cancel.past_no_return {
            drop(cancel);
            return Err(not_cancellable(&id, &tracked.operation.borrow().clone()));
        }
        cancel.asked = true;
        drop(cancel);
        Ok(Response::new(tracked.operation.borrow().clone()))
    }

    /// Runs a failed operation's work again under the same id: its steps
    /// start over (a failed step's work was undone) and its version goes
    /// on. A retry resent with the same key answers with that run.
    async fn retry_operation(
        &self,
        request: Request<pb::RetryOperationRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let id = id_of(request.id)?;
        let key = request
            .idempotency_key
            .map(|key| key.value)
            .filter(|value| !value.is_empty());
        let tracked = self.operations.find(&caller, &id)?;
        let mut retry_key = lock(&tracked.retry_key);
        let operation = tracked.operation.borrow().clone();
        if key.is_some() && *retry_key == key {
            return Ok(Response::new(operation));
        }
        let rerun = lock(&tracked.rerun).clone();
        let Some(rerun) = rerun.filter(|_| state(&operation) == pb::OperationState::Failed) else {
            return Err(not_retryable(&id, &operation));
        };
        *retry_key = key;
        *lock(&tracked.cancel) = Cancel::default();
        let progress = Progress {
            tracked: tracked.clone(),
        };
        progress.change(|operation| {
            operation.state = pb::OperationState::Running as i32;
            operation.cancellable = tracked.cancellable;
            // Work that names its own steps has none to start from, so the
            // steps it showed start over, keeping their titles.
            operation.steps = if tracked.steps.is_empty() {
                operation.steps.iter().map(|shown| waiting_named(&shown.key, &shown.title)).collect()
            } else {
                tracked.steps.iter().map(|&step| waiting(step)).collect()
            };
            operation.error = None;
            operation.result = None;
            operation.finished_at = None;
        });
        drop(retry_key);
        let operation = progress.operation();
        rerun(progress);
        Ok(Response::new(operation))
    }
}

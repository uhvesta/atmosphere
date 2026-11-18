//! [`Machines`] over the daemon's machine, provider, operation and skills
//! services (HO-1 to HO-7, HO-13, MP-2, MP-4, MP-12 to MP-17, SK-2, SK-5,
//! SK-8, SK-11). An add is followed as the operation it is; a login is the
//! provider service's bidirectional stream, one per target.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use humata2_clients::machines::{
    AddEvent, AddFailure, AddOutcome, AddRequest, AddStream, AddTarget, Choices, ClashChoice,
    ChannelFolder, GhLogin, LoginEvent, LoginReason, LoginStream, LoginTarget, MachineDetail, MachineHealth,
    MachineScripts, Machines, OperationId, PromptKind, Provider, ProviderParam, Reach, ScriptOutcome, ScriptRuleView,
    ScriptRun, ScriptStepRun, ScriptStepView, SkillClash, SkillsPreview, Started, StepKind, StepStatus, StepUpdate,
    UnattendedMethod, short_version,
};
use humata2_clients::model::{ChannelId, MachineId, MachineStop, RootPlan, SessionId};
use humata2_clients::{DaemonError, Reply};
use humata2_grpc::humata::v1 as pb;
use humata2_grpc::humata::v1::{
    machine_service_client::MachineServiceClient, provider_service_client::ProviderServiceClient,
    skills_service_client::SkillsServiceClient,
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Code, Status};

use crate::connection::{LiveDaemon, Shared, Wire, fresh_key, not_in_this_release};
use crate::local_time::LocalNow;

const PAGE_SIZE: u32 = 200;
const POLL: Duration = Duration::from_millis(100);
/// How many times a watch that ended early is opened again before the add
/// is reported as lost.
const REOPENS: u32 = 5;
/// How many lines of a failed step's output an add failure shows.
const TAIL_LINES: usize = 6;

type Sender<T> = mpsc::UnboundedSender<Result<T, DaemonError>>;

#[derive(Default)]
struct State {
    /// The request stream of each open login.
    logins: HashMap<LoginTarget, mpsc::Sender<pb::LoginClientMessage>>,
    /// The plan token of each machine's last skills preview.
    plans: HashMap<MachineId, pb::PlanToken>,
    /// The (agent, skill) clashes of each machine's last skills preview.
    clashes: HashMap<MachineId, Vec<(String, String)>>,
}

/// The app's machines client over one daemon connection. Cheap to clone.
#[derive(Clone)]
pub struct LiveMachines {
    shared: Arc<Shared>,
    state: Arc<Mutex<State>>,
}

impl LiveMachines {
    /// A machines client over the connection `daemon` uses.
    #[must_use]
    pub fn new(daemon: &LiveDaemon) -> Self {
        Self {
            shared: daemon.shared.clone(),
            state: Arc::new(Mutex::new(State::default())),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

fn machine_service(shared: &Shared) -> MachineServiceClient<Wire> {
    MachineServiceClient::new(shared.wire.clone())
}

fn provider_service(shared: &Shared) -> ProviderServiceClient<Wire> {
    ProviderServiceClient::new(shared.wire.clone())
}

fn skills_service(shared: &Shared) -> SkillsServiceClient<Wire> {
    SkillsServiceClient::new(shared.wire.clone())
}

/// A gRPC failure as the app's error. A call the daemon doesn't build yet
/// is [`DaemonError::NotServedYet`] (D134), so the app can say so.
fn failure(shared: &Shared, status: &Status, what: &str) -> DaemonError {
    if status.code() == Code::Unimplemented {
        return not_in_this_release(what);
    }
    shared.error(status, what)
}

fn machine_id(machine: &MachineId) -> Option<pb::MachineId> {
    Some(pb::MachineId {
        value: machine.as_str().to_owned(),
    })
}

/// D261: the host's own machine is stopped by its own daemon, which keeps
/// the record and re-execs; there is no link to deliver it over, so the
/// stop is in force once it answers.
fn stop_this_machine(shared: Arc<Shared>) -> Reply<MachineStop> {
    Box::pin(async move {
        const WHAT: &str = "Stopping this Mac's sessions";
        let reply = shared
            .daemon_info()
            .stop_sessions(pb::StopSessionsRequest {
                // The daemon makes the id; the app sends one stop per confirm.
                stop_id: String::new(),
                reason: String::new(),
                via: pb::StopVia::App as i32,
                by_machine: String::new(),
            })
            .await
            .map_err(|status| failure(&shared, &status, WHAT))?
            .into_inner();
        let stopped = reply.stopped.ok_or_else(|| DaemonError::Refused {
            what: WHAT.to_owned(),
            reason: "the daemon answered without the stop it made. Reload This Mac's page to see whether its sessions stopped before trying again".to_owned(),
        })?;
        let now = crate::local_time::LocalNow::read();
        Ok(crate::map::machine_stop(&stopped, true, &now))
    })
}

fn operation_id(operation: &OperationId) -> Option<pb::OperationId> {
    Some(pb::OperationId {
        value: operation.0.clone(),
    })
}

/// The daemon's own words for an error: what failed, then what to do.
fn words(error: &pb::Error) -> String {
    match (error.message.is_empty(), error.fix.is_empty()) {
        (true, true) => "the daemon didn't say why".to_owned(),
        (false, true) => error.message.clone(),
        (false, false) | (true, false) => crate::connection::said(&error.message, &error.fix),
    }
}

fn last_line(text: &str) -> Option<String> {
    text.lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_owned)
}

fn last_lines(text: &str, count: usize) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    let start = lines.len().saturating_sub(count);
    lines[start..].join("\n").trim().to_owned()
}

fn state_of(operation: &pb::Operation) -> pb::OperationState {
    pb::OperationState::try_from(operation.state).unwrap_or(pb::OperationState::Unspecified)
}

fn step_state_of(step: &pb::OperationStep) -> pb::StepState {
    pb::StepState::try_from(step.state).unwrap_or(pb::StepState::Unspecified)
}

// --- Following an operation -------------------------------------------

/// Follows `operation` until it stops running, calling `seen` with every
/// version (the first included). A broken stream is an error.
async fn follow(
    shared: &Shared,
    mut operation: pb::Operation,
    what: &str,
    mut seen: impl FnMut(&pb::Operation),
) -> Result<pb::Operation, DaemonError> {
    seen(&operation);
    let mut reopened = 0;
    while state_of(&operation) == pb::OperationState::Running {
        let Some(id) = operation.id.clone() else {
            return Err(DaemonError::Refused {
                what: what.to_owned(),
                reason: "the daemon didn't name the operation it started. Reload to see where it stands".to_owned(),
            });
        };
        let mut stream = shared
            .operations()
            .watch_operation(pb::WatchOperationRequest {
                id: Some(id.clone()),
                from_version: operation.version,
            })
            .await
            .map_err(|status| failure(shared, &status, what))?
            .into_inner();
        loop {
            match stream.message().await {
                Ok(Some(update)) => {
                    seen(&update);
                    operation = update;
                    if state_of(&operation) != pb::OperationState::Running {
                        break;
                    }
                }
                Ok(None) => break,
                Err(status) => return Err(failure(shared, &status, what)),
            }
        }
        if state_of(&operation) == pb::OperationState::Running {
            reopened += 1;
            if reopened > REOPENS {
                return Err(DaemonError::Refused {
                    what: what.to_owned(),
                    reason: "the daemon stopped reporting on it before it finished. Reload to see where it stands".to_owned(),
                });
            }
            // On the client's runtime: the caller may be polled elsewhere.
            let _ = shared.spawn(async { tokio::time::sleep(POLL).await }).await;
            operation = shared
                .operations()
                .get_operation(pb::GetOperationRequest { id: Some(id) })
                .await
                .map_err(|status| failure(shared, &status, what))?
                .into_inner();
            seen(&operation);
        }
    }
    Ok(operation)
}

/// A finished operation as success, or as the daemon's reason it didn't.
fn succeeded(operation: &pb::Operation, what: &str) -> Result<(), DaemonError> {
    match state_of(operation) {
        pb::OperationState::Succeeded => Ok(()),
        pb::OperationState::Cancelled => Err(DaemonError::Refused {
            what: what.to_owned(),
            reason: "it was cancelled before it finished. Nothing more was done".to_owned(),
        }),
        _ => Err(DaemonError::Refused {
            what: what.to_owned(),
            reason: operation
                .error
                .as_ref()
                .map_or_else(|| "it didn't finish. Try again".to_owned(), words),
        }),
    }
}

// --- Machine scripts (MACHINESCRIPTS, D686, D687) ------------------------

fn script_outcome_of(value: i32) -> ScriptOutcome {
    match pb::MachineScriptOutcome::try_from(value) {
        Ok(pb::MachineScriptOutcome::Passed) => ScriptOutcome::Passed,
        Ok(pb::MachineScriptOutcome::Failed) => ScriptOutcome::Failed,
        Ok(pb::MachineScriptOutcome::TimedOut) => ScriptOutcome::TimedOut,
        Ok(pb::MachineScriptOutcome::Skipped) => ScriptOutcome::Skipped,
        // A run the host hasn't ended, or one this app doesn't know: shown as going.
        Ok(pb::MachineScriptOutcome::Running | pb::MachineScriptOutcome::Unspecified) | Err(_) => {
            ScriptOutcome::Running
        }
    }
}

fn script_run_of(run: &pb::MachineScriptRun, now: &LocalNow) -> ScriptRun {
    let started_ms = run
        .started_at
        .as_ref()
        .map_or(0, |at| at.seconds.saturating_mul(1000).saturating_add(i64::from(at.nanos / 1_000_000)));
    ScriptRun {
        id: run.id.clone(),
        machine: MachineId::new(run.machine.as_ref().map(|machine| machine.value.clone()).unwrap_or_default()),
        rule: run.rule.clone(),
        trigger: run.trigger.clone(),
        session: run.session.as_ref().map(|session| SessionId::new(session.value.clone())),
        at: now.clock(started_ms),
        took_ms: run.took_ms,
        outcome: script_outcome_of(run.outcome),
        steps: run
            .steps
            .iter()
            .map(|step| ScriptStepRun {
                index: step.index,
                kind: step.kind.clone(),
                label: step.label.clone(),
                outcome: script_outcome_of(step.outcome),
                exit: step.exit,
                tail: step.tail.clone(),
                error: step.error.clone(),
            })
            .collect(),
    }
}

fn script_rule_of(rule: &pb::MachineScriptRule) -> ScriptRuleView {
    ScriptRuleView {
        name: rule.name.clone(),
        triggers: rule.triggers.clone(),
        machines: rule.machines.clone(),
        once: rule.once,
        steps: rule
            .steps
            .iter()
            .map(|step| ScriptStepView { kind: step.kind.clone(), label: step.label.clone() })
            .collect(),
    }
}

fn scripts_of(reply: &pb::ListMachineScriptsResponse, warnings: Vec<String>, now: &LocalNow) -> MachineScripts {
    MachineScripts {
        rules: reply.rules.iter().map(script_rule_of).collect(),
        last_runs: reply.last_runs.iter().map(|run| script_run_of(run, now)).collect(),
        script_names: reply.scripts.clone(),
        problem: reply.problem.as_ref().map(words),
        warnings,
    }
}

// --- An add as events ----------------------------------------------------

fn kind_of(key: &str) -> Option<StepKind> {
    match key {
        "create" => Some(StepKind::Create),
        "connect" => Some(StepKind::Connect),
        "check" => Some(StepKind::Check),
        "install" => Some(StepKind::Install),
        "start" => Some(StepKind::Start),
        "find_agents" => Some(StepKind::FindAgents),
        // The daemon has no login step: the dialog adds `StepKind::Login` itself.
        // `identity` and other provider-side steps have no row in the dialog.
        _ => None,
    }
}

fn step_status(step: &pb::OperationStep) -> StepStatus {
    match step_state_of(step) {
        pb::StepState::Running => StepStatus::Running,
        pb::StepState::Done | pb::StepState::Skipped => StepStatus::Done,
        pb::StepState::Failed => StepStatus::Failed,
        pb::StepState::Waiting | pb::StepState::Undone | pb::StepState::Unspecified => {
            StepStatus::NotRun
        }
    }
}

/// The line under a step: what a finished check read, else the daemon's last
/// output line, else the step's own title. Nothing for a step not reached.
fn step_detail(step: &pb::OperationStep) -> String {
    if step.key == "check" && step_state_of(step) == pb::StepState::Done {
        if let Some(line) = check_line(&step.facts) {
            return line;
        }
    }
    match step_state_of(step) {
        pb::StepState::Waiting | pb::StepState::Unspecified => String::new(),
        pb::StepState::Skipped => "Skipped".to_owned(),
        pb::StepState::Undone => "Undone".to_owned(),
        _ => last_line(&step.output_tail).unwrap_or_else(|| step.title.clone()),
    }
}

/// What a check read, worded: "Linux x86_64 · git 2.47 · gh 2.83 · 38 GB
/// free" (HO-1). What the daemon didn't send is left out; none when it sent
/// nothing to word.
fn check_line(facts: &HashMap<String, String>) -> Option<String> {
    let fact = |name: &str| facts.get(name).map(|value| value.trim()).filter(|value| !value.is_empty());
    let mut parts = Vec::new();
    let system: Vec<String> = [
        fact("os").map(|os| match os {
            "linux" => "Linux".to_owned(),
            "darwin" => "macOS".to_owned(),
            other => other.to_owned(),
        }),
        fact("arch").map(str::to_owned),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !system.is_empty() {
        parts.push(system.join(" "));
    }
    for tool in ["git", "gh"] {
        if let Some(version) = fact(tool) {
            parts.push(format!("{tool} {}", short_version(version)));
        }
    }
    if let Some(bytes) = fact("disk_free_bytes").and_then(|bytes| bytes.parse::<u64>().ok()) {
        parts.push(format!("{} GB free", bytes / 1_000_000_000));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// Turns each version of an add into the events the dialog expects.
struct Reporter {
    out: Sender<AddEvent>,
    started: bool,
    sent: HashMap<StepKind, (StepStatus, String)>,
}

impl Reporter {
    fn new(out: Sender<AddEvent>) -> Self {
        Self {
            out,
            started: false,
            sent: HashMap::new(),
        }
    }

    fn send(&self, event: AddEvent) {
        // A closed receiver means the dialog is gone; nothing to tell.
        let _ = self.out.send(Ok(event));
    }

    fn observe(&mut self, operation: &pb::Operation) {
        if !self.started {
            self.started = true;
            self.send(AddEvent::Started {
                operation: OperationId::new(
                    operation.id.as_ref().map(|id| id.value.clone()).unwrap_or_default(),
                ),
                steps: operation.steps.iter().filter_map(|step| kind_of(&step.key)).collect(),
            });
        }
        for step in &operation.steps {
            let Some(kind) = kind_of(&step.key) else {
                continue;
            };
            let now = (step_status(step), step_detail(step));
            let before = self
                .sent
                .get(&kind)
                .cloned()
                .unwrap_or((StepStatus::NotRun, String::new()));
            if now != before {
                self.send(AddEvent::Step(StepUpdate {
                    step: kind,
                    status: now.0,
                    detail: now.1.clone(),
                }));
                self.sent.insert(kind, now);
            }
        }
    }
}

/// The error under any `step_failed` wrappers.
fn root(error: &pb::Error) -> &pb::Error {
    match &error.kind {
        Some(pb::error::Kind::StepFailed(failed)) => failed.cause.as_deref().map_or(error, root),
        _ => error,
    }
}

/// The step an add stopped at: the failed one, or the last one the dialog
/// shows that was reached.
fn failed_step(operation: &pb::Operation) -> (StepKind, Option<&pb::OperationStep>) {
    // An add undoes what it did after a failure, so the step that failed may
    // read Undone by now (its output kept): then it is the last of the add's
    // own steps that ran and wasn't finished.
    let failed = operation
        .steps
        .iter()
        .find(|step| step_state_of(step) == pb::StepState::Failed)
        .or_else(|| {
            operation.steps.iter().rev().find(|step| {
                kind_of(&step.key).is_some()
                    && matches!(step_state_of(step), pb::StepState::Running | pb::StepState::Undone)
            })
        });
    let mut before = None;
    for step in &operation.steps {
        if failed.is_some_and(|failed| std::ptr::eq(failed, step)) {
            break;
        }
        if let Some(kind) = kind_of(&step.key) {
            before = Some(kind);
        }
    }
    let kind = failed
        .and_then(|step| kind_of(&step.key))
        .or(before)
        .or_else(|| operation.steps.iter().find_map(|step| kind_of(&step.key)))
        .unwrap_or(StepKind::Connect);
    (kind, failed)
}

/// Why the add needs a login (D167): that the command hung, or the prompt it
/// printed with its kind. The line is the daemon's when it sent one, else
/// the last line the step printed.
fn login_reason(needs: &pb::LoginNeedsUser, tail: &str, error: &pb::Error) -> LoginReason {
    if needs.hung {
        return LoginReason::Hung;
    }
    let line = Some(needs.prompt_line.clone())
        .filter(|line| !line.is_empty())
        .or_else(|| last_line(tail))
        .or_else(|| Some(error.message.clone()).filter(|text| !text.is_empty()))
        .unwrap_or_default();
    LoginReason::Prompted { kind: prompt_kind(needs.prompt_kind), line }
}

/// The app's kind of a prompt from the daemon's.
fn prompt_kind(value: i32) -> PromptKind {
    match pb::LoginPromptKind::try_from(value) {
        Ok(pb::LoginPromptKind::Password) => PromptKind::Password,
        Ok(pb::LoginPromptKind::Passphrase) => PromptKind::Passphrase,
        Ok(pb::LoginPromptKind::Code) => PromptKind::Code,
        Ok(pb::LoginPromptKind::Confirm) => PromptKind::Confirm,
        Ok(pb::LoginPromptKind::Device) => PromptKind::Device,
        Ok(pb::LoginPromptKind::Unspecified) | Err(_) => PromptKind::Other,
    }
}

/// How a failed add ended, in the dialog's words.
fn outcome_of_failure(operation: &pb::Operation) -> AddOutcome {
    let error = operation.error.clone().unwrap_or_default();
    let (step, failed) = failed_step(operation);
    let tail = failed.map(|step| step.output_tail.as_str()).unwrap_or_default();
    match &root(&error).kind {
        // A first sighting is the one key the Confirm card offers. A key that differs from one
        // already pinned or confirmed is no Confirm card: confirming would replace that key, and
        // the card can't show which one it replaces. The daemon's own words, which say someone
        // may be intercepting, are what the dialog shows.
        Some(pb::error::Kind::HostKeyUnknown(key)) if !key.presented_fingerprint.is_empty() => {
            return AddOutcome::HostKey {
                fingerprint: key.presented_fingerprint.clone(),
            };
        }
        Some(pb::error::Kind::LoginNeedsUser(needs)) => {
            return AddOutcome::NeedsLogin { reason: login_reason(needs, tail, root(&error)) };
        }
        Some(pb::error::Kind::HostNeedsMigrate(needs)) => {
            return AddOutcome::NeedsMigrate { platform: needs.platform.clone(), what: words(root(&error)) };
        }
        _ => {}
    }
    // The fix is the daemon's text: it is not parsed into a command, which
    // could be wrong. It goes after what failed.
    let mut what = words(&error);
    let output = last_lines(tail, TAIL_LINES);
    if !output.is_empty() {
        what = format!("{what}\n\n{output}");
    }
    AddOutcome::Failed(AddFailure {
        step,
        title: format!("{} failed", step.label()),
        what,
        fix: None,
    })
}

/// Follows an add to its end and tells `out` how it went. The stream ends
/// after the outcome; a cancelled add ends it with none.
async fn report_add(shared: Arc<Shared>, operation: pb::Operation, out: Sender<AddEvent>) {
    const WHAT: &str = "Adding the machine";
    let mut reporter = Reporter::new(out.clone());
    let done = follow(&shared, operation, WHAT, |update| reporter.observe(update)).await;
    match done {
        Err(error) => {
            let _ = out.send(Err(error));
        }
        Ok(operation) => match state_of(&operation) {
            pb::OperationState::Succeeded => match &operation.result {
                Some(pb::operation::Result::Machine(machine)) => {
                    reporter.send(AddEvent::Outcome(AddOutcome::Added(MachineId::new(
                        machine.value.clone(),
                    ))));
                }
                _ => {
                    let _ = out.send(Err(DaemonError::Refused {
                        what: WHAT.to_owned(),
                        reason: "the daemon didn't name the machine it added. Reload to see whether it is there".to_owned(),
                    }));
                }
            },
            pb::OperationState::Failed => {
                reporter.send(AddEvent::Outcome(outcome_of_failure(&operation)));
            }
            _ => {}
        },
    }
}

fn started_stream(shared: &Arc<Shared>, operation: pb::Operation) -> AddStream {
    let (out, stream) = mpsc::unbounded_channel();
    let shared = shared.clone();
    shared.clone().spawn(report_add(shared, operation, out));
    stream
}

// --- Detail and skills ----------------------------------------------------

fn reach_of(machine: &pb::Machine) -> Reach {
    match &machine.provider {
        Some(provider) if !provider.value.is_empty() => Reach::Provider(provider.value.clone()),
        _ if machine.is_this_mac => Reach::Local,
        _ => Reach::Ssh,
    }
}

fn health_of(shared: &Shared, health: Option<pb::MachineHealth>) -> MachineHealth {
    let health = health.unwrap_or_default();
    MachineHealth {
        gh: Some(health.gh_user.clone())
            .filter(|user| !user.is_empty())
            .map(|user| GhLogin {
                user,
                scopes: health.gh_scopes.clone(),
            }),
        // One version for each agent found, in the same order; empty when unknown.
        agents: health
            .agents_installed
            .iter()
            .enumerate()
            .map(|(index, agent)| {
                let version = health.agent_versions.get(index).cloned().unwrap_or_default();
                (shared.agents.agent(&agent.value), version)
            })
            .collect(),
        disk_free: health.disk_free_bytes,
        disk_total: health.disk_total_bytes,
        variables: health.variable_names.clone(),
        skills_problem: health.skills_sync_problem.as_ref().map(words),
        git_version: health.git_version.clone(),
        gh_version: health.gh_version.clone(),
    }
}

fn push_once(list: &mut Vec<String>, name: &str) {
    if !list.iter().any(|known| known == name) {
        list.push(name.to_owned());
    }
}

fn preview_of(plan: &pb::SkillsSyncPlan) -> SkillsPreview {
    let mut preview = SkillsPreview {
        first_sync: plan.first_sync,
        ..SkillsPreview::default()
    };
    for change in &plan.changes {
        match pb::skill_change::Kind::try_from(change.kind) {
            Ok(pb::skill_change::Kind::Add) => push_once(&mut preview.added, &change.skill),
            Ok(pb::skill_change::Kind::Update) => push_once(&mut preview.updated, &change.skill),
            Ok(pb::skill_change::Kind::Remove) => push_once(&mut preview.removed, &change.skill),
            _ => {}
        }
    }
    for clash in &plan.clashes {
        if !preview.clashes.iter().any(|known| known.name == clash.skill) {
            preview.clashes.push(SkillClash {
                name: clash.skill.clone(),
            });
        }
    }
    preview
}

/// Reads a machine's skills plan and remembers its token and clashes.
async fn read_plan(
    shared: &Shared,
    state: &Mutex<State>,
    machine: &MachineId,
) -> Result<SkillsPreview, DaemonError> {
    let plan = skills_service(shared)
        .preview_skills_sync(pb::PreviewSkillsSyncRequest {
            machine: machine_id(machine),
        })
        .await
        .map_err(|status| failure(shared, &status, "The skills preview"))?
        .into_inner();
    let mut state = match state.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    };
    match plan.token.clone() {
        Some(token) => {
            state.plans.insert(machine.clone(), token);
        }
        None => {
            state.plans.remove(machine);
        }
    }
    state.clashes.insert(
        machine.clone(),
        plan.clashes
            .iter()
            .map(|clash| (clash.agent.clone(), clash.skill.clone()))
            .collect(),
    );
    Ok(preview_of(&plan))
}

// --- Logins --------------------------------------------------------------

fn login_target(target: &LoginTarget) -> pb::login_open::Target {
    match target {
        LoginTarget::Operation(operation) => pb::login_open::Target::Operation(pb::OperationId {
            value: operation.0.clone(),
        }),
        LoginTarget::Machine(machine) => pb::login_open::Target::Machine(pb::MachineId {
            value: machine.as_str().to_owned(),
        }),
    }
}

/// humatad's version on a machine as the daemon lists it: set once the link's Hello arrived,
/// so none when it isn't known yet or the machine can't be read just now (frame 58).
async fn daemon_version(shared: &Shared, machine: &MachineId) -> Option<String> {
    let reply = machine_service(shared)
        .get_machine(pb::GetMachineRequest {
            id: machine_id(machine),
        })
        .await
        .ok()?
        .into_inner();
    reply
        .machine
        .map(|machine| machine.daemon_version)
        .filter(|version| !version.is_empty())
}

/// Reads a login's server messages into events until it is ready, failed
/// or over, then forgets the login's request stream.
async fn read_login(
    shared: Arc<Shared>,
    mut stream: tonic::Streaming<pb::LoginServerMessage>,
    out: Sender<LoginEvent>,
    state: Arc<Mutex<State>>,
    target: LoginTarget,
    requests: mpsc::Sender<pb::LoginClientMessage>,
) {
    use pb::login_server_message::Message;
    loop {
        let event = match stream.message().await {
            Ok(Some(message)) => match message.message {
                Some(Message::Output(bytes)) => Some((LoginEvent::Output(bytes.to_vec()), false)),
                // The terminal shows the prompt; the dialog uses its kind
                // to say why the login opened and what it accepted (D167).
                Some(Message::PromptSeen(line)) => Some((
                    LoginEvent::Prompt {
                        kind: prompt_kind(message.prompt_kind),
                        line,
                    },
                    false,
                )),
                None => None,
                Some(Message::Ready(ready)) => {
                    let machine = MachineId::new(
                        ready.machine.map(|machine| machine.value).unwrap_or_default(),
                    );
                    let daemon_version = daemon_version(&shared, &machine).await;
                    Some((
                        LoginEvent::Ready {
                            machine,
                            can_make_unattended: ready.can_make_unattended,
                            daemon_version,
                        },
                        true,
                    ))
                }
                Some(Message::Failed(error)) => Some((
                    LoginEvent::Failed {
                        reason: words(&error),
                    },
                    true,
                )),
            },
            Ok(None) => break,
            Err(status) => {
                let _ = out.send(Err(failure(&shared, &status, "The login")));
                break;
            }
        };
        if let Some((event, last)) = event {
            if out.send(Ok(event)).is_err() || last {
                break;
            }
        }
    }
    let mut state = match state.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    };
    if state
        .logins
        .get(&target)
        .is_some_and(|open| open.same_channel(&requests))
    {
        state.logins.remove(&target);
    }
}

impl LiveMachines {
    fn send_to_login(&self, target: &LoginTarget, message: pb::login_client_message::Message) -> Reply<()> {
        let sender = self.state().logins.get(target).cloned();
        Box::pin(async move {
            let missing = || DaemonError::NotFound {
                what: "The login".to_owned(),
            };
            let sender = sender.ok_or_else(missing)?;
            sender
                .send(pb::LoginClientMessage {
                    message: Some(message),
                })
                .await
                .map_err(|_| missing())
        })
    }
}

impl Machines for LiveMachines {
    fn choices(&self) -> Reply<Choices> {
        let shared = self.shared.clone();
        Box::pin(async move {
            let reply = provider_service(&shared)
                .list_providers(pb::ListProvidersRequest {})
                .await
                .map_err(|status| failure(&shared, &status, "The providers"))?
                .into_inner();
            let providers = reply
                .providers
                .iter()
                .map(|provider| Provider {
                    id: provider.id.as_ref().map(|id| id.value.clone()).unwrap_or_default(),
                    label: provider.label.clone(),
                    interactive: provider.interactive,
                    params: provider
                        .params
                        .iter()
                        .map(|param| ProviderParam {
                            name: param.name.clone(),
                            label: param.prompt.clone(),
                            default: param.default_value.clone(),
                            required: param.default_value.is_empty(),
                        })
                        .collect(),
                })
                .collect();
            // The daemon has no call that lists `~/.ssh/config`: the form
            // still takes a typed host.
            Ok(Choices {
                hosts: Vec::new(),
                providers,
            })
        })
    }

    fn add(&self, request: AddRequest) -> Reply<AddStream> {
        let shared = self.shared.clone();
        Box::pin(async move {
            let name = request.name.unwrap_or_default();
            let operation = match request.target {
                AddTarget::Ssh { host } => {
                    machine_service(&shared)
                        .add_ssh_machine(pb::AddSshMachineRequest {
                            idempotency_key: fresh_key(),
                            ssh_host: host,
                            name,
                            root: String::new(),
                            trust_host_key_fingerprint: request.trusted_fingerprint.unwrap_or_default(),
                        })
                        .await
                }
                AddTarget::Provider { provider, params } => {
                    provider_service(&shared)
                        .create_machine(pb::CreateMachineRequest {
                            idempotency_key: fresh_key(),
                            provider: Some(pb::ProviderId { value: provider }),
                            name,
                            params: params.into_iter().collect(),
                        })
                        .await
                }
            }
            .map_err(|status| failure(&shared, &status, "Adding the machine"))?
            .into_inner();
            Ok(started_stream(&shared, operation))
        })
    }

    fn retry(&self, operation: &OperationId) -> Reply<AddStream> {
        let shared = self.shared.clone();
        let id = operation_id(operation);
        Box::pin(async move {
            let operation = shared
                .operations()
                .retry_operation(pb::RetryOperationRequest {
                    id,
                    idempotency_key: fresh_key(),
                })
                .await
                .map_err(|status| failure(&shared, &status, "Trying the add again"))?
                .into_inner();
            Ok(started_stream(&shared, operation))
        })
    }

    fn cancel(&self, operation: &OperationId) -> Reply<()> {
        let shared = self.shared.clone();
        let id = operation_id(operation);
        Box::pin(async move {
            shared
                .operations()
                .cancel_operation(pb::CancelOperationRequest {
                    id,
                    idempotency_key: fresh_key(),
                })
                .await
                .map_err(|status| failure(&shared, &status, "Cancelling the add"))?;
            Ok(())
        })
    }

    fn open_login(&self, target: &LoginTarget) -> Reply<LoginStream> {
        let this = self.clone();
        let target = target.clone();
        Box::pin(async move {
            let (requests, receiver) = mpsc::channel(32);
            let open = pb::LoginClientMessage {
                message: Some(pb::login_client_message::Message::Open(pb::LoginOpen {
                    target: Some(login_target(&target)),
                    cols: 80,
                    rows: 24,
                })),
            };
            // Queued first: the daemon answers once it has read it.
            requests.send(open).await.map_err(|_| DaemonError::NotFound {
                what: "The login".to_owned(),
            })?;
            let stream = provider_service(&this.shared)
                .interactive_login(ReceiverStream::new(receiver))
                .await
                .map_err(|status| failure(&this.shared, &status, "The login"))?
                .into_inner();
            this.state().logins.insert(target.clone(), requests.clone());
            let (out, events) = mpsc::unbounded_channel();
            this.shared.spawn(read_login(
                this.shared.clone(),
                stream,
                out,
                this.state.clone(),
                target,
                requests,
            ));
            Ok(events)
        })
    }

    fn login_input(&self, target: &LoginTarget, bytes: Vec<u8>) -> Reply<()> {
        self.send_to_login(target, pb::login_client_message::Message::Input(bytes.into()))
    }

    fn login_resize(&self, target: &LoginTarget, columns: u16, rows: u16) -> Reply<()> {
        self.send_to_login(
            target,
            pb::login_client_message::Message::Resize(pb::LoginResize {
                cols: u32::from(columns),
                rows: u32::from(rows),
            }),
        )
    }

    fn login_done(&self, target: &LoginTarget) -> Reply<()> {
        self.send_to_login(target, pb::login_client_message::Message::Done(pb::LoginDone {}))
    }

    fn login_cancel(&self, target: &LoginTarget) -> Reply<()> {
        let sent = self.send_to_login(target, pb::login_client_message::Message::Cancel(pb::LoginCancel {}));
        self.state().logins.remove(target);
        sent
    }

    fn make_unattended(&self, machine: &MachineId, method: UnattendedMethod) -> Reply<()> {
        let shared = self.shared.clone();
        let machine = machine_id(machine);
        Box::pin(async move {
            let method = match method {
                UnattendedMethod::AddKey => pb::make_unattended_request::Method::AuthorizedKey,
                UnattendedMethod::KeepToken => pb::make_unattended_request::Method::KeepCachedToken,
            };
            provider_service(&shared)
                .make_unattended(pb::MakeUnattendedRequest {
                    idempotency_key: fresh_key(),
                    machine,
                    method: method as i32,
                    presence: None,
                })
                .await
                .map_err(|status| failure(&shared, &status, "Making reconnects unattended"))?;
            Ok(())
        })
    }

    fn detail(&self, machine: &MachineId) -> Reply<MachineDetail> {
        let shared = self.shared.clone();
        let id = machine.clone();
        Box::pin(async move {
            const WHAT: &str = "The machine";
            let detail = machine_service(&shared)
                .get_machine(pb::GetMachineRequest {
                    id: machine_id(&id),
                })
                .await
                .map_err(|status| failure(&shared, &status, WHAT))?
                .into_inner();
            let mut sessions = Vec::new();
            // How many of the machine's sessions each channel has.
            let mut per_channel: HashMap<String, usize> = HashMap::new();
            let mut token = String::new();
            loop {
                let reply = shared
                    .sessions()
                    .list_sessions(pb::ListSessionsRequest {
                        channel: None,
                        machine: machine_id(&id),
                        include_archived: false,
                        page: Some(pb::PageRequest {
                            page_size: PAGE_SIZE,
                            page_token: token,
                        }),
                    })
                    .await
                    .map_err(|status| failure(&shared, &status, "The machine's sessions"))?
                    .into_inner();
                for session in &reply.sessions {
                    if let Some(channel) = &session.channel {
                        *per_channel.entry(channel.value.clone()).or_insert(0) += 1;
                    }
                }
                sessions.extend(
                    reply
                        .sessions
                        .iter()
                        .filter_map(|session| session.id.as_ref())
                        .map(|session| SessionId::new(session.value.clone())),
                );
                token = reply.page.map(|page| page.next_page_token).unwrap_or_default();
                if token.is_empty() {
                    break;
                }
            }
            // The daemon names a folder by its path; the channel owning it is the
            // one whose folder name it is (CH-2), or the Desk's. A folder no
            // channel owns can't be opened in Files and is left out.
            let mut owners: HashMap<String, String> = HashMap::new();
            let mut channels_token = String::new();
            loop {
                let reply = shared
                    .channels()
                    .list_channels(pb::ListChannelsRequest {
                        page: Some(pb::PageRequest {
                            page_size: PAGE_SIZE,
                            page_token: channels_token,
                        }),
                    })
                    .await
                    .map_err(|status| failure(&shared, &status, "The channels"))?
                    .into_inner();
                for channel in &reply.channels {
                    if let Some(channel_id) = &channel.id {
                        owners.insert(channel.folder.clone(), channel_id.value.clone());
                    }
                }
                channels_token = reply.page.map(|page| page.next_page_token).unwrap_or_default();
                if channels_token.is_empty() {
                    break;
                }
            }
            let folders = detail
                .folders
                .iter()
                .filter_map(|folder| {
                    let name = folder.path.trim_end_matches('/').rsplit('/').next().unwrap_or_default();
                    let channel = owners.get(name).cloned().or_else(|| {
                        (name == humata2_clients::desk::FOLDER).then(|| name.to_owned())
                    })?;
                    Some(ChannelFolder {
                        sessions: per_channel.get(&channel).copied().unwrap_or(0),
                        channel: humata2_clients::model::ChannelId::new(channel),
                        path: folder.path.clone(),
                        bytes: folder.bytes,
                    })
                })
                .collect();
            let machine = detail.machine.unwrap_or_default();
            Ok(MachineDetail {
                machine: id,
                os: machine.os.clone(),
                arch: machine.arch.clone(),
                address: machine.address.clone(),
                reach: reach_of(&machine),
                daemon_version: machine.daemon_version.clone(),
                health: health_of(&shared, detail.health),
                folders,
                // Reach rules aren't served: empty rather than guessed.
                sessions,
                rules: Vec::new(),
                stop: machine
                    .sessions_stopped
                    .as_ref()
                    .map(|stopped| {
                        let now = crate::local_time::LocalNow::read();
                        crate::map::machine_stop(stopped, machine.stop_delivered, &now)
                    }),
            })
        })
    }

    fn skills_preview(&self, machine: &MachineId) -> Reply<SkillsPreview> {
        let shared = self.shared.clone();
        let state = self.state.clone();
        let machine = machine.clone();
        Box::pin(async move { read_plan(&shared, &state, &machine).await })
    }

    fn sync_skills(&self, machine: &MachineId) -> Reply<()> {
        const WHAT: &str = "Syncing skills";
        let shared = self.shared.clone();
        let this = self.clone();
        let machine = machine.clone();
        Box::pin(async move {
            let token = this.state().plans.get(&machine).cloned();
            let operation = skills_service(&shared)
                .sync_skills(pb::SyncSkillsRequest {
                    idempotency_key: fresh_key(),
                    machines: machine_id(&machine).into_iter().collect(),
                    plans: token.into_iter().collect(),
                })
                .await
                .map_err(|status| failure(&shared, &status, WHAT))?
                .into_inner();
            let done = follow(&shared, operation, WHAT, |_| {}).await?;
            succeeded(&done, WHAT)?;
            // A plan is used once.
            this.state().plans.remove(&machine);
            Ok(())
        })
    }

    fn resolve_clash(&self, machine: &MachineId, name: &str, choice: ClashChoice) -> Reply<()> {
        const WHAT: &str = "The skill clash";
        let shared = self.shared.clone();
        let this = self.clone();
        let machine = machine.clone();
        let name = name.to_owned();
        Box::pin(async move {
            let known = this.state().clashes.get(&machine).cloned();
            let clashes = match known {
                Some(clashes) => clashes,
                None => {
                    read_plan(&shared, &this.state, &machine).await?;
                    this.state().clashes.get(&machine).cloned().unwrap_or_default()
                }
            };
            let agents: Vec<String> = clashes
                .into_iter()
                .filter(|(_, skill)| *skill == name)
                .map(|(agent, _)| agent)
                .collect();
            if agents.is_empty() {
                return Err(DaemonError::NotFound {
                    what: format!("The clash on {name}"),
                });
            }
            let picked = match choice {
                ClashChoice::Synced => pb::skill_clash::Use::Synced,
                ClashChoice::MachineLocal => pb::skill_clash::Use::Local,
            };
            for agent in agents {
                skills_service(&shared)
                    .resolve_skill_clash(pb::ResolveSkillClashRequest {
                        idempotency_key: fresh_key(),
                        machine: machine_id(&machine),
                        agent,
                        skill: name.clone(),
                        r#use: picked as i32,
                    })
                    .await
                    .map_err(|status| failure(&shared, &status, WHAT))?;
            }
            Ok(())
        })
    }

    fn set_variable(&self, machine: &MachineId, name: &str, value: &str) -> Reply<()> {
        let shared = self.shared.clone();
        let request = pb::SetHostVariableRequest {
            idempotency_key: fresh_key(),
            id: machine_id(machine),
            name: name.to_owned(),
            value: value.to_owned(),
            presence: None,
        };
        Box::pin(async move {
            machine_service(&shared)
                .set_host_variable(request)
                .await
                .map_err(|status| failure(&shared, &status, "Setting a host variable"))?;
            Ok(())
        })
    }

    fn remove_variable(&self, machine: &MachineId, name: &str) -> Reply<()> {
        let shared = self.shared.clone();
        let request = pb::RemoveHostVariableRequest {
            idempotency_key: fresh_key(),
            id: machine_id(machine),
            name: name.to_owned(),
            presence: None,
        };
        Box::pin(async move {
            machine_service(&shared)
                .remove_host_variable(request)
                .await
                .map_err(|status| failure(&shared, &status, "Removing a host variable"))?;
            Ok(())
        })
    }

    fn stop_sessions(&self, machine: &MachineId) -> Reply<MachineStop> {
        let shared = self.shared.clone();
        if *machine == shared.machine {
            return stop_this_machine(shared);
        }
        let request = pb::StopMachineSessionsRequest {
            machine: machine_id(machine),
            // The host makes the id; the app sends one stop per confirm.
            stop_id: String::new(),
            reason: String::new(),
            via: pb::StopVia::App as i32,
        };
        Box::pin(async move {
            const WHAT: &str = "Stopping the machine's sessions";
            let reply = machine_service(&shared)
                .stop_machine_sessions(request)
                .await
                .map_err(|status| failure(&shared, &status, WHAT))?
                .into_inner();
            let stopped = reply.stopped.ok_or_else(|| DaemonError::Refused {
                what: WHAT.to_owned(),
                reason: "the host answered without the stop it made. Reload the machine's page to see whether its sessions stopped before trying again".to_owned(),
            })?;
            let now = crate::local_time::LocalNow::read();
            Ok(crate::map::machine_stop(&stopped, reply.delivered, &now))
        })
    }

    fn start_sessions(&self, machine: &MachineId) -> Reply<Started> {
        let shared = self.shared.clone();
        if *machine == shared.machine {
            // D265: this machine's own daemon starts its sessions.
            return Box::pin(async move {
                shared
                    .daemon_info()
                    .start_sessions(pb::StartSessionsRequest {})
                    .await
                    .map_err(|status| failure(&shared, &status, "Starting this Mac's sessions"))?;
                Ok(Started { cancelled: false })
            });
        }
        let request = pb::StartMachineSessionsRequest { machine: machine_id(machine) };
        Box::pin(async move {
            let reply = machine_service(&shared)
                .start_machine_sessions(request)
                .await
                .map_err(|status| failure(&shared, &status, "Starting the machine's sessions"))?
                .into_inner();
            Ok(Started { cancelled: reply.cancelled })
        })
    }

    fn scripts(&self, machine: Option<&MachineId>) -> Reply<MachineScripts> {
        let shared = self.shared.clone();
        let request = pb::ListMachineScriptsRequest { machine: machine.and_then(machine_id) };
        Box::pin(async move {
            let reply = machine_service(&shared)
                .list_machine_scripts(request)
                .await
                .map_err(|status| failure(&shared, &status, "Reading the machine scripts"))?
                .into_inner();
            // The warnings come from the check; a failed check leaves the list without them.
            let warnings = machine_service(&shared)
                .check_machine_scripts(pb::CheckMachineScriptsRequest {})
                .await
                .map(|checked| checked.into_inner().warnings)
                .unwrap_or_default();
            Ok(scripts_of(&reply, warnings, &LocalNow::read()))
        })
    }

    fn run_script(&self, machine: &MachineId, rule: &str, session: Option<&SessionId>) -> Reply<()> {
        const WHAT: &str = "Running the machine script";
        let shared = self.shared.clone();
        let request = pb::RunMachineScriptRequest {
            idempotency_key: fresh_key(),
            machine: machine_id(machine),
            rule: rule.to_owned(),
            session: session.map(|session| pb::SessionId { value: session.as_str().to_owned() }),
        };
        Box::pin(async move {
            let operation = machine_service(&shared)
                .run_machine_script(request)
                .await
                .map_err(|status| failure(&shared, &status, WHAT))?
                .into_inner();
            let done = follow(&shared, operation, WHAT, |_| {}).await?;
            succeeded(&done, WHAT)
        })
    }

    fn script_runs(&self, machine: &MachineId, rule: Option<&str>, limit: u32) -> Reply<Vec<ScriptRun>> {
        let shared = self.shared.clone();
        let request = pb::ListMachineScriptRunsRequest {
            machine: machine_id(machine),
            rule: rule.unwrap_or_default().to_owned(),
            limit,
        };
        Box::pin(async move {
            let reply = machine_service(&shared)
                .list_machine_script_runs(request)
                .await
                .map_err(|status| failure(&shared, &status, "Reading the machine script runs"))?
                .into_inner();
            let now = LocalNow::read();
            Ok(reply.runs.iter().map(|run| script_run_of(run, &now)).collect())
        })
    }

    fn plan_change_root(&self, machine: &MachineId, root: &str) -> Reply<RootPlan> {
        const WHAT: &str = "Planning the change of the Humata folder";
        let shared = self.shared.clone();
        let request = pb::PlanChangeRootRequest { id: machine_id(machine), root: root.to_owned() };
        let machine = machine.clone();
        Box::pin(async move {
            let plan = machine_service(&shared)
                .plan_change_root(request)
                .await
                .map_err(|status| failure(&shared, &status, WHAT))?
                .into_inner();
            let token = plan.token.map(|token| token.value).filter(|value| !value.is_empty()).ok_or_else(|| {
                DaemonError::Refused {
                    what: WHAT.to_owned(),
                    reason: "the daemon answered without a plan to apply. Try Plan again".to_owned(),
                }
            })?;
            Ok(RootPlan {
                machine,
                token,
                old_root: plan.old_root,
                new_root: plan.new_root,
                channels: plan.channels.into_iter().map(|channel| ChannelId::new(channel.value)).collect(),
                running: plan.running.into_iter().map(|session| SessionId::new(session.value)).collect(),
            })
        })
    }

    fn change_root(&self, plan: &RootPlan, move_channels: bool) -> Reply<()> {
        const WHAT: &str = "Changing the Humata folder";
        let shared = self.shared.clone();
        let request = pb::ChangeRootRequest {
            idempotency_key: fresh_key(),
            id: machine_id(&plan.machine),
            root: plan.new_root.clone(),
            // The app is the person: no presence proof is sent.
            presence: None,
            plan: Some(pb::PlanToken { value: plan.token.clone() }),
            move_channels,
        };
        Box::pin(async move {
            let operation = machine_service(&shared)
                .change_root(request)
                .await
                .map_err(|status| failure(&shared, &status, WHAT))?
                .into_inner();
            let done = follow(&shared, operation, WHAT, |_| {}).await?;
            succeeded(&done, WHAT)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(key: &str, state: pb::StepState, tail: &str) -> pb::OperationStep {
        pb::OperationStep {
            key: key.to_owned(),
            title: format!("title of {key}"),
            state: state as i32,
            output_tail: tail.to_owned(),
            ..Default::default()
        }
    }

    fn failed_add(steps: Vec<pb::OperationStep>, error: pb::Error) -> pb::Operation {
        pb::Operation {
            state: pb::OperationState::Failed as i32,
            steps,
            error: Some(error),
            ..Default::default()
        }
    }

    fn wire_run(outcome: pb::MachineScriptOutcome, step: pb::MachineScriptStepRun) -> pb::MachineScriptRun {
        pb::MachineScriptRun {
            id: "run-1".to_owned(),
            machine: Some(pb::MachineId { value: "devbox-1".to_owned() }),
            rule: "set-up-tools".to_owned(),
            trigger: "added".to_owned(),
            outcome: outcome as i32,
            steps: vec![step],
            ..pb::MachineScriptRun::default()
        }
    }

    fn wire_step(outcome: pb::MachineScriptOutcome, tail: &[&str]) -> pb::MachineScriptStepRun {
        pb::MachineScriptStepRun {
            index: 2,
            kind: "run".to_owned(),
            label: "setup.sh".to_owned(),
            outcome: outcome as i32,
            exit: Some(1),
            tail: tail.iter().map(|line| (*line).to_owned()).collect(),
            ..pb::MachineScriptStepRun::default()
        }
    }

    const NOW_FOR_SCRIPTS: LocalNow = LocalNow { unix_ms: 1_700_000_000_000, utc_offset_s: 0 };

    #[test]
    fn a_failed_script_run_keeps_the_failed_steps_exit_and_output() {
        let wire = wire_run(
            pb::MachineScriptOutcome::Failed,
            wire_step(pb::MachineScriptOutcome::Failed, &["a", "b"]),
        );

        let run = script_run_of(&wire, &NOW_FOR_SCRIPTS);

        assert_eq!(run.outcome, ScriptOutcome::Failed);
        assert_eq!(run.machine, MachineId::new("devbox-1"));
        let (step, tail) = run.failure_tail().expect("the failed step");
        assert_eq!((step.index, step.exit), (2, Some(1)));
        assert_eq!(tail, ["a".to_owned(), "b".to_owned()]);
    }

    #[test]
    fn every_script_outcome_the_host_names_maps_to_one_the_page_knows() {
        use pb::MachineScriptOutcome as O;
        assert_eq!(script_outcome_of(O::Passed as i32), ScriptOutcome::Passed);
        assert_eq!(script_outcome_of(O::TimedOut as i32), ScriptOutcome::TimedOut);
        assert_eq!(script_outcome_of(O::Skipped as i32), ScriptOutcome::Skipped);
        assert_eq!(script_outcome_of(O::Running as i32), ScriptOutcome::Running);
        // A value from a newer host is shown as going rather than as a result it isn't.
        assert_eq!(script_outcome_of(999), ScriptOutcome::Running);
    }

    #[test]
    fn a_rules_file_the_host_cant_use_comes_through_as_its_problem_with_the_fix() {
        let reply = pb::ListMachineScriptsResponse {
            problem: Some(pb::Error {
                message: "rule 2 names a script that isn't there.".to_owned(),
                fix: "Add it to ~/.humata2/scripts.".to_owned(),
                ..pb::Error::default()
            }),
            ..pb::ListMachineScriptsResponse::default()
        };

        let scripts = scripts_of(&reply, vec![], &NOW_FOR_SCRIPTS);

        assert!(scripts.rules.is_empty());
        let problem = scripts.problem.expect("a problem");
        assert!(problem.contains("rule 2") && problem.contains("~/.humata2/scripts"));
    }

    #[test]
    fn a_rule_keeps_its_triggers_machines_and_steps_in_order() {
        let rule = pb::MachineScriptRule {
            name: "set-up-tools".to_owned(),
            triggers: vec!["added".to_owned(), "connected".to_owned()],
            machines: vec!["devbox-1".to_owned()],
            once: true,
            steps: vec![
                pb::MachineScriptStep { kind: "run".to_owned(), label: "setup.sh".to_owned() },
                pb::MachineScriptStep { kind: "send".to_owned(), label: "/restart".to_owned() },
            ],
        };

        let view = script_rule_of(&rule);

        assert_eq!(view.triggers, ["added", "connected"]);
        assert!(view.once);
        assert_eq!(view.steps.iter().map(|step| step.kind.as_str()).collect::<Vec<_>>(), ["run", "send"]);
    }

    #[test]
    fn an_unknown_host_key_asks_to_confirm_the_fingerprint_it_presented() {
        let error = pb::Error {
            kind: Some(pb::error::Kind::HostKeyUnknown(pb::HostKeyUnknown {
                machine: None,
                algorithm: "ssh-ed25519".to_owned(),
                presented_fingerprint: "SHA256:abc".to_owned(),
            })),
            ..Default::default()
        };
        let operation = failed_add(vec![step("connect", pb::StepState::Failed, "")], error);
        assert_eq!(
            outcome_of_failure(&operation),
            AddOutcome::HostKey {
                fingerprint: "SHA256:abc".to_owned()
            }
        );
    }

    #[test]
    fn a_host_key_that_differs_from_the_one_expected_is_not_offered_for_confirming() {
        let error = pb::Error {
            message: "presents another key".to_owned(),
            kind: Some(pb::error::Kind::HostKeyChanged(pb::HostKeyChanged {
                machine: None,
                expected_fingerprint: "SHA256:old".to_owned(),
                presented_fingerprint: "SHA256:new".to_owned(),
            })),
            ..Default::default()
        };
        let operation = failed_add(vec![step("connect", pb::StepState::Failed, "")], error);
        assert!(
            matches!(outcome_of_failure(&operation), AddOutcome::Failed(_)),
            "{:?}",
            outcome_of_failure(&operation)
        );
    }

    #[test]
    fn a_login_the_person_must_do_gives_the_last_line_the_step_printed() {
        let error = pb::Error {
            kind: Some(pb::error::Kind::LoginNeedsUser(pb::LoginNeedsUser::default())),
            ..Default::default()
        };
        let operation = failed_add(
            vec![
                step("create", pb::StepState::Done, ""),
                step("connect", pb::StepState::Failed, "Welcome\nVerification code: \n"),
            ],
            error,
        );
        assert_eq!(
            outcome_of_failure(&operation),
            AddOutcome::NeedsLogin {
                reason: LoginReason::Prompted {
                    kind: PromptKind::Other,
                    line: "Verification code:".to_owned()
                }
            }
        );
    }

    #[test]
    fn a_host_behind_the_app_stops_the_add_with_its_platform_and_words() {
        let error = pb::Error {
            message: "The daemon is older.".to_owned(),
            fix: "Migrate it.".to_owned(),
            kind: Some(pb::error::Kind::HostNeedsMigrate(pb::HostNeedsMigrate {
                platform: "linux-x86_64".to_owned(),
            })),
            ..Default::default()
        };
        let operation = failed_add(vec![step("install", pb::StepState::Failed, "")], error);
        let AddOutcome::NeedsMigrate { platform, what } = outcome_of_failure(&operation) else {
            panic!("expected the add to need a migrate");
        };
        assert_eq!(platform, "linux-x86_64");
        assert!(!what.is_empty());
    }

    #[test]
    fn a_login_carries_the_daemons_kind_and_line_or_that_the_command_hung() {
        let needs = |needs: pb::LoginNeedsUser| {
            let error = pb::Error {
                kind: Some(pb::error::Kind::LoginNeedsUser(needs)),
                ..Default::default()
            };
            let operation = failed_add(vec![step("connect", pb::StepState::Failed, "Welcome\nIgnored:\n")], error);
            outcome_of_failure(&operation)
        };
        assert_eq!(
            needs(pb::LoginNeedsUser {
                prompt_kind: pb::LoginPromptKind::Password as i32,
                prompt_line: "Password:".to_owned(),
                ..Default::default()
            }),
            AddOutcome::NeedsLogin {
                reason: LoginReason::Prompted {
                    kind: PromptKind::Password,
                    line: "Password:".to_owned()
                }
            }
        );
        assert_eq!(
            needs(pb::LoginNeedsUser { hung: true, ..Default::default() }),
            AddOutcome::NeedsLogin { reason: LoginReason::Hung }
        );
    }

    #[test]
    fn a_failure_names_the_failed_step_and_keeps_the_daemons_text() {
        let error = pb::Error {
            message: "Install stopped.".to_owned(),
            fix: "Free some disk.".to_owned(),
            ..Default::default()
        };
        let operation = failed_add(
            vec![
                step("connect", pb::StepState::Done, ""),
                step("install", pb::StepState::Failed, "no space left"),
            ],
            error,
        );
        let AddOutcome::Failed(failure) = outcome_of_failure(&operation) else {
            panic!("expected a failure");
        };
        assert_eq!(failure.step, StepKind::Install);
        assert!(failure.what.contains("Install stopped."));
        assert!(failure.what.contains("Free some disk."));
        assert!(failure.what.contains("no space left"));
        assert_eq!(failure.fix, None);
    }

    #[test]
    fn a_failed_step_the_undo_took_back_is_still_the_one_that_failed() {
        let operation = failed_add(
            vec![
                step("connect", pb::StepState::Done, ""),
                step("check", pb::StepState::Done, ""),
                step("install", pb::StepState::Undone, "read-only"),
                step("start", pb::StepState::Waiting, ""),
                step("find_agents", pb::StepState::Waiting, ""),
                step("uninstall", pb::StepState::Skipped, ""),
                step("forget", pb::StepState::Done, ""),
            ],
            pb::Error::default(),
        );
        let AddOutcome::Failed(failure) = outcome_of_failure(&operation) else {
            panic!("expected a failure");
        };
        assert_eq!(failure.step, StepKind::Install);
        assert!(failure.what.contains("read-only"));
    }

    #[test]
    fn a_step_without_a_row_fails_at_the_last_row_before_it() {
        let operation = failed_add(
            vec![
                step("create", pb::StepState::Done, ""),
                step("identity", pb::StepState::Failed, "bad identity"),
                step("connect", pb::StepState::Waiting, ""),
            ],
            pb::Error::default(),
        );
        let AddOutcome::Failed(failure) = outcome_of_failure(&operation) else {
            panic!("expected a failure");
        };
        assert_eq!(failure.step, StepKind::Create);
    }

    #[test]
    fn steps_are_reported_once_and_again_only_when_they_change() {
        let (out, mut events) = mpsc::unbounded_channel();
        let mut reporter = Reporter::new(out);
        let mut operation = pb::Operation {
            id: Some(pb::OperationId {
                value: "op_1".to_owned(),
            }),
            state: pb::OperationState::Running as i32,
            steps: vec![
                step("connect", pb::StepState::Running, ""),
                step("check", pb::StepState::Waiting, ""),
            ],
            ..Default::default()
        };
        reporter.observe(&operation);
        reporter.observe(&operation);
        operation.steps[0].state = pb::StepState::Done as i32;
        reporter.observe(&operation);
        let mut seen = Vec::new();
        while let Ok(event) = events.try_recv() {
            seen.push(event.expect("no error"));
        }
        assert_eq!(
            seen[0],
            AddEvent::Started {
                operation: OperationId::new("op_1"),
                steps: vec![StepKind::Connect, StepKind::Check],
            }
        );
        let statuses: Vec<_> = seen[1..]
            .iter()
            .map(|event| match event {
                AddEvent::Step(update) => (update.step, update.status),
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(
            statuses,
            vec![
                (StepKind::Connect, StepStatus::Running),
                (StepKind::Connect, StepStatus::Done)
            ]
        );
    }

    #[test]
    fn a_skills_plan_lists_each_name_once_by_kind() {
        let change = |agent: &str, skill: &str, kind: pb::skill_change::Kind| pb::SkillChange {
            agent: agent.to_owned(),
            skill: skill.to_owned(),
            kind: kind as i32,
        };
        let plan = pb::SkillsSyncPlan {
            first_sync: true,
            changes: vec![
                change("claude", "alpha", pb::skill_change::Kind::Add),
                change("codex", "alpha", pb::skill_change::Kind::Add),
                change("claude", "beta", pb::skill_change::Kind::Update),
                change("claude", "gone", pb::skill_change::Kind::Remove),
            ],
            clashes: vec![pb::SkillClash {
                agent: "claude".to_owned(),
                skill: "beta".to_owned(),
                current: 0,
            }],
            ..Default::default()
        };
        let preview = preview_of(&plan);
        assert!(preview.first_sync);
        assert_eq!(preview.added, vec!["alpha".to_owned()]);
        assert_eq!(preview.updated, vec!["beta".to_owned()]);
        assert_eq!(preview.removed, vec!["gone".to_owned()]);
        assert_eq!(
            preview.clashes,
            vec![SkillClash {
                name: "beta".to_owned()
            }]
        );
    }

    /// A `check` step that finished and read these facts.
    fn check_step_done(facts: &[(&str, &str)]) -> pb::OperationStep {
        pb::OperationStep {
            facts: facts
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
            ..step("check", pb::StepState::Done, "")
        }
    }

    #[test]
    fn a_done_check_step_says_what_it_read() {
        let done = check_step_done(&[
            ("os", "linux"),
            ("arch", "x86_64"),
            ("git", "2.47.1"),
            ("gh", "2.83.0"),
            // 38.9 GB: whole GB, rounded down.
            ("disk_free_bytes", "38911234048"),
        ]);
        assert_eq!(step_detail(&done), "Linux x86_64 · git 2.47 · gh 2.83 · 38 GB free");
    }

    #[test]
    fn a_check_step_on_a_mac_calls_it_macos() {
        let done = check_step_done(&[
            ("os", "darwin"),
            ("arch", "aarch64"),
            ("git", "2.39.5"),
            ("gh", "2.83.0"),
            ("disk_free_bytes", "210000000000"),
        ]);
        assert_eq!(step_detail(&done), "macOS aarch64 · git 2.39 · gh 2.83 · 210 GB free");
    }

    #[test]
    fn a_check_step_leaves_out_what_the_machine_lacks() {
        let no_gh = check_step_done(&[
            ("os", "linux"),
            ("arch", "x86_64"),
            ("git", "2.47.1"),
            ("disk_free_bytes", "38911234048"),
        ]);
        assert_eq!(step_detail(&no_gh), "Linux x86_64 · git 2.47 · 38 GB free");

        let empty_gh = check_step_done(&[
            ("os", "linux"),
            ("arch", "x86_64"),
            ("git", "2.47.1"),
            ("gh", ""),
            ("disk_free_bytes", "38911234048"),
        ]);
        assert_eq!(step_detail(&empty_gh), "Linux x86_64 · git 2.47 · 38 GB free");

        let no_disk = check_step_done(&[
            ("os", "linux"),
            ("arch", "x86_64"),
            ("git", "2.47.1"),
            ("gh", "2.83.0"),
        ]);
        assert_eq!(step_detail(&no_disk), "Linux x86_64 · git 2.47 · gh 2.83");
    }

    #[test]
    fn a_check_step_without_facts_keeps_its_title() {
        assert_eq!(step_detail(&step("check", pb::StepState::Done, "")), "title of check");
    }

    #[test]
    fn the_dialog_gets_the_check_steps_line_when_the_step_is_done() {
        let (out, mut events) = mpsc::unbounded_channel();
        let mut reporter = Reporter::new(out);
        let mut operation = pb::Operation {
            id: Some(pb::OperationId {
                value: "op_1".to_owned(),
            }),
            state: pb::OperationState::Running as i32,
            steps: vec![
                step("connect", pb::StepState::Done, ""),
                step("check", pb::StepState::Running, ""),
            ],
            ..Default::default()
        };
        reporter.observe(&operation);
        operation.steps[1] = check_step_done(&[
            ("os", "linux"),
            ("arch", "x86_64"),
            ("git", "2.47.1"),
            ("gh", "2.83.0"),
            ("disk_free_bytes", "38911234048"),
        ]);
        reporter.observe(&operation);

        let mut last_check = None;
        while let Ok(event) = events.try_recv() {
            if let AddEvent::Step(update) = event.expect("no error") {
                if update.step == StepKind::Check {
                    last_check = Some(update);
                }
            }
        }
        assert_eq!(
            last_check,
            Some(StepUpdate {
                step: StepKind::Check,
                status: StepStatus::Done,
                detail: "Linux x86_64 · git 2.47 · gh 2.83 · 38 GB free".to_owned(),
            })
        );
    }
}

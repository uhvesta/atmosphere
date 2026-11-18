//! A scripted [`Machines`]: tests choose what each call does and read back
//! the calls made. No timers, no real IO.
//!
//! A stop of a machine's sessions is kept by the fake itself, or, when it
//! goes through a [`FakeDaemon`], on that daemon's workspace machine, whose
//! sessions then read Stopped until Start (D260–D265).

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::mpsc;

use super::{
    AddEvent, AddOutcome, AddRequest, AddStream, AddTarget, Choices, ClashChoice, LoginEvent, LoginStream,
    LoginTarget, MachineDetail, Machines, OperationId, SkillsPreview, Started, StepKind, StepStatus,
    MachineScripts, ProviderEntry, ProviderImportPlan, ScriptOutcome, ScriptRun, ScriptStepRun, StepUpdate,
    UnattendedMethod,
};
use crate::daemon::{Daemon, DaemonError, FakeDaemon, Reply};
use crate::model::{MachineId, MachineStop, RootPlan, SessionId, StoppedBy};

/// The clock time a fake stop is made at, as the design frames show it.
const STOPPED_AT: &str = "14:02";

/// The clock time a fake script run is made at.
const SCRIPT_RUN_AT: &str = "14:05";

/// A call a test can read back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    /// `choices`.
    Choices,
    /// `add`.
    Add(AddRequest),
    /// `retry`.
    Retry(OperationId),
    /// `cancel`.
    Cancel(OperationId),
    /// `open_login`.
    OpenLogin(LoginTarget),
    /// `login_input`.
    LoginInput(LoginTarget, Vec<u8>),
    /// `login_resize`.
    LoginResize(LoginTarget, u16, u16),
    /// `login_done`.
    LoginDone(LoginTarget),
    /// `login_cancel`.
    LoginCancel(LoginTarget),
    /// `make_unattended`.
    MakeUnattended(MachineId, UnattendedMethod),
    /// `detail`.
    Detail(MachineId),
    /// `skills_preview`.
    SkillsPreview(MachineId),
    /// `sync_skills`.
    SyncSkills(MachineId),
    /// `resolve_clash`.
    ResolveClash(MachineId, String, ClashChoice),
    /// `set_variable`, with the value sent.
    SetVariable(MachineId, String, String),
    /// `remove_variable`.
    RemoveVariable(MachineId, String),
    /// `stop_sessions`.
    StopSessions(MachineId),
    /// `start_sessions`.
    StartSessions(MachineId),
    /// `plan_change_root`, with the root asked for.
    PlanChangeRoot(MachineId, String),
    /// `change_root`: the machine, the plan's new root, and `move_channels`.
    ChangeRoot(MachineId, String, bool),
    /// `scripts`, with the machine asked for.
    Scripts(Option<MachineId>),
    /// `run_script`: the machine, the rule and the session.
    RunScript(MachineId, String, Option<SessionId>),
    /// `script_runs`: the machine, the rule and the limit.
    ScriptRuns(MachineId, Option<String>, u32),
    /// `providers`.
    Providers,
    /// `preview_provider_import`, with the pasted text.
    PreviewProviderImport(String),
    /// `import_provider`, with the plan's token.
    ImportProvider(Option<String>),
    /// `remove_provider`.
    RemoveProvider(String),
    /// `set_provider_variable`, with the value sent.
    SetProviderVariable(String, String),
    /// `clear_provider_variable`.
    ClearProviderVariable(String),
}

/// What the next login does: what it prints on opening, and what comes after
/// the person presses Done.
#[derive(Clone, Debug, Default)]
struct LoginScript {
    opening: Vec<LoginEvent>,
    after_done: Vec<LoginEvent>,
}

/// What the next add or retry does: its events, and whether its stream then stays open.
#[derive(Clone, Debug, Default)]
struct AddScript {
    events: Vec<AddEvent>,
    held: bool,
}

type AddSender = mpsc::UnboundedSender<Result<AddEvent, DaemonError>>;

struct Inner {
    choices: Choices,
    adds: VecDeque<AddScript>,
    /// The open streams of held adds, by the operation they belong to.
    held: Vec<(Option<OperationId>, AddSender)>,
    logins: VecDeque<LoginScript>,
    pending_login: Option<(LoginScript, mpsc::UnboundedSender<Result<LoginEvent, DaemonError>>)>,
    detail: Result<MachineDetail, DaemonError>,
    preview: Result<SkillsPreview, DaemonError>,
    failures: HashMap<&'static str, DaemonError>,
    calls: Vec<Call>,
    /// The fake daemon an unscripted SSH add and every stop and start go
    /// through, if any.
    through: Option<FakeDaemon>,
    /// Each machine's stop, when no daemon is gone through.
    stops: HashMap<MachineId, MachineStop>,
    /// How many stops were made, for their ids.
    stops_made: u32,
    /// What `plan_change_root` answers, when a test scripted it.
    root_plan: Option<Result<RootPlan, DaemonError>>,
    /// Each machine's Humata folder, when no daemon is gone through.
    roots: HashMap<MachineId, String>,
    /// What `scripts` answers.
    scripts: Result<MachineScripts, DaemonError>,
    /// How the next runs of a rule end, with the output they leave.
    script_outcomes: HashMap<String, (ScriptOutcome, Vec<String>)>,
    /// The runs `run_script` made, oldest first.
    script_log: Vec<ScriptRun>,
    /// How many runs were made, for their ids.
    scripts_run: u32,
    /// The providers `providers` lists; imports and removes change it.
    providers: Vec<ProviderEntry>,
    /// What `preview_provider_import` answers, whatever the text.
    import_plan: Result<ProviderImportPlan, DaemonError>,
}

/// A machines client that does what a test scripts.
#[derive(Clone)]
pub struct FakeMachines {
    shared: Arc<Mutex<Inner>>,
}

impl Default for FakeMachines {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeMachines {
    /// A fake with no hosts or providers, where an add with no script
    /// reports nothing and a detail is not found.
    #[must_use]
    pub fn new() -> Self {
        Self {
            shared: Arc::new(Mutex::new(Inner {
                choices: Choices::default(),
                adds: VecDeque::new(),
                held: Vec::new(),
                logins: VecDeque::new(),
                pending_login: None,
                detail: Err(DaemonError::NotFound { what: "this machine".to_owned() }),
                preview: Ok(SkillsPreview::default()),
                failures: HashMap::new(),
                calls: Vec::new(),
                through: None,
                stops: HashMap::new(),
                stops_made: 0,
                root_plan: None,
                roots: HashMap::new(),
                scripts: Ok(MachineScripts::default()),
                script_outcomes: HashMap::new(),
                script_log: Vec::new(),
                scripts_run: 0,
                providers: Vec::new(),
                import_plan: Ok(ProviderImportPlan::default()),
            })),
        }
    }

    /// Makes an SSH add that has no script go through `daemon`, the way
    /// the app's fakes added a machine before the dialog: the machine
    /// appears in the fake workspace and every step reports done. A
    /// scripted add still plays its script.
    ///
    /// Stops and starts go through it too: a stop is kept on the
    /// workspace's machine, delivered when the machine is online, and its
    /// sessions read Stopped until Start; a machine the workspace doesn't
    /// have is not found.
    #[must_use]
    pub fn through(self, daemon: FakeDaemon) -> Self {
        self.inner().through = Some(daemon);
        self
    }

    /// Sets `machine`'s stop as if it had been made earlier: `None` means
    /// its sessions run. With a daemon gone through, the workspace's
    /// machine and sessions change as a stop or start would change them.
    pub fn set_stop(&self, machine: &MachineId, stop: Option<MachineStop>) {
        let through = self.inner().through.clone();
        match through {
            Some(daemon) => daemon.set_machine_stop(machine, stop),
            None => {
                let mut inner = self.inner();
                match stop {
                    Some(stop) => inner.stops.insert(machine.clone(), stop),
                    None => inner.stops.remove(machine),
                };
            }
        }
    }

    /// `machine`'s stop now, and whether its daemon answers; an error when
    /// a daemon is gone through and has no such machine.
    fn stop_of(&self, machine: &MachineId) -> Result<(Option<MachineStop>, bool), DaemonError> {
        let inner = self.inner();
        match &inner.through {
            Some(daemon) => daemon
                .workspace_machine(machine)
                .map(|found| (found.stop, found.online))
                .ok_or_else(|| DaemonError::NotFound { what: format!("Host {machine}") }),
            None => Ok((inner.stops.get(machine).cloned(), true)),
        }
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// What the form offers.
    pub fn set_choices(&self, choices: Choices) {
        self.inner().choices = choices;
    }

    /// What the next add or retry reports, in order. Scripts are used one
    /// per call, oldest first; the last event should be an outcome.
    pub fn script_add(&self, events: Vec<AddEvent>) {
        self.inner().adds.push_back(AddScript { events, held: false });
    }

    /// Like [`Self::script_add`], but the stream stays open after its events, as a running
    /// add's does, until [`Self::release_adds`] or `cancel` for its operation (the one in
    /// its `Started` event, else the one a retry names).
    pub fn script_add_held(&self, events: Vec<AddEvent>) {
        self.inner().adds.push_back(AddScript { events, held: true });
    }

    /// Ends every stream `script_add_held` is keeping open.
    pub fn release_adds(&self) {
        self.inner().held.clear();
    }

    /// What the next login prints on opening, and what follows Done.
    pub fn script_login(&self, opening: Vec<LoginEvent>, after_done: Vec<LoginEvent>) {
        self.inner().logins.push_back(LoginScript { opening, after_done });
    }

    /// What `detail` returns from now on.
    pub fn set_detail(&self, detail: Result<MachineDetail, DaemonError>) {
        self.inner().detail = detail;
    }

    /// What `plan_change_root` answers from now on, whatever root is asked
    /// for. Without it the fake plans against the daemon it goes through:
    /// the channels on the machine and the sessions running there.
    pub fn set_root_plan(&self, plan: Result<RootPlan, DaemonError>) {
        self.inner().root_plan = Some(plan);
    }

    /// What `skills_preview` returns from now on.
    pub fn set_preview(&self, preview: Result<SkillsPreview, DaemonError>) {
        self.inner().preview = preview;
    }

    /// What `scripts` answers from now on. A run made by `run_script` replaces
    /// its rule's last run in it.
    pub fn set_scripts(&self, scripts: Result<MachineScripts, DaemonError>) {
        self.inner().scripts = scripts;
    }

    /// How the next runs of `rule` end, and the output tail they leave. A rule
    /// with no outcome set passes. A run that failed or timed out also makes
    /// `run_script` fail, as the daemon's operation does.
    pub fn set_script_outcome(&self, rule: &str, outcome: ScriptOutcome, tail: Vec<String>) {
        self.inner().script_outcomes.insert(rule.to_owned(), (outcome, tail));
    }

    /// The providers `providers` lists from now on.
    pub fn set_providers(&self, providers: Vec<ProviderEntry>) {
        self.inner().providers = providers;
    }

    /// What `preview_provider_import` answers from now on, whatever the
    /// text. Importing the plan adds its providers to the list.
    pub fn set_import_plan(&self, plan: Result<ProviderImportPlan, DaemonError>) {
        self.inner().import_plan = plan;
    }

    /// Makes the named call fail with `error` until cleared. Names are the
    /// trait's method names, such as `"login_input"` or `"set_variable"`.
    pub fn fail(&self, method: &'static str, error: DaemonError) {
        self.inner().failures.insert(method, error);
    }

    /// The calls made so far, oldest first.
    #[must_use]
    pub fn calls(&self) -> Vec<Call> {
        self.inner().calls.clone()
    }

    fn call<T: Send + 'static>(&self, method: &'static str, call: Call, value: T) -> Reply<T> {
        let mut inner = self.inner();
        inner.calls.push(call);
        let result = match inner.failures.get(method) {
            Some(error) => Err(error.clone()),
            None => Ok(value),
        };
        Box::pin(std::future::ready(result))
    }

    fn add_stream(
        &self,
        method: &'static str,
        call: Call,
        retried: Option<&OperationId>,
    ) -> Reply<AddStream> {
        let mut inner = self.inner();
        let script = inner.adds.pop_front().unwrap_or_default();
        let (sender, receiver) = mpsc::unbounded_channel();
        let operation = script
            .events
            .iter()
            .find_map(|event| match event {
                AddEvent::Started { operation, .. } => Some(operation.clone()),
                _ => None,
            })
            .or_else(|| retried.cloned());
        for event in script.events {
            let _ = sender.send(Ok(event));
        }
        if script.held {
            inner.held.push((operation, sender));
        }
        drop(inner);
        self.call(method, call, receiver)
    }
}

impl Machines for FakeMachines {
    fn choices(&self) -> Reply<Choices> {
        let choices = self.inner().choices.clone();
        self.call("choices", Call::Choices, choices)
    }

    fn add(&self, request: AddRequest) -> Reply<AddStream> {
        // With no script waiting, an SSH add goes through the fake daemon
        // when one was given: the machine joins the fake workspace.
        let through = {
            let inner = self.inner();
            if inner.adds.is_empty() && !inner.failures.contains_key("add") {
                inner.through.clone().map(|daemon| Arc::new(daemon) as Arc<dyn Daemon>)
            } else {
                None
            }
        };
        let (Some(daemon), AddTarget::Ssh { host }) = (through, &request.target) else {
            return self.add_stream("add", Call::Add(request), None);
        };
        let host = host.clone();
        self.inner().calls.push(Call::Add(request));
        Box::pin(async move {
            let (sender, receiver) = mpsc::unbounded_channel();
            let steps = vec![
                StepKind::Connect,
                StepKind::Check,
                StepKind::Install,
                StepKind::Start,
                StepKind::FindAgents,
            ];
            let _ = sender.send(Ok(AddEvent::Started {
                operation: OperationId::new(format!("add-{host}")),
                steps: steps.clone(),
            }));
            match daemon.add_machine(&host).await {
                Ok(machine) => {
                    for step in steps {
                        let _ = sender.send(Ok(AddEvent::Step(StepUpdate {
                            step,
                            status: StepStatus::Done,
                            detail: String::new(),
                        })));
                    }
                    let _ = sender.send(Ok(AddEvent::Outcome(AddOutcome::Added(machine))));
                }
                Err(error) => {
                    let _ = sender.send(Err(error));
                }
            }
            Ok(receiver)
        })
    }

    fn retry(&self, operation: &OperationId) -> Reply<AddStream> {
        self.add_stream("retry", Call::Retry(operation.clone()), Some(operation))
    }

    fn cancel(&self, operation: &OperationId) -> Reply<()> {
        // A cancelled operation's stream ends.
        self.inner().held.retain(|(held, _)| held.as_ref() != Some(operation));
        self.call("cancel", Call::Cancel(operation.clone()), ())
    }

    fn open_login(&self, target: &LoginTarget) -> Reply<LoginStream> {
        let script = self.inner().logins.pop_front().unwrap_or_default();
        let (sender, receiver) = mpsc::unbounded_channel();
        for event in &script.opening {
            let _ = sender.send(Ok(event.clone()));
        }
        // Held open until Done, so the stream doesn't end before the person is.
        self.inner().pending_login = Some((script, sender));
        self.call("open_login", Call::OpenLogin(target.clone()), receiver)
    }

    fn login_input(&self, target: &LoginTarget, bytes: Vec<u8>) -> Reply<()> {
        self.call("login_input", Call::LoginInput(target.clone(), bytes), ())
    }

    fn login_resize(&self, target: &LoginTarget, columns: u16, rows: u16) -> Reply<()> {
        self.call("login_resize", Call::LoginResize(target.clone(), columns, rows), ())
    }

    fn login_done(&self, target: &LoginTarget) -> Reply<()> {
        if let Some((script, sender)) = self.inner().pending_login.take() {
            for event in script.after_done {
                let _ = sender.send(Ok(event));
            }
        }
        self.call("login_done", Call::LoginDone(target.clone()), ())
    }

    fn login_cancel(&self, target: &LoginTarget) -> Reply<()> {
        self.inner().pending_login = None;
        self.call("login_cancel", Call::LoginCancel(target.clone()), ())
    }

    fn make_unattended(&self, machine: &MachineId, method: UnattendedMethod) -> Reply<()> {
        self.call("make_unattended", Call::MakeUnattended(machine.clone(), method), ())
    }

    fn detail(&self, machine: &MachineId) -> Reply<MachineDetail> {
        let result = {
            let mut inner = self.inner();
            inner.calls.push(Call::Detail(machine.clone()));
            match inner.failures.get("detail") {
                Some(error) => Err(error.clone()),
                None => inner.detail.clone(),
            }
        };
        // The scripted detail carries the machine's stop as the fake keeps it.
        let result = result.map(|mut detail| {
            detail.stop = self.stop_of(&detail.machine).ok().and_then(|(stop, _)| stop);
            detail
        });
        Box::pin(std::future::ready(result))
    }

    fn skills_preview(&self, machine: &MachineId) -> Reply<SkillsPreview> {
        let mut inner = self.inner();
        inner.calls.push(Call::SkillsPreview(machine.clone()));
        let result = match inner.failures.get("skills_preview") {
            Some(error) => Err(error.clone()),
            None => inner.preview.clone(),
        };
        Box::pin(std::future::ready(result))
    }

    fn sync_skills(&self, machine: &MachineId) -> Reply<()> {
        self.call("sync_skills", Call::SyncSkills(machine.clone()), ())
    }

    fn resolve_clash(&self, machine: &MachineId, name: &str, choice: ClashChoice) -> Reply<()> {
        self.call("resolve_clash", Call::ResolveClash(machine.clone(), name.to_owned(), choice), ())
    }

    fn set_variable(&self, machine: &MachineId, name: &str, value: &str) -> Reply<()> {
        self.call(
            "set_variable",
            Call::SetVariable(machine.clone(), name.to_owned(), value.to_owned()),
            (),
        )
    }

    fn remove_variable(&self, machine: &MachineId, name: &str) -> Reply<()> {
        self.call("remove_variable", Call::RemoveVariable(machine.clone(), name.to_owned()), ())
    }

    fn scripts(&self, machine: Option<&MachineId>) -> Reply<MachineScripts> {
        let mut inner = self.inner();
        inner.calls.push(Call::Scripts(machine.cloned()));
        let result = match inner.failures.get("scripts") {
            Some(error) => Err(error.clone()),
            None => inner.scripts.clone().map(|mut scripts| {
                if let Some(machine) = machine {
                    scripts.last_runs.retain(|run| &run.machine == machine);
                }
                scripts
            }),
        };
        Box::pin(std::future::ready(result))
    }

    fn run_script(&self, machine: &MachineId, rule: &str, session: Option<&SessionId>) -> Reply<()> {
        let mut inner = self.inner();
        inner.calls.push(Call::RunScript(machine.clone(), rule.to_owned(), session.cloned()));
        if let Some(error) = inner.failures.get("run_script") {
            return Box::pin(std::future::ready(Err(error.clone())));
        }
        let (outcome, tail) = inner
            .script_outcomes
            .get(rule)
            .cloned()
            .unwrap_or((ScriptOutcome::Passed, Vec::new()));
        inner.scripts_run += 1;
        let run = ScriptRun {
            id: format!("run-{}", inner.scripts_run),
            machine: machine.clone(),
            rule: rule.to_owned(),
            trigger: "manual".to_owned(),
            session: session.cloned(),
            at: SCRIPT_RUN_AT.to_owned(),
            took_ms: 1200,
            outcome,
            steps: vec![ScriptStepRun {
                index: 0,
                kind: "run".to_owned(),
                label: rule.to_owned(),
                outcome,
                exit: Some(i32::from(outcome.is_problem())),
                tail: tail.clone(),
                error: String::new(),
            }],
        };
        if let Ok(scripts) = &mut inner.scripts {
            scripts.last_runs.retain(|last| !(last.rule == run.rule && last.machine == run.machine));
            scripts.last_runs.push(run.clone());
        }
        inner.script_log.push(run);
        let result = if outcome.is_problem() {
            Err(DaemonError::Refused {
                what: format!("Running {rule} on {machine}"),
                reason: tail.join("\n"),
            })
        } else {
            Ok(())
        };
        Box::pin(std::future::ready(result))
    }

    fn script_runs(&self, machine: &MachineId, rule: Option<&str>, limit: u32) -> Reply<Vec<ScriptRun>> {
        let mut inner = self.inner();
        inner.calls.push(Call::ScriptRuns(machine.clone(), rule.map(str::to_owned), limit));
        let result = match inner.failures.get("script_runs") {
            Some(error) => Err(error.clone()),
            None => {
                let mut runs: Vec<ScriptRun> = inner
                    .script_log
                    .iter()
                    .rev()
                    .filter(|run| &run.machine == machine && rule.is_none_or(|rule| run.rule == rule))
                    .cloned()
                    .collect();
                if limit > 0 {
                    runs.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
                }
                Ok(runs)
            }
        };
        Box::pin(std::future::ready(result))
    }

    fn providers(&self) -> Reply<Vec<ProviderEntry>> {
        let providers = self.inner().providers.clone();
        self.call("providers", Call::Providers, providers)
    }

    fn preview_provider_import(&self, text: &str) -> Reply<ProviderImportPlan> {
        let mut inner = self.inner();
        inner.calls.push(Call::PreviewProviderImport(text.to_owned()));
        let result = match inner.failures.get("preview_provider_import").cloned() {
            Some(error) => Err(error),
            None => inner.import_plan.clone(),
        };
        Box::pin(std::future::ready(result))
    }

    fn import_provider(&self, plan: &ProviderImportPlan) -> Reply<()> {
        let mut inner = self.inner();
        inner.calls.push(Call::ImportProvider(plan.token.clone()));
        let result = match (inner.failures.get("import_provider").cloned(), &plan.token) {
            (Some(error), _) => Err(error),
            (None, None) => Err(DaemonError::Refused {
                what: "Importing the provider file".to_owned(),
                reason: "the file has problems, so nothing was added. Fix them and review it again".to_owned(),
            }),
            (None, Some(_)) => {
                for imported in &plan.providers {
                    inner.providers.retain(|kept| kept.id != imported.id);
                    inner.providers.push(imported.clone());
                }
                Ok(())
            }
        };
        Box::pin(std::future::ready(result))
    }

    fn remove_provider(&self, provider: &str) -> Reply<()> {
        let mut inner = self.inner();
        inner.calls.push(Call::RemoveProvider(provider.to_owned()));
        let what = format!("Removing the provider {provider}");
        let result = match inner.failures.get("remove_provider").cloned() {
            Some(error) => Err(error),
            None => match inner.providers.iter().position(|kept| kept.id == provider) {
                None => Err(DaemonError::NotFound { what: format!("The provider {provider}") }),
                Some(at) if inner.providers[at].built_in => Err(DaemonError::Refused {
                    what,
                    reason: "it is built in to Humata, so it can't be removed".to_owned(),
                }),
                Some(at) if !inner.providers[at].machines.is_empty() => Err(DaemonError::Refused {
                    what,
                    reason: format!(
                        "it still runs {}. Delete those machines first",
                        inner.providers[at].machines.join(" and ")
                    ),
                }),
                Some(at) => {
                    inner.providers.remove(at);
                    Ok(())
                }
            },
        };
        Box::pin(std::future::ready(result))
    }

    fn set_provider_variable(&self, name: &str, value: &str) -> Reply<()> {
        let mut inner = self.inner();
        inner.calls.push(Call::SetProviderVariable(name.to_owned(), value.to_owned()));
        let result = match inner.failures.get("set_provider_variable").cloned() {
            Some(error) => Err(error),
            None => {
                mark_variable(&mut inner.providers, name, true);
                Ok(())
            }
        };
        Box::pin(std::future::ready(result))
    }

    fn clear_provider_variable(&self, name: &str) -> Reply<()> {
        let mut inner = self.inner();
        inner.calls.push(Call::ClearProviderVariable(name.to_owned()));
        let result = match inner.failures.get("clear_provider_variable").cloned() {
            Some(error) => Err(error),
            None => {
                mark_variable(&mut inner.providers, name, false);
                Ok(())
            }
        };
        Box::pin(std::future::ready(result))
    }

    fn stop_sessions(&self, machine: &MachineId) -> Reply<MachineStop> {
        let failed = {
            let mut inner = self.inner();
            inner.calls.push(Call::StopSessions(machine.clone()));
            inner.failures.get("stop_sessions").cloned()
        };
        let result = match failed {
            Some(error) => Err(error),
            None => self.stop_of(machine).and_then(|(stop, online)| {
                if stop.is_some() {
                    return Err(DaemonError::Refused {
                        what: format!("Stopping the sessions on {machine}"),
                        reason: "they are already stopped; Start sessions first to stop them again".to_owned(),
                    });
                }
                let stop = {
                    let mut inner = self.inner();
                    inner.stops_made += 1;
                    MachineStop {
                        stop_id: format!("stop-{}", inner.stops_made),
                        at: STOPPED_AT.to_owned(),
                        by: StoppedBy::You,
                        delivered: online,
                    }
                };
                self.set_stop(machine, Some(stop.clone()));
                Ok(stop)
            }),
        };
        Box::pin(std::future::ready(result))
    }

    fn start_sessions(&self, machine: &MachineId) -> Reply<Started> {
        let failed = {
            let mut inner = self.inner();
            inner.calls.push(Call::StartSessions(machine.clone()));
            inner.failures.get("start_sessions").cloned()
        };
        let result = match failed {
            Some(error) => Err(error),
            None => self.stop_of(machine).and_then(|(stop, _)| {
                let Some(stop) = stop else {
                    return Err(DaemonError::Refused {
                        what: format!("Starting the sessions on {machine}"),
                        reason: "they aren't stopped, so there is nothing to start".to_owned(),
                    });
                };
                self.set_stop(machine, None);
                Ok(Started { cancelled: !stop.delivered })
            }),
        };
        Box::pin(std::future::ready(result))
    }

    fn plan_change_root(&self, machine: &MachineId, root: &str) -> Reply<RootPlan> {
        let (failed, scripted, through, kept) = {
            let mut inner = self.inner();
            inner.calls.push(Call::PlanChangeRoot(machine.clone(), root.to_owned()));
            (
                inner.failures.get("plan_change_root").cloned(),
                inner.root_plan.clone(),
                inner.through.clone(),
                inner.roots.get(machine).cloned(),
            )
        };
        let result = match (failed, scripted, through) {
            (Some(error), _, _) => Err(error),
            (None, Some(plan), _) => plan,
            (None, None, Some(daemon)) => daemon.plan_root(machine, root),
            (None, None, None) => Ok(RootPlan {
                machine: machine.clone(),
                token: format!("plan-{machine}-{root}"),
                old_root: kept.unwrap_or_else(|| crate::desk::DEFAULT_ROOT.to_owned()),
                new_root: root.to_owned(),
                channels: Vec::new(),
                running: Vec::new(),
            }),
        };
        Box::pin(std::future::ready(result))
    }

    fn change_root(&self, plan: &RootPlan, move_channels: bool) -> Reply<()> {
        let (failed, through) = {
            let mut inner = self.inner();
            inner.calls.push(Call::ChangeRoot(plan.machine.clone(), plan.new_root.clone(), move_channels));
            (inner.failures.get("change_root").cloned(), inner.through.clone())
        };
        let result = match failed {
            Some(error) => Err(error),
            None if move_channels && !plan.running.is_empty() => Err(DaemonError::Refused {
                what: format!("Moving the channels in {}'s Humata folder", plan.machine),
                reason: format!(
                    "{} session(s) are running there, so the channels can't be moved now. Stop them and try again, or use the new folder for new channels only",
                    plan.running.len()
                ),
            }),
            None => match through {
                Some(daemon) => daemon.apply_root(&plan.machine, &plan.new_root),
                None => {
                    self.inner().roots.insert(plan.machine.clone(), plan.new_root.clone());
                    Ok(())
                }
            },
        };
        Box::pin(std::future::ready(result))
    }
}

/// Every provider that reads the host variable `name` now has it set, or not.
fn mark_variable(providers: &mut [ProviderEntry], name: &str, set: bool) {
    for variable in providers.iter_mut().flat_map(|provider| provider.variables.iter_mut()) {
        if variable.name == name {
            variable.set = set;
        }
    }
}

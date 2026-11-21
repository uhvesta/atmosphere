//! Add machine (HO-1, HO-2, MP-12 to MP-17; frames 56 to 58): pick an SSH
//! host or a provider, follow the steps, and deal with what stops them: a
//! failure with its fix, a host key to confirm, or a login the person does
//! in a terminal inside the dialog.

use std::collections::BTreeMap;
use std::sync::Arc;

use humata2_clients::DaemonError;
use humata2_clients::machines::{
    AddFailure, AddEvent, AddOutcome, AddRequest, AddTarget, Choices, LoginEvent, LoginReason, LoginTarget,
    PromptKind,
    Machines, OperationId, Provider, SshHost, StepKind, StepStatus, StepUpdate, UnattendedMethod,
};
use humata2_clients::model::{
    Answer, MachineId, MigrateFailure, MigratePhase, MigrateThing, Request, RequestId,
};
use tca::{Effect, Reducer, StreamEnd};

const PROGRESS: &str = "add-machine-progress";
const LOGIN: &str = "add-machine-login";

/// What is being added.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    /// An SSH host: picked from the list, or typed.
    Host(String),
    /// A provider, with the value of each of its params.
    Provider {
        /// The provider's id.
        id: String,
        /// Values by param name, starting from the defaults.
        params: BTreeMap<String, String>,
    },
}

/// One row of the steps list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    /// Which step.
    pub kind: StepKind,
    /// Where it is.
    pub status: StepStatus,
    /// The line under it.
    pub detail: String,
}

/// Why the login terminal opened (D167).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginWhy {
    /// The add's connect printed this kind of prompt.
    Prompted(PromptKind),
    /// The add's connect ran until its timeout without finishing.
    Hung,
    /// The provider is marked interactive (MP-12) and the connect didn't finish.
    AlwaysInteractive,
    /// The person opened it, or the daemon named no prompt.
    Asked,
}

/// The sentence above the login terminal (D167, frame 57). `who` is the
/// provider's id, or `ssh` for a plain SSH host; `machine` is what the
/// machine is called in the dialog. Fixed when the terminal opens.
#[must_use]
pub fn login_intro(who: &str, why: LoginWhy, machine: &str) -> String {
    let clause = match why {
        LoginWhy::Prompted(PromptKind::Code) => "is asking for a verification code",
        LoginWhy::Prompted(PromptKind::Password) => "is asking for a password",
        LoginWhy::Prompted(PromptKind::Passphrase) => "is asking for a passphrase",
        LoginWhy::Prompted(PromptKind::Confirm) => "is asking a yes or no question",
        LoginWhy::Prompted(PromptKind::Device) => "is asking you to sign in on a web page",
        LoginWhy::Hung => "didn't finish connecting on its own",
        LoginWhy::AlwaysInteractive => "is set up to log in by hand",
        LoginWhy::Prompted(PromptKind::Other) | LoginWhy::Asked => {
            return format!("Humata opened the login to {machine} here. Finish logging in, then press Done.");
        }
    };
    format!("{who} {clause}, so Humata opened it here. Finish logging in, then press Done.")
}

/// The line under the "Log in" step once it worked (D168, frame 58). `why`
/// is the login's first prompt if one arrived, else the reason it opened.
/// "Accepted" is inferred from a prompt followed by the machine answering.
#[must_use]
pub fn login_detail(who: &str, why: LoginWhy, machine: &str) -> String {
    let tail = match why {
        LoginWhy::Prompted(PromptKind::Code) => format!("{who} accepted the code"),
        LoginWhy::Prompted(PromptKind::Password) => format!("{who} accepted the password"),
        LoginWhy::Prompted(PromptKind::Passphrase) => format!("{who} accepted the passphrase"),
        LoginWhy::Prompted(PromptKind::Device) => format!("{who} accepted the sign-in"),
        LoginWhy::Prompted(PromptKind::Confirm | PromptKind::Other)
        | LoginWhy::Hung
        | LoginWhy::AlwaysInteractive
        | LoginWhy::Asked => format!("{machine} answered"),
    };
    format!("Finished in the terminal; {tail}")
}

/// A login the person is doing in the dialog's terminal (frame 57).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Login {
    /// What it belongs to.
    pub target: LoginTarget,
    /// Why it opened.
    pub why: LoginWhy,
    /// The kind of the first prompt the terminal showed, once one did.
    pub prompt: Option<PromptKind>,
    /// Everything the terminal printed so far, for the view to draw.
    pub bytes: Vec<u8>,
    /// "Done, I'm logged in" is waiting for the answer.
    pub finishing: bool,
    /// Why the login didn't work, to show next to Done.
    pub error: Option<String>,
}

/// The offer after a login worked (frame 58, MP-17).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unattended {
    /// The machine that is added.
    pub machine: MachineId,
    /// The chosen way.
    pub method: UnattendedMethod,
    /// The request is in flight.
    pub working: bool,
    /// The id of the provider the add used; none for a plain SSH host, which has no
    /// cached token to keep (MP-17).
    pub provider: Option<String>,
    /// humatad's version on the machine, when the daemon knew it (frame 58).
    pub version: Option<String>,
}

/// Where an add blocked by This Mac's daemon being behind this app is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MigrateStage {
    /// Waiting for the person to migrate.
    Blocked,
    /// The Migrate item was approved; the add runs again when it is migrated.
    Migrating(RequestId),
    /// The migrate failed or didn't start; it can be tried again.
    Failed(MigrateFailure),
}

/// What the dialog says when an add waits for This Mac's daemon to be migrated (D752).
const HELD_BEHIND_MIGRATE: &str = "This Mac's daemon is at another release than this app. Migrate This Mac first; the add runs once it is migrated.";

/// An add that stopped because This Mac's daemon is behind this app (UP-1, D370).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostMigrate {
    /// The platform the tools are for; empty when the add waited for a Migrate item before it
    /// ran (D752).
    pub platform: String,
    /// The daemon's words: what stopped and how to fix it.
    pub what: String,
    /// Where the migrate is.
    pub stage: MigrateStage,
}

/// The dialog's state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AddMachineState {
    /// Whether the dialog is showing.
    pub open: bool,
    /// The choices are being loaded.
    pub loading: bool,
    /// The SSH hosts to pick from.
    pub hosts: Vec<SshHost>,
    /// The providers to pick from.
    pub providers: Vec<Provider>,
    /// What is chosen; none until the person picks or types.
    pub choice: Option<Choice>,
    /// A host typed in the field next to the pop-up; when not empty it wins over the pick.
    pub typed: String,
    /// The Host pop-up is open (frame 56).
    pub host_menu: bool,
    /// The name to give the machine; empty uses the host name for SSH, auto-names Lima VMs (`lima-1`, `lima-2`, …), or requires a name for other providers.
    pub name: String,
    /// The operation, once the add has begun.
    pub operation: Option<OperationId>,
    /// An add is running.
    pub running: bool,
    /// The steps, in order.
    pub steps: Vec<Step>,
    /// The red card, when a step failed.
    pub failure: Option<AddFailure>,
    /// The fingerprint waiting for the person to confirm.
    pub host_key: Option<String>,
    /// Why the add asked for a login, offering Open interactively.
    pub login_offer: Option<LoginReason>,
    /// The login in the dialog.
    pub login: Option<Login>,
    /// The offer to make reconnects unattended.
    pub unattended: Option<Unattended>,
    /// What went wrong that the card doesn't cover, as text for the view.
    pub error: Option<String>,
    /// The add stopped behind This Mac's daemon; the dialog offers Migrate This Mac.
    pub migrate: Option<HostMigrate>,
    /// This Mac's daemon Migrate items, as the parent last sent them.
    pub migrations: Vec<Request>,
}

impl AddMachineState {
    /// What the title shows after "Add machine": the name, else the host.
    #[must_use]
    pub fn display_name(&self) -> Option<&str> {
        if !self.name.trim().is_empty() {
            return Some(self.name.trim());
        }
        let typed = self.typed.trim();
        if !typed.is_empty() {
            return Some(typed);
        }
        match &self.choice {
            Some(Choice::Host(host)) if !host.trim().is_empty() => Some(host.trim()),
            _ => None,
        }
    }

    /// Placeholder text for the machine name field, tailored to the selected target.
    #[must_use]
    pub fn name_placeholder(&self) -> &'static str {
        match self.effective_choice() {
            Some(Choice::Provider { id, .. }) if id == "lima" => {
                "Auto-named (lima-1, lima-2, …) if blank"
            }
            Some(Choice::Provider { .. }) => "Name for the machine",
            _ => "Leave empty to use the host name",
        }
    }

    /// The Migrate item the Migrate This Mac button approves: shown only while the add is
    /// blocked and not migrating, and a waiting or failed item for This Mac's daemon exists.
    #[must_use]
    pub fn migrate_offer(&self) -> Option<RequestId> {
        let blocked = self.migrate.as_ref()?;
        if matches!(blocked.stage, MigrateStage::Migrating(_)) {
            return None;
        }
        self.migrations
            .iter()
            .find(|request| {
                request.answer.is_none()
                    && request.migrate().is_some_and(|card| {
                        card.thing == MigrateThing::Daemon
                            && matches!(card.phase, MigratePhase::Waiting | MigratePhase::Failed(_))
                    })
            })
            .map(|request| request.id.clone())
    }

    /// How an add would wait behind This Mac's daemon: a Daemon Migrate item not yet answered
    /// means that daemon is at another release than this app (D752).
    fn this_mac_unmigrated(&self) -> Option<MigrateStage> {
        let item = self.migrations.iter().find(|request| {
            request.answer.is_none()
                && request.migrate().is_some_and(|card| card.thing == MigrateThing::Daemon)
        })?;
        Some(match item.migrate().map(|card| &card.phase) {
            Some(MigratePhase::Migrating) => MigrateStage::Migrating(item.id.clone()),
            _ => MigrateStage::Blocked,
        })
    }

    /// Who the login talks to (D167): the provider's id, or `ssh` for a plain SSH host.
    #[must_use]
    pub fn who(&self) -> String {
        match self.effective_choice() {
            Some(Choice::Provider { id, .. }) => id,
            _ => "ssh".to_owned(),
        }
    }

    /// Whether the chosen provider is marked interactive (MP-12).
    fn provider_is_interactive(&self) -> bool {
        match self.effective_choice() {
            Some(Choice::Provider { id, .. }) => self.providers.iter().any(|p| p.id == id && p.interactive),
            _ => false,
        }
    }

    /// Why the picked provider can't add yet: the host variables its
    /// commands read that aren't set on this Mac, as the line the dialog
    /// shows. None for a plain SSH host or a provider with all it needs.
    #[must_use]
    pub fn needs_variables(&self) -> Option<String> {
        let Some(Choice::Provider { id, .. }) = self.effective_choice() else {
            return None;
        };
        let provider = self.providers.iter().find(|provider| provider.id == id)?;
        let names = match provider.unset.as_slice() {
            [] => return None,
            [one] => one.clone(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        };
        Some(format!("{} needs {names}. Set it in Settings > Machines.", provider.label))
    }

    /// What the add targets: the typed host when there is one, else the pop-up's pick.
    #[must_use]
    pub fn effective_choice(&self) -> Option<Choice> {
        let typed = self.typed.trim();
        if typed.is_empty() {
            self.choice.clone()
        } else {
            Some(Choice::Host(typed.to_owned()))
        }
    }
}

/// What the dialog asks its parent to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delegate {
    /// A machine was added: select it.
    Added(MachineId),
    /// Approve this Migrate item, the one the Inbox shows, as its Migrate button does.
    Migrate(RequestId),
    /// Say this in a toast.
    Toast(String),
    /// Open Settings › Machines, where providers and their host variables are.
    ManageProviders,
}

/// What happens in the dialog.
#[derive(Clone, Debug, PartialEq)]
pub enum AddMachineAction {
    /// Open the dialog and load the choices.
    Open,
    /// The choices arrived.
    Loaded(Result<Choices, DaemonError>),
    /// Pick an SSH host in the pop-up.
    SelectHost(String),
    /// Type a host in the field; it wins over the pop-up's pick while it isn't empty.
    TypeHost(String),
    /// Open or close the Host pop-up.
    HostMenu(bool),
    /// Pick a provider; its params start at their defaults.
    SelectProvider(String),
    /// Manage providers… or Open Settings: close the dialog and open
    /// Settings › Machines. Nothing happens while an add or login runs.
    ManageProviders,
    /// Change a provider param.
    SetParam(String, String),
    /// Change the machine's name.
    SetName(String),
    /// Begin the add.
    Start,
    /// The add reported something.
    Progress(AddEvent),
    /// The add's stream ended.
    ProgressEnded(StreamEnd<DaemonError>),
    /// Try again after a failure (HO-2).
    TryAgain,
    /// Cancel a running add and close, or just close.
    Cancel,
    /// The host key is right: add again trusting it.
    ConfirmHostKey,
    /// The host key is not right: add nothing.
    DeclineHostKey,
    /// Open the login in the dialog (MP-13).
    OpenLogin,
    /// The login reported something.
    LoginProgress(LoginEvent),
    /// The login's stream ended.
    LoginEnded(StreamEnd<DaemonError>),
    /// The person typed into the terminal.
    LoginInput(Vec<u8>),
    /// The terminal changed size.
    LoginResize {
        /// Cell columns.
        columns: u16,
        /// Cell rows.
        rows: u16,
    },
    /// "Done, I'm logged in."
    LoginDone,
    /// Pick how to make reconnects unattended.
    ChooseMethod(UnattendedMethod),
    /// Make reconnects unattended with the chosen way.
    MakeUnattended,
    /// Keep the machine interactive (MP-16).
    SkipUnattended,
    /// A fire-and-forget call finished.
    CallFinished(Result<(), DaemonError>),
    /// Making it unattended finished.
    UnattendedFinished(Result<(), DaemonError>),
    /// Migrate This Mac: approve the Migrate item through the parent.
    MigrateThisMac,
    /// From the parent: This Mac's daemon Migrate items, with the phase each shows.
    Migrations(Vec<Request>),
    /// For the parent.
    Delegate(Delegate),
}

/// The dialog's reducer.
#[derive(Clone)]
pub struct AddMachine {
    machines: Arc<dyn Machines>,
}

impl AddMachine {
    /// A dialog that adds machines through `machines`.
    #[must_use]
    pub fn new(machines: Arc<dyn Machines>) -> Self {
        Self { machines }
    }

    fn request(state: &AddMachineState, trusted: Option<String>) -> Option<AddRequest> {
        let target = match &state.effective_choice()? {
            Choice::Host(host) if host.trim().is_empty() => return None,
            Choice::Host(host) => AddTarget::Ssh { host: host.trim().to_owned() },
            Choice::Provider { id, params } => {
                AddTarget::Provider { provider: id.clone(), params: params.clone() }
            }
        };
        let name = state.name.trim();
        Some(AddRequest {
            target,
            name: (!name.is_empty()).then(|| name.to_owned()),
            trusted_fingerprint: trusted,
        })
    }

    fn follow(&self, request: AddRequest) -> Effect<AddMachineAction> {
        Effect::stream(
            self.machines.add(request),
            AddMachineAction::Progress,
            AddMachineAction::ProgressEnded,
        )
        .cancellable(PROGRESS, true)
    }

    /// Runs the add again from the same inputs, after This Mac's daemon was migrated.
    fn rerun(&self, state: &mut AddMachineState) -> Effect<AddMachineAction> {
        state.migrate = None;
        let Some(request) = Self::request(state, None) else {
            return Effect::none();
        };
        state.running = true;
        state.operation = None;
        state.steps.clear();
        state.failure = None;
        state.error = None;
        self.follow(request)
    }

    /// Follows the Migrate item the person approved: migrated runs the add again,
    /// a failure (or an item that went back to waiting) is shown.
    fn after_migrations(&self, state: &mut AddMachineState) -> Effect<AddMachineAction> {
        let Some(HostMigrate { stage: MigrateStage::Migrating(id), .. }) = &state.migrate else {
            return Effect::none();
        };
        let Some(item) = state.migrations.iter().find(|request| &request.id == id) else {
            return Effect::none();
        };
        if item.answer == Some(Answer::Approved) {
            return self.rerun(state);
        }
        let failure = match item.migrate().map(|card| &card.phase) {
            Some(MigratePhase::Failed(failure)) => Some(failure.clone()),
            Some(MigratePhase::Waiting) if item.answer.is_none() => Some(MigrateFailure {
                message: "The migrate didn't start.".to_owned(),
                fix: "Open the Migrate item in the Inbox to see why, then try again.".to_owned(),
            }),
            _ => None,
        };
        if let (Some(failure), Some(migrate)) = (failure, state.migrate.as_mut()) {
            migrate.stage = MigrateStage::Failed(failure);
        }
        Effect::none()
    }

    fn call(&self, call: humata2_clients::Reply<()>) -> Effect<AddMachineAction> {
        Effect::future(async move { AddMachineAction::CallFinished(call.await) })
    }

    /// Closes the dialog, stopping what follows, and says the machine was added.
    fn close_added(state: &mut AddMachineState, machine: MachineId) -> Effect<AddMachineAction> {
        let name = state.display_name().map_or_else(|| machine.to_string(), str::to_owned);
        *state = AddMachineState::default();
        Effect::concat([
            Effect::cancel(PROGRESS),
            Effect::cancel(LOGIN),
            Effect::send(AddMachineAction::Delegate(Delegate::Added(machine))),
            Effect::send(AddMachineAction::Delegate(Delegate::Toast(format!("Added {name}.")))),
        ])
    }

    /// Shows the login as its own done step (frame 58): in the place of the connect attempt
    /// that asked for it, else first. The detail says what was accepted (D168).
    fn mark_logged_in(state: &mut AddMachineState, who: &str, why: LoginWhy, machine: &str) {
        let detail = login_detail(who, why, machine);
        let login = Step { kind: StepKind::Login, status: StepStatus::Done, detail };
        let at = [StepKind::Login, StepKind::Connect]
            .into_iter()
            .find_map(|kind| state.steps.iter().position(|step| step.kind == kind));
        match at.and_then(|at| state.steps.get_mut(at)) {
            Some(slot) => *slot = login,
            None => state.steps.insert(0, login),
        }
    }

    fn update_step(state: &mut AddMachineState, update: StepUpdate) {
        match state.steps.iter_mut().find(|step| step.kind == update.step) {
            Some(step) => {
                step.status = update.status;
                step.detail = update.detail;
            }
            None => state.steps.push(Step {
                kind: update.step,
                status: update.status,
                detail: update.detail,
            }),
        }
    }
}

impl Reducer for AddMachine {
    type State = AddMachineState;
    type Action = AddMachineAction;

    #[allow(clippy::too_many_lines)]
    fn reduce(&self, state: &mut AddMachineState, action: AddMachineAction) -> Effect<AddMachineAction> {
        use AddMachineAction as A;
        match action {
            A::Open => {
                if state.open {
                    return Effect::none();
                }
                *state = AddMachineState { open: true, loading: true, ..AddMachineState::default() };
                let machines = self.machines.clone();
                return Effect::future(async move { A::Loaded(machines.choices().await) });
            }
            A::Loaded(result) => {
                if !state.open {
                    return Effect::none();
                }
                state.loading = false;
                match result {
                    Ok(choices) => {
                        if state.choice.is_none() {
                            state.choice =
                                choices.hosts.first().map(|host| Choice::Host(host.name.clone()));
                        }
                        state.hosts = choices.hosts;
                        state.providers = choices.providers;
                    }
                    Err(error) => state.error = Some(error.to_string()),
                }
            }
            A::SelectHost(host) => {
                if state.open && !state.running {
                    state.choice = Some(Choice::Host(host));
                    state.typed.clear();
                    state.host_menu = false;
                }
            }
            A::TypeHost(text) => {
                if state.open && !state.running {
                    state.typed = text;
                }
            }
            A::HostMenu(open) => state.host_menu = open && state.open && !state.running,
            A::SelectProvider(id) => {
                if state.open
                    && !state.running
                    && let Some(provider) = state.providers.iter().find(|p| p.id == id)
                {
                    let params = provider
                        .params
                        .iter()
                        .map(|param| (param.name.clone(), param.default.clone()))
                        .collect();
                    state.choice = Some(Choice::Provider { id, params });
                    state.typed.clear();
                    state.host_menu = false;
                }
            }
            A::SetParam(name, value) => {
                if !state.running
                    && let Some(Choice::Provider { params, .. }) = &mut state.choice
                    && let Some(slot) = params.get_mut(&name)
                {
                    *slot = value;
                }
            }
            A::SetName(name) => {
                if !state.running {
                    state.name = name;
                }
            }
            A::Start => {
                if !state.open || state.running || state.login.is_some() || state.unattended.is_some() {
                    return Effect::none();
                }
                // A provider whose host variables aren't set would fail at its
                // first command; the dialog says which to set instead.
                if state.needs_variables().is_some() {
                    return Effect::none();
                }
                let Some(request) = Self::request(state, None) else {
                    return Effect::none();
                };
                state.running = true;
                state.operation = None;
                state.steps.clear();
                state.failure = None;
                state.host_menu = false;
                state.host_key = None;
                state.login_offer = None;
                state.error = None;
                state.migrate = None;
                // This Mac's daemon waiting for its Migrate item is at another release than
                // this app, and nothing runs across releases (UP-1, D752): the add waits behind
                // Migrate This Mac and runs again once that item is migrated.
                if let Some(stage) = state.this_mac_unmigrated() {
                    state.running = false;
                    state.migrate = Some(HostMigrate {
                        platform: String::new(),
                        what: HELD_BEHIND_MIGRATE.to_owned(),
                        stage,
                    });
                    return Effect::none();
                }
                return self.follow(request);
            }
            A::Progress(event) => {
                if !state.open {
                    return Effect::none();
                }
                match event {
                    AddEvent::Started { operation, steps } => {
                        state.operation = Some(operation);
                        state.steps = steps
                            .into_iter()
                            .map(|kind| Step { kind, status: StepStatus::NotRun, detail: String::new() })
                            .collect();
                    }
                    AddEvent::Step(update) => Self::update_step(state, update),
                    AddEvent::Outcome(outcome) => {
                        state.running = false;
                        match outcome {
                            AddOutcome::Added(machine) => return Self::close_added(state, machine),
                            AddOutcome::Failed(failure) => {
                                if let Some(step) =
                                    state.steps.iter_mut().find(|step| step.kind == failure.step)
                                {
                                    step.status = StepStatus::Failed;
                                }
                                state.failure = Some(failure);
                            }
                            AddOutcome::HostKey { fingerprint } => state.host_key = Some(fingerprint),
                            AddOutcome::NeedsLogin { reason } => state.login_offer = Some(reason),
                            AddOutcome::NeedsMigrate { platform, what } => {
                                state.migrate = Some(HostMigrate { platform, what, stage: MigrateStage::Blocked });
                            }
                        }
                    }
                }
            }
            A::ProgressEnded(end) => {
                // The stream ends after its outcome; ending without one means
                // the link broke, so the person can try again.
                if state.running {
                    state.running = false;
                    if let StreamEnd::Failed(error) = end {
                        state.error = Some(error.to_string());
                    }
                }
            }
            A::TryAgain => {
                let (true, false, Some(operation)) =
                    (state.failure.is_some(), state.running, state.operation.clone())
                else {
                    return Effect::none();
                };
                state.failure = None;
                state.error = None;
                state.running = true;
                for step in &mut state.steps {
                    step.status = StepStatus::NotRun;
                    step.detail.clear();
                }
                return Effect::stream(
                    self.machines.retry(&operation),
                    A::Progress,
                    A::ProgressEnded,
                )
                .cancellable(PROGRESS, true);
            }
            A::Cancel => {
                if !state.open {
                    return Effect::none();
                }
                let mut effects = vec![Effect::cancel(PROGRESS), Effect::cancel(LOGIN)];
                if let Some(login) = &state.login {
                    effects.push(self.call(self.machines.login_cancel(&login.target)));
                }
                if let (true, Some(operation)) = (state.running, &state.operation) {
                    effects.push(self.call(self.machines.cancel(operation)));
                }
                *state = AddMachineState::default();
                return Effect::merge(effects);
            }
            A::ConfirmHostKey => {
                let Some(fingerprint) = state.host_key.take() else {
                    return Effect::none();
                };
                let Some(request) = Self::request(state, Some(fingerprint)) else {
                    return Effect::none();
                };
                state.running = true;
                state.steps.clear();
                state.operation = None;
                return self.follow(request);
            }
            A::DeclineHostKey => state.host_key = None,
            A::OpenLogin => {
                let (Some(reason), None, Some(operation)) =
                    (&state.login_offer, &state.login, state.operation.clone())
                else {
                    return Effect::none();
                };
                let why = match reason {
                    LoginReason::Prompted { kind: PromptKind::Other, .. } => LoginWhy::Asked,
                    LoginReason::Prompted { kind, .. } => LoginWhy::Prompted(*kind),
                    LoginReason::Hung if state.provider_is_interactive() => LoginWhy::AlwaysInteractive,
                    LoginReason::Hung => LoginWhy::Hung,
                };
                let target = LoginTarget::Operation(operation);
                state.login_offer = None;
                state.login = Some(Login {
                    target: target.clone(),
                    why,
                    prompt: None,
                    bytes: Vec::new(),
                    finishing: false,
                    error: None,
                });
                return Effect::stream(
                    self.machines.open_login(&target),
                    A::LoginProgress,
                    A::LoginEnded,
                )
                .cancellable(LOGIN, true);
            }
            A::LoginProgress(event) => match event {
                LoginEvent::Output(bytes) => {
                    if let Some(login) = &mut state.login {
                        login.bytes.extend(bytes);
                    }
                }
                LoginEvent::Prompt { kind, .. } => {
                    // The first prompt of the session says what was accepted (D168).
                    if let Some(login) = &mut state.login
                        && login.prompt.is_none()
                        && kind != PromptKind::Other
                    {
                        login.prompt = Some(kind);
                    }
                }
                LoginEvent::Step(update) => Self::update_step(state, update),
                LoginEvent::Ready { machine, can_make_unattended, daemon_version } => {
                    let Some(login) = state.login.take() else {
                        return Effect::none();
                    };
                    let provider = match state.effective_choice() {
                        Some(Choice::Provider { id, .. }) => Some(id),
                        _ => None,
                    };
                    let why = login.prompt.map_or(login.why, LoginWhy::Prompted);
                    let who = state.who();
                    let name = state.display_name().map_or_else(|| machine.to_string(), str::to_owned);
                    Self::mark_logged_in(state, &who, why, &name);
                    if can_make_unattended {
                        state.unattended = Some(Unattended {
                            machine,
                            method: UnattendedMethod::AddKey,
                            working: false,
                            provider,
                            version: daemon_version,
                        });
                    } else {
                        return Self::close_added(state, machine);
                    }
                }
                LoginEvent::Failed { reason } => {
                    if let Some(login) = &mut state.login {
                        login.finishing = false;
                        login.error = Some(reason);
                    }
                }
            },
            A::LoginEnded(end) => {
                if let (StreamEnd::Failed(error), Some(login)) = (end, &mut state.login) {
                    login.finishing = false;
                    login.error = Some(error.to_string());
                }
            }
            A::LoginInput(bytes) => {
                if let Some(login) = &state.login {
                    return self.call(self.machines.login_input(&login.target, bytes));
                }
            }
            A::LoginResize { columns, rows } => {
                if let Some(login) = &state.login {
                    return self.call(self.machines.login_resize(&login.target, columns, rows));
                }
            }
            A::LoginDone => {
                if let Some(login) = &mut state.login
                    && !login.finishing
                {
                    login.finishing = true;
                    login.error = None;
                    return self.call(self.machines.login_done(&login.target));
                }
            }
            A::ChooseMethod(method) => {
                if let Some(offer) = &mut state.unattended
                    && !offer.working
                    && (method != UnattendedMethod::KeepToken || offer.provider.is_some())
                {
                    offer.method = method;
                }
            }
            A::MakeUnattended => {
                if let Some(offer) = &mut state.unattended
                    && !offer.working
                {
                    offer.working = true;
                    let call = self.machines.make_unattended(&offer.machine, offer.method);
                    return Effect::future(async move { A::UnattendedFinished(call.await) });
                }
            }
            A::UnattendedFinished(result) => {
                let Some(offer) = state.unattended.as_mut() else {
                    return Effect::none();
                };
                match result {
                    Ok(()) => {
                        let machine = offer.machine.clone();
                        return Self::close_added(state, machine);
                    }
                    Err(error) => {
                        offer.working = false;
                        state.error = Some(error.to_string());
                    }
                }
            }
            A::SkipUnattended => {
                if let Some(offer) = &state.unattended
                    && !offer.working
                {
                    let machine = offer.machine.clone();
                    return Self::close_added(state, machine);
                }
            }
            A::CallFinished(result) => {
                if state.open
                    && let Err(error) = result
                {
                    state.error = Some(error.to_string());
                    if let Some(login) = &mut state.login {
                        login.finishing = false;
                    }
                }
            }
            A::MigrateThisMac => {
                let Some(id) = state.migrate_offer() else {
                    return Effect::none();
                };
                if let Some(migrate) = &mut state.migrate {
                    migrate.stage = MigrateStage::Migrating(id.clone());
                }
                return Effect::send(A::Delegate(Delegate::Migrate(id)));
            }
            A::Migrations(items) => {
                state.migrations = items;
                return self.after_migrations(state);
            }
            A::ManageProviders => {
                if !state.open || state.running || state.login.is_some() || state.unattended.is_some() {
                    return Effect::none();
                }
                *state = AddMachineState::default();
                return Effect::send(A::Delegate(Delegate::ManageProviders));
            }
            A::Delegate(_) => {}
        }
        Effect::none()
    }
}

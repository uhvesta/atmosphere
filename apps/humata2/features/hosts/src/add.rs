//! Add machine (HO-1, HO-2, MP-12 to MP-17; frames 56 to 58): pick an SSH
//! host or a provider, follow the steps, and deal with what stops them: a
//! failure with its fix, a host key to confirm, or a login the person does
//! in a terminal inside the dialog.

use std::collections::BTreeMap;
use std::sync::Arc;

use humata2_clients::DaemonError;
use humata2_clients::machines::{
    AddFailure, AddEvent, AddOutcome, AddRequest, AddTarget, Choices, LoginEvent, LoginTarget,
    Machines, OperationId, Provider, SshHost, StepKind, StepStatus, StepUpdate, UnattendedMethod,
};
use humata2_clients::model::MachineId;
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

/// A login the person is doing in the dialog's terminal (frame 57).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Login {
    /// What it belongs to.
    pub target: LoginTarget,
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
    /// The name to give the machine; empty means the host's own.
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
    /// The prompt that asked for a login, offering Open interactively.
    pub login_offer: Option<String>,
    /// The login in the dialog.
    pub login: Option<Login>,
    /// The offer to make reconnects unattended.
    pub unattended: Option<Unattended>,
    /// What went wrong that the card doesn't cover, as text for the view.
    pub error: Option<String>,
}

impl AddMachineState {
    /// What the title shows after "Add machine": the name, else the host.
    #[must_use]
    pub fn display_name(&self) -> Option<&str> {
        if !self.name.trim().is_empty() {
            return Some(self.name.trim());
        }
        match &self.choice {
            Some(Choice::Host(host)) if !host.trim().is_empty() => Some(host.trim()),
            _ => None,
        }
    }
}

/// What the dialog asks its parent to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delegate {
    /// A machine was added: select it.
    Added(MachineId),
    /// Say this in a toast.
    Toast(String),
}

/// What happens in the dialog.
#[derive(Clone, Debug, PartialEq)]
pub enum AddMachineAction {
    /// Open the dialog and load the choices.
    Open,
    /// The choices arrived.
    Loaded(Result<Choices, DaemonError>),
    /// Pick (or type) an SSH host.
    SelectHost(String),
    /// Pick a provider; its params start at their defaults.
    SelectProvider(String),
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
        let target = match state.choice.as_ref()? {
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
                }
            }
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
                let Some(request) = Self::request(state, None) else {
                    return Effect::none();
                };
                state.running = true;
                state.operation = None;
                state.steps.clear();
                state.failure = None;
                state.host_key = None;
                state.login_offer = None;
                state.error = None;
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
                            AddOutcome::NeedsLogin { prompt } => state.login_offer = Some(prompt),
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
                let (Some(_), None, Some(operation)) =
                    (&state.login_offer, &state.login, state.operation.clone())
                else {
                    return Effect::none();
                };
                let target = LoginTarget::Operation(operation);
                state.login_offer = None;
                state.login = Some(Login {
                    target: target.clone(),
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
                LoginEvent::Step(update) => Self::update_step(state, update),
                LoginEvent::Ready { machine, can_make_unattended } => {
                    if state.login.take().is_none() {
                        return Effect::none();
                    }
                    if can_make_unattended {
                        state.unattended = Some(Unattended {
                            machine,
                            method: UnattendedMethod::AddKey,
                            working: false,
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
            A::Delegate(_) => {}
        }
        Effect::none()
    }
}

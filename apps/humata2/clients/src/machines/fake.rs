//! A scripted [`Machines`]: tests choose what each call does and read back
//! the calls made. No timers, no real IO.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::mpsc;

use super::{
    AddEvent, AddOutcome, AddRequest, AddStream, AddTarget, Choices, ClashChoice, LoginEvent, LoginStream,
    LoginTarget, MachineDetail, Machines, OperationId, SkillsPreview, StepKind, StepStatus, StepUpdate,
    UnattendedMethod,
};
use crate::daemon::{Daemon, DaemonError, Reply};
use crate::model::MachineId;

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
    /// The fake daemon an unscripted SSH add goes through, if any.
    through: Option<Arc<dyn Daemon>>,
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
            })),
        }
    }

    /// Makes an SSH add that has no script go through `daemon`, the way
    /// the app's fakes added a machine before the dialog: the machine
    /// appears in the fake workspace and every step reports done. A
    /// scripted add still plays its script.
    #[must_use]
    pub fn adding_through(self, daemon: Arc<dyn Daemon>) -> Self {
        self.inner().through = Some(daemon);
        self
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

    /// What `skills_preview` returns from now on.
    pub fn set_preview(&self, preview: Result<SkillsPreview, DaemonError>) {
        self.inner().preview = preview;
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
                inner.through.clone()
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
        let mut inner = self.inner();
        inner.calls.push(Call::Detail(machine.clone()));
        let result = match inner.failures.get("detail") {
            Some(error) => Err(error.clone()),
            None => inner.detail.clone(),
        };
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
}

//! A scripted [`Machines`]: tests choose what each call does and read back
//! the calls made. No timers, no real IO.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::mpsc;

use super::{
    AddEvent, AddRequest, AddStream, Choices, ClashChoice, LoginEvent, LoginStream, LoginTarget,
    MachineDetail, Machines, OperationId, SkillsPreview, UnattendedMethod,
};
use crate::daemon::{DaemonError, Reply};
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

struct Inner {
    choices: Choices,
    adds: VecDeque<Vec<AddEvent>>,
    logins: VecDeque<LoginScript>,
    pending_login: Option<(LoginScript, mpsc::UnboundedSender<Result<LoginEvent, DaemonError>>)>,
    detail: Result<MachineDetail, DaemonError>,
    preview: Result<SkillsPreview, DaemonError>,
    failures: HashMap<&'static str, DaemonError>,
    calls: Vec<Call>,
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
                logins: VecDeque::new(),
                pending_login: None,
                detail: Err(DaemonError::NotFound { what: "this machine".to_owned() }),
                preview: Ok(SkillsPreview::default()),
                failures: HashMap::new(),
                calls: Vec::new(),
            })),
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
        self.inner().adds.push_back(events);
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

    fn add_stream(&self, method: &'static str, call: Call) -> Reply<AddStream> {
        let events = self.inner().adds.pop_front().unwrap_or_default();
        let (sender, receiver) = mpsc::unbounded_channel();
        for event in events {
            let _ = sender.send(Ok(event));
        }
        self.call(method, call, receiver)
    }
}

impl Machines for FakeMachines {
    fn choices(&self) -> Reply<Choices> {
        let choices = self.inner().choices.clone();
        self.call("choices", Call::Choices, choices)
    }

    fn add(&self, request: AddRequest) -> Reply<AddStream> {
        self.add_stream("add", Call::Add(request))
    }

    fn retry(&self, operation: &OperationId) -> Reply<AddStream> {
        self.add_stream("retry", Call::Retry(operation.clone()))
    }

    fn cancel(&self, operation: &OperationId) -> Reply<()> {
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

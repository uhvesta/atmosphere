//! The interactive login (MP-12 to MP-18) as ProviderService answers it:
//! `InteractiveLogin`, a stream in which the user logs in by hand in a real
//! terminal on the Mac and the machine becomes a normal one, and
//! `MakeUnattended`, which stops its reconnects needing the user (MP-17).
//! The work is machines' (`humata2_machines::LoginSession` and
//! `make_unattended`); this wires it to the API: the access checks, the
//! stream, the operation a failed create left, and the peer link.
//!
//! `provider_service` delegates here: it holds a [`LoginSvc`] and its two
//! methods call [`LoginSvc::interactive_login`] and
//! [`LoginSvc::make_unattended`].
//!
//! The terminal's output goes out as it is produced (TR-7). The takeover
//! runs on a thread of its own, since the terminal and the takeover are
//! blocking; the bytes of the finished link are copied between that thread
//! and the async peer link by two more.
//!
//! Only the user: an agent or script is refused, and the user must be at
//! the Mac, as for RemoveMachine. After a successful login the machine's
//! peer link is kept up over the terminal that carries it. That terminal
//! can't be dialled again, so when the link drops the machine is Offline
//! and, being interactive, shows NeedsLogin ([`needs_login`]) until the
//! user logs in again.

use std::collections::BTreeMap;
use std::future::Future;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use humata2_access::{Call, Capability, MachineAction, MachineActionKind, MachineId, Refusal, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_machines::{
    HostEntry, LinkReader, LinkWriter, LoginError, LoginSession, MachineName, Method, Observed, OpStep, Plan, Provider,
    StepEvent, SystemRunner, TakeoverError, existing_link, is_interactive, provider_link, register,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Code, Request, Response, Status, Streaming};

use crate::guard::{self, Guard};
use crate::identity::{self, Caller};
use crate::machine_service::{Work, lock, machine_id, machine_of, read_providers, status_of};
use crate::operations::Operations;
use crate::peer_client::{Dial, LinkError, Pipe};
use crate::status;

/// What a failed create left that a login can finish (the failed
/// operation's provider, machine and params).
pub(crate) struct Made {
    pub(crate) provider: String,
    pub(crate) machine: String,
    pub(crate) values: BTreeMap<String, String>,
}

/// How `provider_service` lets the login find what an operation made:
/// `find` by the operation's id, and `done` once a login finished it, so
/// Delete what was made no longer offers to delete a working machine.
pub(crate) struct Pending {
    pub(crate) find: Box<dyn Fn(&str) -> Option<Made> + Send + Sync>,
    pub(crate) done: Arc<dyn Fn(&str) + Send + Sync>,
}

pub(crate) struct LoginSvc {
    guard: Arc<Guard>,
    work: Arc<Work>,
    operations: Arc<Operations>,
    pending: Pending,
}

/// Where a login goes.
struct Resolved {
    name: MachineName,
    link: humata2_machines::Link,
    /// The provider and params of a machine not yet listed.
    made: Option<(Provider, BTreeMap<String, String>)>,
    /// The operation whose connect step asked for the login.
    operation: Option<String>,
}

impl LoginSvc {
    pub(crate) fn new(guard: Arc<Guard>, work: Arc<Work>, operations: Arc<Operations>, pending: Pending) -> Self {
        Self {
            guard,
            work,
            operations,
            pending,
        }
    }

    /// Only the user, at the Mac, may log in or make unattended: the check
    /// is the one RemoveMachine makes, for `machine`.
    fn require(&self, caller: &Caller, machine: &str) -> Result<(), Status> {
        let action = MachineActionKind::Reconnect;
        if !matches!(caller, Caller::User(_)) {
            return Err(status::refused(&Refusal::UserOnly {
                capability: Some(action.capability()),
            }));
        }
        self.guard.require(
            caller,
            &Call::Use {
                capability: action.capability(),
                target: Target::Machine(MachineAction {
                    machine: MachineId::new(machine),
                    action,
                }),
            },
        )?;
        let principal = guard::principal(caller);
        if self
            .guard
            .with_policy(|_, context| principal.present_user(context))
            .is_none()
        {
            return Err(status::refused(&Refusal::UserOnly {
                capability: Some(action.capability()),
            }));
        }
        Ok(())
    }

    fn provider(&self, id: &str) -> Result<Provider, Status> {
        let (_, providers) = read_providers(self.work.machines.hosts_file()).map_err(|error| {
            status::internal(
                format!("couldn't read the providers file: {}", status::chain(&error)),
                "",
            )
        })?;
        providers
            .into_iter()
            .find(|provider| provider.id() == id)
            .ok_or_else(|| {
                status::not_found(
                    pb::EntityKind::Provider,
                    id,
                    format!("there is no provider {id} in the providers file"),
                    "add it back in Settings › Machines › Providers, or add the machine again",
                )
            })
    }

    /// The machine a login opens on: an existing one (MP-16), or the one a
    /// create or add operation could not reach without a person (MP-13).
    fn resolve(&self, caller: &Caller, open: &pb::LoginOpen) -> Result<Resolved, Status> {
        let bad_name = |machine: &str, source: humata2_machines::NameError| {
            status::invalid_field(
                "machine",
                pb::invalid_field::Problem::BadFormat,
                format!("{machine:?} isn't a machine name: {source}"),
                "list the machines to see their names",
            )
        };
        match &open.target {
            Some(pb::login_open::Target::Machine(id)) => {
                let name = machine_of(Some(id.clone()))?;
                self.require(caller, &name)?;
                let machine = MachineName::new(&name).map_err(|source| bad_name(&name, source))?;
                let link = existing_link(&self.work.machines, &name).map_err(|error| login_status(&error, None))?;
                Ok(Resolved {
                    name: machine,
                    link,
                    made: None,
                    operation: None,
                })
            }
            Some(pb::login_open::Target::Operation(id)) => {
                let operation = id.value.clone();
                let made = (self.pending.find)(&operation).ok_or_else(|| {
                    status::not_found(
                        pb::EntityKind::Operation,
                        &operation,
                        format!("operation {operation} left no machine waiting for a login"),
                        "start Create machine again; a login is offered when its connect step asks for one",
                    )
                })?;
                self.require(caller, &made.machine)?;
                let provider = self.provider(&made.provider)?;
                let machine = MachineName::new(&made.machine).map_err(|source| bad_name(&made.machine, source))?;
                if self.work.machines.get(&made.machine).is_ok() {
                    return Err(status::failure(
                        Code::FailedPrecondition,
                        status::error(
                            format!("{} is already listed, so the create it came from can't be finished", made.machine),
                            "log in from the machine's own page instead",
                            pb::error::Kind::NameTaken(pb::NameTaken {
                                kind: pb::EntityKind::Machine as i32,
                                name: made.machine.clone(),
                                suggestion: format!("{}-2", made.machine),
                            }),
                        ),
                    ));
                }
                let link = provider_link(&provider, &machine, &made.values)
                    .map_err(|error| login_status(&error, Some(&made.provider)))?;
                Ok(Resolved {
                    name: machine,
                    link,
                    made: Some((provider, made.values)),
                    operation: Some(operation),
                })
            }
            None => Err(status::missing("target")),
        }
    }

    /// MP-14: opens the connect command in a terminal and streams it. The
    /// first message must be `open`.
    pub(crate) async fn interactive_login(
        &self,
        request: Request<Streaming<pb::LoginClientMessage>>,
    ) -> Result<Response<ReceiverStream<Result<pb::LoginServerMessage, Status>>>, Status> {
        use pb::login_client_message::Message;
        let caller = identity::caller(&request)?;
        // Before anything else, so no path answers "isn't built" (MP-14).
        if matches!(caller, Caller::Peer) {
            return Err(status::refused(&Refusal::UserOnly {
                capability: Some(Capability::MachineProvision),
            }));
        }
        if !matches!(caller, Caller::User(_)) {
            return Err(status::user_only_call(
                Capability::MachineProvision,
                "logging in to a machine by hand",
                "ask the user with inbox_ask to log in from the app (Hosts › the machine › Log in), then retry what you wanted",
            ));
        }
        let mut inbound = request.into_inner();
        let open = match inbound.message().await?.and_then(|message| message.message) {
            Some(Message::Open(open)) => open,
            _ => return Err(status::missing("open")),
        };
        let resolved = self.resolve(&caller, &open)?;
        let progress = match &resolved.operation {
            Some(id) => Some(self.operations.progress_of(&caller, id)?),
            None => None,
        };
        let invocation = resolved
            .link
            .interactive()
            .map_err(|source| login_status(&LoginError::Quote(source), None))?;
        let provider_id = resolved.made.as_ref().map(|(provider, _)| provider.id().to_owned());
        let (session, handle) = LoginSession::open(
            resolved.name.clone(),
            &invocation,
            &crate::host_vars::Installed,
            size(open.cols, 80),
            size(open.rows, 24),
        )
        .map_err(|error| login_status(&error, provider_id.as_deref()))?;

        let (out, receiver) = mpsc::channel(64);
        let runtime = Handle::current();

        // The person's side: input, size, Done and Cancel; the client going
        // away is a cancel.
        let relay = handle;
        tokio::spawn(async move {
            loop {
                let message = match inbound.message().await {
                    Ok(Some(message)) => message.message,
                    _ => {
                        relay.cancel();
                        return;
                    }
                };
                let alive = match message {
                    Some(Message::Input(bytes)) => relay.input(bytes.to_vec()),
                    Some(Message::Resize(resize)) => relay.resize(size(resize.cols, 80), size(resize.rows, 24)),
                    Some(Message::Done(_)) => relay.done(),
                    Some(Message::Cancel(_)) => {
                        relay.cancel();
                        return;
                    }
                    Some(Message::Open(_)) | None => true,
                };
                if !alive {
                    return;
                }
            }
        });

        let work = self.work.clone();
        let done = self.pending.done.clone();
        std::thread::spawn(move || {
            run(session, resolved, progress, provider_id, &work, &done, &out, &runtime);
        });
        Ok(Response::new(ReceiverStream::new(receiver)))
    }

    /// MP-17: makes an interactive machine unattended, on the user's say at
    /// the Mac, and answers with the machine as it now is.
    pub(crate) async fn make_unattended(
        &self,
        request: Request<pb::MakeUnattendedRequest>,
    ) -> Result<Response<pb::Machine>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        // Before anything else, so no path answers "isn't built" (MP-17).
        if matches!(caller, Caller::Peer) {
            return Err(status::refused(&Refusal::UserOnly {
                capability: Some(MachineActionKind::Reconnect.capability()),
            }));
        }
        if !matches!(caller, Caller::User(_)) {
            return Err(status::user_only_call(
                MachineActionKind::Reconnect.capability(),
                "making a machine's login unattended",
                "ask the user with inbox_ask to log in from the app (Hosts › the machine › Log in) and make it unattended there, then retry what you wanted",
            ));
        }
        let name = machine_of(request.machine)?;
        self.require(&caller, &name)?;
        let method = match pb::make_unattended_request::Method::try_from(request.method) {
            Ok(pb::make_unattended_request::Method::AuthorizedKey) => Method::AuthorizedKey,
            Ok(pb::make_unattended_request::Method::KeepCachedToken) => Method::KeepCachedToken,
            _ => return Err(status::missing("method")),
        };
        let work = self.work.clone();
        let target = name.clone();
        let result = tokio::task::spawn_blocking(move || {
            humata2_machines::make_unattended(&work.machines, &target, method, &SystemRunner::new(&crate::host_vars::Installed))
        })
        .await
        .map_err(|error| {
            status::internal(
                format!("making {name} unattended was cut short: {error}"),
                "",
            )
        })?;
        match result {
            Ok(entry) => {
                self.work.publish_machines();
                Ok(Response::new(self.work.machine(&entry)))
            }
            Err(error) => Err(login_status(&error, None)),
        }
    }
}

/// A terminal size from a request, kept to what a terminal can be.
fn size(value: u32, default: u16) -> u16 {
    u16::try_from(value).ok().filter(|size| *size > 0).unwrap_or(default)
}

/// Runs one login to its end on its own thread and tells the stream how it
/// ended. A login that doesn't end Ready leaves nothing listed: the machine
/// is registered only once its takeover is over, and an operation's
/// failure stays as it was.
#[allow(clippy::too_many_arguments)]
fn run(
    session: LoginSession,
    resolved: Resolved,
    progress: Option<crate::operations::Progress>,
    provider_id: Option<String>,
    work: &Arc<Work>,
    done: &Arc<dyn Fn(&str) + Send + Sync>,
    out: &mpsc::Sender<Result<pb::LoginServerMessage, Status>>,
    runtime: &Handle,
) {
    use pb::login_server_message::Message;
    let send = |message: Message| {
        out.blocking_send(Ok(pb::LoginServerMessage { message: Some(message), ..Default::default() })).is_ok()
    };
    let name = resolved.name.as_str().to_owned();
    let keys = work.machines.peer_key(&name);
    // A machine with no enrollment key yet gets one written over the login.
    let enroll_into: Option<PathBuf> = (!keys.exists()).then(|| keys.parent().map(std::path::Path::to_path_buf)).flatten();
    let builds: &(dyn humata2_machines::Builds + Send + Sync) = &*work.builds;
    let plan = Plan {
        builds,
        enroll_into: enroll_into.as_deref(),
        shell_timeout: None,
        sync_timeout: None,
    };
    let result = session.run(&plan, &mut |observed| {
        match observed {
            Observed::Output(bytes) => send(Message::Output(bytes.into())),
            // The line and its kind go in one message (D167).
            Observed::Prompt(prompt) => out
                .blocking_send(Ok(pb::LoginServerMessage {
                    message: Some(Message::PromptSeen(prompt.line)),
                    prompt_kind: prompt_kind(prompt.kind) as i32,
                }))
                .is_ok(),
        }
    });
    let link = match result {
        Ok(link) => link,
        // The person cancelled, or went away: there is nobody to tell.
        Err(LoginError::Cancelled { .. }) => return,
        Err(error) => {
            send(Message::Failed(failed(&error, provider_id.as_deref())));
            return;
        }
    };
    let platform = link.platform();
    let enrolled = link.enrolled();
    let made = resolved.made.as_ref().map(|(provider, values)| (provider, values));
    if let Err(error) = register(&work.machines, &resolved.name, made, &resolved.link, platform) {
        // Dropping the link ends the terminal; nothing was listed.
        drop(link);
        if resolved.made.is_some() {
            let _ = humata2_machines::unregister(&work.machines, &resolved.name, enrolled);
        }
        send(Message::Failed(failed(&error, provider_id.as_deref())));
        return;
    }
    let (reader, writer) = link.split();
    let pipe = bridge(reader, writer, runtime);
    if enroll_into.is_some() {
        // Enrolled afresh: its journal starts over (D226).
        work.forget_journal_now(&name);
    }
    work.start_link(
        &name,
        Box::new(OneShot {
            machine: name.clone(),
            pipe: Mutex::new(Some(pipe)),
        }),
        runtime,
    );
    work.publish_machines();
    if let Some(progress) = progress {
        for step in [OpStep::Connect, OpStep::Check, OpStep::Install, OpStep::Start] {
            progress.step(&StepEvent::Done(step));
        }
        progress.step(&StepEvent::Skipped(OpStep::FindAgents));
        let operation = progress.operation().id.map(|id| id.value).unwrap_or_default();
        progress.succeed(Some(pb::operation::Result::Machine(machine_id(&name))));
        done(&operation);
    }
    send(Message::Ready(pb::LoginReady {
        machine: Some(machine_id(&name)),
        can_make_unattended: true,
    }));
}

/// The peer link's one stream, handed out once: the terminal that carries
/// it can't be dialled again, so a second dial says the link ended and the
/// machine waits for the user to log in again.
struct OneShot {
    machine: String,
    pipe: Mutex<Option<Pipe>>,
}

impl Dial for OneShot {
    fn dial(&self) -> Pin<Box<dyn Future<Output = Result<Pipe, LinkError>> + Send + '_>> {
        Box::pin(async move {
            lock(&self.pipe).take().ok_or_else(|| LinkError::Ended {
                machine: self.machine.clone(),
            })
        })
    }
}

/// The finished link's blocking halves as one async stream: a thread copies
/// each way. When either side ends the other follows, and the terminal is
/// stopped.
fn bridge(mut reader: LinkReader, mut writer: LinkWriter, runtime: &Handle) -> Pipe {
    let (near, far) = tokio::io::duplex(64 * 1024);
    let (mut from_host, mut to_host) = tokio::io::split(far);
    let machine_to_host = runtime.clone();
    std::thread::spawn(move || {
        let mut buffer = vec![0_u8; 16 * 1024];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if machine_to_host.block_on(to_host.write_all(&buffer[..n])).is_err() {
                        break;
                    }
                }
            }
        }
        let _ = machine_to_host.block_on(to_host.shutdown());
    });
    let host_to_machine = runtime.clone();
    std::thread::spawn(move || {
        let mut buffer = vec![0_u8; 16 * 1024];
        loop {
            match host_to_machine.block_on(from_host.read(&mut buffer)) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if writer.write_all(&buffer[..n]).and_then(|()| writer.flush()).is_err() {
                        break;
                    }
                }
            }
        }
        writer.close();
    });
    Box::new(near)
}

/// The reachability `entry` shows: a machine whose link is down and whose
/// reconnects need the user is NeedsLogin, not Offline (MP-16). The machine
/// service applies it where it builds the machine.
pub(crate) fn needs_login(work: &Work, entry: &HostEntry, reach: pb::Reachability) -> pb::Reachability {
    if reach == pb::Reachability::Offline && is_interactive(work.machines.hosts_file(), &entry.name) {
        pb::Reachability::NeedsLogin
    } else {
        reach
    }
}

/// The API's kind of a prompt (D167).
fn prompt_kind(kind: humata2_machines::PromptKind) -> pb::LoginPromptKind {
    use humata2_machines::PromptKind;
    match kind {
        PromptKind::Password => pb::LoginPromptKind::Password,
        PromptKind::Passphrase => pb::LoginPromptKind::Passphrase,
        PromptKind::Code => pb::LoginPromptKind::Code,
        PromptKind::Confirm => pb::LoginPromptKind::Confirm,
        PromptKind::Device => pb::LoginPromptKind::Device,
    }
}

/// An add's "needs a login" (MP-13): which prompt the connect printed, or
/// that it hung, so the app can say why it opened the login (D167).
pub(crate) fn login_needs_user(machine: &str, reason: &humata2_machines::LoginReason) -> pb::LoginNeedsUser {
    use humata2_machines::LoginReason;
    let (kind, line, hung) = match reason {
        LoginReason::Prompted(prompt) => (prompt_kind(prompt.kind), prompt.line.clone(), false),
        LoginReason::Hung => (pb::LoginPromptKind::Unspecified, String::new(), true),
    };
    pb::LoginNeedsUser {
        provider: None,
        machine: Some(machine_id(machine)),
        prompt_kind: kind as i32,
        prompt_line: line,
        hung,
    }
}

/// A login's failure as the API's Error: what failed and how to fix it.
fn failed(error: &LoginError, provider: Option<&str>) -> pb::Error {
    let message = status::chain(error);
    let (fix, exit_code, output_tail) = match error {
        LoginError::Takeover(TakeoverError::NoShell { output_tail, .. }) => (
            "this login ends in a menu or a program, not a shell (MP-18): give the provider an `identity` command that prints SSH details instead, or point `connect` at a wrapper script that ends in a POSIX shell; then log in again",
            -1,
            output_tail.clone(),
        ),
        LoginError::Takeover(TakeoverError::SyncTimedOut { output_tail, .. } | TakeoverError::TerminalEnded { output_tail, .. }) => (
            "check that humatad2 starts on the machine (the output says what it printed), then log in again",
            -1,
            output_tail.clone(),
        ),
        LoginError::Takeover(TakeoverError::Install { output_tail, .. } | TakeoverError::Bundle { output_tail, .. }) => (
            "check that ~/.humata2 on the machine is writable and has room (the output says what the install printed), then log in again",
            -1,
            output_tail.clone(),
        ),
        LoginError::Takeover(_) => (
            "log in again; press Done only when you see the machine's shell prompt",
            -1,
            String::new(),
        ),
        LoginError::Ended { exit, output_tail, .. } => (
            "log in again and press Done only when you see a shell prompt",
            match exit {
                Some(humata2_machines::Exit::Code(code)) => *code,
                _ => -1,
            },
            output_tail.clone(),
        ),
        LoginError::Enrollment { output_tail, .. } => (
            "make sure the machine's home folder is writable, then log in again",
            -1,
            output_tail.clone(),
        ),
        LoginError::Stalled { output_tail, .. } => (
            "log in again and press Done only when you see a POSIX shell prompt (sh, bash, zsh); a shell that doesn't read typed commands can't be taken over",
            -1,
            output_tail.clone(),
        ),
        LoginError::StillNeedsLogin { output_tail, .. } | LoginError::Authorize { output_tail, .. } => (
            "log in again if the connect asks for one, then make the machine unattended again; the machine was left as it was",
            -1,
            output_tail.clone(),
        ),
        _ => (
            "fix what the message names, then try again; nothing was changed",
            -1,
            String::new(),
        ),
    };
    pb::Error {
        message,
        fix: fix.to_owned(),
        retry: None,
        kind: Some(pb::error::Kind::ProviderStepFailed(pb::ProviderStepFailed {
            provider: provider.map(|id| pb::ProviderId { value: id.to_owned() }),
            step: pb::provider_step_failed::Step::Connect as i32,
            exit_code,
            output_tail,
        })),
    }
}

/// A login's failure as the status a call returns.
fn login_status(error: &LoginError, provider: Option<&str>) -> Status {
    if let LoginError::Manage { source, .. } = error {
        return status_of(source);
    }
    let code = match error {
        LoginError::NotInteractive { .. }
        | LoginError::NoSshAddress { .. }
        | LoginError::HostKeyNotPinned { .. }
        | LoginError::IdentityProvider { .. }
        | LoginError::NoLink { .. } => Code::FailedPrecondition,
        _ => Code::Aborted,
    };
    status::failure(code, failed(error, provider))
}

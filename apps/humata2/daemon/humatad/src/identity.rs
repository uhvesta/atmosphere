//! Who a connection is (BOUNDARIES.md §2, MCP.md §1). Each session tab's
//! shell gets a session token of its own in `HUMATA_SESSION_TOKEN`; a call
//! that carries one is that session's agent. A call that carries a CLI
//! login (`cli_login`) from the terminal session it bound is the user. A
//! call with no credential is refused, except WhoAmI and logging in, which
//! answer it as unauthenticated; a test daemon built with the `admin`
//! feature and run with `--admin` takes it for the user instead. A call
//! with a script token (`humata token create`, SEC-14) is that script: not
//! the user, with no session of its own; it wins over a CLI login and
//! loses to a session token, and is not bound to a process tree. A session
//! token is good only from the process tree of its tab's holder; used from
//! anywhere else it is refused and replaced at the session's next resume
//! (SEC-15). Only
//! hashes of the session tokens are kept, in a 0600 file in the state, so
//! tabs that outlive a daemon restart keep theirs.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use humata2_access::{
    ChannelDeskAccess, ChannelId, DeskAccess, MachineId, SessionId, SessionKind, SessionRef,
};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{SessionRecord, Store, StoreError, write_atomic};
use sha2::{Digest, Sha256};
use tonic::body::Body;
use tonic::codegen::http;
use tonic::service::Interceptor;
use tonic::{Request, Status};

use crate::cli_login::CliLogins;
use crate::script_tokens::{ScriptCaller, ScriptTokenUnusable, ScriptTokens};
use crate::error::DaemonError;
use crate::relay_auth::{ConnId, MethodPath, PeerFacts, RelayAuth};
use crate::status;
use crate::terminal_session::{self, Processes};

/// The gRPC metadata key a session token travels in.
pub(crate) const TOKEN_HEADER: &str = "humata-session-token";

/// The gRPC metadata key a CLI login's token travels in.
pub(crate) const LOGIN_HEADER: &str = "humata-cli-login";

/// The gRPC metadata key a script token travels in.
pub(crate) const SCRIPT_HEADER: &str = "humata-script-token";

/// A token's random bytes: 32 hex characters.
const TOKEN_BYTES: usize = 16;

pub(crate) use humata2_daemon_ports::{AgentCaller, Caller, UserVia};

pub(crate) fn agent_caller_of(record: &SessionRecord, here: &str) -> AgentCaller {
    AgentCaller {
        session: session_ref(record, here),
        agent: record.agent.clone(),
        title: record.title.clone(),
        method: String::new(),
        context_only_parent: None,
        desk: None,
        review_push: None,
        channel_desk: None,
    }
}

/// Works out from the store whether this session is a context-only fork
/// still in its parent's folder, so the policy judges its calls on the
/// parent's clones as its own (D185, D187).
pub(crate) fn resolve_fork(mut agent: AgentCaller, store: &Store) -> Result<AgentCaller, StoreError> {
    agent.context_only_parent = context_only_parent(store, agent.session.id.as_str())?;
    agent.desk = desk_access(store, agent.session.id.as_str())?;
    if let Some((channel, access)) =
        crate::desk_service::channel_desk::access_of(store, agent.session.id.as_str())?
    {
        agent.session.channel = channel;
        agent.channel_desk = Some(access);
    }
    agent.review_push = store
        .review_session(agent.session.id.as_str())?
        .map(|review| review.push_allowed);
    Ok(agent)
}

/// The switches of `session`'s Desk row, if it has one (DK-4).
fn desk_access(store: &Store, session: &str) -> Result<Option<DeskAccess>, StoreError> {
    Ok(store.desk(session)?.map(|row| DeskAccess {
        boards: row.boards,
        channels_and_machines: row.channels_and_machines,
    }))
}

/// The parent a session is a context-only fork of, while it still works in
/// the parent's folder (D185, D187). A fork that has its own copy has its
/// own folder by then, so it is no longer one.
fn context_only_parent(store: &Store, session: &str) -> Result<Option<SessionId>, StoreError> {
    let Some(origin) = store.fork_origin(session)? else {
        return Ok(None);
    };
    if origin.kind != pb::ForkKind::ContextOnly as i32 {
        return Ok(None);
    }
    let (Some(fork), Some(parent)) = (store.session(session)?, store.session(&origin.parent)?)
    else {
        return Ok(None);
    };
    Ok((fork.folder == parent.folder).then(|| SessionId::new(parent.id.as_str())))
}

/// The dotted operation id (TP-16) of the gRPC method at `path`
/// (`/humata.v1.ChannelService/ListChannels` → `channel.list`).
fn operation_of(path: &str) -> String {
    let Some((service, method)) = path.trim_start_matches('/').split_once('/') else {
        return String::new();
    };
    let service = service.rsplit('.').next().unwrap_or(service);
    let area = snake(service.strip_suffix("Service").unwrap_or(service));
    let words = snake(method);
    let plural = format!("{area}s");
    let verb: Vec<&str> = words
        .split('_')
        .filter(|word| *word != area && *word != plural)
        .collect();
    if verb.is_empty() {
        return format!("{area}.{words}");
    }
    format!("{area}.{}", verb.join("_"))
}

/// `ListChannels` → `list_channels`.
fn snake(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (at, letter) in name.chars().enumerate() {
        if letter.is_ascii_uppercase() {
            if at > 0 {
                out.push('_');
            }
            out.push(letter.to_ascii_lowercase());
        } else {
            out.push(letter);
        }
    }
    out
}

/// A session as the policy sees it. A session with no machine recorded
/// runs on this one.
pub(crate) fn session_ref(record: &SessionRecord, here: &str) -> SessionRef {
    let machine = if record.machine.is_empty() {
        here
    } else {
        record.machine.as_str()
    };
    SessionRef {
        id: SessionId::new(record.id.as_str()),
        channel: ChannelId::new(record.channel.as_str()),
        machine: MachineId::new(machine),
    }
}

/// The id the host minted for what it asks this machine to create (D139, D150), from
/// `humata-host-assigned-id`. Only a caller that came through the proven relay may name
/// one (D144); anyone else's header is ignored.
pub(crate) fn host_assigned_id<T>(
    request: &Request<T>,
    caller: &Caller,
) -> Result<Option<String>, Status> {
    if !matches!(caller, Caller::User(UserVia::Host)) {
        return Ok(None);
    }
    let Some(value) = request.metadata().get(crate::host_relay::HOST_ASSIGNED_ID) else {
        return Ok(None);
    };
    let id = value
        .to_str()
        .map_err(|_| status::missing(crate::host_relay::HOST_ASSIGNED_ID))?;
    if id.len() > 80 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err(status::missing(crate::host_relay::HOST_ASSIGNED_ID));
    }
    Ok(Some(id.to_owned()))
}

/// Marks a call that carried no credential on a method open to anyone.
#[derive(Clone, Copy, Debug)]
struct Unauthenticated;

/// Marks a request for WhoAmI, the one method an added agent may call
/// (SE-29). Set by `mark_open` only.
#[derive(Clone, Copy, Debug)]
struct AddedAgentMayCall;

/// Marks a request for a method a caller with no credential may make:
/// WhoAmI, so a refused shell can learn why, and logging in.
#[derive(Clone, Copy, Debug)]
struct OpenToAnyone;

/// Marks a call to [`STOP_SESSIONS_PATH`] that carried no credential at
/// all and came from a process outside every Humata session: a person
/// typing `humata stop` on the machine (D263). Set by `authenticate` only.
#[derive(Clone, Copy, Debug)]
struct OutsideSessions;

/// The path of the one method that takes a caller outside every session
/// with no credential.
const STOP_SESSIONS_PATH: &str = "/humata.v1.DaemonService/StopSessions";

/// The methods `OpenToAnyone` covers, as (service, method).
const OPEN_TO_ANYONE: [(&str, &str); 5] = [
    (pb::daemon_service_server::SERVICE_NAME, "WhoAmI"),
    (pb::cli_auth_service_server::SERVICE_NAME, "BeginCliSession"),
    // D144: the peer link proves itself on a connection that has no
    // credential yet; neither call grants anything alone.
    (pb::daemon_service_server::SERVICE_NAME, "BeginRelay"),
    (pb::daemon_service_server::SERVICE_NAME, "CompleteRelay"),
    // D263: `humata stop` on the machine itself needs no login; the
    // handler decides who the caller is and what each may do.
    (pb::daemon_service_server::SERVICE_NAME, "StopSessions"),
];

/// Marks `request` when its method is open to anyone. Runs before
/// `Authenticate`, which can't see the path. Extensions are the server's
/// own: a client can't set this marker.
pub(crate) fn mark_open(request: http::Request<Body>) -> http::Request<Body> {
    // D144: the relay's per-connection check reads the method from here.
    let mut request = crate::relay_auth::mark_method(request);
    let open = request
        .uri()
        .path()
        .strip_prefix('/')
        .and_then(|path| path.split_once('/'))
        .is_some_and(|route| OPEN_TO_ANYONE.iter().any(|listed| *listed == route));
    if open {
        request.extensions_mut().insert(OpenToAnyone);
    }
    let who_am_i = (pb::daemon_service_server::SERVICE_NAME, "WhoAmI");
    if request
        .uri()
        .path()
        .strip_prefix('/')
        .and_then(|path| path.split_once('/'))
        .is_some_and(|route| route == who_am_i)
    {
        request.extensions_mut().insert(AddedAgentMayCall);
    }
    request
}

/// Makes every call on the peer link the peer's. Whatever credential a
/// request carries is ignored: the link is the identity, and nothing on it
/// is the user or an agent, admin daemon or not.
pub(crate) fn mark_peer(mut request: http::Request<Body>) -> http::Request<Body> {
    request.extensions_mut().insert(Caller::Peer);
    request
}

/// How the user reached this daemon, as records and whoami both name it.
/// Admin mode's token-less user is the app's authority (D41), so it is `App`.
pub(crate) fn user_via(via: &UserVia) -> pb::user_actor::Via {
    match via {
        UserVia::App | UserVia::Host => pb::user_actor::Via::App,
        UserVia::CliLogin { on_host: true, .. } => pb::user_actor::Via::CliOnHost,
        UserVia::CliLogin { on_host: false, .. } => pb::user_actor::Via::CliOnMachine,
    }
}

/// Who `caller` is, as events and records name them.
pub(crate) fn actor(caller: &Caller) -> pb::Actor {
    use pb::actor::Actor;
    let actor = match caller {
        Caller::User(via) => Actor::User(pb::UserActor {
            via: user_via(via) as i32,
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
        Caller::Script(script) => Actor::Script(pb::ScriptActor {
            token_id: script.id.clone(),
            name: script.name.clone(),
        }),
        Caller::Peer => Actor::System(pb::SystemActor { reason: 0 }),
    };
    pb::Actor { actor: Some(actor) }
}

/// The caller the interceptor found for `request`.
pub(crate) fn caller<T>(request: &Request<T>) -> Result<Caller, Status> {
    request
        .extensions()
        .get::<Caller>()
        .cloned()
        .ok_or_else(unchecked)
}

/// The caller of a method open to anyone: none when the call carried no
/// credential.
pub(crate) fn caller_if_any<T>(request: &Request<T>) -> Result<Option<Caller>, Status> {
    if let Some(caller) = request.extensions().get::<Caller>() {
        return Ok(Some(caller.clone()));
    }
    match request.extensions().get::<Unauthenticated>() {
        Some(_) => Ok(None),
        None => Err(unchecked()),
    }
}

/// True when the call had no credential and came from outside every Humata
/// session, on a method that takes such a caller (`StopSessions`).
pub(crate) fn outside_every_session<T>(request: &Request<T>) -> bool {
    request.extensions().get::<OutsideSessions>().is_some()
}

/// The process at the other end of the call's socket, when the kernel
/// said. Read when the connection was accepted.
pub(crate) fn peer_pid<T>(request: &Request<T>) -> Option<i32> {
    request
        .extensions()
        .get::<humata2_app_peer::PeerInfo>()
        .and_then(|info| info.uds.peer_cred)
        .and_then(|cred| cred.pid())
}

/// Which accepted connection of the socket the call came on, as the accept
/// loop numbered it (D144). None for a request that didn't come over the
/// socket.
pub(crate) fn conn_id<T>(request: &Request<T>) -> Option<ConnId> {
    request
        .extensions()
        .get::<humata2_app_peer::PeerInfo>()
        .map(|info| ConnId(info.conn))
}

/// What the kernel said of the process at the other end of the call's
/// socket, read when the connection was accepted (D144).
pub(crate) fn peer_facts<T>(request: &Request<T>) -> PeerFacts {
    let cred = request
        .extensions()
        .get::<humata2_app_peer::PeerInfo>()
        .and_then(|info| info.uds.peer_cred);
    PeerFacts {
        pid: cred.and_then(|cred| cred.pid()),
        uid: cred.map(|cred| cred.uid()),
    }
}

/// Whether the other end of the call's socket is the signed Humata app
/// (D135), checked when the connection was accepted.
fn peer_is_app<T>(request: &Request<T>) -> bool {
    request
        .extensions()
        .get::<humata2_app_peer::PeerInfo>()
        .is_some_and(|info| info.app)
}

fn unchecked() -> Status {
    status::internal(
        "a call reached its handler without its caller being checked",
        "",
    )
}

/// The tokens this daemon gave out, by their hash.
pub(crate) struct Identities {
    file: PathBuf,
    machine: String,
    tokens: Mutex<HashMap<String, Holder>>,
}

#[derive(Clone)]
struct Holder {
    tab: String,
    caller: AgentCaller,
    /// Whose token it is: the session's agent, or an agent added to the
    /// session's tabs.
    identity: TabIdentity,
}

/// The variable a session tab's shell finds the daemon's socket in.
const SOCKET_VAR: &str = "HUMATA_SOCKET";

/// What a session tab's shell is started with: its session token, and the
/// daemon's socket, so `humata` run there reaches this daemon even when it
/// serves on a socket other than the default one.
pub(crate) fn tab_env(token: String, socket: &Path) -> Vec<(OsString, OsString)> {
    let mut env = vec![
        (OsString::from(humata2_mcp::TOKEN_VAR), OsString::from(token)),
        // The tab's shell starts in another folder than the daemon did.
        (
            OsString::from(SOCKET_VAR),
            std::path::absolute(socket)
                .unwrap_or_else(|_| socket.to_owned())
                .into_os_string(),
        ),
    ];
    // D748: `humata` is found by name in every session tab.
    if let Some(path) = tools_path(std::env::var_os(PATH_VAR)) {
        env.push((OsString::from(PATH_VAR), path));
    }
    env
}

/// The variable a tab's programs are found on.
pub(crate) const PATH_VAR: &str = "PATH";

/// The folder Humata's own tools (`humata`) are in, put first on every
/// session tab's `PATH` (D748). Set once, at the daemon's start.
static TOOLS_FOLDER: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Names the folder Humata's tools are in for every tab started from now.
pub(crate) fn set_tools_folder(folder: &Path) {
    let _ = TOOLS_FOLDER.set(folder.to_owned());
}

/// `path` with Humata's tools folder first, when the daemon named one.
pub(crate) fn tools_path(path: Option<OsString>) -> Option<OsString> {
    with_tools_first(TOOLS_FOLDER.get()?, path)
}

/// `path` (a `PATH` value) with `tools` first and nowhere else; just
/// `tools` when there is no `path`. None if `tools` can't be on a `PATH`
/// (it holds a `:`).
fn with_tools_first(tools: &Path, path: Option<OsString>) -> Option<OsString> {
    let rest: Vec<PathBuf> = path
        .as_deref()
        .map(|path| std::env::split_paths(path).filter(|entry| entry != tools).collect())
        .unwrap_or_default();
    std::env::join_paths(std::iter::once(tools.to_owned()).chain(rest)).ok()
}

/// What a token minted for a tab says its holder is (SE-29, SEC-8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabIdentity {
    /// The session's own agent, or a shell in its tree: calls act as the
    /// session (SEC-8, SEC-15).
    Session,
    /// An agent added to the session (D103). It is a terminal that runs an
    /// agent and has no identity of its own: every call but WhoAmI is
    /// refused.
    AddedAgent,
}

impl TabIdentity {
    /// The word the token file keeps for it.
    fn word(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::AddedAgent => "added",
        }
    }

    fn of_word(word: Option<&str>) -> Self {
        match word {
            Some("added") => Self::AddedAgent,
            _ => Self::Session,
        }
    }
}

/// An added agent's call, refused (SE-29, D103).
fn added_agent_refused(session: &str) -> Status {
    status::failure(
        tonic::Code::PermissionDenied,
        status::error(
            format!(
                "an agent added to session {session} has no identity of its own, so Humata's tools refuse its calls: only the session's main agent acts for the session"
            ),
            "fork the session, so this agent gets a session of its own and its own identity; then call Humata's tools from the fork",
            pb::error::Kind::Denied(pb::Denied {
                source: pb::denied::Source::Unspecified as i32,
                capability: 0,
                request: None,
                user_note: String::new(),
            }),
        ),
    )
}

impl Identities {
    /// The tokens saved in `file`, less those whose tab or session is gone.
    pub(crate) fn load(file: PathBuf, store: &Store, machine: &str) -> Result<Self, DaemonError> {
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(source) => return Err(DaemonError::Tokens { path: file, source }),
        };
        let mut tokens = HashMap::new();
        let mut dropped = false;
        for line in text.lines() {
            let mut fields = line.split_whitespace();
            let (Some(hash), Some(tab), Some(session)) =
                (fields.next(), fields.next(), fields.next())
            else {
                dropped = true;
                continue;
            };
            let identity = TabIdentity::of_word(fields.next());
            let open = match store.tab(tab) {
                Ok(found) => found.is_some_and(|found| found.session.as_deref() == Some(session)),
                Err(StoreError::InvalidId { .. }) => false,
                Err(source) => return Err(DaemonError::Store { source }),
            };
            let record = match store.session(session) {
                Ok(record) if open => record,
                Ok(_) | Err(StoreError::InvalidId { .. }) => None,
                Err(source) => return Err(DaemonError::Store { source }),
            };
            match record {
                Some(record) => {
                    tokens.insert(
                        hash.to_owned(),
                        Holder {
                            tab: tab.to_owned(),
                            caller: agent_caller_of(&record, machine),
                            identity,
                        },
                    );
                }
                None => dropped = true,
            }
        }
        let identities = Self {
            file,
            machine: machine.to_owned(),
            tokens: Mutex::new(tokens),
        };
        if dropped {
            let tokens = lock(&identities.tokens);
            identities
                .save(&tokens)
                .map_err(|source| DaemonError::Store { source })?;
        }
        Ok(identities)
    }

    /// A new token for `tab` of `session`, saved before it's returned.
    /// Blocks on the write.
    pub(crate) fn mint(&self, tab: &str, session: &SessionRecord) -> Result<String, StoreError> {
        self.mint_as(tab, session, TabIdentity::Session)
    }

    /// As [`Self::mint`], for a token that says its holder is `identity`.
    pub(crate) fn mint_as(
        &self,
        tab: &str,
        session: &SessionRecord,
        identity: TabIdentity,
    ) -> Result<String, StoreError> {
        let (token, hash) = new_token();
        let mut tokens = lock(&self.tokens);
        tokens.insert(
            hash.clone(),
            Holder {
                tab: tab.to_owned(),
                caller: agent_caller_of(session, &self.machine),
                identity,
            },
        );
        if let Err(error) = self.save(&tokens) {
            tokens.remove(&hash);
            return Err(error);
        }
        Ok(token)
    }

    /// Ends `tab`'s token. It stops working at once; if the save fails, the
    /// next start drops it anyway, since its tab is gone by then.
    pub(crate) fn revoke(&self, tab: &str) -> Result<(), StoreError> {
        let mut tokens = lock(&self.tokens);
        let before = tokens.len();
        tokens.retain(|_, holder| holder.tab != tab);
        if tokens.len() == before {
            return Ok(());
        }
        self.save(&tokens)
    }

    fn find(&self, token: &str) -> Option<Holder> {
        lock(&self.tokens).get(&digest(token)).cloned()
    }

    fn save(&self, tokens: &HashMap<String, Holder>) -> Result<(), StoreError> {
        let mut text = String::new();
        for (hash, holder) in tokens {
            let _ = writeln!(
                text,
                "{hash} {} {} {}",
                holder.tab,
                holder.caller.session.id,
                holder.identity.word()
            );
        }
        write_atomic(&self.file, text.as_bytes())
    }
}

/// Finds each call's caller from its token and puts it in the request's
/// extensions. A token this daemon didn't give out, a CLI login from
/// outside its terminal session, or no credential on a method not open to
/// anyone, is refused before any handler runs. A session token wins over
/// a script token, which wins over a CLI login. The token itself is never logged.
#[derive(Clone)]
pub(crate) struct Authenticate {
    identities: Arc<Identities>,
    logins: Arc<CliLogins>,
    scripts: Arc<ScriptTokens>,
    /// Where the tabs and their holders' pids are, to place a token's use.
    store: Arc<Store>,
    processes: Arc<dyn Processes>,
    /// The role of the daemon these calls reach: a CLI login's via depends on it.
    role: pb::DaemonRole,
    /// What tells the peer link's connection from an ordinary one (D144);
    /// none where nothing relays.
    relay: Option<Arc<RelayAuth>>,
    /// Admin mode: a call with no credential is the user. Test daemons
    /// only; release builds pass `None`.
    admin: Option<Admin>,
}

pub(crate) use humata2_daemon_ports::Admin;

impl Authenticate {
    pub(crate) fn new(
        identities: Arc<Identities>,
        logins: Arc<CliLogins>,
        scripts: Arc<ScriptTokens>,
        store: Arc<Store>,
        processes: Arc<dyn Processes>,
        role: pb::DaemonRole,
    ) -> Self {
        Self {
            identities,
            logins,
            scripts,
            store,
            processes,
            role,
            relay: None,
            admin: None,
        }
    }

    /// Takes a call with no credential from `admin` for the user, as a test
    /// daemon run with `--admin` or `--admin-pid` does.
    pub(crate) fn with_admin(mut self, admin: Option<Admin>) -> Self {
        self.admin = admin;
        self
    }

    /// Takes a connection that completed the relay's proof (D144) for the
    /// host's, for the methods the relay allows.
    pub(crate) fn with_relay(mut self, relay: Arc<RelayAuth>) -> Self {
        self.relay = Some(relay);
        self
    }

    /// Who a call with no credential from `pid` is, if anyone. `app` says
    /// the peer passed the signed app's code-signing check (D135); even
    /// then a process inside a session's terminal is not the user, so an
    /// agent can't borrow the app's binary to act as the user.
    fn without_credential(&self, pid: Option<i32>, app: bool) -> Option<Caller> {
        match self.admin {
            Some(Admin::Anyone) => return Some(Caller::User(UserVia::App)),
            Some(Admin::Process(admin)) if pid == Some(admin) => {
                return Some(Caller::User(UserVia::App));
            }
            Some(Admin::Process(_)) | None => {}
        }
        (app && !self.in_a_session(pid)).then_some(Caller::User(UserVia::App))
    }

    /// Whether `pid` is inside any tab's process tree. Fails closed: no
    /// pid, or tabs that can't be read, count as inside.
    pub(crate) fn in_a_session(&self, pid: Option<i32>) -> bool {
        let Some(pid) = pid else {
            return true;
        };
        match self.store.tabs() {
            Ok(tabs) => tabs.iter().any(|tab| {
                i32::try_from(tab.holder_pid)
                    .ok()
                    .filter(|holder| *holder > 0)
                    .is_some_and(|holder| in_tree(&*self.processes, pid, holder))
            }),
            Err(error) => {
                crate::log::say!(
                    "humatad2: couldn't read the tabs to check that the app's call comes from outside every session, so it is refused: {}",
                    status::chain(&error)
                );
                true
            }
        }
    }
}

impl Authenticate {
    /// The caller's session kind, worked out once when the caller is
    /// identified (D185, D187). A store error refuses the call: guessing
    /// Ordinary could let a fork write its parent's code under the wrong rule.
    fn resolve_fork(&self, agent: AgentCaller) -> Result<AgentCaller, Status> {
        let session = agent.session.id.to_string();
        resolve_fork(agent, &self.store).map_err(|error| {
            status::internal(
                format!(
                    "couldn't read where session {session} came from, so its access can't be checked: {}; try the call again, and if it keeps failing, restart humatad2",
                    status::chain(&error)
                ),
                "",
            )
        })
    }

    /// Whether the process at the other end may use `holder`'s token: it
    /// must be in the process tree of the holder of the tab the token was
    /// minted for (SEC-15). Fails closed: no pid, no holder, a tab that is
    /// gone, or a process that can't be read all refuse. A misused token
    /// is revoked, so the session gets a new one when it next resumes.
    fn token_is_in_its_tree(&self, holder: &Holder, pid: Option<i32>) -> Result<(), Status> {
        // An `--admin` test daemon lets the test process stand in for any
        // tab's shell. `--admin-pid` and release daemons enforce.
        if matches!(self.admin, Some(Admin::Anyone)) {
            return Ok(());
        }
        let holder_pid = match self.store.tab(&holder.tab) {
            Ok(Some(tab)) => i32::try_from(tab.holder_pid).ok().filter(|pid| *pid > 0),
            Ok(None) => None,
            Err(error) => {
                return Err(status::internal(
                    format!(
                        "couldn't read tab {} to check who is using its session token: {}",
                        holder.tab,
                        status::chain(&error)
                    ),
                    "",
                ));
            }
        };
        if pid
            .zip(holder_pid)
            .is_some_and(|(pid, holder_pid)| in_tree(&*self.processes, pid, holder_pid))
        {
            return Ok(());
        }
        let session = holder.caller.session.id.as_str();
        crate::log::say!(
            "humatad2: IDENTITY_MISUSED: session {session}'s token was used from process {}, outside its terminal's process tree; the token is revoked and replaced when the session next resumes",
            pid.map_or_else(|| "unknown".to_owned(), |pid| pid.to_string())
        );
        if let Err(error) = self.identities.revoke(&holder.tab) {
            crate::log::say!(
                "humatad2: couldn't revoke tab {}'s misused token: {}",
                holder.tab,
                status::chain(&error)
            );
        }
        Err(token_outside_its_session(session))
    }

    fn authenticate(
        &self,
        mut request: Request<()>,
        pid: Option<i32>,
        app: bool,
    ) -> Result<Request<()>, Status> {
        // D144: a connection that proved itself as the link's is the host's
        // for the methods the relay allows, with no credential on the call;
        // anything else on it is refused. An ordinary connection falls
        // through.
        if let (Some(relay), Some(conn)) = (&self.relay, conn_id(&request)) {
            let method = request
                .extensions()
                .get::<MethodPath>()
                .map(|path| path.0.clone())
                .unwrap_or_default();
            let carries = [TOKEN_HEADER, LOGIN_HEADER, SCRIPT_HEADER]
                .iter()
                .any(|header| request.metadata().contains_key(*header));
            if relay
                .authorize(conn, &method, carries)
                .map_err(|refusal| refusal.status())?
                .is_some()
            {
                request.extensions_mut().insert(Caller::User(UserVia::Host));
                return Ok(request);
            }
        }
        let open = request.extensions().get::<OpenToAnyone>().is_some();
        let session = request.metadata().get(TOKEN_HEADER);
        let login = request.metadata().get(LOGIN_HEADER);
        let script = request.metadata().get(SCRIPT_HEADER);
        let stop_sessions = request
            .extensions()
            .get::<MethodPath>()
            .is_some_and(|path| path.0 == STOP_SESSIONS_PATH);
        let mut outside = false;
        let caller = match (session, script, login) {
            (Some(value), _, _) => match value
                .to_str()
                .ok()
                .and_then(|token| self.identities.find(token))
            {
                Some(holder) => {
                    self.token_is_in_its_tree(&holder, pid)?;
                    match holder.identity {
                        TabIdentity::Session => {
                            let agent = AgentCaller {
                                method: request
                                    .extensions()
                                    .get::<MethodPath>()
                                    .map(|path| path.0.clone())
                                    .unwrap_or_default(),
                                ..holder.caller
                            };
                            Some(Caller::Agent(self.resolve_fork(agent)?))
                        }
                        // An added agent may ask who it is; nothing else.
                        // It answers as a caller with no credential, so it
                        // learns why its other calls are refused.
                        TabIdentity::AddedAgent
                            if request.extensions().get::<AddedAgentMayCall>().is_some() =>
                        {
                            None
                        }
                        TabIdentity::AddedAgent => {
                            return Err(added_agent_refused(holder.caller.session.id.as_str()));
                        }
                    }
                }
                None => return Err(status::identity_mismatch()),
            },
            // A script token (SEC-14): judged strictly like a login, and
            // not bound to a process tree. On a method open to anyone an
            // unusable one counts as no credential.
            (None, Some(value), _) => {
                let checked = match value.to_str() {
                    Ok(token) => self.scripts.caller(token),
                    Err(_) => Err(ScriptTokenUnusable::Unknown),
                };
                match checked {
                    Ok(script) => Some(Caller::Script(script)),
                    Err(_) if open => None,
                    Err(unusable) => return Err(status::script_token_unusable(&unusable)),
                }
            }
            // Judged strictly, even on an admin daemon: a login that
            // doesn't work here is never taken for the user. On a method
            // open to anyone it counts as no credential, so a terminal with
            // a stale login can still log in again.
            (None, None, Some(value)) => {
                let checked = match value.to_str() {
                    Ok(token) => self.logins.caller(token, pid),
                    Err(_) => Err(crate::cli_login::LoginUnusable::Unknown),
                };
                match checked {
                    Ok((login, method)) => Some(Caller::User(UserVia::CliLogin {
                        login,
                        method,
                        // Same rule as whoami always had: only a machine is not the host.
                        on_host: self.role != pb::DaemonRole::Machine,
                    })),
                    Err(_) if open => None,
                    Err(unusable) => return Err(status::login_unusable(&unusable)),
                }
            }
            (None, None, None) => {
                let caller = self.without_credential(pid, app);
                outside = caller.is_none() && stop_sessions && !self.in_a_session(pid);
                caller
            }
        };
        match caller {
            Some(caller) => {
                request.extensions_mut().insert(caller);
            }
            None if open => {
                request.extensions_mut().insert(Unauthenticated);
                if outside {
                    request.extensions_mut().insert(OutsideSessions);
                }
            }
            None => return Err(status::unauthenticated()),
        }
        Ok(request)
    }
}

impl Interceptor for Authenticate {
    fn call(&mut self, request: Request<()>) -> Result<Request<()>, Status> {
        let pid = peer_pid(&request);
        let app = peer_is_app(&request);
        self.authenticate(request, pid, app)
    }
}

/// Whether `pid` is `holder_pid` or descends from it, or its terminal
/// session's leader does: a process the holder's tab started, even one a
/// double fork left with init as its parent, as long as it stayed in the
/// tab's terminal session. One that has no terminal counts by parent
/// chain alone.
fn in_tree(processes: &dyn Processes, pid: i32, holder_pid: i32) -> bool {
    let holders = HashSet::from([holder_pid]);
    match terminal_session::terminal_session(processes, pid) {
        Ok(session) => terminal_session::descends_from(processes, pid, &session, &holders),
        Err(_) => terminal_session::lineage(processes, pid).any(|ancestor| ancestor == holder_pid),
    }
}

/// A session's token used from outside its terminal's process tree
/// (SEC-15).
fn token_outside_its_session(session: &str) -> Status {
    status::failure(
        tonic::Code::Unauthenticated,
        status::error(
            format!(
                "session {session}'s token was used from a process that isn't in that session's terminal, so humatad2 refused the call and revoked the token"
            ),
            "run humata from inside the session's own terminal tab, where it picks the token up itself; to act as yourself from another terminal, run `humata login`. The session gets a new token the next time it resumes, so restart the session if its own tools now fail",
            pb::error::Kind::IdentityMismatch(pb::IdentityMismatch {
                claimed: None,
                claimed_machine: None,
            }),
        ),
    )
}

/// A new random token, and the hash that is all the daemon keeps of it.
pub(crate) fn new_token() -> (String, String) {
    let mut bytes = [0u8; TOKEN_BYTES];
    rand::fill(&mut bytes[..]);
    let token = hex(&bytes);
    let hash = digest(&token);
    (token, hash)
}

pub(crate) fn digest(token: &str) -> String {
    hex(Sha256::digest(token.as_bytes()).as_slice())
}

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tabs_path_starts_with_humatas_tools_and_keeps_the_rest_once() {
        let tools = Path::new("/opt/humata/bin");
        let path = with_tools_first(tools, Some(OsString::from("/usr/bin:/opt/humata/bin:/bin"))).unwrap();
        assert_eq!(path, "/opt/humata/bin:/usr/bin:/bin");
        assert_eq!(with_tools_first(tools, None).unwrap(), "/opt/humata/bin");
        assert_eq!(with_tools_first(Path::new("/a:b"), None), None, "a folder with a colon can't be on PATH");
    }
    use crate::test_support::Fake;

    #[test]
    fn a_method_path_names_its_dotted_operation() {
        for (path, operation) in [
            ("/humata.v1.ChannelService/ListChannels", "channel.list"),
            ("/humata.v1.SessionService/StartSession", "session.start"),
            ("/humata.v1.InboxService/RequestAccess", "inbox.request_access"),
            ("/humata.v1.TerminalService/OpenShellTab", "terminal.open_shell_tab"),
            ("/humata.v1.ChannelService/GetChannel", "channel.get"),
            ("not a path", ""),
        ] {
            assert_eq!(operation_of(path), operation, "{path}");
        }
    }

    fn record() -> SessionRecord {
        SessionRecord {
            id: "se_main".to_owned(),
            channel: "ch_one".to_owned(),
            machine: String::new(),
            title: "main".to_owned(),
            agent: "claude".to_owned(),
            folder: PathBuf::new(),
            created_unix_ms: 0,
        }
    }

    /// A holder (30) that started a shell (31), which ran a command (32);
    /// a command (33) that double-forked away from it, left under init in
    /// the shell's terminal session; and an unrelated process (40).
    fn processes() -> Fake {
        let mut fake = Fake::default();
        fake.add(1, 0, 1, None);
        fake.add(30, 1, 30, None);
        fake.add(31, 30, 31, Some(8));
        fake.add(32, 31, 31, Some(8));
        fake.add(33, 1, 31, Some(8));
        fake.add(40, 1, 40, Some(9));
        fake.add(41, 40, 40, Some(9));
        fake
    }

    struct Rig {
        _dir: tempfile::TempDir,
        file: PathBuf,
        store: Arc<Store>,
        identities: Arc<Identities>,
        authenticate: Authenticate,
    }

    fn rig() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(dir.path().join("store")).unwrap());
        let file = dir.path().join("tokens");
        let identities = Arc::new(Identities::load(file.clone(), &store, "ma_here").unwrap());
        let logins = Arc::new(CliLogins::new(Box::new(Fake::default()), None));
        let scripts = Arc::new(ScriptTokens::load(store.clone(), None).unwrap());
        let authenticate = Authenticate::new(
            identities.clone(),
            logins,
            scripts,
            store.clone(),
            Arc::new(processes()),
            pb::DaemonRole::Host,
        );
        Rig {
            _dir: dir,
            file,
            store,
            identities,
            authenticate,
        }
    }

    fn put_tab(rig: &Rig, id: &str, holder_pid: u32) {
        rig.store.put_session(&record()).unwrap();
        rig.store
            .put_tab(&humata2_store::TabRecord {
                id: id.to_owned(),
                session: Some("se_main".to_owned()),
                kind: humata2_store::TabKind::Shell,
                title: "shell".to_owned(),
                holder_socket: PathBuf::new(),
                holder_pid,
                created_unix_ms: 0,
                holder_generation: 0,
            })
            .unwrap();
    }

    fn call(token: &str, who_am_i: bool) -> Request<()> {
        let mut request = Request::new(());
        request
            .metadata_mut()
            .insert(TOKEN_HEADER, token.parse().unwrap());
        if who_am_i {
            request.extensions_mut().insert(OpenToAnyone);
            request.extensions_mut().insert(AddedAgentMayCall);
        }
        request
    }

    #[test]
    fn an_added_agents_call_is_refused_but_it_may_ask_who_it_is() {
        let rig = rig();
        put_tab(&rig, "tab_added", 30);
        let token = rig
            .identities
            .mint_as("tab_added", &record(), TabIdentity::AddedAgent)
            .unwrap();

        let refused = rig
            .authenticate
            .authenticate(call(&token, false), Some(31), false)
            .unwrap_err();
        assert_eq!(refused.code(), tonic::Code::PermissionDenied);
        let error = status::error_of(&refused).unwrap();
        assert!(!error.fix.is_empty());

        let answered = rig
            .authenticate
            .authenticate(call(&token, true), Some(31), false)
            .unwrap();
        assert!(caller_if_any(&answered).unwrap().is_none());
    }

    #[test]
    fn a_session_token_is_that_sessions_agent_inside_its_tabs_tree() {
        let rig = rig();
        put_tab(&rig, "tab_main", 30);
        let token = rig.identities.mint("tab_main", &record()).unwrap();

        // The shell, its command, and a double-forked one in its terminal
        // session.
        for pid in [31, 32, 33] {
            let accepted = rig
                .authenticate
                .authenticate(call(&token, false), Some(pid), false)
                .unwrap();
            assert!(matches!(
                caller(&accepted).unwrap(),
                Caller::Agent(agent) if agent.session.id.as_str() == "se_main"
            ));
        }
    }

    #[test]
    fn a_session_token_from_outside_its_tree_is_refused_and_revoked() {
        let rig = rig();
        put_tab(&rig, "tab_main", 30);
        let token = rig.identities.mint("tab_main", &record()).unwrap();

        let refused = rig
            .authenticate
            .authenticate(call(&token, false), Some(41), false)
            .unwrap_err();

        let error = status::error_of(&refused).unwrap();
        assert!(matches!(
            error.kind,
            Some(pb::error::Kind::IdentityMismatch(_))
        ));
        assert!(!error.fix.is_empty());
        // The token is gone, for its rightful holder too, until a resume.
        assert!(rig.identities.find(&token).is_none());
        assert!(
            rig.authenticate
                .authenticate(call(&token, false), Some(31), false)
                .is_err()
        );
    }

    #[test]
    fn a_session_token_is_refused_when_the_caller_or_holder_is_unknown() {
        for (holder_pid, pid) in [(30, None), (0, Some(31)), (30, Some(999))] {
            let rig = rig();
            put_tab(&rig, "tab_main", holder_pid);
            let token = rig.identities.mint("tab_main", &record()).unwrap();

            let refused = rig.authenticate.authenticate(call(&token, false), pid, false);

            assert!(refused.is_err(), "holder {holder_pid}, caller {pid:?}");
        }
    }

    #[test]
    fn a_cli_login_call_is_recorded_as_cli_on_host() {
        let caller = Caller::User(UserVia::CliLogin {
            login: "login_1".to_owned(),
            method: humata2_access::PresenceMethod::CliLogin,
            on_host: true,
        });
        let Some(pb::actor::Actor::User(user)) = actor(&caller).actor else {
            panic!("expected a user actor");
        };
        assert_eq!(user.via(), pb::user_actor::Via::CliOnHost);
    }

    #[test]
    fn a_cli_login_on_a_machine_is_recorded_as_cli_on_machine() {
        let caller = Caller::User(UserVia::CliLogin {
            login: "login_1".to_owned(),
            method: humata2_access::PresenceMethod::CliLogin,
            on_host: false,
        });
        let Some(pb::actor::Actor::User(user)) = actor(&caller).actor else {
            panic!("expected a user actor");
        };
        assert_eq!(user.via(), pb::user_actor::Via::CliOnMachine);
    }

    #[test]
    fn the_signed_app_outside_every_session_is_the_user() {
        let rig = rig();
        put_tab(&rig, "tab_main", 30);

        let accepted = rig
            .authenticate
            .authenticate(Request::new(()), Some(41), true)
            .unwrap();

        assert!(matches!(
            caller(&accepted).unwrap(),
            Caller::User(UserVia::App)
        ));
    }

    #[test]
    fn the_app_inside_a_session_or_unknown_or_unsigned_is_not_the_user() {
        let rig = rig();
        put_tab(&rig, "tab_main", 30);

        // Inside the tab's tree, an unknown process, and an unsigned peer.
        for (pid, app) in [(Some(32), true), (None, true), (Some(41), false)] {
            let refused = rig.authenticate.authenticate(Request::new(()), pid, app);
            assert!(refused.is_err(), "caller {pid:?}, app {app}");
        }
    }

    #[test]
    fn a_tokens_kind_survives_a_restart() {
        let rig = rig();
        let token = rig
            .identities
            .mint_as("tab_added", &record(), TabIdentity::AddedAgent)
            .unwrap();
        rig.store.put_session(&record()).unwrap();
        rig.store
            .put_tab(&humata2_store::TabRecord {
                id: "tab_added".to_owned(),
                session: Some("se_main".to_owned()),
                kind: humata2_store::TabKind::Agent,
                title: "added".to_owned(),
                holder_socket: PathBuf::new(),
                holder_pid: 0,
                created_unix_ms: 0,
                holder_generation: 0,
            })
            .unwrap();

        let reloaded = Identities::load(rig.file.clone(), &rig.store, "ma_here").unwrap();

        assert_eq!(
            reloaded.find(&token).map(|holder| holder.identity),
            Some(TabIdentity::AddedAgent)
        );
    }

    fn in_folder(id: &str, folder: &str) -> SessionRecord {
        SessionRecord {
            id: id.to_owned(),
            folder: PathBuf::from(folder),
            ..record()
        }
    }

    fn put_origin(rig: &Rig, fork: &str, parent: &str, kind: pb::ForkKind) {
        rig.store
            .put_fork_origin(&humata2_store::ForkOrigin {
                session_id: fork.to_owned(),
                parent: parent.to_owned(),
                kind: kind as i32,
                before_turn: 0,
                created_unix_ms: 0,
            })
            .unwrap();
    }

    #[test]
    fn a_context_only_fork_in_its_parents_folder_is_judged_as_the_parents_fork() {
        let rig = rig();
        rig.store
            .put_session(&in_folder("se_parent", "/work/parent"))
            .unwrap();
        let fork = in_folder("se_fork", "/work/parent");
        rig.store.put_session(&fork).unwrap();
        put_origin(&rig, "se_fork", "se_parent", pb::ForkKind::ContextOnly);

        let caller = resolve_fork(agent_caller_of(&fork, "ma_here"), &rig.store)
            .unwrap();

        assert_eq!(
            caller.session_kind(),
            SessionKind::ContextOnlyFork {
                parent: SessionId::new("se_parent"),
            }
        );
    }

    #[test]
    fn a_fork_with_its_own_copy_or_code_is_an_ordinary_session() {
        let rig = rig();
        rig.store
            .put_session(&in_folder("se_parent", "/work/parent"))
            .unwrap();
        // A context-only fork that has its own copy now has its own folder.
        let copied = in_folder("se_copied", "/work/copied");
        rig.store.put_session(&copied).unwrap();
        put_origin(&rig, "se_copied", "se_parent", pb::ForkKind::ContextOnly);
        // A fork that carries its own code from the start.
        let own_code = in_folder("se_own_code", "/work/own");
        rig.store.put_session(&own_code).unwrap();
        put_origin(&rig, "se_own_code", "se_parent", pb::ForkKind::CodeAndContext);
        // A session that was never forked.
        let plain = in_folder("se_plain", "/work/parent");
        rig.store.put_session(&plain).unwrap();

        for session in [copied, own_code, plain] {
            let caller = resolve_fork(agent_caller_of(&session, "ma_here"), &rig.store)
                .unwrap();
            assert_eq!(caller.session_kind(), SessionKind::Ordinary, "{}", session.id);
        }
    }
}

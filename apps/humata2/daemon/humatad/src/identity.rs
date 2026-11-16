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
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use humata2_access::{ChannelId, MachineId, SessionId, SessionRef};
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

/// Who made a call, as its credential established it.
#[derive(Clone, Debug)]
pub(crate) enum Caller {
    User(UserVia),
    Agent(AgentCaller),
    /// A script with a token (SEC-14). Never the user, and it has no
    /// session.
    Script(ScriptCaller),
    /// The Mac, over the peer link it started (`serve --stdio`). Bound to
    /// the link, never to anything a request carries (GRPC.md §3).
    Peer,
}

/// How the user reached the daemon.
#[derive(Clone, Debug)]
pub(crate) enum UserVia {
    /// The app. Only an admin test daemon finds it today: it takes a call
    /// with no credential for the user.
    #[cfg_attr(not(feature = "admin"), allow(dead_code))]
    App,
    /// A process in the terminal session that this `humata login` bound.
    CliLogin { login: String },
}

/// A session's agent, or anything else running in one of its tabs.
#[derive(Clone, Debug)]
pub(crate) struct AgentCaller {
    pub(crate) session: SessionRef,
    pub(crate) agent: String,
    pub(crate) title: String,
}

impl AgentCaller {
    pub(crate) fn of(record: &SessionRecord, here: &str) -> Self {
        Self {
            session: session_ref(record, here),
            agent: record.agent.clone(),
            title: record.title.clone(),
        }
    }

    pub(crate) fn actor(&self) -> pb::AgentActor {
        pb::AgentActor {
            session: Some(pb::SessionId {
                value: self.session.id.to_string(),
            }),
            machine: Some(pb::MachineId {
                value: self.session.machine.to_string(),
            }),
            agent: Some(pb::AgentId {
                value: self.agent.clone(),
            }),
            session_title: self.title.clone(),
        }
    }
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

/// The methods `OpenToAnyone` covers, as (service, method).
const OPEN_TO_ANYONE: [(&str, &str); 2] = [
    (pb::daemon_service_server::SERVICE_NAME, "WhoAmI"),
    (pb::cli_auth_service_server::SERVICE_NAME, "BeginCliSession"),
];

/// Marks `request` when its method is open to anyone. Runs before
/// `Authenticate`, which can't see the path. Extensions are the server's
/// own: a client can't set this marker.
pub(crate) fn mark_open(mut request: http::Request<Body>) -> http::Request<Body> {
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

/// Who `caller` is, as events and records name them. A user's CLI login
/// says nothing of the machine it's on here, so its `via` is unset.
pub(crate) fn actor(caller: &Caller) -> pb::Actor {
    use pb::actor::Actor;
    let actor = match caller {
        Caller::User(via) => Actor::User(pb::UserActor {
            via: match via {
                UserVia::App => pb::user_actor::Via::App,
                UserVia::CliLogin { .. } => pb::user_actor::Via::Unspecified,
            } as i32,
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

/// The process at the other end of the call's socket, when the kernel
/// said. Read when the connection was accepted.
pub(crate) fn peer_pid<T>(request: &Request<T>) -> Option<i32> {
    request
        .extensions()
        .get::<humata2_app_peer::PeerInfo>()
        .and_then(|info| info.uds.peer_cred)
        .and_then(|cred| cred.pid())
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
                            caller: AgentCaller::of(&record, machine),
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
                caller: AgentCaller::of(session, &self.machine),
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
    /// Admin mode: a call with no credential is the user. Test daemons
    /// only; release builds don't have the feature, so not the code.
    #[cfg(feature = "admin")]
    admin: Option<Admin>,
}

/// Whose calls with no credential an admin test daemon takes for the user.
#[cfg(feature = "admin")]
#[derive(Clone, Copy, Debug)]
pub(crate) enum Admin {
    /// Anyone's (`--admin`).
    Anyone,
    /// Only this process's (`--admin-pid`), so a test can play the app
    /// while every other process is judged as by a release daemon.
    Process(i32),
}

impl Authenticate {
    pub(crate) fn new(
        identities: Arc<Identities>,
        logins: Arc<CliLogins>,
        scripts: Arc<ScriptTokens>,
        store: Arc<Store>,
        processes: Arc<dyn Processes>,
    ) -> Self {
        Self {
            identities,
            logins,
            scripts,
            store,
            processes,
            #[cfg(feature = "admin")]
            admin: None,
        }
    }

    /// Takes a call with no credential from `who` for the user, as a test
    /// daemon run with `--admin` or `--admin-pid` does.
    #[cfg(feature = "admin")]
    pub(crate) fn admin(
        identities: Arc<Identities>,
        logins: Arc<CliLogins>,
        scripts: Arc<ScriptTokens>,
        store: Arc<Store>,
        processes: Arc<dyn Processes>,
        who: Admin,
    ) -> Self {
        Self {
            identities,
            logins,
            scripts,
            store,
            processes,
            admin: Some(who),
        }
    }

    /// Who a call with no credential from `pid` is, if anyone. `app` says
    /// the peer passed the signed app's code-signing check (D135); even
    /// then a process inside a session's terminal is not the user, so an
    /// agent can't borrow the app's binary to act as the user.
    fn without_credential(&self, pid: Option<i32>, app: bool) -> Option<Caller> {
        #[cfg(feature = "admin")]
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
    fn in_a_session(&self, pid: Option<i32>) -> bool {
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
    /// Whether the process at the other end may use `holder`'s token: it
    /// must be in the process tree of the holder of the tab the token was
    /// minted for (SEC-15). Fails closed: no pid, no holder, a tab that is
    /// gone, or a process that can't be read all refuse. A misused token
    /// is revoked, so the session gets a new one when it next resumes.
    fn token_is_in_its_tree(&self, holder: &Holder, pid: Option<i32>) -> Result<(), Status> {
        // An `--admin` test daemon lets the test process stand in for any
        // tab's shell. `--admin-pid` and release daemons enforce.
        #[cfg(feature = "admin")]
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
        let open = request.extensions().get::<OpenToAnyone>().is_some();
        let session = request.metadata().get(TOKEN_HEADER);
        let login = request.metadata().get(LOGIN_HEADER);
        let script = request.metadata().get(SCRIPT_HEADER);
        let caller = match (session, script, login) {
            (Some(value), _, _) => match value
                .to_str()
                .ok()
                .and_then(|token| self.identities.find(token))
            {
                Some(holder) => {
                    self.token_is_in_its_tree(&holder, pid)?;
                    match holder.identity {
                        TabIdentity::Session => Some(Caller::Agent(holder.caller)),
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
                    Ok(login) => Some(Caller::User(UserVia::CliLogin { login })),
                    Err(_) if open => None,
                    Err(unusable) => return Err(status::login_unusable(&unusable)),
                }
            }
            (None, None, None) => self.without_credential(pid, app),
        };
        match caller {
            Some(caller) => {
                request.extensions_mut().insert(caller);
            }
            None if open => {
                request.extensions_mut().insert(Unauthenticated);
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
    use crate::terminal_session::tests::Fake;

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
            })
            .unwrap();

        let reloaded = Identities::load(rig.file.clone(), &rig.store, "ma_here").unwrap();

        assert_eq!(
            reloaded.find(&token).map(|holder| holder.identity),
            Some(TabIdentity::AddedAgent)
        );
    }
}

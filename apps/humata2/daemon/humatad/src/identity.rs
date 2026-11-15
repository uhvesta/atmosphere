//! Who a connection is (BOUNDARIES.md §2, MCP.md §1). Each session tab's
//! shell gets a session token of its own in `HUMATA_SESSION_TOKEN`; a call
//! that carries one is that session's agent. A call that carries a CLI
//! login (`cli_login`) from the terminal session it bound is the user. A
//! call with no credential is refused, except WhoAmI and logging in, which
//! answer it as unauthenticated; a test daemon built with the `admin`
//! feature and run with `--admin` takes it for the user instead. Only
//! hashes of the session tokens are kept, in a 0600 file in the state, so
//! tabs that outlive a daemon restart keep theirs.

use std::collections::HashMap;
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
use tonic::transport::server::UdsConnectInfo;
use tonic::{Request, Status};

use crate::cli_login::CliLogins;
use crate::error::DaemonError;
use crate::status;

/// The gRPC metadata key a session token travels in.
pub(crate) const TOKEN_HEADER: &str = "humata-session-token";

/// The gRPC metadata key a CLI login's token travels in.
pub(crate) const LOGIN_HEADER: &str = "humata-cli-login";

/// A token's random bytes: 32 hex characters.
const TOKEN_BYTES: usize = 16;

/// Who made a call, as its credential established it.
#[derive(Clone, Debug)]
pub(crate) enum Caller {
    User(UserVia),
    Agent(AgentCaller),
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
        .get::<UdsConnectInfo>()
        .and_then(|info| info.peer_cred)
        .and_then(|cred| cred.pid())
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

    fn find(&self, token: &str) -> Option<(AgentCaller, TabIdentity)> {
        lock(&self.tokens)
            .get(&digest(token))
            .map(|holder| (holder.caller.clone(), holder.identity))
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
/// a CLI login. The token itself is never logged.
#[derive(Clone)]
pub(crate) struct Authenticate {
    identities: Arc<Identities>,
    logins: Arc<CliLogins>,
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
    pub(crate) fn new(identities: Arc<Identities>, logins: Arc<CliLogins>) -> Self {
        Self {
            identities,
            logins,
            #[cfg(feature = "admin")]
            admin: None,
        }
    }

    /// Takes a call with no credential from `who` for the user, as a test
    /// daemon run with `--admin` or `--admin-pid` does.
    #[cfg(feature = "admin")]
    pub(crate) fn admin(identities: Arc<Identities>, logins: Arc<CliLogins>, who: Admin) -> Self {
        Self {
            identities,
            logins,
            admin: Some(who),
        }
    }

    /// Who a call with no credential from `pid` is, if anyone.
    #[cfg_attr(not(feature = "admin"), allow(unused_variables))]
    fn without_credential(&self, pid: Option<i32>) -> Option<Caller> {
        #[cfg(feature = "admin")]
        match self.admin {
            Some(Admin::Anyone) => return Some(Caller::User(UserVia::App)),
            Some(Admin::Process(admin)) if pid == Some(admin) => {
                return Some(Caller::User(UserVia::App));
            }
            Some(Admin::Process(_)) | None => {}
        }
        None
    }
}

impl Interceptor for Authenticate {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        let open = request.extensions().get::<OpenToAnyone>().is_some();
        let session = request.metadata().get(TOKEN_HEADER);
        let login = request.metadata().get(LOGIN_HEADER);
        let caller = match (session, login) {
            (Some(value), _) => match value
                .to_str()
                .ok()
                .and_then(|token| self.identities.find(token))
            {
                Some((agent, TabIdentity::Session)) => Some(Caller::Agent(agent)),
                // An added agent may ask who it is; nothing else. It
                // answers as a caller with no credential, so it learns
                // why its other calls are refused.
                Some((_, TabIdentity::AddedAgent))
                    if request.extensions().get::<AddedAgentMayCall>().is_some() =>
                {
                    None
                }
                Some((agent, TabIdentity::AddedAgent)) => {
                    return Err(added_agent_refused(agent.session.id.as_str()));
                }
                None => return Err(status::identity_mismatch()),
            },
            // Judged strictly, even on an admin daemon: a login that
            // doesn't work here is never taken for the user. On a method
            // open to anyone it counts as no credential, so a terminal with
            // a stale login can still log in again.
            (None, Some(value)) => {
                let checked = match value.to_str() {
                    Ok(token) => self.logins.caller(token, peer_pid(&request)),
                    Err(_) => Err(crate::cli_login::LoginUnusable::Unknown),
                };
                match checked {
                    Ok(login) => Some(Caller::User(UserVia::CliLogin { login })),
                    Err(_) if open => None,
                    Err(unusable) => return Err(status::login_unusable(&unusable)),
                }
            }
            (None, None) => self.without_credential(peer_pid(&request)),
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

    struct Rig {
        _dir: tempfile::TempDir,
        file: PathBuf,
        store: Store,
        identities: Arc<Identities>,
        authenticate: Authenticate,
    }

    fn rig() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("store")).unwrap();
        let file = dir.path().join("tokens");
        let identities = Arc::new(Identities::load(file.clone(), &store, "ma_here").unwrap());
        let logins = Arc::new(CliLogins::new(Box::new(Fake::default()), None));
        let authenticate = Authenticate::new(identities.clone(), logins);
        Rig {
            _dir: dir,
            file,
            store,
            identities,
            authenticate,
        }
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
        let mut rig = rig();
        let token = rig
            .identities
            .mint_as("tab_added", &record(), TabIdentity::AddedAgent)
            .unwrap();

        let refused = rig.authenticate.call(call(&token, false)).unwrap_err();
        assert_eq!(refused.code(), tonic::Code::PermissionDenied);
        let error = status::error_of(&refused).unwrap();
        assert!(!error.fix.is_empty());

        let answered = rig.authenticate.call(call(&token, true)).unwrap();
        assert!(caller_if_any(&answered).unwrap().is_none());
    }

    #[test]
    fn a_session_token_is_that_sessions_agent() {
        let mut rig = rig();
        let token = rig.identities.mint("tab_main", &record()).unwrap();

        let accepted = rig.authenticate.call(call(&token, false)).unwrap();

        assert!(matches!(
            caller(&accepted).unwrap(),
            Caller::Agent(agent) if agent.session.id.as_str() == "se_main"
        ));
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
            reloaded.find(&token).map(|(_, kind)| kind),
            Some(TabIdentity::AddedAgent)
        );
    }
}

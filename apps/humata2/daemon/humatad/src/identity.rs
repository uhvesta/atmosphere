//! Who a connection is (BOUNDARIES.md §2, MCP.md §1). Each session tab's
//! shell gets a session token of its own in `HUMATA_SESSION_TOKEN`; a call
//! that carries one is that session's agent, and a call with none is the
//! user. Only hashes of the tokens are kept, in a 0600 file in the state,
//! so tabs that outlive a daemon restart keep theirs.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use humata2_access::{ChannelId, MachineId, SessionId, SessionRef};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{SessionRecord, Store, StoreError, write_atomic};
use sha2::{Digest, Sha256};
use tonic::service::Interceptor;
use tonic::{Request, Status};

use crate::error::DaemonError;
use crate::status;

/// The gRPC metadata key a session token travels in.
pub(crate) const TOKEN_HEADER: &str = "humata-session-token";

/// A token's random bytes: 32 hex characters.
const TOKEN_BYTES: usize = 16;

/// Who made a call, as the token established it.
#[derive(Clone, Debug)]
pub(crate) enum Caller {
    /// No token: the user. Until the process-tree check exists, an agent
    /// that drops its token is taken for the user too.
    User,
    Agent(AgentCaller),
}

/// A session's agent, or anything else running in one of its tabs.
#[derive(Clone, Debug)]
pub(crate) struct AgentCaller {
    pub(crate) session: SessionRef,
    pub(crate) agent: String,
    pub(crate) title: String,
}

impl AgentCaller {
    fn of(record: &SessionRecord, here: &str) -> Self {
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

/// The caller the interceptor found for `request`.
pub(crate) fn caller<T>(request: &Request<T>) -> Result<Caller, Status> {
    request.extensions().get::<Caller>().cloned().ok_or_else(|| {
        status::internal(
            "a call reached its handler without its caller being checked",
            "",
        )
    })
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
            let (Some(hash), Some(tab), Some(session)) = (fields.next(), fields.next(), fields.next())
            else {
                dropped = true;
                continue;
            };
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
        let mut bytes = [0u8; TOKEN_BYTES];
        rand::fill(&mut bytes[..]);
        let token = hex(&bytes);
        let hash = digest(&token);
        let mut tokens = lock(&self.tokens);
        tokens.insert(
            hash.clone(),
            Holder {
                tab: tab.to_owned(),
                caller: AgentCaller::of(session, &self.machine),
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

    fn find(&self, token: &str) -> Option<AgentCaller> {
        lock(&self.tokens)
            .get(&digest(token))
            .map(|holder| holder.caller.clone())
    }

    fn save(&self, tokens: &HashMap<String, Holder>) -> Result<(), StoreError> {
        let mut text = String::new();
        for (hash, holder) in tokens {
            let _ = writeln!(text, "{hash} {} {}", holder.tab, holder.caller.session.id);
        }
        write_atomic(&self.file, text.as_bytes())
    }
}

/// Finds each call's caller from its token and puts it in the request's
/// extensions. A token this daemon didn't give out is refused before any
/// handler runs. The token itself is never logged.
#[derive(Clone)]
pub(crate) struct Authenticate {
    identities: Arc<Identities>,
}

impl Authenticate {
    pub(crate) fn new(identities: Arc<Identities>) -> Self {
        Self { identities }
    }
}

impl Interceptor for Authenticate {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        let caller = match request.metadata().get(TOKEN_HEADER) {
            None => Caller::User,
            Some(value) => match value
                .to_str()
                .ok()
                .and_then(|token| self.identities.find(token))
            {
                Some(agent) => Caller::Agent(agent),
                None => return Err(status::identity_mismatch()),
            },
        };
        request.extensions_mut().insert(caller);
        Ok(request)
    }
}

fn digest(token: &str) -> String {
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

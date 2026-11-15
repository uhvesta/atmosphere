//! Who `humata` acts as (CLI.md §2, BOUNDARIES.md §2.2). No flag picks it:
//! it follows from the environment, and the daemon checks it against the
//! connection (peer uid, process tree, terminal session).

use std::path::PathBuf;

/// What `humata` reads from its environment, captured once so tests can
/// give their own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Environment {
    /// `HUMATA_SESSION_TOKEN`: set by the holder for an agent's processes.
    pub session_token: Option<String>,
    /// `HUMATA_TOKEN`: a script token from `humata token create`.
    pub script_token: Option<String>,
    /// `HUMATA_SOCKET`: the daemon's socket, instead of looking for it.
    pub socket: Option<PathBuf>,
    /// `HUMATA_ROOT`: this machine's Humata root, for its machine socket.
    pub root: Option<PathBuf>,
    pub home: Option<PathBuf>,
}

impl Environment {
    pub fn from_process() -> Self {
        let text = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
        let path = |name: &str| {
            std::env::var_os(name)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        };
        Self {
            session_token: text("HUMATA_SESSION_TOKEN"),
            script_token: text("HUMATA_TOKEN"),
            socket: path("HUMATA_SOCKET"),
            root: path("HUMATA_ROOT"),
            home: path("HOME"),
        }
    }
}

/// The principal a run asks to be. The daemon may still decide otherwise:
/// a process inside a session's tree is that session's agent with or
/// without a token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Principal {
    /// A session's agent (SEC-8): exactly what it can do through MCP.
    Agent { token: String },
    /// A script (SEC-14): only its token's capabilities and channels.
    Script { token: String },
    /// The person: unverified until `humata login` or a confirmation.
    User,
}

/// The session token wins over a script token: an agent that has a script
/// token in its environment is still the agent (provisional, see the
/// report's gaps).
pub fn principal(environment: &Environment) -> Principal {
    if let Some(token) = &environment.session_token {
        return Principal::Agent {
            token: token.clone(),
        };
    }
    if let Some(token) = &environment.script_token {
        return Principal::Script {
            token: token.clone(),
        };
    }
    Principal::User
}

/// The gRPC metadata that carries a principal's token. Provisional names:
/// GRPC.md doesn't say how a token travels.
pub(crate) const SESSION_TOKEN_HEADER: &str = "humata-session-token";
pub(crate) const SCRIPT_TOKEN_HEADER: &str = "humata-script-token";

impl Principal {
    pub(crate) fn header(&self) -> Option<(&'static str, &str)> {
        match self {
            Self::Agent { token } => Some((SESSION_TOKEN_HEADER, token)),
            Self::Script { token } => Some((SCRIPT_TOKEN_HEADER, token)),
            Self::User => None,
        }
    }
}

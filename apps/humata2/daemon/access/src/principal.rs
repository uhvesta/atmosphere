//! Who is calling (TRACE.md §2's actors) and where the check runs.

use std::collections::BTreeSet;

use crate::capability::Capability;
use crate::ids::{ChannelId, MachineId, SessionId, SessionRef};

/// The caller, as the daemon established it from the socket, the session
/// token, the CLI login or the script token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Principal {
    /// The user in the app.
    App,
    /// The CLI outside any session tree.
    Cli(Presence),
    /// A session's agent, over MCP or through the CLI inside its session
    /// tree (SEC-8). A person typing in a Humata-held terminal is this too:
    /// the daemon can't tell them apart.
    Agent(Agent),
    /// A script with a token (SEC-14).
    Script(ScriptToken),
    /// Another daemon over the peer link.
    Peer(PeerLink),
}

/// Whether a CLI caller outside a session tree has shown the user is there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    Unverified,
    Present(PresenceMethod),
}

/// How the user showed they are there (`PresenceRequired.Method`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PresenceMethod {
    ConfirmInApp,
    CliLogin,
    ApprovalPassphrase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    pub session: SessionRef,
    pub kind: SessionKind,
}

/// What kind of session the agent runs in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionKind {
    Ordinary,
    /// A fork that reads its parent's clones but never writes them (IN-20).
    ContextOnlyFork { parent: SessionId },
    /// A PR review session. Its pushes follow PR-7 and PR-8, not the push
    /// rule: refused unless the user let the agent push to the PR branch.
    Review { push_allowed: bool },
}

/// What a script token lets its holder do (SEC-14).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScriptToken {
    pub capabilities: BTreeSet<Capability>,
    pub channels: BTreeSet<ChannelId>,
}

/// Which way a peer call goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerLink {
    /// The Mac calling a machine it enrolled.
    HostToDaemon,
    /// A machine reporting to the Mac.
    DaemonToHost { machine: MachineId },
}

/// Where a check runs and whether the Mac can be reached from there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    /// The machine whose daemon is checking.
    pub here: MachineId,
    pub host: Host,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    /// This daemon is the Mac's.
    Here,
    /// A machine whose link to the Mac is up.
    Reachable,
    /// A machine whose Mac is away; it judges calls against its last copy
    /// of the policy (AW-8).
    Away,
}

impl Context {
    pub fn on_host(here: MachineId) -> Self {
        Self {
            here,
            host: Host::Here,
        }
    }

    pub fn machine(here: MachineId, host_reachable: bool) -> Self {
        Self {
            here,
            host: if host_reachable {
                Host::Reachable
            } else {
                Host::Away
            },
        }
    }

    pub fn host_away(&self) -> bool {
        self.host == Host::Away
    }
}

/// Proof, for changing the policy, that the user is there. Only
/// [`Principal::present_user`] makes one, so nothing an agent or a script
/// holds can change rules or grants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentUser {
    method: Option<PresenceMethod>,
    on_host: bool,
    machine: MachineId,
}

impl PresentUser {
    /// How presence was shown; `None` for the app.
    pub fn method(&self) -> Option<PresenceMethod> {
        self.method
    }

    /// True when the user is at the Mac's daemon, where rules live.
    pub fn on_host(&self) -> bool {
        self.on_host
    }

    /// The machine the user answered on.
    pub fn machine(&self) -> &MachineId {
        &self.machine
    }
}

impl Principal {
    /// The user, when this caller is the user and has shown they are there.
    pub fn present_user(&self, context: &Context) -> Option<PresentUser> {
        let method = match self {
            Principal::App => None,
            Principal::Cli(Presence::Present(method)) => Some(*method),
            Principal::Cli(Presence::Unverified)
            | Principal::Agent(_)
            | Principal::Script(_)
            | Principal::Peer(_) => return None,
        };
        Some(PresentUser {
            method,
            on_host: context.host == Host::Here,
            machine: context.here.clone(),
        })
    }
}

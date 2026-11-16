//! Identifiers the policy compares. The daemon builds them from the proto
//! ids; the policy never parses or invents them, except grant ids.

use std::fmt;

macro_rules! string_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_id!(
    /// A session (`SessionId`).
    SessionId
);
string_id!(
    /// A machine (`MachineId`). The Mac is one too.
    MachineId
);
string_id!(
    /// A channel (`ChannelId`). Rules are kept per channel.
    ChannelId
);
string_id!(
    /// An agent kind a session runs (`AgentId`), such as Claude Code.
    AgentId
);
string_id!(
    /// A repository (`RepoRef`).
    RepoRef
);
string_id!(
    /// An Inbox request (`RequestId`). The daemon assigns it when it stores
    /// the request the policy asked for.
    RequestId
);

/// A grant made by "Always allow …" (`GrantId`). The policy assigns these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GrantId(u64);

impl GrantId {
    pub fn new(value: u64) -> Self {
        Self(value)
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for GrantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "grant {}", self.0)
    }
}

/// A session with the two facts the policy needs about it: the channel
/// whose rules govern it and the machine it runs on.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionRef {
    pub id: SessionId,
    pub channel: ChannelId,
    pub machine: MachineId,
}

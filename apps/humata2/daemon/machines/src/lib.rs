//! Humata 2's machine providers (MP-1 to MP-22): scripted `create`,
//! `connect` and `identity` commands, the one link every step runs through,
//! setting a machine up (create, identity, connect, check, install humatad2,
//! user service), taking over an interactive login, and the built-in Lima
//! provider that tests use. Registering a machine (hosts.json) is the
//! daemon's; this crate hands it a ready link.

mod check;
mod error;
mod identity;
mod install;
mod link;
mod name;
mod platform;
mod provider;
mod quote;
mod runner;
mod service;
mod setup;
mod system;
mod takeover;

pub use check::Facts;
pub use error::{
    ConfigError, FieldProblem, IdentityError, NameError, Problem, ProblemKind, Problems,
    QuoteError, Remedy, RunError, SetupError, TakeoverError, ValuesError,
};
pub use identity::{parse_identity, HostKey, Identity, SshAccess, SshTarget};
pub use install::{Builds, InstallProblem, InstalledFile, Payload};
pub use link::Link;
pub use name::MachineName;
pub use platform::{Arch, Os, Platform};
pub use provider::{parse_status, Action, MachineStatus, Param, Provider, Reach};
pub use runner::{EnvValue, Exit, Invocation, Output, Purpose, Runner};
pub use service::ServiceKind;
pub use setup::{Failure, LoginNeeded, LoginReason, Progress, Ready, RetryPolicy, Setup, Step};
pub use system::{HostVariables, SystemRunner};
pub use takeover::{Takeover, TakeoverAction};

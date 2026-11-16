//! Humata 2's capability checks.
//!
//! Given who is calling ([`Principal`]), what it wants to do ([`Call`]) and
//! where the check runs ([`Context`]), [`Policy::check`] says whether the
//! call is allowed, becomes an Inbox request, waits for the Mac, or is
//! refused with a reason and a fix ([`Verdict`]). The policy holds each
//! channel's rules and While I'm away rule, and the grants "Always allow …"
//! made (BOUNDARIES.md §3 and §4, TRACE.md §2).
//!
//! This is policy only: no I/O, no networking, no async. The daemon turns
//! proto messages into these types, stores requests and the policy state,
//! and turns verdicts into replies.

mod call;
mod capability;
mod error;
mod ids;
mod path;
mod policy;
mod principal;
mod verdict;

pub use call::{
    BoardAct, BoardRef, Call, Entry, GrantTarget, MachineAction, MachineActionKind,
    NetworkTarget, PathScope, Place, RepoBranch, Root, Target,
};
pub use capability::{AwayRule, Capability, ChannelRules, Decision, Tier};
pub use error::{AnswerError, CheckError, PolicyError};
pub use ids::{AgentId, ChannelId, GrantId, MachineId, RepoRef, RequestId, SessionId, SessionRef};
pub use path::{RepoPath, RepoPathError};
pub use policy::{Answer, Approval, Grant, Policy, PolicyState, Rules, RulesState};
pub use principal::{
    Agent, Context, Host, PeerLink, PresenceMethod, PresentUser, Principal, ScriptToken,
    SessionKind,
};
pub use verdict::{
    AccessRequest, AnswerAt, DeniedSource, Effect, ErrorKind, Fix, Refusal, Source, Verdict,
};

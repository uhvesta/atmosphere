//! A channel's folders and clones on this machine, and the steps that start
//! a session in them: folder and branch names (CH-2, FC-11..FC-14), the
//! folder layout (FC-1), main and session clones (FC-2, FC-3) and the agents
//! a session can run (PER-12). Knows nothing about gRPC.

// REPOS (CH-21): a repo added to a running session.
pub mod add_repo;
pub mod agents;
pub mod continue_in;
pub mod error;
pub mod first_message;
pub mod fork;
pub mod git;
pub mod layout;
pub mod names;
pub mod own_copy;
pub mod prefix;
pub mod push;
mod recover;
pub mod rename;
pub mod restore;
pub mod setup;
pub mod start;
pub mod tree_copy;
#[cfg(test)]
mod versions_tests;
pub mod work;

pub use add_repo::AddedRepo;
pub use agents::{
    AgentKind, Agents, ConversationFormat, FollowError, HumataTools, LaunchPlan, TurnEnd,
    TurnsError,
};
pub use continue_in::ContinueRequest;
pub use error::{AgentError, GitError, LayoutError, NameError, SetupError, StartError};
pub use first_message::{
    Boxed, FirstMessage, GateRefusal, HeldMessages, Resend, StartSignal, WriteGate,
};
pub use fork::{ForkCopy, ForkPlan, ForkRequest, handover_file};
pub use git::{
    DEFAULT_REMOTE_TEMPLATE, clone_main, clone_main_with, clone_session, clone_session_with,
    default_branch, local_branches, remote_branches, remote_url,
};
pub use layout::{Layout, partial};
pub use names::{
    Branch, CHANNELS_ROOT, DESK_CHANNEL, branch_slug, channel_folder, pick_branch,
    valid_branch_name, valid_prefix,
};
pub use own_copy::OwnCopyError;
pub use prefix::{BRANCH_PREFIX_KEY, default_prefix, effective_prefix, set_default_prefix};
pub use push::{PushError, Totals, commit_all, push_branch, totals};
pub use recover::{Leftovers, Recovery, recover};
pub use rename::BranchRename;
pub use restore::{RestoredFrom, restore_clone};
pub use setup::{NewChannel, clone_mains, clone_mains_with, create_channel};
pub use start::{
    CloneUnits, RepoName, ReviewStart, StartRequest, Started, Starter, with_clone_units,
};
pub use work::{FileChange, RepoWork, repo_work};

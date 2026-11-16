//! A channel's folders and clones on this machine, and the steps that start
//! a session in them: folder and branch names (CH-2, FC-11..FC-14), the
//! folder layout (FC-1), main and session clones (FC-2, FC-3) and the agents
//! a session can run (PER-12). Knows nothing about gRPC.

mod agents;
mod error;
mod fork;
mod git;
mod layout;
mod names;
mod recover;
mod restore;
mod setup;
mod start;
mod work;

pub use agents::{AgentKind, Agents, LaunchPlan, TurnEnd};
pub use error::{AgentError, GitError, LayoutError, NameError, SetupError, StartError};
pub use git::{
    DEFAULT_REMOTE_TEMPLATE, clone_main, clone_session, default_branch, local_branches,
    remote_branches, remote_url,
};
pub use layout::{Layout, partial};
pub use names::{Branch, branch_slug, channel_folder, pick_branch};
pub use recover::{Leftovers, Recovery, recover};
pub use setup::{NewChannel, clone_mains, create_channel};
pub use start::{RepoName, StartRequest, Started, Starter};
pub use fork::{ForkCopy, ForkPlan, ForkRequest};
pub use restore::{RestoredFrom, restore_clone};
pub use work::{FileChange, RepoWork, repo_work};

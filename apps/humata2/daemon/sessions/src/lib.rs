//! A channel's folders and clones on this machine, and the steps that start
//! a session in them: folder and branch names (CH-2, FC-11..FC-14), the
//! folder layout (FC-1), main and session clones (FC-2, FC-3) and the agents
//! a session can run (PER-12). Knows nothing about gRPC.

// REPOS (CH-21): a repo added to a running session.
mod add_repo;
mod agents;
mod bring;
mod carry;
mod channel_desk;
mod continue_in;
mod desk;
mod desk_move;
pub mod doc_repo;
mod error;
mod first_message;
mod fork;
mod git;
mod layout;
mod names;
mod prefix;
mod rename;
mod own_copy;
mod push;
mod recover;
mod restore;
mod setup;
mod start;
mod tree_copy;
#[cfg(test)]
mod versions_tests;
mod work;

pub use agents::{AgentKind, Agents, ConversationFormat, HumataTools, LaunchPlan, TurnEnd};
pub use bring::{Brought, check_brought};
pub use carry::{
    CARRY_DIR, CHANNELS_ROOT, CHUNK_SIZE, CHUNKS_DIR, CHUNKS_FILE, CLONES_FILE, CarryError, Discard, ForkPack,
    ForkPackError, ForkPacked, HAVE_DIR, Landing, Packed, REUSE_FILE, Unpacked,
};
pub use channel_desk::ChannelDeskStart;
pub use error::{AgentError, GitError, LayoutError, NameError, SetupError, StartError};
pub use desk::{
    DESK_CHANNEL, DeskError, DeskFolder, DeskStart, check_repo_name, clone_repo, clone_repos,
    copy_document, document_lines_under, documents_under, folder_bytes, held_work, repo_clone,
    repos_under, scratch_child,
};
pub use desk_move::{DeskMoveError, DeskMoveRepo, desk_move_repos, repoint_clone};
pub use git::{
    DEFAULT_REMOTE_TEMPLATE, clone_main, clone_main_with, clone_session, clone_session_with, default_branch, local_branches,
    remote_branches, remote_url,
};
pub use layout::{Layout, partial};
pub use names::{Branch, branch_slug, channel_folder, pick_branch, valid_branch_name, valid_prefix};
pub use prefix::{BRANCH_PREFIX_KEY, default_prefix, effective_prefix, set_default_prefix};
pub use rename::BranchRename;
pub use recover::{Leftovers, Recovery, recover};
pub use setup::{NewChannel, clone_mains, clone_mains_with, create_channel};
pub use start::{CloneUnits, RepoName, with_clone_units, ReviewStart, StartRequest, Started, Starter};
pub use first_message::{Boxed, FirstMessage, GateRefusal, HeldMessages, Resend, StartSignal, WriteGate};
pub use fork::{ForkCopy, ForkPlan, ForkRequest, handover_file};
pub use continue_in::ContinueRequest;
pub use agents::{FollowError, TurnsError};
pub use restore::{RestoredFrom, restore_clone};
pub use work::{FileChange, RepoWork, repo_work};
pub use own_copy::OwnCopyError;
pub use push::{PushError, Totals, commit_all, push_branch, totals};
pub use desk::clone_work;

//! The app's real daemon client: [`LiveDaemon`] implements the app's
//! `Daemon` and `Terminals` traits over gRPC to a `humatad2` on its Unix
//! socket.

mod agent_switches;
mod boards;
// REPOS (CH-6): Add a GitHub repo to a channel.
mod channel_repos;
mod comments;
mod connection;
mod daemon;
mod desk;
mod docs;
mod diff;
mod events;
mod file_manager;
pub mod history;
mod inbox;
mod link;
mod local_time;
mod machines;
pub mod marks;
mod map;
mod message;
mod pull_requests;
mod review_chat;
mod read_marks;
mod recorded;
// REPOS (RL-1): the repo library and search over RepoService.
mod repos;
// REPOS (CH-21)
mod session_repos;
mod settings;
mod setup;
mod terminals;
mod views;

pub use comments::LiveComments;
pub use connection::{ConnectError, DaemonFacts, LiveDaemon, LiveOptions};
pub use file_manager::LiveFileManager;
pub use machines::LiveMachines;
pub use pull_requests::LivePullRequests;
pub use review_chat::LiveReviewChat;
pub use views::LiveViews;
pub use recorded::{Call, Observer};

//! The app's real daemon client: [`LiveDaemon`] implements the app's
//! `Daemon` and `Terminals` traits over gRPC to a `humatad2` on its Unix
//! socket.

mod comments;
mod connection;
mod daemon;
mod desk;
mod events;
mod file_manager;
mod inbox;
mod machines;
mod map;
mod recorded;
mod setup;
mod terminals;

pub use comments::LiveComments;
pub use connection::{ConnectError, LiveDaemon, LiveOptions};
pub use file_manager::LiveFileManager;
pub use machines::LiveMachines;
pub use recorded::{Call, Observer};

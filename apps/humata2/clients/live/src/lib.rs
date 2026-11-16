//! The app's real daemon client: [`LiveDaemon`] implements the app's
//! `Daemon` and `Terminals` traits over gRPC to a `humatad2` on its Unix
//! socket.

mod connection;
mod daemon;
mod desk;
mod events;
mod map;
mod setup;
mod terminals;

pub use connection::{ConnectError, LiveDaemon, LiveOptions};

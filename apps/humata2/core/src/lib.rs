//! Humata 2's features, on tca-rust: each one's state, actions and reducer,
//! and what they ask of `humatad`. Swift lays the state out and sends the
//! actions through what the `ffi` feature exports.

mod daemon;
#[cfg(feature = "ffi")]
mod ffi;
mod sessions;

pub use daemon::{Daemon, DaemonError, LiveDaemon, Session, SessionState};
pub use sessions::{Sessions, SessionsAction, SessionsFeature};

#[cfg(feature = "ffi")]
uniffi::setup_scaffolding!("humata2");

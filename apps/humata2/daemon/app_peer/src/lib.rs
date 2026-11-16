//! Recognises the signed Humata app on the daemon's socket (D135).
//!
//! The daemon reads the connecting process's audit token (not its pid, which
//! races with exec and pid reuse) and asks Security.framework whether that
//! process satisfies a requirement compiled into the daemon.

mod stream;
mod sys;

pub use stream::{incoming, CheckedStream, PeerInfo};
pub use sys::{check, AuditToken, NotApp, Requirement};

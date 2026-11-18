//! The daemon's check of an agent tab's screen before anything is typed
//! into it (D762), as the crates that type without a screen of their own
//! ask it: a resume's interrupted note here, a start's first message in
//! the sessions crate (D763).

use std::future::Future;
use std::path::Path;
use std::pin::Pin;

/// Why an agent tab isn't clear to type into now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateRefusal {
    /// The agent asks a question of its own on its screen: it waits for
    /// the person to answer it.
    Question,
    /// Its screen couldn't be read.
    Unreadable,
}

/// Whether an agent tab is clear to type into.
pub trait WriteGate: std::fmt::Debug + Send + Sync {
    /// Whether the agent tab whose holder is at `socket` is clear to type
    /// into now.
    fn guard(&self, socket: &Path)
    -> Pin<Box<dyn Future<Output = Result<(), GateRefusal>> + Send + 'static>>;

    /// Whether the agent's input at `socket` still shows `text`, typed and
    /// returned but not taken (D763 amendment). `false` when it can't
    /// tell, so nothing is pressed again.
    fn unsent(&self, _socket: &Path, _text: &str) -> Pin<Box<dyn Future<Output = bool> + Send + 'static>> {
        Box::pin(async { false })
    }
}

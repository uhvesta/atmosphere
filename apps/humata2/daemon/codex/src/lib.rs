//! Humata 2's link to Codex's own app-server (D795): reads its JSON-RPC
//! messages, writes the answers Codex waits for, and keeps the websocket
//! link that reports turns and the conversation to the daemon and passes
//! asks to the Inbox.

mod event;
mod link;
mod wrapper;

pub use event::{AskKind, Event, FrameError, RequestId, answer_frame, read_frame};
pub use link::{Humata, HumataError, Turn, run_link};
pub use wrapper::{Wrapper, WrapperError, run_wrapper};

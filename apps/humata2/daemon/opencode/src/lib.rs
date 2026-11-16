//! OpenCode's loopback server as the `humata opencode` wrapper uses it (D201, D201a).
//!
//! - [`Frames`] splits the server's event stream into the data of each frame,
//!   and [`Event::parse`] reads one frame.
//! - [`Activity`] turns the events into the agent's [`Turn`].
//! - [`run_link`] is the wrapper's link: it reports turns and passes each ask
//!   to the daemon (through [`Humata`]) and its answer back to the server
//!   (through [`Server`]). [`Http`] is the real [`Server`].
//! - [`listening_url`] reads the URL `serve` prints, [`supported`] checks the
//!   version, [`session_id`] names the session and [`reply_path`] says where
//!   replies to asks go.
//! - [`run_wrapper`] is the `humata opencode` wrapper: it starts the server,
//!   attaches the TUI and runs the link.
//! - [`StateFile`] is the 0600 file the wrapper leaves for the daemon.
//!
//! The recorded fixtures come from OpenCode 2.3.0, the oldest version Humata
//! reads.

mod activity;
mod event;
mod http;
mod link;
mod serve;
mod sse;
mod state;
mod wrapper;

pub use activity::{Activity, Turn};
pub use event::{Asked, Event, EventError};
pub use http::Http;
pub use link::{Humata, HumataError, Server, ServerError, run_link};
pub use serve::{VersionError, listening_url, reply_path, session_id, supported};
pub use sse::Frames;
pub use state::{StateError, StateFile};
pub use wrapper::{Wrapper, WrapperError, run_wrapper};

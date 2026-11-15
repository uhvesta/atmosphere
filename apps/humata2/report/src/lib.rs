//! Humata 2's bug reports (I9), without a GUI.
//!
//! A report is text an agent can read: a log of events and enough context
//! to see what happened and where, with the UI as its underlying data (the
//! state, the view tree and the layout tree), never pictures.
//!
//! 1. The app takes a [`snapshot`] of one moment, or starts a
//!    [`Recording`] and pushes each interaction, reducer action, view and
//!    layout tree, daemon call, daemon error and log line as it happens,
//!    then stops it. Either gives a [`Captured`].
//! 2. The person fills in a [`Form`]: what happened, what they expected,
//!    and comments pinned to elements.
//! 3. [`scrub`] takes out everything the app seeded in [`Secrets`]
//!    (tokens, host variable values, paste-folder contents, terminal text
//!    not marked to keep), giving a [`Scrubbed`] report. Only a scrubbed
//!    report can be written.
//! 4. [`write`] saves it as `reports/<date>-<slug>/` with `report.md`,
//!    `events.jsonl`, `state.txt`, `view.txt`, `layout.txt` and
//!    `daemon.log`, all readable only by this user.
//! 5. [`copy_prompt`] gives the text "Copy report" puts on the clipboard.
//!
//! [`read_events`] reads a saved `events.jsonl` back, and
//! [`Captured::flow_script`] turns a recording into a flow test (I4).

mod change;
mod error;
mod event;
mod form;
mod jsonl;
mod prompt;
mod recording;
mod replay;
mod scrub;
mod write;

pub use change::Hunk;
pub use error::{PromptError, ReadError, ScrubError, WriteError};
pub use event::{
    ActionStep, Button, Context, Daemon, DaemonError, Element, Event, Gesture, Holds, Interaction,
    Kind, LogLine, Machine, Node, Pasted, Rpc, Session, TreeChange, Trees, WindowSize,
};
pub use form::{Comment, Form};
pub use jsonl::read_events;
pub use prompt::copy_prompt;
pub use recording::{Captured, Limits, Recording, snapshot};
pub use replay::{FlowAction, FlowScript, FlowStep, FlowTarget, SkipReason, Skipped};
pub use scrub::{Scrubbed, Secrets, scrub};
pub use write::write;

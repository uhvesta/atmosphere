//! Humata 2's bug reports (I9), without a GUI.
//!
//! A report is text an agent can read: a log of events and enough context
//! to see what happened and where, with the UI as its underlying data (the
//! state, the view tree and the layout tree), plus a picture of the window
//! the app drew itself (`window.png`, with `marks.svg` over it) when it
//! could.
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
//!    `daemon.log`, and `drawing.json` when marks were drawn, all readable only by this user.
//!    A form filled in after the report was saved goes in with
//!    [`fill_form`], so a report can be saved the moment it stops.
//! 5. [`copy_prompt`] gives the text "Copy report" puts on the clipboard.
//!
//! A [`Recorder`] holds the one recording running at a time, so the menu,
//! the shortcut and `humata report record|stop` agree on it. The [`remote`]
//! socket is how `humata report record|stop|snapshot` reaches the running app.
//!
//! [`read_events`] reads a saved `events.jsonl` back,
//! [`Captured::flow_script`] turns a recording into a flow test (I4), and
//! [`FlowScript::replay`] plays it on a [`Player`] wired to the
//! [`FakeDaemon`] the recording gives, checking that it ends where the
//! recording ended.

mod change;
mod error;
mod event;
mod fake;
mod form;
mod jsonl;
mod picture;
mod prompt;
mod recorder;
mod recording;
mod replay;
mod scrub;
mod write;

pub mod remote;
pub mod video;

pub use change::Hunk;
pub use error::{
    FakeDaemonError, FormError, PictureError, PromptError, ReadError, RecorderError, ReplayError, ScrubError,
    WriteError,
};
pub use event::{
    ActionStep, Button, Context, Daemon, DaemonError, Element, Event, Gesture, Holds, Interaction,
    Kind, LogLine, Machine, Node, Pasted, Rpc, Session, TreeChange, Trees, WindowSize,
};
pub use fake::FakeDaemon;
pub use form::{Bounds, Comment, Drawing, Form, Point, Shape, Size, Stroke};
pub use jsonl::read_events;
pub use picture::{PixelBox, mask, write_window};
pub use prompt::copy_prompt;
pub use recorder::Recorder;
pub use recording::{Captured, Limits, Mark, Recording, snapshot};
pub use replay::{
    Compare, FlowAction, FlowScript, FlowStep, FlowTarget, Player, SkipReason, Skipped,
};
pub use scrub::{Scrubbed, Secrets, scrub};
pub use write::{fill_form, write};

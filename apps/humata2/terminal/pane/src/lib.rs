//! Humata 2's terminal pane: the grid view over a `ghostty_vt` terminal,
//! with what an embedded terminal needs around it (SPEC SE-8 to SE-16).
//!
//! - [`TerminalPane`]: keys, mouse and pastes in the encodings the program
//!   asked for, written to a [`Pty`]; the find bar over the whole history;
//!   the multi-line paste prompt; the offline banner (SE-16). It is a
//!   self-contained pane content (NV-28): any size, any number at once,
//!   hidden with [`TerminalPane::set_visible`] (NV-34), and ⌘\ asks the
//!   pane tree for a split with a new shell ([`PaneEvent::SplitRequested`]).
//! - [`TerminalTabs`]: a tab strip, for a pane that still wants tabs.
//!   Split panes are the main way to see several terminals (D74). Closing
//!   a tab closes its view; the process keeps running until stopped.
//! - [`Script`]: deterministic terminal output for tests and benches.
//!
//! The Wrap setting is not read here: a terminal's lines are as wide as
//! the pane, and the program reflows on resize (ST-12).

mod error;
mod find;
mod host;
mod pane;
mod script;
mod tabs;

pub use error::{FindError, PaneError};
pub use find::Query;
pub use host::{PaneEvent, PendingPaste, Pty};
pub use pane::{FindStatus, TerminalPane};
pub use script::Script;
pub use tabs::{TabId, TabSpec, TabStrip, TabsEvent, TerminalTabs};
pub use terminal_view::is_web_or_mail_uri;

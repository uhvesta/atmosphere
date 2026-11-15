//! Snapshots for Humata 2's tests: what a test produced, saved beside it in
//! `__snapshots__/` and compared on every later run.
//!
//! - State snapshots ([`StateRecorder`]) write a TestStore run as text: each
//!   action sent or received, and how it changed the state.
//! - View snapshots ([`view::capture`]) write a drawn GPUI window as a tree of
//!   roles, labels, text, focus and disabled controls. They don't depend on
//!   fonts, GPUs or pixel sizes, so they are the default.
//! - Layout snapshots ([`layout::capture`]) write the same window with where
//!   each element, text, background and border sits, its size and its
//!   colours, in whole pixels. They are for comparing how a view is laid
//!   out; under `TestAppContext` they read the same on every OS.
//! - Image snapshots (macOS only) compare a headless render with a tolerance.
//!   On a mismatch they write the old image, the new one and their diff side
//!   by side.
//!
//! `HUMATA2_SNAPSHOT=record` writes snapshots; unset, or `verify`, checks
//! them. See [`Mode`].

mod diff;
pub mod layout;
mod mode;
#[cfg(target_os = "macos")]
mod pixels;
mod state;
mod store;
pub mod view;

#[cfg(target_os = "macos")]
pub use crate::pixels::{Tolerance, headless_context, headless_context_with};
pub use image::RgbaImage;
pub use mode::{Mode, ModeError};
pub use state::StateRecorder;
pub use store::{Outcome, SnapshotError, Snapshots};

/// The [`Snapshots`] beside the test file this is written in, in the mode
/// `HUMATA2_SNAPSHOT` sets.
///
/// Evaluates to `Result<Snapshots, SnapshotError>`.
#[macro_export]
macro_rules! snapshots {
    () => {
        $crate::Snapshots::beside(file!(), env!("CARGO_MANIFEST_DIR"))
    };
}

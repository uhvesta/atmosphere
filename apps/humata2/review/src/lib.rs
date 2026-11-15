//! Humata 2's review pane: one session's changes across its repos, read file
//! by file, with comments for the session's agent and the queue that sends
//! them (SPEC §9, RV-*, SM-*).
//!
//! The rules live in [`model`], plain data with no GPUI. [`ReviewPane`] is
//! the GPUI view the session's window embeds; it reaches the agent's
//! terminal through [`CommentSink`] and saves through [`ReviewStore`].

mod error;
mod images;
mod markdown;
pub mod model;
mod seams;
mod view;

pub use error::{ActionError, DeliveryError, HistoryError, ReviewError, StoreError};
pub use images::{ImageSide, ImageStats};
pub use markdown::{
    Block, BlockKind, Change, MarkdownRenderer, PlainBlocks, RenderedBlock, rendered_diff,
};
pub use seams::{CommentSink, HistoryFuture, ReviewHistory, ReviewStore, TypeFuture, TypeRequest};
pub use view::{ActionOutcome, MountStats, PaneEvent, ReviewPane, Theme};

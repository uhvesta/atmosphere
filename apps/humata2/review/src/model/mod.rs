//! The review's plain model: no GPUI, so every rule here is tested alone.

mod agent;
mod anchor;
mod comments;
mod image_diff;
mod memory;
mod message;
mod prepared;
mod review;
mod rows;
mod session;

pub use agent::{
    AgentAction, AgentChange, AgentMay, AgentNote, AgentView, Highlight, HighlightAt, NoteId,
    Selection, Spot, Switch, Target, Tool, Undo,
};
pub use anchor::{Anchor, LineRange, Placement, RenderedSpan, place, quote};
pub use comments::{
    AgentState, Comment, CommentBook, CommentId, Delivery, DeliveryId, DeliveryState, Dispatch,
    Failure, Hold, Kind, Status, Typed,
};
pub use image_diff::{PixelDiff, pixel_diff};
pub use memory::{Filters, ImageMode, ImageView, ReviewMemory, Viewed};
pub use message::{Part, compose};
pub use prepared::{
    Base, Layout, NoteAt, PreparedGroup, PreparedNote, PreparedReview, Reviewed, Showing,
};
pub use review::{
    ChangeKind, Content, DiffLine, FileChange, FileKey, Hunk, ImageBlob, ImageChange, LineKind,
    PullRequest, Review, SessionId, Side, TextDiff,
};
pub use rows::{BodyRow, ComposerAt, ListRow, RenderedLayout, body_rows, list_rows, passes};
pub use session::{ComposerDraft, ReviewSnapshot, SessionReview};

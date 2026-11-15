//! Humata 2's diff view: a virtualized GPUI view of a review's changes
//! across repos (RV-17–RV-24, ST-7–ST-11).
//!
//! Build a [`Review`] (from [`parse_unified`] or by hand), make a
//! [`DiffView`] with `cx.new(|cx| DiffView::new(review, cx))`, register
//! [`key_bindings`], and subscribe to [`DiffEvent`]s. Comments and drafts
//! are [`Annotation`]s the view places under their lines and asks an
//! [`AnnotationRenderer`] to draw.
//!
//! The view computes no diffs. A [`Review`] is the contract with whoever
//! does (the daemon): per repo a [`DiffSpec`] saying what is compared
//! (RV-31), per file its [`Viewed`] state (RV-29), and optionally a
//! [`Guide`] with an agent's order, groups and notes (RV-32–RV-34). The
//! user's edits come back as [`DiffEvent::SpecChanged`] and
//! [`DiffEvent::ViewedChanged`]; answer with a new review through
//! [`DiffView::set_review`].

mod anchor;
pub mod fixture;
mod guide;
mod model;
mod order;
mod patch;
mod spec;
mod view;

pub use anchor::{CommentAnchor, LineRange, LineTarget, Side};
pub use guide::{FileRef, Group, Guide, Note, NoteTarget};
pub use model::{
    DiffLine, FileDiff, FileStatus, Hunk, Language, LineKind, ParseError, RepoDiff, Review, Totals,
    Viewed, parse_unified,
};
pub use order::{Annotation, AnnotationId, DiffMode};
pub use spec::{Base, DiffSpec, Include};
pub use view::{
    AnnotationRenderer, ClearSelection, CommentOnSelection, DiffEvent, DiffView, KEY_CONTEXT,
    LongLines, NextChange, NextFile, PreviousChange, PreviousFile, key_bindings,
};

//! Humata 2's Markdown and Mermaid view.
//!
//! [`MarkdownView`] shows a Markdown document: GitHub-flavoured Markdown with
//! highlighted code, Mermaid diagrams, local images, streaming appends,
//! virtualized scrolling, selection mapped back to source, comments in the
//! margin and a rendered diff against an older version. It is built on
//! gpui-kit's text view, which does the parsing, layout, selection and code
//! highlighting; this crate adds what that view doesn't have.
//!
//! The model ([`Document`], [`Anchor`], [`diff()`]) needs no window, so an app
//! can keep comments and compare versions without showing anything.

mod anchor;
mod diff;
mod document;
mod error;
mod image;
mod mermaid;
mod theme;
mod view;

pub use anchor::{Anchor, AnchorStatus, AnchorTarget, Comment, SourceSelection};
pub use diff::{BlockChange, DeletedBlock, RenderedDiff, diff};
pub use document::{Block, BlockKind, BlockPart, Document, ImageRef, LineRange, ParseStats};
pub use error::{DiagramError, ImageError};
pub use image::ImageStats;
pub use mermaid::{DiagramStats, MermaidCache};
pub use theme::MarkdownTheme;
pub use view::{
    CommentOnSelection, CopyFormat, CopyOther, KEY_CONTEXT, Layout, MarkdownEvent, MarkdownOptions, MarkdownView,
    OutlineEntry, ToggleSource, ViewMode, ViewStats, key_bindings,
};

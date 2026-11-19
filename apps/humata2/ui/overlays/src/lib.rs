//! Overlays, command palette, dialogs, and toasts for Humata 2's GPUI desktop UI.

pub(crate) use humata2_ui_core::{chrome, theme};

pub mod overlays;
pub mod palette_hits;

pub use overlays::Overlays;

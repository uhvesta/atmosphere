//! The Humata 2 GUI spike (milestone I2): three screens that stress what
//! Humata 2's GUI must do well, each with benchmarks built in.
//!
//! - [`tree`]: a 5,000-row sidebar tree whose state lives in a tca-rust reducer.
//! - [`diff`]: a 20,000-line diff across three repositories, wrapped or not.
//! - [`terminal`]: a custom-painted 200×60 cell grid fed fake output.

pub mod args;
pub mod bench;
pub mod diff;
pub mod shell;
pub mod store_bridge;
pub mod terminal;
pub mod tree;

/// Sets up GPUI's component layers. Call once, before opening a window.
pub fn init(cx: &mut gpui_kit::App) {
    gpui_kit::init(cx);
}

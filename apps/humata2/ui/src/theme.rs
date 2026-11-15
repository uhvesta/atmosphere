//! Colors and sizes, from the prototype's design tokens.

use gpui_kit::{Hsla, Rgba, rgb, rgba};

/// The window's background.
pub const BG: u32 = 0x1a1d21;
/// The sidebar.
pub const SIDE: u32 = 0x121417;
/// The narrow rail of screens at the far left.
pub const RAIL: u32 = 0x0b0c0e;
/// Cards and sheets.
pub const PANEL: u32 = 0x202328;
/// Raised controls on a panel.
pub const PANEL2: u32 = 0x262a30;
/// Hairlines and borders.
pub const LINE: u32 = 0x33373e;
/// Body text.
pub const TEXT: u32 = 0xe6e7e9;
/// Secondary text.
pub const DIM: u32 = 0x9aa0a8;
/// Hints.
pub const FAINT: u32 = 0x6b7179;
/// The accent: links, focus, primary buttons.
pub const ACCENT: u32 = 0x5b9dff;
/// The accent, pressed.
pub const ACCENT2: u32 = 0x3d7be0;
/// Working, added, done.
pub const GREEN: u32 = 0x3fb950;
/// Needs you.
pub const ORANGE: u32 = 0xe3a13b;
/// Errors, removed, destructive.
pub const RED: u32 = 0xf05d5d;
/// Forks and agents.
pub const PURPLE: u32 = 0xb58cff;
/// Very important.
pub const YELLOW: u32 = 0xe5d066;
/// Terminal output.
pub const TERM: u32 = 0x0d0f12;
/// The terminal tab bar.
pub const TERMBAR: u32 = 0x15181c;
/// A selected row.
pub const SELECTED: u32 = 0x22354f;
/// A toast.
pub const TOAST: u32 = 0x26303d;
/// A toast's border.
pub const TOAST_LINE: u32 = 0x3b4c63;
/// An added diff line.
pub const DIFF_ADD: u32 = 0x12261a;
/// A removed diff line.
pub const DIFF_DEL: u32 = 0x2a1515;
/// A hunk header.
pub const DIFF_HUNK: u32 = 0x18202b;
/// A review comment.
pub const COMMENT: u32 = 0x1e2530;
/// A review comment's border.
pub const COMMENT_LINE: u32 = 0x2c4a6b;

/// The rail's width.
pub const RAIL_WIDTH: f32 = 62.0;
/// The top bar's height.
pub const TOP_BAR: f32 = 52.0;
/// A sidebar row's height; every row is the same so the list virtualizes.
pub const ROW: f32 = 28.0;
/// A pane divider's width.
pub const DIVIDER: f32 = 6.0;
/// A collapsed pane's strip.
pub const STRIP: f32 = 26.0;
/// A sheet's width.
pub const SHEET: f32 = 640.0;
/// A board column's width.
pub const COLUMN: f32 = 250.0;

/// `hex` as a color.
#[must_use]
pub fn color(hex: u32) -> Rgba {
    rgb(hex)
}

/// The scrim behind sheets.
#[must_use]
pub fn scrim() -> Hsla {
    rgba(0x0000_008c).into()
}

//! D76 appearance tokens and shared control sizes.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{App, Rgba, rgb};

/// Body text.
pub const TEXT: u32 = 0xe6e7e9;
/// Secondary text.
pub const DIM: u32 = 0x9aa0a8;
/// Hints.
pub const FAINT: u32 = 0x6b7179;
/// The accent, pressed.
pub const ACCENT2: u32 = 0x3d7be0;
/// Errors, removed, destructive.
pub const RED: u32 = 0xf05d5d;

/// The top bar's height.
pub const TOP_BAR: f32 = 52.0;
/// A sidebar row's height; every row is the same so the list virtualizes.
pub const ROW: f32 = 26.0;
/// A pane divider's width.
pub const DIVIDER: f32 = 6.0;
/// A sheet's width.
pub const SHEET: f32 = 640.0;

/// `hex` as a color.
#[must_use]
pub fn color(hex: u32) -> Rgba {
    rgb(hex)
}

/// Tokens copied from the approved light/dark frames, rather than the
/// earlier prototype palette used by the rest of the shell.
#[derive(Clone, Copy)]
pub(crate) struct Look {
    pub(crate) sidebar: u32,
    pub(crate) selection: u32,
    pub(crate) panel: u32,
    pub(crate) line: u32,
    pub(crate) text: u32,
    pub(crate) secondary: u32,
    pub(crate) tertiary: u32,
    pub(crate) accent: u32,
    pub(crate) fill: u32,
    pub(crate) card: u32,
    pub(crate) term: u32,
    pub(crate) term_text: u32,
    pub(crate) term_dim: u32,
    pub(crate) add: u32,
    pub(crate) del: u32,
    pub(crate) content: u32,
    pub(crate) attention_bg: u32,
}
impl Look {
    pub(crate) fn of(cx: &App) -> Self {
        if cx.theme().is_dark() {
            Self {
                sidebar: 0x2a2a2d,
                selection: 0x0a6cff,
                panel: 0x252527,
                line: 0x3a3a3d,
                text: 0xe8e8ea,
                secondary: 0x98989e,
                tertiary: 0x6a6a70,
                accent: 0x3b8cff,
                fill: 0x323235,
                card: 0x2a2a2c,
                term: 0x1a1a1b,
                term_text: 0xdcdcde,
                term_dim: 0x7c7c82,
                add: 0x17301f,
                del: 0x3a1d1d,
                content: 0x1e1e1f,
                attention_bg: 0x3a2c12,
            }
        } else {
            Self {
                sidebar: 0xe9e7ea,
                selection: 0x0a6cff,
                panel: 0xf6f5f7,
                line: 0xdcdadd,
                text: 0x1d1d1f,
                secondary: 0x6e6e73,
                tertiary: 0xa1a1a6,
                accent: 0x0a6cff,
                fill: 0xeceaee,
                card: 0xffffff,
                term: 0xffffff,
                term_text: 0x1d1d1f,
                term_dim: 0x8a8a8f,
                add: 0xe6f6ea,
                del: 0xfdeaea,
                content: 0xffffff,
                attention_bg: 0xfff4e0,
            }
        }
    }
}

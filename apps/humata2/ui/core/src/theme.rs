//! D76 appearance tokens and shared control sizes.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{App, Rgba, rgb};

/// Errors, removed, destructive.
pub const RED: u32 = 0xf05d5d;

/// Status: working.
pub const GREEN: u32 = 0x30d158;
/// Status: needs you.
pub const ORANGE: u32 = 0xff9f0a;
/// Status: failed or exited, as the frames draw it.
pub const RED_STATUS: u32 = 0xff453a;
/// Status: idle or offline.
pub const GREY: u32 = 0x8e8e93;

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
pub struct Look {
    pub sidebar: u32,
    pub selection: u32,
    pub panel: u32,
    pub line: u32,
    pub text: u32,
    pub secondary: u32,
    pub tertiary: u32,
    pub accent: u32,
    pub fill: u32,
    pub card: u32,
    pub term: u32,
    pub term_text: u32,
    pub term_dim: u32,
    pub add: u32,
    pub del: u32,
    pub content: u32,
    pub attention_bg: u32,
}
impl Look {
    pub fn of(cx: &App) -> Self {
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

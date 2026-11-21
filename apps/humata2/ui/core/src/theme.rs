//! D76 appearance tokens and shared control sizes.

use std::sync::Arc;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{App, BoxShadow, FontFeatures, Rgba, SharedString, point, px, rgb, rgba};

/// Preferred monospace font families in priority order.
pub const MONO_FONT_STACK: &[&str] = &["SF Mono", "Menlo", "Monaco", "JetBrains Mono"];

/// Picks the first installed family from [`MONO_FONT_STACK`], if any.
#[must_use]
pub fn preferred_mono_font(installed: &[String]) -> Option<SharedString> {
    MONO_FONT_STACK
        .iter()
        .find(|candidate| installed.iter().any(|name| name == *candidate))
        .map(|name| SharedString::from(*name))
}

/// OpenType `tnum` (tabular numerals) feature set for aligned counters, timers,
/// and numeric columns.
#[must_use]
pub fn tabular_nums() -> FontFeatures {
    FontFeatures(Arc::new(vec![("tnum".into(), 1)]))
}

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
    dark: bool,
    pub sidebar: u32,
    pub selection: u32,
    pub panel: u32,
    pub line: u32,
    pub control_border: Rgba,
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
                dark: true,
                sidebar: 0x2a2a2d,
                selection: 0x0a6cff,
                panel: 0x252527,
                line: 0x3a3a3d,
                control_border: rgb(0x48484c),
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
                dark: false,
                sidebar: 0xf3f3f6,
                selection: 0x0a6cff,
                panel: 0xf6f5f7,
                line: 0xe5e5ea,
                control_border: rgb(0xd4d4da),
                text: 0x1d1d1f,
                secondary: 0x6e6e73,
                tertiary: 0xa1a1a6,
                accent: 0x0a6cff,
                fill: 0xf3f3f5,
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

    /// Whether this palette is dark mode.
    #[must_use]
    pub const fn is_dark(&self) -> bool {
        self.dark
    }

    /// Returns a copy of this palette with terminal surface tokens (`term`,
    /// `term_text`, `term_dim`) set for `dark` mode, keeping `is_dark()` and
    /// all app-chrome, editor, and diff tokens untouched.
    #[must_use]
    pub const fn with_terminal_dark(mut self, dark: bool) -> Self {
        if dark {
            self.term = 0x1a1a1b;
            self.term_text = 0xdcdcde;
            self.term_dim = 0x7c7c82;
        } else {
            self.term = 0xffffff;
            self.term_text = 0x1d1d1f;
            self.term_dim = 0x8a8a8f;
        }
        self
    }

    /// A translucent Retina hairline color for pane dividers, toolbar borders,
    /// and card/row separators.
    #[must_use]
    pub fn hairline(&self) -> Rgba {
        if self.dark {
            rgba(0xffffff16)
        } else {
            rgba(0x0000001a)
        }
    }
}

/// Subtle layered elevation for secondary buttons and segmented controls.
#[must_use]
pub fn shadow_control(look: Look) -> Vec<BoxShadow> {
    if look.is_dark() {
        vec![
            BoxShadow {
                color: rgba(0x00000038).into(),
                offset: point(px(0.), px(1.)),
                blur_radius: px(2.),
                spread_radius: px(0.),
                inset: false,
            },
            BoxShadow {
                color: rgba(0x00000024).into(),
                offset: point(px(0.), px(0.5)),
                blur_radius: px(0.5),
                spread_radius: px(0.),
                inset: false,
            },
        ]
    } else {
        vec![
            BoxShadow {
                color: rgba(0x00000014).into(),
                offset: point(px(0.), px(0.5)),
                blur_radius: px(1.),
                spread_radius: px(0.),
                inset: false,
            },
            BoxShadow {
                color: rgba(0x0000000a).into(),
                offset: point(px(0.), px(1.)),
                blur_radius: px(2.),
                spread_radius: px(0.),
                inset: false,
            },
        ]
    }
}

/// Layered elevation for floating menus and popovers.
#[must_use]
pub fn shadow_popover(look: Look) -> Vec<BoxShadow> {
    if look.is_dark() {
        vec![
            BoxShadow {
                color: rgba(0x0000004d).into(),
                offset: point(px(0.), px(2.)),
                blur_radius: px(6.),
                spread_radius: px(0.),
                inset: false,
            },
            BoxShadow {
                color: rgba(0x00000066).into(),
                offset: point(px(0.), px(10.)),
                blur_radius: px(24.),
                spread_radius: px(-2.),
                inset: false,
            },
        ]
    } else {
        vec![
            BoxShadow {
                color: rgba(0x0000001a).into(),
                offset: point(px(0.), px(1.)),
                blur_radius: px(3.),
                spread_radius: px(0.),
                inset: false,
            },
            BoxShadow {
                color: rgba(0x0000001f).into(),
                offset: point(px(0.), px(6.)),
                blur_radius: px(18.),
                spread_radius: px(-2.),
                inset: false,
            },
        ]
    }
}

/// Layered elevation for modal sheets and the command palette.
#[must_use]
pub fn shadow_sheet(look: Look) -> Vec<BoxShadow> {
    if look.is_dark() {
        vec![
            BoxShadow {
                color: rgba(0x00000059).into(),
                offset: point(px(0.), px(2.)),
                blur_radius: px(8.),
                spread_radius: px(0.),
                inset: false,
            },
            BoxShadow {
                color: rgba(0x00000073).into(),
                offset: point(px(0.), px(14.)),
                blur_radius: px(36.),
                spread_radius: px(-4.),
                inset: false,
            },
            BoxShadow {
                color: rgba(0x0000004d).into(),
                offset: point(px(0.), px(28.)),
                blur_radius: px(56.),
                spread_radius: px(-8.),
                inset: false,
            },
        ]
    } else {
        vec![
            BoxShadow {
                color: rgba(0x00000014).into(),
                offset: point(px(0.), px(2.)),
                blur_radius: px(6.),
                spread_radius: px(0.),
                inset: false,
            },
            BoxShadow {
                color: rgba(0x00000026).into(),
                offset: point(px(0.), px(12.)),
                blur_radius: px(32.),
                spread_radius: px(-4.),
                inset: false,
            },
            BoxShadow {
                color: rgba(0x0000001a).into(),
                offset: point(px(0.), px(24.)),
                blur_radius: px(48.),
                spread_radius: px(-8.),
                inset: false,
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Look, preferred_mono_font, shadow_control, shadow_popover, shadow_sheet, tabular_nums,
    };
    use gpui_kit::{SharedString, rgb, rgba};

    fn sample(dark: bool) -> Look {
        Look {
            dark,
            sidebar: if dark { 0x2a2a2d } else { 0xf3f3f6 },
            selection: 0x0a6cff,
            panel: if dark { 0x252527 } else { 0xf6f5f7 },
            line: if dark { 0x3a3a3d } else { 0xe5e5ea },
            control_border: if dark { rgb(0x48484c) } else { rgb(0xd4d4da) },
            text: if dark { 0xe8e8ea } else { 0x1d1d1f },
            secondary: if dark { 0x98989e } else { 0x6e6e73 },
            tertiary: if dark { 0x6a6a70 } else { 0xa1a1a6 },
            accent: if dark { 0x3b8cff } else { 0x0a6cff },
            fill: if dark { 0x323235 } else { 0xf3f3f5 },
            card: if dark { 0x2a2a2c } else { 0xffffff },
            term: if dark { 0x1a1a1b } else { 0xffffff },
            term_text: if dark { 0xdcdcde } else { 0x1d1d1f },
            term_dim: if dark { 0x7c7c82 } else { 0x8a8a8f },
            add: if dark { 0x17301f } else { 0xe6f6ea },
            del: if dark { 0x3a1d1d } else { 0xfdeaea },
            content: if dark { 0x1e1e1f } else { 0xffffff },
            attention_bg: if dark { 0x3a2c12 } else { 0xfff4e0 },
        }
    }

    #[test]
    fn hairline_uses_translucent_tint_for_light_and_dark() {
        assert_eq!(sample(false).hairline(), rgba(0x0000001a));
        assert_eq!(sample(true).hairline(), rgba(0xffffff16));
    }

    #[test]
    fn shadow_helpers_return_layered_shadows_in_both_modes() {
        for dark in [false, true] {
            let look = sample(dark);
            assert!(shadow_control(look).len() >= 2);
            assert!(shadow_popover(look).len() >= 2);
            assert!(shadow_sheet(look).len() >= 3);
        }
    }

    #[test]
    fn preferred_mono_font_prefers_sf_mono_before_fallbacks() {
        let with_sf = vec!["Menlo".to_owned(), "SF Mono".to_owned()];
        assert_eq!(preferred_mono_font(&with_sf), Some(SharedString::from("SF Mono")));

        let fallback = vec!["Monaco".to_owned(), "Menlo".to_owned()];
        assert_eq!(preferred_mono_font(&fallback), Some(SharedString::from("Menlo")));

        assert_eq!(preferred_mono_font(&[]), None);
    }

    #[test]
    fn tabular_nums_enables_tnum_feature() {
        assert_eq!(tabular_nums().tag_value_list(), &[("tnum".to_owned(), 1)]);
    }

    #[test]
    fn with_terminal_dark_splits_terminal_tokens_from_app_chrome() {
        let light_app = sample(false);
        let dark_term = light_app.with_terminal_dark(true);
        assert!(!dark_term.is_dark());
        assert_eq!(dark_term.content, light_app.content);
        assert_eq!(dark_term.term, 0x1a1a1b);
        assert_eq!(dark_term.term_text, 0xdcdcde);
        assert_eq!(dark_term.term_dim, 0x7c7c82);

        let dark_app = sample(true);
        let light_term = dark_app.with_terminal_dark(false);
        assert!(light_term.is_dark());
        assert_eq!(light_term.content, dark_app.content);
        assert_eq!(light_term.term, 0xffffff);
        assert_eq!(light_term.term_text, 0x1d1d1f);
        assert_eq!(light_term.term_dim, 0x8a8a8f);
    }
}

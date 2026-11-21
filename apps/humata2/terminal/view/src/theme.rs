use gpui_kit::SharedString;
use terminal_grid::Rgb;

use crate::frame::Style;

/// How the view draws: the font and the colors the grid doesn't set.
///
/// The default foreground, background and palette belong to the grid,
/// since programs change them (OSC 4, 10, 11); a host that has theme
/// colors pushes them into the engine, as `ghostty_vt`'s
/// `Terminal::set_default_colors` does.
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    /// A monospace font family.
    pub font_family: SharedString,
    /// The font size, in pixels.
    pub font_size: f32,
    /// The height of a row, in pixels.
    pub line_height: f32,
    /// The background of selected text; `None` swaps each cell's colors,
    /// as `Ghostty` does by default.
    pub selection_background: Option<Rgb>,
    /// The foreground of selected text; `None` swaps each cell's colors.
    pub selection_foreground: Option<Rgb>,
    /// The color of the text under a block cursor; `None` uses the cell's
    /// background.
    pub cursor_text: Option<Rgb>,
    /// Draw bold text in palette colors 0-7 with the bright colors 8-15,
    /// as `Ghostty`'s `bold-is-bright` does.
    pub bold_is_bright: bool,
    /// The opacity of faint (SGR 2) text, from 0 to 1.
    pub faint_opacity: f32,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            font_family: if cfg!(target_os = "macos") {
                "Menlo".into()
            } else {
                "monospace".into()
            },
            font_size: 13.0,
            line_height: 17.0,
            selection_background: None,
            selection_foreground: None,
            cursor_text: None,
            bold_is_bright: false,
            faint_opacity: 0.5,
        }
    }
}

impl Theme {
    pub(crate) fn style(&self, light_bg: bool) -> Style {
        let selection_tint = (self.selection_background.is_none()
            && self.selection_foreground.is_none())
        .then(|| {
            if light_bg {
                (Rgb::hex(0x0a6cff), 0.22)
            } else {
                (Rgb::hex(0x3b8cff), 0.32)
            }
        });
        Style {
            selection_background: self.selection_background,
            selection_foreground: self.selection_foreground,
            selection_tint,
            bold_is_bright: self.bold_is_bright,
            faint_opacity: self.faint_opacity,
        }
    }
}

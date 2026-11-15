//! Turning libghostty-vt's cell values into [`Cell`]s. The types themselves
//! live in `terminal_grid`, which the view reads without this crate.

use terminal_grid::{Cell, CellWidth, Color, Underline};

use crate::error::{Status, check};
use crate::ffi;

/// The cell `raw`, drawn in `style`, holding the grapheme cluster `text`.
/// The grid reference and render state APIs both hand out these three, so
/// both build cells here.
pub(crate) fn cell_from(
    raw: ffi::GhosttyCell,
    style: &ffi::GhosttyStyle,
    text: String,
) -> Result<Cell, Status> {
    let wide: i32 = cell_get(raw, ffi::CELL_DATA_WIDE)?;
    let content: i32 = cell_get(raw, ffi::CELL_DATA_CONTENT_TAG)?;

    // A cell erased with a background color holds just that color.
    let bg = match content {
        ffi::CELL_CONTENT_BG_COLOR_PALETTE => {
            Color::Palette(cell_get(raw, ffi::CELL_DATA_COLOR_PALETTE)?)
        }
        ffi::CELL_CONTENT_BG_COLOR_RGB => {
            let rgb: ffi::GhosttyColorRgb = cell_get(raw, ffi::CELL_DATA_COLOR_RGB)?;
            Color::Rgb {
                r: rgb.r,
                g: rgb.g,
                b: rgb.b,
            }
        }
        _ => color_from_style(style.bg_color),
    };

    let flags = style.flags;
    Ok(Cell {
        text,
        width: match wide {
            ffi::CELL_WIDE_WIDE => CellWidth::Wide,
            ffi::CELL_WIDE_SPACER_TAIL => CellWidth::SpacerTail,
            ffi::CELL_WIDE_SPACER_HEAD => CellWidth::SpacerHead,
            _ => CellWidth::Narrow,
        },
        fg: color_from_style(style.fg_color),
        bg,
        underline_color: color_from_style(style.underline_color),
        bold: flags & ffi::STYLE_FLAG_BOLD != 0,
        italic: flags & ffi::STYLE_FLAG_ITALIC != 0,
        faint: flags & ffi::STYLE_FLAG_FAINT != 0,
        blink: flags & ffi::STYLE_FLAG_BLINK != 0,
        inverse: flags & ffi::STYLE_FLAG_INVERSE != 0,
        invisible: flags & ffi::STYLE_FLAG_INVISIBLE != 0,
        strikethrough: flags & ffi::STYLE_FLAG_STRIKETHROUGH != 0,
        overline: flags & ffi::STYLE_FLAG_OVERLINE != 0,
        underline: underline_from_flags(flags),
    })
}

fn cell_get<T: Default>(cell: ffi::GhosttyCell, data: ffi::GhosttyCellData) -> Result<T, Status> {
    let mut out = T::default();
    // SAFETY: callers pick `T` as the output type `data` documents.
    check(unsafe { ffi::ghostty_cell_get(cell, data, (&raw mut out).cast()) })?;
    Ok(out)
}

fn color_from_style(color: ffi::GhosttyStyleColor) -> Color {
    // SAFETY: the tag says which member of the union libghostty-vt set,
    // and both members are plain bytes.
    unsafe {
        match color.tag {
            ffi::STYLE_COLOR_PALETTE => Color::Palette(color.value.palette),
            ffi::STYLE_COLOR_RGB => {
                let rgb = color.value.rgb;
                Color::Rgb {
                    r: rgb.r,
                    g: rgb.g,
                    b: rgb.b,
                }
            }
            _ => Color::Default,
        }
    }
}

fn underline_from_flags(flags: u16) -> Underline {
    match (flags & ffi::STYLE_UNDERLINE_MASK) >> ffi::STYLE_UNDERLINE_SHIFT {
        1 => Underline::Single,
        2 => Underline::Double,
        3 => Underline::Curly,
        4 => Underline::Dotted,
        5 => Underline::Dashed,
        _ => Underline::None,
    }
}

//! What to draw for a row, worked out from the grid alone: no `GPUI`, no
//! fonts, so it is tested as plain data.
//!
//! Modelled on `Ghostty`'s `renderer/generic.zig` (`rebuildRow`): each cell
//! gives a background, a glyph and its decorations, with inverse video and
//! the selection swapping colors first. Where `Ghostty` places every glyph
//! in its own cell, the view groups cells into text pieces that are shaped
//! together, breaking where `Ghostty`'s run iterator (`font/shaper/run.zig`)
//! breaks: on a change of style, at the selection's edges, and around any
//! cell that must stand alone (wide characters, clusters of more than one
//! scalar, right-to-left letters).

use terminal_grid::{Cell, CellWidth, Color, Colors, Rgb, Row, Span, Underline};

use crate::boxes::{self, Drawing};

/// How a row's colors are worked out, from the view's theme.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Style {
    /// Background of selected cells; `None` uses `selection_tint` or swaps.
    pub selection_background: Option<Rgb>,
    /// Foreground of selected cells; `None` uses `selection_tint` or swaps.
    pub selection_foreground: Option<Rgb>,
    /// Translucent accent tint `(color, alpha)` blended over a selected cell's
    /// background when explicit selection colors aren't set, keeping cell fg.
    pub selection_tint: Option<(Rgb, f32)>,
    /// Draw bold text in palette colors 0-7 with their bright twins 8-15.
    pub bold_is_bright: bool,
    /// The opacity of faint (SGR 2) text.
    pub faint_opacity: f32,
}

/// A stretch of cells with one background.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Fill {
    pub col: u16,
    pub cols: u16,
    pub color: Rgb,
}

/// How a piece of text looks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TextStyle {
    pub color: Rgb,
    pub alpha: f32,
    pub bold: bool,
    pub italic: bool,
}

/// Text shaped as one line and drawn from column `col`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Piece {
    pub col: u16,
    pub text: String,
    pub style: TextStyle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LineKind {
    Underline(Underline),
    Strikethrough,
    Overline,
}

/// A line drawn across cells: an underline, strikethrough or overline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Decoration {
    pub col: u16,
    pub cols: u16,
    pub kind: LineKind,
    pub color: Rgb,
    pub alpha: f32,
}

/// Everything drawn for one row, apart from the cursor.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct RowPlan {
    /// Backgrounds other than the default, which the view paints once for
    /// the whole grid.
    pub fills: Vec<Fill>,
    pub pieces: Vec<Piece>,
    pub decorations: Vec<Decoration>,
    /// Box-drawing and block characters, drawn by the view rather than
    /// the font.
    pub drawings: Vec<Drawn>,
}

/// A box-drawing or block character at column `col`, drawn as rectangles.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Drawn {
    pub col: u16,
    pub cols: u16,
    pub drawing: Drawing,
    pub color: Rgb,
    pub alpha: f32,
}

/// The colors a cell is drawn in, after inverse video and selection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CellColors {
    pub fg: Rgb,
    pub bg: Rgb,
}

/// Works out the colors of `cell`, as `Ghostty`'s `rebuildRow` does: a
/// selected cell takes the selection colors, or swaps its own when the
/// theme has none; an inverse cell swaps; bold palette colors 0-7 become
/// bright when asked.
pub(crate) fn cell_colors(
    cell: &Cell,
    selected: bool,
    colors: &Colors,
    style: &Style,
) -> CellColors {
    let fg_color = match cell.fg {
        Color::Palette(index) if cell.bold && style.bold_is_bright && index < 8 => {
            Color::Palette(index + 8)
        }
        other => other,
    };
    let fg = colors.resolve(fg_color).unwrap_or(colors.foreground);
    let bg = colors.resolve(cell.bg).unwrap_or(colors.background);
    let (fg, bg) = if cell.inverse { (bg, fg) } else { (fg, bg) };
    if selected {
        if style.selection_background.is_some() || style.selection_foreground.is_some() {
            CellColors {
                fg: style.selection_foreground.unwrap_or(bg),
                bg: style.selection_background.unwrap_or(fg),
            }
        } else if let Some((tint, alpha)) = style.selection_tint {
            CellColors {
                fg,
                bg: blend_rgb(bg, tint, alpha),
            }
        } else {
            CellColors { fg: bg, bg: fg }
        }
    } else {
        CellColors { fg, bg }
    }
}

fn blend_rgb(base: Rgb, overlay: Rgb, alpha: f32) -> Rgb {
    let a = alpha.clamp(0.0, 1.0);
    let mix = |b: u8, o: u8| -> u8 {
        (f32::from(o) * a + f32::from(b) * (1.0 - a))
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Rgb {
        r: mix(base.r, overlay.r),
        g: mix(base.g, overlay.g),
        b: mix(base.b, overlay.b),
    }
}

/// Plans row `row`, whose selected columns are `selection`.
pub(crate) fn plan_row(
    row: &Row,
    selection: Option<Span>,
    colors: &Colors,
    style: &Style,
) -> RowPlan {
    let mut plan = RowPlan::default();
    let mut open: Option<Piece> = None;
    // Empty cells after the open piece, which join it as spaces if text in
    // the same style follows.
    let mut gap: u16 = 0;

    for (x, cell) in row.cells.iter().enumerate() {
        let Ok(col) = u16::try_from(x) else {
            break;
        };
        let selected = selection.is_some_and(|span| span.contains(col));
        let drawn = cell_colors(cell, selected, colors, style);
        let cols = match cell.width {
            CellWidth::Wide => 2,
            // The wide character before a tail covers it; a head is a
            // blank cell at the end of the row.
            CellWidth::SpacerTail => 0,
            CellWidth::Narrow | CellWidth::SpacerHead => 1,
        };

        if drawn.bg != colors.background {
            // A tail's background is drawn by its wide character; a tail
            // styled apart from it (as an engine may) still gets its own.
            let covered = cell.width == CellWidth::SpacerTail
                && plan
                    .fills
                    .last()
                    .is_some_and(|fill| fill.col + fill.cols > col && fill.color == drawn.bg);
            if !covered {
                push_fill(
                    &mut plan.fills,
                    Fill {
                        col,
                        cols: cols.max(1),
                        color: drawn.bg,
                    },
                );
            }
        }
        if cols == 0 {
            continue;
        }

        let alpha = if cell.faint { style.faint_opacity } else { 1.0 };
        let text_style = TextStyle {
            color: drawn.fg,
            alpha,
            bold: cell.bold,
            italic: cell.italic,
        };
        decorate(
            &mut plan.decorations,
            cell,
            col,
            cols,
            colors,
            drawn.fg,
            alpha,
        );

        let has_text = !cell.text.is_empty() && !cell.invisible && cell.text != " ";
        if !has_text {
            if open.is_some() {
                gap += cols;
            }
            continue;
        }
        if let Some(drawing) = single_char(&cell.text).and_then(boxes::drawing) {
            // Text in the same style may carry on past it, with a space
            // where it is drawn.
            plan.drawings.push(Drawn {
                col,
                cols,
                drawing,
                color: drawn.fg,
                alpha,
            });
            if open.is_some() {
                gap += cols;
            }
            continue;
        }
        let alone = stands_alone(cell);
        match open.as_mut() {
            Some(piece) if !alone && piece.style == text_style => {
                piece
                    .text
                    .extend(std::iter::repeat_n(' ', usize::from(gap)));
                piece.text.push_str(&cell.text);
                gap = 0;
            }
            _ => {
                if let Some(piece) = open.take() {
                    plan.pieces.push(piece);
                }
                gap = 0;
                let piece = Piece {
                    col,
                    text: cell.text.clone(),
                    style: text_style,
                };
                if alone {
                    plan.pieces.push(piece);
                } else {
                    open = Some(piece);
                }
            }
        }
    }
    if let Some(piece) = open {
        plan.pieces.push(piece);
    }
    plan
}

/// Adds `fill`, joining it to the one before when they touch and match.
fn push_fill(fills: &mut Vec<Fill>, fill: Fill) {
    if let Some(last) = fills.last_mut()
        && last.color == fill.color
        && last.col + last.cols == fill.col
    {
        last.cols += fill.cols;
        return;
    }
    fills.push(fill);
}

fn decorate(
    decorations: &mut Vec<Decoration>,
    cell: &Cell,
    col: u16,
    cols: u16,
    colors: &Colors,
    fg: Rgb,
    alpha: f32,
) {
    let mut push = |kind: LineKind, color: Rgb| {
        if let Some(last) = decorations
            .iter_mut()
            .rev()
            .find(|line| line.kind == kind && line.col + line.cols == col)
            && last.color == color
            && last.alpha == alpha
        {
            last.cols += cols;
            return;
        }
        decorations.push(Decoration {
            col,
            cols,
            kind,
            color,
            alpha,
        });
    };
    if cell.underline != Underline::None {
        let color = colors.resolve(cell.underline_color).unwrap_or(fg);
        push(LineKind::Underline(cell.underline), color);
    }
    if cell.strikethrough {
        push(LineKind::Strikethrough, fg);
    }
    if cell.overline {
        push(LineKind::Overline, fg);
    }
}

/// The cell's text when it is one scalar.
fn single_char(text: &str) -> Option<char> {
    let mut chars = text.chars();
    let first = chars.next()?;
    chars.next().is_none().then_some(first)
}

/// Whether a cell must be shaped on its own: a wide character, so it
/// lands on its two columns whatever the fallback font's advance; a
/// cluster of more than one scalar, so a combining mark or emoji sequence
/// can't shift the cells after it; or a right-to-left letter, so the
/// shaper can't reorder the cells around it. Like `Ghostty`, the grid is
/// drawn left to right, one cell at a time, so Arabic letters don't join.
fn stands_alone(cell: &Cell) -> bool {
    if cell.width == CellWidth::Wide {
        return true;
    }
    let mut chars = cell.text.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    chars.next().is_some() || is_right_to_left(first) || is_technical(first)
}

/// Miscellaneous Technical (U+2300–23FF), such as the "⎿" agents draw in
/// their gutter. Monospace fonts often lack them, and a fallback glyph of
/// another width shaped in the same run would push the next cell's text.
fn is_technical(c: char) -> bool {
    matches!(u32::from(c), 0x2300..=0x23ff)
}

/// Letters of scripts written right to left: Hebrew, Arabic, Syriac,
/// Thaana, NKo and their presentation forms, and the historic scripts in
/// the right-to-left planes.
fn is_right_to_left(c: char) -> bool {
    matches!(
        u32::from(c),
        0x0590..=0x08ff | 0xfb1d..=0xfdff | 0xfe70..=0xfeff | 0x10800..=0x10fff | 0x1e800..=0x1efff
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const STYLE: Style = Style {
        selection_background: None,
        selection_foreground: None,
        selection_tint: None,
        bold_is_bright: true,
        faint_opacity: 0.5,
    };

    fn row(cells: Vec<Cell>) -> Row {
        Row {
            cells,
            wrapped: false,
        }
    }

    fn text(text: &str) -> Cell {
        Cell {
            text: text.to_owned(),
            ..Cell::default()
        }
    }

    fn styled(text: &str, style: Cell) -> Cell {
        Cell {
            text: text.to_owned(),
            ..style
        }
    }

    fn pieces(plan: &RowPlan) -> Vec<(u16, &str)> {
        plan.pieces
            .iter()
            .map(|piece| (piece.col, piece.text.as_str()))
            .collect()
    }

    #[::std::prelude::v1::test]
    fn words_in_one_style_are_one_piece_with_their_spaces() {
        let cells = vec![
            text("l"),
            text("s"),
            Cell::default(),
            text("-"),
            text("l"),
            Cell::default(),
        ];
        let plan = plan_row(&row(cells), None, &Colors::default(), &STYLE);
        assert_eq!(pieces(&plan), vec![(0, "ls -l")]);
        assert!(
            plan.fills.is_empty(),
            "default backgrounds aren't filled per row"
        );
    }

    #[::std::prelude::v1::test]
    fn box_characters_are_drawn_and_text_carries_on_around_them() {
        let red = Cell {
            fg: Color::Palette(1),
            ..Cell::default()
        };
        let cells = vec![
            text("│"),
            text("a"),
            text("─"),
            text("b"),
            styled("█", red),
            text("╱"),
        ];
        let plan = plan_row(&row(cells), None, &Colors::default(), &STYLE);
        // The diagonal is the font's, and joins the text before it; the
        // line, the rule and the block are drawn, each in its own cell's
        // color.
        assert_eq!(pieces(&plan), vec![(1, "a b ╱")]);
        let drawn: Vec<(u16, Rgb)> = plan
            .drawings
            .iter()
            .map(|drawn| (drawn.col, drawn.color))
            .collect();
        let colors = Colors::default();
        assert_eq!(
            drawn,
            vec![
                (0, colors.foreground),
                (2, colors.foreground),
                (4, colors.palette[1])
            ]
        );
    }

    #[::std::prelude::v1::test]
    fn a_change_of_color_starts_a_new_piece() {
        let red = Cell {
            fg: Color::Palette(1),
            ..Cell::default()
        };
        let cells = vec![
            text("a"),
            styled("b", red.clone()),
            styled("c", red),
            text("d"),
        ];
        let plan = plan_row(&row(cells), None, &Colors::default(), &STYLE);
        assert_eq!(pieces(&plan), vec![(0, "a"), (1, "bc"), (3, "d")]);
        let colors = Colors::default();
        assert_eq!(plan.pieces[1].style.color, colors.palette[1]);
        assert_eq!(plan.pieces[0].style.color, colors.foreground);
    }

    #[::std::prelude::v1::test]
    fn wide_characters_stand_alone_on_their_two_columns() {
        let wide = Cell {
            text: "日".to_owned(),
            width: CellWidth::Wide,
            ..Cell::default()
        };
        let tail = Cell {
            width: CellWidth::SpacerTail,
            ..Cell::default()
        };
        let cells = vec![text("a"), wide.clone(), tail.clone(), wide, tail, text("b")];
        let plan = plan_row(&row(cells), None, &Colors::default(), &STYLE);
        assert_eq!(
            pieces(&plan),
            vec![(0, "a"), (1, "日"), (3, "日"), (5, "b")]
        );
    }

    #[::std::prelude::v1::test]
    fn clusters_and_right_to_left_letters_stand_alone() {
        let emoji = Cell {
            text: "👩\u{200d}💻".to_owned(),
            width: CellWidth::Wide,
            ..Cell::default()
        };
        let tail = Cell {
            width: CellWidth::SpacerTail,
            ..Cell::default()
        };
        let cells = vec![
            text("x"),
            text("e\u{301}"),
            text("y"),
            text("ש"),
            text("ל"),
            emoji,
            tail,
        ];
        let plan = plan_row(&row(cells), None, &Colors::default(), &STYLE);
        assert_eq!(
            pieces(&plan),
            vec![
                (0, "x"),
                (1, "e\u{301}"),
                (2, "y"),
                (3, "ש"),
                (4, "ל"),
                (5, "👩\u{200d}💻")
            ]
        );
    }

    #[::std::prelude::v1::test]
    fn neighbouring_backgrounds_join_and_a_wide_cell_fills_both_columns() {
        let blue = Cell {
            bg: Color::Palette(4),
            ..Cell::default()
        };
        let wide = Cell {
            text: "日".to_owned(),
            width: CellWidth::Wide,
            ..blue.clone()
        };
        let tail = Cell {
            width: CellWidth::SpacerTail,
            ..blue.clone()
        };
        let cells = vec![styled("a", blue.clone()), wide, tail, Cell::default(), blue];
        let colors = Colors::default();
        let plan = plan_row(&row(cells), None, &colors, &STYLE);
        let blue = colors.palette[4];
        assert_eq!(
            plan.fills,
            vec![
                Fill {
                    col: 0,
                    cols: 3,
                    color: blue
                },
                Fill {
                    col: 4,
                    cols: 1,
                    color: blue
                },
            ]
        );
    }

    #[::std::prelude::v1::test]
    fn inverse_swaps_the_colors() {
        let colors = Colors::default();
        let inverse = Cell {
            inverse: true,
            ..Cell::default()
        };
        let plan = plan_row(&row(vec![styled("a", inverse)]), None, &colors, &STYLE);
        assert_eq!(plan.pieces[0].style.color, colors.background);
        assert_eq!(
            plan.fills,
            vec![Fill {
                col: 0,
                cols: 1,
                color: colors.foreground
            }]
        );
    }

    #[::std::prelude::v1::test]
    fn the_selection_swaps_colors_unless_the_theme_sets_them() {
        let colors = Colors::default();
        let cells = vec![text("a"), text("b"), text("c")];
        let span = Some(Span { start: 1, end: 1 });

        let plan = plan_row(&row(cells.clone()), span, &colors, &STYLE);
        assert_eq!(pieces(&plan), vec![(0, "a"), (1, "b"), (2, "c")]);
        assert_eq!(plan.pieces[1].style.color, colors.background);
        assert_eq!(
            plan.fills,
            vec![Fill {
                col: 1,
                cols: 1,
                color: colors.foreground
            }]
        );

        let themed = Style {
            selection_background: Some(Rgb::hex(0x334455)),
            selection_foreground: Some(Rgb::hex(0xffeedd)),
            ..STYLE
        };
        let plan = plan_row(&row(cells.clone()), span, &colors, &themed);
        assert_eq!(plan.pieces[1].style.color, Rgb::hex(0xffeedd));
        assert_eq!(plan.fills[0].color, Rgb::hex(0x334455));

        let tinted = Style {
            selection_tint: Some((Rgb::hex(0x3b8cff), 0.32)),
            ..STYLE
        };
        let plan = plan_row(&row(cells), span, &colors, &tinted);
        assert_eq!(plan.pieces[0].style.color, colors.foreground);
        assert_ne!(plan.fills[0].color, colors.foreground);
        assert_ne!(plan.fills[0].color, colors.background);
    }

    #[::std::prelude::v1::test]
    fn selecting_an_inverse_cell_shows_its_own_colors() {
        let colors = Colors::default();
        let inverse = Cell {
            inverse: true,
            ..Cell::default()
        };
        let drawn = cell_colors(&inverse, true, &colors, &STYLE);
        assert_eq!(drawn.fg, colors.foreground);
        assert_eq!(drawn.bg, colors.background);
    }

    #[::std::prelude::v1::test]
    fn bold_palette_colors_turn_bright_only_when_asked() {
        let colors = Colors::default();
        let bold_red = Cell {
            fg: Color::Palette(1),
            bold: true,
            ..Cell::default()
        };
        assert_eq!(
            cell_colors(&bold_red, false, &colors, &STYLE).fg,
            colors.palette[9]
        );
        let plain = Style {
            bold_is_bright: false,
            ..STYLE
        };
        assert_eq!(
            cell_colors(&bold_red, false, &colors, &plain).fg,
            colors.palette[1]
        );
    }

    #[::std::prelude::v1::test]
    fn invisible_text_is_not_drawn_but_its_background_is() {
        let hidden = Cell {
            invisible: true,
            bg: Color::Palette(2),
            ..Cell::default()
        };
        let plan = plan_row(
            &row(vec![styled("p", hidden)]),
            None,
            &Colors::default(),
            &STYLE,
        );
        assert!(plan.pieces.is_empty());
        assert_eq!(plan.fills.len(), 1);
    }

    #[::std::prelude::v1::test]
    fn faint_text_is_drawn_translucent() {
        let faint = Cell {
            faint: true,
            ..Cell::default()
        };
        let plan = plan_row(
            &row(vec![styled("f", faint)]),
            None,
            &Colors::default(),
            &STYLE,
        );
        assert_eq!(plan.pieces[0].style.alpha, 0.5);
    }

    #[::std::prelude::v1::test]
    fn underlines_join_across_cells_and_take_their_own_color() {
        let colors = Colors::default();
        let underlined = Cell {
            underline: Underline::Curly,
            underline_color: Color::Rgb { r: 255, g: 0, b: 0 },
            ..Cell::default()
        };
        let struck = Cell {
            strikethrough: true,
            ..Cell::default()
        };
        let cells = vec![
            styled("a", underlined.clone()),
            styled("b", underlined),
            styled("c", struck),
        ];
        let plan = plan_row(&row(cells), None, &colors, &STYLE);
        assert_eq!(
            plan.decorations,
            vec![
                Decoration {
                    col: 0,
                    cols: 2,
                    kind: LineKind::Underline(Underline::Curly),
                    color: Rgb { r: 255, g: 0, b: 0 },
                    alpha: 1.0
                },
                Decoration {
                    col: 2,
                    cols: 1,
                    kind: LineKind::Strikethrough,
                    color: colors.foreground,
                    alpha: 1.0
                },
            ]
        );
    }
}

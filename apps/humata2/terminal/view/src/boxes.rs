//! Box-drawing and block characters, drawn as rectangles rather than taken
//! from the font.
//!
//! A font's `┌` and its `│` needn't put their strokes on the same pixels,
//! so boxes drawn from glyphs show steps at the corners and gaps between
//! rows. Like `Ghostty` (`font/sprite/draw/box.zig`), the view draws the
//! lines of U+2500–U+257F and the blocks of U+2580–U+259F itself, in device
//! pixels, so every stroke in a column lands on the same pixels whatever
//! the character, and arms reach the cell's edges to meet their
//! neighbours. The diagonals (`╱╲╳`) are left to the font. Rounded corners
//! (`╭╮╯╰`) are drawn square.
//!
//! Plain data, no `GPUI`: the view hands in the cell's box in device
//! pixels and paints the rectangles that come back.

/// How heavy one arm of a box-drawing character is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Weight {
    None,
    Light,
    Heavy,
    Double,
}

/// The four arms of a box-drawing character, from the cell's middle to its
/// edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Arms {
    pub up: Weight,
    pub right: Weight,
    pub down: Weight,
    pub left: Weight,
}

/// What a character is drawn as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Drawing {
    Lines(Arms),
    /// A line broken into `count` dashes.
    Dashes { vertical: bool, heavy: bool, count: u8 },
    /// A solid part of the cell, its edges in eighths of the cell.
    Block { left: u8, top: u8, right: u8, bottom: u8 },
    /// The whole cell in the foreground at this many quarters of its
    /// strength: `░▒▓`.
    Shade(u8),
    /// Quarters of the cell: 1 upper left, 2 upper right, 4 lower left,
    /// 8 lower right.
    Quadrants(u8),
}

/// A cell's box in device pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Area {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

/// A rectangle to fill, in device pixels, at `alpha` of the foreground.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub alpha: f32,
}

/// The arms of U+2500–U+257F, four letters each, up, right, down, left:
/// `L` light, `H` heavy, `D` double, `0` none. `-` marks the dashed lines,
/// the diagonals and anything else not drawn from arms.
const ARMS: [&str; 128] = [
    // ─ ━ │ ┃ ┄ ┅ ┆ ┇ ┈ ┉ ┊ ┋ ┌ ┍ ┎ ┏
    "0L0L", "0H0H", "L0L0", "H0H0", "-", "-", "-", "-", "-", "-", "-", "-", "0LL0", "0HL0",
    "0LH0", "0HH0",
    // ┐ ┑ ┒ ┓ └ ┕ ┖ ┗ ┘ ┙ ┚ ┛ ├ ┝ ┞ ┟
    "00LL", "00LH", "00HL", "00HH", "LL00", "LH00", "HL00", "HH00", "L00L", "L00H", "H00L",
    "H00H", "LLL0", "LHL0", "HLL0", "LLH0",
    // ┠ ┡ ┢ ┣ ┤ ┥ ┦ ┧ ┨ ┩ ┪ ┫ ┬ ┭ ┮ ┯
    "HLH0", "HHL0", "LHH0", "HHH0", "L0LL", "L0LH", "H0LL", "L0HL", "H0HL", "H0LH", "L0HH",
    "H0HH", "0LLL", "0LLH", "0HLL", "0HLH",
    // ┰ ┱ ┲ ┳ ┴ ┵ ┶ ┷ ┸ ┹ ┺ ┻ ┼ ┽ ┾ ┿
    "0LHL", "0LHH", "0HHL", "0HHH", "LL0L", "LL0H", "LH0L", "LH0H", "HL0L", "HL0H", "HH0L",
    "HH0H", "LLLL", "LLLH", "LHLL", "LHLH",
    // ╀ ╁ ╂ ╃ ╄ ╅ ╆ ╇ ╈ ╉ ╊ ╋ ╌ ╍ ╎ ╏
    "HLLL", "LLHL", "HLHL", "HLLH", "HHLL", "LLHH", "LHHL", "HHLH", "LHHH", "HLHH", "HHHL",
    "HHHH", "-", "-", "-", "-",
    // ═ ║ ╒ ╓ ╔ ╕ ╖ ╗ ╘ ╙ ╚ ╛ ╜ ╝ ╞ ╟
    "0D0D", "D0D0", "0DL0", "0LD0", "0DD0", "00LD", "00DL", "00DD", "LD00", "DL00", "DD00",
    "L00D", "D00L", "D00D", "LDL0", "DLD0",
    // ╠ ╡ ╢ ╣ ╤ ╥ ╦ ╧ ╨ ╩ ╪ ╫ ╬ ╭ ╮ ╯
    "DDD0", "L0LD", "D0DL", "D0DD", "0DLD", "0LDL", "0DDD", "LD0D", "DL0L", "DD0D", "LDLD",
    "DLDL", "DDDD", "0LL0", "00LL", "L00L",
    // ╰ ╱ ╲ ╳ ╴ ╵ ╶ ╷ ╸ ╹ ╺ ╻ ╼ ╽ ╾ ╿
    "LL00", "-", "-", "-", "000L", "L000", "0L00", "00L0", "000H", "H000", "0H00", "00H0",
    "0H0L", "L0H0", "0L0H", "H0L0",
];

/// How `c` is drawn, or `None` when the font draws it.
pub(crate) fn drawing(c: char) -> Option<Drawing> {
    let code = u32::from(c);
    match code {
        0x2500..=0x257f => lines(code),
        0x2580..=0x259f => block(code),
        _ => None,
    }
}

fn lines(code: u32) -> Option<Drawing> {
    let dashes = |vertical, heavy, count| Some(Drawing::Dashes { vertical, heavy, count });
    match code {
        0x2504 => dashes(false, false, 3),
        0x2505 => dashes(false, true, 3),
        0x2506 => dashes(true, false, 3),
        0x2507 => dashes(true, true, 3),
        0x2508 => dashes(false, false, 4),
        0x2509 => dashes(false, true, 4),
        0x250a => dashes(true, false, 4),
        0x250b => dashes(true, true, 4),
        0x254c => dashes(false, false, 2),
        0x254d => dashes(false, true, 2),
        0x254e => dashes(true, false, 2),
        0x254f => dashes(true, true, 2),
        _ => {
            let entry = ARMS.get(usize::try_from(code - 0x2500).ok()?)?.as_bytes();
            let weight = |index: usize| match entry.get(index) {
                Some(b'L') => Some(Weight::Light),
                Some(b'H') => Some(Weight::Heavy),
                Some(b'D') => Some(Weight::Double),
                Some(b'0') => Some(Weight::None),
                _ => None,
            };
            Some(Drawing::Lines(Arms {
                up: weight(0)?,
                right: weight(1)?,
                down: weight(2)?,
                left: weight(3)?,
            }))
        }
    }
}

fn block(code: u32) -> Option<Drawing> {
    let block = |left, top, right, bottom| Some(Drawing::Block { left, top, right, bottom });
    match code {
        // ▀ upper half
        0x2580 => block(0, 0, 8, 4),
        // ▁▂▃▄▅▆▇█ lower eighths, up to the full block
        0x2581..=0x2588 => block(0, 8 - (code - 0x2580) as u8, 8, 8),
        // ▉▊▋▌▍▎▏ left eighths, from seven down to one
        0x2589..=0x258f => block(0, 0, 8 - (code - 0x2588) as u8, 8),
        // ▐ right half
        0x2590 => block(4, 0, 8, 8),
        // ░▒▓
        0x2591..=0x2593 => Some(Drawing::Shade((code - 0x2590) as u8)),
        // ▔ upper eighth, ▕ right eighth
        0x2594 => block(0, 0, 8, 1),
        0x2595 => block(7, 0, 8, 8),
        0x2596 => Some(Drawing::Quadrants(4)),
        0x2597 => Some(Drawing::Quadrants(8)),
        0x2598 => Some(Drawing::Quadrants(1)),
        0x2599 => Some(Drawing::Quadrants(1 | 4 | 8)),
        0x259a => Some(Drawing::Quadrants(1 | 8)),
        0x259b => Some(Drawing::Quadrants(1 | 2 | 4)),
        0x259c => Some(Drawing::Quadrants(1 | 2 | 8)),
        0x259d => Some(Drawing::Quadrants(2)),
        0x259e => Some(Drawing::Quadrants(2 | 4)),
        0x259f => Some(Drawing::Quadrants(2 | 4 | 8)),
        _ => None,
    }
}

/// The rectangles that draw `drawing` in `area`, with light lines `light`
/// device pixels thick. Every edge is a whole device pixel.
pub(crate) fn rects(drawing: Drawing, area: Area, light: f32) -> Vec<Rect> {
    let area = Area {
        left: area.left.round(),
        top: area.top.round(),
        right: area.right.round(),
        bottom: area.bottom.round(),
    };
    let light = light.round().max(1.0);
    match drawing {
        Drawing::Lines(arms) => arm_rects(arms, area, light),
        Drawing::Dashes { vertical, heavy, count } => dash_rects(vertical, heavy, count, area, light),
        Drawing::Block { left, top, right, bottom } => {
            let x = |eighths: u8| eighth(area.left, area.right, eighths);
            let y = |eighths: u8| eighth(area.top, area.bottom, eighths);
            vec![solid(x(left), y(top), x(right), y(bottom), 1.0)]
        }
        Drawing::Shade(quarters) => vec![solid(
            area.left,
            area.top,
            area.right,
            area.bottom,
            f32::from(quarters) / 4.0,
        )],
        Drawing::Quadrants(mask) => {
            let x = eighth(area.left, area.right, 4);
            let y = eighth(area.top, area.bottom, 4);
            [
                (1, area.left, area.top, x, y),
                (2, x, area.top, area.right, y),
                (4, area.left, y, x, area.bottom),
                (8, x, y, area.right, area.bottom),
            ]
            .into_iter()
            .filter(|(bit, ..)| mask & bit != 0)
            .map(|(_, left, top, right, bottom)| solid(left, top, right, bottom, 1.0))
            .collect()
        }
    }
}

fn eighth(from: f32, to: f32, eighths: u8) -> f32 {
    (from + (to - from) * f32::from(eighths) / 8.0).round()
}

fn solid(left: f32, top: f32, right: f32, bottom: f32, alpha: f32) -> Rect {
    Rect {
        left,
        top,
        right,
        bottom,
        alpha,
    }
}

/// Where the strokes of a `weight` line sit across its axis, as start and
/// end, for a line whose light stroke starts at `start`: a light stroke at
/// `start`, a heavy one twice as thick around it, a double one either side
/// of it with a light stroke's gap between.
fn strokes(weight: Weight, start: f32, light: f32) -> Vec<(f32, f32)> {
    match weight {
        Weight::None => Vec::new(),
        Weight::Light => vec![(start, start + light)],
        Weight::Heavy => {
            let half = (light / 2.0).floor();
            vec![(start + half - light, start + half + light)]
        }
        Weight::Double => vec![(start - light, start), (start + light, start + 2.0 * light)],
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dir {
    Up,
    Right,
    Down,
    Left,
}

fn arm_rects(arms: Arms, area: Area, light: f32) -> Vec<Rect> {
    let half = (light / 2.0).floor();
    // Where a light vertical and a light horizontal stroke start: the
    // same for every character, so strokes line up from cell to cell.
    let start_x = ((area.left + area.right) / 2.0).floor() - half;
    let start_y = ((area.top + area.bottom) / 2.0).floor() - half;

    let mut rects = Vec::new();
    for dir in [Dir::Up, Dir::Right, Dir::Down, Dir::Left] {
        let (weight, opposite, low, high) = match dir {
            Dir::Up => (arms.up, arms.down, arms.left, arms.right),
            Dir::Down => (arms.down, arms.up, arms.left, arms.right),
            Dir::Right => (arms.right, arms.left, arms.up, arms.down),
            Dir::Left => (arms.left, arms.right, arms.up, arms.down),
        };
        if weight == Weight::None {
            continue;
        }
        let horizontal = matches!(dir, Dir::Right | Dir::Left);
        // Along the arm and across it.
        let (along, across) = if horizontal { (start_x, start_y) } else { (start_y, start_x) };
        // The arm runs from the middle towards the edge, increasing
        // (right, down) or decreasing (left, up).
        let increasing = matches!(dir, Dir::Right | Dir::Down);
        let edge = match dir {
            Dir::Up => area.top,
            Dir::Right => area.right,
            Dir::Down => area.bottom,
            Dir::Left => area.left,
        };
        let crossing: Vec<(f32, f32)> =
            strokes(low, along, light).into_iter().chain(strokes(high, along, light)).collect();
        // The far side of the crossing strokes, so the arm covers the
        // corner; with none, through the middle.
        let far = if increasing {
            crossing.iter().map(|(from, _)| *from).reduce(f32::min).unwrap_or(match weight {
                Weight::Double => along - light,
                _ => along,
            })
        } else {
            crossing.iter().map(|(_, to)| *to).reduce(f32::max).unwrap_or(match weight {
                Weight::Double => along + 2.0 * light,
                _ => along + light,
            })
        };
        // The near stroke of a double line crossing the arm, where an arm
        // that stops at it ends.
        let near = if increasing { along + light } else { along };

        for (index, (from, to)) in strokes(weight, across, light).into_iter().enumerate() {
            let end = if weight == Weight::Double {
                // Each stroke of a double arm turns into the double arm on
                // its side, or runs on across anything else.
                let side = if index == 0 { low } else { high };
                if side == Weight::Double { near } else { far }
            } else if low == Weight::Double && high == Weight::Double && opposite == Weight::None {
                // A single arm meeting a double line it doesn't cross.
                near
            } else {
                far
            };
            let (start, stop) = if increasing { (end, edge) } else { (edge, end) };
            rects.push(if horizontal {
                solid(start, from, stop, to, 1.0)
            } else {
                solid(from, start, to, stop, 1.0)
            });
        }
    }
    rects
}

fn dash_rects(vertical: bool, heavy: bool, count: u8, area: Area, light: f32) -> Vec<Rect> {
    let half = (light / 2.0).floor();
    let weight = if heavy { Weight::Heavy } else { Weight::Light };
    let (from, to, across) = if vertical {
        (area.top, area.bottom, ((area.left + area.right) / 2.0).floor() - half)
    } else {
        (area.left, area.right, ((area.top + area.bottom) / 2.0).floor() - half)
    };
    let Some((across_from, across_to)) = strokes(weight, across, light).first().copied() else {
        return Vec::new();
    };
    // Each dash sits in the middle half of its share of the cell, so
    // dashes keep the same spacing across neighbouring cells.
    let share = (to - from) / f32::from(count.max(1));
    (0..count)
        .map(|index| {
            let start = (from + share * (f32::from(index) + 0.25)).round();
            let stop = (from + share * (f32::from(index) + 0.75)).round().max(start + 1.0);
            if vertical {
                solid(across_from, start, across_to, stop, 1.0)
            } else {
                solid(start, across_from, stop, across_to, 1.0)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CELL: Area = Area {
        left: 100.0,
        top: 40.0,
        right: 116.0,
        bottom: 74.0,
    };
    const LIGHT: f32 = 2.0;

    fn draw(c: char) -> Vec<Rect> {
        match drawing(c) {
            Some(drawing) => rects(drawing, CELL, LIGHT),
            None => panic!("{c} is not drawn"),
        }
    }

    /// Whether the device pixel whose top-left corner is (`x`, `y`) is
    /// painted.
    fn painted(rects: &[Rect], x: f32, y: f32) -> bool {
        rects.iter().any(|rect| rect.left <= x && x < rect.right && rect.top <= y && y < rect.bottom)
    }

    /// The painted columns of pixel row `y`, as runs of start and end.
    fn runs_across(rects: &[Rect], y: f32) -> Vec<(f32, f32)> {
        runs((CELL.left as i32..CELL.right as i32).map(|x| (x as f32, painted(rects, x as f32, y))))
    }

    /// The painted rows of pixel column `x`, as runs of start and end.
    fn runs_down(rects: &[Rect], x: f32) -> Vec<(f32, f32)> {
        runs((CELL.top as i32..CELL.bottom as i32).map(|y| (y as f32, painted(rects, x, y as f32))))
    }

    fn runs(pixels: impl Iterator<Item = (f32, bool)>) -> Vec<(f32, f32)> {
        let mut runs: Vec<(f32, f32)> = Vec::new();
        for (at, on) in pixels {
            if !on {
                continue;
            }
            match runs.last_mut() {
                Some(last) if last.1 == at => last.1 = at + 1.0,
                _ => runs.push((at, at + 1.0)),
            }
        }
        runs
    }

    #[test]
    fn letters_and_diagonals_are_left_to_the_font() {
        for c in ['a', '|', '-', '╱', '╲', '╳', '日'] {
            assert_eq!(drawing(c), None, "{c}");
        }
    }

    #[test]
    fn every_line_character_and_block_is_drawn() {
        for code in (0x2500..=0x2570).chain(0x2574..=0x259f) {
            let Some(c) = char::from_u32(code) else {
                continue;
            };
            assert!(!draw(c).is_empty(), "{c} draws nothing");
        }
    }

    #[test]
    fn vertical_strokes_land_on_the_same_pixels_whatever_the_character() {
        let bottom_row = CELL.bottom - 1.0;
        let expected = runs_across(&draw('│'), bottom_row);
        assert_eq!(expected.len(), 1);
        for c in ['┌', '┐', '┬', '├', '┤', '┼', '╷', '╭', '╮'] {
            assert_eq!(runs_across(&draw(c), bottom_row), expected, "{c}");
        }
        let top_row = CELL.top;
        for c in ['└', '┘', '┴', '├', '┤', '┼', '╵', '╯', '╰'] {
            assert_eq!(runs_across(&draw(c), top_row), expected, "{c}");
        }
    }

    #[test]
    fn horizontal_strokes_land_on_the_same_pixels_whatever_the_character() {
        let expected = runs_down(&draw('─'), CELL.left);
        assert_eq!(expected.len(), 1);
        for c in ['┐', '┘', '┬', '┴', '┤', '┼', '╴'] {
            assert_eq!(runs_down(&draw(c), CELL.left), expected, "{c}");
        }
        for c in ['┌', '└', '┬', '┴', '├', '┼', '╶'] {
            assert_eq!(runs_down(&draw(c), CELL.right - 1.0), expected, "{c}");
        }
    }

    #[test]
    fn arms_reach_the_edges_so_neighbouring_cells_join() {
        let cross = draw('┼');
        let middle_y = runs_down(&cross, CELL.left)[0].0;
        let middle_x = runs_across(&cross, CELL.top)[0].0;
        assert_eq!(runs_across(&cross, middle_y), [(CELL.left, CELL.right)]);
        assert_eq!(runs_down(&cross, middle_x), [(CELL.top, CELL.bottom)]);
    }

    #[test]
    fn a_corner_is_closed() {
        // ┌: the horizontal arm and the vertical arm meet with no notch, so
        // the stroke's outer corner pixel is painted.
        let corner = draw('┌');
        let x = runs_across(&corner, CELL.bottom - 1.0)[0].0;
        let y = runs_down(&corner, CELL.right - 1.0)[0].0;
        assert!(painted(&corner, x, y));
        // Nothing sticks out up or to the left of the corner.
        assert!(!painted(&corner, x - 1.0, y));
        assert!(!painted(&corner, x, y - 1.0));
    }

    #[test]
    fn heavy_lines_are_twice_as_thick_as_light_ones() {
        let width = |rects: &[Rect]| -> f32 {
            runs_across(rects, CELL.top).iter().map(|(from, to)| to - from).sum()
        };
        assert_eq!(width(&draw('│')), LIGHT);
        assert_eq!(width(&draw('┃')), 2.0 * LIGHT);
        // The heavy stroke covers the light one, so mixed joins line up.
        let light = runs_across(&draw('│'), CELL.top)[0];
        let heavy = runs_across(&draw('┃'), CELL.top)[0];
        assert!(heavy.0 <= light.0 && light.1 <= heavy.1);
    }

    #[test]
    fn double_lines_are_two_strokes_with_a_gap_and_join_their_neighbours() {
        let lines = runs_across(&draw('║'), CELL.top);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].0 - lines[0].1, LIGHT);
        for c in ['╔', '╗', '╦', '╠', '╣', '╬'] {
            assert_eq!(runs_across(&draw(c), CELL.bottom - 1.0), lines, "{c}");
        }
        // ╬ is four corners: nothing crosses the gap between the strokes,
        // either way.
        let crossing = draw('╬');
        let gap_x = lines[0].1;
        let gap_y = runs_down(&draw('═'), CELL.left)[0].1;
        assert!(runs_down(&crossing, gap_x).is_empty());
        assert!(runs_across(&crossing, gap_y).is_empty());
        assert_eq!(runs_down(&crossing, lines[0].0).len(), 2);
    }

    #[test]
    fn a_double_corner_draws_an_outer_and_an_inner_corner() {
        // ╔: the outer stroke turns from the top line into the left line,
        // the inner one from the lower line into the right line, and the
        // gap between them stays open down to the corner.
        let corner = draw('╔');
        let columns = runs_across(&corner, CELL.bottom - 1.0);
        let rows = runs_down(&corner, CELL.right - 1.0);
        let (outer_x, inner_x) = (columns[0].0, columns[1].0);
        let (outer_y, inner_y) = (rows[0].0, rows[1].0);
        assert!(painted(&corner, outer_x, outer_y));
        assert!(painted(&corner, inner_x, inner_y));
        assert!(!painted(&corner, inner_x, outer_y + LIGHT));
        assert!(!painted(&corner, outer_x - 1.0, outer_y));
    }

    #[test]
    fn a_single_line_crosses_a_double_one_but_stops_at_one_it_meets() {
        // ╪ crosses straight through; ╤ stops at the lower stroke.
        let middle_x = runs_across(&draw('│'), CELL.top)[0].0;
        assert_eq!(runs_down(&draw('╪'), middle_x), [(CELL.top, CELL.bottom)]);
        let tee = draw('╤');
        let strokes = runs_down(&draw('═'), CELL.left);
        let down = runs_down(&tee, middle_x);
        assert_eq!(down.len(), 2, "the upper stroke and the arm from the lower stroke down");
        assert_eq!(down[1], (strokes[1].0, CELL.bottom));
    }

    #[test]
    fn dashes_are_evenly_spaced_and_stay_inside_the_cell() {
        let middle_y = runs_down(&draw('─'), CELL.left)[0].0;
        for (c, count) in [('┄', 3), ('┈', 4), ('╌', 2)] {
            let dashes = runs_across(&draw(c), middle_y);
            assert_eq!(dashes.len(), count, "{c}");
            assert!(dashes.iter().all(|(from, to)| CELL.left < *from && *to < CELL.right), "{c}");
        }
        assert_eq!(runs_down(&draw('┆'), runs_across(&draw('│'), CELL.top)[0].0).len(), 3);
    }

    #[test]
    fn blocks_cover_their_part_of_the_cell() {
        let full = draw('█');
        assert_eq!(full, [solid(CELL.left, CELL.top, CELL.right, CELL.bottom, 1.0)]);
        let lower = draw('▄');
        let middle = (CELL.top + CELL.bottom) / 2.0;
        assert!(!painted(&lower, CELL.left, middle - 1.0));
        assert!(painted(&lower, CELL.left, middle));
        let left = draw('▌');
        let centre = (CELL.left + CELL.right) / 2.0;
        assert!(painted(&left, centre - 1.0, CELL.top));
        assert!(!painted(&left, centre, CELL.top));
        // ▁ to ▇ grow upwards an eighth at a time.
        let heights: Vec<f32> = ('▁'..='█')
            .map(|c| runs_down(&draw(c), CELL.left).iter().map(|(from, to)| to - from).sum())
            .collect();
        assert!(heights.windows(2).all(|pair| pair[0] < pair[1]), "{heights:?}");
    }

    #[test]
    fn shades_fill_the_cell_translucently_and_quadrants_fill_their_corners() {
        let alphas: Vec<f32> = ['░', '▒', '▓'].into_iter().map(|c| draw(c)[0].alpha).collect();
        assert_eq!(alphas, [0.25, 0.5, 0.75]);
        let diagonal = draw('▚');
        assert!(painted(&diagonal, CELL.left, CELL.top));
        assert!(painted(&diagonal, CELL.right - 1.0, CELL.bottom - 1.0));
        assert!(!painted(&diagonal, CELL.right - 1.0, CELL.top));
        assert!(!painted(&diagonal, CELL.left, CELL.bottom - 1.0));
    }

    #[test]
    fn fractional_cells_snap_to_whole_pixels() {
        let area = Area {
            left: 100.4,
            top: 40.6,
            right: 116.4,
            bottom: 74.6,
        };
        for c in ['┼', '╬', '▄', '┄'] {
            let Some(drawing) = drawing(c) else {
                panic!("{c} is not drawn");
            };
            for rect in rects(drawing, area, 1.6) {
                for edge in [rect.left, rect.top, rect.right, rect.bottom] {
                    assert_eq!(edge, edge.round(), "{c}");
                }
            }
        }
    }
}

//! Draw mode's painting and geometry (D159): the red marks, their number
//! badges and the pill of tools. The view in `report_view` owns the state;
//! everything here is a function of what it is given.

use gpui_kit::{prelude::*, *};
use humata2_app::Point as Mark;

/// The one stroke colour, in both themes.
pub const RED: u32 = 0xff453a;

/// A stroke's line width.
const WIDTH: f32 = 3.;
/// A dot's radius.
const DOT: f32 = 4.;
/// A path shorter than this is a dot.
const MIN_LENGTH: f32 = 6.;
/// A badge's size.
pub const BADGE: f32 = 16.;
/// Points closer than this to the last painted one are skipped.
const STEP: f32 = 2.;

/// `position` in whole window points.
#[allow(clippy::cast_possible_truncation)]
pub fn mark(position: Point<Pixels>) -> Mark {
    Mark {
        x: f32::from(position.x).round() as i32,
        y: f32::from(position.y).round() as i32,
    }
}

/// The path's length in points.
#[allow(clippy::cast_precision_loss)]
pub fn length(points: &[Mark]) -> f32 {
    points
        .windows(2)
        .map(|pair| {
            let dx = (pair[1].x - pair[0].x) as f32;
            let dy = (pair[1].y - pair[0].y) as f32;
            dx.hypot(dy)
        })
        .sum()
}

/// Whether the path paints as a dot.
pub fn is_dot(points: &[Mark]) -> bool {
    length(points) < MIN_LENGTH
}

/// Where a badge for a stroke starting at `first` sits (its top-left),
/// centred on the point but kept inside a window of `window` points.
#[allow(clippy::cast_precision_loss)]
pub fn badge_origin(first: Mark, window: (f32, f32)) -> (f32, f32) {
    let place = |at: i32, extent: f32| {
        let room = (extent - BADGE).max(0.);
        (at as f32 - BADGE / 2.).clamp(0., room)
    };
    (place(first.x, window.0), place(first.y, window.1))
}

#[allow(clippy::cast_precision_loss)]
fn at(mark: Mark) -> (f32, f32) {
    (mark.x as f32, mark.y as f32)
}

fn red() -> Hsla {
    rgb(RED).into()
}

fn disc(window: &mut Window, centre: (f32, f32), radius: f32) {
    let bounds = Bounds::new(
        point(px(centre.0 - radius), px(centre.1 - radius)),
        size(px(2. * radius), px(2. * radius)),
    );
    window.paint_quad(quad(
        bounds,
        Corners::all(px(radius)),
        red(),
        Edges::default(),
        transparent_black(),
        BorderStyle::default(),
    ));
}

/// Paints one stroke: a dot when it is short, else the line with round
/// ends and corners (the path tessellator can't name a round cap here, so
/// discs at the kept points round them).
fn paint_stroke(window: &mut Window, points: &[Mark]) {
    let Some(first) = points.first() else { return };
    if is_dot(points) {
        disc(window, at(*first), DOT);
        return;
    }
    let mut kept: Vec<(f32, f32)> = Vec::with_capacity(points.len());
    for mark in points {
        let here = at(*mark);
        let far = kept
            .last()
            .is_none_or(|last| (here.0 - last.0).hypot(here.1 - last.1) >= STEP);
        if far {
            kept.push(here);
        }
    }
    if let Some(last) = points.last().map(|mark| at(*mark))
        && kept.last() != Some(&last)
    {
        kept.push(last);
    }
    let mut path = PathBuilder::stroke(px(WIDTH));
    for (index, (x, y)) in kept.iter().enumerate() {
        if index == 0 {
            path.move_to(point(px(*x), px(*y)));
        } else {
            path.line_to(point(px(*x), px(*y)));
        }
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, red());
    }
    for centre in kept {
        disc(window, centre, WIDTH / 2.);
    }
}

/// The layer's paint: every finished stroke, then the one being drawn.
pub fn strokes(finished: Vec<Vec<Mark>>, live: Vec<Mark>) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |_, (), window, _| {
            for points in &finished {
                paint_stroke(window, points);
            }
            paint_stroke(window, &live);
        },
    )
    .absolute()
    .size_full()
}

/// The numbered badge of a finished stroke, at its first point.
pub fn badge(number: usize, first: Mark, window: (f32, f32)) -> Div {
    let (left, top) = badge_origin(first, window);
    div()
        .absolute()
        .left(px(left))
        .top(px(top))
        .size(px(BADGE))
        .rounded_full()
        .bg(rgb(RED))
        .text_color(rgb(0xffffff))
        .text_size(px(10.))
        .font_weight(FontWeight::SEMIBOLD)
        .flex()
        .items_center()
        .justify_center()
        .child(number.to_string())
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui prelude's `test` attribute would shadow the standard one.
    use super::{Mark, badge_origin, is_dot};

    fn at(x: i32, y: i32) -> Mark {
        Mark { x, y }
    }

    #[test]
    fn badge_stays_inside_the_window() {
        assert_eq!(badge_origin(at(0, 0), (800., 600.)), (0., 0.));
        assert_eq!(badge_origin(at(800, 600), (800., 600.)), (784., 584.));
        assert_eq!(badge_origin(at(100, 200), (800., 600.)), (92., 192.));
        assert_eq!(badge_origin(at(-30, 9000), (800., 600.)), (0., 584.));
    }

    #[test]
    fn a_short_path_is_a_dot() {
        assert!(is_dot(&[at(10, 10)]));
        assert!(is_dot(&[at(10, 10), at(13, 12)]));
        assert!(!is_dot(&[at(10, 10), at(30, 10)]));
    }
}

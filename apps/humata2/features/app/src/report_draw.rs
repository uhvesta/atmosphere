//! Turning a drawn path into a shape and the elements it marks.
use crate::report::{PinTarget, Point, Rect, Shape};

/// A path shorter than this many points of length is a dot.
const DOT_LENGTH: f64 = 6.0;
/// A loop must have at least this many points.
const LOOP_POINTS: usize = 8;
/// The least distance, in points, between a loop's ends that still closes it.
const LOOP_GAP: f64 = 24.0;
/// How many covered elements a stroke lists.
const LIST_LIMIT: usize = 5;

fn distance(a: Point, b: Point) -> f64 {
    let dx = f64::from(a.x) - f64::from(b.x);
    let dy = f64::from(a.y) - f64::from(b.y);
    dx.hypot(dy)
}

fn bounds(points: &[Point]) -> Option<Rect> {
    let first = points.first()?;
    let (mut x0, mut y0, mut x1, mut y1) = (first.x, first.y, first.x, first.y);
    for point in points {
        x0 = x0.min(point.x);
        y0 = y0.min(point.y);
        x1 = x1.max(point.x);
        y1 = y1.max(point.y);
    }
    Some(Rect {
        x: x0,
        y: y0,
        width: x1 - x0,
        height: y1 - y0,
    })
}

/// What a path is: a dot, a loop or a line.
pub(crate) fn classify(points: &[Point]) -> Shape {
    let length: f64 = points.windows(2).map(|pair| distance(pair[0], pair[1])).sum();
    if length < DOT_LENGTH {
        return Shape::Dot;
    }
    let (Some(first), Some(last), Some(bounds)) = (points.first(), points.last(), bounds(points))
    else {
        return Shape::Dot;
    };
    let diagonal = f64::from(bounds.width).hypot(f64::from(bounds.height));
    if points.len() >= LOOP_POINTS && distance(*first, *last) <= LOOP_GAP.max(diagonal / 4.0) {
        Shape::Loop
    } else {
        Shape::Line
    }
}

fn area(rect: Rect) -> i64 {
    i64::from(rect.width) * i64::from(rect.height)
}

fn within(inner: Rect, outer: Rect) -> bool {
    inner.x >= outer.x
        && inner.y >= outer.y
        && i64::from(inner.x) + i64::from(inner.width) <= i64::from(outer.x) + i64::from(outer.width)
        && i64::from(inner.y) + i64::from(inner.height)
            <= i64::from(outer.y) + i64::from(outer.height)
}

/// Even-odd point in polygon.
fn inside(polygon: &[Point], point: Point) -> bool {
    let (px, py) = (f64::from(point.x), f64::from(point.y));
    let mut inside = false;
    let mut previous = match polygon.last() {
        Some(last) => *last,
        None => return false,
    };
    for current in polygon {
        let (ax, ay) = (f64::from(current.x), f64::from(current.y));
        let (bx, by) = (f64::from(previous.x), f64::from(previous.y));
        if (ay > py) != (by > py) && px < (bx - ax) * (py - ay) / (by - ay) + ax {
            inside = !inside;
        }
        previous = *current;
    }
    inside
}

/// Index of the smallest target containing `point`; the first of equals.
fn smallest_at(targets: &[(&PinTarget, Rect)], point: Point) -> Option<usize> {
    let mut best: Option<usize> = None;
    for (index, (_, rect)) in targets.iter().enumerate() {
        if rect.contains(point) && best.is_none_or(|b| area(*rect) < area(targets[b].1)) {
            best = Some(index);
        }
    }
    best
}

/// The elements a stroke marks, at most five, and how many more it marks.
pub(crate) fn covers(
    shape: Shape,
    points: &[Point],
    targets: &[(&PinTarget, Rect)],
) -> (Vec<PinTarget>, usize) {
    let mut found: Vec<usize> = Vec::new();
    match shape {
        Shape::Dot => found.extend(points.first().and_then(|p| smallest_at(targets, *p))),
        Shape::Line => {
            for point in points {
                if let Some(index) = smallest_at(targets, *point)
                    && !found.contains(&index)
                {
                    found.push(index);
                }
            }
        }
        Shape::Loop => {
            let mut inner: Vec<usize> = (0..targets.len())
                .filter(|&i| inside(points, targets[i].1.centre()))
                .collect();
            inner.sort_by_key(|&i| std::cmp::Reverse(area(targets[i].1)));
            let mut kept: Vec<usize> = Vec::new();
            for index in inner {
                if !kept.iter().any(|&k| within(targets[index].1, targets[k].1)) {
                    kept.push(index);
                }
            }
            kept.sort_unstable();
            found = kept;
            if found.is_empty() && !points.is_empty() {
                let count = i64::try_from(points.len()).unwrap_or(i64::MAX);
                let sum_x: i64 = points.iter().map(|p| i64::from(p.x)).sum();
                let sum_y: i64 = points.iter().map(|p| i64::from(p.y)).sum();
                let centroid = Point {
                    x: i32::try_from(sum_x / count).unwrap_or_default(),
                    y: i32::try_from(sum_y / count).unwrap_or_default(),
                };
                found.extend(smallest_at(targets, centroid));
            }
        }
    }
    let more = found.len().saturating_sub(LIST_LIMIT);
    let listed = found
        .into_iter()
        .take(LIST_LIMIT)
        .map(|i| targets[i].0.clone())
        .collect();
    (listed, more)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(role: &str, label: &str) -> PinTarget {
        PinTarget {
            role: role.to_owned(),
            label: label.to_owned(),
            within: Vec::new(),
        }
    }

    fn rect(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect { x, y, width, height }
    }

    fn p(x: i32, y: i32) -> Point {
        Point { x, y }
    }

    /// A closed square path around the given box, 16 points.
    fn around(x: i32, y: i32, w: i32, h: i32) -> Vec<Point> {
        let mut points = Vec::new();
        for i in 0..4 {
            points.push(p(x + w * i / 4, y));
        }
        for i in 0..4 {
            points.push(p(x + w, y + h * i / 4));
        }
        for i in 0..4 {
            points.push(p(x + w - w * i / 4, y + h));
        }
        for i in 0..4 {
            points.push(p(x, y + h - h * i / 4));
        }
        points.push(p(x, y));
        points
    }

    #[test]
    fn short_paths_are_dots_closed_paths_loops_and_the_rest_lines() {
        assert_eq!(classify(&[p(5, 5), p(6, 6)]), Shape::Dot);
        assert_eq!(classify(&around(100, 100, 200, 100)), Shape::Loop);
        let line: Vec<Point> = (0..20).map(|i| p(i * 20, 50)).collect();
        assert_eq!(classify(&line), Shape::Line);
    }

    #[test]
    fn a_loop_around_a_row_covers_the_row_not_its_label() {
        let row = target("Row", "Build");
        let label = target("Label", "Build");
        let other = target("Row", "Test");
        let targets = [
            (&row, rect(100, 100, 200, 40)),
            (&label, rect(110, 110, 60, 20)),
            (&other, rect(100, 200, 200, 40)),
        ];
        let path = around(90, 90, 220, 60);
        let (found, more) = covers(Shape::Loop, &path, &targets);
        assert_eq!(found, vec![row]);
        assert_eq!(more, 0);
    }

    #[test]
    fn a_loop_around_a_list_covers_the_list() {
        let list = target("List", "Jobs");
        let a = target("Row", "A");
        let b = target("Row", "B");
        let targets = [
            (&list, rect(100, 100, 200, 100)),
            (&a, rect(100, 100, 200, 40)),
            (&b, rect(100, 150, 200, 40)),
        ];
        let path = around(80, 80, 240, 140);
        let (found, _) = covers(Shape::Loop, &path, &targets);
        assert_eq!(found, vec![list]);
    }

    #[test]
    fn a_click_becomes_a_dot_on_the_smallest_element_under_it() {
        let panel = target("Panel", "Side");
        let button = target("Button", "Add");
        let targets = [
            (&panel, rect(0, 0, 300, 300)),
            (&button, rect(10, 10, 50, 20)),
        ];
        let path = [p(20, 20), p(20, 21)];
        assert_eq!(classify(&path), Shape::Dot);
        let (found, _) = covers(Shape::Dot, &path, &targets);
        assert_eq!(found, vec![button]);
    }

    #[test]
    fn a_line_across_three_rows_covers_them_in_order() {
        let rows = [target("Row", "A"), target("Row", "B"), target("Row", "C")];
        let targets = [
            (&rows[0], rect(0, 0, 100, 30)),
            (&rows[1], rect(0, 30, 100, 30)),
            (&rows[2], rect(0, 60, 100, 30)),
        ];
        let path: Vec<Point> = (0..9).map(|i| p(50, 5 + i * 10)).collect();
        let (found, more) = covers(Shape::Line, &path, &targets);
        assert_eq!(found, rows.to_vec());
        assert_eq!(more, 0);
    }

    #[test]
    fn more_than_five_covered_elements_are_capped_with_the_count() {
        let rows: Vec<PinTarget> = (0..8).map(|i| target("Row", &i.to_string())).collect();
        let targets: Vec<(&PinTarget, Rect)> = rows
            .iter()
            .enumerate()
            .map(|(i, t)| (t, rect(0, i as i32 * 10, 100, 10)))
            .collect();
        let path: Vec<Point> = (0..80).map(|i| p(50, i)).collect();
        let (found, more) = covers(Shape::Line, &path, &targets);
        assert_eq!(found, rows[..5].to_vec());
        assert_eq!(more, 3);
    }
}

//! The pictures a report can hold: `window.png`, the window as the engine
//! drew it, and `marks.svg`, the person's drawings and pinned comments laid
//! over it. Nothing here asks the OS for a screenshot.

use std::fmt::Write as _;
use std::io::Cursor;
use std::path::Path;

use image::{ImageFormat, Rgba, RgbaImage};

use crate::error::PictureError;
use crate::form::{Bounds, Form, Point, Shape};
use crate::write::{self, WINDOW};

/// What a masked box is filled with.
const MASK: Rgba<u8> = Rgba([0x30, 0x30, 0x30, 0xff]);

/// The red of every mark.
const RED: &str = "#ff453a";

/// The longest comment label drawn on the overlay, in characters.
const LABEL: usize = 40;

/// A box in the picture's pixels, which are the window's points times its
/// scale factor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PixelBox {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Fills each of `boxes` with an opaque dark grey, the part outside
/// `image` cut off. Used to hide secrets before a picture is saved.
pub fn mask(image: &mut RgbaImage, boxes: &[PixelBox]) {
    let (columns, rows) = (i64::from(image.width()), i64::from(image.height()));
    for hidden in boxes {
        let left = i64::from(hidden.x).clamp(0, columns);
        let top = i64::from(hidden.y).clamp(0, rows);
        let right = (i64::from(hidden.x) + i64::from(hidden.width)).clamp(0, columns);
        let bottom = (i64::from(hidden.y) + i64::from(hidden.height)).clamp(0, rows);
        for y in top..bottom {
            for x in left..right {
                if let (Ok(x), Ok(y)) = (u32::try_from(x), u32::try_from(y)) {
                    image.put_pixel(x, y, MASK);
                }
            }
        }
    }
}

/// Saves `image` as `window.png` in the report folder `dir`, readable only
/// by this user, replacing an earlier one whole.
///
/// # Errors
///
/// [`PictureError`] when it can't be encoded or written; the earlier
/// `window.png`, if any, is kept.
pub fn write_window(dir: &Path, image: &RgbaImage) -> Result<(), PictureError> {
    let mut bytes = Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, ImageFormat::Png)
        .map_err(|source| PictureError::Encode { source })?;
    write::replace_file(dir, WINDOW.0, &bytes.into_inner()).map_err(|source| PictureError::Write {
        path: dir.join(WINDOW.0),
        source,
    })
}

/// The overlay for `form` over `window.png` (when `with_picture`), or
/// `None` when the form has no strokes and no placed comments. Text in it
/// is already scrubbed, since `form` is.
pub(crate) fn marks_svg(form: &Form, with_picture: bool) -> Option<String> {
    let strokes = &form.drawing.strokes;
    let placed: Vec<(usize, &str, Bounds)> = form
        .comments
        .iter()
        .enumerate()
        .filter_map(|(at, comment)| comment.place.map(|place| (at + 1, comment.text.as_str(), place)))
        .collect();
    if strokes.is_empty() && placed.is_empty() {
        return None;
    }

    let (mut width, mut height) = (form.drawing.window.width, form.drawing.window.height);
    if width == 0 || height == 0 {
        // The window's size wasn't kept; take what the marks reach.
        for point in strokes.iter().flat_map(|stroke| &stroke.points) {
            width = width.max(u32::try_from(point.x.max(0)).unwrap_or(0));
            height = height.max(u32::try_from(point.y.max(0)).unwrap_or(0));
        }
        for (_, _, place) in &placed {
            width = width.max(extent(place.x, place.width));
            height = height.max(extent(place.y, place.height));
        }
    }

    let mut svg = String::new();
    let _ = writeln!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" \
         viewBox=\"0 0 {width} {height}\">"
    );
    if with_picture {
        let _ = writeln!(
            svg,
            "<image href=\"{}\" x=\"0\" y=\"0\" width=\"{width}\" height=\"{height}\"/>",
            WINDOW.0
        );
    }
    for (number, text, place) in &placed {
        let _ = writeln!(
            svg,
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"none\" stroke=\"{RED}\" \
             stroke-width=\"2\" stroke-dasharray=\"6 3\"/>",
            place.x, place.y, place.width, place.height
        );
        let _ = writeln!(
            svg,
            "<text x=\"{}\" y=\"{}\" font-family=\"sans-serif\" font-size=\"11\" \
             fill=\"{RED}\">C{number} {}</text>",
            place.x + 3,
            place.y + 12,
            escape(&short(text))
        );
    }
    for (at, stroke) in strokes.iter().enumerate() {
        match (stroke.shape, stroke.points.as_slice()) {
            (_, []) => continue,
            (Shape::Dot, [first, ..]) | (_, [first]) => {
                let _ = writeln!(
                    svg,
                    "<circle cx=\"{}\" cy=\"{}\" r=\"4\" fill=\"{RED}\"/>",
                    first.x, first.y
                );
            }
            (_, points) => {
                let points: Vec<String> = points.iter().map(|Point { x, y }| format!("{x},{y}")).collect();
                let _ = writeln!(
                    svg,
                    "<polyline points=\"{}\" fill=\"none\" stroke=\"{RED}\" stroke-width=\"3\" \
                     stroke-linecap=\"round\" stroke-linejoin=\"round\"/>",
                    points.join(" ")
                );
            }
        }
        if let Some(first) = stroke.points.first() {
            let _ = writeln!(
                svg,
                "<circle cx=\"{}\" cy=\"{}\" r=\"8\" fill=\"{RED}\"/>\n\
                 <text x=\"{}\" y=\"{}\" font-family=\"sans-serif\" font-size=\"10\" \
                 text-anchor=\"middle\" fill=\"#ffffff\">{}</text>",
                first.x,
                first.y,
                first.x,
                first.y + 4,
                at + 1
            );
        }
    }
    svg.push_str("</svg>\n");
    Some(svg)
}

fn extent(start: i32, length: u32) -> u32 {
    u32::try_from(start.max(0)).unwrap_or(0).saturating_add(length)
}

/// The first line of `text`, cut to a label's length.
fn short(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default().trim();
    let mut label: String = line.chars().take(LABEL).collect();
    if line.chars().count() > LABEL {
        label.push('…');
    }
    label
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

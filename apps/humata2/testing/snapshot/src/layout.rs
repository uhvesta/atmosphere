//! Layout snapshots: a drawn GPUI window as text, with where everything
//! sits, how big it is and how it is painted.
//!
//! The view snapshot says what a person can see and use; this says how it
//! is laid out, for comparing a layout change line by line. It holds:
//!
//! - every visible element marked with `.test_support()`, and every gpui-kit
//!   component, by role and label (`#id` when it has neither);
//! - every visible text recorded with `.test_text(..)`, as `text` (or `alt`
//!   for an alternative, such as an icon's description);
//! - every painted quad (a background or a border) and underline. GPUI
//!   paints one border as several clipped copies of one quad; those are
//!   written once. A quad that fills exactly an element's bounds is that
//!   element's paint and is written on its line; any other is a `box`.
//!
//! Each line is `x,y wxh` in whole logical pixels from the window's top
//! left, then `bg`, `border` (width and colour, `t,r,b,l` widths when the
//! edges differ, `dashed` when dashed), `radius` (`tl,tr,br,bl` when the
//! corners differ) and `clip` (the part left showing when a parent clips
//! it). Colours are `#rrggbb`, with `aa` added when translucent. A line sits
//! under the smallest element or box that contains its centre (for an
//! element or box, the smallest larger one), so text that overflows its
//! input by a pixel or two still sits under it.
//! Lines at one level are in reading order: top to bottom, then left to
//! right, then outer before inner.
//!
//! ```text
//! window 360x320, test fonts
//! box 0,0 360x320 bg #ffffff
//!   text "Shopping" 8,8 344x26
//!   Button "Decrement" 8,42 98x32 bg #ffffff border 1 #e5e5e5 radius 6
//!   TextInput "New item" 8,82 200x24
//!     text "tea" 9,83 198x26
//!   List "Items" 8,122 344x52
//!     ListItem "milk" 8,122 344x26
//!       text "milk" 16,122 328x26
//!     ListItem "eggs" 8,148 344x26 bg #2f4a72
//!       text "eggs" 16,148 328x26
//! ```
//!
//! What text measures depends on the fonts. Under `TestAppContext` every
//! glyph is the same fixed width on every OS, so the snapshot is one file
//! for all of them, `<name>.layout.txt`. Drawn with the platform's fonts,
//! as `headless_context()` does, it is kept per OS, as
//! `<name>.<os>.layout.txt`, the way image snapshots are macOS only.
//!
//! Not in it: text colour, font and size, shadows, icons and images. GPUI
//! does not expose them to tests yet; image snapshots show them.

use std::cmp::Ordering;
use std::fmt::{self, Write as _};

use gpui_kit::base::test_support::{TextSnapshot, texts};
use gpui_kit::test::{ElementSnapshot, TestQueryExt as _};
use gpui_kit::{
    Background, BorderStyle, Bounds, Corners, Edges, Hsla, PaddedBool32, Pixels, Quad, ScaledPixels,
    Underline, Window,
};

use crate::store::{Outcome, SnapshotError, Snapshots, checked};

/// A drawn window's layout, written as text. See the [module](self) for
/// what it holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    text: String,
    fonts: Fonts,
}

/// Which fonts measured the text, so which file the snapshot is kept in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fonts {
    /// GPUI's test text system: the same fixed-width glyphs on every OS.
    Test,
    /// The platform's fonts, which measure differently on each OS.
    Platform,
}

impl Layout {
    /// The snapshot's text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The snapshot file `name` is kept in.
    fn file(&self, name: &str) -> String {
        match self.fonts {
            Fonts::Test => format!("{name}.layout.txt"),
            Fonts::Platform => format!("{name}.{}.layout.txt", std::env::consts::OS),
        }
    }
}

impl fmt::Display for Layout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// The window's last drawn frame as a layout snapshot. Draw a frame first,
/// with `window.render_frame(cx)`, so it shows the current state.
pub fn capture(window: &Window) -> Layout {
    let fonts = if window.text_system().all_font_names().is_empty() {
        Fonts::Test
    } else {
        Fonts::Platform
    };
    let scale = window.scale_factor();
    let elements = window
        .elements()
        .into_iter()
        .filter(ElementSnapshot::visible)
        .map(|element| Item::plain(Kind::Element(name(&element)), Rect::logical(element.bounds())));
    let texts = texts(window)
        .into_iter()
        .filter(TextSnapshot::visible)
        .map(|text| {
            let kind = Kind::Text {
                text: text.text().to_owned(),
                alternative: text.is_alternative(),
            };
            Item::plain(kind, Rect::logical(text.bounds()))
        });
    let mut items: Vec<Item> = elements.chain(texts).filter(|item| !item.rect.is_empty()).collect();
    let paint: Vec<Paint> = window
        .painted_quads()
        .iter()
        .filter_map(|quad| Paint::of(quad, scale))
        .collect();
    merge(&mut items, paint);
    items.extend(
        window
            .painted_underlines()
            .iter()
            .filter_map(|underline| underline_item(underline, scale)),
    );
    let viewport = window.viewport_size();
    let header = format!(
        "window {}x{}, {} fonts",
        whole(viewport.width.as_f32()),
        whole(viewport.height.as_f32()),
        match fonts {
            Fonts::Test => "test",
            Fonts::Platform => std::env::consts::OS,
        }
    );
    Layout {
        text: write(&header, &items),
        fonts,
    }
}

impl Snapshots {
    /// Checks `layout` against the layout snapshot `name`, or records it.
    /// It is kept as `<name>.layout.txt`, or `<name>.<os>.layout.txt` when
    /// the platform's fonts measured its text.
    ///
    /// # Errors
    ///
    /// [`SnapshotError::Missing`] or [`SnapshotError::Mismatch`] when
    /// verifying, or an error reading or writing the files.
    pub fn check_layout(&self, name: &str, layout: &Layout) -> Result<Outcome, SnapshotError> {
        let name = checked(name)?;
        self.check_text_file(&layout.file(name), layout.text())
    }

    /// Like [`check_layout`](Self::check_layout), panicking with what
    /// failed and how to fix it.
    #[track_caller]
    pub fn assert_layout(&self, name: &str, layout: &Layout) {
        if let Err(error) = self.check_layout(name, layout) {
            panic!("{error}");
        }
    }
}

/// A rectangle in whole logical pixels. Each edge is rounded on its own, so
/// two boxes that touch still touch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl Rect {
    fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left: whole(left),
            top: whole(top),
            right: whole(right),
            bottom: whole(bottom),
        }
    }

    fn logical(bounds: Bounds<Pixels>) -> Self {
        let (x, y) = (bounds.origin.x.as_f32(), bounds.origin.y.as_f32());
        let (width, height) = (bounds.size.width.as_f32(), bounds.size.height.as_f32());
        Self::new(x, y, x + width, y + height)
    }

    fn scaled(bounds: Bounds<ScaledPixels>, scale: f32) -> Self {
        let (x, y) = (bounds.origin.x.as_f32() / scale, bounds.origin.y.as_f32() / scale);
        let width = bounds.size.width.as_f32() / scale;
        let height = bounds.size.height.as_f32() / scale;
        Self::new(x, y, x + width, y + height)
    }

    fn width(&self) -> i32 {
        self.right - self.left
    }

    fn height(&self) -> i32 {
        self.bottom - self.top
    }

    fn area(&self) -> i64 {
        i64::from(self.width().max(0)) * i64::from(self.height().max(0))
    }

    fn is_empty(&self) -> bool {
        self.width() <= 0 || self.height() <= 0
    }

    /// Whether `other`'s centre is inside, edges included.
    fn contains_centre_of(&self, other: &Self) -> bool {
        // Doubled, so a centre half way between pixels stays whole.
        let (x, y) = (other.left + other.right, other.top + other.bottom);
        2 * self.left <= x && x <= 2 * self.right && 2 * self.top <= y && y <= 2 * self.bottom
    }

    fn union(&self, other: &Self) -> Self {
        Self {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }

    fn intersection(&self, other: &Self) -> Self {
        Self {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
        }
    }
}

impl fmt::Display for Rect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{},{} {}x{}", self.left, self.top, self.width(), self.height())
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Kind {
    Element(String),
    Text { text: String, alternative: bool },
    Box,
    Underline { color: String, wavy: bool },
}

impl Kind {
    /// Outer before inner when two lines have the same bounds.
    fn rank(&self) -> u8 {
        match self {
            Self::Element(_) => 0,
            Self::Box => 1,
            Self::Text { .. } => 2,
            Self::Underline { .. } => 3,
        }
    }

    /// Whether other lines can sit under it.
    fn holds(&self) -> bool {
        matches!(self, Self::Element(_) | Self::Box)
    }
}

/// How a quad is painted, as the tokens written after its bounds.
#[derive(Clone, Debug, Default, PartialEq)]
struct Style {
    background: Option<String>,
    border: Option<String>,
    radius: Option<String>,
    clip: Option<Rect>,
}

impl fmt::Display for Style {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(background) = &self.background {
            write!(f, " bg {background}")?;
        }
        if let Some(border) = &self.border {
            write!(f, " border {border}")?;
        }
        if let Some(radius) = &self.radius {
            write!(f, " radius {radius}")?;
        }
        if let Some(clip) = &self.clip {
            write!(f, " clip {clip}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Item {
    kind: Kind,
    rect: Rect,
    style: Style,
}

impl Item {
    fn plain(kind: Kind, rect: Rect) -> Self {
        Self {
            kind,
            rect,
            style: Style::default(),
        }
    }
}

/// A painted quad: its bounds and how it looks.
#[derive(Clone, Debug, PartialEq)]
struct Paint {
    rect: Rect,
    style: Style,
}

impl Paint {
    /// `quad` in logical pixels, or `None` when it paints nothing: no
    /// background or border showing, or clipped away entirely.
    fn of(quad: &Quad, scale: f32) -> Option<Self> {
        let rect = Rect::scaled(quad.bounds, scale);
        let shown = rect.intersection(&Rect::scaled(quad.content_mask.bounds, scale));
        if rect.is_empty() || shown.is_empty() {
            return None;
        }
        let background = (!quad.background.is_transparent()).then(|| background(&quad.background));
        let border = border(&quad.border_widths, quad.border_color, quad.border_style, scale);
        if background.is_none() && border.is_none() {
            return None;
        }
        Some(Self {
            rect,
            style: Style {
                background,
                border,
                radius: radius(&quad.corner_radii, scale, &rect),
                clip: (shown != rect).then_some(shown),
            },
        })
    }
}

fn underline_item(underline: &Underline, scale: f32) -> Option<Item> {
    let rect = Rect::scaled(underline.bounds, scale);
    let shown = rect.intersection(&Rect::scaled(underline.content_mask.bounds, scale));
    if rect.is_empty() || shown.is_empty() || underline.color.is_transparent() {
        return None;
    }
    let kind = Kind::Underline {
        color: hex(underline.color),
        wavy: underline.wavy == PaddedBool32::from(true),
    };
    Some(Item {
        kind,
        rect,
        style: Style {
            clip: (shown != rect).then_some(shown),
            ..Style::default()
        },
    })
}

/// Puts each quad that fills exactly an element's bounds on that element's
/// line, and adds the rest as boxes. A quad joins an element or box with
/// the same bounds when what each says doesn't contradict the other, such
/// as a background and then a border with the same radius.
fn merge(items: &mut Vec<Item>, paint: Vec<Paint>) {
    for paint in fold(paint) {
        let joined = items.iter_mut().find_map(|item| {
            if !matches!(item.kind, Kind::Element(_) | Kind::Box) || item.rect != paint.rect {
                return None;
            }
            let style = combined(&item.style, &paint.style)?;
            Some((item, style))
        });
        match joined {
            Some((item, style)) => item.style = style,
            None => items.push(Item {
                kind: Kind::Box,
                rect: paint.rect,
                style: paint.style,
            }),
        }
    }
}

/// Quads with the same bounds and look but different clips as one, showing
/// everything the copies showed. GPUI paints a border as one quad clipped
/// to each edge in turn.
fn fold(paint: Vec<Paint>) -> Vec<Paint> {
    let mut folded: Vec<Paint> = Vec::new();
    for paint in paint {
        let copy = folded.iter_mut().find(|earlier| {
            earlier.rect == paint.rect
                && earlier.style.background == paint.style.background
                && earlier.style.border == paint.style.border
                && earlier.style.radius == paint.style.radius
        });
        match copy {
            Some(earlier) => {
                let shown = earlier
                    .style
                    .clip
                    .unwrap_or(earlier.rect)
                    .union(&paint.style.clip.unwrap_or(paint.rect));
                earlier.style.clip = (shown != earlier.rect).then_some(shown);
            }
            None => folded.push(paint),
        }
    }
    folded
}

/// `a` and `b` as one line's paint, or `None` when they contradict: a
/// different background, border, radius or clip.
fn combined(a: &Style, b: &Style) -> Option<Style> {
    fn either<T: Clone + PartialEq>(a: &Option<T>, b: &Option<T>) -> Option<Option<T>> {
        match (a, b) {
            (Some(a), Some(b)) if a != b => None,
            (Some(a), _) => Some(Some(a.clone())),
            (None, b) => Some(b.clone()),
        }
    }
    if a.clip != b.clip {
        return None;
    }
    Some(Style {
        background: either(&a.background, &b.background)?,
        border: either(&a.border, &b.border)?,
        radius: either(&a.radius, &b.radius)?,
        clip: a.clip,
    })
}

/// Whether `a` goes outside `b` when one could contain the other: larger
/// first, then elements before boxes before text, then the one drawn first.
fn outer(items: &[Item], a: usize, b: usize) -> Ordering {
    items[b]
        .rect
        .area()
        .cmp(&items[a].rect.area())
        .then_with(|| items[a].kind.rank().cmp(&items[b].kind.rank()))
        .then_with(|| a.cmp(&b))
}

/// The lines as text, each under the innermost element or box that
/// contains its centre. An element or box goes only under one that is
/// outside it (larger, or first at the same size), so there are no loops;
/// text and underlines hold nothing, so they can go under a smaller element,
/// as an input's text that overflows it does. A box is never put inside
/// another box with the same bounds.
fn write(header: &str, items: &[Item]) -> String {
    let root = items.len();
    let mut children = vec![Vec::new(); root + 1];
    for (index, item) in items.iter().enumerate() {
        let parent = (0..items.len())
            .filter(|&candidate| {
                let holder = &items[candidate];
                let same_box =
                    holder.kind == Kind::Box && item.kind == Kind::Box && holder.rect == item.rect;
                candidate != index
                    && holder.kind.holds()
                    && !same_box
                    && holder.rect.contains_centre_of(&item.rect)
                    && (!item.kind.holds() || outer(items, candidate, index) == Ordering::Less)
            })
            .max_by(|&a, &b| outer(items, a, b))
            .unwrap_or(root);
        children[parent].push(index);
    }
    for nodes in &mut children {
        nodes.sort_by(|&a, &b| {
            let (ra, rb) = (&items[a].rect, &items[b].rect);
            ra.top
                .cmp(&rb.top)
                .then(ra.left.cmp(&rb.left))
                .then_with(|| outer(items, a, b))
        });
    }

    let mut out = format!("{header}\n");
    if children[root].is_empty() {
        out.push_str("(nothing observed: mark elements with .test_support())\n");
    }
    write_children(items, &children, root, 0, &mut out);
    out
}

fn write_children(items: &[Item], children: &[Vec<usize>], parent: usize, depth: usize, out: &mut String) {
    for &index in &children[parent] {
        let item = &items[index];
        for _ in 0..depth {
            out.push_str("  ");
        }
        match &item.kind {
            Kind::Element(name) => out.push_str(name),
            Kind::Text { text, alternative } => {
                let kind = if *alternative { "alt" } else { "text" };
                let _ = write!(out, "{kind} {text:?}");
            }
            Kind::Box => out.push_str("box"),
            Kind::Underline { .. } => out.push_str("underline"),
        }
        let _ = write!(out, " {}{}", item.rect, item.style);
        if let Kind::Underline { color, wavy } = &item.kind {
            let _ = write!(out, " {color}");
            if *wavy {
                out.push_str(" wavy");
            }
        }
        out.push('\n');
        write_children(items, children, index, depth + 1, out);
    }
}

/// How the view snapshot names an element: role and label, or `#id`.
fn name(element: &ElementSnapshot) -> String {
    match (element.role(), element.label()) {
        (Some(role), Some(label)) => format!("{role:?} {label:?}"),
        (Some(role), None) => format!("{role:?}"),
        (None, Some(label)) => format!("{label:?}"),
        (None, None) => format!("#{}", crate::view::leaf(element)),
    }
}

fn background(background: &Background) -> String {
    if let Some(color) = background.as_solid() {
        return hex(color);
    }
    // Gradients and patterns keep their colours private; name the kind.
    let debug = format!("{background:?}");
    match debug.split('(').next() {
        Some("LinearGradient") => "gradient".to_owned(),
        Some("PatternSlash") => "stripes".to_owned(),
        Some("Checkerboard") => "checkerboard".to_owned(),
        _ => "pattern".to_owned(),
    }
}

fn border(widths: &Edges<ScaledPixels>, color: Hsla, style: BorderStyle, scale: f32) -> Option<String> {
    let edges = [widths.top, widths.right, widths.bottom, widths.left].map(|width| width.as_f32() / scale);
    if color.is_transparent() || edges.iter().all(|width| *width <= 0.0) {
        return None;
    }
    let mut border = sides(&edges);
    let _ = write!(border, " {}", hex(color));
    if style == BorderStyle::Dashed {
        border.push_str(" dashed");
    }
    Some(border)
}

/// The corner radii, each no more than half the shorter side, as GPUI draws
/// them. `None` when the corners are square.
fn radius(corners: &Corners<ScaledPixels>, scale: f32, rect: &Rect) -> Option<String> {
    let most = rect.width().min(rect.height()) as f32 / 2.0;
    let radii = [
        corners.top_left,
        corners.top_right,
        corners.bottom_right,
        corners.bottom_left,
    ]
    .map(|radius| (radius.as_f32() / scale).clamp(0.0, most.max(0.0)));
    radii.iter().any(|radius| *radius > 0.0).then(|| sides(&radii))
}

/// One number when all four are the same, otherwise all four.
fn sides(values: &[f32; 4]) -> String {
    let numbers = values.map(number);
    if numbers.iter().all(|number| *number == numbers[0]) {
        numbers[0].clone()
    } else {
        numbers.join(",")
    }
}

/// A width or radius to a tenth of a pixel, so a hairline doesn't vanish:
/// `1`, `0.5`.
fn number(value: f32) -> String {
    let tenths = (value * 10.0).round() as i64;
    if tenths % 10 == 0 {
        format!("{}", tenths / 10)
    } else {
        format!("{}.{}", tenths / 10, tenths % 10)
    }
}

/// `#rrggbb`, or `#rrggbbaa` when not opaque.
fn hex(color: Hsla) -> String {
    let rgba = color.to_rgb();
    let byte = |channel: f32| (channel.clamp(0.0, 1.0) * 255.0).round() as u8;
    let mut hex = format!("#{:02x}{:02x}{:02x}", byte(rgba.r), byte(rgba.g), byte(rgba.b));
    let alpha = byte(rgba.a);
    if alpha != u8::MAX {
        let _ = write!(hex, "{alpha:02x}");
    }
    hex
}

fn whole(value: f32) -> i32 {
    value.round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mode::Mode;
    use crate::store::tests::snapshots;
    use gpui_kit::{ContentMask, point, px, rgb, rgba, size};

    const SCALE: f32 = 2.0;

    fn rect(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect {
            left: x,
            top: y,
            right: x + width,
            bottom: y + height,
        }
    }

    fn element(name: &str, rect: Rect) -> Item {
        Item::plain(Kind::Element(name.to_owned()), rect)
    }

    fn text(text: &str, rect: Rect) -> Item {
        let kind = Kind::Text {
            text: text.to_owned(),
            alternative: false,
        };
        Item::plain(kind, rect)
    }

    fn scaled(x: f32, y: f32, width: f32, height: f32) -> Bounds<ScaledPixels> {
        Bounds::new(
            point(ScaledPixels(x * SCALE), ScaledPixels(y * SCALE)),
            size(ScaledPixels(width * SCALE), ScaledPixels(height * SCALE)),
        )
    }

    /// A quad at logical `x,y wxh` filled with `color`, inside a window-sized
    /// mask.
    fn filled(x: f32, y: f32, width: f32, height: f32, color: u32) -> Quad {
        Quad {
            bounds: scaled(x, y, width, height),
            content_mask: ContentMask {
                bounds: scaled(0.0, 0.0, 400.0, 400.0),
            },
            background: rgb(color).into(),
            ..Quad::default()
        }
    }

    fn lines(text: &str) -> Vec<&str> {
        text.lines().skip(1).collect()
    }

    #[test]
    fn lines_sit_under_the_smallest_element_containing_them_in_reading_order() {
        let items = [
            text("b", rect(60, 10, 10, 10)),
            element("Button \"Add\"", rect(50, 0, 40, 30)),
            element("List", rect(0, 0, 100, 100)),
            text("a", rect(10, 10, 10, 10)),
            element("Status", rect(0, 50, 100, 10)),
        ];
        let written = write("window", &items);
        assert_eq!(
            lines(&written),
            [
                "List 0,0 100x100",
                "  Button \"Add\" 50,0 40x30",
                "    text \"b\" 60,10 10x10",
                "  text \"a\" 10,10 10x10",
                "  Status 0,50 100x10",
            ]
        );
    }

    #[test]
    fn an_element_with_the_same_bounds_as_another_goes_inside_the_one_drawn_first() {
        let items = [element("Outer", rect(0, 0, 10, 10)), element("Inner", rect(0, 0, 10, 10))];
        let written = write("window", &items);
        assert_eq!(lines(&written), ["Outer 0,0 10x10", "  Inner 0,0 10x10"]);
    }

    #[test]
    fn text_holds_nothing_even_when_it_contains_something() {
        let items = [
            text("label", rect(0, 0, 50, 20)),
            text("i", rect(0, 0, 5, 20)),
        ];
        let written = write("window", &items);
        assert_eq!(lines(&written), ["text \"label\" 0,0 50x20", "text \"i\" 0,0 5x20"]);
    }

    #[test]
    fn a_quad_filling_an_element_is_written_on_its_line_and_others_are_boxes() {
        let mut items = vec![element("ListItem", rect(8, 20, 100, 20))];
        let paint = [
            filled(8.0, 20.0, 100.0, 20.0, 0x2f4a72),
            filled(10.0, 22.0, 4.0, 4.0, 0xff0000),
        ];
        merge(&mut items, paint.iter().filter_map(|quad| Paint::of(quad, SCALE)).collect());
        let written = write("window", &items);
        assert_eq!(
            lines(&written),
            ["ListItem 8,20 100x20 bg #2f4a72", "  box 10,22 4x4 bg #ff0000"]
        );
    }

    #[test]
    fn a_second_quad_over_a_painted_element_is_a_box_inside_it() {
        let mut items = vec![element("Button", rect(0, 0, 10, 10))];
        let paint = [filled(0.0, 0.0, 10.0, 10.0, 0xffffff), filled(0.0, 0.0, 10.0, 10.0, 0x000000)];
        merge(&mut items, paint.iter().filter_map(|quad| Paint::of(quad, SCALE)).collect());
        let written = write("window", &items);
        assert_eq!(lines(&written), ["Button 0,0 10x10 bg #ffffff", "  box 0,0 10x10 bg #000000"]);
    }

    /// `quad` clipped to logical `x,y wxh`.
    fn clipped(quad: Quad, x: f32, y: f32, width: f32, height: f32) -> Quad {
        Quad {
            content_mask: ContentMask {
                bounds: scaled(x, y, width, height),
            },
            ..quad
        }
    }

    #[test]
    fn a_border_painted_as_clipped_copies_is_written_once_on_its_element() {
        let background = Quad {
            corner_radii: Corners::all(ScaledPixels(6.0 * SCALE)),
            ..filled(8.0, 42.0, 98.0, 32.0, 0xffffff)
        };
        let border = Quad {
            background: rgba(0x00000000).into(),
            border_widths: Edges::all(ScaledPixels(1.0 * SCALE)),
            border_color: rgb(0xe5e5e5).into(),
            ..background
        };
        let paint = [
            background,
            clipped(border, 8.0, 42.0, 98.0, 2.0),
            clipped(border, 8.0, 73.0, 98.0, 1.0),
            clipped(border, 8.0, 44.0, 7.0, 29.0),
            clipped(border, 100.0, 44.0, 6.0, 29.0),
        ];
        let mut items = vec![element("Button", rect(8, 42, 98, 32))];
        merge(&mut items, paint.iter().filter_map(|quad| Paint::of(quad, SCALE)).collect());
        let written = write("window", &items);
        assert_eq!(
            lines(&written),
            ["Button 8,42 98x32 bg #ffffff border 1 #e5e5e5 radius 6"]
        );
    }

    #[test]
    fn copies_that_show_only_part_of_a_quad_keep_that_part_as_the_clip() {
        let quad = filled(0.0, 0.0, 100.0, 40.0, 0xffffff);
        let paint = [clipped(quad, 0.0, 0.0, 100.0, 10.0), clipped(quad, 0.0, 10.0, 100.0, 10.0)];
        let folded = fold(paint.iter().filter_map(|quad| Paint::of(quad, SCALE)).collect());
        let clips: Vec<Option<Rect>> = folded.iter().map(|paint| paint.style.clip).collect();
        assert_eq!(clips, [Some(rect(0, 0, 100, 20))]);
    }

    #[test]
    fn boxes_with_the_same_bounds_sit_side_by_side() {
        let mut items = Vec::new();
        let paint = [filled(0.0, 0.0, 10.0, 10.0, 0xffffff), filled(0.0, 0.0, 10.0, 10.0, 0x000000)];
        merge(&mut items, paint.iter().filter_map(|quad| Paint::of(quad, SCALE)).collect());
        let written = write("window", &items);
        assert_eq!(lines(&written), ["box 0,0 10x10 bg #ffffff", "box 0,0 10x10 bg #000000"]);
    }

    #[test]
    fn text_overflowing_its_input_by_a_little_still_sits_under_it() {
        let items = [
            Item {
                kind: Kind::Box,
                rect: rect(0, 0, 360, 320),
                style: Style::default(),
            },
            element("TextInput", rect(8, 82, 200, 24)),
            text("tea", rect(9, 83, 198, 26)),
        ];
        let written = write("window", &items);
        assert_eq!(
            lines(&written),
            ["box 0,0 360x320", "  TextInput 8,82 200x24", "    text \"tea\" 9,83 198x26"]
        );
    }

    #[test]
    fn edges_round_to_whole_pixels_so_touching_boxes_still_touch() {
        let left = Paint::of(&filled(0.4, 0.0, 10.2, 10.0, 0x111111), SCALE);
        let right = Paint::of(&filled(10.6, 0.0, 10.0, 10.0, 0x222222), SCALE);
        let (Some(left), Some(right)) = (left, right) else {
            panic!("both quads paint");
        };
        assert_eq!(left.rect, rect(0, 0, 11, 10));
        assert_eq!(right.rect.left, left.rect.right);
    }

    #[test]
    fn quads_that_paint_nothing_or_are_clipped_away_are_left_out() {
        let clear = Quad {
            background: rgba(0xffffff00).into(),
            ..filled(0.0, 0.0, 10.0, 10.0, 0)
        };
        assert_eq!(Paint::of(&clear, SCALE), None);

        let invisible_border = Quad {
            border_widths: Edges::all(ScaledPixels(2.0)),
            border_color: rgba(0x00000000).into(),
            ..clear
        };
        assert_eq!(Paint::of(&invisible_border, SCALE), None);

        let outside = Quad {
            content_mask: ContentMask {
                bounds: scaled(50.0, 50.0, 10.0, 10.0),
            },
            ..filled(0.0, 0.0, 10.0, 10.0, 0xffffff)
        };
        assert_eq!(Paint::of(&outside, SCALE), None);
    }

    #[test]
    fn a_partly_clipped_quad_says_what_is_left_showing() {
        let quad = Quad {
            content_mask: ContentMask {
                bounds: scaled(0.0, 0.0, 100.0, 30.0),
            },
            ..filled(0.0, 20.0, 100.0, 20.0, 0xffffff)
        };
        let paint = Paint::of(&quad, SCALE);
        assert_eq!(paint.and_then(|paint| paint.style.clip), Some(rect(0, 20, 100, 10)));
    }

    #[test]
    fn borders_and_radii_are_one_number_when_even_and_four_when_not() {
        let even = Quad {
            border_widths: Edges::all(ScaledPixels(1.0 * SCALE)),
            border_color: rgb(0xe5e5e5).into(),
            corner_radii: Corners::all(ScaledPixels(6.0 * SCALE)),
            ..filled(0.0, 0.0, 40.0, 20.0, 0xffffff)
        };
        let style = Paint::of(&even, SCALE).map(|paint| paint.style);
        assert_eq!(style.as_ref().and_then(|style| style.border.as_deref()), Some("1 #e5e5e5"));
        assert_eq!(style.as_ref().and_then(|style| style.radius.as_deref()), Some("6"));

        let uneven = Quad {
            border_widths: Edges {
                top: ScaledPixels(0.0),
                right: ScaledPixels(0.0),
                bottom: ScaledPixels(1.0),
                left: ScaledPixels(0.0),
            },
            border_style: BorderStyle::Dashed,
            ..even
        };
        let border = Paint::of(&uneven, SCALE).and_then(|paint| paint.style.border);
        assert_eq!(border.as_deref(), Some("0,0,0.5,0 #e5e5e5 dashed"));
    }

    #[test]
    fn a_radius_is_never_more_than_half_the_shorter_side() {
        let pill = Quad {
            corner_radii: Corners::all(px(9999.).scale(SCALE)),
            ..filled(0.0, 0.0, 60.0, 24.0, 0xffffff)
        };
        let radius = Paint::of(&pill, SCALE).and_then(|paint| paint.style.radius);
        assert_eq!(radius.as_deref(), Some("12"));
    }

    #[test]
    fn colours_show_alpha_only_when_translucent() {
        assert_eq!(hex(rgb(0x2f4a72).into()), "#2f4a72");
        assert_eq!(hex(rgba(0x2f4a7280).into()), "#2f4a7280");
    }

    #[test]
    fn test_fonts_keep_one_file_for_every_os_and_platform_fonts_one_per_os() {
        let record = snapshots("layout", Mode::Record);
        let test = Layout {
            text: "window 10x10, test fonts\n".to_owned(),
            fonts: Fonts::Test,
        };
        let platform = Layout {
            fonts: Fonts::Platform,
            ..test.clone()
        };
        assert!(record.check_layout("view", &test).is_ok());
        assert!(record.check_layout("view", &platform).is_ok());
        assert!(record.path("view.layout.txt").is_file());
        assert!(
            record
                .path(&format!("view.{}.layout.txt", std::env::consts::OS))
                .is_file()
        );
        assert!(!record.path("view.txt").exists(), "a layout doesn't take the view snapshot's file");
    }

    #[test]
    fn a_changed_layout_fails_verification_and_leaves_the_snapshot_as_it_was() {
        let record = snapshots("layout-mismatch", Mode::Record);
        let before = Layout {
            text: "window 10x10, test fonts\nbox 0,0 4x4 bg #ffffff\n".to_owned(),
            fonts: Fonts::Test,
        };
        assert!(record.check_layout("view", &before).is_ok());
        let after = Layout {
            text: "window 10x10, test fonts\nbox 0,0 5x4 bg #ffffff\n".to_owned(),
            ..before.clone()
        };
        let verify = record.with_mode(Mode::Verify);
        assert!(matches!(
            verify.check_layout("view", &after),
            Err(SnapshotError::Mismatch { .. })
        ));
        assert_eq!(verify.check_layout("view", &before).ok(), Some(Outcome::Matched));
    }
}

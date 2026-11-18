//! How far a diagram is zoomed and where it is panned to, and the pictures a
//! zoom needs. Pure arithmetic in a diagram's natural units (its size at
//! 100%), so it is tested without a window.
//!
//! A frame shows a diagram at `scale` with its top-left corner at `offset`
//! from the frame's top-left. A diagram smaller than its frame is centred;
//! a larger one can't be panned past its edges.

/// The most a diagram is magnified, unless fitting it takes more.
pub(crate) const MAX_SCALE: f32 = 8.;
/// One press of zoom in or out.
pub(crate) const STEP: f32 = 1.25;
/// How close to fitted a zoom snaps back to fitted.
const SNAP: f32 = 0.005;
/// How much a pixel of zoom scrolling zooms: a wheel notch of 20 pixels is
/// about a fifth.
const SCROLL_ZOOM: f32 = 0.01;

/// A point or a size.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Vec2 {
    pub(crate) x: f32,
    pub(crate) y: f32,
}

impl Vec2 {
    pub(crate) const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// The scale at which the whole diagram just fits its frame.
pub(crate) fn fit_scale(frame: Vec2, content: Vec2) -> f32 {
    if content.x <= 0. || content.y <= 0. || frame.x <= 0. || frame.y <= 0. {
        return 1.;
    }
    (frame.x / content.x).min(frame.y / content.y)
}

/// The least and most a diagram can be zoomed in this frame: never smaller
/// than both fitted and 100%, never larger than [`MAX_SCALE`] or fitted.
pub(crate) fn scale_limits(frame: Vec2, content: Vec2) -> (f32, f32) {
    let fit = fit_scale(frame, content);
    (fit.min(1.), fit.max(MAX_SCALE))
}

/// Where a diagram's edge may sit on one axis: centred when it fits,
/// otherwise never leaving a gap at either side.
fn clamp_axis(offset: f32, size: f32, frame: f32) -> f32 {
    if size <= frame {
        (frame - size) / 2.
    } else {
        offset.clamp(frame - size, 0.)
    }
}

/// A zoom and pan.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Viewport {
    /// Screen points per natural unit.
    pub(crate) scale: f32,
    /// The diagram's top-left corner, from the frame's top-left.
    pub(crate) offset: Vec2,
}

impl Viewport {
    /// The whole diagram, centred.
    pub(crate) fn fit(frame: Vec2, content: Vec2) -> Self {
        Self {
            scale: fit_scale(frame, content),
            offset: Vec2::default(),
        }
        .clamped(frame, content)
    }

    /// The diagram at 100%, centred.
    pub(crate) fn actual(frame: Vec2, content: Vec2) -> Self {
        Self {
            scale: 1.,
            offset: Vec2::new((frame.x - content.x) / 2., (frame.y - content.y) / 2.),
        }
        .clamped(frame, content)
    }

    /// This zoom kept within the limits and this pan within the edges.
    pub(crate) fn clamped(self, frame: Vec2, content: Vec2) -> Self {
        let (min, max) = scale_limits(frame, content);
        let scale = if self.scale.is_finite() { self.scale.clamp(min, max) } else { min };
        let offset = Vec2::new(
            clamp_axis(self.offset.x, content.x * scale, frame.x),
            clamp_axis(self.offset.y, content.y * scale, frame.y),
        );
        Self { scale, offset }
    }

    /// Zoomed by `factor` about `point` in the frame: what is under the
    /// point stays under it, unless an edge or a limit stops it.
    pub(crate) fn zoomed_about(self, point: Vec2, factor: f32, frame: Vec2, content: Vec2) -> Self {
        if !factor.is_finite() || factor <= 0. || self.scale <= 0. {
            return self.clamped(frame, content);
        }
        let (min, max) = scale_limits(frame, content);
        let scale = (self.scale * factor).clamp(min, max);
        let ratio = scale / self.scale;
        let offset = Vec2::new(
            point.x - (point.x - self.offset.x) * ratio,
            point.y - (point.y - self.offset.y) * ratio,
        );
        Self { scale, offset }.clamped(frame, content)
    }

    /// Zoomed by `factor` about the frame's centre.
    pub(crate) fn zoomed(self, factor: f32, frame: Vec2, content: Vec2) -> Self {
        self.zoomed_about(Vec2::new(frame.x / 2., frame.y / 2.), factor, frame, content)
    }

    /// Moved by `delta`, within the edges.
    pub(crate) fn panned(self, delta: Vec2, frame: Vec2, content: Vec2) -> Self {
        Self {
            scale: self.scale,
            offset: Vec2::new(self.offset.x + delta.x, self.offset.y + delta.y),
        }
        .clamped(frame, content)
    }

    /// Whether this is, near enough, the fitted view.
    pub(crate) fn is_fit(self, frame: Vec2, content: Vec2) -> bool {
        let fit = fit_scale(frame, content);
        (self.scale - fit).abs() <= fit * SNAP
    }
}

/// The zoom a scroll of `dy` pixels makes with the zoom key held: away from
/// the reader zooms in.
pub(crate) fn scroll_factor(dy: f32) -> f32 {
    (dy * SCROLL_ZOOM).exp()
}

/// The zoom a pinch step of `delta` makes.
pub(crate) fn pinch_factor(delta: f32) -> f32 {
    (1. + delta).max(0.01)
}

/// What a key does to a focused diagram.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeyCommand {
    ZoomIn,
    ZoomOut,
    Fit,
    Close,
}

/// The command for a key press without Command, Control or Option: `+` (or
/// `=`, its unshifted key), `-`, `0` and Escape.
pub(crate) fn key_command(key: &str, key_char: Option<&str>, chord: bool) -> Option<KeyCommand> {
    if chord {
        return None;
    }
    match (key, key_char) {
        (_, Some("+")) | ("+" | "=" | "plus", _) => Some(KeyCommand::ZoomIn),
        (_, Some("-")) | ("-" | "minus", _) => Some(KeyCommand::ZoomOut),
        (_, Some("0")) | ("0", _) => Some(KeyCommand::Fit),
        ("escape", _) => Some(KeyCommand::Close),
        _ => None,
    }
}

/// The picture every diagram has: drawn at twice its natural size, sharp on
/// a Retina screen at 100%.
pub(crate) const BASE_RASTER: u8 = 2;
/// The sizes a zoomed diagram is drawn at again, as multiples of natural.
/// Buckets, so a zoom draws a few pictures rather than one per step.
const RASTERS: [u8; 4] = [BASE_RASTER, 4, 8, 16];
/// The most pixels one picture may have (48 MB), and its longest side, so a
/// zoom into a large diagram stays within memory and a GPU texture.
const MAX_RASTER_PIXELS: f32 = 12_000_000.;
const MAX_RASTER_SIDE: f32 = 8192.;

/// The picture size to draw a diagram of natural size `content` at, shown
/// at `scale` on a screen of `device` pixels per point: the smallest bucket
/// that is sharp, or the largest that fits the limits.
pub(crate) fn raster_for(scale: f32, device: f32, content: Vec2) -> u8 {
    let wanted = scale * device;
    let mut chosen = BASE_RASTER;
    for raster in RASTERS {
        let factor = f32::from(raster);
        let width = content.x * factor;
        let height = content.y * factor;
        if raster != BASE_RASTER && (width > MAX_RASTER_SIDE || height > MAX_RASTER_SIDE || width * height > MAX_RASTER_PIXELS) {
            break;
        }
        chosen = raster;
        if factor >= wanted {
            break;
        }
    }
    chosen
}

/// The buckets no larger than `wanted`, largest first: what to show while
/// the wanted picture is drawn.
pub(crate) fn rasters_down_from(wanted: u8) -> impl Iterator<Item = u8> {
    RASTERS.into_iter().rev().filter(move |raster| *raster <= wanted)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: Vec2 = Vec2::new(400., 300.);
    const CONTENT: Vec2 = Vec2::new(800., 300.);

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn fitting_shows_the_whole_diagram_centred() {
        let fit = Viewport::fit(FRAME, CONTENT);
        assert!(near(fit.scale, 0.5));
        assert!(near(fit.offset.x, 0.));
        // 150 high in a 300 frame: centred.
        assert!(near(fit.offset.y, 75.));
        assert!(fit.is_fit(FRAME, CONTENT));
        // A small diagram in a large frame is fitted larger than 100%.
        assert!(near(fit_scale(Vec2::new(1000., 1000.), Vec2::new(100., 50.)), 10.));
    }

    #[test]
    fn what_is_under_the_pointer_stays_under_it() {
        let start = Viewport::fit(FRAME, CONTENT);
        let pointer = Vec2::new(100., 150.);
        let under = |view: Viewport| {
            Vec2::new(
                (pointer.x - view.offset.x) / view.scale,
                (pointer.y - view.offset.y) / view.scale,
            )
        };
        let before = under(start);
        let zoomed = start.zoomed_about(pointer, 2., FRAME, CONTENT);
        assert!(near(zoomed.scale, 1.));
        let after = under(zoomed);
        assert!(near(before.x, after.x), "{before:?} became {after:?}");
        assert!(near(before.y, after.y), "{before:?} became {after:?}");
    }

    #[test]
    fn zoom_stops_at_its_limits() {
        let fit = Viewport::fit(FRAME, CONTENT);
        let out = fit.zoomed(0.01, FRAME, CONTENT);
        assert!(near(out.scale, 0.5), "never smaller than fitted when fitted is under 100%");
        let mut zoomed = fit;
        for _ in 0..100 {
            zoomed = zoomed.zoomed(STEP, FRAME, CONTENT);
        }
        assert!(near(zoomed.scale, MAX_SCALE));
        // A frame that needs more than the maximum to fit allows it.
        let tiny = Vec2::new(10., 10.);
        let (min, max) = scale_limits(Vec2::new(1000., 1000.), tiny);
        assert!(near(min, 1.) && near(max, 100.));
        assert!(near(Viewport::actual(FRAME, CONTENT).scale, 1.));
    }

    #[test]
    fn a_pan_never_leaves_a_gap_at_an_edge() {
        let zoomed = Viewport::fit(FRAME, CONTENT).zoomed(4., FRAME, CONTENT);
        // 1600x600 in 400x300.
        let far_left = zoomed.panned(Vec2::new(10_000., 10_000.), FRAME, CONTENT);
        assert!(near(far_left.offset.x, 0.) && near(far_left.offset.y, 0.));
        let far_right = zoomed.panned(Vec2::new(-10_000., -10_000.), FRAME, CONTENT);
        assert!(near(far_right.offset.x, 400. - 1600.));
        assert!(near(far_right.offset.y, 300. - 600.));
        // Fitted, a pan has nowhere to go.
        let fit = Viewport::fit(FRAME, CONTENT);
        assert_eq!(fit.panned(Vec2::new(50., 50.), FRAME, CONTENT), fit);
    }

    #[test]
    fn a_bad_factor_or_a_zero_frame_changes_nothing_odd() {
        let fit = Viewport::fit(FRAME, CONTENT);
        assert_eq!(fit.zoomed(f32::NAN, FRAME, CONTENT), fit);
        assert_eq!(fit.zoomed(-1., FRAME, CONTENT), fit);
        assert!(near(fit_scale(Vec2::default(), CONTENT), 1.));
    }

    #[test]
    fn scrolling_away_and_pinching_out_zoom_in() {
        assert!(scroll_factor(20.) > 1.);
        assert!(scroll_factor(-20.) < 1.);
        assert!(near(scroll_factor(0.), 1.));
        assert!(pinch_factor(0.1) > 1.);
        assert!(pinch_factor(-5.) > 0.);
    }

    #[test]
    fn keys_zoom_fit_and_close_only_without_a_chord() {
        assert_eq!(key_command("=", Some("+"), false), Some(KeyCommand::ZoomIn));
        assert_eq!(key_command("=", Some("="), false), Some(KeyCommand::ZoomIn));
        assert_eq!(key_command("-", None, false), Some(KeyCommand::ZoomOut));
        assert_eq!(key_command("0", Some("0"), false), Some(KeyCommand::Fit));
        assert_eq!(key_command("escape", None, false), Some(KeyCommand::Close));
        assert_eq!(key_command("=", Some("+"), true), None);
        assert_eq!(key_command("a", Some("a"), false), None);
    }

    #[test]
    fn a_zoom_draws_the_smallest_sharp_bucket_within_the_limits() {
        let small = Vec2::new(400., 300.);
        assert_eq!(raster_for(1., 2., small), BASE_RASTER);
        assert_eq!(raster_for(0.5, 1., small), BASE_RASTER);
        assert_eq!(raster_for(1.5, 2., small), 4);
        // Nearby zooms share a picture.
        assert_eq!(raster_for(1.6, 2., small), raster_for(1.9, 2., small));
        assert_eq!(raster_for(3., 2., small), 8);
        // A deep zoom asks for more than the buckets hold: a tiny diagram
        // gets the sharpest, a bigger one stops where the pixel budget does.
        let tiny = Vec2::new(200., 150.);
        assert_eq!(raster_for(100., 2., tiny), 16);
        assert_eq!(raster_for(100., 2., small), 8);
        // A large diagram stops at the bucket that fits the pixel budget.
        let large = Vec2::new(2000., 1500.);
        assert_eq!(raster_for(8., 2., large), BASE_RASTER);
        let medium = Vec2::new(1000., 700.);
        assert_eq!(raster_for(8., 2., medium), 4);
        // While a zoom's picture is drawn, the next sharpest stands in.
        assert_eq!(rasters_down_from(8).collect::<Vec<_>>(), [8, 4, BASE_RASTER]);
    }
}

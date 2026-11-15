//! Image snapshots, macOS only: a headless render compared with a tolerance.
//!
//! Pixels depend on the GPU, the fonts installed and their versions, so an
//! image snapshot is for what the view and layout snapshots can't show:
//! glyphs, text colour, icons, shadows, anti-aliasing. Linux runs the state,
//! view, layout and flow snapshots only.

use std::io::Cursor;
use std::sync::Arc;

use gpui_kit::HeadlessAppContext;
use image::{ImageFormat, Rgba, RgbaImage};

use crate::mode::Mode;
use crate::store::{Outcome, SnapshotError, Snapshots, checked, read, write};

/// How far a drawn image may stray from its snapshot and still match.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tolerance {
    channel: u8,
    fraction: f64,
}

impl Tolerance {
    /// Every pixel the same.
    pub const EXACT: Self = Self {
        channel: 0,
        fraction: 0.0,
    };

    /// A pixel differs when any of its channels is off by more than
    /// `channel` (of 255). The image matches when no more than `fraction`
    /// (0 to 1) of its pixels differ.
    pub fn new(channel: u8, fraction: f64) -> Self {
        Self {
            channel,
            fraction: if fraction.is_nan() {
                0.0
            } else {
                fraction.clamp(0.0, 1.0)
            },
        }
    }

    fn allowed(&self, total: u64) -> u64 {
        // Rounds down, so a tolerance never allows more than it says.
        (self.fraction * total as f64) as u64
    }

    fn differs(&self, a: &Rgba<u8>, b: &Rgba<u8>) -> bool {
        a.0.iter()
            .zip(b.0.iter())
            .any(|(a, b)| a.abs_diff(*b) > self.channel)
    }
}

impl Default for Tolerance {
    /// Anti-aliasing noise: channels off by up to 2, in up to 0.1% of the
    /// pixels.
    fn default() -> Self {
        Self::new(2, 0.001)
    }
}

/// A headless GPUI context that renders with Metal, off screen. No window
/// is shown and it runs on any thread, so it works under `cargo test`.
/// gpui-kit is initialized in it.
pub fn headless_context() -> HeadlessAppContext {
    let mut cx = HeadlessAppContext::with_platform(
        Arc::new(gpui_apple::AppleTextSystem::new()),
        Arc::new(gpui_kit::assets::Assets),
        gpui_kit::platform::current_headless_renderer,
    );
    cx.update(gpui_kit::init);
    cx
}

impl Snapshots {
    /// Checks `actual` against the image snapshot `name`, or records it.
    /// Recording keeps a snapshot that still matches within `tolerance`, so
    /// rendering noise doesn't rewrite it.
    ///
    /// # Errors
    ///
    /// [`SnapshotError::Missing`], [`SnapshotError::ImageSize`] or
    /// [`SnapshotError::ImageMismatch`] when verifying, or an error reading,
    /// decoding or writing the files.
    pub fn check_image(
        &self,
        name: &str,
        actual: &RgbaImage,
        tolerance: Tolerance,
    ) -> Result<Outcome, SnapshotError> {
        let name = checked(name)?;
        let file = format!("{name}.png");
        let path = self.path(&file);
        let failed = [
            format!("{name}.old.png"),
            format!("{name}.new.png"),
            format!("{name}.diff.png"),
            format!("{name}.review.png"),
        ];
        let failed: Vec<&str> = failed.iter().map(String::as_str).collect();

        let recorded = match read(&path)? {
            Some(bytes) => Some(
                image::load_from_memory_with_format(&bytes, ImageFormat::Png)
                    .map_err(|source| SnapshotError::DecodeImage {
                        path: path.clone(),
                        source,
                    })?
                    .into_rgba8(),
            ),
            None => None,
        };
        let comparison = recorded
            .as_ref()
            .map(|recorded| compare(recorded, actual, tolerance));
        if matches!(comparison, Some(Comparison::Within)) {
            self.clear_failed(&failed)?;
            return Ok(Outcome::Matched);
        }
        if self.mode() == Mode::Record {
            write(&path, &png(actual, &path)?)?;
            self.clear_failed(&failed)?;
            return Ok(Outcome::Recorded);
        }

        let new = self.write_failed(&format!("{name}.new.png"), &png(actual, &path)?)?;
        let (Some(recorded), Some(comparison)) = (recorded, comparison) else {
            return Err(SnapshotError::Missing { path, review: new });
        };
        self.write_failed(&format!("{name}.old.png"), &png(&recorded, &path)?)?;
        match comparison {
            Comparison::Within => Ok(Outcome::Matched),
            Comparison::Size => {
                let review = self.write_failed(
                    &format!("{name}.review.png"),
                    &png(&side_by_side(&[&recorded, actual]), &path)?,
                )?;
                Err(SnapshotError::ImageSize {
                    path,
                    recorded_width: recorded.width(),
                    recorded_height: recorded.height(),
                    width: actual.width(),
                    height: actual.height(),
                    review,
                })
            }
            Comparison::Beyond { differing, total } => {
                let diff = diff_image(&recorded, actual, tolerance);
                self.write_failed(&format!("{name}.diff.png"), &png(&diff, &path)?)?;
                let review = self.write_failed(
                    &format!("{name}.review.png"),
                    &png(&side_by_side(&[&recorded, actual, &diff]), &path)?,
                )?;
                Err(SnapshotError::ImageMismatch {
                    path,
                    differing,
                    total,
                    allowed: tolerance.allowed(total),
                    review,
                })
            }
        }
    }

    /// Like [`check_image`](Self::check_image), panicking with what failed
    /// and how to fix it.
    #[track_caller]
    pub fn assert_image(&self, name: &str, actual: &RgbaImage, tolerance: Tolerance) {
        if let Err(error) = self.check_image(name, actual, tolerance) {
            panic!("{error}");
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Comparison {
    Within,
    Size,
    Beyond { differing: u64, total: u64 },
}

fn compare(recorded: &RgbaImage, actual: &RgbaImage, tolerance: Tolerance) -> Comparison {
    if recorded.dimensions() != actual.dimensions() {
        return Comparison::Size;
    }
    let total = u64::from(recorded.width()) * u64::from(recorded.height());
    let differing = recorded
        .pixels()
        .zip(actual.pixels())
        .filter(|(a, b)| tolerance.differs(a, b))
        .count() as u64;
    if differing <= tolerance.allowed(total) {
        Comparison::Within
    } else {
        Comparison::Beyond { differing, total }
    }
}

/// The recorded image, faded, with the pixels that differ in red.
fn diff_image(recorded: &RgbaImage, actual: &RgbaImage, tolerance: Tolerance) -> RgbaImage {
    let mut diff = RgbaImage::new(recorded.width(), recorded.height());
    for ((out, a), b) in diff.pixels_mut().zip(recorded.pixels()).zip(actual.pixels()) {
        *out = if tolerance.differs(a, b) {
            Rgba([255, 0, 0, 255])
        } else {
            let [r, g, b, _] = a.0;
            let luma = (u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000;
            // A quarter of the original's brightness over white.
            let faded = (191 + luma / 4).min(255) as u8;
            Rgba([faded, faded, faded, 255])
        };
    }
    diff
}

/// The images left to right, top-aligned, with a gap between them.
fn side_by_side(images: &[&RgbaImage]) -> RgbaImage {
    const GAP: u32 = 8;
    let width = images.iter().map(|image| image.width()).sum::<u32>()
        + GAP * images.len().saturating_sub(1) as u32;
    let height = images.iter().map(|image| image.height()).max().unwrap_or(0);
    let mut out = RgbaImage::from_pixel(width, height, Rgba([128, 128, 128, 255]));
    let mut left = 0;
    for image in images {
        for (x, y, pixel) in image.enumerate_pixels() {
            out.put_pixel(left + x, y, *pixel);
        }
        left += image.width() + GAP;
    }
    out
}

fn png(image: &RgbaImage, path: &std::path::Path) -> Result<Vec<u8>, SnapshotError> {
    let mut bytes = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .map_err(|source| SnapshotError::EncodeImage {
            path: path.to_owned(),
            source,
        })?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::snapshots;

    fn filled(width: u32, height: u32, value: u8) -> RgbaImage {
        RgbaImage::from_pixel(width, height, Rgba([value, value, value, 255]))
    }

    #[test]
    fn small_channel_noise_is_within_tolerance() {
        let a = filled(10, 10, 100);
        let b = filled(10, 10, 102);
        assert_eq!(compare(&a, &b, Tolerance::new(2, 0.0)), Comparison::Within);
        assert_ne!(compare(&a, &b, Tolerance::EXACT), Comparison::Within);
    }

    #[test]
    fn a_few_changed_pixels_are_allowed_up_to_the_fraction() {
        let a = filled(10, 10, 0);
        let mut b = a.clone();
        b.put_pixel(0, 0, Rgba([255, 255, 255, 255]));
        assert_eq!(compare(&a, &b, Tolerance::new(0, 0.01)), Comparison::Within);
        assert_eq!(
            compare(&a, &b, Tolerance::new(0, 0.0)),
            Comparison::Beyond {
                differing: 1,
                total: 100
            }
        );
    }

    #[test]
    fn a_different_size_never_matches() {
        let a = filled(10, 10, 0);
        let b = filled(10, 11, 0);
        assert_eq!(compare(&a, &b, Tolerance::new(255, 1.0)), Comparison::Size);
    }

    #[test]
    fn the_diff_marks_exactly_the_changed_pixels() {
        let a = filled(4, 4, 0);
        let mut b = a.clone();
        b.put_pixel(2, 1, Rgba([200, 0, 0, 255]));
        let diff = diff_image(&a, &b, Tolerance::EXACT);
        let red: Vec<(u32, u32)> = diff
            .enumerate_pixels()
            .filter(|(_, _, pixel)| pixel.0 == [255, 0, 0, 255])
            .map(|(x, y, _)| (x, y))
            .collect();
        assert_eq!(red, [(2, 1)]);
    }

    #[test]
    fn a_mismatch_writes_old_new_and_diff_for_review() {
        let record = snapshots("image", Mode::Record);
        let old = filled(6, 6, 10);
        assert_eq!(
            record.check_image("view", &old, Tolerance::EXACT).ok(),
            Some(Outcome::Recorded)
        );
        let verify = record.with_mode(Mode::Verify);
        let new = filled(6, 6, 240);
        let error = verify.check_image("view", &new, Tolerance::EXACT).err();
        let Some(SnapshotError::ImageMismatch { review, .. }) = error else {
            panic!("expected an image mismatch, got {error:?}");
        };
        let review = image::open(review).map(|image| image.into_rgba8()).ok();
        // Old, new and diff side by side.
        assert_eq!(review.map(|image| image.width() > 3 * 6), Some(true));
        assert_eq!(
            verify.check_image("view", &old, Tolerance::EXACT).ok(),
            Some(Outcome::Matched)
        );
    }
}

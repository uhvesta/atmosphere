//! Which pixels an image change touched, as a count and as a picture.

use image::{Rgba, RgbaImage};

/// How two images differ.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PixelDiff {
    /// Pixels that differ, counting those only one image covers.
    pub changed: u64,
    /// Pixels of the canvas both images are laid on.
    pub total: u64,
    /// Whether the images' sizes differ.
    pub resized: bool,
    /// The new image faded to grey, with changed pixels in magenta, on a
    /// canvas as big as the larger image.
    pub picture: RgbaImage,
}

/// Changed pixels are drawn this colour.
pub const CHANGED: Rgba<u8> = Rgba([255, 0, 200, 255]);

/// Compares `old` and `new` pixel by pixel, both at the top left of one
/// canvas. A pixel changed when any channel differs by more than
/// `threshold`, or when only one image covers it.
pub fn pixel_diff(old: &RgbaImage, new: &RgbaImage, threshold: u8) -> PixelDiff {
    let width = old.width().max(new.width());
    let height = old.height().max(new.height());
    let mut picture = RgbaImage::new(width, height);
    let mut changed = 0u64;
    for y in 0..height {
        for x in 0..width {
            let before = (x < old.width() && y < old.height()).then(|| old.get_pixel(x, y));
            let after = (x < new.width() && y < new.height()).then(|| new.get_pixel(x, y));
            let differs = match (before, after) {
                (Some(a), Some(b)) => a
                    .0
                    .iter()
                    .zip(b.0.iter())
                    .any(|(a, b)| a.abs_diff(*b) > threshold),
                (None, None) => false,
                _ => true,
            };
            let pixel = if differs {
                changed += 1;
                CHANGED
            } else {
                faded(after.or(before))
            };
            picture.put_pixel(x, y, pixel);
        }
    }
    PixelDiff {
        changed,
        total: u64::from(width) * u64::from(height),
        resized: old.dimensions() != new.dimensions(),
        picture,
    }
}

/// An unchanged pixel, as a dim grey of its brightness.
fn faded(pixel: Option<&Rgba<u8>>) -> Rgba<u8> {
    let Some(Rgba([r, g, b, a])) = pixel.copied() else {
        return Rgba([0, 0, 0, 0]);
    };
    let luma = (u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000;
    let grey = u8::try_from(40 + luma * 90 / 255).unwrap_or(u8::MAX);
    Rgba([grey, grey, grey, a.max(160)])
}

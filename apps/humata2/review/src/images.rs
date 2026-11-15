//! Decoded images for the review, cached by blob, decoded off the UI thread.
//! An image too big to decode is said to be, and one that won't decode is
//! named broken; neither stops the rest of the review.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

use gpui_kit::RenderImage;
use image::{Frame, ImageReader, RgbaImage};

use crate::model::{ImageBlob, PixelDiff, pixel_diff};

/// Bigger files aren't decoded.
pub(crate) const MAX_BYTES: usize = 32 * 1024 * 1024;
/// Images with more pixels than this aren't decoded.
pub(crate) const MAX_PIXELS: u64 = 40_000_000;
/// Channel difference under which pixels count as the same.
const THRESHOLD: u8 = 8;

/// One side of an image change, as the pane shows it.
#[derive(Clone)]
pub enum ImageSide {
    /// There's no such side: the image was added or deleted.
    Absent,
    /// Still decoding.
    Loading,
    Ready {
        image: Arc<RenderImage>,
        width: u32,
        height: u32,
    },
    /// Too big to show; its size says why.
    TooLarge { bytes: usize, pixels: Option<(u32, u32)> },
    /// It doesn't decode.
    Broken { reason: String },
}

/// What the cache did, for tests and the large-review measure.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImageStats {
    /// Images decoded.
    pub decoded: usize,
    /// Times a decoded image was asked for again and reused.
    pub reused: usize,
    /// Pixel differences computed.
    pub diffs: usize,
}

pub(crate) struct Decoded {
    render: Arc<RenderImage>,
    rgba: Arc<RgbaImage>,
}

pub(crate) enum Outcome {
    Ready(Decoded),
    TooLarge { bytes: usize, pixels: Option<(u32, u32)> },
    Broken(String),
}

enum Slot {
    Loading,
    Done(Outcome),
}

/// A change's pixel difference.
#[derive(Clone)]
pub(crate) enum DiffSlot {
    Loading,
    Ready {
        picture: Arc<RenderImage>,
        changed: u64,
        total: u64,
        resized: bool,
    },
}

#[derive(Default)]
pub(crate) struct ImageCache {
    slots: HashMap<String, Slot>,
    diffs: HashMap<(String, String), DiffSlot>,
    pub(crate) stats: ImageStats,
}

/// Work for the background: decode a blob, or compare two decoded images.
pub(crate) enum Job {
    Decode(ImageBlob),
    Diff {
        key: (String, String),
        old: Arc<RgbaImage>,
        new: Arc<RgbaImage>,
    },
}

impl ImageCache {
    /// The side for `blob`, starting a decode the first time it's asked for.
    pub(crate) fn side(&mut self, blob: Option<&ImageBlob>, jobs: &mut Vec<Job>) -> ImageSide {
        let Some(blob) = blob else {
            return ImageSide::Absent;
        };
        match self.slots.get(&blob.sha) {
            None => {
                self.slots.insert(blob.sha.clone(), Slot::Loading);
                jobs.push(Job::Decode(blob.clone()));
                ImageSide::Loading
            }
            Some(Slot::Loading) => ImageSide::Loading,
            Some(Slot::Done(outcome)) => {
                self.stats.reused += 1;
                match outcome {
                    Outcome::Ready(decoded) => ImageSide::Ready {
                        image: decoded.render.clone(),
                        width: decoded.rgba.width(),
                        height: decoded.rgba.height(),
                    },
                    Outcome::TooLarge { bytes, pixels } => ImageSide::TooLarge {
                        bytes: *bytes,
                        pixels: *pixels,
                    },
                    Outcome::Broken(reason) => ImageSide::Broken {
                        reason: reason.clone(),
                    },
                }
            }
        }
    }

    /// The pixel difference between two decoded sides, starting it the
    /// first time; `None` while either side isn't decoded.
    pub(crate) fn diff(
        &mut self,
        old: &ImageBlob,
        new: &ImageBlob,
        jobs: &mut Vec<Job>,
    ) -> Option<DiffSlot> {
        let key = (old.sha.clone(), new.sha.clone());
        if let Some(slot) = self.diffs.get(&key) {
            return Some(slot.clone());
        }
        let rgba = |sha: &str| match self.slots.get(sha) {
            Some(Slot::Done(Outcome::Ready(decoded))) => Some(decoded.rgba.clone()),
            _ => None,
        };
        let (old_rgba, new_rgba) = (rgba(&old.sha)?, rgba(&new.sha)?);
        self.diffs.insert(key.clone(), DiffSlot::Loading);
        jobs.push(Job::Diff {
            key,
            old: old_rgba,
            new: new_rgba,
        });
        Some(DiffSlot::Loading)
    }

    pub(crate) fn decoded(&mut self, sha: String, outcome: Outcome) {
        self.stats.decoded += 1;
        self.slots.insert(sha, Slot::Done(outcome));
    }

    pub(crate) fn diffed(&mut self, key: (String, String), diff: PixelDiff) {
        self.stats.diffs += 1;
        self.diffs.insert(
            key,
            DiffSlot::Ready {
                picture: Arc::new(render_image(diff.picture)),
                changed: diff.changed,
                total: diff.total,
                resized: diff.resized,
            },
        );
    }
}

/// Decodes a blob. Runs on a background thread.
pub(crate) fn decode(blob: &ImageBlob) -> Outcome {
    let bytes = blob.bytes.len();
    if bytes > MAX_BYTES {
        return Outcome::TooLarge {
            bytes,
            pixels: None,
        };
    }
    let reader = match ImageReader::new(Cursor::new(&blob.bytes[..])).with_guessed_format() {
        Ok(reader) => reader,
        Err(error) => return Outcome::Broken(error.to_string()),
    };
    let dimensions = match ImageReader::new(Cursor::new(&blob.bytes[..]))
        .with_guessed_format()
        .map_err(image::ImageError::IoError)
        .and_then(ImageReader::into_dimensions)
    {
        Ok(dimensions) => dimensions,
        Err(error) => return Outcome::Broken(error.to_string()),
    };
    if u64::from(dimensions.0) * u64::from(dimensions.1) > MAX_PIXELS {
        return Outcome::TooLarge {
            bytes,
            pixels: Some(dimensions),
        };
    }
    match reader.decode() {
        Ok(image) => {
            let rgba = image.into_rgba8();
            Outcome::Ready(Decoded {
                render: Arc::new(render_image(rgba.clone())),
                rgba: Arc::new(rgba),
            })
        }
        Err(error) => Outcome::Broken(error.to_string()),
    }
}

/// Compares two decoded images. Runs on a background thread.
pub(crate) fn compare(old: &RgbaImage, new: &RgbaImage) -> PixelDiff {
    pixel_diff(old, new, THRESHOLD)
}

/// GPUI draws BGRA, so red and blue swap.
fn render_image(mut rgba: RgbaImage) -> RenderImage {
    for pixel in rgba.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    RenderImage::new(vec![Frame::new(rgba)])
}

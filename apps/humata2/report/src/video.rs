//! The video of a bug-report recording: the engine-rendered frames of the
//! window, sampled while recording, kept small and saved as an animated PNG.
//!
//! Frames come in as device pixels with secrets already masked by the
//! caller. Each is scaled down to logical points (a box average by the
//! integer scale factor), compared with the picture before it and stored as
//! the box around what changed, zstd-compressed. A screen that did not
//! change stores nothing and lengthens the previous frame's delay. When the
//! video grows past its limits the oldest frames fold into the first, which
//! is kept whole, so the video always starts from a complete picture.
//!
//! [`Video::write`] encodes `video.png` (APNG, frames as deltas over the
//! earlier ones) and `video.jsonl` (one line a frame, for agents) once, at
//! the end.

use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions, Permissions};
use std::io::{BufWriter, Write as _};
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::json;

/// The file holding the animation.
pub(crate) const PNG: &str = "video.png";

/// The file listing each frame, one JSON object a line.
pub(crate) const JSONL: &str = "video.jsonl";

/// How often the app is asked for a frame, in milliseconds.
pub const SAMPLE_MS: u64 = 200;

/// The most frames kept: the last 180 seconds at 5 a second.
const MAX_FRAMES: usize = 900;

/// The most compressed bytes of changed frames kept.
const MAX_BYTES: usize = 48 * 1024 * 1024;

/// zstd's fastest level; frames are mostly transparent, so it is enough.
const ZSTD_LEVEL: i32 = 1;

static WRITES: AtomicU64 = AtomicU64::new(0);

/// Why a video couldn't be kept or saved.
#[derive(Debug, thiserror::Error)]
pub enum VideoError {
    /// A frame couldn't be compressed to be kept.
    #[error(
        "a frame of the recording's video couldn't be compressed, so the video was dropped; \
         the rest of the report is unaffected. If this keeps happening, the computer may be \
         out of memory: close other apps and record again"
    )]
    Compress(#[source] std::io::Error),
    /// A kept frame couldn't be decompressed to be written.
    #[error(
        "a kept frame of the recording's video couldn't be read back, so the video wasn't \
         saved; the rest of the report is unaffected. This is a bug in Humata; report it \
         without a video"
    )]
    Decompress(#[source] std::io::Error),
    /// The animation couldn't be encoded.
    #[error(
        "the recording's video couldn't be encoded as {}; the rest of the report is unaffected. \
         Record again, and if it fails again, report it without a video",
        path.display()
    )]
    Encode {
        path: PathBuf,
        #[source]
        source: png::EncodingError,
    },
    /// A video file couldn't be written.
    #[error(
        "couldn't write {}; check that the disk has free space and the report folder is \
         writable. The rest of the report is unaffected",
        path.display()
    )]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// What [`Video::write`] saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFiles {
    /// The animated PNG.
    pub png: PathBuf,
    /// The frame list.
    pub jsonl: PathBuf,
    /// How many frames the animation holds.
    pub frames: usize,
    /// The recording time, in milliseconds, the first frame shows; later
    /// than 0 when the oldest frames were folded away.
    pub video_start_ms: u64,
}

/// One kept frame: the box that changed, compressed.
struct Delta {
    at_ms: u64,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    /// Bytes before compression.
    raw_len: usize,
    /// RGBA of the box; pixels that didn't change have alpha 0.
    packed: Vec<u8>,
}

/// The video being recorded.
pub struct Video {
    scale: u32,
    max_frames: usize,
    max_bytes: usize,
    /// Scaled size of every frame, from the first one.
    size: Option<(u32, u32)>,
    /// The picture the first frame shows, after folding.
    base: Vec<u8>,
    base_at_ms: u64,
    /// The picture now, scaled, to diff the next frame against.
    canvas: Vec<u8>,
    deltas: VecDeque<Delta>,
    stored_bytes: usize,
    /// The time of the latest frame given, changed or not.
    last_at_ms: u64,
    skipped: usize,
}

impl Video {
    /// A video of frames drawn at `scale` device pixels to a point (1 when
    /// 0 is given).
    #[must_use]
    pub fn new(scale: u32) -> Self {
        Self::with_limits(scale, MAX_FRAMES, MAX_BYTES)
    }

    /// Like [`Video::new`] but keeping at most `max_frames` frames (at
    /// least 1) and `max_bytes` compressed bytes of changes.
    #[must_use]
    pub fn with_limits(scale: u32, max_frames: usize, max_bytes: usize) -> Self {
        Self {
            scale: scale.max(1),
            max_frames: max_frames.max(1),
            max_bytes,
            size: None,
            base: Vec::new(),
            base_at_ms: 0,
            canvas: Vec::new(),
            deltas: VecDeque::new(),
            stored_bytes: 0,
            last_at_ms: 0,
            skipped: 0,
        }
    }

    /// How many frames the animation would hold now.
    #[must_use]
    pub fn frames(&self) -> usize {
        if self.size.is_some() { 1 + self.deltas.len() } else { 0 }
    }

    /// How many frames were left out because the window had changed size
    /// since the first one.
    #[must_use]
    pub fn skipped(&self) -> usize {
        self.skipped
    }

    /// Adds the window as it was `at_ms` milliseconds into the recording:
    /// `rgba` is `width * height` device pixels, 4 bytes each. A frame the
    /// wrong length or of a different size than the first is left out.
    ///
    /// # Errors
    ///
    /// [`VideoError::Compress`] when the changed part can't be compressed.
    pub fn push(&mut self, at_ms: u64, width: u32, height: u32, rgba: &[u8]) -> Result<(), VideoError> {
        let scaled = scale_down(width, height, rgba, self.scale);
        let Some((sw, sh, pixels)) = scaled else {
            self.skipped += 1;
            return Ok(());
        };
        let at_ms = at_ms.max(self.last_at_ms);
        let Some((cw, ch)) = self.size else {
            self.size = Some((sw, sh));
            self.base.clone_from(&pixels);
            self.base_at_ms = at_ms;
            self.canvas = pixels;
            self.last_at_ms = at_ms;
            return Ok(());
        };
        if (sw, sh) != (cw, ch) {
            self.skipped += 1;
            return Ok(());
        }
        self.last_at_ms = at_ms;

        let Some((x, y, w, h)) = changed_box(&self.canvas, &pixels, cw, ch) else {
            return Ok(());
        };
        let mut delta = Vec::with_capacity(w as usize * h as usize * 4);
        for row in y..y + h {
            let start = (row as usize * cw as usize + x as usize) * 4;
            let end = start + w as usize * 4;
            for (old, new) in self.canvas[start..end].chunks_exact(4).zip(pixels[start..end].chunks_exact(4)) {
                if old == new {
                    delta.extend_from_slice(&[0, 0, 0, 0]);
                } else {
                    delta.extend_from_slice(new);
                }
            }
        }
        let raw_len = delta.len();
        let packed = zstd::bulk::compress(&delta, ZSTD_LEVEL).map_err(VideoError::Compress)?;
        self.stored_bytes += packed.len();
        self.deltas.push_back(Delta {
            at_ms,
            x,
            y,
            width: w,
            height: h,
            raw_len,
            packed,
        });
        self.canvas = pixels;
        self.fold()
    }

    /// Folds the oldest changes into the first frame until the video is
    /// within its limits.
    fn fold(&mut self) -> Result<(), VideoError> {
        let Some((cw, _)) = self.size else {
            return Ok(());
        };
        while (1 + self.deltas.len() > self.max_frames || self.stored_bytes > self.max_bytes)
            && let Some(oldest) = self.deltas.pop_front()
        {
            self.stored_bytes = self.stored_bytes.saturating_sub(oldest.packed.len());
            let raw = zstd::bulk::decompress(&oldest.packed, oldest.raw_len).map_err(VideoError::Decompress)?;
            composite(&mut self.base, cw, &oldest, &raw);
            self.base_at_ms = oldest.at_ms;
        }
        Ok(())
    }

    /// Saves `video.png` and `video.jsonl` in `dir`, which must exist, each
    /// readable only by this user. Returns what was saved, or `None` when
    /// no frame was ever given (nothing is written).
    ///
    /// # Errors
    ///
    /// [`VideoError`] when a frame can't be read back, the animation can't
    /// be encoded or a file can't be written. A failed write leaves neither
    /// file.
    pub fn write(self, dir: &Path) -> Result<Option<VideoFiles>, VideoError> {
        let Some((width, height)) = self.size else {
            return Ok(None);
        };
        let frames = self.frames();
        let png_path = dir.join(PNG);
        let jsonl_path = dir.join(JSONL);

        // The time each frame shows until: the next frame's, and for the
        // last one the latest sample (at least one sample interval).
        let mut starts: Vec<u64> = vec![self.base_at_ms];
        starts.extend(self.deltas.iter().map(|delta| delta.at_ms));
        let delay_ms = |index: usize| -> u64 {
            match starts.get(index + 1) {
                Some(next) => next.saturating_sub(starts[index]),
                None => self.last_at_ms.saturating_sub(starts[index]).max(SAMPLE_MS),
            }
        };

        let mut lines = String::new();
        let mut line = |index: usize, rect: (u32, u32, u32, u32)| {
            let entry = json!({
                "index": index,
                "at_ms": starts[index],
                "delay_ms": delay_ms(index),
                "x": rect.0,
                "y": rect.1,
                "width": rect.2,
                "height": rect.3,
                "video_start_ms": self.base_at_ms,
            });
            lines.push_str(&entry.to_string());
            lines.push('\n');
        };
        line(0, (0, 0, width, height));
        for (offset, delta) in self.deltas.iter().enumerate() {
            line(offset + 1, (delta.x, delta.y, delta.width, delta.height));
        }

        let encode = |source: png::EncodingError| VideoError::Encode {
            path: png_path.clone(),
            source,
        };
        let mut bytes: Vec<u8> = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_compression(png::Compression::Fast);
            let count = u32::try_from(frames).unwrap_or(u32::MAX);
            // Plays once.
            encoder.set_animated(count, 1).map_err(encode)?;
            encoder.set_frame_delay(centis(delay_ms(0)), 100).map_err(encode)?;
            let mut writer = encoder.write_header().map_err(encode)?;
            writer.write_image_data(&self.base).map_err(encode)?;
            for (offset, delta) in self.deltas.iter().enumerate() {
                let raw = zstd::bulk::decompress(&delta.packed, delta.raw_len).map_err(VideoError::Decompress)?;
                writer.set_frame_delay(centis(delay_ms(offset + 1)), 100).map_err(encode)?;
                // Back to the corner first, so the new size is never checked
                // against the last frame's offset.
                writer.set_frame_position(0, 0).map_err(encode)?;
                writer.set_frame_dimension(delta.width, delta.height).map_err(encode)?;
                writer.set_frame_position(delta.x, delta.y).map_err(encode)?;
                writer.set_blend_op(png::BlendOp::Over).map_err(encode)?;
                writer.set_dispose_op(png::DisposeOp::None).map_err(encode)?;
                writer.write_image_data(&raw).map_err(encode)?;
            }
            writer.finish().map_err(encode)?;
        }

        write_file(dir, PNG, &bytes)?;
        if let Err(error) = write_file(dir, JSONL, lines.as_bytes()) {
            // Keep the pair whole.
            let _ = fs::remove_file(&png_path);
            return Err(error);
        }
        Ok(Some(VideoFiles {
            png: png_path,
            jsonl: jsonl_path,
            frames,
            video_start_ms: self.base_at_ms,
        }))
    }
}

/// `ms` in hundredths of a second, at least 1.
fn centis(ms: u64) -> u16 {
    u16::try_from((ms / 10).max(1)).unwrap_or(u16::MAX)
}

/// `rgba` averaged down by `scale` in each direction, with alpha 255, as
/// `(width, height, pixels)`; `None` when it is empty or not `width *
/// height` pixels.
fn scale_down(width: u32, height: u32, rgba: &[u8], scale: u32) -> Option<(u32, u32, Vec<u8>)> {
    let (w, h, s) = (width as usize, height as usize, scale as usize);
    if rgba.len() != w.checked_mul(h)?.checked_mul(4)? {
        return None;
    }
    let (sw, sh) = (w / s, h / s);
    if sw == 0 || sh == 0 {
        return None;
    }
    let area = (s * s) as u32;
    let mut out = Vec::with_capacity(sw * sh * 4);
    for oy in 0..sh {
        for ox in 0..sw {
            let mut sum = [0u32; 3];
            for dy in 0..s {
                let row = ((oy * s + dy) * w + ox * s) * 4;
                for pixel in rgba[row..row + s * 4].chunks_exact(4) {
                    sum[0] += u32::from(pixel[0]);
                    sum[1] += u32::from(pixel[1]);
                    sum[2] += u32::from(pixel[2]);
                }
            }
            // Rounded to nearest; each average is at most 255.
            let avg = |total: u32| u8::try_from((total + area / 2) / area).unwrap_or(u8::MAX);
            out.extend_from_slice(&[avg(sum[0]), avg(sum[1]), avg(sum[2]), 255]);
        }
    }
    Some((sw as u32, sh as u32, out))
}

/// The smallest box `(x, y, width, height)` holding every pixel that
/// differs between `old` and `new`, which are `width * height` RGBA; `None`
/// when they match.
fn changed_box(old: &[u8], new: &[u8], width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
    let (w, h) = (width as usize, height as usize);
    let mut bounds: Option<(usize, usize, usize, usize)> = None;
    for y in 0..h {
        let row = y * w * 4;
        let (a, b) = (&old[row..row + w * 4], &new[row..row + w * 4]);
        if a == b {
            continue;
        }
        let first = a.chunks_exact(4).zip(b.chunks_exact(4)).position(|(p, q)| p != q)?;
        let last = a.chunks_exact(4).zip(b.chunks_exact(4)).rposition(|(p, q)| p != q)?;
        bounds = Some(match bounds {
            None => (first, y, last, y),
            Some((left, top, right, _)) => (left.min(first), top, right.max(last), y),
        });
    }
    let (left, top, right, bottom) = bounds?;
    Some((
        u32::try_from(left).ok()?,
        u32::try_from(top).ok()?,
        u32::try_from(right - left + 1).ok()?,
        u32::try_from(bottom - top + 1).ok()?,
    ))
}

/// Paints `delta`'s changed pixels (`raw`) onto `canvas`, `width` wide.
fn composite(canvas: &mut [u8], width: u32, delta: &Delta, raw: &[u8]) {
    for (row, source) in raw.chunks_exact(delta.width as usize * 4).enumerate() {
        let start = ((delta.y as usize + row) * width as usize + delta.x as usize) * 4;
        let Some(target) = canvas.get_mut(start..start + source.len()) else {
            continue;
        };
        for (to, from) in target.chunks_exact_mut(4).zip(source.chunks_exact(4)) {
            if from[3] != 0 {
                to.copy_from_slice(from);
            }
        }
    }
}

/// Writes `dir/name` whole through a temporary file, readable only by this
/// user; the target is never left half-written.
fn write_file(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), VideoError> {
    let target = dir.join(name);
    let temp = dir.join(format!(
        ".{name}.{}-{}.partial",
        std::process::id(),
        WRITES.fetch_add(1, Ordering::Relaxed)
    ));
    let written = (|| {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        file.set_permissions(Permissions::from_mode(0o600))?;
        let mut out = BufWriter::new(file);
        out.write_all(bytes)?;
        out.flush()?;
        out.get_ref().sync_all()?;
        fs::rename(&temp, &target)?;
        File::open(dir)?.sync_all()
    })();
    if let Err(source) = written {
        let _ = fs::remove_file(&temp);
        return Err(VideoError::Write { path: target, source });
    }
    Ok(())
}

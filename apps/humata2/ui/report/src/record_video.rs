//! The video of a recording, sampled from the window while it runs.
//!
//! The UI thread only renders a frame and hands it over; a background thread
//! scales, compares and keeps it (`humata2_report::video`). The hand-over is
//! a queue of two, and a frame offered while it is full is dropped, so a
//! slow thread never slows the window.

use std::cell::Cell;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use humata2_report::video::{SAMPLE_MS, Video, VideoError, VideoFiles};
use image::RgbaImage;

/// How many rendered frames wait for the background thread.
const QUEUE: usize = 2;

/// Why the video of a recording couldn't be saved. The text report is never
/// affected.
#[derive(Debug, thiserror::Error)]
pub enum SamplerError {
    /// The video couldn't be kept or written.
    #[error("The recording's video wasn't saved. {0}")]
    Video(#[source] VideoError),
    /// The thread keeping the frames stopped unexpectedly.
    #[error(
        "The recording's video wasn't saved because the thread keeping its frames stopped \
         unexpectedly. The rest of the report is unaffected; record again to get a video"
    )]
    Panicked,
}

struct Frame {
    at_ms: u64,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// Takes frames of the window at a fixed rate while a recording runs.
pub struct Sampler {
    sender: SyncSender<Frame>,
    thread: JoinHandle<(Video, Option<VideoError>)>,
    last: Cell<Option<Instant>>,
}

impl Sampler {
    /// Starts the thread that keeps frames drawn at `scale_factor` device
    /// pixels to a point (rounded to a whole number, at least 1). `None`
    /// when the thread can't be started; the recording goes on without a
    /// video.
    pub fn start(scale_factor: f32) -> Option<Self> {
        let scale = scale_factor.round().max(1.0);
        // Scale factors are small whole numbers; a huge one is clamped.
        let scale = if scale >= 16.0 { 16 } else { scale as u32 };
        let (sender, receiver) = mpsc::sync_channel::<Frame>(QUEUE);
        let thread = thread::Builder::new()
            .name("humata-record-video".to_owned())
            .spawn(move || keep(scale, &receiver))
            .ok()?;
        Some(Self {
            sender,
            thread,
            last: Cell::new(None),
        })
    }

    /// Whether a frame is due at `now`: the first one always, then one every
    /// 200 ms. Answering yes claims the slot, so ask once a frame.
    pub fn due(&self, now: Instant) -> bool {
        let due = self
            .last
            .get()
            .is_none_or(|last| now.saturating_duration_since(last) >= Duration::from_millis(SAMPLE_MS));
        if due {
            self.last.set(Some(now));
        }
        due
    }

    /// Hands over the window `at_ms` into the recording, secrets already
    /// masked. Dropped when the thread is behind or gone.
    pub fn offer(&self, at_ms: u64, image: RgbaImage) {
        let (width, height) = image.dimensions();
        let frame = Frame {
            at_ms,
            width,
            height,
            rgba: image.into_raw(),
        };
        // Full or gone: the sample is dropped, never waited for.
        let _ = self.sender.try_send(frame);
    }

    /// Stops sampling and saves `video.png` and `video.jsonl` in `dir`.
    /// `None` when no frame was taken.
    pub fn finish(self, dir: &Path) -> Result<Option<VideoFiles>, SamplerError> {
        let Self { sender, thread, .. } = self;
        drop(sender);
        let (video, failed) = thread.join().map_err(|_| SamplerError::Panicked)?;
        if let Some(error) = failed {
            return Err(SamplerError::Video(error));
        }
        video.write(dir).map_err(SamplerError::Video)
    }
}

/// The background thread: keeps every frame until the channel closes, and
/// returns the video with the first error that stopped it keeping more.
fn keep(scale: u32, frames: &Receiver<Frame>) -> (Video, Option<VideoError>) {
    let mut video = Video::new(scale);
    let mut failed = None;
    for frame in frames {
        if let Err(error) = video.push(frame.at_ms, frame.width, frame.height, &frame.rgba) {
            failed = Some(error);
            break;
        }
    }
    (video, failed)
}

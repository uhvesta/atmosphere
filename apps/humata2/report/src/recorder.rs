//! The one recording an app runs at a time, started and stopped from the
//! menu, a shortcut or `humata report record|stop`, which all go through
//! here so they agree on whether one is running.

use crate::error::RecorderError;
use crate::event::{Context, Trees};
use crate::recording::{Captured, Limits, Recording};

/// Holds the recording in progress, if any. The title bar's dot shows
/// while [`is_recording`](Self::is_recording) is true. A snapshot needs
/// none of this: take one with [`snapshot`](crate::snapshot) at any time,
/// recording or not.
#[derive(Debug, Default)]
pub struct Recorder {
    limits: Limits,
    active: Option<Recording>,
}

impl Recorder {
    /// No recording yet; recordings it starts keep at most `limits`.
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            active: None,
        }
    }

    /// Starts recording with the trees as they are now.
    ///
    /// # Errors
    ///
    /// [`RecorderError::AlreadyRecording`] when one is already running; it
    /// keeps running.
    pub fn start(&mut self, context: Context, now: Trees) -> Result<(), RecorderError> {
        if let Some(running) = &self.active {
            return Err(RecorderError::AlreadyRecording {
                started_unix_ms: running.context().started_unix_ms,
            });
        }
        self.active = Some(Recording::start(context, now, self.limits));
        Ok(())
    }

    /// Whether a recording is running.
    pub fn is_recording(&self) -> bool {
        self.active.is_some()
    }

    /// The recording in progress, to tell it what happens, or `None` when
    /// nothing is being recorded.
    pub fn recording_mut(&mut self) -> Option<&mut Recording> {
        self.active.as_mut()
    }

    /// Stops the recording at `at_ms` since it started.
    ///
    /// # Errors
    ///
    /// [`RecorderError::NotRecording`] when none is running.
    pub fn stop(&mut self, at_ms: u64) -> Result<Captured, RecorderError> {
        self.active
            .take()
            .map(|recording| recording.stop(at_ms))
            .ok_or(RecorderError::NotRecording)
    }
}

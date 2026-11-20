//! Progress-bounded `fsync` pass-through.
//!
//! Large dirty-file flushes should never fail due to an arbitrary fixed
//! wall-clock timeout while bytes are actively draining. Instead, `fsync`
//! enforces a stall timeout that resets whenever `bytes_flushed` advances.

use std::thread;
use std::time::{Duration, Instant};

use fusefs_vfs::{CallerContext, FileHandle, NodeId, VfsError, VfsResult, VfsTarget, Volume};

/// Policy governing progress-bounded `fsync` execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FsyncPolicy {
    /// Maximum duration allowed with zero forward flush progress before failing with `EIO`.
    pub stall_timeout: Duration,
    /// Interval between progress polls when the flush is still in flight.
    pub poll_interval: Duration,
}

impl Default for FsyncPolicy {
    fn default() -> Self {
        Self {
            stall_timeout: Duration::from_secs(15),
            poll_interval: Duration::from_millis(25),
        }
    }
}

/// Snapshot returned by a flush progress callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FsyncProgress {
    pub bytes_flushed: u64,
    pub done: bool,
}

/// Summary returned upon successful completion of [`progress_bounded_fsync`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FsyncResult {
    pub total_bytes_flushed: u64,
    pub polls: u32,
    pub datasync: bool,
}

/// Single-shot direct `fsync` pass-through to the underlying [`Volume`].
pub fn passthrough_fsync(
    volume: &dyn Volume,
    caller: &CallerContext,
    ino: NodeId,
    fh: FileHandle,
    datasync: bool,
) -> VfsResult<()> {
    volume.fsync(caller, ino, fh, datasync)
}

/// Executes a progress-bounded flush followed by `volume.fsync`.
///
/// The stall timer resets every time `poll_flush()` reports a strictly larger
/// `bytes_flushed` value. If `bytes_flushed` does not advance for
/// `policy.stall_timeout`, this returns [`VfsError::Eio`].
pub fn progress_bounded_fsync<F>(
    volume: &dyn Volume,
    caller: &CallerContext,
    ino: NodeId,
    fh: FileHandle,
    datasync: bool,
    policy: FsyncPolicy,
    mut poll_flush: F,
) -> VfsResult<FsyncResult>
where
    F: FnMut() -> VfsResult<FsyncProgress>,
{
    let mut last_bytes = 0u64;
    let mut last_progress_at = Instant::now();
    let mut polls = 0u32;

    loop {
        polls = polls.saturating_add(1);
        let progress = poll_flush()?;
        if progress.bytes_flushed > last_bytes {
            last_bytes = progress.bytes_flushed;
            last_progress_at = Instant::now();
        }
        if progress.done {
            volume.fsync(caller, ino, fh, datasync)?;
            return Ok(FsyncResult {
                total_bytes_flushed: last_bytes,
                polls,
                datasync,
            });
        }
        if last_progress_at.elapsed() >= policy.stall_timeout {
            return Err(VfsError::Eio {
                op: "fsync (no progress)",
                target: VfsTarget::Flush {
                    ino,
                    bytes_flushed: last_bytes,
                },
            });
        }
        if !policy.poll_interval.is_zero() {
            thread::sleep(policy.poll_interval);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fusefs_vfs::{InMemoryVolume, OpenFlags, ROOT_NODE_ID};

    #[test]
    fn progress_bounded_fsync_succeeds_across_slow_steady_progress() {
        let vol = InMemoryVolume::builder()
            .file("/data.bin", b"hello".to_vec())
            .build()
            .unwrap();
        let caller = CallerContext::new(1, 501, 20);
        let attr = vol.lookup(&caller, ROOT_NODE_ID, "data.bin").unwrap();
        let fh = vol
            .open(&caller, attr.ino, OpenFlags::read_write())
            .unwrap();

        // Total duration (~50ms) exceeds stall_timeout (25ms), yet succeeds
        // because each poll advances `bytes_flushed`.
        let policy = FsyncPolicy {
            stall_timeout: Duration::from_millis(25),
            poll_interval: Duration::from_millis(10),
        };
        let mut step = 0u64;
        let res = progress_bounded_fsync(&vol, &caller, attr.ino, fh, true, policy, || {
            step += 1;
            Ok(FsyncProgress {
                bytes_flushed: step * 1024,
                done: step >= 5,
            })
        })
        .expect("steady progress must not time out");

        assert_eq!(res.total_bytes_flushed, 5 * 1024);
        assert_eq!(res.polls, 5);
        assert!(res.datasync);
    }

    #[test]
    fn progress_bounded_fsync_fails_on_stall() {
        let vol = InMemoryVolume::builder()
            .file("/stall.bin", b"hello".to_vec())
            .build()
            .unwrap();
        let caller = CallerContext::new(1, 501, 20);
        let attr = vol.lookup(&caller, ROOT_NODE_ID, "stall.bin").unwrap();
        let fh = vol
            .open(&caller, attr.ino, OpenFlags::read_write())
            .unwrap();

        let policy = FsyncPolicy {
            stall_timeout: Duration::from_millis(20),
            poll_interval: Duration::from_millis(5),
        };
        let err = progress_bounded_fsync(&vol, &caller, attr.ino, fh, false, policy, || {
            Ok(FsyncProgress {
                bytes_flushed: 512,
                done: false,
            })
        })
        .unwrap_err();

        assert!(matches!(err, VfsError::Eio { .. }));
    }
}

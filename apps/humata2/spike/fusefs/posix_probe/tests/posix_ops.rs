//! Gate T1 POSIX probe:
//! 1. `sed -i` / atomic temp-file + `fsync` + `rename` + parent-dir `fsync` with
//!    `expected_generation` CAS (`ESTALE` on concurrent edit).
//! 2. `cp -r` recursive copy under a turn-bounded target-directory subtree lease
//!    (in-subtree allowed; out-of-subtree denied with `EACCES`).
//! 3. Progress-bounded `fsync` semantics (spec §7.1): active draining over a
//!    throttled link returns `Ok(())` (POSIX `0`) even when total wall time
//!    exceeds the stall bound; a progress stall exceeding the bound returns
//!    `Err(FsyncError::LinkLost)` (`ENETUNREACH` / `EIO`), never `EAGAIN`.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Explicit typed error enum for the POSIX lease & fsync probe.
#[derive(Debug, PartialEq, Eq)]
enum ProbeError {
    /// Target path is outside the approved subtree lease (POSIX `EACCES`).
    PermissionDenied { path: PathBuf },
    /// Lease expired before operation completed (POSIX `EACCES`).
    LeaseExpired,
    /// File generation changed concurrently before atomic rename (POSIX `ESTALE`).
    StaleGeneration { expected: u64, actual: u64 },
    /// Remote `fsync` stalled without forward progress past the stall bound
    /// (POSIX `ENETUNREACH` / `EIO`, never `EAGAIN`).
    FsyncLinkLost { stalled_for: Duration },
    /// Underlying OS I/O error.
    Io(io::ErrorKind),
}

impl From<io::Error> for ProbeError {
    fn from(err: io::Error) -> Self {
        Self::Io(err.kind())
    }
}

/// Turn-bounded target-directory subtree lease (spec §7.1).
struct SubtreeLease {
    approved_root: PathBuf,
    expires_at: Instant,
    generations: HashMap<PathBuf, u64>,
}

impl SubtreeLease {
    fn new(approved_root: PathBuf, ttl: Duration) -> Self {
        Self {
            approved_root,
            expires_at: Instant::now() + ttl,
            generations: HashMap::new(),
        }
    }

    fn check_path(&self, path: &Path) -> Result<(), ProbeError> {
        if Instant::now() >= self.expires_at {
            return Err(ProbeError::LeaseExpired);
        }
        if !path.starts_with(&self.approved_root) {
            return Err(ProbeError::PermissionDenied {
                path: path.to_path_buf(),
            });
        }
        Ok(())
    }

    fn current_generation(&self, path: &Path) -> u64 {
        self.generations.get(path).copied().unwrap_or(0)
    }

    /// Simulates an external/concurrent modification bumping the file generation.
    fn bump_external_generation(&mut self, path: &Path) {
        let entry = self.generations.entry(path.to_path_buf()).or_insert(0);
        *entry += 1;
    }

    /// Performs `sed -i`-style atomic write:
    /// random temp file in target dir -> `write` -> `fsync` -> generation check
    /// -> atomic `rename` -> parent dir `fsync`.
    fn atomic_replace(
        &mut self,
        target: &Path,
        expected_generation: u64,
        new_contents: &[u8],
    ) -> Result<u64, ProbeError> {
        self.check_path(target)?;
        let parent = target.parent().expect("target has parent");
        self.check_path(parent)?;

        let tmp_name = format!(
            ".{}.sed-tmp.{}",
            target.file_name().expect("file name").to_string_lossy(),
            std::process::id()
        );
        let tmp_path = parent.join(tmp_name);

        let mut tmp_file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp_path)?;
        tmp_file.write_all(new_contents)?;
        tmp_file.sync_all()?;
        drop(tmp_file);

        let actual = self.current_generation(target);
        if actual != expected_generation {
            let _ = fs::remove_file(&tmp_path);
            return Err(ProbeError::StaleGeneration {
                expected: expected_generation,
                actual,
            });
        }

        fs::rename(&tmp_path, target)?;

        // Sync parent directory to persist the directory entry rename.
        let dir_fd = File::open(parent)?;
        rustix::fs::fsync(&dir_fd).map_err(|e| ProbeError::Io(io::Error::from(e).kind()))?;

        let next_gen = actual + 1;
        self.generations.insert(target.to_path_buf(), next_gen);
        Ok(next_gen)
    }

    /// Recursive `cp -r` under a single subtree lease approval.
    fn copy_tree(&mut self, src: &Path, dst: &Path) -> Result<usize, ProbeError> {
        self.check_path(dst)?;
        let meta = fs::symlink_metadata(src)?;
        if meta.is_dir() {
            fs::create_dir_all(dst)?;
            let mut copied = 0;
            for entry in fs::read_dir(src)? {
                let entry = entry?;
                copied += self.copy_tree(&entry.path(), &dst.join(entry.file_name()))?;
            }
            Ok(copied)
        } else if meta.file_type().is_symlink() {
            let target = fs::read_link(src)?;
            std::os::unix::fs::symlink(target, dst)?;
            Ok(1)
        } else {
            let bytes = fs::read(src)?;
            let generation = self.current_generation(dst);
            let _ = self.atomic_replace(dst, generation, &bytes)?;
            Ok(1)
        }
    }
}

/// Progress-bounded `fsync` state machine (spec §7.1):
/// Accepts a sequence of `(step_delay, bytes_acked)` drain events.
/// Returns `Ok(total_bytes)` as long as no single step stalls past `stall_bound`,
/// even if the sum of all active drain steps exceeds `stall_bound`.
fn progress_bounded_fsync(
    total_bytes: usize,
    stall_bound: Duration,
    drain_steps: &[(Duration, usize)],
) -> Result<usize, ProbeError> {
    let mut acked = 0usize;
    let mut since_progress = Duration::ZERO;

    for &(step_duration, bytes) in drain_steps {
        if bytes > 0 {
            if since_progress + step_duration > stall_bound {
                return Err(ProbeError::FsyncLinkLost {
                    stalled_for: since_progress + step_duration,
                });
            }
            acked = acked.saturating_add(bytes);
            since_progress = Duration::ZERO;
            if acked >= total_bytes {
                return Ok(total_bytes);
            }
        } else {
            since_progress += step_duration;
            if since_progress > stall_bound {
                return Err(ProbeError::FsyncLinkLost {
                    stalled_for: since_progress,
                });
            }
        }
    }

    if acked >= total_bytes {
        Ok(total_bytes)
    } else {
        Err(ProbeError::FsyncLinkLost {
            stalled_for: stall_bound,
        })
    }
}

#[test]
fn sed_i_atomic_rename_replaces_inode_and_enforces_estale() {
    let dir = tempfile::tempdir().expect("tempdir");
    let approved = dir.path().join("approved");
    fs::create_dir_all(&approved).expect("mkdir");
    let target = approved.join("config.toml");
    fs::write(&target, "mode = \"old\"\n").expect("seed file");
    let old_ino = fs::metadata(&target).expect("stat old").ino();

    let mut lease = SubtreeLease::new(approved.clone(), Duration::from_secs(600));

    // First atomic replace succeeds with expected generation 0 -> 1.
    let gen1 = lease
        .atomic_replace(&target, 0, b"mode = \"new\"\n")
        .expect("atomic replace succeeds");
    assert_eq!(gen1, 1);
    assert_eq!(fs::read_to_string(&target).expect("read"), "mode = \"new\"\n");
    let new_ino = fs::metadata(&target).expect("stat new").ino();
    assert_ne!(old_ino, new_ino, "atomic rename must replace inode");

    // Concurrent edit bumps generation to 2; stale writer with expected=1 gets ESTALE.
    lease.bump_external_generation(&target);
    let err = lease
        .atomic_replace(&target, 1, b"mode = \"stale\"\n")
        .expect_err("stale generation must fail");
    assert_eq!(
        err,
        ProbeError::StaleGeneration {
            expected: 1,
            actual: 2,
        }
    );
    // Original content remains untouched after ESTALE rejection.
    assert_eq!(fs::read_to_string(&target).expect("read"), "mode = \"new\"\n");
}

#[test]
fn cp_r_succeeds_inside_subtree_lease_and_rejects_outside() {
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("src_tree");
    fs::create_dir_all(src.join("nested/sub")).expect("mkdir src");
    fs::write(src.join("a.txt"), "alpha\n").expect("write a");
    fs::write(src.join("nested/b.txt"), "beta\n").expect("write b");
    fs::write(src.join("nested/sub/c.txt"), "gamma\n").expect("write c");
    std::os::unix::fs::symlink("a.txt", src.join("link_a")).expect("symlink");

    let approved = dir.path().join("leased_dir");
    fs::create_dir_all(&approved).expect("mkdir approved");
    let mut lease = SubtreeLease::new(approved.clone(), Duration::from_secs(600));

    let dst = approved.join("copied");
    let count = lease.copy_tree(&src, &dst).expect("cp -r inside lease");
    assert_eq!(count, 4);
    assert_eq!(
        fs::read_to_string(dst.join("nested/sub/c.txt")).expect("read c"),
        "gamma\n"
    );
    assert_eq!(
        fs::read_link(dst.join("link_a")).expect("readlink"),
        PathBuf::from("a.txt")
    );

    // Attempting to copy outside the approved subtree returns EACCES.
    let outside = dir.path().join("unapproved/copied");
    let err = lease
        .copy_tree(&src, &outside)
        .expect_err("outside lease must fail");
    assert_eq!(err, ProbeError::PermissionDenied { path: outside });
}

#[test]
fn progress_bounded_fsync_allows_slow_active_drain_and_fails_on_stall() {
    let stall_bound = Duration::from_millis(1_000);

    // Active drain: 6 chunks x 256 KiB taking 300 ms each = 1,800 ms total (> 1,000 ms),
    // but each chunk makes steady forward progress well within the 1,000 ms stall bound.
    let slow_active_steps = vec![(Duration::from_millis(300), 256 * 1024); 6];
    let drained = progress_bounded_fsync(6 * 256 * 1024, stall_bound, &slow_active_steps)
        .expect("active drain must return Ok (POSIX 0), never EAGAIN");
    assert_eq!(drained, 6 * 256 * 1024);

    // Link stall: first chunk succeeds, then zero bytes acked for 1,200 ms (> 1,000 ms).
    let stalled_steps = vec![
        (Duration::from_millis(200), 256 * 1024),
        (Duration::from_millis(600), 0),
        (Duration::from_millis(600), 0),
    ];
    let err = progress_bounded_fsync(2 * 256 * 1024, stall_bound, &stalled_steps)
        .expect_err("progress stall must fail with LinkLost");
    assert_eq!(
        err,
        ProbeError::FsyncLinkLost {
            stalled_for: Duration::from_millis(1_200),
        }
    );
}

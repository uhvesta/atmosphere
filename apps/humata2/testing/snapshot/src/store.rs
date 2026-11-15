//! Where snapshots live, and recording or checking one.
//!
//! A test file's snapshots sit beside it, in `__snapshots__/<file stem>/`.
//! When a check fails, what the test produced goes to that directory's
//! `.failed/`, which ignores itself in git, so it can be read beside the
//! recorded snapshot.

use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::diff;
use crate::mode::{Mode, ModeError};

/// The directory beside a test that holds its snapshots.
const DIRECTORY: &str = "__snapshots__";
/// The directory, inside a test file's snapshots, for failed checks' output.
const FAILED: &str = ".failed";
/// Unchanged lines shown around each change in a mismatch's diff.
const CONTEXT: usize = 3;

/// The snapshots of one test file, checked or recorded in one [`Mode`].
#[derive(Clone, Debug)]
pub struct Snapshots {
    directory: PathBuf,
    mode: Mode,
}

/// What a check did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The output matched the snapshot. Nothing was written.
    Matched,
    /// The snapshot was missing or different, and the output was written as
    /// the new snapshot. Only happens in [`Mode::Record`].
    Recorded,
}

/// Why a snapshot could not be checked or recorded.
#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error(transparent)]
    Mode(#[from] ModeError),

    #[error(
        "could not find the test file {file} from {manifest_directory}, so there is nowhere to \
         keep its snapshots. Build the test with cargo from its workspace, or set \
         BUILD_WORKSPACE_DIRECTORY to the repository root when the build tool runs it elsewhere"
    )]
    TestFileNotFound {
        file: PathBuf,
        manifest_directory: PathBuf,
    },

    #[error(
        "{name:?} can't name a snapshot: use letters, digits, `-`, `_` and `.`, not starting \
         with `.`, so it is one file in __snapshots__"
    )]
    InvalidName { name: String },

    #[error(
        "there is no snapshot at {path} yet. What the test produced is at {review}. Read it; if \
         it is what the test should produce, run the test again with HUMATA2_SNAPSHOT=record to \
         save it, then commit the new file"
    )]
    Missing { path: PathBuf, review: PathBuf },

    #[error(
        "the snapshot at {path} does not match what the test produced (- recorded, + now):\n\
         {diff}\
         What the test produced is at {review}. If the change is intended, run the test again \
         with HUMATA2_SNAPSHOT=record, then read `git diff` of the __snapshots__ directory \
         before committing it. Don't re-record just to make the test pass: that accepts \
         whatever the code does now, bugs included"
    )]
    Mismatch {
        path: PathBuf,
        diff: String,
        review: PathBuf,
    },

    #[error(
        "the image at {path} is {recorded_width}x{recorded_height} but the test drew \
         {width}x{height}. Old and new, side by side, are at {review}. If the view's size \
         changed on purpose, run the test again with HUMATA2_SNAPSHOT=record and look at the \
         new image before committing it. Don't re-record just to make the test pass: that \
         accepts whatever the code draws now, bugs included"
    )]
    ImageSize {
        path: PathBuf,
        recorded_width: u32,
        recorded_height: u32,
        width: u32,
        height: u32,
        review: PathBuf,
    },

    #[error(
        "the image at {path} differs from what the test drew in {differing} of {total} pixels, \
         more than the {allowed} allowed. Old, new and their diff (changed pixels in red) are \
         side by side at {review}. If the change is intended, run the test again with \
         HUMATA2_SNAPSHOT=record and look at the new image before committing it. Don't \
         re-record just to make the test pass, and don't widen the tolerance to hide a real \
         change: both accept whatever the code draws now, bugs included"
    )]
    ImageMismatch {
        path: PathBuf,
        differing: u64,
        total: u64,
        allowed: u64,
        review: PathBuf,
    },

    #[error("could not read the snapshot at {path}: {source}. Check the file's permissions")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error(
        "could not write {path}: {source}. Check that the directory is writable and the disk \
         is not full"
    )]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error(
        "the snapshot at {path} is not a PNG image that can be read: {source}. If it was \
         damaged, restore it from git; if it was never valid, delete it and record it again \
         with HUMATA2_SNAPSHOT=record, then look at the new image before committing it"
    )]
    DecodeImage {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },

    #[error("could not encode {path} as PNG: {source}. This is a bug in the snapshot tool")]
    EncodeImage {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },
}

impl Snapshots {
    /// The snapshots of the test file `file`, as `file!()` gives it, in a
    /// package whose manifest is in `manifest_directory`, in the mode
    /// `HUMATA2_SNAPSHOT` sets. [`snapshots!`](crate::snapshots) calls this.
    ///
    /// # Errors
    ///
    /// [`SnapshotError::Mode`] for an unknown mode, or
    /// [`SnapshotError::TestFileNotFound`].
    pub fn beside(file: &str, manifest_directory: &str) -> Result<Self, SnapshotError> {
        let mode = Mode::from_env()?;
        let test_file = locate(Path::new(file), Path::new(manifest_directory))?;
        let stem = test_file
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let parent = test_file.parent().unwrap_or(Path::new("."));
        Ok(Self::at(parent.join(DIRECTORY).join(stem), mode))
    }

    fn at(directory: PathBuf, mode: Mode) -> Self {
        Self { directory, mode }
    }

    /// The same snapshots in another mode.
    #[cfg(test)]
    pub(crate) fn with_mode(&self, mode: Mode) -> Self {
        Self::at(self.directory.clone(), mode)
    }

    /// Whether checks record or verify.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Checks `actual` against the text snapshot `name`, or records it.
    ///
    /// # Errors
    ///
    /// [`SnapshotError::Missing`] or [`SnapshotError::Mismatch`] when
    /// verifying, or an error reading or writing the files.
    pub fn check_text(&self, name: &str, actual: &str) -> Result<Outcome, SnapshotError> {
        self.check_text_file(&format!("{}.txt", checked(name)?), actual)
    }

    /// Checks `actual` against the text snapshot in `file`, a name
    /// [`checked`] already, or records it.
    pub(crate) fn check_text_file(&self, file: &str, actual: &str) -> Result<Outcome, SnapshotError> {
        let path = self.directory.join(file);
        let actual = normalize(actual);
        let recorded = read(&path)?.map(|bytes| normalize(&String::from_utf8_lossy(&bytes)));
        if recorded.as_deref() == Some(actual.as_str()) {
            self.clear_failed(&[file])?;
            return Ok(Outcome::Matched);
        }
        match self.mode {
            Mode::Record => {
                write(&path, actual.as_bytes())?;
                self.clear_failed(&[file])?;
                Ok(Outcome::Recorded)
            }
            Mode::Verify => {
                let review = self.write_failed(file, actual.as_bytes())?;
                Err(match recorded {
                    None => SnapshotError::Missing { path, review },
                    Some(recorded) => SnapshotError::Mismatch {
                        diff: diff::render(&diff::lines(&recorded, &actual), CONTEXT, "    "),
                        path,
                        review,
                    },
                })
            }
        }
    }

    /// Like [`check_text`](Self::check_text), panicking with what failed and
    /// how to fix it.
    #[track_caller]
    pub fn assert_text(&self, name: &str, actual: &str) {
        if let Err(error) = self.check_text(name, actual) {
            panic!("{error}");
        }
    }

    #[cfg_attr(all(not(target_os = "macos"), not(test)), allow(dead_code))]
    pub(crate) fn path(&self, file: &str) -> PathBuf {
        self.directory.join(file)
    }

    /// Writes a failed check's output to `.failed/`, which ignores itself in
    /// git, and returns its path.
    pub(crate) fn write_failed(&self, file: &str, bytes: &[u8]) -> Result<PathBuf, SnapshotError> {
        let failed = self.directory.join(FAILED);
        write(&failed.join(".gitignore"), b"*\n")?;
        let path = failed.join(file);
        write(&path, bytes)?;
        Ok(path)
    }

    /// Removes what earlier failed checks of these files left in `.failed/`.
    pub(crate) fn clear_failed(&self, files: &[&str]) -> Result<(), SnapshotError> {
        for file in files {
            let path = self.directory.join(FAILED).join(file);
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(source) => return Err(SnapshotError::Write { path, source }),
            }
        }
        Ok(())
    }
}

/// `name` when it is one plain file name.
pub(crate) fn checked(name: &str) -> Result<&str, SnapshotError> {
    let plain = !name.is_empty()
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if plain {
        Ok(name)
    } else {
        Err(SnapshotError::InvalidName {
            name: name.to_owned(),
        })
    }
}

/// Finds the test file `file!()` names. Cargo gives it relative to the
/// workspace root, which is an ancestor of the manifest directory; Bazel's
/// `bazel run` gives it relative to BUILD_WORKSPACE_DIRECTORY.
fn locate(file: &Path, manifest_directory: &Path) -> Result<PathBuf, SnapshotError> {
    if file.is_absolute() {
        if file.is_file() {
            return Ok(file.to_owned());
        }
    } else {
        let workspace = std::env::var_os("BUILD_WORKSPACE_DIRECTORY").map(PathBuf::from);
        let roots = workspace
            .iter()
            .map(PathBuf::as_path)
            .chain(manifest_directory.ancestors());
        for root in roots {
            let candidate = root.join(file);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(SnapshotError::TestFileNotFound {
        file: file.to_owned(),
        manifest_directory: manifest_directory.to_owned(),
    })
}

/// Text as a snapshot holds it: `\n` line endings, ending in one `\n`.
fn normalize(text: &str) -> String {
    let mut text = text.replace("\r\n", "\n");
    let kept = text.trim_end_matches('\n').len();
    text.truncate(kept);
    text.push('\n');
    text
}

/// The file's bytes, or `None` when there is no file.
pub(crate) fn read(path: &Path) -> Result<Option<Vec<u8>>, SnapshotError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(SnapshotError::Read {
            path: path.to_owned(),
            source,
        }),
    }
}

/// Writes `bytes` to `path`, making its directory first.
pub(crate) fn write(path: &Path, bytes: &[u8]) -> Result<(), SnapshotError> {
    let failed = |source| SnapshotError::Write {
        path: path.to_owned(),
        source,
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(failed)?;
    }
    std::fs::write(path, bytes).map_err(failed)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A fresh directory under the system's temporary directory.
    pub(crate) fn scratch(label: &str) -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "humata2-snapshot-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    pub(crate) fn snapshots(label: &str, mode: Mode) -> Snapshots {
        Snapshots::at(scratch(label), mode)
    }

    #[test]
    fn verifying_without_a_snapshot_fails_and_keeps_the_output_for_review() {
        let snapshots = snapshots("missing", Mode::Verify);
        let error = snapshots.check_text("tree", "a\n").err();
        let Some(SnapshotError::Missing { path, review }) = error else {
            panic!("expected a missing snapshot, got {error:?}");
        };
        assert!(!path.exists());
        assert_eq!(std::fs::read_to_string(review).ok().as_deref(), Some("a\n"));
    }

    #[test]
    fn recording_writes_the_snapshot_and_verifying_then_matches() {
        let directory = scratch("record");
        let record = Snapshots::at(directory.clone(), Mode::Record);
        assert_eq!(record.check_text("tree", "a\nb").ok(), Some(Outcome::Recorded));
        assert_eq!(record.check_text("tree", "a\nb").ok(), Some(Outcome::Matched));

        let verify = Snapshots::at(directory, Mode::Verify);
        assert_eq!(verify.check_text("tree", "a\nb\n").ok(), Some(Outcome::Matched));
    }

    #[test]
    fn a_different_output_fails_verification_with_the_changed_lines() {
        let directory = scratch("mismatch");
        let record = Snapshots::at(directory.clone(), Mode::Record);
        assert!(record.check_text("tree", "same\nold\n").is_ok());

        let verify = Snapshots::at(directory.clone(), Mode::Verify);
        let error = verify.check_text("tree", "same\nnew\n").err();
        let Some(SnapshotError::Mismatch { diff, review, .. }) = error else {
            panic!("expected a mismatch, got {error:?}");
        };
        assert!(diff.contains("old") && diff.contains("new"));
        assert!(review.exists());

        // The recorded snapshot is left as it was.
        assert_eq!(
            std::fs::read_to_string(directory.join("tree.txt")).ok().as_deref(),
            Some("same\nold\n")
        );
    }

    #[test]
    fn a_match_clears_what_an_earlier_failure_left() {
        let directory = scratch("clear");
        let record = Snapshots::at(directory.clone(), Mode::Record);
        assert!(record.check_text("tree", "a\n").is_ok());
        let verify = Snapshots::at(directory, Mode::Verify);
        let Some(SnapshotError::Mismatch { review, .. }) = verify.check_text("tree", "b\n").err()
        else {
            panic!("expected a mismatch");
        };
        assert!(review.exists());
        assert!(verify.check_text("tree", "a\n").is_ok());
        assert!(!review.exists());
    }

    #[test]
    fn names_must_be_one_plain_file() {
        let snapshots = snapshots("names", Mode::Record);
        for name in ["", "../escape", ".hidden", "a/b", "with space"] {
            assert!(
                matches!(
                    snapshots.check_text(name, "x"),
                    Err(SnapshotError::InvalidName { .. })
                ),
                "{name:?} was accepted"
            );
        }
        assert!(snapshots.check_text("flow.01-click-add", "x").is_ok());
    }
}

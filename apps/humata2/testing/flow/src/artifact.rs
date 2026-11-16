//! A flow run written for review: the steps and their snapshots in order,
//! as markdown, with images beside it.

use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use humata2_snapshot::{Mode, Outcome, RgbaImage, SnapshotError};
use image::ImageFormat;

use crate::flow::SpecFlow;

/// The markdown file in a run's directory.
const FILE: &str = "flow.md";

/// Makes each staging directory's name unique within the process.
static NEXT: AtomicUsize = AtomicUsize::new(0);

/// How many times [`publish`] moves a run that finished at the same moment
/// out of the way before giving up.
const PUBLISH_ATTEMPTS: usize = 8;

/// Why an artifact could not be written.
#[derive(Debug)]
pub(crate) enum WriteError {
    Io { path: PathBuf, source: io::Error },
    Image { path: PathBuf, source: image::ImageError },
}

/// How a run ended, for the artifact's summary.
pub(crate) enum Ending {
    Passed,
    SnapshotsFailed(usize),
    StoppedAt(usize),
}

pub(crate) struct Artifact {
    /// Where the finished run goes.
    directory: PathBuf,
    /// Where the run is written until it finishes: a directory of its own,
    /// so runs of the same spec at once (tests in one binary) never write
    /// into or empty each other's.
    staging: PathBuf,
    spec: SpecFlow,
    driver: String,
    mode: Mode,
    body: String,
}

impl Artifact {
    /// A run of `spec` that replaces `directory` when it finishes, so
    /// nothing from an earlier run is left in it.
    pub(crate) fn new(
        directory: PathBuf,
        spec: SpecFlow,
        driver: &str,
        mode: Mode,
    ) -> Result<Self, WriteError> {
        let staging = sibling(&directory, "partial");
        std::fs::create_dir_all(&staging).map_err(|source| WriteError::Io {
            path: staging.clone(),
            source,
        })?;
        Ok(Self {
            directory,
            staging,
            spec,
            driver: driver.to_owned(),
            mode,
            body: String::new(),
        })
    }

    pub(crate) fn section(&mut self, title: &str) {
        let _ = write!(self.body, "\n## {title}\n");
    }

    pub(crate) fn view(&mut self, tree: &str) {
        let _ = write!(self.body, "\n```text\n{}\n```\n", tree.trim_end());
    }

    pub(crate) fn layout(&mut self, layout: &str) {
        let _ = write!(self.body, "\nLayout:\n\n```text\n{}\n```\n", layout.trim_end());
    }

    pub(crate) fn snapshot(&mut self, name: &str, outcome: &Result<Outcome, SnapshotError>) {
        match outcome {
            Ok(Outcome::Matched) => {
                let _ = write!(self.body, "\nSnapshot `{name}`: matched.\n");
            }
            Ok(Outcome::Recorded) => {
                let _ = write!(
                    self.body,
                    "\nSnapshot `{name}`: **recorded**. Review it before committing.\n"
                );
            }
            Err(error) => {
                let _ = write!(
                    self.body,
                    "\nSnapshot `{name}`: **failed**.\n\n```text\n{error}\n```\n"
                );
            }
        }
    }

    /// Writes `image` beside the markdown and shows it there.
    pub(crate) fn image(&mut self, name: &str, image: &RgbaImage) -> Result<(), WriteError> {
        let file = format!("{name}.png");
        let path = self.staging.join(&file);
        image
            .save_with_format(&path, ImageFormat::Png)
            .map_err(|source| WriteError::Image {
                path: path.clone(),
                source,
            })?;
        let _ = write!(self.body, "\n![{name}]({file})\n");
        Ok(())
    }

    pub(crate) fn failed(&mut self, error: &str) {
        let _ = write!(self.body, "\n**This step failed:**\n\n```text\n{error}\n```\n");
    }

    /// Writes the markdown, moves the run into its directory and returns
    /// the markdown's path there.
    pub(crate) fn finish(self, ending: Ending) -> Result<PathBuf, WriteError> {
        let result = match ending {
            Ending::Passed => "passed".to_owned(),
            Ending::SnapshotsFailed(1) => "1 snapshot failed".to_owned(),
            Ending::SnapshotsFailed(count) => format!("{count} snapshots failed"),
            Ending::StoppedAt(step) => format!("stopped: step {step} failed"),
        };
        let text = format!(
            "# Flow {}\n\n\
             - Result: {result}\n\
             - Played by: {}\n\
             - GUI engine: {}\n\
             - Snapshot mode: {}\n{}",
            self.spec,
            self.driver,
            humata2_engine::Backend::current(),
            self.mode,
            self.body
        );
        let path = self.staging.join(FILE);
        std::fs::write(&path, text).map_err(|source| WriteError::Io { path, source })?;
        publish(&self.staging, &self.directory)?;
        Ok(self.directory.join(FILE))
    }
}

impl Drop for Artifact {
    /// Removes the staging directory of a run that stopped before it
    /// finished; a finished run has already moved it.
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.staging);
    }
}

/// A new name beside `directory`, hidden and unique to this call.
fn sibling(directory: &Path, kind: &str) -> PathBuf {
    let name = directory
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    directory.with_file_name(format!(".{name}.{}-{id}.{kind}", std::process::id()))
}

/// Moves `staging` to `directory`, replacing the run there. Another run
/// that finished at the same moment may take `directory` between the two
/// renames; its run is moved away in turn, so the last to finish stays.
fn publish(staging: &Path, directory: &Path) -> Result<(), WriteError> {
    let mut attempt = 0;
    loop {
        let error = match std::fs::rename(staging, directory) {
            Ok(()) => return Ok(()),
            Err(error) => error,
        };
        attempt += 1;
        let occupied = matches!(
            error.kind(),
            io::ErrorKind::AlreadyExists | io::ErrorKind::DirectoryNotEmpty
        );
        if !occupied || attempt == PUBLISH_ATTEMPTS {
            return Err(WriteError::Io {
                path: directory.to_owned(),
                source: error,
            });
        }
        let old = sibling(directory, "old");
        match std::fs::rename(directory, &old) {
            Ok(()) => std::fs::remove_dir_all(&old)
                .map_err(|source| WriteError::Io { path: old, source })?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(WriteError::Io {
                    path: directory.to_owned(),
                    source,
                });
            }
        }
    }
}

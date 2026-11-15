//! A flow run written for review: the steps and their snapshots in order,
//! as markdown, with images beside it.

use std::fmt::Write as _;
use std::io;
use std::path::PathBuf;

use humata2_snapshot::{Mode, Outcome, RgbaImage, SnapshotError};
use image::ImageFormat;

use crate::flow::SpecFlow;

/// The markdown file in a run's directory.
const FILE: &str = "flow.md";

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
    directory: PathBuf,
    spec: SpecFlow,
    driver: String,
    mode: Mode,
    body: String,
}

impl Artifact {
    /// A run of `spec` written into `directory`, which is emptied first so
    /// nothing from an earlier run is left in it.
    pub(crate) fn new(
        directory: PathBuf,
        spec: SpecFlow,
        driver: &str,
        mode: Mode,
    ) -> Result<Self, WriteError> {
        match std::fs::remove_dir_all(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(source) => return Err(WriteError::Io { path: directory, source }),
        }
        std::fs::create_dir_all(&directory).map_err(|source| WriteError::Io {
            path: directory.clone(),
            source,
        })?;
        Ok(Self {
            directory,
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
        let path = self.directory.join(&file);
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

    /// Writes the markdown and returns its path.
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
        let path = self.directory.join(FILE);
        std::fs::write(&path, text).map_err(|source| WriteError::Io {
            path: path.clone(),
            source,
        })?;
        Ok(path)
    }
}

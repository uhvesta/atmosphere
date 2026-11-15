//! Playing a flow, snapshotting its steps and writing its artifact.

use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};

use humata2_snapshot::{Outcome, SnapshotError, Snapshots};
use thiserror::Error;

use crate::artifact::{Artifact, Ending, WriteError};
use crate::driver::Driver;
use crate::flow::{Capture, Flow};

/// Overrides where flow artifacts are written.
const OUT_VARIABLE: &str = "HUMATA2_FLOW_OUT";

/// Why a flow run failed. The artifact is written either way, so the run
/// can be read up to where it failed.
#[derive(Debug, Error)]
pub enum FlowError<E: std::error::Error + Send + Sync + 'static> {
    #[error(
        "the flow {flow} failed at {during}: {source}\nThe run up to there is written in \
         {artifact}"
    )]
    Driver {
        flow: String,
        during: String,
        artifact: PathBuf,
        #[source]
        source: E,
    },

    #[error(
        "{} of the flow {flow} failed. Every step and snapshot is written in {artifact}; read \
         it beside the snapshots. Each failure says how to re-record it if the change is \
         intended.\n{}",
        count(.rest),
        list(.first, .rest)
    )]
    Snapshots {
        flow: String,
        artifact: PathBuf,
        #[source]
        first: SnapshotError,
        rest: Vec<SnapshotError>,
    },

    #[error(
        "could not write the flow's artifact at {path}: {source}. Check that the directory is \
         writable and the disk is not full, or set HUMATA2_FLOW_OUT to write artifacts \
         somewhere else"
    )]
    Artifact {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error(
        "could not write the image {path} into the flow's artifact: {source}. Check that the \
         directory is writable and the disk is not full, or set HUMATA2_FLOW_OUT to write \
         artifacts somewhere else"
    )]
    ArtifactImage {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },
}

fn count(rest: &[SnapshotError]) -> String {
    let failed = 1 + rest.len();
    if failed == 1 {
        "1 snapshot".to_owned()
    } else {
        format!("{failed} snapshots")
    }
}

fn list(first: &SnapshotError, rest: &[SnapshotError]) -> String {
    let mut text = String::new();
    for error in std::iter::once(first).chain(rest) {
        let _ = write!(text, "\n- {error}\n");
    }
    text
}

impl<E: std::error::Error + Send + Sync + 'static> From<WriteError> for FlowError<E> {
    fn from(error: WriteError) -> Self {
        match error {
            WriteError::Io { path, source } => Self::Artifact { path, source },
            WriteError::Image { path, source } => Self::ArtifactImage { path, source },
        }
    }
}

/// A flow that passed.
#[derive(Clone, Debug)]
pub struct Report {
    artifact: PathBuf,
    recorded: usize,
}

impl Report {
    /// The markdown file the run was written to.
    pub fn artifact(&self) -> &Path {
        &self.artifact
    }

    /// How many snapshots were recorded rather than matched.
    pub fn recorded(&self) -> usize {
        self.recorded
    }
}

/// Where flow artifacts go: `HUMATA2_FLOW_OUT` when it is set, otherwise
/// `humata2-flows` inside `default`, such as a test's
/// `env!("CARGO_TARGET_TMPDIR")`. Each flow gets a directory in it, named
/// after its spec flow.
pub fn artifact_root(default: &Path) -> PathBuf {
    match std::env::var_os(OUT_VARIABLE) {
        Some(out) if !out.is_empty() => PathBuf::from(out),
        _ => default.join("humata2-flows"),
    }
}

/// Plays `flow` through `driver`, checks each snapshot it asks for against
/// `snapshots` (or records it), and writes the run into `out`, in a
/// directory named after the spec flow.
///
/// A failing snapshot doesn't stop the run, so the artifact shows every
/// step. A failing action does.
///
/// # Errors
///
/// [`FlowError::Driver`] when an action or a capture fails,
/// [`FlowError::Snapshots`] when any snapshot didn't match, or an error
/// writing the artifact.
pub fn run<D: Driver>(
    flow: &Flow,
    driver: &mut D,
    snapshots: &Snapshots,
    out: &Path,
) -> Result<Report, FlowError<D::Error>> {
    let spec = flow.spec();
    let slug = spec.slug();
    let mut artifact = Artifact::new(out.join(&slug), spec, driver.name(), snapshots.mode())?;
    let mut run = Run {
        driver,
        snapshots,
        failures: Vec::new(),
        recorded: 0,
    };

    if flow.start() != Capture::None {
        artifact.section("Start");
        let name = format!("{slug}.00");
        if let Err(source) = run.capture(&name, flow.start(), &mut artifact)? {
            artifact.failed(&source.to_string());
            let path = artifact.finish(Ending::StoppedAt(0))?;
            return Err(FlowError::Driver {
                flow: spec.to_string(),
                during: "the start".to_owned(),
                artifact: path,
                source,
            });
        }
    }

    for (index, step) in flow.steps().iter().enumerate() {
        let number = index + 1;
        let during = format!("step {number}, {}", step.action());
        artifact.section(&format!("{number}. {}", step.action()));
        let done = match run.driver.perform(step.action()) {
            Ok(()) => run.capture(&format!("{slug}.{number:02}"), step.capture(), &mut artifact)?,
            Err(source) => Err(source),
        };
        if let Err(source) = done {
            artifact.failed(&source.to_string());
            let path = artifact.finish(Ending::StoppedAt(number))?;
            return Err(FlowError::Driver {
                flow: spec.to_string(),
                during,
                artifact: path,
                source,
            });
        }
    }

    let Run {
        mut failures,
        recorded,
        ..
    } = run;
    let ending = if failures.is_empty() {
        Ending::Passed
    } else {
        Ending::SnapshotsFailed(failures.len())
    };
    let path = artifact.finish(ending)?;
    if failures.is_empty() {
        return Ok(Report {
            artifact: path,
            recorded,
        });
    }
    let first = failures.remove(0);
    Err(FlowError::Snapshots {
        flow: spec.to_string(),
        artifact: path,
        first,
        rest: failures,
    })
}

struct Run<'a, D: Driver> {
    driver: &'a mut D,
    snapshots: &'a Snapshots,
    failures: Vec<SnapshotError>,
    recorded: usize,
}

impl<D: Driver> Run<'_, D> {
    /// Takes the snapshots `capture` asks for and writes them into the
    /// artifact. The outer error is the artifact's; the inner one is the
    /// driver's, which stops the flow.
    fn capture(
        &mut self,
        name: &str,
        capture: Capture,
        artifact: &mut Artifact,
    ) -> Result<Result<(), D::Error>, WriteError> {
        if capture == Capture::None {
            return Ok(Ok(()));
        }
        let tree = match self.driver.view() {
            Ok(tree) => tree,
            Err(error) => return Ok(Err(error)),
        };
        artifact.view(&tree);
        let outcome = self.snapshots.check_text(name, &tree);
        artifact.snapshot(name, &outcome);
        self.note(outcome);

        if capture == Capture::ViewAndImage {
            let image = match self.driver.image() {
                Ok(image) => image,
                Err(error) => return Ok(Err(error)),
            };
            if let Some(image) = image {
                artifact.image(name, &image)?;
                #[cfg(target_os = "macos")]
                {
                    let image_name = format!("{name}.image");
                    let outcome = self.snapshots.check_image(
                        &image_name,
                        &image,
                        humata2_snapshot::Tolerance::default(),
                    );
                    artifact.snapshot(&image_name, &outcome);
                    self.note(outcome);
                }
            }
        }
        Ok(Ok(()))
    }

    fn note(&mut self, outcome: Result<Outcome, SnapshotError>) {
        match outcome {
            Ok(Outcome::Matched) => {}
            Ok(Outcome::Recorded) => self.recorded += 1,
            Err(error) => self.failures.push(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::{Action, SpecFlow};
    use gpui_kit::Role;
    use humata2_snapshot::RgbaImage;

    #[derive(Debug, Error)]
    #[error("the fake driver was told to fail")]
    struct Refused;

    /// Does every action except a click on "Broken".
    #[derive(Default)]
    struct Fake {
        done: Vec<String>,
    }

    impl Driver for Fake {
        type Error = Refused;
        fn name(&self) -> &str {
            "fake"
        }
        fn perform(&mut self, action: &Action) -> Result<(), Refused> {
            if matches!(action, Action::Click(target) if target.label() == "Broken") {
                return Err(Refused);
            }
            self.done.push(action.to_string());
            Ok(())
        }
        fn view(&mut self) -> Result<String, Refused> {
            Ok(String::new())
        }
        fn image(&mut self) -> Result<Option<RgbaImage>, Refused> {
            Ok(None)
        }
    }

    fn out(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("humata2-flow-{label}-{}", std::process::id()))
    }

    #[::std::prelude::v1::test]
    fn a_failing_action_stops_the_flow_and_still_writes_the_run() {
        let snapshots = humata2_snapshot::snapshots!().expect("the test file is found");
        let flow = Flow::new(SpecFlow::new("demo", "Stops at a broken control"))
            .type_text("a")
            .click(Role::Button, "Broken")
            .type_text("never typed");
        let mut driver = Fake::default();
        let error = run(&flow, &mut driver, &snapshots, &out("stops")).err();

        let Some(FlowError::Driver { artifact, .. }) = error else {
            panic!("expected the driver's error, got {error:?}");
        };
        assert_eq!(driver.done.len(), 1, "no step runs after the failing one");
        let written = std::fs::read_to_string(artifact).unwrap_or_default();
        assert!(written.contains("Broken"), "the failing step is in the artifact");
        assert!(!written.contains("never typed"));
    }
}

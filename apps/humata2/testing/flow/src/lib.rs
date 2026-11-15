//! Humata 2's flow runner. A [`Flow`] is a script of what a person does
//! (click a control by its role and label, type, press a key, drag, paste),
//! named after the spec flow it proves. [`run`] plays it through a
//! [`Driver`] against real state and views with fake clients, snapshots the
//! steps that ask for it, and writes the run as a reviewable artifact:
//! the steps and their snapshots in order, as markdown, with images on
//! macOS.
//!
//! [`GpuiDriver`] plays flows in a headless GPUI window. The [`Driver`]
//! trait is there so the same flows can later play against real daemons.

mod artifact;
mod driver;
mod flow;
mod gpui_driver;
mod run;

pub use driver::Driver;
pub use flow::{Action, Capture, Flow, SpecFlow, Step, Target};
pub use gpui_driver::{FrameMode, GpuiDriver, GpuiDriverError, Host};
pub use run::{FlowError, Report, artifact_root, run};

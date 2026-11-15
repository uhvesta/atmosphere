//! Turning a recording into a flow (I4), so a report can become a
//! regression test.
//!
//! The steps mirror `humata2_flow::Action` (click, type, key, drag, paste),
//! with roles as names (`Button`), so this crate needs no GUI. The app or a
//! test maps each name to its `gpui_kit::Role` and plays the steps, or
//! pastes [`FlowScript::to_rust`] into a test.

use std::fmt::{self, Write as _};

use crate::change;
use crate::error::ReplayError;
use crate::event::{Button, Element, Event, Gesture, Pasted, Trees};
use crate::recording::Captured;

/// A recording as a flow: its steps in order, the interactions a flow
/// can't play, and the trees it should end on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlowScript {
    pub steps: Vec<FlowStep>,
    /// Interactions left out, in order, with why.
    pub skipped: Vec<Skipped>,
    /// How many of the oldest events the recording folded into its start
    /// because it was full. When not 0, the flow starts where the kept
    /// events start, so its fixture has to be set up to match the
    /// recording's start trees.
    pub dropped: usize,
    /// The trees the recording started from.
    pub start: Trees,
    /// The trees the recording ended on, which a replay should reach.
    pub end: Trees,
}

/// One step a flow plays, and whether the view is snapshot after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlowStep {
    pub action: FlowAction,
    /// Whether the recording saw the view change after this step, before
    /// the next one.
    pub snapshot: bool,
}

/// A control as a flow finds it: by role name and accessible label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlowTarget {
    pub role: String,
    pub label: String,
}

/// What a flow does, as `humata2_flow::Action` does it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlowAction {
    Click(FlowTarget),
    Type(String),
    Key(String),
    Drag { from: FlowTarget, to: FlowTarget },
    Paste(String),
}

impl fmt::Display for FlowTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {:?}", self.role, self.label)
    }
}

impl fmt::Display for FlowAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Click(target) => write!(f, "click {target}"),
            Self::Type(text) => write!(f, "type {text:?}"),
            Self::Key(key) => write!(f, "press {key}"),
            Self::Drag { from, to } => write!(f, "drag {from} to {to}"),
            Self::Paste(text) => write!(f, "paste {text:?}"),
        }
    }
}

/// An interaction a flow can't play.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    pub at_ms: u64,
    /// The interaction, as the report shows it.
    pub what: String,
    pub reason: SkipReason,
}

/// Why an interaction can't be played as a flow step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// Flows have no scroll action yet.
    Scroll,
    /// Flows click with the left button only.
    NotLeftButton,
    /// Flows find controls by label, and this one had none.
    NoLabel,
    /// Flows paste text only, and this pasted a file or an image.
    PastedFile,
}

impl fmt::Display for SkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Scroll => "flows can't scroll yet; set the scroll position in the fixture",
            Self::NotLeftButton => "flows click with the left button only",
            Self::NoLabel => "the control has no accessible label for a flow to find it by; give it one",
            Self::PastedFile => "flows paste text only; put the file in the fake paste folder instead",
        })
    }
}

impl Captured {
    /// This recording as a flow script. A snapshot gives a script with no
    /// steps.
    pub fn flow_script(&self) -> FlowScript {
        let mut steps: Vec<FlowStep> = Vec::new();
        let mut skipped = Vec::new();
        for event in &self.events {
            match event {
                Event::Interaction(interaction) => match step(&interaction.gesture) {
                    Ok(action) => steps.push(FlowStep {
                        action,
                        snapshot: false,
                    }),
                    Err(reason) => skipped.push(Skipped {
                        at_ms: interaction.at_ms,
                        what: describe(&interaction.gesture),
                        reason,
                    }),
                },
                Event::View(_) => {
                    if let Some(last) = steps.last_mut() {
                        last.snapshot = true;
                    }
                }
                _ => {}
            }
        }
        FlowScript {
            steps,
            skipped,
            dropped: self.dropped,
            start: self.start.clone(),
            end: self.end.clone(),
        }
    }
}

fn step(gesture: &Gesture) -> Result<FlowAction, SkipReason> {
    Ok(match gesture {
        Gesture::Click { target, button } => {
            if *button != Button::Left {
                return Err(SkipReason::NotLeftButton);
            }
            FlowAction::Click(target_of(target)?)
        }
        Gesture::Type { text, .. } => FlowAction::Type(text.clone()),
        Gesture::Key { key, .. } => FlowAction::Key(key.clone()),
        Gesture::Scroll { .. } => return Err(SkipReason::Scroll),
        Gesture::Drag { from, to } => FlowAction::Drag {
            from: target_of(from)?,
            to: target_of(to)?,
        },
        Gesture::Paste { content, .. } => match content {
            Pasted::Text(text) => FlowAction::Paste(text.clone()),
            Pasted::Image { .. } | Pasted::File { .. } => return Err(SkipReason::PastedFile),
        },
    })
}

fn target_of(element: &Element) -> Result<FlowTarget, SkipReason> {
    let label = element.label.clone().ok_or(SkipReason::NoLabel)?;
    Ok(FlowTarget {
        role: element.role.clone(),
        label,
    })
}

/// An interaction as one line: `click Button "Add" in List "Items"`.
pub(crate) fn describe(gesture: &Gesture) -> String {
    let on = |target: &Option<Element>| match target {
        Some(target) => format!(" into {target}"),
        None => String::new(),
    };
    match gesture {
        Gesture::Click { target, button } => match button {
            Button::Left => format!("click {target}"),
            Button::Right => format!("right-click {target}"),
            Button::Middle => format!("middle-click {target}"),
        },
        Gesture::Key { key, target } => format!("press {key}{}", on(target)),
        Gesture::Type { text, target } => format!("type {text:?}{}", on(target)),
        Gesture::Scroll { target, dx, dy } => format!("scroll {target} by {dx},{dy}"),
        Gesture::Drag { from, to } => format!("drag {from} to {to}"),
        Gesture::Paste { target, content } => match content {
            Pasted::Text(text) => format!("paste {text:?}{}", on(target)),
            Pasted::Image { path, bytes } => format!("paste image {path} ({bytes} bytes){}", on(target)),
            Pasted::File { path, bytes } => format!("paste file {path} ({bytes} bytes){}", on(target)),
        },
    }
}

impl FlowScript {
    /// The script as a `humata2_flow::Flow` builder, to paste into a flow
    /// test named after `section` and `title`.
    pub fn to_rust(&self, section: &str, title: &str) -> String {
        // The view before the first step is snapshot, so the flow shows
        // where it started.
        let mut out = format!("Flow::new(SpecFlow::new({section:?}, {title:?}))\n    .snapshot()\n");
        let target = |target: &FlowTarget| format!("Target::new(Role::{}, {:?})", target.role, target.label);
        for step in &self.steps {
            let _ = match &step.action {
                FlowAction::Click(on) => writeln!(out, "    .click(Role::{}, {:?})", on.role, on.label),
                FlowAction::Type(text) => writeln!(out, "    .type_text({text:?})"),
                FlowAction::Key(key) => writeln!(out, "    .key({key:?})"),
                FlowAction::Drag { from, to } => {
                    writeln!(out, "    .drag({}, {})", target(from), target(to))
                }
                FlowAction::Paste(text) => writeln!(out, "    .paste({text:?})"),
            };
            if step.snapshot {
                out.push_str("    .snapshot()\n");
            }
        }
        out
    }
}

/// The app under test as a replay plays it: a flow driver over the app's
/// real state and views, wired to fake services (such as a
/// [`FakeDaemon`](crate::FakeDaemon) made from the same recording). It
/// does what `humata2_flow::Driver` does and also gives the state, so the
/// app adapts its flow driver to this; this crate has no GUI to do it.
pub trait Player {
    /// Why a step or reading the trees failed.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Does `action`, and returns once everything it started has settled.
    ///
    /// # Errors
    ///
    /// When the action can't be done, such as a control that isn't there
    /// or a call the fake services can't answer.
    fn play(&mut self, action: &FlowAction) -> Result<(), Self::Error>;

    /// The state, view and layout now, written as a report writes them.
    ///
    /// # Errors
    ///
    /// When they can't be read.
    fn trees(&mut self) -> Result<Trees, Self::Error>;
}

/// Which trees a replay compares with the recording's. The state always
/// is. The view and layout are opt-in: a recording made in the app's
/// window has its fonts, size and terminal text, which a headless test
/// window may not.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Compare {
    pub view: bool,
    pub layout: bool,
}

impl FlowScript {
    /// Plays the script on `player` and checks that it starts and ends as
    /// the recording did, comparing the trees `compare` names. The
    /// interactions in [`skipped`](Self::skipped) aren't played.
    ///
    /// # Errors
    ///
    /// [`ReplayError::StartDiffers`] when the player doesn't start where
    /// the recording did, [`ReplayError::Play`] or
    /// [`ReplayError::Trees`] when the player fails, and
    /// [`ReplayError::EndDiffers`] when it ends anywhere else.
    pub fn replay<P: Player>(&self, player: &mut P, compare: Compare) -> Result<(), ReplayError<P::Error>> {
        let start = player.trees().map_err(|source| ReplayError::Trees {
            when: "before the first step",
            source,
        })?;
        if let Some((tree, diff)) = differs(&self.start, &start, compare) {
            return Err(ReplayError::StartDiffers { tree, diff });
        }
        for (index, step) in self.steps.iter().enumerate() {
            player.play(&step.action).map_err(|source| ReplayError::Play {
                step: index + 1,
                what: step.action.to_string(),
                source,
            })?;
        }
        let end = player.trees().map_err(|source| ReplayError::Trees {
            when: "after the last step",
            source,
        })?;
        if let Some((tree, diff)) = differs(&self.end, &end, compare) {
            return Err(ReplayError::EndDiffers { tree, diff });
        }
        Ok(())
    }
}

/// The first tree `compare` names that differs, and how, as a readable
/// diff from the recorded tree to the replayed one.
fn differs(recorded: &Trees, replayed: &Trees, compare: Compare) -> Option<(&'static str, String)> {
    [
        ("state", true, &recorded.state, &replayed.state),
        ("view", compare.view, &recorded.view, &replayed.view),
        ("layout", compare.layout, &recorded.layout, &replayed.layout),
    ]
    .into_iter()
    .filter(|(_, compared, _, _)| *compared)
    .find_map(|(tree, _, recorded, replayed)| {
        let hunks = change::diff(recorded, replayed);
        (!hunks.is_empty()).then(|| (tree, change::render(&hunks, "    ")))
    })
}

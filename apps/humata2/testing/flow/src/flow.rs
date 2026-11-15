//! What a flow is: the spec flow it proves and its steps.

use std::fmt;

use gpui_kit::Role;

/// The flow in SPEC.md §26 that a [`Flow`] proves, such as
/// `SpecFlow::new("26.13", "Resize and collapse panes")`. Its artifact and
/// snapshots are named after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpecFlow {
    section: &'static str,
    title: &'static str,
}

impl SpecFlow {
    /// The spec flow numbered `section`, titled `title` as the spec titles
    /// it.
    pub const fn new(section: &'static str, title: &'static str) -> Self {
        Self { section, title }
    }

    /// The section and title, lower case, with `-` between words: a file
    /// name. `26.13 Resize and collapse panes` is
    /// `26.13-resize-and-collapse-panes`.
    pub(crate) fn slug(&self) -> String {
        let words = format!("{} {}", self.section, self.title);
        let mut slug = String::with_capacity(words.len());
        for c in words.chars() {
            if c.is_ascii_alphanumeric() || c == '.' {
                slug.push(c.to_ascii_lowercase());
            } else if !slug.ends_with('-') && !slug.is_empty() {
                slug.push('-');
            }
        }
        slug.trim_end_matches('-').trim_start_matches('.').to_owned()
    }
}

impl fmt::Display for SpecFlow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.section, self.title)
    }
}

/// A control, found as a person finds it: by what it is and what it says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    role: Role,
    label: String,
}

impl Target {
    /// The one visible control with `role` and accessible `label`.
    pub fn new(role: Role, label: impl Into<String>) -> Self {
        Self {
            role,
            label: label.into(),
        }
    }

    /// What the control is.
    pub fn role(&self) -> Role {
        self.role
    }

    /// Its accessible label.
    pub fn label(&self) -> &str {
        &self.label
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} {:?}", self.role, self.label)
    }
}

/// One thing a person does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Clicks the middle of a control.
    Click(Target),
    /// Types text into whatever has keyboard focus.
    Type(String),
    /// Presses a key, written as GPUI writes keystrokes: `enter`,
    /// `backspace`, `secondary-k` (cmd on macOS, ctrl elsewhere).
    Key(String),
    /// Drags from the middle of one control to the middle of another.
    Drag { from: Target, to: Target },
    /// Puts text on the clipboard and presses `secondary-v`.
    Paste(String),
}

impl fmt::Display for Action {
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

/// What to snapshot after a step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Capture {
    /// Nothing.
    #[default]
    None,
    /// The view snapshot.
    View,
    /// The view snapshot, and on macOS an image snapshot too when the driver
    /// can draw pixels.
    ViewAndImage,
}

/// An action and what to snapshot after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    action: Action,
    capture: Capture,
}

impl Step {
    /// What the person does.
    pub fn action(&self) -> &Action {
        &self.action
    }

    /// What is snapshot afterwards.
    pub fn capture(&self) -> Capture {
        self.capture
    }
}

/// A script of what a person does, proving one spec flow.
///
/// ```ignore
/// Flow::new(SpecFlow::new("26.22", "Act from the palette"))
///     .snapshot()                         // the view before anything is done
///     .key("secondary-k")
///     .type_text("rename")
///     .snapshot()                         // the view after typing
///     .click(Role::ListBoxOption, "Rename session")
///     .snapshot();
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flow {
    spec: SpecFlow,
    start: Capture,
    steps: Vec<Step>,
}

impl Flow {
    /// A flow proving `spec`, with no steps yet.
    pub fn new(spec: SpecFlow) -> Self {
        Self {
            spec,
            start: Capture::None,
            steps: Vec::new(),
        }
    }

    /// The spec flow this proves.
    pub fn spec(&self) -> SpecFlow {
        self.spec
    }

    /// What is snapshot before the first step.
    pub fn start(&self) -> Capture {
        self.start
    }

    /// The steps, in order.
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// Clicks the control with `role` and accessible `label`.
    pub fn click(self, role: Role, label: impl Into<String>) -> Self {
        self.then(Action::Click(Target::new(role, label)))
    }

    /// Types `text` into whatever has keyboard focus.
    pub fn type_text(self, text: impl Into<String>) -> Self {
        self.then(Action::Type(text.into()))
    }

    /// Presses `key`, such as `enter` or `secondary-k`.
    pub fn key(self, key: impl Into<String>) -> Self {
        self.then(Action::Key(key.into()))
    }

    /// Drags from one control to another.
    pub fn drag(self, from: Target, to: Target) -> Self {
        self.then(Action::Drag { from, to })
    }

    /// Pastes `text`.
    pub fn paste(self, text: impl Into<String>) -> Self {
        self.then(Action::Paste(text.into()))
    }

    /// Adds `action` as the next step.
    pub fn then(mut self, action: Action) -> Self {
        self.steps.push(Step {
            action,
            capture: Capture::None,
        });
        self
    }

    /// Takes a view snapshot after the last step, or before the first when
    /// there are no steps yet.
    pub fn snapshot(self) -> Self {
        self.capture(Capture::View)
    }

    /// Takes a view snapshot and, on macOS, an image snapshot after the last
    /// step, or before the first when there are no steps yet.
    pub fn snapshot_image(self) -> Self {
        self.capture(Capture::ViewAndImage)
    }

    fn capture(mut self, capture: Capture) -> Self {
        match self.steps.last_mut() {
            Some(step) => step.capture = capture,
            None => self.start = capture,
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spec_flow_is_named_as_one_file() {
        let slug = SpecFlow::new("26.13", "Resize and collapse panes").slug();
        assert_eq!(slug, "26.13-resize-and-collapse-panes");
        let slug = SpecFlow::new("26.22", "Act from the palette: rename / move").slug();
        assert!(slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.'));
        assert!(!slug.contains("--"));
    }

    #[test]
    fn a_snapshot_belongs_to_the_step_before_it() {
        let flow = Flow::new(SpecFlow::new("demo", "Steps"))
            .snapshot()
            .key("enter")
            .type_text("a")
            .snapshot_image();
        assert_eq!(flow.start(), Capture::View);
        let captures: Vec<Capture> = flow.steps().iter().map(Step::capture).collect();
        assert_eq!(captures, [Capture::None, Capture::ViewAndImage]);
    }
}

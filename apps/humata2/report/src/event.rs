//! What a report records: the context it was taken in, and the events in
//! order. Every type here is plain data, serialized as one JSON object a
//! line in `events.jsonl` (see [`crate::jsonl`]).

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::change::Hunk;

/// Which kind of report this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Everything between a start and a stop.
    Recording,
    /// One moment.
    Snapshot,
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Recording => "recording",
            Self::Snapshot => "snapshot",
        })
    }
}

/// Where a report was taken: the app, the OS, the window, and the machines
/// and sessions involved. The app fills it in; a recording may add
/// machines and sessions as they are touched (see
/// [`Recording::context_mut`](crate::Recording::context_mut)).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    /// When the report started, in milliseconds since the Unix epoch. Every
    /// event's `at_ms` counts from here. The folder's date is taken from it
    /// (UTC).
    pub started_unix_ms: u64,
    /// The app's version, such as `0.1.0 (42)`.
    pub app_version: String,
    /// The OS and its version, such as `macOS 26.7`.
    pub os: String,
    /// The window's size in logical pixels.
    pub window: WindowSize,
    /// Each daemon's version, by machine.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub daemons: Vec<Daemon>,
    /// The machines involved.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub machines: Vec<Machine>,
    /// The sessions involved.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sessions: Vec<Session>,
}

/// A window's size in whole logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowSize {
    pub width: u32,
    pub height: u32,
}

/// A daemon's version on one machine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Daemon {
    pub machine: String,
    pub version: String,
}

/// A machine involved in the report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Machine {
    /// Its name as the sidebar shows it.
    pub name: String,
    /// How it is reached, such as `local`, `ssh` or `vm`.
    pub kind: String,
    /// What the app showed for it, such as `online` or `offline`.
    pub state: String,
}

/// A session involved in the report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub channel: String,
    pub machine: String,
    /// The agent or shell it runs, such as `claude` or `shell`.
    pub agent: String,
}

/// One element in the view tree, named as a person finds it: its role, its
/// accessible label, and the elements it sits in, nearest first. Roles are
/// written as the view snapshot writes them (`Button`, `ListItem`,
/// `Terminal`); an element with no role is `Unknown`.
///
/// Shown as `Button "Add" in List "Items" in Group "#web"`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Element {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The elements this one sits in, nearest first.
    #[serde(default, rename = "in", skip_serializing_if = "Vec::is_empty")]
    pub within: Vec<Node>,
}

impl Element {
    /// The element with `role` and accessible `label`, at the top of the
    /// tree until [`inside`](Self::inside) says what holds it.
    pub fn new(role: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            label: Some(label.into()),
            within: Vec::new(),
        }
    }

    /// The same element, inside one more element: call it nearest first.
    #[must_use]
    pub fn inside(mut self, role: impl Into<String>, label: impl Into<String>) -> Self {
        self.within.push(Node {
            role: role.into(),
            label: Some(label.into()),
        });
        self
    }
}

impl fmt::Display for Element {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_node(f, &self.role, self.label.as_deref())?;
        for node in &self.within {
            f.write_str(" in ")?;
            write_node(f, &node.role, node.label.as_deref())?;
        }
        Ok(())
    }
}

fn write_node(f: &mut fmt::Formatter<'_>, role: &str, label: Option<&str>) -> fmt::Result {
    match label {
        Some(label) => write!(f, "{role} {label:?}"),
        None => f.write_str(role),
    }
}

/// An element an [`Element`] sits in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// A mouse button.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Button {
    #[default]
    Left,
    Right,
    Middle,
}

impl Button {
    fn is_left(&self) -> bool {
        *self == Self::Left
    }
}

/// What was pasted. Files and images are named by where they went (the
/// session's paste folder, D30) and their size; their bytes are never
/// recorded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pasted {
    Text(String),
    Image { path: String, bytes: u64 },
    File { path: String, bytes: u64 },
}

/// One thing the person did. The same set as a flow's actions
/// (`humata2_flow::Action`), plus scrolling and which mouse button.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "gesture", rename_all = "snake_case")]
pub enum Gesture {
    /// A click in the middle of `target`.
    Click {
        target: Element,
        #[serde(default, skip_serializing_if = "Button::is_left")]
        button: Button,
    },
    /// A keystroke that types nothing, written as GPUI writes keystrokes:
    /// `enter`, `backspace`, `secondary-k`. Text that key presses produce is
    /// [`Gesture::Type`]. `target` is what had keyboard focus.
    Key {
        key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<Element>,
    },
    /// Text typed into `target`, the element with keyboard focus. The app
    /// may send one event a key or join a run of keys into one.
    Type {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<Element>,
    },
    /// A scroll over `target` by whole logical pixels.
    Scroll { target: Element, dx: i32, dy: i32 },
    /// A drag from the middle of one element to the middle of another.
    Drag { from: Element, to: Element },
    /// A paste into `target`, the element with keyboard focus.
    Paste {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<Element>,
        content: Pasted,
    },
}

/// An interaction and when it happened.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Interaction {
    /// Milliseconds since the report started.
    pub at_ms: u64,
    #[serde(flatten)]
    pub gesture: Gesture,
}

/// A reducer action and the lines of the state (as `{:#?}` shows it) that it
/// changed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionStep {
    pub at_ms: u64,
    /// The action as `{:?}` shows it.
    pub action: String,
    /// How the state changed. Empty when it didn't.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub change: Vec<Hunk>,
}

/// How the view or layout tree changed after a step.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeChange {
    pub at_ms: u64,
    pub change: Vec<Hunk>,
}

/// What a call's request and response hold, so scrubbing knows what to
/// leave out.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Holds {
    /// Ordinary data. Seeded secrets in it are replaced.
    #[default]
    Plain,
    /// Bytes shown in the terminal with this label. Kept only when that
    /// terminal is marked.
    Terminal { terminal: String },
    /// A paste-folder file's contents, such as an image paste. Never kept.
    PasteContents,
}

impl Holds {
    fn is_plain(&self) -> bool {
        *self == Self::Plain
    }
}

/// A daemon call the app made.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rpc {
    pub at_ms: u64,
    pub machine: String,
    /// The full method, such as `humata.v1.SessionService/List`.
    pub method: String,
    /// `ok`, or the gRPC status code's name, such as `unavailable`.
    pub status: String,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<String>,
    #[serde(default, skip_serializing_if = "Holds::is_plain")]
    pub holds: Holds,
}

/// An error a daemon reported, outside a call's status: a stream that
/// broke, a holder that died.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonError {
    pub at_ms: u64,
    pub machine: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    pub code: String,
    pub message: String,
}

/// A line of a daemon's log from the report's period.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogLine {
    pub at_ms: u64,
    pub machine: String,
    pub line: String,
}

/// One event of a recording, in the order it happened.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    Interaction(Interaction),
    Action(ActionStep),
    View(TreeChange),
    Layout(TreeChange),
    Rpc(Rpc),
    DaemonError(DaemonError),
    Log(LogLine),
}

impl Event {
    /// Milliseconds since the report started.
    pub fn at_ms(&self) -> u64 {
        match self {
            Self::Interaction(event) => event.at_ms,
            Self::Action(event) => event.at_ms,
            Self::View(event) | Self::Layout(event) => event.at_ms,
            Self::Rpc(event) => event.at_ms,
            Self::DaemonError(event) => event.at_ms,
            Self::Log(event) => event.at_ms,
        }
    }

    /// Roughly how many bytes it holds, for the recording's limit.
    pub(crate) fn weight(&self) -> usize {
        const BASE: usize = 32;
        let element = element_weight;
        let optional = |element: &Option<Element>| element.as_ref().map_or(0, element_weight);
        let hunks = |hunks: &[Hunk]| hunks.iter().map(Hunk::weight).sum::<usize>();
        BASE + match self {
            Self::Interaction(event) => match &event.gesture {
                Gesture::Click { target, .. } | Gesture::Scroll { target, .. } => element(target),
                Gesture::Key { key, target } => key.len() + optional(target),
                Gesture::Type { text, target } => text.len() + optional(target),
                Gesture::Drag { from, to } => element(from) + element(to),
                Gesture::Paste { target, content } => {
                    optional(target)
                        + match content {
                            Pasted::Text(text) => text.len(),
                            Pasted::Image { path, .. } | Pasted::File { path, .. } => path.len(),
                        }
                }
            },
            Self::Action(event) => event.action.len() + hunks(&event.change),
            Self::View(event) | Self::Layout(event) => hunks(&event.change),
            Self::Rpc(event) => {
                event.machine.len()
                    + event.method.len()
                    + event.request.as_ref().map_or(0, String::len)
                    + event.response.as_ref().map_or(0, String::len)
            }
            Self::DaemonError(event) => event.machine.len() + event.code.len() + event.message.len(),
            Self::Log(event) => event.machine.len() + event.line.len(),
        }
    }
}

fn element_weight(element: &Element) -> usize {
    let node = |role: &str, label: &Option<String>| role.len() + label.as_ref().map_or(0, String::len);
    node(&element.role, &element.label)
        + element
            .within
            .iter()
            .map(|within| node(&within.role, &within.label))
            .sum::<usize>()
}

/// The state, view and layout at one moment, each as its snapshot text:
/// the state as `{:#?}` shows it, the view as `humata2_snapshot::view`
/// writes it and the layout as `humata2_snapshot::layout` writes it. A
/// recording or snapshot stores each with every line ending in one newline.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trees {
    pub state: String,
    pub view: String,
    pub layout: String,
}

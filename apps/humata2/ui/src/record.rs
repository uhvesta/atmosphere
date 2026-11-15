//! Record mode: what the person did, each action the store reduced, and the
//! state and view trees after it, kept by `humata2_report` and written as a
//! report folder that replays as a flow (I9).
//!
//! Gestures are told in terms of what was on screen: a click names the
//! element under the pointer by role and label, a key names what had focus.
//! That is what a replay looks for again, so a recording survives layout
//! changes that keep the controls.

use std::cell::RefCell;
use std::fmt::Write as _;
use std::rc::Rc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use gpui_kit::base::test_support::snapshots;
use gpui_kit::test::ElementSnapshot;
use gpui_kit::{Pixels, Point, Window};
use humata2_app::{AppAction, AppState};
use humata2_report::{
    Captured, Context, Element, Gesture, Limits, Recorder as Reports, RecorderError, Trees,
    WindowSize,
};

/// The longest action name kept; longer ones (a whole workspace loading)
/// are cut after their variant.
const NAME: usize = 160;

#[derive(Default)]
struct Inner {
    reports: Reports,
    started: Option<Instant>,
}

/// The app's one recorder, shared by the store and the views. Cheap to
/// clone; every clone is the same recorder.
#[derive(Clone, Default)]
pub struct Recorder {
    inner: Rc<RefCell<Inner>>,
}

impl Recorder {
    /// A recorder that isn't recording.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(Inner {
                reports: Reports::new(Limits::default()),
                started: None,
            })),
        }
    }

    /// Whether a recording runs.
    #[must_use]
    pub fn is_recording(&self) -> bool {
        self.inner.borrow().reports.is_recording()
    }

    /// Starts recording from `now`, in a window of `size`.
    ///
    /// # Errors
    ///
    /// [`RecorderError::AlreadyRecording`] when one already runs.
    pub fn start(&self, size: WindowSize, now: Trees) -> Result<(), RecorderError> {
        let started_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| u64::try_from(since.as_millis()).unwrap_or(u64::MAX));
        let context = Context {
            started_unix_ms,
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            os: std::env::consts::OS.to_owned(),
            window: size,
            ..Context::default()
        };
        let mut inner = self.inner.borrow_mut();
        inner.reports.start(context, now)?;
        inner.started = Some(Instant::now());
        Ok(())
    }

    /// Stops recording and gives what was recorded.
    ///
    /// # Errors
    ///
    /// [`RecorderError::NotRecording`] when none runs.
    pub fn stop(&self) -> Result<Captured, RecorderError> {
        let mut inner = self.inner.borrow_mut();
        let at = elapsed(inner.started);
        inner.started = None;
        inner.reports.stop(at)
    }

    /// Records the action the store just reduced and the state after it.
    pub fn action(&self, name: &str, state: &AppState) {
        let text = state_text(state);
        let mut inner = self.inner.borrow_mut();
        let at = elapsed(inner.started);
        if let Some(recording) = inner.reports.recording_mut() {
            recording.action(at, name, &text);
        }
    }

    /// Records something the person did.
    pub fn gesture(&self, gesture: Gesture) {
        let mut inner = self.inner.borrow_mut();
        let at = elapsed(inner.started);
        if let Some(recording) = inner.reports.recording_mut() {
            recording.interaction(at, gesture);
        }
    }

    /// Records the view and layout trees of the frame just drawn, when they
    /// changed.
    pub fn frame(&self, window: &Window) {
        if !self.is_recording() {
            return;
        }
        let elements = snapshots(window);
        let (view, layout) = (view_text(&elements), layout_text(&elements));
        let mut inner = self.inner.borrow_mut();
        let at = elapsed(inner.started);
        if let Some(recording) = inner.reports.recording_mut() {
            recording.view(at, &view);
            recording.layout(at, &layout);
        }
    }
}

fn elapsed(started: Option<Instant>) -> u64 {
    started.map_or(0, |started| {
        u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
    })
}

/// `action` as a recording names it: its `{:?}`, cut after the variant
/// when it is long.
#[must_use]
pub fn action_name(action: &AppAction) -> String {
    let text = format!("{action:?}");
    if text.len() <= NAME {
        return text;
    }
    let mut end = text
        .char_indices()
        .filter(|(_, c)| *c == '(' || *c == '{')
        .map(|(at, _)| at)
        .nth(1)
        .unwrap_or(NAME)
        .min(NAME);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}(…)", &text[..end])
}

/// The state as a report keeps it: where the person is, what is open over
/// it, and what a restart would bring back. The workspace's data isn't in
/// it; the recording's daemon calls and the fakes hold that.
#[must_use]
pub fn state_text(state: &AppState) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "load: {:?}", state.load);
    let _ = writeln!(text, "overlays: {:#?}", state.overlays);
    let toasts: Vec<&str> = state.toasts.iter().map(|toast| toast.text.as_str()).collect();
    let _ = writeln!(text, "toasts: {toasts:#?}");
    let _ = writeln!(text, "sidebar cursor: {:?}", state.sidebar.cursor);
    let _ = writeln!(text, "palette: {:?} {:?}", state.palette.query, state.palette.cursor);
    let _ = writeln!(text, "saved: {:#?}", state.saved());
    text
}

/// The semantic tree of what is drawn: every visible element with a role,
/// one a line, indented by depth, with its label and state.
#[must_use]
pub fn view_text(elements: &[ElementSnapshot]) -> String {
    let mut text = String::new();
    for element in elements.iter().filter(|element| element.visible()) {
        let Some(role) = element.role() else { continue };
        let depth = element.path().len().saturating_sub(1).min(24);
        let _ = write!(text, "{:depth$}{role:?}", "");
        if let Some(label) = element.label() {
            let _ = write!(text, " {label:?}");
        }
        for (flag, on) in [
            ("selected", element.selected()),
            ("expanded", element.expanded()),
            ("checked", element.checked()),
            ("disabled", element.disabled()),
            ("focused", element.focused()),
        ] {
            if on == Some(true) {
                let _ = write!(text, " {flag}");
            }
        }
        if let Some(value) = element.value() {
            let _ = write!(text, " = {value:?}");
        }
        text.push('\n');
    }
    text
}

/// Where each labelled element is drawn, in whole logical pixels.
#[must_use]
pub fn layout_text(elements: &[ElementSnapshot]) -> String {
    let mut text = String::new();
    for element in elements.iter().filter(|element| element.visible()) {
        let (Some(role), Some(label)) = (element.role(), element.label()) else {
            continue;
        };
        let bounds = element.bounds();
        let _ = writeln!(
            text,
            "{role:?} {label:?} {:.0},{:.0} {:.0}×{:.0}",
            f32::from(bounds.origin.x),
            f32::from(bounds.origin.y),
            f32::from(bounds.size.width),
            f32::from(bounds.size.height),
        );
    }
    text
}

/// An element as a recording names it.
#[must_use]
pub fn element(snapshot: &ElementSnapshot) -> Option<Element> {
    let role = snapshot.role()?;
    let label = snapshot.label()?;
    Some(Element::new(format!("{role:?}"), label))
}

/// The labelled element under `at` that a click would land on: the
/// smallest visible one containing it.
#[must_use]
pub fn under(elements: &[ElementSnapshot], at: Point<Pixels>) -> Option<Element> {
    elements
        .iter()
        .filter(|element| {
            element.visible()
                && element.role().is_some()
                && element.label().is_some_and(|label| !label.is_empty())
                && element.bounds().contains(&at)
        })
        .min_by(|a, b| {
            let area = |e: &ElementSnapshot| {
                f32::from(e.bounds().size.width) * f32::from(e.bounds().size.height)
            };
            area(a).total_cmp(&area(b))
        })
        .and_then(element)
}

/// The labelled element with keyboard focus.
#[must_use]
pub fn focused(elements: &[ElementSnapshot]) -> Option<Element> {
    elements
        .iter()
        .rev()
        .find(|element| element.focused() == Some(true) && element.label().is_some())
        .and_then(element)
}

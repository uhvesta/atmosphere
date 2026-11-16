//! Record mode: what the person did, each action the store reduced, and the
//! state and view trees after it, kept by `humata2_report` and written as a
//! report folder that replays as a flow (I9).
//!
//! Gestures are told in terms of what was on screen: a click names the
//! element under the pointer by role and label, a key names what had focus.
//! That is what a replay looks for again, so a recording survives layout
//! changes that keep the controls.

use std::cell::RefCell;
use std::cmp::Reverse;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use gpui_kit::base::test_support::snapshots;
use gpui_kit::test::ElementSnapshot;
use gpui_kit::{ElementId, Pixels, Point, Window};
use humata2_app::{AppAction, AppState, PinTarget, PinnedComment, ReportForm, ReportKind};
use humata2_report::{
    Captured, Comment, Context, Element, Form, FormError, Gesture, Limits, Pasted, PromptError,
    Recorder as Reports, RecorderError, ScrubError, Secrets, Trees, WindowSize, WriteError,
};

/// The accessible label of the dot that stops a recording; a click on it
/// ends the recording and is never part of it.
pub(crate) const STOP_LABEL: &str = "Stop recording";

/// The label of the bug-report sheet; its own elements can't be pinned.
pub(crate) const SHEET_LABEL: &str = "Bug report";

/// The most elements a form offers to pin.
const MAX_PINS: usize = 5000;

/// Variables whose values are always secrets, whatever their length.
const SECRET_NAMES: [&str; 4] = [
    "HUMATA_SESSION_TOKEN",
    "HUMATA_TOKEN",
    "GH_TOKEN",
    "GITHUB_TOKEN",
];

/// Variables ending in one of these hold a secret when long enough.
const SECRET_SUFFIXES: [&str; 4] = ["_TOKEN", "_KEY", "_SECRET", "_PASSWORD"];

/// The shortest value of a suffix-named variable taken as a secret.
const SECRET_MIN: usize = 8;

/// Why a report couldn't be taken, saved or copied.
#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    /// The app wasn't started with a folder to keep reports in.
    #[error(
        "This run of Humata has no folder to save reports in. Quit it and start it with \
         `--data <folder>`, or from the app bundle, then try again."
    )]
    NoFolder,
    /// The recorder was in the wrong state.
    #[error("{0}")]
    Recorder(#[source] RecorderError),
    /// The recording couldn't be scrubbed.
    #[error("The report wasn't saved. {0}")]
    Scrub(#[source] ScrubError),
    /// The report folder couldn't be written.
    #[error("The report wasn't saved. {0}")]
    Write(#[source] WriteError),
    /// The form couldn't be added to the saved report.
    #[error("The details couldn't be added to the report. {0}")]
    Form(#[source] FormError),
    /// The prompt couldn't be made from the saved report.
    #[error("The report couldn't be copied. {0}")]
    Prompt(#[source] PromptError),
}

/// The longest action name kept; longer ones (a whole workspace loading)
/// are cut after their variant.
const NAME: usize = 160;

struct Inner {
    reports: Reports,
    started: Option<Instant>,
    folder: Option<PathBuf>,
    title: Option<String>,
    seeded: Vec<(String, String)>,
    /// A run of typing or scrolling that more of the same may extend.
    pending: Option<(u64, Gesture)>,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            reports: Reports::new(Limits::default()),
            started: None,
            folder: None,
            title: None,
            seeded: Vec::new(),
            pending: None,
        }
    }
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
        Self::default()
    }

    /// The same recorder, saving reports in `reports`.
    #[must_use]
    pub fn saving_to(self, reports: PathBuf) -> Self {
        self.inner.borrow_mut().folder = Some(std::path::absolute(&reports).unwrap_or(reports));
        self
    }

    /// Teaches the recorder a secret the app learned during the run, such
    /// as a host variable's value; it is taken out of every report.
    pub fn seed(&self, name: &str, value: &str) {
        if !value.is_empty() {
            self.inner
                .borrow_mut()
                .seeded
                .push((name.to_owned(), value.to_owned()));
        }
    }

    /// What to take out of a report: the environment's tokens now, and what
    /// was seeded.
    fn secrets(&self) -> Secrets {
        let mut secrets = Secrets::new();
        for (name, value) in env_secrets(std::env::vars()) {
            secrets = secrets.secret(&name, &value);
        }
        for (name, value) in &self.inner.borrow().seeded {
            secrets = secrets.secret(name, value);
        }
        secrets
    }

    fn folder(&self) -> Result<PathBuf, ReportError> {
        self.inner.borrow().folder.clone().ok_or(ReportError::NoFolder)
    }

    /// Starts a recording of `window` as `state` shows it. `title` is what
    /// the report's "what happened" starts as, unless stopping gives one.
    ///
    /// # Errors
    ///
    /// [`ReportError::Recorder`] when one already runs.
    pub fn begin(
        &self,
        window: &Window,
        state: &AppState,
        title: Option<String>,
    ) -> Result<(), ReportError> {
        self.start(window_size(window), trees(window, state))
            .map_err(ReportError::Recorder)?;
        self.inner.borrow_mut().title = title;
        Ok(())
    }

    /// Stops the recording, saves it scrubbed and gives the form to fill in.
    ///
    /// # Errors
    ///
    /// [`ReportError`] when there is no folder (the recording keeps
    /// running), none runs, or it can't be scrubbed or written.
    pub fn finish(
        &self,
        window: &Window,
        title: Option<String>,
    ) -> Result<ReportForm, ReportError> {
        let folder = self.folder()?;
        self.frame(window);
        let kept = self.inner.borrow_mut().title.take();
        let captured = self.stop().map_err(ReportError::Recorder)?;
        self.save(
            &folder,
            captured,
            title.or(kept).unwrap_or_default(),
            ReportKind::Recording,
            window,
        )
    }

    /// Saves one moment as a report and gives the form to fill in.
    ///
    /// # Errors
    ///
    /// [`ReportError`] when there is no folder or it can't be written.
    pub fn snapshot(
        &self,
        window: &Window,
        state: &AppState,
        title: Option<String>,
    ) -> Result<ReportForm, ReportError> {
        let folder = self.folder()?;
        let mut context = context(window_size(window));
        context.started_unix_ms = now_ms();
        let captured = humata2_report::snapshot(context, trees(window, state));
        self.save(
            &folder,
            captured,
            title.unwrap_or_default(),
            ReportKind::Snapshot,
            window,
        )
    }

    fn save(
        &self,
        folder: &std::path::Path,
        captured: Captured,
        title: String,
        kind: ReportKind,
        window: &Window,
    ) -> Result<ReportForm, ReportError> {
        let form = Form {
            happened: title.clone(),
            ..Form::default()
        };
        let scrubbed = humata2_report::scrub(captured, form, &self.secrets())
            .map_err(ReportError::Scrub)?;
        let dir = humata2_report::write(folder, &scrubbed).map_err(ReportError::Write)?;
        Ok(ReportForm::new(dir, kind, title, pins(&snapshots(window))))
    }

    /// Adds `form`'s details to its saved report and gives the prompt to put
    /// on the clipboard.
    ///
    /// # Errors
    ///
    /// [`ReportError::Form`] or [`ReportError::Prompt`] when the saved
    /// report can't be updated or read.
    pub fn copy(&self, form: &ReportForm) -> Result<String, ReportError> {
        let details = Form {
            happened: form.happened.clone(),
            expected: form.expected.clone(),
            comments: form.comments.iter().map(comment).collect(),
        };
        humata2_report::fill_form(&form.folder, details, &self.secrets())
            .map_err(ReportError::Form)?;
        humata2_report::copy_prompt(&form.folder).map_err(ReportError::Prompt)
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
        let context = Context {
            started_unix_ms: now_ms(),
            ..context(size)
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
        inner.flush();
        inner.started = None;
        inner.reports.stop(at)
    }

    /// Records the action the store just reduced and the state after it.
    pub fn action(&self, name: &str, state: &AppState) {
        let text = state_text(state);
        let mut inner = self.inner.borrow_mut();
        inner.flush();
        let at = elapsed(inner.started);
        if let Some(recording) = inner.reports.recording_mut() {
            recording.action(at, name, &text);
        }
    }

    /// Records something the person did.
    ///
    /// Typing into one element and scrolling over one element are joined
    /// into a single gesture each, written when something else happens.
    pub fn gesture(&self, gesture: Gesture) {
        let mut inner = self.inner.borrow_mut();
        if !inner.reports.is_recording() {
            return;
        }
        let at = elapsed(inner.started);
        let joined = match (&mut inner.pending, &gesture) {
            (
                Some((
                    _,
                    Gesture::Type { text, target },
                )),
                Gesture::Type {
                    text: more,
                    target: next,
                },
            ) if target == next => {
                text.push_str(more);
                true
            }
            (
                Some((_, Gesture::Scroll { target, dx, dy })),
                Gesture::Scroll {
                    target: next,
                    dx: more_x,
                    dy: more_y,
                },
            ) if target == next => {
                *dx = dx.saturating_add(*more_x);
                *dy = dy.saturating_add(*more_y);
                true
            }
            _ => false,
        };
        if joined {
            return;
        }
        inner.flush();
        if matches!(gesture, Gesture::Type { .. } | Gesture::Scroll { .. }) {
            inner.pending = Some((at, gesture));
        } else if let Some(recording) = inner.reports.recording_mut() {
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

impl Inner {
    /// Writes the joined typing or scrolling, if any.
    fn flush(&mut self) {
        if let Some((at, gesture)) = self.pending.take()
            && let Some(recording) = self.reports.recording_mut()
        {
            recording.interaction(at, gesture);
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| u64::try_from(since.as_millis()).unwrap_or(u64::MAX))
}

fn context(window: WindowSize) -> Context {
    Context {
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        os: std::env::consts::OS.to_owned(),
        window,
        ..Context::default()
    }
}

/// `window`'s size in whole logical pixels.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn window_size(window: &Window) -> WindowSize {
    let whole = |length: Pixels| f32::from(length).round().max(0.) as u32;
    let size = window.viewport_size();
    WindowSize {
        width: whole(size.width),
        height: whole(size.height),
    }
}

/// The state, view and layout of `window` now.
fn trees(window: &Window, state: &AppState) -> Trees {
    let elements = snapshots(window);
    Trees {
        state: state_text(state),
        view: view_text(&elements),
        layout: layout_text(&elements),
    }
}

/// A form's comment as the report names it.
fn comment(comment: &PinnedComment) -> Comment {
    let target = &comment.target;
    let element = target
        .within
        .iter()
        .fold(Element::new(&target.role, &target.label), |element, (role, label)| {
            element.inside(role, label)
        });
    Comment {
        element,
        text: comment.text.clone(),
    }
}

/// The secrets in `vars`, as (name, value): the well-known token variables
/// when set, and any other variable named `…_TOKEN`, `…_KEY`, `…_SECRET` or
/// `…_PASSWORD` whose value is at least 8 characters.
pub(crate) fn env_secrets(
    vars: impl IntoIterator<Item = (String, String)>,
) -> Vec<(String, String)> {
    vars.into_iter()
        .filter(|(name, value)| {
            !value.is_empty()
                && (SECRET_NAMES.contains(&name.as_str())
                    || (value.chars().count() >= SECRET_MIN
                        && SECRET_SUFFIXES.iter().any(|suffix| name.ends_with(suffix))))
        })
        .collect()
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

/// The role and label of `snapshot`, when it has both.
fn named(snapshot: &ElementSnapshot) -> Option<(String, String)> {
    let role = snapshot.role()?;
    let label = snapshot.label().filter(|label| !label.is_empty())?;
    Some((format!("{role:?}"), label.to_owned()))
}

/// What holds the element at `path`, nearest first: the labelled
/// candidates whose path is a strict prefix of it.
fn ancestors<'a>(
    path: &[ElementId],
    candidates: impl IntoIterator<Item = (&'a [ElementId], Option<(String, String)>)>,
) -> Vec<(String, String)> {
    let mut found: Vec<(usize, (String, String))> = candidates
        .into_iter()
        .filter(|(above, _)| above.len() < path.len() && path.starts_with(above))
        .filter_map(|(above, name)| name.map(|name| (above.len(), name)))
        .collect();
    found.sort_by_key(|(depth, _)| Reverse(*depth));
    found.into_iter().map(|(_, name)| name).collect()
}

fn holders(snapshot: &ElementSnapshot, all: &[ElementSnapshot]) -> Vec<(String, String)> {
    ancestors(
        snapshot.path(),
        all.iter().map(|other| (other.path(), named(other))),
    )
}

/// An element as a recording names it, with the labelled elements it sits
/// in.
#[must_use]
pub fn element(snapshot: &ElementSnapshot, all: &[ElementSnapshot]) -> Option<Element> {
    let (role, label) = named(snapshot)?;
    Some(
        holders(snapshot, all)
            .into_iter()
            .fold(Element::new(role, label), |element, (role, label)| {
                element.inside(role, label)
            }),
    )
}

/// The labelled element under `at` that a click would land on: the
/// smallest visible one containing it.
#[must_use]
pub fn under(elements: &[ElementSnapshot], at: Point<Pixels>) -> Option<Element> {
    elements
        .iter()
        .filter(|element| {
            element.visible() && named(element).is_some() && element.bounds().contains(&at)
        })
        .min_by(|a, b| {
            let area = |e: &ElementSnapshot| {
                f32::from(e.bounds().size.width) * f32::from(e.bounds().size.height)
            };
            area(a).total_cmp(&area(b))
        })
        .and_then(|found| element(found, elements))
}

/// The labelled element with keyboard focus.
#[must_use]
pub fn focused(elements: &[ElementSnapshot]) -> Option<Element> {
    elements
        .iter()
        .rev()
        .find(|element| element.focused() == Some(true) && named(element).is_some())
        .and_then(|found| element(found, elements))
}

/// What a form can pin a comment to: every visible labelled element of the
/// view, each once, outside the bug-report sheet itself.
#[must_use]
pub fn pins(elements: &[ElementSnapshot]) -> Vec<PinTarget> {
    let mut targets: Vec<PinTarget> = Vec::new();
    for found in elements.iter().filter(|element| element.visible()) {
        let Some((role, label)) = named(found) else {
            continue;
        };
        let within = holders(found, elements);
        let in_sheet = within
            .iter()
            .chain(std::iter::once(&(role.clone(), label.clone())))
            .any(|(role, label)| role == "Dialog" && label == SHEET_LABEL);
        if in_sheet || label == STOP_LABEL {
            continue;
        }
        let target = PinTarget {
            role,
            label,
            within,
        };
        if !targets.contains(&target) {
            targets.push(target);
        }
        if targets.len() >= MAX_PINS {
            break;
        }
    }
    targets
}

/// A keystroke as a flow spells it: `enter`, `secondary-k`, `shift-tab`.
#[must_use]
pub fn key_text(keystroke: &gpui_kit::Keystroke) -> String {
    let modifiers = &keystroke.modifiers;
    let mut text = String::new();
    for (held, name) in [
        (modifiers.function, "fn-"),
        (modifiers.control && cfg!(target_os = "macos"), "ctrl-"),
        (modifiers.alt, "alt-"),
        (
            modifiers.platform || (modifiers.control && !cfg!(target_os = "macos")),
            "secondary-",
        ),
        (modifiers.shift, "shift-"),
    ] {
        if held {
            text.push_str(name);
        }
    }
    text.push_str(&keystroke.key);
    text
}

/// What a keystroke the person made is, as a recording tells it: typed
/// text, a paste of `clipboard`, or a key. `None` for the keystrokes that
/// start, stop or snapshot a report, which are never part of one.
#[must_use]
pub fn keystroke_gesture(
    keystroke: &gpui_kit::Keystroke,
    target: Option<Element>,
    clipboard: Option<String>,
) -> Option<Gesture> {
    let modifiers = &keystroke.modifiers;
    if modifiers.platform && modifiers.shift && matches!(keystroke.key.as_str(), "r" | "s") {
        return None;
    }
    let command = modifiers.platform || modifiers.control || modifiers.function;
    if command
        && keystroke.key == "v"
        && let Some(text) = clipboard.filter(|text| !text.is_empty())
    {
        return Some(Gesture::Paste {
            target,
            content: Pasted::Text(text),
        });
    }
    if !command
        && let Some(text) = keystroke
            .key_char
            .clone()
            .filter(|text| !text.is_empty() && !text.chars().any(char::is_control))
    {
        return Some(Gesture::Type { text, target });
    }
    Some(Gesture::Key {
        key: key_text(keystroke),
        target,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn well_known_tokens_are_secrets_at_any_length() {
        let found = env_secrets(vars(&[("GH_TOKEN", "abc"), ("HOME", "/Users/x")]));
        assert_eq!(found, vars(&[("GH_TOKEN", "abc")]));
    }

    #[test]
    fn suffixed_variables_need_eight_characters() {
        let found = env_secrets(vars(&[
            ("STRIPE_KEY", "short"),
            ("DB_PASSWORD", "correct-horse"),
            ("MY_SECRET", "12345678"),
            ("API_TOKEN", ""),
            ("PATH_KEYS", "/usr/local/bin:/usr/bin"),
        ]));
        assert_eq!(
            found,
            vars(&[("DB_PASSWORD", "correct-horse"), ("MY_SECRET", "12345678")])
        );
    }

    fn id(name: &'static str) -> ElementId {
        ElementId::from(name)
    }

    #[test]
    fn ancestors_are_the_labelled_strict_prefixes_nearest_first() {
        let shell = [id("shell")];
        let list = [id("shell"), id("list")];
        let other = [id("shell"), id("other")];
        let item = [id("shell"), id("list"), id("item")];
        let name = |role: &str, label: &str| Some((role.to_owned(), label.to_owned()));
        let found = ancestors(
            &item,
            [
                (&shell[..], name("Region", "web")),
                (&list[..], name("List", "Items")),
                (&other[..], name("List", "Other")),
                (&item[..], name("Button", "Add")),
            ],
        );
        assert_eq!(
            found,
            vec![
                ("List".to_owned(), "Items".to_owned()),
                ("Region".to_owned(), "web".to_owned())
            ]
        );
    }

    #[test]
    fn unlabelled_holders_are_skipped() {
        let shell = [id("shell")];
        let list = [id("shell"), id("list")];
        let item = [id("shell"), id("list"), id("item")];
        let found = ancestors(
            &item,
            [
                (&shell[..], Some(("Region".to_owned(), "web".to_owned()))),
                (&list[..], None),
            ],
        );
        assert_eq!(found, vec![("Region".to_owned(), "web".to_owned())]);
    }

    fn stroke(modifiers: gpui_kit::Modifiers, key: &str, key_char: Option<&str>) -> gpui_kit::Keystroke {
        gpui_kit::Keystroke {
            modifiers,
            key: key.to_owned(),
            key_char: key_char.map(str::to_owned),
        }
    }

    #[test]
    fn the_report_shortcuts_are_never_recorded() {
        let held = gpui_kit::Modifiers {
            platform: true,
            shift: true,
            ..gpui_kit::Modifiers::default()
        };
        assert_eq!(keystroke_gesture(&stroke(held, "r", None), None, None), None);
        assert_eq!(keystroke_gesture(&stroke(held, "s", None), None, None), None);
    }

    #[test]
    fn printable_keys_are_typing_and_others_are_keys() {
        let typed = keystroke_gesture(
            &stroke(gpui_kit::Modifiers::default(), "a", Some("a")),
            None,
            None,
        );
        assert_eq!(
            typed,
            Some(Gesture::Type {
                text: "a".to_owned(),
                target: None
            })
        );
        let command = gpui_kit::Modifiers {
            platform: true,
            ..gpui_kit::Modifiers::default()
        };
        let key = keystroke_gesture(&stroke(command, "k", Some("k")), None, None);
        assert_eq!(
            key,
            Some(Gesture::Key {
                key: "secondary-k".to_owned(),
                target: None
            })
        );
        let enter = keystroke_gesture(
            &stroke(gpui_kit::Modifiers::default(), "enter", Some("\n")),
            None,
            None,
        );
        assert_eq!(
            enter,
            Some(Gesture::Key {
                key: "enter".to_owned(),
                target: None
            })
        );
    }

    #[test]
    fn a_paste_carries_the_clipboard_text() {
        let command = gpui_kit::Modifiers {
            platform: true,
            ..gpui_kit::Modifiers::default()
        };
        let pasted = keystroke_gesture(&stroke(command, "v", None), None, Some("hi".to_owned()));
        assert_eq!(
            pasted,
            Some(Gesture::Paste {
                target: None,
                content: Pasted::Text("hi".to_owned())
            })
        );
    }
}

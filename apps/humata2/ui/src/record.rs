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
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gpui_kit::base::test_support::{snapshots, texts};
use gpui_kit::test::ElementSnapshot;
use gpui_kit::{ElementId, Pixels, Point, Window};
use humata2_app::{
    AppAction, AppState, Overlay, PinTarget, PinnedComment, Rect, ReportForm, ReportKind,
};
use humata2_clients::model::{MachineId, MachineKind};
use crate::record_video::{Sampler, SamplerError};
use humata2_report::{
    Bounds, Captured, Comment, Context, Drawing, Element, Form, FormError, Gesture, Limits, Mark,
    Pasted, PictureError, PixelBox, PromptError, Recorder as Reports, RecorderError, Rpc, ScrubError, Secrets, Trees, WindowSize,
    WriteError,
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
    /// The window couldn't be drawn to a picture. The report's text files
    /// were saved without `window.png`.
    #[error(
        "The report was saved without a picture of the window: it couldn't be drawn ({message}). \
         The text files are complete; take a new snapshot if you need the picture."
    )]
    Render {
        /// What the renderer said.
        message: String,
    },
    /// The window's picture couldn't be saved. The report's text files
    /// were saved without it.
    #[error("The report was saved without a picture of the window. {0}")]
    Picture(#[source] PictureError),
    /// The recording's video couldn't be saved. The report's text files
    /// and picture were saved without it.
    #[error("{0}")]
    Video(#[source] SamplerError),
}

/// The longest action name kept; longer ones (a whole workspace loading)
/// are cut after their variant.
const NAME: usize = 160;

/// The most daemon calls held between two drains.
const MAX_CALLS: usize = 20_000;

/// A daemon call the app made: its method, when it began, how long it took
/// to answer and how it ended (`ok`, or the status code's name).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RpcCall {
    /// The full method, such as `humata.v1.SessionService/List`.
    pub method: String,
    /// When the call began.
    pub started: Instant,
    /// How long it took to answer.
    pub duration: Duration,
    /// `ok`, or the status code's name, such as `unavailable`.
    pub status: String,
}

#[derive(Default)]
struct Feed {
    /// When the running recording began; `None` when none runs.
    since: Option<Instant>,
    calls: Vec<RpcCall>,
}

/// Where daemon calls are told to the recorder, from any thread. Cheap to
/// clone; every clone is the same feed.
#[derive(Clone, Default)]
pub struct RpcFeed {
    inner: Arc<Mutex<Feed>>,
}

impl RpcFeed {
    fn lock(&self) -> MutexGuard<'_, Feed> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Tells of a call. It is kept only while a recording runs, and only
    /// when it began after the recording did.
    pub fn push(&self, call: RpcCall) {
        let mut feed = self.lock();
        if feed
            .since
            .is_some_and(|since| call.started >= since && feed.calls.len() < MAX_CALLS)
        {
            feed.calls.push(call);
        }
    }

    fn begin(&self, at: Instant) {
        let mut feed = self.lock();
        feed.since = Some(at);
        feed.calls.clear();
    }

    fn end(&self) {
        let mut feed = self.lock();
        feed.since = None;
        feed.calls.clear();
    }

    fn take(&self) -> Vec<RpcCall> {
        std::mem::take(&mut self.lock().calls)
    }
}

struct Inner {
    reports: Reports,
    feed: RpcFeed,
    /// Where the recording stood when the palette opened.
    mark: Option<Mark>,
    started: Option<Instant>,
    folder: Option<PathBuf>,
    title: Option<String>,
    seeded: Vec<(String, String)>,
    /// A run of typing or scrolling that more of the same may extend.
    pending: Option<(u64, Gesture)>,
    /// What went wrong with the last report's picture, until the caller
    /// takes it to tell the person.
    warning: Option<ReportError>,
    /// The running recording's video, when its thread started.
    video: Option<Sampler>,
    /// The view and layout last offered to the video, so a frame that
    /// changed nothing isn't drawn again.
    sampled: Option<u64>,
    /// Whether the palette was open after the last action.
    palette_open: bool,
    /// A palette command was just rewound out: the key that ran it,
    /// told after the app handled it, is left out too.
    drop_enter: bool,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            reports: Reports::new(Limits::default()),
            feed: RpcFeed::default(),
            mark: None,
            started: None,
            folder: None,
            title: None,
            seeded: Vec::new(),
            pending: None,
            warning: None,
            video: None,
            sampled: None,
            palette_open: false,
            drop_enter: false,
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

    /// The values of every secret, for covering them in pictures.
    pub(crate) fn secret_values(&self) -> Vec<String> {
        let mut values: Vec<String> = env_secrets(std::env::vars())
            .into_iter()
            .map(|(_, value)| value)
            .collect();
        values.extend(self.inner.borrow().seeded.iter().map(|(_, value)| value.clone()));
        values
    }

    /// What went wrong with the picture of the last report saved, if
    /// anything: the report itself was saved, only `window.png` is missing.
    /// Taking it clears it; the caller toasts it.
    pub fn take_warning(&self) -> Option<ReportError> {
        self.inner.borrow_mut().warning.take()
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
        self.start(context(window_size(window), state), trees(window, state))
            .map_err(ReportError::Recorder)?;
        let mut inner = self.inner.borrow_mut();
        inner.title = title;
        inner.sampled = None;
        inner.video = Sampler::start(window.scale_factor());
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
        state: &AppState,
        title: Option<String>,
    ) -> Result<ReportForm, ReportError> {
        let folder = self.folder()?;
        self.frame(window);
        let picture = render_masked(window, &self.secret_values());
        let kept = self.inner.borrow_mut().title.take();
        let video = self.inner.borrow_mut().video.take();
        // Daemon versions read while the recording ran (a report reads the
        // ones it lacks when it starts) are listed as well.
        let versions = daemons(state);
        if let Some(recording) = self.inner.borrow_mut().reports.recording_mut() {
            recording.context_mut().daemons = versions;
        }
        let captured = self.stop().map_err(ReportError::Recorder)?;
        let form = self.save(
            &folder,
            captured,
            title.or(kept).unwrap_or_default(),
            ReportKind::Recording,
            window,
            picture,
        )?;
        // A video that can't be saved never loses the report.
        if let Some(Err(error)) = video.map(|video| video.finish(&form.folder)) {
            let mut inner = self.inner.borrow_mut();
            if inner.warning.is_none() {
                inner.warning = Some(ReportError::Video(error));
            }
        }
        Ok(form)
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
        let mut context = context(window_size(window), state);
        context.started_unix_ms = now_ms();
        let captured = humata2_report::snapshot(context, trees(window, state));
        let picture = render_masked(window, &self.secret_values());
        self.save(
            &folder,
            captured,
            title.unwrap_or_default(),
            ReportKind::Snapshot,
            window,
            picture,
        )
    }

    fn save(
        &self,
        folder: &std::path::Path,
        captured: Captured,
        title: String,
        kind: ReportKind,
        window: &Window,
        picture: Result<image::RgbaImage, ReportError>,
    ) -> Result<ReportForm, ReportError> {
        let form = Form {
            happened: title.clone(),
            ..Form::default()
        };
        let scrubbed = humata2_report::scrub(captured, form, &self.secrets())
            .map_err(ReportError::Scrub)?;
        let dir = humata2_report::write(folder, &scrubbed).map_err(ReportError::Write)?;
        // A missing picture never loses the text files.
        let warning = picture.and_then(|image| {
            humata2_report::write_window(&dir, &image).map_err(ReportError::Picture)
        });
        self.inner.borrow_mut().warning = warning.err();
        let (targets, places) = pins_placed(&snapshots(window));
        let size = window_size(window);
        Ok(ReportForm::new(dir, kind, title, targets)
            .with_places(places, (size.width, size.height)))
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
            comments: form
                .comments
                .iter()
                .map(|pinned| comment(pinned, form))
                .collect(),
            drawing: Drawing {
                window: humata2_report::Size {
                    width: form.window.0,
                    height: form.window.1,
                },
                strokes: form.strokes.iter().map(stroke).collect(),
            },
        };
        humata2_report::fill_form(&form.folder, details, &self.secrets())
            .map_err(ReportError::Form)?;
        humata2_report::copy_prompt(&form.folder).map_err(ReportError::Prompt)
    }

    /// Where daemon calls are told to this recorder, from any thread.
    #[must_use]
    pub fn rpc_feed(&self) -> RpcFeed {
        self.inner.borrow().feed.clone()
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
    pub fn start(&self, context: Context, now: Trees) -> Result<(), RecorderError> {
        let context = Context {
            started_unix_ms: now_ms(),
            ..context
        };
        let mut inner = self.inner.borrow_mut();
        inner.reports.start(context, now)?;
        let at = Instant::now();
        inner.started = Some(at);
        inner.mark = None;
        inner.drop_enter = false;
        inner.feed.begin(at);
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
        inner.drain();
        inner.started = None;
        inner.mark = None;
        inner.video = None;
        inner.feed.end();
        inner.reports.stop(at)
    }

    /// Records the action the store just reduced and the state after it.
    pub fn action(&self, name: &str, state: &AppState) {
        let text = state_text(state);
        let palette = state
            .overlays
            .iter()
            .any(|overlay| matches!(overlay, Overlay::Palette));
        let requested = state.report.request.is_some();
        let mut inner = self.inner.borrow_mut();
        inner.flush();
        inner.drain();
        let opened = palette && !inner.palette_open;
        inner.palette_open = palette;
        if requested && let Some(mark) = inner.mark.take() {
            // A report command run from the palette: the steps that led to
            // it, and this action, stay out of the recording.
            if inner
                .reports
                .recording_mut()
                .is_some_and(|recording| recording.rewind(mark))
            {
                inner.drop_enter = true;
                return;
            }
        } else if opened && !requested {
            inner.mark = inner.reports.recording_mut().map(|recording| recording.mark());
        }
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
        if std::mem::take(&mut inner.drop_enter)
            && matches!(&gesture, Gesture::Key { key, .. } if key == "enter")
        {
            return;
        }
        if !inner.palette_open {
            // Done with the palette closed: what came before stays.
            inner.mark = None;
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
        inner.drain();
        let at = elapsed(inner.started);
        if let Some(recording) = inner.reports.recording_mut() {
            recording.view(at, &view);
            recording.layout(at, &layout);
        }
        drop(inner);
        let mut hasher = std::hash::DefaultHasher::new();
        std::hash::Hash::hash(&(&view, &layout), &mut hasher);
        self.sample(window, std::hash::Hasher::finish(&hasher));
    }

    /// Offers the window to the recording's video when a sample is due
    /// (at most 5 a second, D160) and the frame's view or layout (`key`)
    /// changed since the last sample; a still screen costs nothing. A
    /// picture that can't be drawn is skipped.
    fn sample(&self, window: &Window, key: u64) {
        let at_ms = {
            let inner = self.inner.borrow();
            let (Some(video), Some(started)) = (inner.video.as_ref(), inner.started) else {
                return;
            };
            if inner.sampled == Some(key) {
                return;
            }
            let now = Instant::now();
            if !video.due(now) {
                return;
            }
            u64::try_from(now.saturating_duration_since(started).as_millis())
                .unwrap_or(u64::MAX)
        };
        let Ok(image) = render_masked(window, &self.secret_values()) else {
            return;
        };
        let mut inner = self.inner.borrow_mut();
        inner.sampled = Some(key);
        if let Some(video) = inner.video.as_ref() {
            video.offer(at_ms, image);
        }
    }
}

impl Inner {
    /// Moves the daemon calls told since the last time into the recording.
    fn drain(&mut self) {
        let calls = self.feed.take();
        let (Some(start), Some(recording)) = (self.started, self.reports.recording_mut()) else {
            return;
        };
        let millis = |length: Duration| u64::try_from(length.as_millis()).unwrap_or(u64::MAX);
        for call in calls {
            recording.rpc(Rpc {
                at_ms: millis(call.started.saturating_duration_since(start)),
                machine: "local".to_owned(),
                method: call.method,
                status: call.status,
                duration_ms: millis(call.duration),
                request: None,
                response: None,
                holds: humata2_report::Holds::default(),
            });
        }
    }

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

/// The report's context for a window of `window` as `state` shows it: the
/// app's version and OS, and the daemons, machines and sessions it has loaded.
fn context(window: WindowSize, state: &AppState) -> Context {
    let workspace = &state.workspace;
    let machine_name = |id: &MachineId| {
        workspace
            .machines
            .iter()
            .find(|machine| machine.id == *id)
            .map_or_else(|| id.as_str().to_owned(), |machine| machine.name.clone())
    };
    let machines = workspace
        .machines
        .iter()
        .map(|machine| humata2_report::Machine {
            name: machine.name.clone(),
            kind: match &machine.kind {
                MachineKind::ThisMac => "local",
                MachineKind::Ssh { .. } => "ssh",
                MachineKind::Provider { .. } => "provider",
            }
            .to_owned(),
            state: if machine.online { "online" } else { "offline" }.to_owned(),
        })
        .collect();
    let sessions = workspace
        .sessions
        .iter()
        .map(|session| humata2_report::Session {
            id: session.id.as_str().to_owned(),
            title: session.title.clone(),
            channel: workspace
                .channels
                .iter()
                .find(|channel| channel.id == session.channel)
                .map_or_else(
                    || session.channel.as_str().to_owned(),
                    |channel| channel.name.clone(),
                ),
            machine: machine_name(&session.machine),
            agent: session.agent.name().to_owned(),
        })
        .collect();
    Context {
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        os: std::env::consts::OS.to_owned(),
        window,
        daemons: daemons(state),
        machines,
        sessions,
        ..Context::default()
    }
}

/// Each machine's daemon version as the app has it now, for the report.
fn daemons(state: &AppState) -> Vec<humata2_report::Daemon> {
    state
        .workspace
        .machines
        .iter()
        .filter_map(|machine| {
            Some(humata2_report::Daemon {
                machine: machine.name.clone(),
                version: state.daemon_version(&machine.id)?,
            })
        })
        .collect()
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
fn comment(comment: &PinnedComment, form: &ReportForm) -> Comment {
    let place = form
        .elements
        .iter()
        .position(|target| *target == comment.target)
        .and_then(|at| form.places.get(at).copied().flatten())
        .map(|rect| Bounds {
            x: rect.x,
            y: rect.y,
            width: u32::try_from(rect.width).unwrap_or(0),
            height: u32::try_from(rect.height).unwrap_or(0),
        });
    Comment {
        element: pinned(&comment.target),
        text: comment.text.clone(),
        place,
    }
}

/// `window` as the engine draws it, with the boxes of everything that
/// shows a secret (and of terminals, whose text is scrubbed from reports)
/// filled in. No OS capture is involved. Used for `window.png` and for each
/// video frame.
///
/// # Errors
///
/// [`ReportError::Render`] when the window can't be drawn to an image.
pub(crate) fn render_masked(
    window: &Window,
    secrets: &[String],
) -> Result<image::RgbaImage, ReportError> {
    let mut picture = window
        .render_to_image()
        .map_err(|error| ReportError::Render {
            message: error.to_string(),
        })?;
    let scale = window.scale_factor();
    let mut boxes: Vec<PixelBox> = Vec::new();
    for element in snapshots(window).iter().filter(|element| element.visible()) {
        let terminal = element
            .role()
            .is_some_and(|role| matches!(role, gpui_kit::Role::Terminal | gpui_kit::Role::Log));
        let shows = [
            element.label(),
            element.value(),
            element.description(),
            element.placeholder(),
        ]
        .into_iter()
        .flatten()
        .any(|text| shows_secret(text, secrets));
        if terminal || shows {
            boxes.push(pixel_box(element.bounds(), scale));
        }
    }
    for text in texts(window).iter().filter(|text| text.visible()) {
        if shows_secret(text.text(), secrets) {
            boxes.push(pixel_box(text.bounds(), scale));
        }
    }
    humata2_report::mask(&mut picture, &boxes);
    Ok(picture)
}

/// Whether `text` holds any of `secrets`.
fn shows_secret(text: &str, secrets: &[String]) -> bool {
    secrets
        .iter()
        .any(|secret| !secret.is_empty() && text.contains(secret.as_str()))
}

/// `bounds` in window points as a box in picture pixels at `scale`,
/// rounded outwards so a box never under-covers.
fn pixel_box(bounds: gpui_kit::Bounds<Pixels>, scale: f32) -> PixelBox {
    let left = f32::from(bounds.origin.x) * scale;
    let top = f32::from(bounds.origin.y) * scale;
    let right = (f32::from(bounds.origin.x) + f32::from(bounds.size.width)) * scale;
    let bottom = (f32::from(bounds.origin.y) + f32::from(bounds.size.height)) * scale;
    PixelBox {
        x: left.floor() as i32,
        y: top.floor() as i32,
        width: (right.ceil() - left.floor()).max(0.) as u32,
        height: (bottom.ceil() - top.floor()).max(0.) as u32,
    }
}

/// A pin target as the report names an element.
fn pinned(target: &PinTarget) -> Element {
    target
        .within
        .iter()
        .fold(Element::new(&target.role, &target.label), |element, (role, label)| {
            element.inside(role, label)
        })
}

/// A form's mark as the report keeps it.
fn stroke(stroke: &humata2_app::Stroke) -> humata2_report::Stroke {
    let shape = match stroke.shape {
        humata2_app::Shape::Loop => humata2_report::Shape::Loop,
        humata2_app::Shape::Line => humata2_report::Shape::Line,
        humata2_app::Shape::Dot => humata2_report::Shape::Dot,
    };
    humata2_report::Stroke {
        shape,
        points: stroke
            .points
            .iter()
            .map(|point| humata2_report::Point {
                x: point.x,
                y: point.y,
            })
            .collect(),
        covers: stroke.covers.iter().map(pinned).collect(),
        more: stroke.more,
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
    if state.overlays.iter().any(|overlay| matches!(overlay, Overlay::Palette)) {
        let _ = writeln!(text, "palette: {:?} {:?}", state.palette.query, state.palette.cursor);
    }
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
    pins_placed(elements).0
}

/// [`pins`], and where each is drawn in window points (the first one drawn
/// when a target is on screen more than once).
#[must_use]
pub fn pins_placed(elements: &[ElementSnapshot]) -> (Vec<PinTarget>, Vec<Option<Rect>>) {
    let mut targets: Vec<PinTarget> = Vec::new();
    let mut places: Vec<Option<Rect>> = Vec::new();
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
            places.push(Some(rect(found)));
        }
        if targets.len() >= MAX_PINS {
            break;
        }
    }
    (targets, places)
}

/// Where `snapshot` is drawn, in whole window points.
#[allow(clippy::cast_possible_truncation)]
fn rect(snapshot: &ElementSnapshot) -> Rect {
    let whole = |length: Pixels| f32::from(length).round() as i32;
    let bounds = snapshot.bounds();
    Rect {
        x: whole(bounds.origin.x),
        y: whole(bounds.origin.y),
        width: whole(bounds.size.width),
        height: whole(bounds.size.height),
    }
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
    use std::collections::BTreeMap;

    use humata2_clients::model::{Machine, MachineHealth, MachineId, MachineKind};
    use humata2_clients::{Clients, Workspace, fixture};
    use tca::TestClock;

    use super::*;

    fn vars(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn text_with_a_secret_in_it_is_covered_and_other_text_is_not() {
        let secrets = vec!["ghp_Zx81Qm2Lp0Vb7Rt4".to_owned(), String::new()];
        assert!(shows_secret("token ghp_Zx81Qm2Lp0Vb7Rt4 set", &secrets));
        assert!(!shows_secret("nothing to hide", &secrets));
        assert!(!shows_secret("anything", &[String::new()]));
    }

    #[test]
    fn a_covering_box_scales_with_the_window_and_rounds_outwards() {
        let bounds = gpui_kit::Bounds {
            origin: gpui_kit::point(gpui_kit::px(10.5), gpui_kit::px(20.)),
            size: gpui_kit::size(gpui_kit::px(30.), gpui_kit::px(10.25)),
        };
        let covering = pixel_box(bounds, 2.);
        assert_eq!(
            covering,
            PixelBox {
                x: 21,
                y: 40,
                width: 60,
                height: 21,
            }
        );
    }

    fn recording() -> Recorder {
        let recorder = Recorder::new();
        let started = recorder.start(Context::default(), Trees::default());
        assert!(started.is_ok());
        recorder
    }

    fn call(started: Instant, method: &str) -> RpcCall {
        RpcCall {
            method: method.to_owned(),
            started,
            duration: Duration::from_millis(7),
            status: "ok".to_owned(),
        }
    }

    fn rpcs(recorder: &Recorder) -> Vec<Rpc> {
        let Ok(captured) = recorder.stop() else {
            return Vec::new();
        };
        captured
            .events()
            .iter()
            .filter_map(|event| match event {
                humata2_report::Event::Rpc(rpc) => Some(rpc.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn calls_outside_a_recording_are_dropped() {
        let recorder = Recorder::new();
        let feed = recorder.rpc_feed();
        feed.push(call(Instant::now(), "a/B"));
        let started = recorder.start(Context::default(), Trees::default());
        assert!(started.is_ok());
        assert!(rpcs(&recorder).is_empty());
    }

    #[test]
    fn calls_that_began_before_the_recording_are_dropped() {
        let before = Instant::now();
        std::thread::sleep(Duration::from_millis(5));
        let recorder = recording();
        let feed = recorder.rpc_feed();
        feed.push(call(before, "old/Call"));
        feed.push(call(Instant::now(), "new/Call"));
        let kept = rpcs(&recorder);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].method, "new/Call");
        assert_eq!(kept[0].machine, "local");
        assert_eq!(kept[0].duration_ms, 7);
        assert_eq!(kept[0].request, None);
    }

    #[test]
    fn a_call_is_told_at_its_offset_into_the_recording() {
        let recorder = recording();
        std::thread::sleep(Duration::from_millis(30));
        recorder
            .rpc_feed()
            .push(call(Instant::now(), "later/Call"));
        let kept = rpcs(&recorder);
        assert_eq!(kept.len(), 1);
        assert!(kept[0].at_ms >= 30);
    }

    #[test]
    fn a_stroke_covers_the_elements_it_names() {
        let target = PinTarget {
            role: "Button".to_owned(),
            label: "Save".to_owned(),
            within: vec![("Dialog".to_owned(), "Edit".to_owned())],
        };
        let mark = humata2_app::Stroke {
            shape: humata2_app::Shape::Loop,
            points: vec![humata2_app::Point { x: 3, y: 4 }],
            covers: vec![target.clone()],
            more: 2,
        };
        let made = stroke(&mark);
        assert_eq!(made.shape, humata2_report::Shape::Loop);
        assert_eq!(made.points, vec![humata2_report::Point { x: 3, y: 4 }]);
        assert_eq!(made.covers, vec![pinned(&target)]);
        assert_eq!(made.more, 2);
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

    fn key_stroke(modifiers: gpui_kit::Modifiers, key: &str, key_char: Option<&str>) -> gpui_kit::Keystroke {
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
        assert_eq!(keystroke_gesture(&key_stroke(held, "r", None), None, None), None);
        assert_eq!(keystroke_gesture(&key_stroke(held, "s", None), None, None), None);
    }

    #[test]
    fn printable_keys_are_typing_and_others_are_keys() {
        let typed = keystroke_gesture(
            &key_stroke(gpui_kit::Modifiers::default(), "a", Some("a")),
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
        let key = keystroke_gesture(&key_stroke(command, "k", Some("k")), None, None);
        assert_eq!(
            key,
            Some(Gesture::Key {
                key: "secondary-k".to_owned(),
                target: None
            })
        );
        let enter = keystroke_gesture(
            &key_stroke(gpui_kit::Modifiers::default(), "enter", Some("\n")),
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
    fn the_report_context_lists_the_daemons_machines_and_sessions_the_app_has() {
        let machine = Machine {
            id: MachineId::new("this-mac"),
            name: "This Mac".to_owned(),
            kind: MachineKind::ThisMac,
            online: true,
            detail: String::new(),
            last_seen: "now".to_owned(),
            lost_at: None,
        };
        let mut session = fixture::standard().sessions.remove(0);
        session.machine = machine.id.clone();
        let workspace = Workspace {
            machines: vec![machine.clone()],
            sessions: vec![session.clone()],
            machine_health: BTreeMap::from([(
                machine.id.clone(),
                MachineHealth {
                    daemon: "2.0.3".to_owned(),
                    ..MachineHealth::default()
                },
            )]),
            ..Workspace::default()
        };
        let (clients, _) = Clients::fake(workspace.clone(), Arc::new(TestClock::new()));
        let mut state = AppState::launch(&clients);
        state.workspace = Arc::new(workspace);

        let report = context(WindowSize::default(), &state);

        assert_eq!(
            report.daemons,
            vec![humata2_report::Daemon {
                machine: "This Mac".to_owned(),
                version: "2.0.3".to_owned(),
            }]
        );
        assert_eq!(
            report.machines,
            vec![humata2_report::Machine {
                name: "This Mac".to_owned(),
                kind: "local".to_owned(),
                state: "online".to_owned(),
            }]
        );
        assert_eq!(
            report.sessions,
            vec![humata2_report::Session {
                id: session.id.as_str().to_owned(),
                title: session.title.clone(),
                channel: session.channel.as_str().to_owned(),
                machine: "This Mac".to_owned(),
                agent: session.agent.name().to_owned(),
            }]
        );
    }

    #[test]
    fn a_paste_carries_the_clipboard_text() {
        let command = gpui_kit::Modifiers {
            platform: true,
            ..gpui_kit::Modifiers::default()
        };
        let pasted = keystroke_gesture(&key_stroke(command, "v", None), None, Some("hi".to_owned()));
        assert_eq!(
            pasted,
            Some(Gesture::Paste {
                target: None,
                content: Pasted::Text("hi".to_owned())
            })
        );
    }
}

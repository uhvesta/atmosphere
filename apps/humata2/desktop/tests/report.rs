//! I9: bug reports as the app does them. A person records, edits the form,
//! pins a comment and copies the report; takes a snapshot; replays a saved
//! recording on a fresh app; and an agent starts and stops a recording over
//! the socket. All of it headless: no window is shown.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use gpui_kit::{
    AnyWindowHandle, Bounds, HeadlessAppContext, InputEvent as _, MouseButton, MouseDownEvent,
    MouseUpEvent, Role, WindowBounds, WindowOptions, point, px, size,
};
use humata2_app::{AppAction, ReportAction, ReportForm, ReportKind, ToastAction};
use humata2_clients::{Clients, Fakes, fixture};
use humata2_engine::Resources;
use humata2_flow::{Action, Driver as _, FrameMode, GpuiDriver, GpuiDriverError, Target};
use humata2_report::remote::{self, RefusalKind, Request};
use humata2_report::{Compare, Event, FlowAction, FlowTarget, Gesture, Player, Trees, read_events};
use humata2_ui::bridge::Ui;
use humata2_ui::record::{Recorder, RpcCall, state_text};
use tca::TestClock;
use tokio::runtime::Runtime;

/// ⇧⌘R and ⇧⌘S, as flows write keys (`secondary` is ⌘ on macOS).
const RECORD: &str = "shift-secondary-r";
const SNAPSHOT: &str = "shift-secondary-s";

/// The app headless on the standard fakes, with a recorder saving reports
/// in `reports` (when given).
struct App {
    runtime: Runtime,
    cx: HeadlessAppContext,
    window: AnyWindowHandle,
    ui: Rc<Ui>,
    _fakes: Fakes,
}

impl App {
    fn new(recorder: Recorder) -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a current-thread runtime starts");
        let mut cx = humata2_engine::headless(&Resources::new(humata2_ui::Assets))
            .expect("the headless context starts");
        cx.update(humata2_ui::init);
        let (clients, fakes) = Clients::fake(fixture::standard(), Arc::new(TestClock::new()));
        let ui = Ui::new(runtime.handle(), clients, recorder);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(0.), px(0.)),
                size(px(humata2_ui::WINDOW.0), px(humata2_ui::WINDOW.1)),
            ))),
            show: false,
            focus: false,
            ..WindowOptions::default()
        };
        let (window, _shell) = cx
            .update(|cx| humata2_ui::open(cx, &ui, options))
            .expect("the window opens");
        let mut app = Self {
            runtime,
            cx,
            window,
            ui,
            _fakes: fakes,
        };
        app.settle();
        app
    }

    /// An app whose reports go in `folder`.
    fn saving_to(folder: &Path) -> Self {
        Self::new(Recorder::new().saving_to(folder.to_path_buf()))
    }

    /// Runs what the last step started, then draws a frame as the app would.
    fn settle(&mut self) {
        for _ in 0..10 {
            self.runtime.block_on(async {
                for _ in 0..32 {
                    tokio::task::yield_now().await;
                }
            });
            self.cx.run_until_parked();
        }
        self.cx
            .update_window(self.window, |_, window, cx| {
                humata2_engine::frame::natural_frame(window, cx);
            })
            .expect("the window stays open");
    }

    fn play(&mut self, action: &Action) -> Result<(), GpuiDriverError> {
        GpuiDriver::with_frames(&mut self.cx, self.window, FrameMode::Natural).perform(action)?;
        self.settle();
        Ok(())
    }

    fn press(&mut self, key: &str) {
        self.play(&Action::Key(key.to_owned()))
            .expect("the key can be pressed");
    }

    fn type_text(&mut self, text: &str) {
        self.play(&Action::Type(text.to_owned()))
            .expect("the text can be typed");
    }

    fn click(&mut self, role: Role, label: &str) {
        self.play(&Action::Click(Target::new(role, label)))
            .expect("the control is drawn and can be clicked");
    }

    fn send(&mut self, action: ReportAction) {
        self.cx
            .update(|cx| self.ui.send(AppAction::Report(action), cx));
        self.settle();
    }

    fn recording(&self) -> bool {
        self.ui.read(|state| state.report.recording)
    }

    fn form(&self) -> Option<ReportForm> {
        self.ui.read(|state| state.report.form.clone())
    }

    fn clipboard(&mut self) -> Option<String> {
        self.cx
            .update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
    }

    /// Records a few interactions: a click in the sidebar, the palette
    /// opened and `typed` typed into it. Returns the form the stop opened.
    fn record(&mut self, typed: &str) -> ReportForm {
        self.press(RECORD);
        assert!(self.recording(), "the shortcut starts a recording");
        assert!(self.ui.recorder().is_recording());
        self.click(Role::Link, "Inbox");
        self.press("secondary-k");
        self.type_text(typed);
        self.press(RECORD);
        assert!(!self.recording(), "the shortcut stops it again");
        assert!(!self.ui.recorder().is_recording());
        self.form().expect("stopping from the keyboard opens the form")
    }

    /// Draws `count` more frames, for what acts a couple of frames later.
    fn frames(&mut self, count: usize) {
        for _ in 0..count {
            self.settle();
        }
    }

    /// Opens the palette, types `query` and presses Enter, then lets the
    /// frames pass that a report command waits for.
    fn palette(&mut self, query: &str) {
        self.press("secondary-k");
        self.type_text(query);
        self.press("enter");
        self.frames(4);
    }

    /// Draws one stroke through `points` (window points): the pointer goes
    /// to the first, the left button goes down, moves through the rest and
    /// goes up at the last, a natural frame after each event.
    fn stroke(&mut self, points: &[(f32, f32)]) {
        let at = |(x, y): (f32, f32)| point(px(x), px(y));
        let (Some(&first), Some(&last)) = (points.first(), points.last()) else {
            return;
        };
        self.cx
            .update_window(self.window, |_, window, cx| {
                let modifiers = window.modifiers();
                humata2_engine::frame::move_pointer(window, at(first), None, cx);
                window.dispatch_event(
                    MouseDownEvent {
                        button: MouseButton::Left,
                        position: at(first),
                        modifiers,
                        click_count: 1,
                        first_mouse: false,
                    }
                    .to_platform_input(),
                    cx,
                );
                humata2_engine::frame::natural_frame(window, cx);
                for &next in &points[1..] {
                    humata2_engine::frame::move_pointer(window, at(next), Some(MouseButton::Left), cx);
                }
                window.dispatch_event(
                    MouseUpEvent {
                        button: MouseButton::Left,
                        position: at(last),
                        modifiers,
                        click_count: 1,
                    }
                    .to_platform_input(),
                    cx,
                );
                humata2_engine::frame::natural_frame(window, cx);
            })
            .expect("the window stays open");
        self.settle();
    }

    /// Where the form saw the element called `label` with `role` when it
    /// was taken: (x, y, width, height) in window points.
    fn place(&self, role: &str, label: &str) -> (f32, f32, f32, f32) {
        let form = self.form().expect("the form is open");
        let at = form
            .elements
            .iter()
            .position(|e| e.role == role && e.label == label)
            .unwrap_or_else(|| panic!("the form offers {role} {label:?}"));
        let rect = form
            .places
            .get(at)
            .copied()
            .flatten()
            .expect("the element has a place on the window");
        #[allow(clippy::cast_precision_loss)]
        (rect.x as f32, rect.y as f32, rect.width as f32, rect.height as f32)
    }

    /// Clicks Copy report and gives what it put on the clipboard.
    fn copy(&mut self) -> String {
        self.click(Role::Button, "Copy report");
        self.clipboard().expect("Copy report writes the clipboard")
    }
}

/// Every file under `dir`, by relative name, with its text.
/// Every file in the report folder, by its path in the folder.
fn folder_files(dir: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(root: &Path, dir: &Path, found: &mut Vec<(String, Vec<u8>)>) {
        for entry in fs::read_dir(dir).expect("the report folder can be read") {
            let path = entry.expect("a folder entry").path();
            if path.is_dir() {
                walk(root, &path, found);
            } else {
                let name = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .display()
                    .to_string();
                let bytes = fs::read(&path).expect("report files can be read");
                found.push((name, bytes));
            }
        }
    }
    let mut found = Vec::new();
    walk(dir, dir, &mut found);
    found.sort();
    found
}

/// The report folder's text files (the pictures and video are left out).
fn folder_text(dir: &Path) -> Vec<(String, String)> {
    folder_files(dir)
        .into_iter()
        .filter_map(|(name, bytes)| String::from_utf8(bytes).ok().map(|text| (name, text)))
        .collect()
}

fn reports_folder() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary folder")
}

/// Whether the report folder holds a file called `name`.
fn has_file(dir: &Path, name: &str) -> bool {
    dir.join(name).is_file()
}

#[test]
fn a_recording_becomes_a_form_a_pinned_comment_and_a_copied_report() {
    let reports = reports_folder();
    let mut app = App::saving_to(reports.path());

    let form = app.record("hello");

    assert_eq!(form.kind, ReportKind::Recording);
    assert!(form.folder.is_absolute(), "the form names where the report is");
    assert!(form.folder.starts_with(reports.path()));
    assert!(form.folder.is_dir());
    for file in ["report.md", "events.jsonl", "state.txt", "view.txt", "layout.txt"] {
        assert!(has_file(&form.folder, file), "{file} is saved the moment recording stops");
    }
    for file in ["window.png", "video.png", "video.jsonl"] {
        assert!(has_file(&form.folder, file), "{file} is saved with a recording (D160)");
    }
    let png = fs::read(form.folder.join("window.png")).expect("window.png can be read");
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"), "window.png is a PNG");
    assert!(!form.elements.is_empty(), "the form offers the controls on screen to pin");

    app.send(ReportAction::Happened("the inbox looked wrong".to_owned()));
    app.send(ReportAction::Expected("a list of requests".to_owned()));
    app.send(ReportAction::Query("Inbox".to_owned()));
    let matches = app.form().expect("the form is open").matches();
    let first = *matches.first().expect("Inbox can be pinned");
    let pinned = app.form().expect("the form is open").elements[first].clone();
    app.send(ReportAction::Pin(first));
    app.send(ReportAction::CommentText(0, "this link is the one".to_owned()));
    let edited = app.form().expect("the form is open");
    assert_eq!(edited.comments.len(), 1);
    assert_eq!(edited.comments[0].target, pinned);
    assert!(!edited.copied, "editing the form leaves nothing copied yet");

    let prompt = app.copy();

    let copied = app.form().expect("the form stays open after copying");
    assert!(copied.copied, "Copy report says it worked");
    assert!(copied.error.is_none());
    assert!(
        prompt.contains(&form.folder.display().to_string()),
        "the prompt tells an agent where the report is"
    );
    let saved = folder_text(&form.folder);
    for details in ["the inbox looked wrong", "a list of requests", "this link is the one"] {
        assert!(
            saved.iter().any(|(_, text)| text.contains(details)),
            "the saved report holds {details:?}"
        );
    }

    app.click(Role::Button, "Done");
    assert!(app.form().is_none(), "Done closes the form");
}

#[test]
fn a_snapshot_opens_the_form_and_copies() {
    let reports = reports_folder();
    let mut app = App::saving_to(reports.path());

    app.press(SNAPSHOT);

    assert!(!app.recording(), "a snapshot starts no recording");
    let form = app.form().expect("a snapshot from the keyboard opens the form");
    assert_eq!(form.kind, ReportKind::Snapshot);
    assert!(form.folder.starts_with(reports.path()));
    assert!(has_file(&form.folder, "report.md"));
    let prompt = app.copy();
    assert!(prompt.contains(&form.folder.display().to_string()));
    assert!(app.form().is_some_and(|form| form.copied));
}

/// Plays a flow script's steps in a fresh app.
struct Replay<'a>(&'a mut App);

#[derive(Debug)]
enum PlayError {
    Drive(GpuiDriverError),
    Role(String),
}

impl fmt::Display for PlayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Drive(error) => write!(f, "the step could not be played: {error}"),
            Self::Role(role) => write!(
                f,
                "this test has no role named {role}; add it to `role` in tests/report.rs"
            ),
        }
    }
}

impl std::error::Error for PlayError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Drive(error) => Some(error),
            Self::Role(_) => None,
        }
    }
}

fn role(name: &str) -> Result<Role, PlayError> {
    match name {
        "Button" => Ok(Role::Button),
        "Link" => Ok(Role::Link),
        "TreeItem" => Ok(Role::TreeItem),
        other => Err(PlayError::Role(other.to_owned())),
    }
}

fn target(target: &FlowTarget) -> Result<Target, PlayError> {
    Ok(Target::new(role(&target.role)?, target.label.clone()))
}

impl Player for Replay<'_> {
    type Error = PlayError;

    fn play(&mut self, action: &FlowAction) -> Result<(), PlayError> {
        let action = match action {
            FlowAction::Click(on) => Action::Click(target(on)?),
            FlowAction::Type(text) => Action::Type(text.clone()),
            FlowAction::Key(key) => Action::Key(key.clone()),
            FlowAction::Drag { from, to } => Action::Drag {
                from: target(from)?,
                to: target(to)?,
            },
            FlowAction::Paste(text) => Action::Paste(text.clone()),
        };
        self.0.play(&action).map_err(PlayError::Drive)
    }

    fn trees(&mut self) -> Result<Trees, PlayError> {
        // Only the state is compared (the replay's default), so the app's
        // view and layout need not match a recording made in another window.
        let state = self.0.ui.read(state_text);
        Ok(Trees {
            state,
            view: String::new(),
            layout: String::new(),
        })
    }
}

#[test]
fn a_saved_recording_replays_on_a_fresh_app_to_the_same_state() {
    let reports = reports_folder();
    let mut recorded = App::saving_to(reports.path());
    let form = recorded.record("hello");
    let events = fs::read_to_string(form.folder.join("events.jsonl")).expect("events.jsonl is saved");
    let script = read_events(&events)
        .expect("the saved events read back")
        .flow_script();
    assert!(
        !script.steps.is_empty(),
        "the clicks and typing became steps: {:?}",
        script.skipped
    );

    let mut fresh = App::new(Recorder::new());
    let replayed = script.replay(&mut Replay(&mut fresh), Compare::default());

    assert!(replayed.is_ok(), "{replayed:?}");
    assert!(!fresh.ui.recorder().is_recording(), "a replay records nothing");
}

#[test]
fn a_secret_typed_while_recording_is_in_no_file_of_the_report() {
    const SECRET: &str = "sekret-value-123";
    let reports = reports_folder();
    let recorder = Recorder::new().saving_to(reports.path().to_path_buf());
    recorder.seed("TEST_TOKEN", SECRET);
    let mut app = App::new(recorder);

    let form = app.record(SECRET);
    app.send(ReportAction::Happened(format!("typed {SECRET} and it vanished")));
    let prompt = app.copy();

    let events = fs::read_to_string(form.folder.join("events.jsonl")).expect("events.jsonl is saved");
    let typed = read_events(&events)
        .expect("the saved events read back")
        .events()
        .iter()
        .any(|event| matches!(event, Event::Interaction(i) if matches!(i.gesture, Gesture::Type { .. })));
    assert!(typed, "the typing was recorded, so the check below is not vacuous");
    let files = folder_files(&form.folder);
    assert!(
        files.iter().any(|(name, _)| name == "window.png"),
        "the picture is saved, so the byte check below covers it"
    );
    for (name, bytes) in &files {
        assert!(
            !bytes.windows(SECRET.len()).any(|window| window == SECRET.as_bytes()),
            "{name} holds the secret"
        );
        if name.ends_with(".png") {
            for chunk in [b"tEXt", b"zTXt", b"iTXt"] {
                assert!(
                    !bytes.windows(4).any(|window| window == chunk),
                    "{name} carries a text chunk, which could hold the secret compressed"
                );
            }
        }
    }
    assert!(!prompt.contains(SECRET), "the copied prompt holds the secret");
}

/// Runs the app until `done` gives a value or the time is up; the socket's
/// work happens on the app's runtime and its foreground, so both turn.
fn drive<T>(app: &mut App, mut done: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(found) = done() {
            return found;
        }
        assert!(Instant::now() < deadline, "the socket never answered");
        app.runtime.block_on(async {
            for _ in 0..2000 {
                tokio::task::yield_now().await;
            }
        });
        app.cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn the_socket_records_and_stops_without_opening_the_form() {
    let reports = reports_folder();
    // Unix socket paths are short, so not under the long temp folder.
    let socket_dir = tempfile::Builder::new()
        .prefix("h2")
        .tempdir_in("/tmp")
        .expect("a short temporary folder");
    let socket = socket_dir.path().join("a.sock");
    let mut app = App::saving_to(reports.path());
    let handle = app.runtime.handle().clone();
    app.cx
        .update(|cx| humata2_ui::report_view::serve(cx, &app.ui, app.window, &handle, &socket))
        .expect("the socket binds");

    let (send, receive) = mpsc::channel();
    let path = socket.clone();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a client runtime starts");
        let answers: Vec<_> = runtime.block_on(async {
            let mut answers = Vec::new();
            for request in [
                Request::Record { title: None },
                Request::Stop,
                Request::Stop,
            ] {
                answers.push(
                    remote::ask(&path, &request)
                        .await
                        .map_err(|error| error.to_string()),
                );
            }
            answers
        });
        let _ = send.send(answers);
    });
    let answers = drive(&mut app, || receive.try_recv().ok());
    app.settle();

    let [record, stop, again] = answers.as_slice() else {
        panic!("three answers came back, got {answers:?}");
    };
    let record = record.as_ref().expect("the app answered Record").as_ref().expect("Record is accepted");
    assert!(record.recording);
    let stop = stop.as_ref().expect("the app answered Stop").as_ref().expect("Stop is accepted");
    assert!(!stop.recording);
    let folder = PathBuf::from(stop.folder.clone().expect("Stop names the saved folder"));
    assert!(folder.is_dir(), "the saved report exists");
    assert!(folder.starts_with(reports.path()));

    let refused = again
        .as_ref()
        .expect("the app answered the second Stop")
        .as_ref()
        .expect_err("Stop with no recording is refused");
    assert_eq!(refused.kind, RefusalKind::NotRecording);

    assert!(app.form().is_none(), "a save the CLI asked for does not take over the window");
    assert!(!app.recording());
    let offered = app
        .ui
        .read(|state| {
            state.toasts.iter().find_map(|toast| match &toast.action {
                Some(ToastAction::OpenReport(form)) => Some(form.clone()),
                _ => None,
            })
        })
        .expect("a toast offers to open the report");
    assert_eq!(offered.folder, folder);
}

/// The events a saved recording holds.
fn saved_events(folder: &Path) -> Vec<Event> {
    let text = fs::read_to_string(folder.join("events.jsonl")).expect("events.jsonl is saved");
    read_events(&text)
        .expect("the saved events read back")
        .events()
        .to_vec()
}

/// Replays the saved recording in `folder` on a fresh app and expects the
/// same end state.
fn assert_replays(folder: &Path) {
    let text = fs::read_to_string(folder.join("events.jsonl")).expect("events.jsonl is saved");
    let script = read_events(&text)
        .expect("the saved events read back")
        .flow_script();
    assert!(!script.steps.is_empty(), "there are steps to replay: {:?}", script.skipped);
    let mut fresh = App::new(Recorder::new());
    let replayed = script.replay(&mut Replay(&mut fresh), Compare::default());
    assert!(replayed.is_ok(), "{replayed:?}");
}

#[test]
fn palette_snapshot_opens_the_form() {
    let reports = reports_folder();
    let mut app = App::saving_to(reports.path());

    app.palette("snapshot");

    let form = app.form().expect("the palette's snapshot opens the form");
    assert_eq!(form.kind, ReportKind::Snapshot);
    assert!(form.folder.is_dir());
    assert!(form.folder.starts_with(reports.path()));
    let view = fs::read_to_string(form.folder.join("view.txt")).expect("view.txt is saved");
    assert!(
        !view.contains("snapshot for a bug report"),
        "the palette was gone when the snapshot was taken"
    );
}

#[test]
fn palette_records_and_stops() {
    let reports = reports_folder();
    let mut app = App::saving_to(reports.path());

    app.palette("start recording");
    assert!(app.recording(), "the palette starts a recording");
    assert!(app.ui.recorder().is_recording());
    app.click(Role::Link, "Inbox");
    app.press("secondary-k");
    app.type_text("hello");
    app.press("escape");
    app.palette("stop recording");

    assert!(!app.recording());
    assert!(!app.ui.recorder().is_recording());
    let form = app.form().expect("stopping from the palette opens the form");
    assert_eq!(form.kind, ReportKind::Recording);
    let events = saved_events(&form.folder);
    for event in &events {
        match event {
            Event::Action(step) => assert!(
                !step.action.contains("Request"),
                "the stop command's own step is cut: {}",
                step.action
            ),
            Event::Interaction(i) => {
                if let Gesture::Type { text, .. } = &i.gesture {
                    assert!(!text.contains("stop"), "the palette typing for the stop is cut");
                }
            }
            _ => {}
        }
    }
    assert!(
        events.iter().any(|event| matches!(event, Event::Interaction(_))),
        "the steps before the stop were kept"
    );
    // Typing may be saved a key at a time, so read it joined up.
    let typed: String = events
        .iter()
        .filter_map(|event| match event {
            Event::Interaction(i) => match &i.gesture {
                Gesture::Type { text, .. } => Some(text.as_str()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert!(typed.contains("hello"), "the palette typing before the stop is kept: {typed:?}");
    assert!(!typed.contains("stop"), "the palette typing for the stop is cut: {typed:?}");
    let opens = events
        .iter()
        .filter(|event| {
            matches!(event, Event::Interaction(i)
                if matches!(&i.gesture, Gesture::Key { key, .. } if key == "secondary-k"))
        })
        .count();
    assert_eq!(opens, 1, "only the palette opened in the middle is kept, not the stop's");
    assert_replays(&form.folder);
}

#[test]
fn open_last_report_reopens_the_edited_form() {
    let reports = reports_folder();
    let mut app = App::saving_to(reports.path());
    app.palette("snapshot");
    let folder = app.form().expect("the snapshot opens the form").folder;
    app.send(ReportAction::Happened("the sidebar flickers".to_owned()));
    app.press("escape");
    if app.form().is_some() {
        app.send(ReportAction::Close);
    }
    assert!(app.form().is_none(), "the form is closed");

    app.palette("open the last");

    let form = app.form().expect("the last report opens again");
    assert_eq!(form.folder, folder);
    assert_eq!(form.happened, "the sidebar flickers");
}

#[test]
fn draw_circle_names_the_element() {
    let reports = reports_folder();
    let mut app = App::saving_to(reports.path());
    app.press(SNAPSHOT);
    let folder = app.form().expect("the snapshot opens the form").folder;
    let (x, y, width, height) = app.place("Link", "Inbox");

    app.click(Role::Button, "Draw on the window");
    assert!(app.form().is_some_and(|form| form.drawing), "draw mode is on");

    // A loop round the row, 16 steps, back where it began.
    let (cx, cy) = (x + width / 2., y + height / 2.);
    let (rx, ry) = (width / 2. + 4., height / 2. + 4.);
    let ring: Vec<(f32, f32)> = (0..=16)
        .map(|step| {
            #[allow(clippy::cast_precision_loss)]
            let angle = step as f32 / 16. * std::f32::consts::TAU;
            (cx + rx * angle.cos(), cy + ry * angle.sin())
        })
        .collect();
    app.stroke(&ring);
    let strokes = app.form().expect("the form is open").strokes;
    assert_eq!(strokes.len(), 1, "the loop is one stroke");
    assert!(strokes[0].points.len() >= 12, "the stroke keeps its path");
    assert!(
        strokes[0].covers.iter().any(|c| c.label == "Inbox"),
        "the loop covers the row: {:?}",
        strokes[0].covers
    );

    // A second mark, away from the tools, then undo takes it back.
    app.stroke(&[(500., 400.), (560., 420.), (620., 400.), (680., 430.)]);
    assert_eq!(app.form().expect("the form is open").strokes.len(), 2);
    app.click(Role::Button, "Undo last mark");
    let after_undo = app.form().expect("the form is open").strokes;
    assert_eq!(after_undo.len(), 1, "Undo removes the last mark");
    assert!(after_undo[0].covers.iter().any(|c| c.label == "Inbox"));

    app.click(Role::Button, "Finish drawing");
    assert!(app.form().is_some_and(|form| !form.drawing), "Done leaves draw mode");
    let prompt = app.copy();

    assert!(prompt.contains("Inbox"), "the prompt names the row: {prompt}");
    assert!(prompt.contains("circle around"), "the prompt names the mark: {prompt}");
    let drawing = fs::read_to_string(folder.join("drawing.json")).expect("drawing.json is saved");
    let text = drawing.trim();
    assert!(text.starts_with('{') && text.ends_with('}'), "drawing.json is a JSON object");
    assert_eq!(drawing.matches("\"shape\"").count(), 1, "one stroke is saved");
    assert!(drawing.contains("loop"));
    assert!(drawing.contains("Inbox"), "the stroke's covers name the row");
    assert!(has_file(&folder, "window.png"), "a snapshot saves a picture of the window");
    let marks = fs::read_to_string(folder.join("marks.svg")).expect("marks.svg is saved with a mark");
    assert!(marks.contains("window.png"), "the marks are drawn over the picture");
    assert_eq!(marks.matches("<polyline").count(), 1, "the one kept stroke is drawn");
}

fn rpc(method: &str) -> RpcCall {
    RpcCall {
        method: method.to_owned(),
        started: Instant::now(),
        duration: Duration::from_millis(7),
        status: "ok".to_owned(),
    }
}

#[test]
fn rpc_calls_are_recorded_and_replay_still_matches() {
    let reports = reports_folder();
    let mut app = App::saving_to(reports.path());
    let feed = app.ui.recorder().rpc_feed();

    feed.push(rpc("humata.v1.Test/BeforeRecording"));
    app.press(RECORD);
    assert!(app.recording());
    feed.push(rpc("humata.v1.Test/First"));
    app.click(Role::Link, "Inbox");
    feed.push(rpc("humata.v1.Test/Second"));
    app.press(RECORD);
    let form = app.form().expect("stopping opens the form");
    feed.push(rpc("humata.v1.Test/AfterStop"));

    let events = saved_events(&form.folder);
    let calls: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            Event::Rpc(call) => Some(call),
            _ => None,
        })
        .collect();
    let methods: Vec<&str> = calls.iter().map(|call| call.method.as_str()).collect();
    assert_eq!(methods, ["humata.v1.Test/First", "humata.v1.Test/Second"]);
    for call in &calls {
        assert_eq!(call.machine, "local");
        assert_eq!(call.status, "ok");
        assert!(call.request.is_none() && call.response.is_none(), "no bodies are kept");
    }
    assert_replays(&form.folder);

    // The call after the stop belongs to no recording: a later one has none.
    app.send(ReportAction::Close);
    app.press(RECORD);
    app.click(Role::Link, "Inbox");
    app.press(RECORD);
    let later = app.form().expect("the second stop opens the form");
    assert!(
        !saved_events(&later.folder)
            .iter()
            .any(|event| matches!(event, Event::Rpc(_))),
        "a call made between recordings is in neither"
    );
}

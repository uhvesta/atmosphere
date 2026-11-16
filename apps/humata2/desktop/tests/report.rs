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
    AnyWindowHandle, Bounds, HeadlessAppContext, Role, WindowBounds, WindowOptions, point, px, size,
};
use humata2_app::{AppAction, ReportAction, ReportForm, ReportKind, ToastAction};
use humata2_clients::{Clients, Fakes, fixture};
use humata2_engine::Resources;
use humata2_flow::{Action, Driver as _, FrameMode, GpuiDriver, GpuiDriverError, Target};
use humata2_report::remote::{self, RefusalKind, Request};
use humata2_report::{Compare, Event, FlowAction, FlowTarget, Gesture, Player, Trees, read_events};
use humata2_ui::bridge::Ui;
use humata2_ui::record::{Recorder, state_text};
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

    /// Clicks Copy report and gives what it put on the clipboard.
    fn copy(&mut self) -> String {
        self.click(Role::Button, "Copy report");
        self.clipboard().expect("Copy report writes the clipboard")
    }
}

/// Every file under `dir`, by relative name, with its text.
fn folder_text(dir: &Path) -> Vec<(String, String)> {
    fn walk(root: &Path, dir: &Path, found: &mut Vec<(String, String)>) {
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
                let text = fs::read_to_string(&path).expect("report files are text");
                found.push((name, text));
            }
        }
    }
    let mut found = Vec::new();
    walk(dir, dir, &mut found);
    found.sort();
    found
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
    for (name, text) in folder_text(&form.folder) {
        assert!(!text.contains(SECRET), "{name} holds the secret");
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

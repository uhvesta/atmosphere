//! The bug-report form: recording, saving, editing and closing.
use std::path::PathBuf;
use std::sync::Arc;

use humata2_app::{
    App, AppAction, AppState, PinTarget, Point, Rect, ReportAction, ReportForm, ReportKind,
    ReportRequest, Shape, ToastAction,
};
use humata2_clients::{Clients, fixture};
use tca::{TestClock, TestStore};

fn target(role: &str, label: &str) -> PinTarget {
    PinTarget {
        role: role.to_owned(),
        label: label.to_owned(),
        within: Vec::new(),
    }
}

fn form(kind: ReportKind) -> ReportForm {
    ReportForm::new(
        PathBuf::from("/tmp/report"),
        kind,
        "It broke".to_owned(),
        vec![target("Button", "Add"), target("List", "Items")],
    )
}

fn store(state: impl FnOnce(&mut AppState)) -> TestStore<App> {
    let (clients, _) = Clients::fake(fixture::design(), Arc::new(TestClock::new()));
    let mut initial = AppState::launch(&clients);
    state(&mut initial);
    TestStore::new(initial, App::new(clients)).non_exhaustive()
}

fn send(store: &mut TestStore<App>, action: ReportAction) {
    store.send(AppAction::Report(action), |_| {});
}

#[test]
fn saved_recording_stops_recording_and_opens_the_form() {
    let mut store = store(|_| {});
    send(&mut store, ReportAction::Started);
    assert!(store.state().report.recording);
    let saved = form(ReportKind::Recording);
    send(&mut store, ReportAction::Saved { form: saved.clone(), open: true });
    assert!(!store.state().report.recording);
    assert_eq!(store.state().report.form, Some(saved));
}

#[test]
fn saved_snapshot_leaves_recording_alone() {
    let mut store = store(|s| s.report.recording = true);
    send(&mut store, ReportAction::Saved { form: form(ReportKind::Snapshot), open: true });
    assert!(store.state().report.recording);
}

#[test]
fn saved_without_open_offers_the_form_in_a_toast() {
    let mut store = store(|_| {});
    let saved = form(ReportKind::Snapshot);
    send(&mut store, ReportAction::Saved { form: saved.clone(), open: false });
    assert_eq!(store.state().report.form, None);
    let toast = store.state().toasts.last().expect("a toast").clone();
    assert_eq!(toast.action, Some(ToastAction::OpenReport(Box::new(saved.clone()))));
    store.send(AppAction::UndoToast(toast.id), |_| {});
    assert_eq!(store.state().report.form, Some(saved));
    assert!(store.state().toasts.is_empty());
}

#[test]
fn failed_sets_recording_and_toasts() {
    let mut store = store(|s| s.report.recording = true);
    send(&mut store, ReportAction::Failed { message: "no".to_owned(), recording: false });
    assert!(!store.state().report.recording);
    assert_eq!(store.state().toasts.len(), 1);
}

#[test]
fn pinning_edits_comments_and_resets_copied() {
    let mut store = store(|s| s.report.form = Some(form(ReportKind::Snapshot)));
    send(&mut store, ReportAction::Copied(Ok(())));
    assert!(store.state().report.form.as_ref().is_some_and(|f| f.copied));
    send(&mut store, ReportAction::Pin(1));
    let f = store.state().report.form.clone().expect("form");
    assert!(!f.copied);
    assert_eq!(f.comments.len(), 1);
    assert_eq!(f.comments[0].target, target("List", "Items"));
    send(&mut store, ReportAction::Pin(9));
    assert_eq!(store.state().report.form.as_ref().map(|f| f.comments.len()), Some(1));
    send(&mut store, ReportAction::Copied(Ok(())));
    send(&mut store, ReportAction::CommentText(0, "wrong".to_owned()));
    let f = store.state().report.form.clone().expect("form");
    assert_eq!(f.comments[0].text, "wrong");
    assert!(!f.copied);
    send(&mut store, ReportAction::Copied(Ok(())));
    send(&mut store, ReportAction::Unpin(0));
    let f = store.state().report.form.clone().expect("form");
    assert!(f.comments.is_empty());
    assert!(!f.copied);
}

#[test]
fn copied_is_cleared_by_an_edit_and_errors_are_kept() {
    let mut store = store(|s| s.report.form = Some(form(ReportKind::Snapshot)));
    send(&mut store, ReportAction::Copied(Ok(())));
    send(&mut store, ReportAction::Expected("fine".to_owned()));
    let f = store.state().report.form.clone().expect("form");
    assert!(!f.copied);
    assert_eq!(f.expected, "fine");
    send(&mut store, ReportAction::Copied(Err("no clipboard".to_owned())));
    let f = store.state().report.form.clone().expect("form");
    assert!(!f.copied);
    assert_eq!(f.error.as_deref(), Some("no clipboard"));
    send(&mut store, ReportAction::Query("x".to_owned()));
    assert_eq!(
        store.state().report.form.as_ref().and_then(|f| f.error.clone()).as_deref(),
        Some("no clipboard")
    );
}

#[test]
fn matches_ignore_case_and_cap_at_fifty() {
    let mut f = form(ReportKind::Snapshot);
    f.query = "BUTTON".to_owned();
    assert_eq!(f.matches(), vec![0]);
    f.elements = (0..80).map(|i| target("Row", &i.to_string())).collect();
    f.query.clear();
    assert_eq!(f.matches(), (0..50).collect::<Vec<_>>());
}

#[test]
fn escape_and_close_close_the_form() {
    let mut store = store(|s| s.report.form = Some(form(ReportKind::Snapshot)));
    store.send(AppAction::Escape, |_| {});
    assert_eq!(store.state().report.form, None);
    send(&mut store, ReportAction::Saved { form: form(ReportKind::Snapshot), open: true });
    send(&mut store, ReportAction::Close);
    assert_eq!(store.state().report.form, None);
}

fn placed(kind: ReportKind) -> ReportForm {
    form(kind).with_places(
        vec![
            Some(Rect { x: 0, y: 0, width: 100, height: 30 }),
            Some(Rect { x: 0, y: 100, width: 100, height: 100 }),
        ],
        (800, 600),
    )
}

fn click(x: i32, y: i32) -> Vec<Point> {
    vec![Point { x, y }]
}

#[test]
fn request_is_kept_until_handled() {
    let mut store = store(|_| {});
    send(&mut store, ReportAction::Request(ReportRequest::Snapshot));
    assert_eq!(store.state().report.request, Some(ReportRequest::Snapshot));
    send(&mut store, ReportAction::Handled);
    assert_eq!(store.state().report.request, None);
}

#[test]
fn open_last_without_a_report_toasts() {
    let mut store = store(|_| {});
    send(&mut store, ReportAction::OpenLast);
    assert_eq!(store.state().report.form, None);
    assert_eq!(store.state().toasts.len(), 1);
}

#[test]
fn open_last_reopens_the_saved_report() {
    let mut store = store(|_| {});
    let saved = form(ReportKind::Snapshot);
    send(&mut store, ReportAction::Saved { form: saved.clone(), open: false });
    assert_eq!(store.state().report.last, Some(saved.clone()));
    send(&mut store, ReportAction::OpenLast);
    assert_eq!(store.state().report.form, Some(saved));
}

#[test]
fn close_keeps_edits_for_open_last() {
    let mut store = store(|s| s.report.form = Some(form(ReportKind::Snapshot)));
    send(&mut store, ReportAction::Expected("fine".to_owned()));
    send(&mut store, ReportAction::Close);
    assert_eq!(store.state().report.form, None);
    send(&mut store, ReportAction::OpenLast);
    assert_eq!(
        store.state().report.form.as_ref().map(|f| f.expected.clone()).as_deref(),
        Some("fine")
    );
}

#[test]
fn strokes_are_drawn_undone_cleared_and_removed() {
    let mut store = store(|s| s.report.form = Some(placed(ReportKind::Snapshot)));
    send(&mut store, ReportAction::Stroke(click(1, 1)));
    assert!(store.state().report.form.as_ref().is_some_and(|f| f.strokes.is_empty()));
    send(&mut store, ReportAction::Draw);
    assert!(store.state().report.form.as_ref().is_some_and(|f| f.drawing));
    send(&mut store, ReportAction::Copied(Ok(())));
    send(&mut store, ReportAction::Stroke(click(10, 10)));
    let f = store.state().report.form.clone().expect("form");
    assert!(!f.copied);
    assert_eq!(f.strokes.len(), 1);
    assert_eq!(f.strokes[0].shape, Shape::Dot);
    assert_eq!(f.strokes[0].covers, vec![target("Button", "Add")]);
    send(&mut store, ReportAction::Stroke(click(10, 150)));
    send(&mut store, ReportAction::Stroke(click(500, 500)));
    assert_eq!(store.state().report.form.as_ref().map(|f| f.strokes.len()), Some(3));
    send(&mut store, ReportAction::UndoStroke);
    assert_eq!(store.state().report.form.as_ref().map(|f| f.strokes.len()), Some(2));
    send(&mut store, ReportAction::RemoveStroke(0));
    let f = store.state().report.form.clone().expect("form");
    assert_eq!(f.strokes.len(), 1);
    assert_eq!(f.strokes[0].covers, vec![target("List", "Items")]);
    send(&mut store, ReportAction::Copied(Ok(())));
    send(&mut store, ReportAction::ClearStrokes);
    let f = store.state().report.form.clone().expect("form");
    assert!(f.strokes.is_empty());
    assert!(!f.copied);
}

#[test]
fn escape_in_draw_mode_keeps_the_form_and_strokes() {
    let mut store = store(|s| s.report.form = Some(placed(ReportKind::Snapshot)));
    send(&mut store, ReportAction::Draw);
    send(&mut store, ReportAction::Stroke(click(10, 10)));
    store.send(AppAction::Escape, |_| {});
    let f = store.state().report.form.clone().expect("form kept");
    assert!(!f.drawing);
    assert_eq!(f.strokes.len(), 1);
    store.send(AppAction::Escape, |_| {});
    assert_eq!(store.state().report.form, None);
}

#[test]
fn opening_the_palette_tells_it_about_the_report() {
    let mut store = store(|s| {
        s.report.recording = true;
        s.report.last = Some(form(ReportKind::Snapshot));
    });
    store.send(AppAction::OpenPalette, |_| {});
    let status = store.state().palette.report;
    assert!(status.recording);
    assert!(status.last);
}

/// Each machine's daemon version the report reads when it starts, and
/// only for the machines the app has no version for yet.
fn reads_when_started(workspace: humata2_clients::Workspace) -> Vec<humata2_clients::model::MachineId> {
    let (clients, fakes) = Clients::fake(workspace.clone(), Arc::new(TestClock::new()));
    let mut initial = AppState::launch(&clients);
    initial.workspace = Arc::new(workspace);
    let mut store = TestStore::new(initial, App::new(clients)).non_exhaustive();
    send(&mut store, ReportAction::Started);
    store.skip_received();
    fakes
        .machines
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            humata2_clients::machines::Call::Detail(machine) => Some(machine),
            _ => None,
        })
        .collect()
}

#[test]
fn starting_a_report_reads_each_machine_once_when_no_version_is_loaded() {
    let mut workspace = fixture::design();
    workspace.machine_health.clear();
    let machines: Vec<_> = workspace.machines.iter().map(|machine| machine.id.clone()).collect();
    assert!(!machines.is_empty());
    assert_eq!(reads_when_started(workspace), machines);
}

#[test]
fn starting_a_report_does_not_read_a_machine_whose_version_is_loaded() {
    let mut workspace = fixture::design();
    workspace.machine_health = workspace
        .machines
        .iter()
        .map(|machine| {
            (
                machine.id.clone(),
                humata2_clients::model::MachineHealth {
                    daemon: "2.0.3".to_owned(),
                    ..Default::default()
                },
            )
        })
        .collect();
    assert!(reads_when_started(workspace).is_empty());
}

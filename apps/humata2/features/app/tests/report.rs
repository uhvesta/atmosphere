//! The bug-report form: recording, saving, editing and closing.
use std::path::PathBuf;
use std::sync::Arc;

use humata2_app::{
    App, AppAction, AppState, PinTarget, ReportAction, ReportForm, ReportKind, ToastAction,
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

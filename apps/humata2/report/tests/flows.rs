//! The whole path a report takes, for both kinds: taken (a recording from
//! start to stop, or a snapshot), saved at once under the title it was
//! given, the form filled in after, and copied for an agent.

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use humata2_report::{
    Captured, Comment, FlowAction, Form, FormError, Gesture, Kind, Recorder, Secrets, copy_prompt,
    fill_form, read_events, scrub, snapshot, write,
};
use support::{Toy, assert_no_piece_of, context, draft_field, files, item};

const TOKEN: &str = "ghp_Zx81Qm2Lp0Vb7Rt4";

fn secrets() -> Secrets {
    Secrets::new().secret("GITHUB_TOKEN", TOKEN)
}

/// Records typing "milk" into the toy through a [`Recorder`], as the menu,
/// the shortcut or `humata report record` would start it.
fn recorded() -> Captured {
    let mut toy = Toy::default();
    let mut recorder = Recorder::default();
    recorder.start(context(), toy.trees()).expect("starts");
    let gesture = Gesture::Type {
        text: "milk".to_owned(),
        target: Some(draft_field()),
    };
    toy.play(&FlowAction::Type("milk".to_owned()));
    if let Some(recording) = recorder.recording_mut() {
        recording.interaction(100, gesture);
        recording.action(101, "DraftChanged(\"milk\")", &toy.state());
        recording.view(102, &toy.view());
        recording.layout(103, &toy.layout());
    }
    recorder.stop(200).expect("stops")
}

/// Saves `captured` the moment it stops, with only its title as what
/// happened, as `humata report stop --title` does.
fn saved_with_title(reports: &Path, captured: Captured, title: &str) -> PathBuf {
    let form = Form {
        happened: title.to_owned(),
        ..Form::default()
    };
    let scrubbed = scrub(captured, form, &secrets()).expect("scrubs");
    let dir = write(reports, &scrubbed).expect("writes");
    fs::canonicalize(dir).expect("the folder exists")
}

fn the_form() -> Form {
    Form {
        happened: format!("the draft kept milk, and the log showed {TOKEN}"),
        expected: "an empty draft".to_owned(),
        comments: vec![Comment {
            element: item("milk"),
            text: "should be gone".to_owned(),
            place: None,
        }],
        ..Form::default()
    }
}

fn takes_the_whole_path(captured: Captured, kind: Kind) {
    let reports = tempfile::tempdir().expect("a temporary folder");
    let dir = saved_with_title(reports.path(), captured, "the draft kept milk");

    fill_form(&dir, the_form(), &secrets()).expect("fills in the form");
    let prompt = copy_prompt(&dir).expect("makes the prompt");

    let folders: Vec<PathBuf> = fs::read_dir(reports.path())
        .expect("the reports folder can be read")
        .map(|entry| entry.expect("a folder entry").path())
        .collect();
    assert_eq!(folders.len(), 1, "the form goes into the saved folder: {folders:?}");
    assert!(prompt.contains("an empty draft"), "{prompt}");
    let summary = fs::read_to_string(dir.join("report.md")).expect("report.md");
    assert!(summary.contains("should be gone"), "{summary}");
    let listed = files(&dir);
    assert_eq!(listed.len(), 6);
    for (name, _) in &listed {
        assert!(prompt.contains(&dir.join(name).display().to_string()), "{name}:\n{prompt}");
    }
    assert_no_piece_of(&dir, TOKEN, 8);
    assert!(!prompt.contains(TOKEN));
    let events = fs::read_to_string(dir.join("events.jsonl")).expect("events.jsonl");
    assert_eq!(read_events(&events).expect("reads back").kind(), kind);
}

#[test]
fn a_recording_is_saved_at_stop_then_given_its_form_and_copied() {
    takes_the_whole_path(recorded(), Kind::Recording);
}

#[test]
fn a_snapshot_is_saved_then_given_its_form_and_copied() {
    let mut toy = Toy::default();
    toy.play(&FlowAction::Type("milk".to_owned()));

    takes_the_whole_path(snapshot(context(), toy.trees()), Kind::Snapshot);
}

#[test]
fn a_report_saved_with_its_title_alone_already_says_what_happened() {
    let reports = tempfile::tempdir().expect("a temporary folder");
    let dir = saved_with_title(reports.path(), recorded(), "the draft kept milk");

    let prompt = copy_prompt(&dir).expect("makes the prompt");

    assert!(prompt.contains("the draft kept milk"), "{prompt}");
}

#[test]
fn filling_in_a_form_twice_keeps_only_the_last() {
    let reports = tempfile::tempdir().expect("a temporary folder");
    let dir = saved_with_title(reports.path(), recorded(), "the draft kept milk");
    fill_form(&dir, the_form(), &secrets()).expect("fills in the form");

    let second = Form {
        expected: "the draft cleared after Add".to_owned(),
        ..the_form()
    };
    fill_form(&dir, second, &secrets()).expect("fills it in again");

    let prompt = copy_prompt(&dir).expect("makes the prompt");
    assert!(prompt.contains("the draft cleared after Add"), "{prompt}");
    assert!(!prompt.contains("an empty draft"), "{prompt}");
    assert_eq!(files(&dir).len(), 6, "no partial file is left behind");
}

#[test]
fn a_form_for_a_folder_without_events_is_refused() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    fs::write(folder.path().join("report.md"), "# a report\n").expect("writes");

    let filled = fill_form(folder.path(), the_form(), &secrets());

    assert!(matches!(filled, Err(FormError::ReadEvents { .. })), "{filled:?}");
    assert_eq!(
        fs::read_to_string(folder.path().join("report.md")).ok().as_deref(),
        Some("# a report\n"),
        "the old report.md is kept"
    );
}

#[test]
fn a_form_for_a_folder_with_broken_events_is_refused() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    fs::write(folder.path().join("events.jsonl"), "not json\n").expect("writes");

    let filled = fill_form(folder.path(), the_form(), &secrets());

    assert!(matches!(filled, Err(FormError::Events { .. })), "{filled:?}");
}

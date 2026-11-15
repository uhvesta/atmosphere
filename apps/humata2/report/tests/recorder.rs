//! One recording runs at a time, whichever of the menu, the shortcut or
//! `humata report record|stop` starts and stops it.

mod support;

use humata2_report::{Gesture, Kind, Recorder, RecorderError};
use support::{STARTED, Toy, context, draft_field};

#[test]
fn a_started_recording_runs_until_it_is_stopped() {
    let mut recorder = Recorder::default();
    assert!(!recorder.is_recording());

    recorder.start(context(), Toy::default().trees()).expect("starts");
    assert!(recorder.is_recording());

    let captured = recorder.stop(100).expect("stops");
    assert!(!recorder.is_recording());
    assert_eq!(captured.kind(), Kind::Recording);
}

#[test]
fn starting_while_recording_is_refused_and_the_first_keeps_running() {
    let mut recorder = Recorder::default();
    recorder.start(context(), Toy::default().trees()).expect("starts");
    if let Some(recording) = recorder.recording_mut() {
        recording.interaction(
            10,
            Gesture::Type {
                text: "milk".to_owned(),
                target: Some(draft_field()),
            },
        );
    }

    let mut later = context();
    later.started_unix_ms = STARTED + 5_000;
    let again = recorder.start(later, Toy::default().trees());

    assert!(matches!(
        again,
        Err(RecorderError::AlreadyRecording { started_unix_ms: STARTED })
    ));
    assert!(recorder.is_recording());
    let captured = recorder.stop(100).expect("stops");
    assert_eq!(captured.events().len(), 1, "what the first recorded is kept");
}

#[test]
fn stopping_when_nothing_is_recording_is_an_error() {
    let mut recorder = Recorder::default();

    assert!(matches!(recorder.stop(0), Err(RecorderError::NotRecording)));
    assert!(recorder.recording_mut().is_none());
}

#[test]
fn a_stopped_recorder_can_record_again() {
    let mut recorder = Recorder::default();
    recorder.start(context(), Toy::default().trees()).expect("starts");
    recorder.stop(100).expect("stops");

    assert!(recorder.start(context(), Toy::default().trees()).is_ok());
    assert!(recorder.is_recording());
}

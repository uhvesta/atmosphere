//! A saved recording turns into a flow that, played on a fresh app wired
//! to fake services made from the recording, ends where the recording
//! ended; what a flow can't play is listed with why.

mod support;

use std::convert::Infallible;
use std::fs;

use humata2_report::{
    Button, Captured, Compare, Element, FakeDaemon, FakeDaemonError, FlowAction, Form, Gesture,
    Holds, Pasted, Player, ReplayError, Rpc, Secrets, SkipReason, Trees, read_events, scrub,
    snapshot, write,
};
use support::{Driver, Toy, add_button, context, draft_field, item};

const EVERYTHING: Compare = Compare {
    view: true,
    layout: true,
};

const ADD: &str = "humata.v1.ItemService/Add";

/// The toy on its own, with no services.
struct ToyPlayer(Toy);

impl Player for ToyPlayer {
    type Error = Infallible;

    fn play(&mut self, action: &FlowAction) -> Result<(), Infallible> {
        self.0.play(action);
        Ok(())
    }

    fn trees(&mut self) -> Result<Trees, Infallible> {
        Ok(self.0.trees())
    }
}

/// The toy as an app whose Add asks the daemon first, and adds only when
/// the daemon says ok.
struct ServicePlayer {
    toy: Toy,
    daemon: FakeDaemon,
}

impl Player for ServicePlayer {
    type Error = FakeDaemonError;

    fn play(&mut self, action: &FlowAction) -> Result<(), FakeDaemonError> {
        let adds = matches!(action, FlowAction::Click(target) if target.label == "Add");
        if adds && self.daemon.answer("studio", ADD)?.status != "ok" {
            return Ok(());
        }
        self.toy.play(action);
        Ok(())
    }

    fn trees(&mut self) -> Result<Trees, FakeDaemonError> {
        Ok(self.toy.trees())
    }
}

fn add_call(at_ms: u64, status: &str) -> Rpc {
    Rpc {
        at_ms,
        machine: "studio".to_owned(),
        method: ADD.to_owned(),
        status: status.to_owned(),
        duration_ms: 4,
        request: Some("{\"item\":\"milk\"}".to_owned()),
        response: None,
        holds: Holds::Plain,
    }
}

/// Saves `captured` and reads it back from `events.jsonl`.
fn saved(captured: Captured) -> Captured {
    let scrubbed = scrub(captured, Form::default(), &Secrets::new()).expect("scrubs");
    let reports = tempfile::tempdir().expect("a temporary folder");
    let dir = write(reports.path(), &scrubbed).expect("writes");
    let text = fs::read_to_string(dir.join("events.jsonl")).expect("events.jsonl");
    read_events(&text).expect("reads back")
}

/// Adds milk, which the daemon takes, then tries to add eggs, which it
/// refuses, so eggs stays in the field.
fn refused_add() -> Captured {
    let mut driver = Driver::start();
    driver.type_into(draft_field(), "milk");
    driver.click(add_button());
    driver.recording.rpc(add_call(driver.now + 5, "ok"));
    driver.type_into(draft_field(), "eggs");
    driver.now += 100;
    let at = driver.now;
    driver.recording.interaction(
        at,
        Gesture::Click {
            target: add_button(),
            button: Button::Left,
        },
    );
    driver.recording.rpc(add_call(at + 5, "unavailable"));
    saved(driver.stop())
}

/// Records a session with every kind of interaction, saves it, and reads
/// it back from `events.jsonl`.
fn saved_session() -> Captured {
    let mut driver = Driver::start();
    driver.type_into(draft_field(), "milk");
    driver.key(draft_field(), "enter");
    driver.type_into(draft_field(), "egg");
    driver.type_into(draft_field(), "s");
    driver.key(draft_field(), "escape");
    driver.click(add_button());
    driver.act(Gesture::Scroll {
        target: item("milk").inside("ScrollView", "Items"),
        dx: 0,
        dy: 120,
    });
    driver.act(Gesture::Click {
        target: item("milk"),
        button: Button::Right,
    });
    driver.act(Gesture::Paste {
        target: Some(draft_field()),
        content: Pasted::Image {
            path: "/home/me/.humata/paste/s-1/1.png".to_owned(),
            bytes: 4096,
        },
    });
    driver.act(Gesture::Click {
        target: Element {
            role: "Unknown".to_owned(),
            label: None,
            within: Vec::new(),
        },
        button: Button::Left,
    });
    driver.act(Gesture::Paste {
        target: Some(draft_field()),
        content: Pasted::Text("bread".to_owned()),
    });
    driver.key(draft_field(), "enter");
    driver.click(item("milk"));
    saved(driver.stop())
}

#[test]
fn a_replayed_recording_ends_where_it_ended() {
    let script = saved_session().flow_script();

    let replayed = script.replay(&mut ToyPlayer(Toy::default()), EVERYTHING);

    assert!(replayed.is_ok(), "{replayed:?}");
}

#[test]
fn a_replay_against_the_recordings_fake_daemon_reproduces_what_it_answered() {
    let captured = refused_add();
    let mut player = ServicePlayer {
        toy: Toy::default(),
        daemon: captured.fake_daemon(),
    };

    let replayed = captured.flow_script().replay(&mut player, EVERYTHING);

    assert!(replayed.is_ok(), "{replayed:?}");
    assert!(player.daemon.unanswered().is_empty());
    assert!(captured.end().state.contains("eggs"), "the refused item stayed in the field");
}

#[test]
fn an_app_that_no_longer_does_what_it_did_ends_somewhere_else() {
    let captured = refused_add();

    // This app adds without asking the daemon, so eggs is added too.
    let replayed = captured
        .flow_script()
        .replay(&mut ToyPlayer(Toy::default()), Compare::default());

    assert!(matches!(replayed, Err(ReplayError::EndDiffers { tree: "state", .. })), "{replayed:?}");
}

#[test]
fn a_call_the_recording_never_made_fails_the_step_that_made_it() {
    // The session has Add clicks but no recorded calls.
    let captured = saved_session();
    let mut player = ServicePlayer {
        toy: Toy::default(),
        daemon: captured.fake_daemon(),
    };

    let replayed = captured.flow_script().replay(&mut player, Compare::default());

    assert!(
        matches!(
            replayed,
            Err(ReplayError::Play {
                source: FakeDaemonError::Unrecorded { recorded: 0, .. },
                ..
            })
        ),
        "{replayed:?}"
    );
}

#[test]
fn a_fake_daemon_answers_each_call_once_in_recorded_order() {
    let mut daemon = refused_add().fake_daemon();

    let first = daemon.answer("studio", ADD).map(|call| call.status.clone());
    let second = daemon.answer("studio", ADD).map(|call| call.status.clone());
    let third = daemon.answer("studio", ADD);

    assert_eq!(first.ok().as_deref(), Some("ok"));
    assert_eq!(second.ok().as_deref(), Some("unavailable"));
    assert!(matches!(third, Err(FakeDaemonError::Unrecorded { recorded: 2, .. })));
    assert!(matches!(
        daemon.answer("laptop", ADD),
        Err(FakeDaemonError::Unrecorded { recorded: 0, .. })
    ));
}

#[test]
fn a_fixture_that_starts_elsewhere_is_refused_before_any_step() {
    let script = saved_session().flow_script();
    let mut elsewhere = Toy::default();
    elsewhere.play(&FlowAction::Type("tea".to_owned()));

    let replayed = script.replay(&mut ToyPlayer(elsewhere), Compare::default());

    assert!(matches!(replayed, Err(ReplayError::StartDiffers { tree: "state", .. })), "{replayed:?}");
}

#[test]
fn what_a_flow_cant_play_is_skipped_with_why() {
    let script = saved_session().flow_script();

    let reasons: Vec<SkipReason> = script.skipped.iter().map(|skipped| skipped.reason).collect();

    assert_eq!(
        reasons,
        vec![
            SkipReason::Scroll,
            SkipReason::NotLeftButton,
            SkipReason::PastedFile,
            SkipReason::NoLabel,
        ]
    );
}

#[test]
fn a_step_is_snapshot_only_when_the_view_changed_after_it() {
    let script = saved_session().flow_script();

    let snapshot_after = |action: &FlowAction| {
        script
            .steps
            .iter()
            .find(|step| &step.action == action)
            .map(|step| step.snapshot)
    };

    assert_eq!(snapshot_after(&FlowAction::Key("escape".to_owned())), Some(false));
    assert_eq!(snapshot_after(&FlowAction::Type("milk".to_owned())), Some(true));
}

#[test]
fn a_snapshot_has_no_steps() {
    let captured = snapshot(context(), Toy::default().trees());

    let script = captured.flow_script();

    assert!(script.steps.is_empty());
    assert_eq!(script.start, script.end);
}

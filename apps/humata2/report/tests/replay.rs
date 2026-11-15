//! A saved recording turns into a flow that, played on a fresh app, ends
//! where the recording ended; what a flow can't play is listed with why.

mod support;

use std::fs;

use humata2_report::{
    Button, Element, FlowAction, Form, Gesture, Pasted, Secrets, SkipReason, read_events, scrub,
    snapshot, write,
};
use support::{Driver, Toy, add_button, context, draft_field, item};

/// Records a session, saves it, and reads it back from `events.jsonl`.
fn saved_session() -> humata2_report::Captured {
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

    let scrubbed = scrub(driver.stop(), Form::default(), &Secrets::new()).expect("scrubs");
    let reports = tempfile::tempdir().expect("a temporary folder");
    let dir = write(reports.path(), &scrubbed).expect("writes");
    let text = fs::read_to_string(dir.join("events.jsonl")).expect("events.jsonl");
    read_events(&text).expect("reads back")
}

#[test]
fn a_replayed_recording_ends_where_it_ended() {
    let script = saved_session().flow_script();
    let mut toy = Toy::default();
    assert_eq!(toy.trees(), script.start);

    for step in &script.steps {
        toy.play(&step.action);
    }

    assert_eq!(toy.trees(), script.end);
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

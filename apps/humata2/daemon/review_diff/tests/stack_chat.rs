//! The review chat with a stack (D800): a plan carries the stack, the agent
//! moves the user between layers, Undo puts the layer and the stack back,
//! and a re-sent stack keeps the user's layer when it still exists. All of
//! it survives the wire, as the review is saved (PR-26).

use humata2_review_diff::{
    AgentSwitches, ChangeKind, ChatError, DiffSpec, FileRef, Layer, LayerRepo, Note, NoteTarget,
    Part, ReviewChat, ReviewPlan, Stack, ViewState, wire,
};

fn range(repo: &str, base: &str, head: &str) -> LayerRepo {
    LayerRepo {
        repo: repo.to_owned(),
        part: Part::Range {
            base: base.to_owned(),
            head: head.to_owned(),
        },
        pull_request: None,
    }
}

fn layer(title: &str, repos: Vec<LayerRepo>) -> Layer {
    Layer {
        title: title.to_owned(),
        purpose: None,
        repos,
        ..Layer::default()
    }
}

fn stack(titles: &[&str]) -> Stack {
    Stack::new(
        titles
            .iter()
            .map(|title| layer(title, vec![range("acme/api", "main", title)]))
            .collect(),
    )
}

fn plan(stack: Stack) -> ReviewPlan {
    ReviewPlan {
        spec: DiffSpec::default(),
        stack: Some(stack),
        ..ReviewPlan::default()
    }
}

fn file(path: &str) -> FileRef {
    FileRef {
        repo: "acme/api".to_owned(),
        path: path.to_owned(),
    }
}

/// A chat whose agent prepared the three-layer stack.
fn prepared() -> ReviewChat {
    let mut chat = ReviewChat::default();
    chat.report(ViewState::default());
    chat.prepare("Claude Code", plan(stack(&["codec", "journal", "wip"])))
        .expect("the plan applies");
    chat
}

#[test]
fn a_plan_with_a_stack_opens_at_whole_stack() {
    let chat = prepared();
    assert_eq!(chat.view().stack, Some(stack(&["codec", "journal", "wip"])));
    assert_eq!(chat.view().layer, None);
}

#[test]
fn the_agent_moves_the_user_to_a_layer_by_title_and_undo_moves_back() {
    let mut chat = prepared();
    let change = chat
        .focus_layer(Some(" Journal ".to_owned()))
        .expect("the layer exists");
    assert_eq!(chat.view().layer.as_deref(), Some("journal"));
    assert_eq!(change.what, ChangeKind::Layer(Some("journal".to_owned())));
    chat.undo(&change.undo_id).expect("undo the move");
    assert_eq!(chat.view().layer, None);
}

#[test]
fn moving_to_whole_stack_is_a_layer_of_none() {
    let mut chat = prepared();
    chat.focus_layer(Some("wip".to_owned())).expect("to wip");
    chat.focus_layer(None).expect("to Whole stack");
    assert_eq!(chat.view().layer, None);
}

#[test]
fn an_unknown_layer_is_refused_with_the_titles_there_are() {
    let mut chat = prepared();
    let error = chat
        .focus_layer(Some("nope".to_owned()))
        .expect_err("no such layer");
    assert!(
        matches!(&error, ChatError::NoSuchLayer { known, .. } if known.len() == 3),
        "{error:?}"
    );
}

#[test]
fn a_layer_without_a_stack_is_refused() {
    let mut chat = ReviewChat::default();
    chat.report(ViewState::default());
    let error = chat
        .focus_layer(Some("codec".to_owned()))
        .expect_err("there is no stack");
    assert!(matches!(error, ChatError::NoSuchLayer { .. }), "{error:?}");
}

#[test]
fn moving_layers_needs_the_focus_switch() {
    let mut chat = prepared();
    chat.set_switches(AgentSwitches {
        focus: false,
        ..AgentSwitches::default()
    });
    let error = chat
        .focus_layer(Some("codec".to_owned()))
        .expect_err("focus is off");
    assert!(matches!(error, ChatError::SwitchOff { .. }), "{error:?}");
}

#[test]
fn a_resent_stack_keeps_the_layer_showing_if_its_title_is_still_there() {
    let mut chat = prepared();
    chat.focus_layer(Some("journal".to_owned())).expect("to journal");
    chat.prepare("Claude Code", plan(stack(&["codec", "Journal", "tests"])))
        .expect("the re-sent plan applies");
    assert_eq!(chat.view().layer.as_deref(), Some("Journal"));
    chat.prepare("Claude Code", plan(stack(&["codec", "tests"])))
        .expect("a plan without journal");
    assert_eq!(chat.view().layer, None);
}

#[test]
fn undoing_a_plan_puts_back_the_stack_and_layer_before_it() {
    let mut chat = prepared();
    chat.focus_layer(Some("journal".to_owned())).expect("to journal");
    let change = chat
        .prepare("Claude Code", plan(stack(&["other"])))
        .expect("another stack");
    chat.undo(&change.undo_id).expect("undo the plan");
    assert_eq!(chat.view().stack, Some(stack(&["codec", "journal", "wip"])));
    assert_eq!(chat.view().layer.as_deref(), Some("journal"));
}

#[test]
fn the_stack_and_layer_showing_survive_the_wire() {
    let mut chat = prepared();
    chat.focus_layer(Some("journal".to_owned())).expect("to journal");
    let view = chat.view().clone();
    let back = wire::view_from_pb(&wire::view_to_pb(&view, None)).expect("the view reads back");
    assert_eq!(back, view);
}

#[test]
fn a_plans_stack_and_a_notes_layer_survive_the_wire() {
    let mut sent = plan(stack(&["codec", "journal"]));
    sent.notes = vec![Note {
        file: file("src/codec.rs"),
        target: NoteTarget::File,
        text: "Start here".to_owned(),
        anchor: None,
        layer: Some("codec".to_owned()),
    }];
    let back = wire::stored_plan_from_pb(&wire::plan_to_pb(&sent, &[false], None))
        .expect("the plan reads back");
    assert_eq!(back.stack, sent.stack);
    assert_eq!(back.notes[0].layer.as_deref(), Some("codec"));
}

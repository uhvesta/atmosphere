//! D857: the opening prompt names what the Diff panel shows.
use std::sync::Arc;

use humata2_clients::fixture;
use humata2_clients::model::Session;
use humata2_clients::views::{LayerView, PlanView, StackView};
use humata2_clients::Workspace;

use humata2_review::ReviewState;
use humata2_review::opening_prompt::opening_prompt;

fn session(title: &str, review: bool) -> Session {
    let mut workspace: Workspace = fixture::design();
    let mut found = workspace.sessions.remove(0);
    found.title = title.to_owned();
    found.pull_request_review = review;
    found
}

fn with_stack(session: &Session, shown: Option<&str>) -> ReviewState {
    let layer = |title: &str| LayerView {
        title: title.to_owned(),
        ..LayerView::default()
    };
    let mut state = ReviewState {
        workspace: Arc::new(Workspace::default()),
        session: Some(session.id.clone()),
        ..ReviewState::default()
    };
    state.plans.insert(
        session.id.clone(),
        PlanView {
            stack: Some(StackView {
                layers: vec![layer("schema"), layer("client"), layer("screen")],
                whole: Vec::new(),
            }),
            ..PlanView::default()
        },
    );
    state
        .layer
        .insert(session.id.clone(), shown.map(str::to_owned));
    state
}

#[test]
fn a_shown_layer_is_named_by_its_place_from_the_bottom() {
    let each = session("acme/api", false);
    let state = with_stack(&each, Some("client"));
    let prompt = opening_prompt(&state, &each);
    assert!(prompt.starts_with("Look at layer 2 of the stack in the Diff panel."));
    assert!(prompt.ends_with("Don't edit files. Leave what you find on the lines."));
}

#[test]
fn whole_stack_names_the_stack() {
    let each = session("acme/api", false);
    let state = with_stack(&each, None);
    assert!(opening_prompt(&state, &each)
        .starts_with("Look at the whole stack in the Diff panel."));
}

#[test]
fn a_pull_request_review_with_one_number_names_it() {
    let each = session("acme/api #482", true);
    let state = ReviewState::default();
    assert!(opening_prompt(&state, &each)
        .starts_with("Look at pull request #482 in the Diff panel."));
}

#[test]
fn a_review_without_exactly_one_number_and_a_plain_diff_say_the_changes() {
    let state = ReviewState::default();
    for each in [
        session("acme/api", true),
        session("acme/api #1 and acme/web #2", true),
        session("acme/api #482", false),
    ] {
        assert!(opening_prompt(&state, &each)
            .starts_with("Look at the changes in the Diff panel."));
    }
}

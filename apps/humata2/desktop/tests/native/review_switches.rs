//! PR-8 and PR-25: a pull request review's Info panel carries the agent's
//! switches; any other session's does not.
use super::*;

fn info_of(review: bool) -> App {
    let mut workspace = fixture::design();
    for s in &mut workspace.sessions {
        if s.id.as_str() == "ledger" {
            s.pull_request_review = review;
        }
    }
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Info);
    app
}

#[test]
fn a_review_shows_its_switches_and_another_session_does_not() {
    let mut review = info_of(true);
    for id in [
        "session-info-push",
        "session-info-agent-focus",
        "session-info-agent-notes",
        "session-info-agent-order",
    ] {
        assert!(review.contains(id), "a review shows {id}");
    }
    let mut plain = info_of(false);
    assert!(!plain.contains("session-info-push"));
    assert!(!plain.contains("session-info-agent-notes"));
}

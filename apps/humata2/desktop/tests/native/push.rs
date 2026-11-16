//! RV-2, flow 26.3 step 8: each repo group in the Diff panel has its own
//! Push & PR, which pushes that repo only and then names its pull request.
use super::*;

#[test]
fn each_repo_group_pushes_its_own_branch_and_names_its_pull_request() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Changes);
    assert!(app.contains("changes-push-api"));
    assert!(app.contains("changes-push-ledger"));
    assert!(!app.contains("changes-pushed-api"));
    app.click("changes-push-api");
    let pushes = app.fakes.daemon.pushes();
    assert_eq!(pushes.len(), 1);
    assert_eq!(pushes[0].1.repo, "api");
    assert!(app.contains("changes-pushed-api"), "the api group names its pull request");
    assert!(!app.contains("changes-pushed-ledger"), "the ledger group is left as it was");
    app.snapshot("changes-pushed");
}

#[test]
fn a_failed_push_shows_on_its_group_with_try_again() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Changes);
    app.fakes
        .daemon
        .fail_next(humata2_clients::Operation::Push, "gh is signed out");
    app.click("changes-push-api");
    assert!(app.contains("changes-push-failed-api"));
    app.click("changes-push-api");
    assert!(app.contains("changes-pushed-api"));
    assert!(!app.contains("changes-push-failed-api"));
}

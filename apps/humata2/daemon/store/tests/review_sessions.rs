//! Review sessions' pull requests (PR-4..PR-8, D283): kept per session in
//! the order they were asked for, and gone with the session.

use std::path::PathBuf;

use humata2_store::{ReviewPr, ReviewSessionRecord, SessionRecord, Store, Take};

fn session(id: &str) -> SessionRecord {
    SessionRecord {
        id: id.to_owned(),
        channel: "ch_reviews".to_owned(),
        machine: "ma_1".to_owned(),
        title: "Review acme/api#482".to_owned(),
        agent: "claude".to_owned(),
        folder: PathBuf::from("/h/pr-reviews/sessions/se_1"),
        created_unix_ms: 1,
    }
}

fn pr(owner: &str, name: &str, number: u32) -> ReviewPr {
    ReviewPr {
        owner: owner.to_owned(),
        name: name.to_owned(),
        number,
        base_branch: "main".to_owned(),
        head_branch: "fix-rounding".to_owned(),
        head_sha: "a".repeat(40),
        checked_sha: "a".repeat(40),
        state: 1,
        submit_started_unix_ms: None,
    }
}

fn review(id: &str) -> ReviewSessionRecord {
    ReviewSessionRecord {
        session: id.to_owned(),
        push_allowed: false,
        prs: vec![pr("acme", "ledger", 91), pr("acme", "api", 482)],
    }
}

#[test]
fn a_binding_survives_reopening_the_store_with_its_prs_in_order() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = Store::open(dir.path()).unwrap();
        store.put_session(&session("se_1")).unwrap();
        store.put_review_session(&review("se_1")).unwrap();
    }
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.review_session("se_1").unwrap(), Some(review("se_1")));
    assert_eq!(store.review_sessions().unwrap(), vec![review("se_1")]);
}

#[test]
fn putting_again_replaces_the_binding_and_its_prs() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&session("se_1")).unwrap();
    store.put_review_session(&review("se_1")).unwrap();
    let updated = ReviewSessionRecord {
        push_allowed: true,
        prs: vec![ReviewPr {
            checked_sha: "b".repeat(40),
            submit_started_unix_ms: Some(77),
            ..pr("acme", "api", 482)
        }],
        ..review("se_1")
    };
    store.put_review_session(&updated).unwrap();
    assert_eq!(store.review_session("se_1").unwrap(), Some(updated));
}

#[test]
fn letting_the_agent_push_changes_only_that() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&session("se_1")).unwrap();
    store.put_review_session(&review("se_1")).unwrap();
    assert!(store.set_review_push_allowed("se_1", true).unwrap());
    let kept = store.review_session("se_1").unwrap().unwrap();
    assert!(kept.push_allowed);
    assert_eq!(kept.prs, review("se_1").prs);
}

#[test]
fn a_session_that_is_no_review_has_no_binding_and_cant_be_let_push() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&session("se_2")).unwrap();
    assert_eq!(store.review_session("se_2").unwrap(), None);
    assert!(!store.set_review_push_allowed("se_2", true).unwrap());
    assert_eq!(store.review_session("se_2").unwrap(), None);
}

#[test]
fn removing_the_session_removes_its_binding() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&session("se_1")).unwrap();
    store.put_review_session(&review("se_1")).unwrap();
    store.remove_session("se_1").unwrap();
    assert_eq!(store.review_session("se_1").unwrap(), None);
    assert!(store.review_sessions().unwrap().is_empty());
}

#[test]
fn a_binding_needs_its_session() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    assert!(store.put_review_session(&review("se_9")).is_err());
}

/// PR-21: a review session moved to a machine that never had it arrives as a
/// review of the same pull requests, in the same order; the host, which keeps
/// its own binding, is not touched by a move back onto it.
#[test]
fn a_moved_review_session_brings_its_binding_and_the_host_keeps_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let host = Store::open(dir.path().join("host")).unwrap();
    host.put_session(&session("se_1")).unwrap();
    host.put_review_session(&review("se_1")).unwrap();
    let side = dir.path().join("records.sqlite");
    host.export_session("se_1", &side).unwrap();

    let remote = Store::open(dir.path().join("box")).unwrap();
    remote.import_moved_session("se_1", &side, Take::Everything, &[]).unwrap();
    assert_eq!(remote.review_session("se_1").unwrap(), Some(review("se_1")));

    // The binding moved on while the review ran on the box; coming home
    // changes nothing of what the host holds.
    let mut newer = review("se_1");
    newer.prs[0].head_sha = "b".repeat(40);
    host.put_review_session(&newer).unwrap();
    let back = dir.path().join("back.sqlite");
    remote.export_session("se_1", &back).unwrap();
    host.import_moved_session("se_1", &back, Take::AgentOnly, &[]).unwrap();
    assert_eq!(host.review_session("se_1").unwrap(), Some(newer));
}

#[test]
fn a_session_that_is_no_review_carries_no_binding() {
    let dir = tempfile::tempdir().unwrap();
    let old = Store::open(dir.path().join("old")).unwrap();
    old.put_session(&session("se_1")).unwrap();
    let side = dir.path().join("records.sqlite");
    old.export_session("se_1", &side).unwrap();

    let new = Store::open(dir.path().join("new")).unwrap();
    new.import_moved_session("se_1", &side, Take::Everything, &[]).unwrap();
    assert_eq!(new.review_session("se_1").unwrap(), None);
}

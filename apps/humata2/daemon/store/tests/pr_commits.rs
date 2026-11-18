//! A pull request's reviewed commits (D299): a mark per sha, replaced when
//! the sha is marked again, and a history numbered in the order it was
//! added to, both kept per pull request across a reopen.

use humata2_store::{CommitMark, KeptAnswer, MarkedBy, ReviewEvent, Store};

fn by() -> MarkedBy {
    MarkedBy {
        actor: vec![10, 0],
        gh_login: "uhvesta".to_owned(),
    }
}

fn mark(sha: &str, reviewed: bool, at: i64) -> CommitMark {
    CommitMark {
        sha: sha.to_owned(),
        patch_id: format!("p{sha}"),
        reviewed,
        reviewed_as_sha: String::new(),
        reviewed_unix_ms: at,
        reviewed_head_sha: "h".to_owned(),
        by: Some(by()),
    }
}

fn event(kind: i32, commits: &[&str], at: i64, snapshot_n: Option<u32>) -> ReviewEvent {
    ReviewEvent {
        at_unix_ms: at,
        kind,
        commits: commits.iter().map(|sha| (*sha).to_owned()).collect(),
        head_sha: "h".to_owned(),
        snapshot_n,
        by: Some(by()),
    }
}

#[test]
fn a_pull_request_without_marks_has_none() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let saved = store.pr_commit_marks("acme", "api", 482).unwrap();
    assert!(saved.marks.is_empty() && saved.history.is_empty());
}

#[test]
fn marks_replace_the_same_sha_and_history_only_grows() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = Store::open(dir.path()).unwrap();
        let marked = [mark("a", true, 1), mark("b", true, 1)];
        store
            .add_pr_commit_marks("acme", "api", 482, &marked, &event(1, &["a", "b"], 1, None))
            .unwrap();
        let unmarked = [mark("b", false, 2)];
        store
            .add_pr_commit_marks("acme", "api", 482, &unmarked, &event(2, &["b"], 2, Some(3)))
            .unwrap();
    }
    let store = Store::open(dir.path()).unwrap();
    let saved = store.pr_commit_marks("acme", "api", 482).unwrap();
    assert_eq!(saved.marks, vec![mark("a", true, 1), mark("b", false, 2)]);
    let seqs: Vec<u32> = saved.history.iter().map(|entry| entry.seq).collect();
    assert_eq!(seqs, [1, 2]);
    assert_eq!(saved.history[0].event, event(1, &["a", "b"], 1, None));
    assert_eq!(saved.history[1].event, event(2, &["b"], 2, Some(3)));
}

#[test]
fn an_entry_nobody_made_keeps_no_reviewer() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let carried = CommitMark {
        reviewed_as_sha: "a".to_owned(),
        by: None,
        ..mark("c", true, 1)
    };
    let carry = ReviewEvent {
        by: None,
        ..event(3, &["c"], 5, None)
    };
    let saved = store
        .add_pr_commit_marks("acme", "api", 482, &[carried.clone()], &carry)
        .unwrap();
    assert_eq!(saved.marks, vec![carried]);
    assert_eq!(saved.history[0].event, carry);
}

#[test]
fn marks_belong_to_one_pull_request() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store
        .add_pr_commit_marks(
            "acme",
            "api",
            482,
            &[mark("a", true, 1)],
            &event(1, &["a"], 1, None),
        )
        .unwrap();
    store
        .add_pr_commit_marks(
            "acme",
            "api",
            7,
            &[mark("a", true, 1)],
            &event(1, &["a"], 1, None),
        )
        .unwrap();
    let other = store.pr_commit_marks("acme", "ledger", 482).unwrap();
    assert!(other.marks.is_empty() && other.history.is_empty());
    let seven = store.pr_commit_marks("acme", "api", 7).unwrap();
    assert_eq!(
        seven
            .history
            .iter()
            .map(|entry| entry.seq)
            .collect::<Vec<_>>(),
        [1]
    );
}

#[test]
fn a_keyed_entry_keeps_its_answer_across_a_reopen() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = Store::open(dir.path()).unwrap();
        store
            .add_keyed_pr_commit_marks(
                "acme",
                "api",
                482,
                &[mark("a", true, 1)],
                &event(1, &["a"], 1, None),
                "k1",
            )
            .unwrap();
        let made = store.pr_marks_answer("acme", "api", 482, "k1").unwrap();
        assert_eq!(
            made,
            Some(KeptAnswer::Lost),
            "made, its answer not kept yet"
        );
        store
            .keep_pr_marks_answer("acme", "api", 482, "k1", b"first")
            .unwrap();
    }
    let store = Store::open(dir.path()).unwrap();
    let kept = store.pr_marks_answer("acme", "api", 482, "k1").unwrap();
    assert_eq!(kept, Some(KeptAnswer::Kept(b"first".to_vec())));
    assert_eq!(
        store.pr_marks_answer("acme", "api", 482, "k2").unwrap(),
        None
    );
    assert_eq!(
        store.pr_marks_answer("acme", "api", 483, "k1").unwrap(),
        None,
        "a key belongs to one pull request"
    );
}

#[test]
fn only_the_latest_answers_are_kept_and_older_keys_stay_made() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    for n in 0..80_i64 {
        let key = format!("k{n}");
        store
            .add_keyed_pr_commit_marks(
                "acme",
                "api",
                482,
                &[mark("a", n % 2 == 0, n)],
                &event(1, &["a"], n, None),
                &key,
            )
            .unwrap();
        store
            .keep_pr_marks_answer("acme", "api", 482, &key, key.as_bytes())
            .unwrap();
    }
    assert_eq!(
        store.pr_marks_answer("acme", "api", 482, "k0").unwrap(),
        Some(KeptAnswer::Lost),
        "an old key is still known, so a repeat doesn't mark again"
    );
    assert_eq!(
        store.pr_marks_answer("acme", "api", 482, "k79").unwrap(),
        Some(KeptAnswer::Kept(b"k79".to_vec()))
    );
    let kept = (0..80)
        .filter(|n| {
            matches!(
                store
                    .pr_marks_answer("acme", "api", 482, &format!("k{n}"))
                    .unwrap(),
                Some(KeptAnswer::Kept(_))
            )
        })
        .count();
    assert!(
        0 < kept && kept < 80,
        "a bounded number of answers is kept: {kept}"
    );
    assert_eq!(
        store
            .pr_commit_marks("acme", "api", 482)
            .unwrap()
            .history
            .len(),
        80
    );
}

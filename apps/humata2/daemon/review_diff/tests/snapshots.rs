//! Reviewed snapshots (RV-28, RV-30), per-file Viewed (RV-29) and big files
//! in chunks (TR-11), on real repositories.

mod support;

use humata2_review_diff::{
    Budget, Cursor, DiffError, KEPT_SNAPSHOTS, Matching, RepoSpec, Viewed,
};
use support::{Repo, SESSION, added, context, diff, hunks, paths};

fn reviewed(repo: &Repo) -> u32 {
    let checkout = repo.checkout();
    let shown = diff(repo, &RepoSpec::uncommitted("api"));
    checkout
        .mark_reviewed_blocking(SESSION, &shown.trees.new)
        .expect("mark reviewed")
        .n
}

#[test]
fn since_the_last_review_shows_only_the_edits_made_after_it() {
    let repo = Repo::new();
    repo.write("a.txt", "one\n");
    repo.write("b.txt", "bee\n");
    repo.commit_all("base");
    repo.write("a.txt", "one\nedited before the review\n");
    repo.write("new.txt", "made before the review\n");
    assert_eq!(reviewed(&repo), 1);

    repo.write("a.txt", "one\nedited before the review\nand after\n");
    repo.write("b.txt", "bee\nafter\n");
    let spec = RepoSpec::since_last_review("api");
    let shown = diff(&repo, &spec);
    assert_eq!(paths(&shown), ["a.txt", "b.txt"]);
    assert_eq!(added(&hunks(&repo, &shown, "a.txt", &spec)), ["and after"]);
}

#[test]
fn a_commit_after_the_review_does_not_resurface_what_was_reviewed() {
    let repo = Repo::new();
    repo.write("a.txt", "one\n");
    repo.commit_all("base");
    repo.write("a.txt", "one\ntwo\n");
    reviewed(&repo);
    repo.commit_all("commit what was reviewed");
    assert!(paths(&diff(&repo, &RepoSpec::since_last_review("api"))).is_empty());
}

#[test]
fn marking_reviewed_leaves_the_users_index_and_status_alone() {
    let repo = Repo::new();
    repo.write("a.txt", "one\n");
    repo.commit_all("base");
    repo.write("s.txt", "staged\n");
    repo.git(&["add", "s.txt"]);
    repo.write("a.txt", "two\n");
    repo.write("u.txt", "untracked\n");
    let (index, status) = (repo.index_bytes(), repo.status());
    reviewed(&repo);
    diff(&repo, &RepoSpec::since_last_review("api"));
    assert_eq!(repo.index_bytes(), index);
    assert_eq!(repo.status(), status);
}

#[test]
fn an_earlier_review_can_be_picked_by_number() {
    let repo = Repo::new();
    repo.write("a.txt", "1\n");
    repo.commit_all("base");
    repo.write("a.txt", "1\n2\n");
    reviewed(&repo);
    repo.write("a.txt", "1\n2\n3\n");
    reviewed(&repo);
    repo.write("a.txt", "1\n2\n3\n4\n");
    let spec = |n| {
        RepoSpec::new(
            "api",
            humata2_review_diff::Base::Snapshot { n },
            humata2_review_diff::Compare::everything(),
        )
    };
    let since_first = diff(&repo, &spec(Some(1)));
    assert_eq!(added(&hunks(&repo, &since_first, "a.txt", &spec(Some(1)))), ["3", "4"]);
    let since_latest = diff(&repo, &spec(None));
    assert_eq!(added(&hunks(&repo, &since_latest, "a.txt", &spec(None))), ["4"]);
}

#[test]
fn without_a_review_since_last_review_says_there_is_none() {
    let repo = Repo::new();
    repo.write("a.txt", "1\n");
    repo.commit_all("base");
    let result = repo
        .checkout()
        .diff_blocking(&RepoSpec::since_last_review("api"), &context());
    assert!(matches!(result, Err(DiffError::NoSnapshot { .. })));
}

/// D198: never reviewed means everything is unreviewed, so a repo with no
/// snapshot shows what changed since origin/main and says it isn't reviewed.
#[test]
fn without_a_review_since_last_review_shows_everything_since_origin_main() {
    let repo = Repo::new();
    repo.write("a.txt", "1\n");
    repo.commit_all("base");
    repo.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
    repo.write("a.txt", "1\n2\n");
    repo.commit_all("the agent's commit");
    repo.write("new.txt", "made by the agent\n");
    let spec = RepoSpec::since_last_review("api");
    let shown = diff(&repo, &spec);
    assert_eq!(paths(&shown), ["a.txt", "new.txt"]);
    assert!(shown.label.contains("not reviewed yet"), "{}", shown.label);
    assert_eq!(added(&hunks(&repo, &shown, "a.txt", &spec)), ["2"]);

    reviewed(&repo);
    repo.write("new.txt", "made by the agent\nand after\n");
    let since = diff(&repo, &spec);
    assert_eq!(paths(&since), ["new.txt"]);
    assert!(!since.label.contains("not reviewed yet"), "{}", since.label);
}

#[test]
fn only_the_last_twenty_unpinned_snapshots_are_kept() {
    let repo = Repo::new();
    repo.write("a.txt", "0\n");
    repo.commit_all("base");
    let checkout = repo.checkout();
    let first = reviewed(&repo);
    checkout
        .pin_blocking(SESSION, first, true)
        .expect("pin the first review");
    for n in 1..=KEPT_SNAPSHOTS + 2 {
        repo.write("a.txt", &format!("{n}\n"));
        reviewed(&repo);
    }
    let kept = checkout.snapshots_blocking(SESSION).expect("list snapshots");
    let unpinned = kept.iter().filter(|snapshot| !snapshot.pinned).count();
    assert_eq!(unpinned, KEPT_SNAPSHOTS);
    assert!(kept.iter().any(|snapshot| snapshot.n == first && snapshot.pinned));
    let numbers: Vec<u32> = kept.iter().map(|snapshot| snapshot.n).collect();
    let mut sorted = numbers.clone();
    sorted.sort_unstable();
    assert_eq!(numbers, sorted);
}

#[test]
fn a_session_id_unfit_for_a_ref_is_refused() {
    let repo = Repo::new();
    repo.write("a.txt", "1\n");
    repo.commit_all("base");
    let shown = diff(&repo, &RepoSpec::uncommitted("api"));
    assert!(matches!(
        repo.checkout().mark_reviewed_blocking("../evil", &shown.trees.new),
        Err(DiffError::BadSession { .. })
    ));
}

#[test]
fn a_viewed_file_that_changed_shows_only_the_change_since() {
    let repo = Repo::new();
    repo.write("a.txt", "one\n");
    repo.commit_all("base");
    repo.write("a.txt", "one\ntwo\n");
    let spec = RepoSpec::uncommitted("api");
    let viewed_blob = diff(&repo, &spec)
        .file("a.txt")
        .and_then(|file| file.new_blob.clone())
        .expect("a.txt has content");
    let checkout = repo.checkout();
    let everything = Budget {
        max_lines: usize::MAX,
        max_bytes: usize::MAX,
    };
    let unchanged = diff(&repo, &spec);
    let now = unchanged.file("a.txt").expect("still shown").clone();
    assert_eq!(
        checkout
            .since_viewed_blocking(&viewed_blob, &now, Matching::from(&spec), Cursor::default(), everything)
            .expect("compare"),
        Viewed::Unchanged
    );

    repo.write("a.txt", "one\ntwo\nthree\n");
    let changed = diff(&repo, &spec);
    let now = changed.file("a.txt").expect("still shown").clone();
    let Viewed::Changed(chunk) = checkout
        .since_viewed_blocking(&viewed_blob, &now, Matching::from(&spec), Cursor::default(), everything)
        .expect("compare")
    else {
        panic!("the change since Viewed should show");
    };
    assert_eq!(added(&chunk), ["three"]);
}

#[test]
fn a_viewed_blob_git_no_longer_has_shows_the_whole_file() {
    let repo = Repo::new();
    repo.write("a.txt", "one\n");
    repo.commit_all("base");
    repo.write("a.txt", "two\n");
    let spec = RepoSpec::uncommitted("api");
    let shown = diff(&repo, &spec);
    let now = shown.file("a.txt").expect("shown").clone();
    let viewed = repo.checkout().since_viewed_blocking(
        "0123456789abcdef0123456789abcdef01234567",
        &now,
        Matching::from(&spec),
        Cursor::default(),
        Budget::default(),
    );
    assert_eq!(viewed.expect("compare"), Viewed::Forgotten);
}

#[test]
fn a_huge_file_comes_in_chunks_that_add_up_to_the_whole_diff() {
    let repo = Repo::new();
    repo.write("big.txt", "start\n");
    repo.commit_all("base");
    let body: String = (0..5000).map(|n| format!("line {n}\n")).collect();
    repo.write("big.txt", &format!("start\n{body}"));
    let spec = RepoSpec::uncommitted("api");
    let shown = diff(&repo, &spec);
    let file = shown.file("big.txt").expect("shown");
    assert_eq!(file.added, 5000);
    let checkout = repo.checkout();
    let budget = Budget {
        max_lines: 700,
        max_bytes: usize::MAX,
    };
    let mut from = Cursor::default();
    let mut lines = Vec::new();
    let mut chunks = 0;
    loop {
        let chunk = checkout
            .file_chunk_blocking(&shown.trees, file, Matching::from(&spec), from, budget)
            .expect("read a chunk");
        chunks += 1;
        let count: usize = chunk.hunks.iter().map(|hunk| hunk.lines.len()).sum();
        assert!(count <= budget.max_lines, "a chunk went over its budget");
        lines.extend(added(&chunk));
        match chunk.next {
            Some(next) => from = next,
            None => break,
        }
        assert!(chunks < 100, "chunks never ended");
    }
    assert!(chunks > 1);
    let expected: Vec<String> = (0..5000).map(|n| format!("line {n}")).collect();
    assert_eq!(lines, expected);
}

#[test]
fn a_byte_budget_cuts_a_chunk_too() {
    let repo = Repo::new();
    repo.write("wide.txt", "\n");
    repo.commit_all("base");
    let line = "x".repeat(1000);
    repo.write("wide.txt", &format!("\n{}", format!("{line}\n").repeat(50)));
    let spec = RepoSpec::uncommitted("api");
    let shown = diff(&repo, &spec);
    let chunk = repo
        .checkout()
        .file_chunk_blocking(
            &shown.trees,
            shown.file("wide.txt").expect("shown"),
            Matching::from(&spec),
            Cursor::default(),
            Budget {
                max_lines: usize::MAX,
                max_bytes: 10_000,
            },
        )
        .expect("read a chunk");
    assert!(chunk.next.is_some());
    assert!(added(&chunk).len() < 50);
}

#[test]
fn a_binary_file_has_no_lines() {
    let repo = Repo::new();
    repo.write("a.txt", "a\n");
    repo.commit_all("base");
    std::fs::write(repo.path.join("image.bin"), [0_u8, 159, 146, 150, 0, 1, 2]).expect("write");
    let spec = RepoSpec::uncommitted("api");
    let shown = diff(&repo, &spec);
    assert!(shown.file("image.bin").is_some_and(|file| file.binary));
    let chunk = hunks(&repo, &shown, "image.bin", &spec);
    assert!(chunk.binary);
    assert!(chunk.hunks.is_empty());
}

#[test]
fn another_sessions_commit_is_fetched_to_compare_against_and_its_clone_is_left_alone() {
    let mine = Repo::new();
    mine.write("a.txt", "one\n");
    mine.commit_all("base");
    let theirs = Repo::new();
    theirs.write("a.txt", "one\ntheirs\n");
    let their_head = theirs.commit_all("their work");
    let their_refs = theirs.git(&["for-each-ref"]);
    mine.write("a.txt", "one\nmine\n");

    let checkout = mine.checkout();
    let commit = checkout
        .fetch_session_head_blocking(theirs.dir(), "se_other")
        .expect("fetch their HEAD");
    assert_eq!(commit, their_head);
    let spec = RepoSpec::new(
        "api",
        humata2_review_diff::Base::Rev { rev: commit },
        humata2_review_diff::Compare::everything(),
    );
    let shown = diff(&mine, &spec);
    assert_eq!(added(&hunks(&mine, &shown, "a.txt", &spec)), ["mine"]);
    assert_eq!(theirs.git(&["for-each-ref"]), their_refs, "their clone gained a ref");

    // A clone with no commit has nothing to compare against.
    let empty = Repo::new();
    let error = checkout.fetch_session_head_blocking(empty.dir(), "se_empty").unwrap_err();
    assert!(matches!(error, DiffError::OtherSession { .. }), "{error:?}");
}

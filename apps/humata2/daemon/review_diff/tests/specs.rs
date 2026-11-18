//! Each base and compare of a diff spec (RV-31) on a real repository gives
//! the files and lines it should, and never writes the user's index.

mod support;

use humata2_review_diff::{
    Base, BaseContext, Checkout, Compare, DiffError, FileStatus, PathFilter, RepoSpec, Untracked,
    Upto, Whitespace,
};
use support::{Repo, added, context, diff, hunks, paths, removed};

/// `main` with a.txt and c.txt (c.txt after `feature` left), `feature`
/// with a committed change to b.txt, then a staged s.txt, an unstaged
/// edit of a.txt and untracked new/u.txt and other/v.txt.
fn pull_request() -> Repo {
    let repo = Repo::new();
    repo.write("a.txt", "one\ntwo\nthree\n");
    repo.write("b.txt", "bee\n");
    repo.write(".gitignore", "*.log\n");
    repo.commit_all("base");
    repo.git(&["checkout", "-q", "-b", "feature"]);
    repo.write("b.txt", "bee\nbuzz\n");
    repo.commit_all("feature work");
    repo.git(&["checkout", "-q", "main"]);
    repo.write("c.txt", "sea\n");
    repo.commit_all("main moves on");
    repo.git(&["checkout", "-q", "feature"]);
    repo.write("s.txt", "staged\n");
    repo.git(&["add", "s.txt"]);
    repo.write("a.txt", "one\nTWO\nthree\n");
    repo.write("new/u.txt", "untracked\n");
    repo.write("other/v.txt", "elsewhere\n");
    repo.write("debug.log", "ignored\n");
    repo
}

fn since_main(compare: Compare) -> RepoSpec {
    RepoSpec::new(
        "api",
        Base::MergeBase {
            with: "main".to_owned(),
        },
        compare,
    )
}

fn compare(upto: Upto, untracked: Untracked) -> Compare {
    Compare {
        upto,
        untracked,
        ..Compare::commits()
    }
}

#[test]
fn the_merge_base_shows_what_github_would_and_nothing_from_main() {
    let repo = pull_request();
    let spec = since_main(Compare::commits());
    let shown = diff(&repo, &spec);
    assert_eq!(paths(&shown), ["b.txt"]);
    assert_eq!(added(&hunks(&repo, &shown, "b.txt", &spec)), ["buzz"]);
}

#[test]
fn each_compare_level_adds_its_own_changes() {
    let repo = pull_request();
    let cases = [
        (compare(Upto::Commits, Untracked::None), vec!["b.txt"]),
        (compare(Upto::Staged, Untracked::None), vec!["b.txt", "s.txt"]),
        (
            compare(Upto::Unstaged, Untracked::None),
            vec!["a.txt", "b.txt", "s.txt"],
        ),
        (
            compare(Upto::Unstaged, Untracked::All),
            vec!["a.txt", "b.txt", "new/u.txt", "other/v.txt", "s.txt"],
        ),
        (
            compare(Upto::Commits, Untracked::All),
            vec!["b.txt", "new/u.txt", "other/v.txt"],
        ),
        (
            compare(Upto::Staged, Untracked::Paths(vec!["new/".to_owned()])),
            vec!["b.txt", "new/u.txt", "s.txt"],
        ),
    ];
    for (compare, expected) in cases {
        let shown = diff(&repo, &since_main(compare.clone()));
        assert_eq!(paths(&shown), expected, "for {compare:?}");
    }
}

#[test]
fn a_branch_compared_directly_shows_its_newer_commits_as_removed() {
    let repo = pull_request();
    let spec = RepoSpec::new(
        "api",
        Base::Rev {
            rev: "main".to_owned(),
        },
        Compare::commits(),
    );
    let shown = diff(&repo, &spec);
    assert_eq!(paths(&shown), ["b.txt", "c.txt"]);
    assert_eq!(
        shown.file("c.txt").map(|file| file.status.clone()),
        Some(FileStatus::Deleted)
    );
}

#[test]
fn only_unstaged_changes_compare_the_disk_with_the_index() {
    let repo = pull_request();
    let spec = RepoSpec::new(
        "api",
        Base::Index,
        compare(Upto::Unstaged, Untracked::None),
    );
    let shown = diff(&repo, &spec);
    assert_eq!(paths(&shown), ["a.txt"]);
    let chunk = hunks(&repo, &shown, "a.txt", &spec);
    assert_eq!(added(&chunk), ["TWO"]);
    assert_eq!(removed(&chunk), ["two"]);
}

#[test]
fn only_staged_changes_compare_the_index_with_head() {
    let repo = pull_request();
    let spec = RepoSpec::new(
        "api",
        Base::Rev {
            rev: "HEAD".to_owned(),
        },
        compare(Upto::Staged, Untracked::None),
    );
    assert_eq!(paths(&diff(&repo, &spec)), ["s.txt"]);
}

#[test]
fn ignored_files_never_count_as_untracked() {
    let repo = pull_request();
    let shown = diff(&repo, &since_main(Compare::everything()));
    assert!(shown.file("debug.log").is_none());
}

#[test]
fn path_filters_include_and_exclude() {
    let repo = pull_request();
    let mut spec = since_main(Compare::everything());
    spec.paths = PathFilter {
        include: vec!["new/".to_owned(), "a.txt".to_owned()],
        exclude: Vec::new(),
    };
    assert_eq!(paths(&diff(&repo, &spec)), ["a.txt", "new/u.txt"]);
    spec.paths = PathFilter {
        include: Vec::new(),
        exclude: vec!["*.txt".to_owned()],
    };
    assert!(paths(&diff(&repo, &spec)).is_empty());
}

#[test]
fn computing_any_diff_leaves_the_users_index_and_status_alone() {
    let repo = pull_request();
    // Every tracked file's cached stat is now stale (same bytes, a new
    // mtime), so anything that refreshes the user's index rewrites it: the
    // check can't pass by luck of timing.
    repo.make_index_stale(&["a.txt", "b.txt", ".gitignore", "s.txt"]);
    let status = repo.status();
    let index = repo.index_bytes();
    for upto in [Upto::Commits, Upto::Staged, Upto::Unstaged] {
        for untracked in [Untracked::None, Untracked::All] {
            diff(&repo, &since_main(compare(upto, untracked)));
        }
    }
    diff(
        &repo,
        &RepoSpec::new("api", Base::Index, compare(Upto::Unstaged, Untracked::All)),
    );
    assert_eq!(repo.index_bytes(), index);
    assert_eq!(repo.status(), status);
}

#[test]
fn a_moved_file_is_one_rename_unless_renames_are_off() {
    let repo = Repo::new();
    let body: String = (1..=40).map(|n| format!("line {n}\n")).collect();
    repo.write("old/name.rs", &body);
    repo.commit_all("base");
    repo.remove("old/name.rs");
    repo.write("new/name.rs", &body.replace("line 20\n", "line twenty\n"));
    let mut spec = RepoSpec::uncommitted("api");
    let shown = diff(&repo, &spec);
    assert_eq!(paths(&shown), ["new/name.rs"]);
    let Some(FileStatus::Renamed { from, .. }) = shown.file("new/name.rs").map(|f| f.status.clone())
    else {
        panic!("not a rename: {shown:?}");
    };
    assert_eq!(from, "old/name.rs");
    assert_eq!(added(&hunks(&repo, &shown, "new/name.rs", &spec)), ["line twenty"]);

    spec.renames = false;
    let split = diff(&repo, &spec);
    assert_eq!(paths(&split), ["new/name.rs", "old/name.rs"]);
}

#[test]
fn ignoring_whitespace_hides_reindented_lines() {
    let repo = Repo::new();
    repo.write("f.py", "def f():\n    return 1\n");
    repo.commit_all("base");
    repo.write("f.py", "def f():\n        return 1\n");
    let mut spec = RepoSpec::uncommitted("api");
    let shown = diff(&repo, &spec);
    assert_eq!(added(&hunks(&repo, &shown, "f.py", &spec)), ["        return 1"]);
    spec.whitespace = Whitespace::IgnoreAll;
    let shown = diff(&repo, &spec);
    if shown.file("f.py").is_some() {
        assert!(added(&hunks(&repo, &shown, "f.py", &spec)).is_empty());
    }
}

#[test]
fn a_repo_without_commits_shows_its_files_as_added() {
    let repo = Repo::new();
    repo.write("first.txt", "hello\n");
    let shown = diff(&repo, &RepoSpec::uncommitted("api"));
    assert_eq!(paths(&shown), ["first.txt"]);
    assert_eq!(shown.head, None);
    let since_branch = repo
        .checkout()
        .diff_blocking(&RepoSpec::since_branch("api", "main"), &context());
    assert!(matches!(since_branch, Err(DiffError::NoCommits { .. })));
}

#[test]
fn a_fork_compares_against_its_fork_point() {
    let repo = Repo::new();
    repo.write("a.txt", "a\n");
    let fork_point = repo.commit_all("parent");
    repo.write("a.txt", "a\nfork\n");
    repo.commit_all("fork's work");
    let spec = RepoSpec::new("api", Base::ForkPoint, Compare::commits());
    let context = BaseContext {
        session: "s-1".to_owned(),
        fork_point: Some(fork_point),
    };
    let shown = repo
        .checkout()
        .diff_blocking(&spec, &context)
        .expect("diff since the fork");
    assert_eq!(added(&hunks(&repo, &shown, "a.txt", &spec)), ["fork"]);
    let not_a_fork = repo
        .checkout()
        .diff_blocking(&spec, &support::context());
    assert!(matches!(not_a_fork, Err(DiffError::NotAFork)));
}

#[test]
fn an_unknown_branch_says_so() {
    let repo = pull_request();
    let spec = RepoSpec::since_branch("api", "origin/nope");
    assert!(matches!(
        repo.checkout().diff_blocking(&spec, &context()),
        Err(DiffError::UnknownRevision { .. })
    ));
}

#[test]
fn a_folder_that_isnt_a_repository_is_refused() {
    let dir = tempfile::tempdir().expect("temporary folder");
    assert!(matches!(
        Checkout::open(dir.path()),
        Err(DiffError::NotARepository { .. })
    ));
}

#[test]
fn moving_head_or_the_index_changes_the_stamp_but_editing_files_does_not() {
    let repo = pull_request();
    let checkout = repo.checkout();
    let watch = checkout.watch_set().expect("the files to watch");
    let before = Checkout::stamp(&watch);
    repo.write("a.txt", "edited again\n");
    assert_eq!(Checkout::stamp(&watch), before);
    repo.commit_all("commit everything");
    assert_ne!(Checkout::stamp(&watch), before);
}

#[tokio::test]
async fn several_repos_are_diffed_together_in_the_specs_order() {
    let api = pull_request();
    let ledger = Repo::new();
    ledger.write("l.txt", "ledger\n");
    let clones = humata2_review_diff::Clones::new(
        [
            ("acme/api".to_owned(), api.checkout()),
            ("acme/ledger".to_owned(), ledger.checkout()),
        ]
        .into(),
    );
    let spec = humata2_review_diff::DiffSpec {
        repos: vec![
            RepoSpec::uncommitted("acme/ledger"),
            RepoSpec::new(
                "acme/api",
                Base::MergeBase {
                    with: "main".to_owned(),
                },
                Compare::commits(),
            ),
        ],
    };
    let diffs = clones.diff(&spec, &context()).await.expect("diff both repos");
    let repos: Vec<&str> = diffs.iter().map(|diff| diff.repo.as_str()).collect();
    assert_eq!(repos, ["acme/ledger", "acme/api"]);
    assert_eq!(paths(&diffs[1]), ["b.txt"]);
    let unknown = humata2_review_diff::DiffSpec {
        repos: vec![RepoSpec::uncommitted("acme/web")],
    };
    assert!(matches!(
        clones.diff(&unknown, &context()).await,
        Err(DiffError::UnknownRepo { .. })
    ));
}

// D800: a diff with its own new side, for the layers of a stack.

#[test]
fn a_range_with_a_head_shows_only_its_commits_whatever_is_checked_out() {
    let repo = pull_request();
    let spec = RepoSpec::new(
        "api",
        Base::Rev {
            rev: "feature~1".to_owned(),
        },
        Compare::at("feature"),
    );
    // On feature with staged, unstaged and untracked work: none of it counts.
    assert_eq!(paths(&diff(&repo, &spec)), ["b.txt"]);
    // Checked out somewhere else entirely: the same diff.
    repo.git(&["stash", "-u", "-q"]);
    repo.git(&["checkout", "-q", "main"]);
    assert_eq!(paths(&diff(&repo, &spec)), ["b.txt"]);
    assert_eq!(added(&hunks(&repo, &diff(&repo, &spec), "b.txt", &spec)), ["buzz"]);
}

#[test]
fn worktree_paths_lay_only_the_work_under_them_over_head() {
    let repo = pull_request();
    let only_new = RepoSpec::new(
        "api",
        Base::Rev {
            rev: "HEAD".to_owned(),
        },
        Compare::worktree(vec!["new/".to_owned()]),
    );
    assert_eq!(paths(&diff(&repo, &only_new)), ["new/u.txt"]);
    let tracked = RepoSpec::new(
        "api",
        Base::Rev {
            rev: "HEAD".to_owned(),
        },
        Compare::worktree(vec!["a.txt".to_owned(), "s.txt".to_owned()]),
    );
    // Staged and unstaged tracked work under the paths, nothing else.
    assert_eq!(paths(&diff(&repo, &tracked)), ["a.txt", "s.txt"]);
    let ignored = RepoSpec::new(
        "api",
        Base::Rev {
            rev: "HEAD".to_owned(),
        },
        Compare::worktree(vec![".".to_owned()]),
    );
    assert!(diff(&repo, &ignored).file("debug.log").is_none());
}

#[test]
fn a_head_with_worktree_paths_ends_at_head_plus_the_work_under_them() {
    let repo = pull_request();
    let spec = RepoSpec::new(
        "api",
        Base::Rev {
            rev: "feature~1".to_owned(),
        },
        Compare {
            head: Some("feature".to_owned()),
            worktree_paths: vec!["other/".to_owned()],
            ..Compare::commits()
        },
    );
    assert_eq!(paths(&diff(&repo, &spec)), ["b.txt", "other/v.txt"]);
}

#[test]
fn a_head_or_worktree_paths_with_a_checkout_level_is_refused() {
    let repo = pull_request();
    for compare in [
        Compare {
            head: Some("feature".to_owned()),
            ..Compare::everything()
        },
        Compare {
            worktree_paths: vec!["new/".to_owned()],
            upto: Upto::Staged,
            ..Compare::commits()
        },
    ] {
        let spec = since_main(compare.clone());
        let error = repo
            .checkout()
            .diff_blocking(&spec, &context())
            .expect_err("a conflicting compare is refused");
        assert!(
            matches!(error, DiffError::ConflictingCompare { .. }),
            "for {compare:?}: {error:?}"
        );
    }
}

#[test]
fn a_head_that_does_not_resolve_is_an_unknown_revision() {
    let repo = pull_request();
    let spec = since_main(Compare::at("no-such-branch"));
    let error = repo
        .checkout()
        .diff_blocking(&spec, &context())
        .expect_err("an unknown head is refused");
    assert!(matches!(error, DiffError::UnknownRevision { .. }), "{error:?}");
}

use std::path::Path;

use super::*;
use crate::review::parse;

const TEN: &str = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n";

/// A session on a repository whose `list.txt` has ten lines, with its
/// first checkpoint taken.
async fn session(
    dir: &Path,
) -> (
    Workspaces,
    Vec<Repository>,
    Vec<Checkout>,
    Vec<Vec<Snapshot>>,
) {
    let repository = repository(dir, "app");
    std::fs::write(repository.path.join("list.txt"), TEN).expect("wrote");
    for arguments in [
        &["add", "list.txt"][..],
        &["commit", "--quiet", "-m", "list"],
    ] {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&repository.path)
            .args(["-c", "user.name=T", "-c", "user.email=t@localhost"])
            .args(arguments)
            .status()
            .expect("git runs");
        assert!(status.success(), "git {arguments:?}");
    }
    let workspaces = Workspaces::new(dir.join("worktrees"));
    let repositories = vec![repository];
    let checkouts = workspaces
        .create("s", &repositories, &[])
        .await
        .expect("created");
    let base = workspaces
        .snapshot("s", 0, &checkouts)
        .await
        .expect("snapshotted");
    (workspaces, repositories, checkouts, vec![base])
}

fn query(range: Range) -> Query {
    Query {
        range,
        ..Query::default()
    }
}

fn whole(path: &str) -> Query {
    Query {
        only: Some(FileRef {
            repository: "app".to_owned(),
            path: path.to_owned(),
        }),
        whole_file: true,
        ..Query::default()
    }
}

fn list_file() -> FileRef {
    FileRef {
        repository: "app".to_owned(),
        path: "list.txt".to_owned(),
    }
}

/// `(kind, old, new, text)` of each line of each hunk of the only file.
fn lines(changes: &[Changes]) -> Vec<Vec<(LineKind, Option<u32>, Option<u32>, String)>> {
    let [repository] = changes else {
        panic!("{changes:?}")
    };
    repository
        .files
        .iter()
        .flat_map(|file| &file.hunks)
        .map(|hunk| {
            hunk.lines
                .iter()
                .map(|line| (line.kind, line.old, line.new, line.text.clone()))
                .collect()
        })
        .collect()
}

fn changed(changes: &[Changes]) -> Vec<String> {
    lines(changes)
        .into_iter()
        .flatten()
        .filter(|(kind, ..)| *kind != LineKind::Context)
        .map(|(kind, _, _, text)| {
            format!("{}{text}", if kind == LineKind::Added { '+' } else { '-' })
        })
        .collect()
}

#[tokio::test]
async fn a_change_reads_as_hunks_of_numbered_lines_with_the_context_asked_for() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let (workspaces, repositories, checkouts, checkpoints) = session(dir.path()).await;
    let edited = TEN.replace("five\n", "FIVE\n");
    std::fs::write(checkouts[0].path.join("list.txt"), edited).expect("wrote");

    let one = Query {
        context: 1,
        ..Query::default()
    };
    let changes = workspaces
        .changes("s", &repositories, &checkpoints, &checkouts, &one)
        .await
        .expect("compared");
    assert_eq!(
        lines(&changes),
        vec![vec![
            (LineKind::Context, Some(4), Some(4), "four".to_owned()),
            (LineKind::Removed, Some(5), None, "five".to_owned()),
            (LineKind::Added, None, Some(5), "FIVE".to_owned()),
            (LineKind::Context, Some(6), Some(6), "six".to_owned()),
        ]]
    );

    let all = workspaces
        .changes(
            "s",
            &repositories,
            &checkpoints,
            &checkouts,
            &whole("list.txt"),
        )
        .await
        .expect("compared");
    // Ten lines and the one taken out.
    assert_eq!(lines(&all)[0].len(), 11);
}

#[tokio::test]
async fn a_whole_file_that_did_not_change_is_every_line_unchanged() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let (workspaces, repositories, checkouts, checkpoints) = session(dir.path()).await;
    let changes = workspaces
        .changes(
            "s",
            &repositories,
            &checkpoints,
            &checkouts,
            &whole("list.txt"),
        )
        .await
        .expect("read");
    assert_eq!(changes[0].files[0].status, FileStatus::Unchanged);
    assert_eq!(lines(&changes)[0].len(), 10);
    assert!(matches!(
        workspaces
            .changes(
                "s",
                &repositories,
                &checkpoints,
                &checkouts,
                &whole("missing.txt")
            )
            .await,
        Err(WorkspaceError::NoSuchFile { .. })
    ));
}

#[tokio::test]
async fn staging_one_hunk_leaves_the_other_unstaged_and_unstaging_puts_it_back() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let (workspaces, repositories, checkouts, checkpoints) = session(dir.path()).await;
    let edited = TEN.replace("two\n", "TWO\n").replace("nine\n", "NINE\n");
    std::fs::write(checkouts[0].path.join("list.txt"), edited).expect("wrote");
    let read = |range| {
        let workspaces = &workspaces;
        let repositories = &repositories;
        let checkouts = &checkouts;
        let checkpoints = &checkpoints;
        async move {
            let query = Query {
                context: 1,
                ..query(range)
            };
            workspaces
                .changes("s", repositories, checkpoints, checkouts, &query)
                .await
                .expect("compared")
        }
    };
    let unstaged = read(Range::Unstaged).await;
    let second = &unstaged[0].files[0].hunks[1];
    let at = HunkAt {
        old_start: second.old_start,
        new_start: second.new_start,
        context: 1,
    };
    workspaces
        .edit(
            &checkouts,
            &Edit {
                file: list_file(),
                action: Action::Stage,
                hunk: Some(at),
            },
        )
        .await
        .expect("staged");
    assert_eq!(changed(&read(Range::Staged).await), ["-nine", "+NINE"]);
    assert_eq!(changed(&read(Range::Unstaged).await), ["-two", "+TWO"]);
    // Everything since the copy was made, staged or not.
    assert_eq!(changed(&read(Range::Session).await).len(), 4);

    let staged = read(Range::Staged).await;
    let hunk = &staged[0].files[0].hunks[0];
    workspaces
        .edit(
            &checkouts,
            &Edit {
                file: list_file(),
                action: Action::Unstage,
                hunk: Some(HunkAt {
                    old_start: hunk.old_start,
                    new_start: hunk.new_start,
                    context: 1,
                }),
            },
        )
        .await
        .expect("unstaged");
    assert!(changed(&read(Range::Staged).await).is_empty());
    assert_eq!(changed(&read(Range::Unstaged).await).len(), 4);
}

#[tokio::test]
async fn reverting_a_hunk_or_a_new_file_throws_only_that_away() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let (workspaces, repositories, checkouts, checkpoints) = session(dir.path()).await;
    let work = &checkouts[0].path;
    let edited = TEN.replace("two\n", "TWO\n").replace("nine\n", "NINE\n");
    std::fs::write(work.join("list.txt"), edited).expect("wrote");
    std::fs::write(work.join("NEW.md"), "new\n").expect("wrote");

    let unstaged = workspaces
        .changes(
            "s",
            &repositories,
            &checkpoints,
            &checkouts,
            &Query {
                context: 0,
                only: Some(list_file()),
                ..query(Range::Unstaged)
            },
        )
        .await
        .expect("compared");
    let first = &unstaged[0].files[0].hunks[0];
    workspaces
        .edit(
            &checkouts,
            &Edit {
                file: list_file(),
                action: Action::Revert,
                hunk: Some(HunkAt {
                    old_start: first.old_start,
                    new_start: first.new_start,
                    context: 0,
                }),
            },
        )
        .await
        .expect("reverted");
    assert_eq!(
        read(&work.join("list.txt")).as_deref(),
        Some(TEN.replace("nine\n", "NINE\n").as_str())
    );

    workspaces
        .edit(
            &checkouts,
            &Edit {
                file: FileRef {
                    repository: "app".to_owned(),
                    path: "NEW.md".to_owned(),
                },
                action: Action::Revert,
                hunk: None,
            },
        )
        .await
        .expect("reverted");
    assert!(!work.join("NEW.md").exists());

    // A hunk that isn't there any more.
    let gone = workspaces
        .edit(
            &checkouts,
            &Edit {
                file: list_file(),
                action: Action::Revert,
                hunk: Some(HunkAt {
                    old_start: first.old_start,
                    new_start: first.new_start,
                    context: 0,
                }),
            },
        )
        .await;
    assert!(
        matches!(gone, Err(WorkspaceError::ChangeGone { .. })),
        "{gone:?}"
    );
}

#[tokio::test]
async fn a_turn_is_what_changed_between_its_checkpoints() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let (workspaces, repositories, checkouts, mut checkpoints) = session(dir.path()).await;
    let work = &checkouts[0].path;
    std::fs::write(work.join("list.txt"), TEN.replace("one\n", "ONE\n")).expect("wrote");
    checkpoints.push(
        workspaces
            .snapshot("s", 1, &checkouts)
            .await
            .expect("snapshotted"),
    );
    std::fs::write(
        work.join("list.txt"),
        TEN.replace("one\n", "ONE\n").replace("ten\n", "TEN\n"),
    )
    .expect("wrote");
    checkpoints.push(
        workspaces
            .snapshot("s", 2, &checkouts)
            .await
            .expect("snapshotted"),
    );

    let turn = |turn| {
        let (workspaces, repositories, checkouts, checkpoints) =
            (&workspaces, &repositories, &checkouts, &checkpoints);
        async move {
            workspaces
                .changes(
                    "s",
                    repositories,
                    checkpoints,
                    checkouts,
                    &query(Range::Turn(turn)),
                )
                .await
        }
    };
    assert_eq!(changed(&turn(1).await.expect("read")), ["-one", "+ONE"]);
    assert_eq!(changed(&turn(2).await.expect("read")), ["-ten", "+TEN"]);
    assert!(matches!(
        turn(3).await,
        Err(WorkspaceError::UnknownTurn { .. })
    ));
}

#[tokio::test]
async fn against_a_branch_is_from_where_the_two_meet() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let (workspaces, repositories, checkouts, checkpoints) = session(dir.path()).await;
    std::fs::write(
        checkouts[0].path.join("list.txt"),
        TEN.replace("six\n", "SIX\n"),
    )
    .expect("wrote");
    let changes = workspaces
        .changes(
            "s",
            &repositories,
            &checkpoints,
            &checkouts,
            &query(Range::Branch("main".to_owned())),
        )
        .await
        .expect("compared");
    assert_eq!(changed(&changes), ["-six", "+SIX"]);
    let unknown = workspaces
        .changes(
            "s",
            &repositories,
            &checkpoints,
            &checkouts,
            &query(Range::Branch("nope".to_owned())),
        )
        .await;
    assert!(matches!(unknown, Err(WorkspaceError::UnknownBranch { .. })));
}

#[test]
fn renamed_deleted_and_binary_files_are_told_apart() {
    let patch = "\
diff --git a/old name.txt b/new name.txt
similarity index 90%
rename from old name.txt
rename to new name.txt
index 1111111111111111111111111111111111111111..2222222222222222222222222222222222222222 100644
--- a/old name.txt
+++ b/new name.txt
@@ -1 +1 @@ fn main
-a
+b
diff --git a/gone.txt b/gone.txt
deleted file mode 100644
index 3333333333333333333333333333333333333333..0000000000000000000000000000000000000000
--- a/gone.txt
+++ /dev/null
@@ -1,2 +0,0 @@
-x
-y
\\ No newline at end of file
diff --git a/image.png b/image.png
new file mode 100644
index 0000000000000000000000000000000000000000..4444444444444444444444444444444444444444
Binary files /dev/null and b/image.png differ
";
    let files = parse(patch);
    let summary: Vec<_> = files
        .iter()
        .map(|file| {
            (
                file.path.as_str(),
                file.old_path.as_deref(),
                file.status,
                file.binary,
                file.blob.is_some(),
                file.hunks
                    .iter()
                    .map(|hunk| hunk.lines.len())
                    .sum::<usize>(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            (
                "new name.txt",
                Some("old name.txt"),
                FileStatus::Renamed,
                false,
                true,
                2
            ),
            ("gone.txt", None, FileStatus::Deleted, false, false, 2),
            ("image.png", None, FileStatus::Added, true, true, 0),
        ]
    );
    assert_eq!(files[0].hunks[0].header, "fn main");
}

use std::path::Path;

use super::*;

/// A repository with one commit of `README.md`.
fn repository(dir: &Path, name: &str) -> Repository {
    let path = dir.join(name);
    std::fs::create_dir_all(&path).expect("made the folder");
    std::fs::write(path.join("README.md"), "hello\n").expect("wrote");
    for arguments in [
        &["init", "--quiet", "--initial-branch=main"][..],
        &["add", "README.md"],
        &["commit", "--quiet", "-m", "first"],
    ] {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&path)
            .args(arguments)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@localhost")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@localhost")
            .status()
            .expect("git runs");
        assert!(status.success(), "git {arguments:?}");
    }
    Repository {
        name: name.to_owned(),
        path,
    }
}

fn head(path: &Path) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("git runs");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

#[tokio::test]
async fn a_session_works_on_its_own_branch_and_leaves_the_repository_alone() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let repository = repository(dir.path(), "app");
    let workspaces = Workspaces::new(dir.path().join("worktrees"));

    let checkouts = workspaces
        .create("session-one", std::slice::from_ref(&repository), &[])
        .await
        .expect("created");
    let [checkout] = checkouts.as_slice() else {
        panic!("one checkout: {checkouts:?}");
    };
    assert_eq!(
        checkout.path,
        workspaces.working_directory("session-one", std::slice::from_ref(&repository))
    );
    assert_eq!(head(&checkout.path), head(&repository.path));
    std::fs::write(checkout.path.join("README.md"), "changed\n").expect("wrote");
    assert_eq!(
        read(&repository.path.join("README.md")).as_deref(),
        Some("hello\n")
    );
}

#[tokio::test]
async fn a_fork_from_a_snapshot_has_the_files_as_they_were_without_new_commits() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let repository = repository(dir.path(), "app");
    let repositories = std::slice::from_ref(&repository);
    let workspaces = Workspaces::new(dir.path().join("worktrees"));
    let parent = workspaces
        .create("parent", repositories, &[])
        .await
        .expect("created");
    let work = &parent[0].path;

    std::fs::write(work.join("README.md"), "first turn\n").expect("wrote");
    std::fs::write(work.join("NEW.md"), "added\n").expect("wrote");
    let first = workspaces
        .snapshot("parent", 1, &parent)
        .await
        .expect("snapshotted");
    std::fs::write(work.join("README.md"), "second turn\n").expect("wrote");
    std::fs::remove_file(work.join("NEW.md")).expect("removed");
    workspaces
        .snapshot("parent", 2, &parent)
        .await
        .expect("snapshotted");

    assert_eq!(
        head(work),
        head(&repository.path),
        "no commit on the branch"
    );
    let fork = workspaces
        .create("fork", repositories, &first)
        .await
        .expect("forked");
    let copy = &fork[0].path;
    assert_ne!(copy, work);
    assert_eq!(
        read(&copy.join("README.md")).as_deref(),
        Some("first turn\n")
    );
    assert_eq!(read(&copy.join("NEW.md")).as_deref(), Some("added\n"));
    assert_eq!(head(copy), first[0].head);
    assert_eq!(
        read(&work.join("README.md")).as_deref(),
        Some("second turn\n")
    );
    assert_eq!(read(&work.join("NEW.md")), None);
}

#[tokio::test]
async fn changes_are_everything_since_the_copy_was_made_new_files_included() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let repository = repository(dir.path(), "app");
    let repositories = std::slice::from_ref(&repository);
    let workspaces = Workspaces::new(dir.path().join("worktrees"));
    let checkouts = workspaces
        .create("s", repositories, &[])
        .await
        .expect("created");
    let base = workspaces
        .snapshot("s", 0, &checkouts)
        .await
        .expect("snapshotted");
    let checkpoints = [base];
    let unchanged = workspaces
        .changes(
            "s",
            repositories,
            &checkpoints,
            &checkouts,
            &Query::default(),
        )
        .await
        .expect("compared");
    assert_eq!(unchanged[0].patch, "");

    std::fs::write(checkouts[0].path.join("NEW.md"), "added\n").expect("wrote");
    let changes = workspaces
        .changes(
            "s",
            repositories,
            &checkpoints,
            &checkouts,
            &Query::default(),
        )
        .await
        .expect("compared");
    assert!(changes[0].patch.contains("+added"), "{changes:?}");
    let [file] = changes[0].files.as_slice() else {
        panic!("{changes:?}")
    };
    assert_eq!(
        (file.path.as_str(), file.status),
        ("NEW.md", FileStatus::Added)
    );
}

#[tokio::test]
async fn a_save_with_nothing_changed_since_the_last_keeps_nothing() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let repository = repository(dir.path(), "app");
    let workspaces = Workspaces::new(dir.path().join("worktrees"));
    let checkouts = workspaces
        .create("s", std::slice::from_ref(&repository), &[])
        .await
        .expect("created");
    let base = workspaces
        .snapshot("s", 0, &checkouts)
        .await
        .expect("snapshotted");

    let unchanged = workspaces
        .save("s", 0, &checkouts, &base)
        .await
        .expect("saved");
    assert_eq!(unchanged, None);

    std::fs::write(checkouts[0].path.join("NEW.md"), "added\n").expect("wrote");
    let saved = workspaces
        .save("s", 0, &checkouts, &base)
        .await
        .expect("saved")
        .expect("a change is kept");
    assert_ne!(saved[0].commit, base[0].commit);
    let again = workspaces
        .save("s", 1, &checkouts, &saved)
        .await
        .expect("saved");
    assert_eq!(again, None);
}

#[tokio::test]
async fn removing_a_copy_keeps_its_snapshots() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let repository = repository(dir.path(), "app");
    let repositories = std::slice::from_ref(&repository);
    let workspaces = Workspaces::new(dir.path().join("worktrees"));
    let checkouts = workspaces
        .create("s", repositories, &[])
        .await
        .expect("created");
    std::fs::write(checkouts[0].path.join("README.md"), "kept\n").expect("wrote");
    let snapshot = workspaces
        .snapshot("s", 1, &checkouts)
        .await
        .expect("snapshotted");

    workspaces.remove(&checkouts).await.expect("removed");
    assert!(!checkouts[0].path.exists());
    let changes = workspaces
        .changes(
            "s",
            repositories,
            &[snapshot.clone(), snapshot.clone()],
            &[],
            &Query::default(),
        )
        .await
        .expect("compared");
    assert!(changes[0].patch.contains("+kept"), "{changes:?}");
    let restored = workspaces
        .create("s", repositories, &snapshot)
        .await
        .expect("made again");
    assert_eq!(
        read(&restored[0].path.join("README.md")).as_deref(),
        Some("kept\n")
    );
}

#[tokio::test]
async fn a_project_of_several_repositories_gets_a_folder_holding_each() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let repositories = [repository(dir.path(), "app"), repository(dir.path(), "api")];
    let workspaces = Workspaces::new(dir.path().join("worktrees"));
    let checkouts = workspaces
        .create("s", &repositories, &[])
        .await
        .expect("created");
    let folder = workspaces.working_directory("s", &repositories);
    assert_eq!(checkouts[0].path, folder.join("app"));
    assert_eq!(checkouts[1].path, folder.join("api"));
    assert!(folder.join("api/README.md").is_file());
}

#[tokio::test]
async fn a_repository_named_like_a_hidden_folder_is_snapshotted() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let repository = repository(dir.path(), ".hidden");
    let workspaces = Workspaces::new(dir.path().join("worktrees"));
    let checkouts = workspaces
        .create("s", std::slice::from_ref(&repository), &[])
        .await
        .expect("created");
    workspaces
        .snapshot("s", 0, &checkouts)
        .await
        .expect("snapshotted");
}

#[tokio::test]
async fn a_copy_left_part_way_is_used_again() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let repository = repository(dir.path(), "app");
    let repositories = std::slice::from_ref(&repository);
    let workspaces = Workspaces::new(dir.path().join("worktrees"));
    let first = workspaces
        .create("s", repositories, &[])
        .await
        .expect("created");
    let again = workspaces
        .create("s", repositories, &[])
        .await
        .expect("created again");
    assert_eq!(first, again);
}

#[tokio::test]
async fn a_folder_in_the_way_is_refused() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let repository = repository(dir.path(), "app");
    let workspaces = Workspaces::new(dir.path().join("worktrees"));
    std::fs::create_dir_all(dir.path().join("worktrees/s/app")).expect("made");
    let result = workspaces
        .create("s", std::slice::from_ref(&repository), &[])
        .await;
    assert!(
        matches!(result, Err(WorkspaceError::InTheWay { .. })),
        "{result:?}"
    );
}

#[tokio::test]
async fn a_folder_without_git_or_without_commits_is_refused() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let workspaces = Workspaces::new(dir.path().join("worktrees"));
    let plain = Repository {
        name: "plain".into(),
        path: dir.path().join("plain"),
    };
    std::fs::create_dir_all(&plain.path).expect("made");
    let result = workspaces.create("s", &[plain], &[]).await;
    assert!(
        matches!(result, Err(WorkspaceError::NotARepository { .. })),
        "{result:?}"
    );

    let empty = Repository {
        name: "empty".into(),
        path: dir.path().join("empty"),
    };
    std::fs::create_dir_all(&empty.path).expect("made");
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(&empty.path)
        .args(["init", "--quiet"])
        .status()
        .expect("git runs");
    assert!(status.success());
    let result = workspaces.create("t", &[empty], &[]).await;
    assert!(
        matches!(result, Err(WorkspaceError::NoCommits { .. })),
        "{result:?}"
    );
}

#[tokio::test]
async fn a_file_has_a_version_for_each_turn_that_changed_it() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let repository = repository(dir.path(), "app");
    let workspaces = Workspaces::new(dir.path().join("worktrees"));
    let checkouts = workspaces
        .create("s", std::slice::from_ref(&repository), &[])
        .await
        .expect("created");
    let work = checkouts[0].path.clone();
    let mut checkpoints = Vec::new();
    // Turn 0 has no SPEC.md; 1 writes it, 2 leaves it, 3 changes it.
    for (turn, text) in [None, Some("one\n"), None, Some("two\n")]
        .into_iter()
        .enumerate()
    {
        if let Some(text) = text {
            std::fs::write(work.join("SPEC.md"), text).expect("wrote");
        }
        let snapshot = workspaces
            .snapshot("s", turn, &checkouts)
            .await
            .expect("snapshotted");
        checkpoints.push(snapshot);
    }

    let turns = versions(&repository, &checkpoints, "SPEC.md")
        .await
        .expect("listed");
    assert_eq!(turns, [1, 3]);
    let first = file_at(&repository, &checkpoints, 1, "SPEC.md")
        .await
        .expect("read");
    assert_eq!(first, "one\n");
    let before = file_at(&repository, &checkpoints, 0, "SPEC.md").await;
    assert!(
        matches!(before, Err(WorkspaceError::NoSuchFileThen { turn: 0, .. })),
        "{before:?}"
    );
    let later = file_at(&repository, &checkpoints, 9, "SPEC.md").await;
    assert!(
        matches!(
            later,
            Err(WorkspaceError::UnknownTurn { turn: 9, turns: 3 })
        ),
        "{later:?}"
    );
}

mod land;
mod review;

#[tokio::test]
async fn snapshots_go_to_a_mirror_and_on_to_a_fresh_clone_a_session_starts_from() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let repository = repository(dir.path(), "app");
    let repositories = std::slice::from_ref(&repository);
    let workspaces = Workspaces::new(dir.path().join("worktrees"));
    let checkouts = workspaces
        .create("one", repositories, &[])
        .await
        .expect("created");
    let path = &checkouts[0].path;
    let mirror = dir.path().join("mirror.git");
    bundle::mirror(&mirror).await.expect("mirror made");
    assert_eq!(
        bundle::create(&repository.path, "one", &[]).await.ok(),
        Some(None)
    );

    std::fs::write(path.join("a.txt"), "first\n").expect("wrote");
    workspaces
        .snapshot("one", 0, &checkouts)
        .await
        .expect("snapshot");
    let haves = bundle::tips(&mirror).await.expect("tips");
    let first = bundle::create(&repository.path, "one", &haves)
        .await
        .expect("bundled")
        .expect("something new");
    bundle::apply(&mirror, &first).await.expect("applied");
    let haves = bundle::tips(&mirror).await.expect("tips");
    assert_eq!(
        bundle::create(&repository.path, "one", &haves).await.ok(),
        Some(None)
    );

    std::fs::write(path.join("a.txt"), "second\n").expect("wrote");
    let last = workspaces
        .snapshot("one", 1, &checkouts)
        .await
        .expect("snapshot");
    let second = bundle::create(&repository.path, "one", &haves)
        .await
        .expect("bundled")
        .expect("something new");
    assert!(second.len() < first.len());
    bundle::apply(&mirror, &second).await.expect("applied");

    // A fresh clone elsewhere takes the snapshots from the mirror, and a
    // session there starts from the last one.
    let clone = dir.path().join("elsewhere");
    let status = std::process::Command::new("git")
        .args(["clone", "--quiet"])
        .arg(&repository.path)
        .arg(&clone)
        .status()
        .expect("git runs");
    assert!(status.success());
    let haves = bundle::tips(&clone).await.expect("tips");
    let restoring = bundle::create(&mirror, "one", &haves)
        .await
        .expect("bundled")
        .expect("something new");
    bundle::apply(&clone, &restoring).await.expect("applied");
    let elsewhere = Workspaces::new(dir.path().join("elsewhere-worktrees"));
    let moved = Repository {
        name: "app".into(),
        path: clone,
    };
    let checkouts = elsewhere
        .create("one", &[moved], &last)
        .await
        .expect("created from the snapshot");
    assert_eq!(
        read(&checkouts[0].path.join("a.txt")).as_deref(),
        Some("second\n")
    );
}

#[tokio::test]
async fn a_copy_whose_folder_was_deleted_comes_back_from_its_snapshot() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let repository = repository(dir.path(), "app");
    let repositories = std::slice::from_ref(&repository);
    let workspaces = Workspaces::new(dir.path().join("worktrees"));
    let made = workspaces
        .create("s", repositories, &[])
        .await
        .expect("created");
    std::fs::write(made[0].path.join("README.md"), "saved\n").expect("wrote");
    let saved = workspaces
        .snapshot("s", 1, &made)
        .await
        .expect("snapshotted");

    // As when the cache folder is cleared: git still lists the worktree.
    std::fs::remove_dir_all(dir.path().join("worktrees")).expect("removed");
    let again = workspaces
        .create("s", repositories, &saved)
        .await
        .expect("created again");
    assert_eq!(
        read(&again[0].path.join("README.md")).as_deref(),
        Some("saved\n")
    );
}

#[tokio::test]
async fn making_a_missing_copy_again_keeps_the_edits_in_the_others() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let repositories = [repository(dir.path(), "app"), repository(dir.path(), "api")];
    let workspaces = Workspaces::new(dir.path().join("worktrees"));
    let made = workspaces
        .create("s", &repositories, &[])
        .await
        .expect("created");
    let saved = workspaces
        .snapshot("s", 1, &made)
        .await
        .expect("snapshotted");
    std::fs::write(made[0].path.join("README.md"), "since the snapshot\n").expect("wrote");

    std::fs::remove_dir_all(&made[1].path).expect("removed");
    let again = workspaces
        .create("s", &repositories, &saved)
        .await
        .expect("created again");
    assert_eq!(
        read(&again[0].path.join("README.md")).as_deref(),
        Some("since the snapshot\n")
    );
    assert!(again[1].path.join("README.md").is_file());
}

#[tokio::test]
async fn a_rewind_puts_the_files_back_and_undoing_it_brings_the_newer_ones_back() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let repository = repository(dir.path(), "app");
    let workspaces = Workspaces::new(dir.path().join("worktrees"));
    let checkouts = workspaces
        .create("session", std::slice::from_ref(&repository), &[])
        .await
        .expect("created");
    let work = &checkouts[0].path;
    std::fs::write(work.join("README.md"), "first turn\n").expect("wrote");
    let first = workspaces
        .snapshot("session", 1, &checkouts)
        .await
        .expect("snapshotted");
    std::fs::write(work.join("README.md"), "second turn\n").expect("wrote");
    std::fs::write(work.join("LATER.md"), "made later\n").expect("wrote");

    let rewound = workspaces
        .save_rewound("session", 1, &checkouts)
        .await
        .expect("saved");
    workspaces
        .restore(&checkouts, &first)
        .await
        .expect("rewound");
    assert_eq!(
        read(&work.join("README.md")).as_deref(),
        Some("first turn\n")
    );
    assert_eq!(read(&work.join("LATER.md")), None);

    workspaces
        .restore(&checkouts, &rewound)
        .await
        .expect("undone");
    assert_eq!(
        read(&work.join("README.md")).as_deref(),
        Some("second turn\n")
    );
    assert_eq!(
        read(&work.join("LATER.md")).as_deref(),
        Some("made later\n")
    );
}

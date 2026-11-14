use std::path::Path;

use super::*;

fn git_in(path: &Path, arguments: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(arguments)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@localhost")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@localhost")
        .output()
        .expect("git runs");
    assert!(output.status.success(), "git {arguments:?}");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// A session in two repositories, `app` and `docs`, whose person is set up.
async fn session(dir: &Path) -> (Vec<Repository>, Vec<Checkout>) {
    let repositories = vec![repository(dir, "app"), repository(dir, "docs")];
    for repository in &repositories {
        git_in(&repository.path, &["config", "user.name", "Pat"]);
        git_in(
            &repository.path,
            &["config", "user.email", "pat@example.com"],
        );
    }
    let workspaces = Workspaces::new(dir.join("worktrees"));
    let checkouts = workspaces
        .create("session-one", &repositories, &[])
        .await
        .expect("created");
    (repositories, checkouts)
}

#[tokio::test]
async fn landing_commits_each_changed_copy_on_the_branch_as_the_person() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let (_, checkouts) = session(dir.path()).await;
    std::fs::write(checkouts[0].path.join("README.md"), "changed\n").expect("wrote");
    std::fs::write(checkouts[0].path.join("new.txt"), "new\n").expect("wrote");

    let landed = land(&checkouts, "humata/fix-it", "Fix it")
        .await
        .expect("landed");

    let [app] = landed.as_slice() else {
        panic!("only the changed repository lands: {landed:?}");
    };
    assert_eq!(app.checkout.branch, "humata/fix-it");
    let commit = app.commit.clone().expect("a commit");
    assert_eq!(
        git_in(&app.checkout.path, &["rev-parse", "humata/fix-it"]),
        commit
    );
    assert_eq!(
        git_in(&app.checkout.path, &["log", "-1", "--format=%an <%ae>|%s"]),
        "Pat <pat@example.com>|Fix it"
    );
    assert_eq!(git_in(&app.checkout.path, &["status", "--porcelain"]), "");
    assert_eq!(
        git_in(&checkouts[1].path, &["rev-parse", "--abbrev-ref", "HEAD"]),
        checkouts[1].branch
    );
}

#[tokio::test]
async fn landing_again_adds_a_commit_only_when_there_is_more() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let (_, checkouts) = session(dir.path()).await;
    std::fs::write(checkouts[0].path.join("README.md"), "changed\n").expect("wrote");
    let landed = land(&checkouts, "humata/fix-it", "Fix it")
        .await
        .expect("landed");
    let mut moved = checkouts.clone();
    moved[0] = landed[0].checkout.clone();

    let again = land(&moved, "humata/fix-it", "Fix it")
        .await
        .expect("landed");
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].commit, None);

    std::fs::write(moved[0].path.join("README.md"), "more\n").expect("wrote");
    let more = land(&moved, "humata/fix-it", "Fix it")
        .await
        .expect("landed");
    let second = more[0].commit.clone().expect("another commit");
    assert_ne!(Some(second.clone()), landed[0].commit);
    assert_eq!(
        git_in(&moved[0].path, &["rev-parse", "HEAD~1"]),
        landed[0].commit.clone().expect("the first")
    );
}

#[tokio::test]
async fn a_bad_or_taken_branch_is_refused_and_nothing_changes() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let (repositories, checkouts) = session(dir.path()).await;
    std::fs::write(checkouts[0].path.join("README.md"), "changed\n").expect("wrote");

    let bad = land(&checkouts, "no..dots", "Fix it").await;
    assert!(
        matches!(bad, Err(WorkspaceError::BadBranch { .. })),
        "{bad:?}"
    );

    git_in(&repositories[0].path, &["branch", "theirs"]);
    let taken = land(&checkouts, "theirs", "Fix it").await;
    assert!(
        matches!(taken, Err(WorkspaceError::BranchTaken { .. })),
        "{taken:?}"
    );
    assert_eq!(
        git_in(&checkouts[0].path, &["status", "--porcelain"]),
        "M README.md"
    );
}

#[tokio::test]
async fn landing_without_a_person_set_up_is_refused() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let (repositories, checkouts) = session(dir.path()).await;
    git_in(&repositories[0].path, &["config", "--unset", "user.email"]);
    std::fs::write(checkouts[0].path.join("README.md"), "changed\n").expect("wrote");
    // Other tests' variables would stand in for the config.
    if std::env::var_os("GIT_AUTHOR_EMAIL").is_some() {
        return;
    }
    let landed = land(&checkouts, "humata/fix-it", "Fix it").await;
    assert!(
        matches!(landed, Err(WorkspaceError::NoIdentity { .. })),
        "{landed:?}"
    );
}

#[tokio::test]
async fn the_base_is_the_repositorys_branch_and_merging_it_brings_its_commits() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let (repositories, checkouts) = session(dir.path()).await;
    let app = &checkouts[0];
    assert_eq!(base(app).await.expect("a base"), "main");

    std::fs::write(repositories[0].path.join("later.txt"), "later\n").expect("wrote");
    git_in(&repositories[0].path, &["add", "later.txt"]);
    git_in(&repositories[0].path, &["commit", "--quiet", "-m", "later"]);
    std::fs::write(app.path.join("mine.txt"), "mine\n").expect("wrote");

    assert_eq!(merge(app, "main").await.expect("merged"), Merged::Merged);
    assert_eq!(
        std::fs::read_to_string(app.path.join("later.txt"))
            .ok()
            .as_deref(),
        Some("later\n")
    );
    // What the session hadn't committed is still there, uncommitted.
    assert_eq!(git_in(&app.path, &["status", "--porcelain"]), "?? mine.txt");
    assert_eq!(merge(app, "main").await.expect("merged"), Merged::UpToDate);
}

#[tokio::test]
async fn a_merge_that_conflicts_names_the_files() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let (repositories, checkouts) = session(dir.path()).await;
    let app = &checkouts[0];
    std::fs::write(repositories[0].path.join("README.md"), "theirs\n").expect("wrote");
    git_in(
        &repositories[0].path,
        &["commit", "--quiet", "-am", "theirs"],
    );
    std::fs::write(app.path.join("README.md"), "ours\n").expect("wrote");
    git_in(&app.path, &["commit", "--quiet", "-am", "ours"]);

    assert_eq!(
        merge(app, "main").await.expect("merged"),
        Merged::Conflicts(vec!["README.md".to_owned()])
    );
}

#[tokio::test]
async fn deleting_a_branch_twice_is_fine() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let (repositories, _) = session(dir.path()).await;
    git_in(&repositories[0].path, &["branch", "humata/done"]);

    delete_branch(&repositories[0].path, "humata/done")
        .await
        .expect("deleted");
    delete_branch(&repositories[0].path, "humata/done")
        .await
        .expect("already gone");
    assert_eq!(
        git_in(&repositories[0].path, &["branch", "--list", "humata/done"]),
        ""
    );
}

/// Makes every commit in `repository` fail until the hook is removed.
fn refuse_commits(repository: &Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let hook = repository.join(".git/hooks/pre-commit");
    std::fs::create_dir_all(hook.parent().expect("a hooks folder")).expect("made");
    std::fs::write(&hook, "#!/bin/sh\nexit 1\n").expect("wrote");
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).expect("set");
    hook
}

#[tokio::test]
async fn a_land_that_failed_part_way_lands_again_on_the_same_name() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let (repositories, checkouts) = session(dir.path()).await;
    std::fs::write(checkouts[0].path.join("README.md"), "changed\n").expect("wrote");
    std::fs::write(checkouts[1].path.join("README.md"), "changed\n").expect("wrote");
    let hook = refuse_commits(&repositories[1].path);

    land(&checkouts, "humata/fix-it", "Fix it")
        .await
        .expect_err("the hook refuses the commit");
    std::fs::remove_file(hook).expect("removed");

    let landed = land(&checkouts, "humata/fix-it", "Fix it")
        .await
        .expect("landed on the second try");
    assert_eq!(landed.len(), 2);
    for checkout in &checkouts {
        assert_eq!(
            branch_of(&checkout.path).await.as_deref(),
            Some("humata/fix-it")
        );
        assert_eq!(git_in(&checkout.path, &["status", "--porcelain"]), "");
    }
}

#[tokio::test]
async fn a_land_tried_again_under_a_new_name_takes_the_repository_committed_in_before() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let (repositories, checkouts) = session(dir.path()).await;
    std::fs::write(checkouts[0].path.join("README.md"), "changed\n").expect("wrote");
    std::fs::write(checkouts[1].path.join("README.md"), "changed\n").expect("wrote");
    // `app` is committed on the first name before `docs` fails.
    let hook = refuse_commits(&repositories[1].path);
    land(&checkouts, "humata/fix-it", "Fix it")
        .await
        .expect_err("the hook refuses the commit");
    std::fs::remove_file(hook).expect("removed");
    let committed = head(&checkouts[0].path);

    let landed = land(&checkouts, "humata/fixed", "Fix it")
        .await
        .expect("landed under the new name");

    let names: Vec<&str> = landed
        .iter()
        .map(|landed| landed.checkout.name.as_str())
        .collect();
    assert_eq!(names, ["app", "docs"]);
    assert_eq!(landed[0].commit, None);
    for checkout in &checkouts {
        assert_eq!(
            branch_of(&checkout.path).await.as_deref(),
            Some("humata/fixed")
        );
    }
    assert_eq!(head(&checkouts[0].path), committed);
}

/// A clone of a repository `remote` with one commit, and a session's copy
/// of the clone.
async fn copy_of_a_clone(dir: &Path) -> (Repository, Checkout) {
    let remote = repository(dir, "remote");
    git_in(
        dir,
        &["clone", "--quiet", &remote.path.to_string_lossy(), "app"],
    );
    let clone = Repository {
        name: "app".to_owned(),
        path: dir.join("app"),
    };
    let workspaces = Workspaces::new(dir.join("worktrees"));
    let mut checkouts = workspaces
        .create("session-one", std::slice::from_ref(&clone), &[])
        .await
        .expect("created");
    (remote, checkouts.remove(0))
}

fn commit_in(repository: &Path, file: &str, text: &str) -> String {
    std::fs::write(repository.join(file), text).expect("wrote");
    git_in(repository, &["add", file]);
    git_in(repository, &["commit", "--quiet", "-m", "more"]);
    git_in(repository, &["rev-parse", "HEAD"])
}

#[tokio::test]
async fn a_copy_without_changes_follows_its_origin_to_the_latest_commit() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let (remote, checkout) = copy_of_a_clone(dir.path()).await;
    let latest = commit_in(&remote.path, "new.txt", "new\n");

    let followed = follow(&checkout, std::time::Duration::from_secs(60))
        .await
        .expect("followed");

    assert_eq!(followed.commit, latest);
    assert_eq!(followed.fetch_failed, None);
    assert!(!followed.changed);
    assert_eq!(
        read(&checkout.path.join("new.txt")).as_deref(),
        Some("new\n")
    );
}

#[tokio::test]
async fn a_copy_with_changes_stays_where_it_is_and_keeps_them() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let (remote, checkout) = copy_of_a_clone(dir.path()).await;
    let before = head(&checkout.path);
    std::fs::write(checkout.path.join("README.md"), "mine\n").expect("wrote");
    commit_in(&remote.path, "new.txt", "new\n");

    let followed = follow(&checkout, std::time::Duration::from_secs(60))
        .await
        .expect("followed");

    assert!(followed.changed);
    assert_eq!(followed.commit, before);
    assert_eq!(
        read(&checkout.path.join("README.md")).as_deref(),
        Some("mine\n")
    );
}

#[tokio::test]
async fn a_copy_of_a_repository_with_no_origin_follows_the_branch_it_has_checked_out() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let app = repository(dir.path(), "app");
    let workspaces = Workspaces::new(dir.path().join("worktrees"));
    let checkouts = workspaces
        .create("session-one", std::slice::from_ref(&app), &[])
        .await
        .expect("created");
    let latest = commit_in(&app.path, "new.txt", "new\n");

    let followed = follow(&checkouts[0], std::time::Duration::from_secs(60))
        .await
        .expect("followed");

    assert_eq!(followed.commit, latest);
    assert_eq!(followed.fetch_failed, None);
}

#[tokio::test]
async fn a_fetch_that_hangs_is_stopped_once_its_patience_runs_out() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let (_, checkout) = copy_of_a_clone(dir.path()).await;
    let before = head(&checkout.path);
    // An origin whose transport never answers: the "ssh" sleeps, and its
    // arguments go into a comment.
    let clone = dir.path().join("app");
    git_in(
        &clone,
        &[
            "remote",
            "set-url",
            "origin",
            "ssh://humata.invalid/app.git",
        ],
    );
    git_in(&clone, &["config", "core.sshCommand", "exec sleep 30 #"]);
    git_in(&clone, &["config", "ssh.variant", "simple"]);

    let patience = std::time::Duration::from_secs(1);
    let started = std::time::Instant::now();
    let followed = follow(&checkout, patience).await.expect("followed");

    assert!(
        started.elapsed() < patience + std::time::Duration::from_secs(3),
        "the fetch went on for {:?}",
        started.elapsed()
    );
    assert_eq!(
        followed.fetch_failed,
        Some(FetchFailed::TimedOut { seconds: 1 })
    );
    assert_eq!(followed.commit, before);
}

#[tokio::test]
async fn a_fetch_from_an_origin_that_isnt_there_fails_with_gits_reason() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let (_, checkout) = copy_of_a_clone(dir.path()).await;
    let before = head(&checkout.path);
    let gone = dir.path().join("gone");
    git_in(
        &dir.path().join("app"),
        &["remote", "set-url", "origin", &gone.to_string_lossy()],
    );

    let followed = follow(&checkout, std::time::Duration::from_secs(60))
        .await
        .expect("followed");

    let Some(FetchFailed::Failed { stderr }) = &followed.fetch_failed else {
        panic!("not a failed fetch: {followed:?}");
    };
    assert!(!stderr.is_empty(), "git's reason is kept");
    assert_eq!(followed.commit, before);
}

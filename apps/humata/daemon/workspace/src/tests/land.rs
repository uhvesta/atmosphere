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

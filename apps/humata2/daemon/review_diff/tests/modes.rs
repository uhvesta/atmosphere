//! File modes and git's "no newline at end of file" marker reach the model.

mod support;

use std::os::unix::fs::PermissionsExt;

use humata2_review_diff::{LineKind, RepoSpec};
use support::{Repo, diff, hunks};

#[test]
fn a_mode_only_change_lists_both_modes_and_no_hunks() {
    let repo = Repo::new();
    repo.write("run.sh", "echo hi\n");
    repo.commit_all("base");
    let full = repo.dir().join("run.sh");
    std::fs::set_permissions(&full, std::fs::Permissions::from_mode(0o755))
        .expect("make it executable");

    let spec = RepoSpec::uncommitted("api");
    let shown = diff(&repo, &spec);
    let file = shown.file("run.sh").expect("the file is listed");
    assert_eq!((file.old_mode, file.new_mode), (100_644, 100_755));
    assert_eq!((file.added, file.removed), (0, 0));
    assert!(hunks(&repo, &shown, "run.sh", &spec).hunks.is_empty());
}

#[test]
fn an_added_file_has_no_old_mode() {
    let repo = Repo::new();
    repo.write("keep.txt", "one\n");
    repo.commit_all("base");
    repo.write("new.txt", "new\n");
    repo.git(&["add", "new.txt"]);

    let shown = diff(&repo, &RepoSpec::uncommitted("api"));
    let file = shown.file("new.txt").expect("the file is listed");
    assert_eq!((file.old_mode, file.new_mode), (0, 100_644));
}

#[test]
fn losing_the_final_newline_marks_the_right_line_on_the_right_side() {
    let repo = Repo::new();
    repo.write("a.txt", "one\ntwo\n");
    repo.commit_all("base");
    repo.write("a.txt", "one\ntwo");

    let spec = RepoSpec::uncommitted("api");
    let shown = diff(&repo, &spec);
    let chunk = hunks(&repo, &shown, "a.txt", &spec);
    let lines: Vec<_> = chunk.hunks.iter().flat_map(|hunk| &hunk.lines).collect();
    let marked: Vec<_> = lines
        .iter()
        .filter(|line| line.no_newline)
        .map(|line| (line.kind, line.text.as_str()))
        .collect();
    assert_eq!(marked, [(LineKind::Added, "two")]);
    assert!(lines.iter().any(|line| line.kind == LineKind::Removed && !line.no_newline));
}

//! An untracked file that is deleted between git listing it and the snapshot
//! taking it is left out of the diff; the diff still computes.

mod support;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use humata2_review_diff::{Base, Checkout, Compare, RepoSpec, Untracked, Upto};
use support::{Repo, context, paths};

/// A git stand-in: when its arguments list untracked files (`--others`) it
/// runs git, then deletes `doomed`; any other call runs git unchanged.
fn git_that_loses_a_file(folder: &Path, doomed: &Path) -> PathBuf {
    let script = folder.join("git-that-loses-a-file");
    let body = format!(
        "#!/bin/sh\n\
         case \" $* \" in\n\
         *\" --others \"*)\n\
         git \"$@\" || exit $?\n\
         rm -f '{doomed}'\n\
         ;;\n\
         *) exec git \"$@\" ;;\n\
         esac\n",
        doomed = doomed.display(),
    );
    std::fs::write(&script, body).expect("write the stand-in");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
        .expect("make the stand-in runnable");
    script
}

fn compare(upto: Upto, untracked: Untracked) -> Compare {
    Compare {
        upto,
        untracked,
        ..Compare::commits()
    }
}

#[test]
fn an_untracked_file_deleted_mid_snapshot_is_left_out() {
    let repo = Repo::new();
    repo.write("tracked.txt", "one\n");
    repo.commit_all("base");
    repo.write("keep.txt", "kept\n");
    repo.write("doomed.txt", "gone soon\n");

    let folder = tempfile::tempdir().expect("make a folder for the stand-in");
    let script = git_that_loses_a_file(folder.path(), &repo.dir().join("doomed.txt"));
    let checkout =
        Checkout::open_with_git(repo.dir(), script).expect("open the repo through the stand-in");

    let spec = RepoSpec::new(
        "api",
        Base::Rev {
            rev: "HEAD".to_owned(),
        },
        compare(Upto::Unstaged, Untracked::All),
    );
    let shown = checkout
        .diff_blocking(&spec, &context())
        .expect("the diff still computes");
    assert_eq!(paths(&shown), ["keep.txt"]);
}

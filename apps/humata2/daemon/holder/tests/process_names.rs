//! A holder is found in the process list by its program's name, however
//! long that name is. Linux's `ps` cuts a name to its first 15 bytes, and
//! the test build's name is longer, so a program whose cut name only looks
//! like a holder's must not count as one.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use humata2_holder::process::{Snapshot, holder_names};
use tempfile::TempDir;

/// A copy of `sleep` under the file name `name`, running until the test
/// ends.
struct Named {
    child: Child,
    _dir: TempDir,
}

impl Named {
    fn start(name: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let program: PathBuf = dir.path().join(name);
        std::fs::copy("/bin/sleep", &program).unwrap();
        let child = Command::new(&program).arg("30").spawn().unwrap();
        Self { child, _dir: dir }
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for Named {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[tokio::test]
async fn a_program_with_the_test_builds_name_counts_as_a_holder() {
    let program = Named::start("humata2_holder_faults");

    let snapshot = Snapshot::take().await.unwrap();

    assert!(snapshot.is_holder(program.pid(), &holder_names(Path::new(""))));
}

#[tokio::test]
async fn a_program_whose_name_only_starts_like_a_holders_is_not_one() {
    let program = Named::start("humata2_holder_other");

    let snapshot = Snapshot::take().await.unwrap();

    assert!(snapshot.is_alive(program.pid()));
    assert!(!snapshot.is_holder(program.pid(), &holder_names(Path::new(""))));
}

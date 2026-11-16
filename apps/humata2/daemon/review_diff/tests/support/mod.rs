//! A real git repository in a temporary folder.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

use humata2_review_diff::{
    BaseContext, Budget, Checkout, Cursor, FileChunk, LineKind, Matching, RepoDiff, RepoSpec,
};

pub struct Repo {
    _dir: tempfile::TempDir,
    pub path: PathBuf,
}

pub const SESSION: &str = "s-1";

impl Repo {
    /// An empty repository on branch `main`.
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("make a temporary folder");
        let path = dir.path().join("repo");
        std::fs::create_dir(&path).expect("make the repo folder");
        let repo = Self { _dir: dir, path };
        repo.git(&["init", "-q", "-b", "main"]);
        repo.git(&["config", "user.name", "Test"]);
        repo.git(&["config", "user.email", "test@example.com"]);
        repo.git(&["config", "commit.gpgsign", "false"]);
        repo
    }

    pub fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(&self.path)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", &self.path)
            .args(args)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    pub fn write(&self, path: &str, text: &str) {
        let full = self.path.join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("make folders");
        }
        std::fs::write(full, text).expect("write a file");
    }

    pub fn remove(&self, path: &str) {
        std::fs::remove_file(self.path.join(path)).expect("remove a file");
    }

    pub fn commit_all(&self, message: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "-m", message]);
        self.git(&["rev-parse", "HEAD"])
    }

    pub fn checkout(&self) -> Checkout {
        Checkout::open(&self.path).expect("open the repo")
    }

    pub fn index_bytes(&self) -> Vec<u8> {
        std::fs::read(self.path.join(".git/index")).unwrap_or_default()
    }

    /// The status, read without refreshing the index (`git status` would
    /// otherwise rewrite it when cached stats are stale).
    pub fn status(&self) -> String {
        self.git(&["--no-optional-locks", "status", "--porcelain=v1", "-uall"])
    }

    /// Gives each of `paths` a new mtime, a minute on from its own, without
    /// touching its bytes, so the index's cached stat for it no longer
    /// matches and a refresh would rewrite the index.
    pub fn make_index_stale(&self, paths: &[&str]) {
        for path in paths {
            let file = std::fs::File::options()
                .write(true)
                .open(self.path.join(path))
                .expect("open a tracked file");
            let modified = file
                .metadata()
                .and_then(|meta| meta.modified())
                .expect("read its mtime");
            file.set_modified(modified + std::time::Duration::from_secs(61))
                .expect("set its mtime");
        }
    }

    pub fn dir(&self) -> &Path {
        &self.path
    }
}

pub fn context() -> BaseContext {
    BaseContext {
        session: SESSION.to_owned(),
        fork_point: None,
    }
}

pub fn diff(repo: &Repo, spec: &RepoSpec) -> RepoDiff {
    repo.checkout()
        .diff_blocking(spec, &context())
        .expect("compute the diff")
}

/// The paths a diff shows, sorted.
pub fn paths(diff: &RepoDiff) -> Vec<String> {
    let mut paths: Vec<String> = diff.files.iter().map(|file| file.path.clone()).collect();
    paths.sort();
    paths
}

/// A file's whole diff in one chunk.
pub fn hunks(repo: &Repo, diff: &RepoDiff, path: &str, spec: &RepoSpec) -> FileChunk {
    let file = diff.file(path).expect("the file is in the diff");
    repo.checkout()
        .file_chunk_blocking(
            &diff.trees,
            file,
            Matching::from(spec),
            Cursor::default(),
            Budget {
                max_lines: usize::MAX,
                max_bytes: usize::MAX,
            },
        )
        .expect("read the file's hunks")
}

/// The added lines' text of a chunk.
pub fn added(chunk: &FileChunk) -> Vec<String> {
    chunk
        .hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .filter(|line| line.kind == LineKind::Added)
        .map(|line| line.text.clone())
        .collect()
}

pub fn removed(chunk: &FileChunk) -> Vec<String> {
    chunk
        .hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .filter(|line| line.kind == LineKind::Removed)
        .map(|line| line.text.clone())
        .collect()
}

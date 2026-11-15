//! D66's measurement: the git CLI on a big synthetic repository. Prints
//! timings and asserts nothing about them; run it on a release build
//! (`bazel test -c opt //apps/humata2/daemon/review_diff:big_repo_bench
//! --test_output=streamed --test_arg=--nocapture`).

use std::process::Command;
use std::time::{Duration, Instant};

use humata2_review_diff::{BaseContext, Budget, Checkout, Cursor, Matching, RepoSpec};

const FILES: usize = 20_000;
const EDITED: usize = 200;
const BIG_FILE_LINES: usize = 200_000;
const RUNS: usize = 5;

fn git(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", dir)
        .args(args)
        .status()
        .expect("run git");
    assert!(status.success(), "git {args:?} failed");
}

fn median(mut runs: Vec<Duration>) -> Duration {
    runs.sort();
    runs[runs.len() / 2]
}

fn time<T>(what: &str, mut work: impl FnMut() -> T) -> T {
    let mut runs = Vec::new();
    let mut last = None;
    for _ in 0..RUNS {
        let start = Instant::now();
        last = Some(work());
        runs.push(start.elapsed());
    }
    println!("{what}: median {:?} of {RUNS}", median(runs));
    last.expect("ran at least once")
}

#[test]
fn big_repo() {
    let dir = tempfile::tempdir().expect("temporary folder");
    let root = dir.path();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["config", "user.name", "Bench"]);
    git(root, &["config", "user.email", "bench@example.com"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    for n in 0..FILES {
        let path = root.join(format!("src/m{}/f{n}.rs", n % 100));
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("folders");
        let body: String = (0..40).map(|line| format!("// file {n} line {line}\n")).collect();
        std::fs::write(path, body).expect("write");
    }
    let big: String = (0..BIG_FILE_LINES).map(|line| format!("row {line}\n")).collect();
    std::fs::write(root.join("big.txt"), &big).expect("write");
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "base"]);
    for n in (0..FILES).step_by(FILES / EDITED) {
        let path = root.join(format!("src/m{}/f{n}.rs", n % 100));
        let mut body = std::fs::read_to_string(&path).expect("read");
        body.push_str("// edited\n");
        std::fs::write(path, body).expect("write");
    }
    std::fs::write(root.join("big.txt"), big.replace("row 1", "ROW 1")).expect("write");
    std::fs::write(root.join("untracked.rs"), "new\n").expect("write");

    let checkout = Checkout::open(root).expect("open");
    let spec = RepoSpec::uncommitted("bench");
    let context = BaseContext {
        session: "bench".to_owned(),
        fork_point: None,
    };
    println!("{FILES} files, {EDITED} edited, big.txt {BIG_FILE_LINES} lines");
    let shown = time("file list (uncommitted)", || {
        checkout.diff_blocking(&spec, &context).expect("diff")
    });
    println!("  {} files in the diff", shown.files.len());
    let big_file = shown.file("big.txt").expect("big.txt changed").clone();
    time("first chunk of big.txt (default budget)", || {
        checkout
            .file_chunk_blocking(
                &shown.trees,
                &big_file,
                Matching::from(&spec),
                Cursor::default(),
                Budget::default(),
            )
            .expect("chunk")
    });
    let small = shown.file("src/m0/f0.rs").expect("an edited file").clone();
    time("one small file's hunks", || {
        checkout
            .file_chunk_blocking(
                &shown.trees,
                &small,
                Matching::from(&spec),
                Cursor::default(),
                Budget::default(),
            )
            .expect("chunk")
    });
    time("mark reviewed", || {
        checkout
            .mark_reviewed_blocking("bench", &shown.trees.new)
            .expect("snapshot")
    });
    time("file list since the last review", || {
        checkout
            .diff_blocking(&RepoSpec::since_last_review("bench"), &context)
            .expect("diff")
    });
}

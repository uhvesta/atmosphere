//! Runs the real humatad and humata binaries, as a person does.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

pub fn binary(variable: &str) -> PathBuf {
    let path =
        std::env::var_os(variable).unwrap_or_else(|| panic!("set {variable} to the binary's path"));
    std::fs::canonicalize(&path).unwrap_or_else(|error| panic!("{variable}: {error}"))
}

/// Stops a process when the test ends, passed or not.
pub struct Running(pub Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub struct Humata {
    cli: PathBuf,
    socket: PathBuf,
}

impl Humata {
    pub fn run(&self, arguments: &[&str]) -> String {
        let output = Command::new(&self.cli)
            .arg("--socket")
            .arg(&self.socket)
            .args(arguments)
            .output()
            .unwrap_or_else(|error| panic!("couldn't run humata: {error}"));
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(
            output.status.success(),
            "humata {arguments:?} failed: {}\n{stdout}",
            String::from_utf8_lossy(&output.stderr)
        );
        stdout
    }

    /// The (kind, detail) of each line `humata` printed for events.
    pub fn said(&self, arguments: &[&str]) -> Vec<(String, String)> {
        self.run(arguments)
            .lines()
            .filter_map(|line| {
                let mut fields = line.splitn(3, '\t');
                let (_, kind, detail) = (fields.next()?, fields.next()?, fields.next()?);
                matches!(kind, "user" | "assistant").then(|| (kind.to_owned(), detail.to_owned()))
            })
            .collect()
    }
}

pub fn wait_for(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "{} never appeared",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Makes `path` a git repository with `files` in its first commit.
#[allow(dead_code, reason = "not every test works in a repository")]
pub fn git_repository(path: &Path, files: &[(&str, &str)]) {
    std::fs::create_dir_all(path).unwrap_or_else(|error| panic!("{error}"));
    for (name, contents) in files {
        std::fs::write(path.join(name), contents).unwrap_or_else(|error| panic!("{error}"));
    }
    for arguments in [
        &["init", "--quiet", "--initial-branch=main"][..],
        &["add", "--all"],
        &["commit", "--quiet", "-m", "first"],
    ] {
        let status = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(arguments)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@localhost")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@localhost")
            .status()
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(status.success(), "git {arguments:?}");
    }
}

/// Whether the repository at `path` has nothing uncommitted.
#[allow(dead_code, reason = "not every test works in a repository")]
pub fn clean(path: &Path) -> bool {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["status", "--porcelain"])
        .output()
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(output.status.success());
    output.stdout.is_empty()
}

impl Humata {
    /// The folder `session` works in, as `humata sessions` prints it.
    #[allow(dead_code, reason = "not every test works in a repository")]
    pub fn working_directory(&self, session: &str) -> PathBuf {
        let sessions = self.run(&["sessions"]);
        let line = sessions
            .lines()
            .find(|line| line.starts_with(session))
            .unwrap_or_else(|| panic!("{session} isn't in {sessions}"));
        let directory = line.split('\t').nth(9).unwrap_or_else(|| panic!("{line}"));
        PathBuf::from(directory)
    }
}

pub fn pairs(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(kind, text)| ((*kind).to_owned(), (*text).to_owned()))
        .collect()
}

/// A scratch folder short enough for a socket path, which is limited to
/// about 100 bytes, so not under Bazel's long test folder.
pub fn scratch() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("humata")
        .tempdir_in("/tmp")
        .unwrap_or_else(|error| panic!("{error}"))
}

/// Starts humatad with its state and socket in `root`, and `arguments`
/// after them; it stops when the first value is dropped.
pub fn start(root: &Path, arguments: &[&std::ffi::OsStr]) -> (Running, Humata) {
    let socket = root.join("d.sock");
    let daemon = Command::new(binary("HUMATAD"))
        .args(["daemon", "--state"])
        .arg(root.join("daemon"))
        .arg("--cache")
        .arg(root.join("cache"))
        .arg("--socket")
        .arg(&socket)
        .args(arguments)
        .spawn()
        .unwrap_or_else(|error| panic!("couldn't start humatad: {error}"));
    let daemon = Running(daemon);
    wait_for(&socket);
    let humata = Humata {
        cli: binary("HUMATA"),
        socket,
    };
    (daemon, humata)
}

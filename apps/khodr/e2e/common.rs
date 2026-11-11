//! Runs the real khodrd and khodr binaries, as a person does.

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

pub struct Khodr {
    cli: PathBuf,
    socket: PathBuf,
}

impl Khodr {
    pub fn run(&self, arguments: &[&str]) -> String {
        let output = Command::new(&self.cli)
            .arg("--socket")
            .arg(&self.socket)
            .args(arguments)
            .output()
            .unwrap_or_else(|error| panic!("couldn't run khodr: {error}"));
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(
            output.status.success(),
            "khodr {arguments:?} failed: {}\n{stdout}",
            String::from_utf8_lossy(&output.stderr)
        );
        stdout
    }

    /// The (kind, detail) of each line `khodr` printed for events.
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
        .prefix("khodr")
        .tempdir_in("/tmp")
        .unwrap_or_else(|error| panic!("{error}"))
}

/// Starts khodrd with its state and socket in `root`, and `arguments`
/// after them; it stops when the first value is dropped.
pub fn start(root: &Path, arguments: &[&std::ffi::OsStr]) -> (Running, Khodr) {
    let socket = root.join("d.sock");
    let daemon = Command::new(binary("KHODRD"))
        .args(["daemon", "--state"])
        .arg(root.join("daemon"))
        .arg("--socket")
        .arg(&socket)
        .args(arguments)
        .spawn()
        .unwrap_or_else(|error| panic!("couldn't start khodrd: {error}"));
    let daemon = Running(daemon);
    wait_for(&socket);
    let khodr = Khodr {
        cli: binary("KHODR"),
        socket,
    };
    (daemon, khodr)
}

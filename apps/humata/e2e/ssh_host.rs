//! A host reached over SSH to the test's own sshd, and cutting it off.

#![allow(dead_code)]

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::common::{self, Humata, Running};

/// Stops the `ssh` processes that reach humatad in `folder`, as a dropped
/// network would, and says how many there were.
pub fn cut_ssh(folder: &Path) -> usize {
    let pattern = format!("{}/bin/humatad", folder.display());
    let output = Command::new("pgrep")
        .args(["-f", &pattern])
        .output()
        .unwrap_or_else(|error| panic!("{error}"));
    let mut cut = 0;
    for pid in String::from_utf8_lossy(&output.stdout).split_whitespace() {
        let name = Command::new("ps")
            .args(["-o", "comm=", "-p", pid])
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .unwrap_or_default();
        if name.ends_with("ssh") {
            let status = Command::new("kill").arg(pid).status();
            assert!(status.is_ok_and(|status| status.success()));
            cut += 1;
        }
    }
    cut
}

/// Stops the host humatad in `folder` runs on its socket, as a crash or a
/// reboot would, and says whether one was running.
pub fn stop_host(folder: &Path) -> bool {
    let pattern = format!("{}/bin/humatad host --socket", folder.display());
    Command::new("pkill")
        .args(["-KILL", "-f", &pattern])
        .status()
        .is_ok_and(|status| status.success())
}

/// Stops the host in its folder when the test ends, since it outlives the
/// connections to it.
pub struct DetachedHost<'a>(pub &'a Path);

impl Drop for DetachedHost<'_> {
    fn drop(&mut self) {
        stop_host(self.0);
    }
}

/// The test's sshd, with its client config.
pub struct Sshd {
    pub running: Running,
    pub config: PathBuf,
}

pub fn sshd(root: &Path) -> Sshd {
    let keys = root.join("ssh");
    std::fs::create_dir_all(&keys).unwrap_or_else(|error| panic!("{error}"));
    let running = Command::new(common::binary("TEST_SSHD"))
        .arg(&keys)
        .spawn()
        .unwrap_or_else(|error| panic!("couldn't start test_sshd: {error}"));
    let running = Running(running);
    let config = keys.join("ssh_config");
    common::wait_for(&config);
    Sshd { running, config }
}

/// A daemon with host `name`, in `folder` over `sshd`, added and installed.
pub fn daemon_with_host(root: &Path, sshd: &Sshd, name: &str, folder: &Path) -> (Running, Humata) {
    let hosts_file = root.join("hosts.json");
    let (daemon, humata) = common::start(root, &[OsStr::new("--hosts"), hosts_file.as_os_str()]);
    humata.run(&[
        "host",
        "add",
        name,
        "humata-test",
        "--ssh-config",
        &sshd.config.display().to_string(),
        "--folder",
        &folder.display().to_string(),
    ]);
    humata.run(&["host", "install", name]);
    (daemon, humata)
}

/// Runs `humata` and says whether it succeeded, for commands that should fail.
pub fn succeeds(humata: &Humata, arguments: &[&str]) -> bool {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| humata.run(arguments))).is_ok()
}

/// Asks `check` every so often until it is true.
pub fn eventually(what: &str, mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !check() {
        assert!(Instant::now() < deadline, "{what} never happened");
        std::thread::sleep(Duration::from_millis(100));
    }
}

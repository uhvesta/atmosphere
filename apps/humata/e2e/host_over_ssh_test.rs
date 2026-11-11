//! A host Humata manages over SSH, as a person sets one up: added by its SSH
//! destination, humatad installed and set up there, a project's repository
//! cloned there for a session, a connection dropped mid-turn, and the host
//! going away.

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use common::{Humata, pairs};

mod common;

const SCRIPT: &str = r#"{"rules": [
    {"when": "hold", "steps": [{"step": "wait_for", "path": "never"}]},
    {"when": "hello", "steps": [{"step": "say", "text": "hi there"}]}
]}"#;

/// Runs `humata` and says whether it succeeded, for commands that should fail.
fn succeeds(humata: &Humata, arguments: &[&str]) -> bool {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| humata.run(arguments))).is_ok()
}

/// Asks `check` every so often until it is true.
fn eventually(what: &str, mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !check() {
        assert!(Instant::now() < deadline, "{what} never happened");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn session_line(humata: &Humata, session: &str) -> String {
    humata
        .run(&["sessions"])
        .lines()
        .find(|line| line.starts_with(session))
        .map(str::to_owned)
        .unwrap_or_default()
}

/// Stops the `ssh` processes that reach humatad in `folder`, as a dropped
/// network would, and says how many there were.
fn cut_ssh(folder: &Path) -> usize {
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

fn git(path: &Path, arguments: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(arguments)
        .status()
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(status.success(), "git {arguments:?}");
}

#[test]
fn a_managed_host_is_installed_set_up_and_survives_dropped_connections() {
    let scratch = common::scratch();
    let root = scratch.path();
    let keys = root.join("ssh");
    std::fs::create_dir_all(&keys).unwrap_or_else(|error| panic!("{error}"));
    let sshd = Command::new(common::binary("TEST_SSHD"))
        .arg(&keys)
        .spawn()
        .unwrap_or_else(|error| panic!("couldn't start test_sshd: {error}"));
    let sshd = common::Running(sshd);
    let config = keys.join("ssh_config");
    common::wait_for(&config);

    // A repository with an origin the host can clone from.
    let code = root.join("code");
    common::git_repository(&code, &[("README.md", "hello\n")]);
    let origin = root.join("origin.git");
    let status = Command::new("git")
        .args(["clone", "--quiet", "--bare"])
        .arg(&code)
        .arg(&origin)
        .status()
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(status.success());
    git(
        &code,
        &["remote", "add", "origin", &origin.display().to_string()],
    );

    let hosts_file = root.join("hosts.json");
    let (_daemon, humata) = common::start(root, &[OsStr::new("--hosts"), hosts_file.as_os_str()]);
    let folder = root.join("far");
    let folder_text = folder.display().to_string();
    humata.run(&[
        "host",
        "add",
        "far",
        "humata-test",
        "--ssh-config",
        &config.display().to_string(),
        "--folder",
        &folder_text,
    ]);
    assert!(
        humata
            .run(&["host", "list"])
            .starts_with("far\thumata-test")
    );

    // Not installed yet: unreachable, and saying so.
    assert!(!succeeds(&humata, &["host", "check", "far"]));
    assert!(!humata.run(&["host", "install", "far"]).trim().is_empty());
    assert!(folder.join("bin/humatad").exists());

    // Its agents' variables reach the setup script without being written
    // on the host.
    let ran = root.join("setup-ran");
    let script = root.join("setup.sh");
    std::fs::write(
        &script,
        format!("echo \"$GREETING\" > '{}'\n", ran.display()),
    )
    .unwrap_or_else(|error| panic!("{error}"));
    humata.run(&[
        "host",
        "env",
        "far",
        "--set",
        "GREETING=hi",
        "--set",
        "GH_TOKEN=secret",
    ]);
    let saved = humata.run(&["host", "setup", "far", &script.display().to_string()]);
    assert!(
        saved.contains("GH_TOKEN") && !saved.contains("secret"),
        "{saved}"
    );
    let health = humata.run(&["host", "check", "far"]);
    assert!(
        health.contains("reachable") && health.contains("setup ok"),
        "{health}"
    );
    assert!(health.contains("\ttoken"), "{health}");
    assert_eq!(std::fs::read_to_string(&ran).ok().as_deref(), Some("hi\n"));
    let on_host = std::fs::read_to_string(folder.join("state/setup.json")).unwrap_or_default();
    assert!(!on_host.contains("secret"));

    // A project's session on the host works in a clone made there.
    let project = humata.run(&[
        "project",
        "add",
        "--name",
        "code",
        "--repository",
        &code.display().to_string(),
    ]);
    let project = project.trim();
    let script = root.join("script.json");
    std::fs::write(&script, SCRIPT).unwrap_or_else(|error| panic!("{error}"));
    let session = humata.run(&[
        "start",
        "--backend",
        "scripted",
        "--project",
        project,
        "--host",
        "far",
        "--approval",
        "everything",
        "--backend-config",
        &script.display().to_string(),
    ]);
    let session = session.trim();
    assert!(
        folder
            .join("state/repos")
            .join(project)
            .join("code/README.md")
            .exists()
    );
    assert_eq!(
        humata.said(&["send", session, "hello", "--wait"]),
        pairs(&[("user", "hello"), ("assistant", "hi there")])
    );

    // A turn cut off by a dropped connection ends aborted, and the next one
    // runs once the daemon reconnects.
    humata.run(&["send", session, "hold"]);
    eventually("the turn working", || {
        session_line(&humata, session).contains("WORKING")
    });
    assert!(cut_ssh(&folder) > 0, "no ssh reached the host");
    eventually("the cut-off turn closed", || {
        succeeds(&humata, &["history", session])
            && humata
                .run(&["history", session])
                .contains("turn_ended\taborted")
    });
    assert_eq!(
        humata.said(&["send", session, "hello", "--wait"]),
        pairs(&[("user", "hello"), ("assistant", "hi there")])
    );
    let history = humata.run(&["history", session]);

    // With the host gone, its sessions still show, as they last were.
    drop(sshd);
    let _ = cut_ssh(&folder);
    eventually("the session shown offline", || {
        session_line(&humata, session).contains("offline")
    });
    assert_eq!(humata.run(&["history", session]), history);
}

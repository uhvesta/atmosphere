//! A host Humata manages over SSH, as a person sets one up: added by its SSH
//! destination, humatad installed and set up there, a project's repository
//! cloned there for a session, a connection dropped mid-turn, the host
//! stopping mid-turn, the host losing its sessions and getting them back,
//! and the host going away, its sessions moved to this Mac.

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

use common::{Humata, pairs};
use ssh_host::{DetachedHost, cut_ssh, eventually, stop_host, succeeds};

mod common;
mod ssh_host;

const SCRIPT: &str = r#"{"rules": [
    {"when": "hold", "steps": [{"step": "wait_for", "path": "never"}]},
    {"when": "wait", "steps": [
        {"step": "wait_for", "path": "go"},
        {"step": "say", "text": "went"}
    ]},
    {"when": "hello", "steps": [{"step": "say", "text": "hi there"}]}
]}"#;

fn session_line(humata: &Humata, session: &str) -> String {
    humata
        .run(&["sessions"])
        .lines()
        .find(|line| line.starts_with(session))
        .map(str::to_owned)
        .unwrap_or_default()
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
    let _host = DetachedHost(&folder);
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

    // A turn going when the connection drops keeps going on the host, and a
    // watch from before the drop sees it through once the daemon reconnects.
    let mut waiting = humata.spawn(&["send", session, "wait", "--wait"]);
    eventually("the turn working", || {
        session_line(&humata, session).contains("WORKING")
    });
    assert!(cut_ssh(&folder) > 0, "no ssh reached the host");
    eventually("the turn still working after reconnecting", || {
        session_line(&humata, session).contains("WORKING")
    });
    let go = humata.working_directory(session).join("go");
    std::fs::write(&go, "").unwrap_or_else(|error| panic!("{error}"));
    eventually("the watch from before the drop finished", || {
        waiting.0.try_wait().ok().flatten().is_some()
    });
    let mut seen = String::new();
    if let Some(mut stdout) = waiting.0.stdout.take() {
        std::io::Read::read_to_string(&mut stdout, &mut seen)
            .unwrap_or_else(|error| panic!("{error}"));
    }
    assert!(seen.contains("assistant\twent"), "{seen}");

    // A host that stops mid-turn, as in a crash, is started again by the
    // next connection, with the cut-off turn ended aborted so the session
    // takes messages again.
    humata.run(&["send", session, "hold"]);
    eventually("the held turn working", || {
        session_line(&humata, session).contains("WORKING")
    });
    assert!(stop_host(&folder), "no host was running on its socket");
    let _ = cut_ssh(&folder);
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
    // A turn only watched, never read as history, is kept on the Mac too.
    assert_eq!(
        humata.said(&["send", session, "hello again", "--wait"]),
        pairs(&[("user", "hello again"), ("assistant", "hi there")])
    );

    // A host that loses its sessions, as a machine wiped or replaced does,
    // gets them back from the Mac's copy of each, with their code and
    // history, and they carry on.
    eventually("the recovery copy up to date", || {
        recovery_line(&humata, session).is_some_and(|line| {
            let turns = line.split('\t').nth(3).unwrap_or_default();
            turns
                .trim_end_matches(" turns")
                .split_once('/')
                .is_some_and(|(kept, all)| kept == all && kept != "0")
        })
    });
    let before = humata.said(&["history", session]);
    assert!(stop_host(&folder), "no host was running on its socket");
    let _ = cut_ssh(&folder);
    for lost in ["state", "cache"] {
        std::fs::remove_dir_all(folder.join(lost)).unwrap_or_else(|error| panic!("{error}"));
    }
    eventually("the session lost", || {
        recovery_line(&humata, session).is_some_and(|line| line.ends_with("\tlost"))
    });
    humata.run(&["recover", "host", "far"]);
    assert!(session_line(&humata, session).contains("\tfar\t"));
    assert!(humata.working_directory(session).join("go").is_file());
    assert_eq!(humata.said(&["history", session]), before);
    assert_eq!(
        humata.said(&["send", session, "hello", "--wait"]),
        pairs(&[("user", "hello"), ("assistant", "hi there")])
    );
    eventually("the recovery copy of the restored session", || {
        recovery_line(&humata, session).is_some_and(|line| line.ends_with("\ton host"))
    });

    // A host that hangs, as one swapping or on a dying link does, doesn't
    // hold up the list: its sessions show as they last were, offline, and
    // come back once it answers again.
    assert!(
        pause_host(&folder, true),
        "no host was running on its socket"
    );
    let asked = std::time::Instant::now();
    let line = session_line(&humata, session);
    let waited = asked.elapsed();
    assert!(pause_host(&folder, false));
    assert!(waited < std::time::Duration::from_secs(10), "{waited:?}");
    assert!(line.contains("offline"), "{line}");
    eventually("the session back online", || {
        let line = session_line(&humata, session);
        !line.is_empty() && !line.contains("offline")
    });

    // With the host gone, its sessions still show, as they last were, and
    // watching one plays its history and ends.
    drop(sshd);
    let _ = cut_ssh(&folder);
    stop_host(&folder);
    eventually("the session shown offline", || {
        session_line(&humata, session).contains("offline")
    });
    let offline = humata.run(&["history", session]);
    assert!(offline.starts_with(&history), "{offline}");
    assert!(offline.contains("hello again"), "{offline}");
    assert_eq!(humata.run(&["watch", session]), offline);

    // Moved to this Mac from its copy, it carries on here, and isn't shown
    // on the host it left.
    let moved = humata.run(&["recover", "session", session, "--host", ""]);
    assert!(moved.starts_with(session), "{moved}");
    let line = session_line(&humata, session);
    assert!(!line.contains("far") && !line.contains("offline"), "{line}");
    assert!(humata.working_directory(session).join("go").is_file());
    assert_eq!(
        humata.said(&["send", session, "hello", "--wait"]),
        pairs(&[("user", "hello"), ("assistant", "hi there")])
    );
    assert_eq!(
        humata
            .run(&["sessions"])
            .lines()
            .filter(|line| line.starts_with(session))
            .count(),
        1
    );
}

/// The line `humata recover list` prints for `session`'s copy.
fn recovery_line(humata: &Humata, session: &str) -> Option<String> {
    humata
        .run(&["recover", "list"])
        .lines()
        .find(|line| line.starts_with(session))
        .map(str::to_owned)
}

/// Stops or resumes the host humatad in `folder` runs on its socket, as a
/// machine that hangs would, and says whether one was running.
fn pause_host(folder: &Path, paused: bool) -> bool {
    let pattern = format!("{}/bin/humatad host --socket", folder.display());
    Command::new("pkill")
        .args([if paused { "-STOP" } else { "-CONT" }, "-f", &pattern])
        .status()
        .is_ok_and(|status| status.success())
}

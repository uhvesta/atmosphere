//! A short folder under /tmp for a test, which records the test process
//! that made it. A test killed outright (a Bazel timeout, an interrupted
//! run) never runs its drops, so the holders it started, which outlive
//! their daemon on purpose, keep running in a folder nobody removes.
//! Making a folder first ends the holders of every folder with the same
//! prefix whose test process is gone, and removes that folder.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use rustix::process::Pid;
use tempfile::TempDir;

/// The file in a folder naming the test process that made it.
const OWNER_FILE: &str = ".test-owner";

/// A fresh folder under /tmp named `prefix…`, only its user can open,
/// after sweeping orphaned folders with that prefix.
///
/// # Panics
///
/// When the folder can't be made, as a test can't go on without it.
pub fn folder(prefix: &str) -> TempDir {
    sweep(Path::new("/tmp"), prefix);
    let dir = tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in("/tmp")
        .expect("make a test folder under /tmp");
    // Daemons refuse a socket folder other users can open.
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
        .expect("restrict the test folder");
    std::fs::write(dir.path().join(OWNER_FILE), std::process::id().to_string())
        .expect("record the test folder's owner");
    dir
}

/// Ends the processes run from, and removes, each folder in `parent`
/// named `prefix…` whose owner process is gone. Folders with no owner
/// file, or a live owner, are left alone.
fn sweep(parent: &Path, prefix: &str) {
    let Ok(entries) = std::fs::read_dir(parent) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let named = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(prefix));
        if !named || owner_alive(&path) != Some(false) {
            continue;
        }
        let _ = Command::new("pkill").arg("-f").arg(format!("{}/", path.display())).status();
        let _ = std::fs::remove_dir_all(&path);
    }
}

/// Whether the process that made `folder` may still run; None when the
/// folder names no owner.
fn owner_alive(folder: &Path) -> Option<bool> {
    let pid = std::fs::read_to_string(folder.join(OWNER_FILE)).ok()?;
    let pid = Pid::from_raw(pid.trim().parse().ok()?)?;
    // Only "no such process" means gone: a refusal (another sandbox, say)
    // means the process is there.
    Some(rustix::process::test_kill_process(pid) != Err(rustix::io::Errno::SRCH))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A process that has ended, for an owner nobody runs.
    fn ended_pid() -> u32 {
        let mut child = Command::new("/usr/bin/true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        pid
    }

    #[test]
    fn a_folder_whose_owner_ended_is_removed_with_what_runs_from_it() {
        let parent = tempfile::tempdir().unwrap();
        let orphan = parent.path().join("h2zorphan");
        std::fs::create_dir(&orphan).unwrap();
        std::fs::write(orphan.join(OWNER_FILE), ended_pid().to_string()).unwrap();
        let script = orphan.join("hold.sh");
        std::fs::write(&script, "#!/bin/sh\nsleep 60\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut held = Command::new(&script).spawn().unwrap();

        sweep(parent.path(), "h2z");

        assert!(!orphan.exists(), "the orphaned folder is removed");
        let status = held.wait().unwrap();
        assert!(!status.success(), "what ran from the folder was ended");
    }

    #[test]
    fn folders_with_a_live_owner_or_none_are_left_alone() {
        let parent = tempfile::tempdir().unwrap();
        let live = parent.path().join("h2zlive");
        std::fs::create_dir(&live).unwrap();
        std::fs::write(live.join(OWNER_FILE), std::process::id().to_string()).unwrap();
        let unowned = parent.path().join("h2zplain");
        std::fs::create_dir(&unowned).unwrap();
        let other = parent.path().join("otherdead");
        std::fs::create_dir(&other).unwrap();
        std::fs::write(other.join(OWNER_FILE), ended_pid().to_string()).unwrap();

        sweep(parent.path(), "h2z");

        assert!(live.exists() && unowned.exists() && other.exists());
    }

    #[test]
    fn a_new_folder_names_this_test_as_its_owner() {
        let dir = folder("h2ztest");
        assert_eq!(owner_alive(dir.path()), Some(true));
    }
}

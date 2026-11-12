//! Agent processes this daemon starts. Each runs in a process group of its
//! own and is noted in a folder while it runs, so that after the daemon is
//! killed outright, by SIGKILL, a crash or a power cut, its next start stops
//! the agents and the tools they ran, which would otherwise run on unseen.

use std::path::{Path, PathBuf};

/// A running agent's note, taken away when it's dropped.
pub(crate) struct Noted {
    file: PathBuf,
}

impl Noted {
    /// Notes the process `pid`, which leads its own process group, in
    /// `folder`. None if it can't be noted; the agent runs anyway, as it
    /// did before notes, and is only not stopped after a hard kill.
    pub(crate) fn note(folder: &Path, pid: Option<u32>) -> Option<Self> {
        let pid = pid?;
        let started = started(pid)?;
        std::fs::create_dir_all(folder).ok()?;
        let file = folder.join(pid.to_string());
        std::fs::write(&file, started).ok()?;
        Some(Self { file })
    }
}

impl Drop for Noted {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.file);
    }
}

/// Stops the process groups noted in `folder` by a daemon that ended
/// without taking its notes away. A note whose process is gone, or whose
/// pid now names another process, is only removed.
pub(crate) fn stop_left(folder: &Path) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let file = entry.path();
        let noted = std::fs::read_to_string(&file).ok();
        let pid = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse().ok());
        if let (Some(pid), Some(noted)) = (pid, noted)
            && started(pid).as_deref() == Some(noted.as_str())
            && let Some(group) = i32::try_from(pid)
                .ok()
                .and_then(rustix::process::Pid::from_raw)
        {
            let _ = rustix::process::kill_process_group(group, rustix::process::Signal::TERM);
        }
        let _ = std::fs::remove_file(&file);
    }
}

/// When `pid` started, as `ps` says; it tells a process from a later one
/// given the same pid.
fn started(pid: u32) -> Option<String> {
    let output = std::process::Command::new("/bin/ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let started = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (output.status.success() && !started.is_empty()).then_some(started)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_group_left_by_a_killed_daemon_is_stopped_at_the_next_start() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let folder = dir.path().join("running");
        let mut child = tokio::process::Command::new("/bin/sleep")
            .arg("30")
            .process_group(0)
            .kill_on_drop(true)
            .spawn()
            .unwrap_or_else(|error| panic!("{error}"));
        let noted = Noted::note(&folder, child.id()).expect("noted");
        // As a killed daemon would, it leaves the note behind.
        std::mem::forget(noted);
        stop_left(&folder);
        let status = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait())
            .await
            .expect("stopped")
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(!status.success());
        assert_eq!(
            std::fs::read_dir(&folder).map(Iterator::count).ok(),
            Some(0)
        );
    }

    #[test]
    fn a_note_for_a_pid_now_used_by_another_process_stops_nothing() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let folder = dir.path().join("running");
        std::fs::create_dir_all(&folder).unwrap_or_else(|error| panic!("{error}"));
        let me = std::process::id();
        std::fs::write(folder.join(me.to_string()), "Thu Jan  1 00:00:00 1970")
            .unwrap_or_else(|error| panic!("{error}"));
        stop_left(&folder);
        assert_eq!(
            std::fs::read_dir(&folder).map(Iterator::count).ok(),
            Some(0)
        );
    }
}

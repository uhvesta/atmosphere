//! The sweep of a stop (D261, D269): for every holder found by its pid
//! file, the sweep hangs up on the process groups under it (its program's
//! among them) and gives them `grace` to end; a group still alive then is
//! SIGKILLed. Only then does each holder get SIGTERM, and `grace` to end
//! before it is SIGKILLed too. The pid files and sockets are left for the
//! next holder to clear.
//!
//! A holder does the same hangup itself on SIGTERM, with its own 2 s
//! timer; the sweep does it first so that whatever had to be killed is
//! always the sweep's kill, and counted as one.
//!
//! The process table and the signals are the holder crate's
//! ([`humata2_holder::process`]); ending one holder by its pid is
//! [`humata2_holder::shutdown_or_end`].

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::io::ErrorKind;
use std::path::PathBuf;
use std::time::Duration;

use humata2_holder::process::{KILL_WAIT, Snapshot, own_group, send, send_group, settle_until};
use rustix::process::Signal;

// The names a holder may have when this daemon starts `holder`; the holder
// crate knows them.
pub use humata2_holder::process::holder_names;

/// How long a holder's programs get to end once hung up on, and how long
/// a holder gets to end once sent SIGTERM, before they are killed: the 2 s
/// a holder gives its program itself.
pub const GRACE: Duration = Duration::from_secs(2);

/// The holders of one machine to end, and how long they get to end.
#[derive(Debug)]
pub struct Sweep {
    /// The folder the holders keep their `<name>.pid` and `<name>.sock`
    /// files in.
    pub holders: PathBuf,
    /// How long the programs get to end on hangup, and then the holders
    /// on SIGTERM, before they are killed.
    pub grace: Duration,
    /// The process names a live pid may have to count as a holder. A pid
    /// with any other name is stale (reused) and is never signalled.
    pub programs: Vec<OsString>,
}

/// What a sweep did.
#[derive(Debug, Default)]
pub struct Swept {
    /// Each holder that was running when the sweep began.
    pub ended: Vec<HolderEnd>,
    /// Pids of holders or program-group processes still alive after the
    /// sweep. Empty when the machine is stopped.
    pub left: Vec<u32>,
}

/// One holder the sweep ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HolderEnd {
    pub socket: PathBuf,
    pub pid: u32,
    pub how: How,
}

/// How a holder ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum How {
    /// Its programs ended on hangup and it ended on SIGTERM, each within
    /// the grace.
    Ended,
    /// It, or a process group under it, had to be SIGKILLed by the sweep.
    Killed,
}

#[derive(Debug, thiserror::Error)]
pub enum SweepError {
    #[error("couldn't read the holders folder {}: {source}; check that humatad2 can read it, then stop again", .path.display())]
    ReadFolder {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("couldn't read the holder pid file {}: {source}; check that the file is readable by humatad2, then stop again", .path.display())]
    ReadPidFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "couldn't list the running processes with `ps`: {source}; check that `ps` runs for this user, then stop again"
    )]
    ListProcesses {
        #[from]
        source: std::io::Error,
    },
}

impl Sweep {
    /// Ends every holder found by its pid file, and whatever won't end.
    /// Running it again finds nothing left to end.
    pub async fn run(&self) -> Result<Swept, SweepError> {
        let found = self.pid_files().await?;
        let names = self.programs.as_slice();
        let mut live = Vec::new();
        if !found.is_empty() {
            let snapshot = Snapshot::take().await?;
            let own = own_group();
            live = found
                .into_iter()
                .filter(|(_, pid)| snapshot.is_holder(*pid, names))
                .map(|(socket, pid)| Live {
                    groups: snapshot.groups_under(pid, own),
                    socket,
                    pid,
                })
                .collect();
        }
        let mut killed = vec![false; live.len()];

        // The programs first: hung up on, as the holder would, then killed
        // if they're still there once the grace is out.
        for group in live.iter().flat_map(|holder| &holder.groups) {
            send_group(*group, Signal::HUP);
        }
        humata2_daemon_runtime::fault::point("stop.programs.hungup");
        let snapshot = settle_groups(&live, self.grace).await?;
        let mut kills = false;
        for (holder, killed) in live.iter().zip(killed.iter_mut()) {
            for group in &holder.groups {
                if snapshot.group_alive(*group) {
                    send_group(*group, Signal::KILL);
                    *killed = true;
                    kills = true;
                }
            }
        }
        if kills {
            settle_groups(&live, KILL_WAIT).await?;
        }

        // Then the holders, whose programs are gone: each saves what its
        // terminal printed and ends.
        for holder in &live {
            send(holder.pid, Signal::TERM);
        }
        humata2_daemon_runtime::fault::point("stop.holders.signalled");
        let mut snapshot = settle(&live, names, self.grace).await?;
        // Decided on the latest process list, so a pid that has since
        // ended, or been reused, is not signalled.
        let mut kills = false;
        for (holder, killed) in live.iter().zip(killed.iter_mut()) {
            if snapshot.is_holder(holder.pid, names) {
                send(holder.pid, Signal::KILL);
                *killed = true;
                kills = true;
            }
            for group in &holder.groups {
                if snapshot.group_alive(*group) {
                    send_group(*group, Signal::KILL);
                    *killed = true;
                    kills = true;
                }
            }
        }
        if kills {
            snapshot = settle(&live, names, KILL_WAIT).await?;
        }

        let left = still_alive(&snapshot, &live, names);
        let ended = live
            .into_iter()
            .zip(killed)
            .map(|(holder, killed)| HolderEnd {
                socket: holder.socket,
                pid: holder.pid,
                how: if killed { How::Killed } else { How::Ended },
            })
            .collect();
        humata2_daemon_runtime::fault::point("stop.holders.gone");
        Ok(Swept { ended, left })
    }

    /// The `(socket, pid)` of each `*.pid` file in the holders folder whose
    /// content is a pid. A missing folder has none.
    async fn pid_files(&self) -> Result<Vec<(PathBuf, u32)>, SweepError> {
        let mut entries = match tokio::fs::read_dir(&self.holders).await {
            Ok(entries) => entries,
            Err(source) if source.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(SweepError::ReadFolder {
                    path: self.holders.clone(),
                    source,
                });
            }
        };
        let own = std::process::id();
        let mut found = Vec::new();
        while let Some(entry) =
            entries
                .next_entry()
                .await
                .map_err(|source| SweepError::ReadFolder {
                    path: self.holders.clone(),
                    source,
                })?
        {
            let path = entry.path();
            if path.extension() != Some(OsStr::new("pid")) {
                continue;
            }
            let text = match tokio::fs::read_to_string(&path).await {
                Ok(text) => text,
                Err(source) if source.kind() == ErrorKind::NotFound => continue,
                Err(source) => return Err(SweepError::ReadPidFile { path, source }),
            };
            // A file that doesn't hold a pid is not a holder's: skipped.
            let Ok(pid) = text.trim().parse::<u32>() else {
                continue;
            };
            if pid != 0 && pid != own {
                found.push((path.with_extension("sock"), pid));
            }
        }
        found.sort();
        Ok(found)
    }
}

/// A holder the sweep ends: its socket and pid, and the process groups of
/// the processes it started (its program's among them).
struct Live {
    socket: PathBuf,
    pid: u32,
    groups: BTreeSet<u32>,
}

/// The pids of the holders in `live`.
fn pids_of(live: &[Live]) -> Vec<u32> {
    live.iter().map(|holder| holder.pid).collect()
}

/// The process groups under the holders in `live`.
fn groups_of(live: &[Live]) -> BTreeSet<u32> {
    live.iter()
        .flat_map(|holder| holder.groups.iter().copied())
        .collect()
}

/// The pids still alive of the holders in `live`, and of the processes in
/// their groups.
fn still_alive(snapshot: &Snapshot, live: &[Live], names: &[OsString]) -> Vec<u32> {
    snapshot.members(&pids_of(live), &groups_of(live), names)
}

/// Takes the process list until no holder in `live` and no group under one
/// has a live process, or until `within` has passed, and returns the last
/// list taken.
async fn settle(
    live: &[Live],
    names: &[OsString],
    within: Duration,
) -> Result<Snapshot, SweepError> {
    settle_while(live, within, |snapshot| {
        !still_alive(snapshot, live, names).is_empty()
    })
    .await
}

/// As [`settle`], for the groups under the holders only.
async fn settle_groups(live: &[Live], within: Duration) -> Result<Snapshot, SweepError> {
    let groups = groups_of(live);
    settle_while(live, within, |snapshot| {
        groups.iter().any(|group| snapshot.group_alive(*group))
    })
    .await
}

/// As [`settle_until`], and a list of nothing to wait for is taken once.
async fn settle_while(
    live: &[Live],
    within: Duration,
    waiting: impl Fn(&Snapshot) -> bool,
) -> Result<Snapshot, SweepError> {
    if live.is_empty() {
        return Ok(Snapshot::default());
    }
    Ok(settle_until(within, waiting).await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::process::{Child, Command};
    use tokio::time::{Instant, sleep};

    fn sweep_in(dir: &Path, grace: Duration) -> Sweep {
        Sweep {
            holders: dir.to_path_buf(),
            grace,
            programs: vec![OsString::from("sh")],
        }
    }

    fn start_sh(script: &str, args: &[&Path]) -> Child {
        Command::new("sh")
            .arg("-c")
            .arg(script)
            .arg("sh")
            .args(args)
            .spawn()
            .expect("sh starts")
    }

    fn reap(mut child: Child) {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }

    fn write_pid(dir: &Path, name: &str, pid: u32) {
        fs::write(dir.join(name), pid.to_string()).expect("pid file is written");
    }

    async fn alive(pid: u32) -> bool {
        Snapshot::take().await.expect("ps runs").is_alive(pid)
    }

    async fn wait_for_file(path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !path.exists() {
            assert!(Instant::now() < deadline, "the stand-in never got ready");
            sleep(Duration::from_millis(10)).await;
        }
    }

    async fn wait_for_exit(pid: u32) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while alive(pid).await {
            assert!(Instant::now() < deadline, "process {pid} never ended");
            sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn a_holder_that_ends_on_term_is_ended_and_leaves_nothing() {
        let dir = tempfile::tempdir().expect("temp dir");
        let holder = start_sh("while :; do sleep 1; done", &[]);
        let pid = holder.id();
        write_pid(dir.path(), "x.pid", pid);
        reap(holder);

        let sweep = sweep_in(dir.path(), GRACE);
        let swept = sweep.run().await.expect("sweep runs");
        assert_eq!(swept.ended.len(), 1);
        assert_eq!(swept.ended[0].pid, pid);
        assert_eq!(swept.ended[0].how, How::Ended);
        assert!(swept.left.is_empty());
        wait_for_exit(pid).await;

        let again = sweep.run().await.expect("second sweep runs");
        assert!(again.ended.is_empty());
        assert!(again.left.is_empty());
    }

    #[tokio::test]
    async fn a_holder_that_ignores_term_is_killed_and_leaves_nothing() {
        let dir = tempfile::tempdir().expect("temp dir");
        let ready = dir.path().join("ready");
        let holder = start_sh(
            r#"trap "" TERM HUP; : > "$1"; while :; do sleep 1; done"#,
            &[ready.as_path()],
        );
        let pid = holder.id();
        write_pid(dir.path(), "x.pid", pid);
        reap(holder);
        wait_for_file(&ready).await;

        let swept = sweep_in(dir.path(), Duration::from_millis(300))
            .run()
            .await
            .expect("sweep runs");
        assert_eq!(swept.ended.len(), 1);
        assert_eq!(swept.ended[0].pid, pid);
        assert_eq!(swept.ended[0].how, How::Killed);
        assert!(swept.left.is_empty());
        wait_for_exit(pid).await;
    }

    #[tokio::test]
    async fn a_program_group_under_a_holder_is_killed_with_it() {
        let dir = tempfile::tempdir().expect("temp dir");
        let ready = dir.path().join("ready");
        let child_file = dir.path().join("child");
        let holder = start_sh(
            r#"trap "" TERM HUP; set -m; sleep 1000 & echo $! > "$2"; : > "$1"; while :; do sleep 1; done"#,
            &[ready.as_path(), child_file.as_path()],
        );
        let pid = holder.id();
        write_pid(dir.path(), "x.pid", pid);
        reap(holder);
        wait_for_file(&ready).await;
        let child: u32 = fs::read_to_string(&child_file)
            .expect("child pid is written before ready")
            .trim()
            .parse()
            .expect("child pid parses");
        let snapshot = Snapshot::take().await.expect("ps runs");
        assert_eq!(
            snapshot.group_of(child),
            Some(child),
            "the program must be in a group of its own for this test"
        );

        let swept = sweep_in(dir.path(), Duration::from_millis(300))
            .run()
            .await
            .expect("sweep runs");
        assert_eq!(swept.ended.len(), 1);
        assert_eq!(swept.ended[0].how, How::Killed);
        assert!(swept.left.is_empty());
        wait_for_exit(pid).await;
        wait_for_exit(child).await;
    }

    async fn holder_with_program(dir: &Path, program: &str) -> (Child, u32) {
        let ready = dir.join("ready");
        let child_file = dir.join("child");
        let holder = start_sh(
            &format!(
                r#"set -m; sh -c '{program}; echo $$ > "$0.tmp"; mv "$0.tmp" "$0"; while :; do sleep 1; done' "$2" & : > "$1"; while :; do sleep 1; done"#
            ),
            &[ready.as_path(), child_file.as_path()],
        );
        wait_for_file(&ready).await;
        wait_for_file(&child_file).await;
        let child: u32 = fs::read_to_string(&child_file)
            .expect("child pid is written")
            .trim()
            .parse()
            .expect("child pid parses");
        (holder, child)
    }

    #[tokio::test]
    async fn a_program_that_ignores_hangup_is_killed_and_counted() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (holder, child) = holder_with_program(dir.path(), r#"trap "" HUP TERM"#).await;
        let pid = holder.id();
        write_pid(dir.path(), "x.pid", pid);
        reap(holder);

        let swept = sweep_in(dir.path(), Duration::from_millis(300))
            .run()
            .await
            .expect("sweep runs");
        assert_eq!(swept.ended.len(), 1);
        assert_eq!(swept.ended[0].how, How::Killed);
        assert!(swept.left.is_empty());
        wait_for_exit(child).await;
        wait_for_exit(pid).await;
    }

    #[tokio::test]
    async fn a_program_that_ends_on_hangup_leaves_its_holder_ended() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (holder, child) = holder_with_program(dir.path(), ":").await;
        let pid = holder.id();
        write_pid(dir.path(), "x.pid", pid);
        reap(holder);

        let swept = sweep_in(dir.path(), GRACE).run().await.expect("sweep runs");
        assert_eq!(swept.ended.len(), 1);
        assert_eq!(swept.ended[0].how, How::Ended);
        assert!(swept.left.is_empty());
        wait_for_exit(child).await;
        wait_for_exit(pid).await;
    }

    #[tokio::test]
    async fn a_pid_file_naming_a_dead_process_is_skipped() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut done = Command::new("true").spawn().expect("true starts");
        let pid = done.id();
        done.wait().expect("true ends");
        write_pid(dir.path(), "x.pid", pid);

        let swept = sweep_in(dir.path(), GRACE).run().await.expect("sweep runs");
        assert!(swept.ended.is_empty());
        assert!(swept.left.is_empty());
    }

    #[tokio::test]
    async fn a_pid_file_naming_a_live_process_with_another_name_leaves_it_alone() {
        let dir = tempfile::tempdir().expect("temp dir");
        let other = Command::new("sleep")
            .arg("1000")
            .spawn()
            .expect("sleep starts");
        let pid = other.id();
        reap(other);
        write_pid(dir.path(), "x.pid", pid);

        let swept = sweep_in(dir.path(), GRACE).run().await.expect("sweep runs");
        assert!(swept.ended.is_empty());
        assert!(alive(pid).await, "a process with another name is untouched");

        send(pid, Signal::KILL);
        wait_for_exit(pid).await;
    }

    #[tokio::test]
    async fn a_missing_holders_folder_has_nothing_to_sweep() {
        let dir = tempfile::tempdir().expect("temp dir");
        let swept = sweep_in(&dir.path().join("missing"), GRACE)
            .run()
            .await
            .expect("sweep runs");
        assert!(swept.ended.is_empty());
        assert!(swept.left.is_empty());
    }
}

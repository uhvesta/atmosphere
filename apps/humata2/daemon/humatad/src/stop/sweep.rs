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

use std::collections::{BTreeSet, HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rustix::process::{Pid, Signal, getpgrp, kill_process, kill_process_group};
use tokio::time::{Instant, sleep};

/// How long a holder's programs get to end once hung up on, and how long
/// a holder gets to end once sent SIGTERM: the 2 s a holder gives its
/// program itself.
pub(crate) const GRACE: Duration = Duration::from_secs(2);
/// The names a holder's process may have: the holder this daemon starts,
/// and its test build's, which a daemon of either build may find left.
const HOLDER_NAMES: [&str; 2] = ["humata2_holder", "humata2_holder_faults"];
/// How long the holders and groups get to end once they have been killed.
const KILL_WAIT: Duration = Duration::from_secs(1);
/// How often the process list is taken while waiting for things to end.
const POLL: Duration = Duration::from_millis(20);

/// The holders of one machine to end, and how long they get to end.
#[derive(Debug)]
pub(crate) struct Sweep {
    /// The folder the holders keep their `<name>.pid` and `<name>.sock`
    /// files in.
    pub(crate) holders: PathBuf,
    /// How long the programs get to end on hangup, and then the holders
    /// on SIGTERM, before they are killed.
    pub(crate) grace: Duration,
    /// The process names a live pid may have to count as a holder. A pid
    /// with any other name is stale (reused) and is never signalled.
    pub(crate) programs: Vec<OsString>,
}

/// The names a holder may have when this daemon starts `holder`: its file
/// name, and those of the holder builds.
pub(crate) fn holder_names(holder: &Path) -> Vec<OsString> {
    let mut names: Vec<OsString> = HOLDER_NAMES.iter().map(OsString::from).collect();
    if let Some(name) = holder.file_name()
        && !names.iter().any(|known| known.as_os_str() == name)
    {
        names.push(name.to_os_string());
    }
    names
}

/// What a sweep did.
#[derive(Debug, Default)]
pub(crate) struct Swept {
    /// Each holder that was running when the sweep began.
    pub(crate) ended: Vec<HolderEnd>,
    /// Pids of holders or program-group processes still alive after the
    /// sweep. Empty when the machine is stopped.
    pub(crate) left: Vec<u32>,
}

/// One holder the sweep ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HolderEnd {
    pub(crate) socket: PathBuf,
    pub(crate) pid: u32,
    pub(crate) how: How,
}

/// How a holder ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum How {
    /// Its programs ended on hangup and it ended on SIGTERM, each within
    /// the grace.
    Ended,
    /// It, or a process group under it, had to be SIGKILLed by the sweep.
    Killed,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SweepError {
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
    #[error("couldn't list the running processes with `ps`: {source}; check that `ps` runs for this user, then stop again")]
    ListProcesses {
        #[source]
        source: std::io::Error,
    },
}

impl Sweep {
    /// Ends every holder found by its pid file, and whatever won't end.
    /// Running it again finds nothing left to end.
    pub(crate) async fn run(&self) -> Result<Swept, SweepError> {
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
        crate::fault::point("stop.programs.hungup");
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
        crate::fault::point("stop.holders.signalled");
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

        let left = snapshot.left(&live, names);
        let ended = live
            .into_iter()
            .zip(killed)
            .map(|(holder, killed)| HolderEnd {
                socket: holder.socket,
                pid: holder.pid,
                how: if killed { How::Killed } else { How::Ended },
            })
            .collect();
        crate::fault::point("stop.holders.gone");
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
        while let Some(entry) = entries
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

/// Takes the process list until no holder in `live` and no group under one
/// has a live process, or until `within` has passed, and returns the last
/// list taken.
async fn settle(live: &[Live], names: &[OsString], within: Duration) -> Result<Snapshot, SweepError> {
    settle_until(live, within, |snapshot| snapshot.anything_alive(live, names)).await
}

/// As [`settle`], for the groups under the holders only.
async fn settle_groups(live: &[Live], within: Duration) -> Result<Snapshot, SweepError> {
    settle_until(live, within, |snapshot| {
        live.iter()
            .flat_map(|holder| &holder.groups)
            .any(|group| snapshot.group_alive(*group))
    })
    .await
}

async fn settle_until(
    live: &[Live],
    within: Duration,
    waiting: impl Fn(&Snapshot) -> bool,
) -> Result<Snapshot, SweepError> {
    if live.is_empty() {
        return Ok(Snapshot::default());
    }
    let deadline = Instant::now() + within;
    loop {
        let snapshot = Snapshot::take().await?;
        if !waiting(&snapshot) || Instant::now() >= deadline {
            return Ok(snapshot);
        }
        sleep(POLL).await;
    }
}

/// The process list, as `ps` reports it.
#[derive(Default)]
struct Snapshot {
    rows: Vec<Row>,
}

struct Row {
    pid: u32,
    ppid: u32,
    pgid: u32,
    zombie: bool,
    /// The file name of the process's command, without its directory.
    name: OsString,
}

impl Snapshot {
    async fn take() -> Result<Self, SweepError> {
        let output = tokio::process::Command::new("ps")
            .args(["-A", "-o", "pid=,ppid=,pgid=,stat=,comm="])
            .output()
            .await
            .map_err(|source| SweepError::ListProcesses { source })?;
        if !output.status.success() {
            return Err(SweepError::ListProcesses {
                source: std::io::Error::other(format!(
                    "ps exited with code {:?}",
                    output.status.code()
                )),
            });
        }
        let text = String::from_utf8_lossy(&output.stdout);
        Ok(Self {
            rows: text.lines().filter_map(parse_row).collect(),
        })
    }

    /// Whether `pid` is a live process with one of `names`. A zombie has
    /// ended.
    fn is_holder(&self, pid: u32, names: &[OsString]) -> bool {
        self.rows
            .iter()
            .any(|row| row.pid == pid && !row.zombie && names.contains(&row.name))
    }

    fn group_alive(&self, group: u32) -> bool {
        self.rows
            .iter()
            .any(|row| row.pgid == group && !row.zombie)
    }

    fn anything_alive(&self, live: &[Live], names: &[OsString]) -> bool {
        live.iter().any(|holder| {
            self.is_holder(holder.pid, names)
                || holder.groups.iter().any(|group| self.group_alive(*group))
        })
    }

    /// The process groups of the processes under `holder`, which are the
    /// holder's own children and everything they started. Our own group,
    /// the holder's (it is ended on its own, after its programs) and the
    /// system's (0 and 1) are never noted.
    fn groups_under(&self, holder: u32, own: Option<u32>) -> BTreeSet<u32> {
        let holder_group = self.rows.iter().find(|row| row.pid == holder).map(|row| row.pgid);
        let mut children: HashMap<u32, Vec<&Row>> = HashMap::new();
        for row in &self.rows {
            children.entry(row.ppid).or_default().push(row);
        }
        let mut groups = BTreeSet::new();
        let mut seen = HashSet::from([holder]);
        let mut queue = vec![holder];
        while let Some(parent) = queue.pop() {
            for child in children.get(&parent).into_iter().flatten() {
                if seen.insert(child.pid) {
                    queue.push(child.pid);
                }
                if child.pgid > 1 && Some(child.pgid) != own && Some(child.pgid) != holder_group {
                    groups.insert(child.pgid);
                }
            }
        }
        groups
    }

    /// The pids of the holders and the group processes still alive.
    fn left(&self, live: &[Live], names: &[OsString]) -> Vec<u32> {
        let groups: BTreeSet<u32> = live
            .iter()
            .flat_map(|holder| holder.groups.iter().copied())
            .collect();
        let mut pids = BTreeSet::new();
        for row in self.rows.iter().filter(|row| !row.zombie) {
            let holder_alive = names.contains(&row.name)
                && live.iter().any(|holder| holder.pid == row.pid);
            if holder_alive || groups.contains(&row.pgid) {
                pids.insert(row.pid);
            }
        }
        pids.into_iter().collect()
    }
}

/// One `ps` line: `pid ppid pgid stat comm`, where comm is the rest of the
/// line (it may hold spaces, and is a full path on macOS).
fn parse_row(line: &str) -> Option<Row> {
    let (pid, rest) = split_field(line)?;
    let (ppid, rest) = split_field(rest)?;
    let (pgid, rest) = split_field(rest)?;
    let (stat, comm) = split_field(rest)?;
    let comm = comm.trim_end();
    if comm.is_empty() {
        return None;
    }
    Some(Row {
        pid: pid.parse().ok()?,
        ppid: ppid.parse().ok()?,
        pgid: pgid.parse().ok()?,
        zombie: stat.starts_with('Z'),
        name: Path::new(comm)
            .file_name()
            .unwrap_or(OsStr::new(comm))
            .to_os_string(),
    })
}

/// The first whitespace-free field of `text`, and the text after it.
fn split_field(text: &str) -> Option<(&str, &str)> {
    let text = text.trim_start();
    let end = text.find(char::is_whitespace)?;
    Some((&text[..end], text[end..].trim_start()))
}

/// Our own process group, which a sweep never signals.
fn own_group() -> Option<u32> {
    u32::try_from(getpgrp().as_raw_pid()).ok()
}

fn to_pid(raw: u32) -> Option<Pid> {
    i32::try_from(raw).ok().and_then(Pid::from_raw)
}

/// Sends `sig` to `pid`. A process that has already ended is fine. One that
/// refuses to end stays in the sweep's `left`, so nothing is reported gone
/// that isn't.
fn send(pid: u32, sig: Signal) {
    if let Some(pid) = to_pid(pid) {
        let _ = kill_process(pid, sig);
    }
}

fn send_group(group: u32, sig: Signal) {
    if let Some(group) = to_pid(group) {
        let _ = kill_process_group(group, sig);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::{Child, Command};

    /// A sweep over `dir`, whose holders are `sh` processes.
    fn sweep_in(dir: &Path, grace: Duration) -> Sweep {
        Sweep {
            holders: dir.to_path_buf(),
            grace,
            programs: vec![OsString::from("sh")],
        }
    }

    /// Starts `sh -c script`, with `args` as its `$1`, `$2`, ..., and
    /// reaps it on a thread once it ends.
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

    /// Whether `pid` is a process that hasn't ended. A zombie has.
    async fn alive(pid: u32) -> bool {
        Snapshot::take()
            .await
            .expect("ps runs")
            .rows
            .iter()
            .any(|row| row.pid == pid && !row.zombie)
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
        // With job control on, the background sleep gets a group of its
        // own, as a holder's program does.
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
        assert!(
            snapshot
                .rows
                .iter()
                .any(|row| row.pid == child && row.pgid == child),
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

    /// Starts a stand-in holder that ends on SIGTERM, with a program in a
    /// group of its own running `program`; returns the holder and the
    /// program's pid once both are up.
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

        let swept = sweep_in(dir.path(), GRACE)
            .run()
            .await
            .expect("sweep runs");
        assert_eq!(swept.ended.len(), 1);
        assert_eq!(swept.ended[0].how, How::Ended);
        assert!(swept.left.is_empty());
        wait_for_exit(child).await;
        wait_for_exit(pid).await;
    }

    #[test]
    fn a_holder_of_either_build_or_the_configured_name_counts() {
        let names = holder_names(Path::new("/opt/bin/my_holder"));
        for name in ["humata2_holder", "humata2_holder_faults", "my_holder"] {
            assert!(names.contains(&OsString::from(name)), "{name}: {names:?}");
        }
        let names = holder_names(Path::new("/opt/bin/humata2_holder"));
        assert_eq!(names.len(), 2, "{names:?}");
    }

    #[tokio::test]
    async fn a_pid_file_naming_a_dead_process_is_skipped() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut done = Command::new("true").spawn().expect("true starts");
        let pid = done.id();
        done.wait().expect("true ends");
        write_pid(dir.path(), "x.pid", pid);

        let swept = sweep_in(dir.path(), GRACE)
            .run()
            .await
            .expect("sweep runs");
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

        let swept = sweep_in(dir.path(), GRACE)
            .run()
            .await
            .expect("sweep runs");
        assert!(swept.ended.is_empty());
        assert!(alive(pid).await, "a process with another name is untouched");

        kill_process(to_pid(pid).expect("pid is positive"), Signal::KILL)
            .expect("the test stops its own sleep");
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

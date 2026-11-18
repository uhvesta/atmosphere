//! The process table, as `ps` reports it, and what it says about holders:
//! whether a pid is a live holder, and which process groups run under one.
//! A holder is ended by its pid (see [`crate::shutdown_or_end`]), and
//! humatad's stop sweeps them. A pid counts as a holder only while it
//! carries one of the holder names, so a pid that has ended, or was reused
//! by another program, is never signalled.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::time::Duration;

use rustix::process::{Pid, Signal, getpgrp, kill_process, kill_process_group};
use tokio::process::Command;
use tokio::time::{Instant, sleep};

/// The names a holder process has: this build's, and its test build's.
const HOLDER_NAMES: [&str; 2] = ["humata2_holder", "humata2_holder_faults"];
/// How long the holders and groups get to end once they have been killed.
pub const KILL_WAIT: Duration = Duration::from_secs(1);
/// How often the process list is taken while waiting for things to end.
const POLL: Duration = Duration::from_millis(20);
/// How much of a command's name Linux's `ps` lists: the rest is cut off.
const LINUX_NAME_BYTES: usize = 15;
/// What Linux adds to a program's path once its file is replaced or
/// removed, as an upgrade does to a running holder's.
#[cfg(any(target_os = "linux", test))]
const REPLACED: &[u8] = b" (deleted)";

/// The names a holder may have when started from `holder`: the holder
/// builds' names, and `holder`'s own file name. Pass an empty path to get
/// the builds' names alone.
pub fn holder_names(holder: &Path) -> Vec<OsString> {
    let mut names: Vec<OsString> = HOLDER_NAMES.iter().map(OsString::from).collect();
    if let Some(name) = holder.file_name()
        && !names.iter().any(|known| known.as_os_str() == name)
    {
        names.push(name.to_os_string());
    }
    names
}

/// The process list at one moment.
#[derive(Debug, Default)]
pub struct Snapshot {
    rows: Vec<Row>,
}

#[derive(Debug)]
struct Row {
    pid: u32,
    ppid: u32,
    pgid: u32,
    /// Listed but no longer running: a zombie, a dead process (Linux's X),
    /// or one still being torn down after its exit (macOS's E flag), as a
    /// holder can be for a moment after SIGKILL on a busy machine.
    ended: bool,
    /// The file name of the process's command, without its directory. Cut
    /// to [`LINUX_NAME_BYTES`] on Linux.
    name: OsString,
}

impl Row {
    /// Whether this process carries one of `names`.
    fn carries(&self, names: &[OsString]) -> bool {
        carries(&self.name, names, || program_name(self.pid))
    }
}

/// Whether a process listed as `name` carries one of `names`. A name cut to
/// [`LINUX_NAME_BYTES`] that starts one of them counts only when the
/// program the process runs, from `program`, is named one of them.
fn carries(
    name: &OsStr,
    names: &[OsString],
    program: impl FnOnce() -> Option<OsString>,
) -> bool {
    if names.iter().any(|known| known.as_os_str() == name) {
        return true;
    }
    let cut = name.as_bytes();
    cut.len() == LINUX_NAME_BYTES
        && names.iter().any(|known| known.as_bytes().starts_with(cut))
        && program().is_some_and(|program| names.contains(&program))
}

/// The file name of the program `pid` runs, from Linux's `/proc`.
#[cfg(target_os = "linux")]
fn program_name(pid: u32) -> Option<OsString> {
    std::fs::read_link(format!("/proc/{pid}/exe"))
        .ok()
        .and_then(|path| file_name_of(&path))
}

/// Only Linux cuts a listed name short, so elsewhere there is nothing more
/// to look up.
#[cfg(not(target_os = "linux"))]
fn program_name(_pid: u32) -> Option<OsString> {
    None
}

/// The file name in a program's path as Linux links it, without the mark
/// it adds once that file is replaced or removed.
#[cfg(any(target_os = "linux", test))]
fn file_name_of(path: &Path) -> Option<OsString> {
    let name = path.file_name()?.as_bytes();
    let name = name.strip_suffix(REPLACED).unwrap_or(name);
    Some(OsStr::from_bytes(name).to_os_string())
}

impl Snapshot {
    /// Takes the process list with `ps`.
    pub async fn take() -> io::Result<Self> {
        let output = Command::new("ps")
            .args(["-A", "-o", "pid=,ppid=,pgid=,stat=,comm="])
            .output()
            .await?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "ps exited with code {:?}",
                output.status.code()
            )));
        }
        let text = String::from_utf8_lossy(&output.stdout);
        Ok(Self {
            rows: text.lines().filter_map(parse_row).collect(),
        })
    }

    /// Whether `pid` is a live process with one of `names`. A zombie, or a
    /// process still exiting, has ended.
    pub fn is_holder(&self, pid: u32, names: &[OsString]) -> bool {
        self.rows
            .iter()
            .any(|row| row.pid == pid && !row.ended && row.carries(names))
    }

    /// Whether `pid` is a live process, of any name. A zombie, or a process
    /// still exiting, has ended.
    pub fn is_alive(&self, pid: u32) -> bool {
        self.rows.iter().any(|row| row.pid == pid && !row.ended)
    }

    /// The process group `pid` is in, if it is in the list.
    pub fn group_of(&self, pid: u32) -> Option<u32> {
        self.rows
            .iter()
            .find(|row| row.pid == pid)
            .map(|row| row.pgid)
    }

    /// Whether any process in `group` is live.
    pub fn group_alive(&self, group: u32) -> bool {
        self.rows.iter().any(|row| row.pgid == group && !row.ended)
    }

    /// The live processes that are one of `pids` and carry one of `names`,
    /// or that are in one of `groups`. Empty when none of them is alive.
    pub fn members(&self, pids: &[u32], groups: &BTreeSet<u32>, names: &[OsString]) -> Vec<u32> {
        let mut found = BTreeSet::new();
        for row in self.rows.iter().filter(|row| !row.ended) {
            let holder_alive = pids.contains(&row.pid) && row.carries(names);
            if holder_alive || groups.contains(&row.pgid) {
                found.insert(row.pid);
            }
        }
        found.into_iter().collect()
    }

    /// The process groups of the processes under `holder`, which are the
    /// holder's own children and everything they started. Our own group,
    /// the holder's (it is ended on its own, after its programs) and the
    /// system's (0 and 1) are never noted.
    pub fn groups_under(&self, holder: u32, own: Option<u32>) -> BTreeSet<u32> {
        let holder_group = self.group_of(holder);
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
}

/// Takes the process list until `waiting` is false for it or `within` has
/// passed, and returns the last list taken. Polls every [`POLL`].
pub async fn settle_until(
    within: Duration,
    waiting: impl Fn(&Snapshot) -> bool,
) -> io::Result<Snapshot> {
    let deadline = Instant::now() + within;
    loop {
        let snapshot = Snapshot::take().await?;
        if !waiting(&snapshot) || Instant::now() >= deadline {
            return Ok(snapshot);
        }
        sleep(POLL).await;
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
        ended: stat.starts_with(['Z', 'X'])
            || stat.get(1..).is_some_and(|flags| flags.contains('E')),
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

fn to_pid(raw: u32) -> Option<Pid> {
    i32::try_from(raw).ok().and_then(Pid::from_raw)
}

/// Sends `sig` to `pid`. A process that has already ended is fine. One that
/// refuses to end is still in the list, so nothing is reported gone that
/// isn't.
pub fn send(pid: u32, sig: Signal) {
    if let Some(pid) = to_pid(pid) {
        let _ = kill_process(pid, sig);
    }
}

/// Sends `sig` to the process group `group`, as [`send`] does to a pid.
pub fn send_group(group: u32, sig: Signal) {
    if let Some(group) = to_pid(group) {
        let _ = kill_process_group(group, sig);
    }
}

/// Our own process group, which a sweep never signals.
pub fn own_group() -> Option<u32> {
    u32::try_from(getpgrp().as_raw_pid()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_holder_of_either_build_or_the_configured_name_counts() {
        let names = holder_names(Path::new("/opt/bin/my_holder"));
        for name in ["humata2_holder", "humata2_holder_faults", "my_holder"] {
            assert!(names.contains(&OsString::from(name)), "{name}: {names:?}");
        }
        let names = holder_names(Path::new("/opt/bin/humata2_holder"));
        assert_eq!(names.len(), 2, "{names:?}");
    }

    fn listed(text: &str) -> Snapshot {
        Snapshot {
            rows: text.lines().filter_map(parse_row).collect(),
        }
    }

    #[test]
    fn a_name_cut_short_counts_only_when_its_program_is_a_holder() {
        let names = holder_names(Path::new(""));
        let cut = OsStr::new("humata2_holder_");
        let named = |program: &str| Some(OsString::from(program));

        assert!(carries(cut, &names, || named("humata2_holder_faults")));
        assert!(!carries(cut, &names, || named("humata2_holder_other")));
        assert!(!carries(cut, &names, || None));
        let other = OsStr::new("humata2_holdee_");
        assert!(!carries(other, &names, || named("humata2_holder_faults")));
    }

    #[test]
    fn a_holder_whose_program_was_replaced_keeps_its_name() {
        let replaced = file_name_of(Path::new("/x/humata2_holder_faults (deleted)"));
        let kept = file_name_of(Path::new("/x/humata2_holder_faults"));

        assert_eq!(replaced, Some(OsString::from("humata2_holder_faults")));
        assert_eq!(kept, replaced);
    }

    #[test]
    fn a_holder_that_is_exiting_or_dead_has_ended() {
        let names = holder_names(Path::new(""));
        // Running; still exiting after its kill (macOS); dead (Linux); a zombie.
        let snapshot = listed(
            "   10     1    10 Ss   /x/humata2_holder\n\
                11     1    11 ?E   /x/humata2_holder\n\
                12     1    12 RE   /x/humata2_holder\n\
                13     1    13 X    /x/humata2_holder\n\
                14     1    14 Z    /x/humata2_holder\n",
        );
        assert!(snapshot.is_holder(10, &names));
        assert!(snapshot.is_alive(10));
        for pid in [11, 12, 13, 14] {
            assert!(!snapshot.is_holder(pid, &names), "pid {pid} counts as a holder");
            assert!(!snapshot.is_alive(pid), "pid {pid} counts as alive");
        }
    }
}

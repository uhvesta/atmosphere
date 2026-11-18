//! Ends every process a test world started, whether or not the test got
//! as far as dropping its world.
//!
//! Holders start their own sessions, so killing the test's process group
//! (as a test runner does on a timeout) leaves them running, along with
//! the shells and programs under them. A `yes` left flooding a holder
//! costs a core for as long as it runs. So:
//!
//! - [`Leftovers::end`], from the world's `Drop`, kills every process
//!   whose command line names one of the world's folders, and everything
//!   under those, by process and process group, and reports any that are
//!   still there after [`END_WAIT`];
//! - a watcher started with the world does the same once the test process
//!   is gone without having called `end` (a timeout, a signal, an abort).
//!   A runner ends a timed-out test's process tree, so the watcher leaves
//!   it as the holders do: it forks off, in a process group of its own.
//!
//! Only a process this test started counts: one under the test process,
//! or one left to launchd (pid 1) as holders are. A shell elsewhere whose
//! command line happens to name the folder, such as a `ps | grep`, is
//! left alone.
//!
//! [`CaseClock`] times each case. Every case also has a watch
//! ([`case_begins`]) that ends its world's processes once it has run past
//! [`CASE_LIMIT`]: whatever the case waits on then fails, so a
//! hang reports as that case's failure while the others go on, instead of
//! holding the whole run until the runner's timeout.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, getpgrp, kill_process, kill_process_group};

/// How long killed processes get to be gone before the test fails.
const END_WAIT: Duration = Duration::from_secs(10);

/// The longest one case runs before its world is ended as hung: about three
/// times the slowest passing case (about 60 s with other tests running
/// beside it), and past the longest single wait a case makes (`LONG`).
const CASE_LIMIT: Duration = Duration::from_secs(180);

/// How long a case past [`CASE_LIMIT`] gets, once its world is ended, to
/// fail on its own. One still running then waits on nothing its world
/// does, so the run stops there and says so, rather than at the runner's
/// timeout.
const CASE_GRACE: Duration = Duration::from_secs(60);

/// The watcher. Its command line names no folder, so it never lists
/// itself; the folders come in `LEFTOVER_DIRS`, one per line.
const WATCHER: &str = r#"
(
while kill -0 "$LEFTOVER_TEST" 2>/dev/null; do sleep 1; done
for round in 1 2 3 4 5; do
  pids=$(ps -A -o pid=,ppid=,stat=,command= | awk -v me="$$" '
    BEGIN { n = split(ENVIRON["LEFTOVER_DIRS"], dirs, "\n") }
    $3 ~ /^Z/ { next }
    {
      parent[$1] = $2
      if ($2 != 1) next
      for (i = 1; i <= n; i++) if (dirs[i] != "" && index($0, dirs[i])) mine[$1] = 1
    }
    END {
      grew = 1
      while (grew) {
        grew = 0
        for (p in parent) if (!(p in mine) && (parent[p] in mine)) { mine[p] = 1; grew = 1 }
      }
      for (p in mine) if (p != me) print p
    }')
  [ -z "$pids" ] && exit 0
  kill -KILL $pids 2>/dev/null
  sleep 1
done
) </dev/null >/dev/null 2>&1 &
echo $!
"#;

/// A process the world started, as `ps` lists it.
#[derive(Clone, Debug)]
struct Process {
    pid: i32,
    pgid: i32,
    command: String,
}

/// The processes a world's folders started, ended by [`Leftovers::end`].
pub struct Leftovers {
    dirs: Vec<PathBuf>,
    /// The forked-off watcher's pid.
    watcher: Option<i32>,
}

impl Leftovers {
    /// Watches for processes started from `dirs` (a world's temp folders)
    /// and starts the watcher that ends them if the test process dies.
    pub fn watch(dirs: &[&Path]) -> Self {
        use std::os::unix::process::CommandExt;
        let dirs: Vec<PathBuf> = dirs.iter().map(|dir| dir.to_path_buf()).collect();
        let listed = dirs
            .iter()
            .map(|dir| dir.display().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        // A group of its own: a runner that kills the test's group on a
        // timeout doesn't kill the watcher with it.
        let watcher = Command::new("/bin/sh")
            .arg("-c")
            .arg(WATCHER)
            .env("LEFTOVER_TEST", std::process::id().to_string())
            .env("LEFTOVER_DIRS", listed)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .output()
            .ok()
            .and_then(|output| String::from_utf8_lossy(&output.stdout).trim().parse().ok());
        Self { dirs, watcher }
    }

    /// Kills every process started from the folders, and every process
    /// under one, until none is left. Answers the command lines of those
    /// still running after [`END_WAIT`].
    pub fn end(&mut self) -> Result<(), Vec<String>> {
        let own_group = getpgrp().as_raw_nonzero().get();
        let watcher = self.watcher;
        let deadline = Instant::now() + END_WAIT;
        let result = loop {
            let left = started_from(&self.dirs, &listing(), watcher);
            if left.is_empty() {
                break Ok(());
            }
            if Instant::now() > deadline {
                break Err(left.into_iter().map(|process| format!("{} {}", process.pid, process.command)).collect());
            }
            // Each pid comes from the listing just taken, so it is still
            // the process the world started.
            kill(&left, own_group);
            std::thread::sleep(Duration::from_millis(50));
        };
        // The watcher is still waiting on this process; nothing is left
        // for it. Its pid is checked against its script first.
        if let Some(pid) = self.watcher.take()
            && listing().get(&pid).is_some_and(|(_, process)| process.command.contains("LEFTOVER_TEST"))
            && let Some(pid) = Pid::from_raw(pid)
        {
            let _ = kill_process(pid, Signal::KILL);
        }
        result
    }
}

/// Kills each of `processes` and its process group, unless that group is
/// `own_group`, the test process's own.
fn kill(processes: &[Process], own_group: i32) {
    for process in processes {
        if process.pgid > 1
            && process.pgid != own_group
            && let Some(group) = Pid::from_raw(process.pgid)
        {
            let _ = kill_process_group(group, Signal::KILL);
        }
        if let Some(pid) = Pid::from_raw(process.pid) {
            let _ = kill_process(pid, Signal::KILL);
        }
    }
}

/// The watch over the case running on this thread: libtest runs each case
/// on a thread of its own, so the watch ends with the thread, however the
/// case ends. Every case gets one from [`case_begins`] (the live flows'
/// `runtime()`), whether or not its daemon comes from a [`World`]; a world
/// or fixture adds its folders with [`case_watches`].
///
/// [`World`]: super::world::World
struct CaseWatch {
    /// The folders whose processes are ended once the case is hung.
    dirs: std::sync::Arc<std::sync::Mutex<Vec<PathBuf>>>,
    /// Dropped as the case's thread ends, which ends the watch.
    _running: std::sync::mpsc::Sender<()>,
}

thread_local! {
    static CASE: std::cell::RefCell<Option<CaseWatch>> = const { std::cell::RefCell::new(None) };
}

/// Starts this case's watch, if it has none yet: past [`CASE_LIMIT`] from
/// now it ends the processes started from the folders the case added with
/// [`case_watches`], and past [`CASE_GRACE`] more it stops the run.
pub fn case_begins() {
    case_watches(&[]);
}

/// Adds `dirs` (a world's or fixture's folders) to the folders this case's
/// watch ends the processes of, starting the watch if the case has none.
pub fn case_watches(dirs: &[&Path]) {
    CASE.with(|case| {
        let mut case = case.borrow_mut();
        let watch = case.get_or_insert_with(|| watch_case(Instant::now(), CASE_LIMIT));
        let mut watched = watch.dirs.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        watched.extend(dirs.iter().map(|dir| dir.to_path_buf()));
    });
}

/// The watch of a case that began at `started` and is hung past `limit`,
/// on a thread of its own.
fn watch_case(started: Instant, limit: Duration) -> CaseWatch {
    let name = std::thread::current().name().unwrap_or("a case").to_owned();
    let dirs = std::sync::Arc::new(std::sync::Mutex::new(Vec::<PathBuf>::new()));
    let (running, ended) = std::sync::mpsc::channel::<()>();
    let folders = dirs.clone();
    std::thread::spawn(move || {
        let left = limit.saturating_sub(started.elapsed());
        if ended.recv_timeout(left) != Err(std::sync::mpsc::RecvTimeoutError::Timeout) {
            return;
        }
        // Straight to stderr, past libtest's capture, so the run's log
        // has it whether or not the case reports.
        let _ = writeln!(
            std::io::stderr(),
            "live flow {name}: still running after {} s, so it is taken as hung and its world's \
             processes are ended; it fails at whatever it waits on, and the other cases go on",
            limit.as_secs()
        );
        let dirs = folders.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        let own_group = getpgrp().as_raw_nonzero().get();
        for _ in 0..5 {
            let left = started_from(&dirs, &listing(), None);
            if left.is_empty() {
                break;
            }
            kill(&left, own_group);
            std::thread::sleep(Duration::from_millis(200));
        }
        if ended.recv_timeout(CASE_GRACE) != Err(std::sync::mpsc::RecvTimeoutError::Timeout) {
            return;
        }
        let _ = writeln!(
            std::io::stderr(),
            "live flow {name}: still running {} s after its world was ended, so it waits on \
             something in the test itself; the run stops here and fails this case, and the \
             shard's script runs the cases that had not ended again without it. The stacks below \
             show where its thread (named after it) is",
            CASE_GRACE.as_secs()
        );
        let _ = writeln!(std::io::stderr(), "{}", stacks());
        say_hung(&name);
        std::process::exit(101);
    });
    CaseWatch {
        dirs,
        _running: running,
    }
}

/// Names a case hung past its grace in the file `LIVE_FLOWS_HUNG` names, a
/// line a case, so the shard's script (`live_flows_shard.sh`) fails that one
/// case and runs the others again without it.
fn say_hung(name: &str) {
    let Some(path) = std::env::var_os(HUNG_FILE_VAR) else {
        return;
    };
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut file| writeln!(file, "{name}"));
    if let Err(error) = written {
        let _ = writeln!(
            std::io::stderr(),
            "live flow {name}: couldn't name it as hung in {}: {error}; the shard fails as a whole",
            std::path::Path::new(&path).display()
        );
    }
}

/// The variable that names the file hung cases are named in.
const HUNG_FILE_VAR: &str = "LIVE_FLOWS_HUNG";

/// One case's clock: how long its world took to come up and the case to
/// end, said once it ends. Its world's folders join the case's watch
/// ([`case_watches`]).
pub struct CaseClock {
    /// The test's name: libtest names each test's thread after it.
    name: String,
    started: Instant,
    up: Option<Duration>,
}

impl CaseClock {
    /// The clock of a case that began at `started`, with the processes
    /// started from `dirs`, the case's world's folders, under its watch.
    pub fn start(started: Instant, dirs: &[&Path]) -> Self {
        case_watches(dirs);
        Self {
            name: std::thread::current().name().unwrap_or("a case").to_owned(),
            started,
            up: None,
        }
    }

    /// The world is up: the rest is the case's own.
    pub fn up(&mut self) {
        self.up = Some(self.started.elapsed());
    }
}

impl Drop for CaseClock {
    fn drop(&mut self) {
        let up = self.up.map_or_else(|| "never came up".to_owned(), |up| format!("up in {:.1} s", up.as_secs_f64()));
        let failed = if std::thread::panicking() { ", failed" } else { "" };
        // Past libtest's capture, so a passing case's time shows too.
        let _ = writeln!(
            std::io::stderr(),
            "live flow timing: {} took {:.1} s (world {up}{failed})",
            self.name,
            self.started.elapsed().as_secs_f64()
        );
    }
}

/// Every thread's stack in this process, from a one-second `sample`, or
/// why there are none.
fn stacks() -> String {
    let file = std::env::temp_dir().join(format!("live-flows-{}.sample.txt", std::process::id()));
    let sampled = Command::new("/usr/bin/sample")
        .arg(std::process::id().to_string())
        .arg("1")
        .arg("-file")
        .arg(&file)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match (sampled, std::fs::read_to_string(&file)) {
        (Ok(status), Ok(text)) if status.success() => text,
        (Ok(status), _) => format!("(no stacks: /usr/bin/sample ended with {status})"),
        (Err(error), _) => format!("(no stacks: couldn't run /usr/bin/sample: {error})"),
    }
}

/// Every running process, by pid: (parent, process).
fn listing() -> BTreeMap<i32, (i32, Process)> {
    let Ok(output) = Command::new("/bin/ps")
        .args(["-A", "-o", "pid=,ppid=,pgid=,stat=,command="])
        .output()
    else {
        return BTreeMap::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let ppid = fields.next()?.parse().ok()?;
            let pgid = fields.next()?.parse().ok()?;
            // A zombie is gone already; only its parent can reap it.
            if fields.next()?.starts_with('Z') {
                return None;
            }
            let command = fields.collect::<Vec<_>>().join(" ");
            Some((pid, (ppid, Process { pid, pgid, command })))
        })
        .collect()
}

/// The processes this test started whose command lines name one of
/// `dirs` (under this process, or left to launchd), and every process
/// under one of those, leaving out this process and `watcher`.
fn started_from(dirs: &[PathBuf], listing: &BTreeMap<i32, (i32, Process)>, watcher: Option<i32>) -> Vec<Process> {
    let me = std::process::id() as i32;
    let names: Vec<String> = dirs.iter().map(|dir| dir.display().to_string()).collect();
    let ours = under(listing, BTreeSet::from([me]));
    let named: BTreeSet<i32> = listing
        .iter()
        .filter(|(pid, (ppid, process))| {
            (*ppid == 1 || ours.contains(pid)) && names.iter().any(|name| process.command.contains(name.as_str()))
        })
        .map(|(pid, _)| *pid)
        .collect();
    let mine = under(listing, named);
    mine.into_iter()
        .filter(|pid| *pid != me && Some(*pid) != watcher)
        .filter_map(|pid| listing.get(&pid).map(|(_, process)| process.clone()))
        .collect()
}

/// `roots` and every process under one of them.
fn under(listing: &BTreeMap<i32, (i32, Process)>, mut roots: BTreeSet<i32>) -> BTreeSet<i32> {
    loop {
        let more: Vec<i32> = listing
            .iter()
            .filter(|(pid, (ppid, _))| !roots.contains(pid) && roots.contains(ppid))
            .map(|(pid, _)| *pid)
            .collect();
        if more.is_empty() {
            return roots;
        }
        roots.extend(more);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder a case adds after its watch began is watched too: once the
    /// case is past its limit, a process started from it is ended.
    #[test]
    fn a_hung_case_has_the_processes_of_folders_added_after_its_watch_began_ended() {
        let dir = tempfile::Builder::new().prefix("h2w").tempdir_in("/tmp").expect("a temp folder");
        let watch = watch_case(Instant::now(), Duration::from_secs(1));
        let mut child = Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("sleep 600; : {}", dir.path().display()))
            .spawn()
            .expect("a sleeping shell starts");
        watch
            .dirs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(dir.path().to_path_buf());
        let deadline = Instant::now() + Duration::from_secs(20);
        let ended = loop {
            if let Some(status) = child.try_wait().expect("the shell can be waited on") {
                break Some(status);
            }
            if Instant::now() > deadline {
                break None;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        drop(watch);
        if ended.is_none() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let ended = ended.expect("the hung case's process was ended");
        assert!(!ended.success(), "it was killed, not left to finish: {ended}");
    }
}

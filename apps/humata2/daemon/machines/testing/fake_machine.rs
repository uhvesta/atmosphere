//! A machine reached over a fake `ssh`, for tests: no network and never the
//! real `~/.ssh`. The fake `ssh` checks the host key against the
//! `UserKnownHostsFile` it is given (a scan records it there), then runs
//! the remote command with `/bin/sh -c` in a home folder of the test's own,
//! with fake `uname`, `systemctl`, `loginctl`, `pgrep` and friends first on
//! PATH. Flags in the state folder change what the machine does:
//!
//! - `down`: every ssh fails as an unreachable host would (exit 255);
//! - `hang`: every ssh waits, once logged, until the flag is cleared or
//!   the test process that made the machine is gone;
//! - `holder`: a `humata2_holder` of the user runs;
//! - `uname`: what `uname -sm` prints (default `Linux x86_64`).
//!
//! [`FakeMachine::runner`] wraps [`SystemRunner`] so its commands find the
//! fake `ssh`; a daemon under test gets [`FakeMachine::ssh_dir`] first on
//! its PATH instead.

#![allow(dead_code)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use humata2_machines::{EnvValue, Invocation, Output, Purpose, RunError, Runner, SystemRunner};

/// Puts an executable file holding `text` at `path`, in place of any file
/// there: a hard link to the one file of this process with that text,
/// written and run once the first time it is asked for.
///
/// macOS checks a new executable file the first time it runs, one file at
/// a time across the whole machine, at about a tenth of a second each, and
/// links to it that run before the first check ends are each checked too.
/// A copy per machine queues each test behind every other test's checks; a
/// link to a file that already ran skips the check. So the file runs once
/// as it is written, with no arguments, nothing on stdin and `HOME` and
/// `STATE` in a folder of its own. Every script given here does nothing
/// harmful run that way: the ssh scripts refuse to run outside their
/// machine's folder, and a fake tool reads or writes only `STATE`. The
/// shared file is read-only, so a write through a link fails instead of
/// changing every machine's copy.
pub fn shared_executable(path: &Path, text: &str) {
    static FOLDER: OnceLock<tempfile::TempDir> = OnceLock::new();
    static FILES: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    let folder = FOLDER.get_or_init(|| {
        // Bazel's own temporary folder when there is one: it is emptied
        // after the test.
        let parent = std::env::var_os("TEST_TMPDIR").map_or_else(std::env::temp_dir, PathBuf::from);
        tempfile::Builder::new().prefix("h2x").tempdir_in(parent).unwrap()
    });
    let mut files = FILES.get_or_init(Mutex::default).lock().unwrap();
    let count = files.len();
    let shared = files
        .entry(text.to_owned())
        .or_insert_with(|| {
            let shared = folder.path().join(count.to_string());
            fs::write(&shared, text).unwrap();
            fs::set_permissions(&shared, fs::Permissions::from_mode(0o555)).unwrap();
            let scratch = folder.path().join(format!("{count}.run"));
            fs::create_dir_all(&scratch).unwrap();
            // Its exit says nothing here; only that it ran matters.
            let _ = std::process::Command::new(&shared)
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("HOME", &scratch)
                .env("STATE", &scratch)
                .current_dir(&scratch)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            shared
        })
        .clone();
    drop(files);
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("couldn't replace {}: {error}", path.display()),
    }
    // Another volume can't take a link: a copy of its own then.
    if fs::hard_link(&shared, path).is_err() {
        fs::copy(&shared, path).unwrap();
    }
}

/// The host key the fake machine presents.
pub const HOST_KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";

const SSH: &str = r#"#!/bin/sh
# The fake machine's ssh. Options are read; nothing reaches the network.
# The machine's folders are beside the `ssh` folder this file is in, so
# every machine of a test runs this same file.
root=${0%/ssh/ssh}
if [ "$root" = "$0" ]; then
  echo "fake ssh: run as $0, not as <machine>/ssh/ssh, so its machine is unknown" >&2
  exit 255
fi
state=$root/state; home=$root/home; bin=$root/bin
khf=""; alias=""; strict=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o) shift
        case "$1" in
          UserKnownHostsFile=*) khf=${1#UserKnownHostsFile=}; khf=${khf#\"}; khf=${khf%\"} ;;
          HostKeyAlias=*) alias=${1#HostKeyAlias=} ;;
          StrictHostKeyChecking=*) strict=${1#StrictHostKeyChecking=} ;;
        esac ;;
    --) shift; break ;;
  esac
  shift
done
host=$1; shift
printf '%s\n' "$*" >> "$state/ssh.log"
# A test killed with the flag set leaves its folder behind: the wait ends
# with the test, not only with the flag.
while [ -e "$state/hang" ] && kill -0 __OWNER__ 2>/dev/null; do /bin/sleep 0.05; done
if [ -e "$state/down" ]; then
  echo "ssh: connect to host $host port 22: Connection refused" >&2
  exit 255
fi
key=$(cat "$state/hostkey")
if [ "$strict" = accept-new ]; then
  printf '%s %s\n' "$alias" "$key" >> "$khf"
  exit 0
fi
if ! grep -qxF "$alias $key" "$khf" 2>/dev/null; then
  echo "No ED25519 host key is known for $alias and you have requested strict checking." >&2
  echo "Host key verification failed." >&2
  exit 255
fi
cd "$home" || exit 255
exec env -i HOME="$home" PATH="$bin:/usr/bin:/bin" STATE="$state" XDG_RUNTIME_DIR="$state" USER=fake LOGNAME=fake /bin/sh -c "$1"
"#;

/// The machine's own tools, each a `/bin/sh` script in its `bin` folder.
const FAKES: [(&str, &str); 12] = [
    (
        "uname",
        r#"u=$(cat "$STATE/uname" 2>/dev/null || echo "Linux x86_64")
case "$1" in
  -s) echo "${u% *}" ;;
  -m) echo "${u#* }" ;;
  *) echo "$u" ;;
esac"#,
    ),
    (
        "systemctl",
        r#"case "$*" in
  *show-environment*) exit 0 ;;
  *is-active*) [ -e "$STATE/active" ] ;;
  *start*) touch "$STATE/active" ;;
  *stop*|*disable*) rm -f "$STATE/active" ;;
  *) exit 0 ;;
esac"#,
    ),
    (
        "loginctl",
        r#"case "$1" in
  show-user) [ -e "$STATE/linger" ] && echo yes || echo no ;;
  enable-linger) touch "$STATE/linger" ;;
esac"#,
    ),
    ("journalctl", "echo journal"),
    (
        "crontab",
        r#"if [ "$1" = -l ]; then [ -e "$STATE/crontab" ] && cat "$STATE/crontab"; else cat > "$STATE/crontab.new" && mv "$STATE/crontab.new" "$STATE/crontab"; fi"#,
    ),
    ("pkill", "exit 1"),
    (
        "pgrep",
        r#"case "$*" in
  *humata2_holder*) [ -e "$STATE/holder" ] ;;
  *) exit 1 ;;
esac"#,
    ),
    ("setsid", "exit 0"),
    ("sleep", "exit 0"),
    ("git", "echo 'git version 2.50.0'"),
    // Linux's `sync FILE` flushes that file; macOS's ignores the file and
    // flushes the whole computer, one caller at a time, so every install
    // waited behind every other test's writes. A power cut in these tests
    // is a killed process, which loses no write the OS took, so nothing
    // here depends on a flush.
    ("sync", "exit 0"),
    // Linux's own sha256sum, not macOS's `shasum`, a Perl script.
    (
        "sha256sum",
        r#"if [ -x /sbin/sha256sum ]; then exec /sbin/sha256sum "$@"; fi
exec shasum -a 256 "$@""#,
    ),
];

/// One fake machine and the host-side folder holding its `ssh`.
pub struct FakeMachine {
    _folder: tempfile::TempDir,
    home: PathBuf,
    state: PathBuf,
    ssh_dir: PathBuf,
}

impl FakeMachine {
    pub fn new() -> Self {
        let folder = tempfile::tempdir().unwrap();
        // Canonical, so paths the scripts print match the ones tests build.
        let root = folder.path().canonicalize().unwrap();
        let home = root.join("home");
        let state = root.join("state");
        let bin = root.join("bin");
        let ssh_dir = root.join("ssh");
        for dir in [&home, &state, &bin, &ssh_dir] {
            fs::create_dir_all(dir).unwrap();
        }
        for (name, body) in FAKES {
            shared_executable(&bin.join(name), &format!("#!/bin/sh\n{body}\n"));
        }
        shared_executable(&ssh_dir.join("ssh"), &SSH.replace("__OWNER__", &std::process::id().to_string()));
        fs::write(state.join("hostkey"), HOST_KEY).unwrap();
        Self {
            _folder: folder,
            home,
            state,
            ssh_dir,
        }
    }

    /// The folder to put first on PATH so `ssh` is the fake one.
    pub fn ssh_dir(&self) -> &Path {
        &self.ssh_dir
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    /// `~/.humata2/bin` on the machine.
    pub fn bin(&self) -> PathBuf {
        self.home.join(".humata2/bin")
    }

    pub fn installed(&self, name: &str) -> Option<Vec<u8>> {
        fs::read(self.bin().join(name)).ok()
    }

    pub fn unit_installed(&self) -> bool {
        self.home.join(".config/systemd/user/humatad2.service").exists()
    }

    pub fn set(&self, flag: &str) {
        fs::write(self.state.join(flag), "").unwrap();
    }

    pub fn clear(&self, flag: &str) {
        let _ = fs::remove_file(self.state.join(flag));
    }

    pub fn set_uname(&self, uname: &str) {
        fs::write(self.state.join("uname"), uname).unwrap();
    }

    /// Puts a tool first on the machine's PATH: `name` runs `body` with
    /// `/bin/sh`, in place of the default fake or the system's own.
    pub fn set_tool(&self, name: &str, body: &str) {
        shared_executable(&self.home.with_file_name("bin").join(name), &format!("#!/bin/sh\n{body}\n"));
    }

    /// Every remote command line ssh was asked to run, one per line.
    pub fn ssh_log(&self) -> String {
        fs::read_to_string(self.state.join("ssh.log")).unwrap_or_default()
    }

    /// The enrollment key the host wrote on the machine, if any.
    pub fn enrollment_key(&self) -> Option<Vec<u8>> {
        fs::read(self.enrollment_key_path()).ok()
    }

    pub fn enrollment_key_path(&self) -> PathBuf {
        self.home.join(".humata2/state/enrollment-key")
    }

    /// The remote command lines ssh was asked to run, scans included.
    pub fn ssh_calls(&self) -> usize {
        fs::read_to_string(self.state.join("ssh.log"))
            .map(|log| log.lines().count())
            .unwrap_or(0)
    }

    /// A runner whose commands find the fake `ssh`.
    pub fn runner(&self) -> FakeSshRunner {
        FakeSshRunner {
            path: format!("{}:/usr/bin:/bin", self.ssh_dir.display()),
            fault: None,
            calls: AtomicUsize::new(0),
            purposes: Mutex::new(Vec::new()),
            down_flag: self.state.join("down"),
        }
    }
}

impl Default for FakeMachine {
    fn default() -> Self {
        Self::new()
    }
}

/// What happens at the faulted call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// The host is killed: the call never returns (it panics).
    HostKilled,
    /// The machine goes away for that call and every one after.
    MachineKilled,
}

/// [`SystemRunner`] with the fake `ssh` on PATH, and an optional fault at
/// the `n`th call (from 0).
pub struct FakeSshRunner {
    path: String,
    fault: Option<(usize, Fault)>,
    calls: AtomicUsize,
    purposes: Mutex<Vec<Purpose>>,
    down_flag: PathBuf,
}

impl FakeSshRunner {
    pub fn fault_at(mut self, call: usize, fault: Fault) -> Self {
        self.fault = Some((call, fault));
        self
    }

    /// How many commands it ran (or was asked to).
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// How many commands for `purpose` it ran.
    pub fn ran(&self, purpose: Purpose) -> usize {
        self.purposes.lock().unwrap().iter().filter(|&&p| p == purpose).count()
    }
}

impl Runner for FakeSshRunner {
    fn run(&self, invocation: &Invocation) -> Result<Output, RunError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        self.purposes.lock().unwrap().push(invocation.purpose);
        match self.fault {
            Some((at, Fault::HostKilled)) if at == call => {
                panic!("the host was killed at call {call}")
            }
            Some((at, Fault::MachineKilled)) if at == call => fs::write(&self.down_flag, "").unwrap(),
            _ => {}
        }
        let mut invocation = invocation.clone();
        invocation
            .env
            .push(("PATH".to_owned(), EnvValue::Value(self.path.clone())));
        SystemRunner::new(&()).run(&invocation)
    }
}

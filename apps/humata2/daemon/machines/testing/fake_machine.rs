//! A machine reached over a fake `ssh`, for tests: no network and never the
//! real `~/.ssh`. The fake `ssh` checks the host key against the
//! `UserKnownHostsFile` it is given (a scan records it there), then runs
//! the remote command with `/bin/sh -c` in a home folder of the test's own,
//! with fake `uname`, `systemctl`, `loginctl`, `pgrep` and friends first on
//! PATH. Flags in the state folder change what the machine does:
//!
//! - `down`: every ssh fails as an unreachable host would (exit 255);
//! - `hang`: every ssh waits, once logged, until the flag is cleared;
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
use std::sync::Mutex;

use humata2_machines::{EnvValue, Invocation, Output, Purpose, RunError, Runner, SystemRunner};

/// The host key the fake machine presents.
pub const HOST_KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";

const SSH: &str = r#"#!/bin/sh
# The fake machine's ssh. Options are read; nothing reaches the network.
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
printf '%s\n' "$*" >> "__STATE__/ssh.log"
while [ -e "__STATE__/hang" ]; do /bin/sleep 0.05; done
if [ -e "__STATE__/down" ]; then
  echo "ssh: connect to host $host port 22: Connection refused" >&2
  exit 255
fi
key=$(cat "__STATE__/hostkey")
if [ "$strict" = accept-new ]; then
  printf '%s %s\n' "$alias" "$key" >> "$khf"
  exit 0
fi
if ! grep -qxF "$alias $key" "$khf" 2>/dev/null; then
  echo "No ED25519 host key is known for $alias and you have requested strict checking." >&2
  echo "Host key verification failed." >&2
  exit 255
fi
cd "__HOME__" || exit 255
exec env -i HOME="__HOME__" PATH="__BIN__:/usr/bin:/bin" STATE="__STATE__" XDG_RUNTIME_DIR="__STATE__" USER=fake LOGNAME=fake /bin/sh -c "$1"
"#;

const FAKES: [(&str, &str); 10] = [
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
        let write = |path: PathBuf, text: String| {
            fs::write(&path, text).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        };
        for (name, body) in FAKES {
            write(bin.join(name), format!("#!/bin/sh\n{body}\n"));
        }
        let ssh = SSH
            .replace("__STATE__", &state.display().to_string())
            .replace("__HOME__", &home.display().to_string())
            .replace("__BIN__", &bin.display().to_string());
        write(ssh_dir.join("ssh"), ssh);
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

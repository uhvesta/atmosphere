//! The app's own path against a REAL machine (cases real/02 and real/03):
//! the app (`Ui` on live clients) -> the host's `humatad2` on this Mac
//! (admin build) -> the real `/usr/bin/ssh` -> `humatad2 --role machine`
//! (the release linux-aarch64 build, installed by the host's own Add
//! machine flow) and its holder on the shared Lima VM `h2-terminal-c1` -> a
//! real `bash`. Headless: no window is shown.
//!
//! GATE: each test returns at once, printing why, unless
//! `TERMINAL_LAB_SSH_CONFIG` names the VM's Lima `ssh.config`
//! (`TERMINAL_LAB_SSH_HOST` is its host, default `lima-h2-terminal-c1`).
//! With it set, `HUMATA2_TOOLS` must name the tools bundle (a folder with
//! `manifest.json` and `linux-aarch64/`): the bazel target passes it.
//!
//! ISOLATION: the VM is shared, so each run makes its own throwaway Linux
//! user (`h2e` and 8 hex digits) whose home is a sparse 3 GiB ext4 image
//! mounted under `/var/tmp/h2e-<user>`: the Add flow installs into `$HOME`,
//! starts a systemd --user service and kills by uid, and the 2 GiB free
//! check passes while the root disk pays only for what is written. The
//! host side has its own temp folder, state, `known_hosts`-less ssh config
//! (one `Host` block copied from Lima's, with this user) and a `ssh` wrapper
//! first on the daemon's PATH, which is how the test holds the link down.
//! Dropping the fixture removes the user, the mount and the files. The VM
//! is never stopped or restarted. Runs are serialised (one CPU).
//!
//! No test sleeps: waits end on what the app, the host or the VM shows.
//! Checks read answers the shell computes (`| tr a-z A-Z` capitals), and
//! words start with `xq`, as in `typing.rs`.
//!
//! Run, after `./terminal-ssh-demo.sh start`:
//!
//! ```sh
//! bazel test //apps/humata2/desktop:live_flows_ssh_test \
//!   --test_env=TERMINAL_LAB_SSH_CONFIG="$HOME/.lima/h2-terminal-c1/ssh.config" \
//!   --test_output=streamed
//! ```
//! See `terminal/platform_test/SSH_DEMO.md`.
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::test::{ClickOptions, TestWindowExt as _};
use gpui_kit::{Role, SharedString, px, size};
use humata2_app::{AppAction, Route};
use humata2_clients::Clients;
use humata2_clients::model::{Agent, ChannelId, MachineId, SessionId};
use humata2_flow::Action;
use humata2_grpc::humata::v1 as pb;
use hyper_util::rt::TokioIo;
use sha2::{Digest, Sha256};
use tokio::net::UnixStream;
use tonic::transport::{Channel, Endpoint, Uri};

use super::typing::{UPPER, answered, count, key, run, shows, wait_for, wait_gone};
use super::world::{LONG, WorldError, live_clients, spawn_ready};
use super::{App, runtime};

/// The most the install and the machine's first link take: the add sends
/// real binaries over a 1-CPU VM's ssh.
const INSTALL: Duration = Duration::from_secs(300);
/// What the tab says while its machine is out of reach.
const LOST: &str = "Lost the connection to";
/// The host name the daemon is told to add; the temp ssh config defines it.
const SSH_ALIAS: &str = "h2e2e-vm";
/// The least free space on the VM's disk before a run starts.
const NEED_FREE_MIB: u64 = 600;
/// Runs share one small VM: one at a time.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
static RUNS: AtomicU64 = AtomicU64::new(0);

/// Why the VM fixture couldn't be set up or driven. Tests stop on any of
/// these.
#[derive(Debug, thiserror::Error)]
enum VmError {
    /// A needed setting is missing from the test's environment.
    #[error("{what}: {fix}")]
    Setup { what: String, fix: String },
    /// A file or folder on this Mac couldn't be made or read.
    #[error("{what}: {source}")]
    Io {
        what: String,
        #[source]
        source: std::io::Error,
    },
    /// The VM's ssh config doesn't hold what the fixture copies from it.
    #[error("the ssh config {} is not usable: {problem}; start the VM with ./terminal-ssh-demo.sh start and pass its ssh.config", path.display())]
    Config { path: PathBuf, problem: String },
    /// `ssh` couldn't be started.
    #[error("couldn't run /usr/bin/ssh for the step {step:?}: {source}")]
    Spawn {
        step: String,
        #[source]
        source: std::io::Error,
    },
    /// A command on the VM failed.
    #[error("the step {step:?} failed on the VM ({status}): {stderr}\nIs the VM running? ./terminal-ssh-demo.sh status")]
    Remote {
        step: String,
        status: String,
        stderr: String,
    },
    /// The VM isn't the arm64 Debian the tools bundle is built for.
    #[error("the VM reports {found:?}, not aarch64: use the h2-terminal-c1 VM from ./terminal-ssh-demo.sh")]
    Arch { found: String },
    /// The VM's disk is too full for a run.
    #[error("the VM has {free_mib} MiB free under /var/tmp and a run needs {need_mib} MiB: free space on the VM (remove old /var/tmp/h2e-* folders or files you put there); the VM is never grown or restarted here, other runs share it")]
    DiskLow { free_mib: u64, need_mib: u64 },
    /// The VM lacks a program the test types.
    #[error("the VM has no {tool}: install it there, e.g. ssh -F <ssh.config> <host> sudo apt-get install -y vim-tiny less (check `df -h /` first, the disk is 4 GiB)")]
    NoTool { tool: &'static str },
    /// The daemon or a client could not start.
    #[error("the step {step:?} failed: {source}")]
    World {
        step: &'static str,
        #[source]
        source: WorldError,
    },
    /// The host's answer to an add wasn't what the flow promises.
    #[error("adding the VM: {step} failed: {detail}")]
    Link { step: &'static str, detail: String },
}

fn io(what: impl Into<String>) -> impl FnOnce(std::io::Error) -> VmError {
    let what = what.into();
    move |source| VmError::Io { what, source }
}

/// `text` as one word for a POSIX shell.
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// The ssh the test uses as the VM's admin (the Lima user, who has
/// passwordless sudo).
struct Admin {
    config: PathBuf,
    host: String,
}

impl Admin {
    /// Runs `script` on the VM as the Lima user and returns what it printed.
    fn run(&self, step: &str, script: &str) -> Result<String, VmError> {
        let output = Command::new("/usr/bin/ssh")
            .arg("-F")
            .arg(&self.config)
            .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=10"])
            .arg(&self.host)
            .arg(format!("sh -c {}", quote(script)))
            .stdin(Stdio::null())
            .output()
            .map_err(|source| VmError::Spawn {
                step: step.to_owned(),
                source,
            })?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(VmError::Remote {
                step: step.to_owned(),
                status: output.status.to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            })
        }
    }
}

/// The shell function that removes one run's user, mount and files. Used by
/// the sweep of old runs and by Drop; every step is best effort.
const CLEANUP: &str = r#"h2e_cleanup() {
  u=$1; base=/var/tmp/h2e-$u
  sudo loginctl disable-linger "$u" 2>/dev/null || true
  sudo loginctl terminate-user "$u" 2>/dev/null || true
  sudo pkill -KILL -u "$u" 2>/dev/null || true
  i=0
  while sudo pgrep -u "$u" >/dev/null 2>&1 && [ "$i" -lt 50 ]; do i=$((i+1)); sleep 0.2; done
  sudo umount "$base/home" 2>/dev/null || sudo umount -l "$base/home" 2>/dev/null || true
  sudo userdel "$u" 2>/dev/null || true
  sudo rm -rf --one-file-system "$base"
}
"#;

/// A random-looking 8 hex digits: this process, the clock and a counter,
/// hashed. Two tests in one process never share one.
fn unique_hex() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_nanos())
        .unwrap_or_default();
    let n = RUNS.fetch_add(1, Ordering::SeqCst);
    let digest = Sha256::digest(format!("{}-{nanos}-{n}", std::process::id()).as_bytes());
    digest.iter().take(4).map(|byte| format!("{byte:02x}")).collect()
}

/// Where a line of Lima's ssh config goes: kept for the temp config, or
/// dropped.
const KEPT: [&str; 9] = [
    "hostname",
    "port",
    "identityfile",
    "identitiesonly",
    "preferredauthentications",
    "ciphers",
    "compression",
    "gssapiauthentication",
    "nohostauthenticationforlocalhost",
];

/// The `Host` block of Lima's ssh config for `host`, as (key, value) lines.
fn lima_block(path: &Path, host: &str) -> Result<Vec<(String, String)>, VmError> {
    let text = std::fs::read_to_string(path).map_err(io(format!("read {}", path.display())))?;
    let mut inside = false;
    let mut found = false;
    let mut lines = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some((key, value)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let value = value.trim();
        if key.eq_ignore_ascii_case("host") {
            inside = value.split_whitespace().any(|name| name == host);
            found |= inside;
        } else if inside {
            lines.push((key.to_owned(), value.to_owned()));
        }
    }
    if !found {
        return Err(VmError::Config {
            path: path.to_owned(),
            problem: format!("it has no Host block for {host}; set TERMINAL_LAB_SSH_HOST to the host name in it"),
        });
    }
    Ok(lines)
}

/// The public keys beside the config's identity files.
fn public_keys(config: &Path, block: &[(String, String)]) -> Result<Vec<String>, VmError> {
    let mut keys = Vec::new();
    for (key, value) in block {
        if !key.eq_ignore_ascii_case("identityfile") {
            continue;
        }
        let plain = value.trim_matches('"');
        let identity = match plain.strip_prefix("~/") {
            Some(rest) => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(rest),
            None => PathBuf::from(plain),
        };
        let mut public = identity.into_os_string();
        public.push(".pub");
        let public = PathBuf::from(public);
        if let Ok(text) = std::fs::read_to_string(&public) {
            keys.push(text.trim().to_owned());
        }
    }
    if keys.is_empty() {
        return Err(VmError::Config {
            path: config.to_owned(),
            problem: "none of its IdentityFile lines has a .pub file beside it".to_owned(),
        });
    }
    Ok(keys)
}

/// The VM, one throwaway user on it, and the host daemon (admin build) that
/// reaches it over the real `ssh`.
struct Vm {
    admin: Admin,
    user: String,
    base: String,
    name: String,
    dir: tempfile::TempDir,
    daemon: Option<Child>,
    /// Released last, after the VM is clean.
    _one_at_a_time: MutexGuard<'static, ()>,
}

impl Vm {
    /// Checks the VM, makes the user, starts the host daemon and adds the
    /// VM as a machine; returns once the host shows it online.
    async fn start(config: PathBuf) -> Result<Self, VmError> {
        let tools = std::env::var_os("HUMATA2_TOOLS").map(PathBuf::from);
        let tools = match tools.and_then(|path| std::fs::canonicalize(path).ok()) {
            Some(tools) if tools.join("linux-aarch64").is_dir() && tools.join("manifest.json").is_file() => tools,
            _ => {
                return Err(VmError::Setup {
                    what: "HUMATA2_TOOLS does not name a tools bundle with linux-aarch64/".to_owned(),
                    fix: "run //apps/humata2/desktop:live_flows_ssh_test, which passes it".to_owned(),
                });
            }
        };
        let host = std::env::var("TERMINAL_LAB_SSH_HOST").unwrap_or_else(|_| "lima-h2-terminal-c1".to_owned());
        let guard = ONE_AT_A_TIME.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let hex = unique_hex();
        let user = format!("h2e{hex}");
        let dir = tempfile::Builder::new()
            .prefix("h2e")
            .tempdir_in("/tmp")
            .map_err(io("make the host temp folder"))?;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
            .map_err(io("restrict the host temp folder"))?;
        // Built before anything on the VM exists, so a failure below still
        // cleans up through Drop.
        let mut vm = Self {
            admin: Admin { config: config.clone(), host },
            base: format!("/var/tmp/h2e-{user}"),
            user,
            name: format!("vm-{hex}"),
            dir,
            daemon: None,
            _one_at_a_time: guard,
        };
        vm.preflight()?;
        vm.sweep()?;
        let block = lima_block(&config, &vm.admin.host)?;
        let keys = public_keys(&config, &block)?;
        vm.make_user(&keys)?;
        vm.write_host_files(&config, &block)?;
        vm.spawn_daemon(&tools).await?;
        vm.add_machine().await?;
        Ok(vm)
    }

    /// The VM is arm64, has room, and has the programs the tests type.
    fn preflight(&self) -> Result<(), VmError> {
        let arch = self.admin.run("check the VM's architecture", "uname -m")?;
        if arch.trim() != "aarch64" {
            return Err(VmError::Arch { found: arch.trim().to_owned() });
        }
        let free = self
            .admin
            .run("check the VM's free disk", "df -Pk /var/tmp | awk 'NR==2{print $4}'")?;
        let free_mib = free.trim().parse::<u64>().unwrap_or(0) / 1024;
        if free_mib < NEED_FREE_MIB {
            return Err(VmError::DiskLow { free_mib, need_mib: NEED_FREE_MIB });
        }
        for tool in ["vim", "less"] {
            self.admin
                .run("look for a program", &format!("command -v {tool} >/dev/null"))
                .map_err(|_| VmError::NoTool { tool })?;
        }
        Ok(())
    }

    /// Removes runs left over from more than a day ago.
    fn sweep(&self) -> Result<(), VmError> {
        let script = format!(
            "{CLEANUP}for dir in $(find /var/tmp -maxdepth 1 -name 'h2e-*' -mmin +1440); do u=${{dir#/var/tmp/h2e-}}; case \"$u\" in h2e????????) h2e_cleanup \"$u\" ;; esac; done"
        );
        self.admin.run("remove old runs from the VM", &script).map(drop)
    }

    /// The throwaway user: its home is a mounted sparse image, it takes the
    /// test's keys, and its systemd --user manager runs.
    fn make_user(&self, keys: &[String]) -> Result<(), VmError> {
        let keys = keys.join("\n");
        let script = format!(
            r#"set -eu
u={user}; base={base}
sudo mkdir -p "$base/home"
sudo truncate -s 3G "$base/home.img"
sudo /usr/sbin/mkfs.ext4 -q -F -m 0 "$base/home.img"
sudo mount -o loop "$base/home.img" "$base/home"
sudo /usr/sbin/useradd -M -d "$base/home" -s /bin/bash "$u"
sudo chown "$u:" "$base/home"
sudo chmod 700 "$base/home"
sudo install -d -m 700 -o "$u" -g "$u" "$base/home/.ssh"
printf '%s\n' {keys} | sudo tee "$base/home/.ssh/authorized_keys" >/dev/null
sudo chown "$u:" "$base/home/.ssh/authorized_keys"
sudo chmod 600 "$base/home/.ssh/authorized_keys"
sudo loginctl enable-linger "$u"
uid=$(id -u "$u")
i=0
until systemctl is-active --quiet "user@$uid.service"; do
  i=$((i+1))
  [ "$i" -lt 150 ] || {{ echo "user@$uid.service never became active" >&2; exit 1; }}
  sleep 0.2
done
"#,
            user = quote(&self.user),
            base = quote(&self.base),
            keys = quote(&keys),
        );
        self.admin.run("make the throwaway user", &script).map(drop)
    }

    /// The ssh config for this user, the `ssh` wrapper that can hold the
    /// link down, and the folders the daemon is given.
    fn write_host_files(&self, config: &Path, block: &[(String, String)]) -> Result<(), VmError> {
        let root = self.dir.path();
        let mut text = format!("Host {SSH_ALIAS}\n");
        for (key, value) in block {
            if KEPT.contains(&key.to_ascii_lowercase().as_str()) {
                text.push_str(&format!("  {key} {value}\n"));
            }
        }
        if !block.iter().any(|(key, _)| key.eq_ignore_ascii_case("hostname")) {
            return Err(VmError::Config {
                path: config.to_owned(),
                problem: "its Host block has no Hostname".to_owned(),
            });
        }
        text.push_str(&format!("  User {}\n", self.user));
        std::fs::write(root.join("ssh_config"), text).map_err(io("write the ssh config"))?;
        for folder in ["bin", "home", "skills-home"] {
            std::fs::create_dir_all(root.join(folder)).map_err(io(format!("make {folder}")))?;
        }
        let wrapper = root.join("bin").join("ssh");
        std::fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\nif [ -e '{hang}' ]; then echo 'ssh: the test holds the link down' >&2; exit 255; fi\nexec /usr/bin/ssh \"$@\"\n",
                hang = root.join("hang").display()
            ),
        )
        .map_err(io("write the ssh wrapper"))?;
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755))
            .map_err(io("make the ssh wrapper runnable"))?;
        Ok(())
    }

    fn env_binary(variable: &'static str) -> Result<PathBuf, VmError> {
        let path = std::env::var_os(variable).ok_or_else(|| VmError::Setup {
            what: format!("{variable} is not set"),
            fix: "run //apps/humata2/desktop:live_flows_ssh_test, which passes it".to_owned(),
        })?;
        std::fs::canonicalize(&path).map_err(io(format!("find {}", Path::new(&path).display())))
    }

    /// The admin host daemon, its own state and folders, reaching the VM
    /// through the wrapper `ssh`.
    async fn spawn_daemon(&mut self, tools: &Path) -> Result<(), VmError> {
        let root = self.dir.path().to_path_buf();
        let socket = self.socket();
        let mut command = std::process::Command::new(Self::env_binary("HUMATAD2")?);
        command
            .args(["serve", "--admin", "--tools"])
            .arg(tools)
            .arg("--ssh-config")
            .arg(root.join("ssh_config"))
            .arg("--skills-home")
            .arg(root.join("skills-home"))
            .arg("--channels")
            .arg(root.join("ch"))
            .arg("--socket")
            .arg(&socket)
            .arg("--state")
            .arg(root.join("state"))
            .arg("--root")
            .arg(&root)
            .arg("--holder")
            .arg(Self::env_binary("HUMATA2_HOLDER")?)
            .args(["--shell", "/bin/sh", "--shell-arg=-i"])
            .env("PATH", format!("{}:/usr/bin:/bin", root.join("bin").display()))
            .env("HOME", root.join("home"))
            .env("PS1", "$ ")
            .env("ENV", "/dev/null");
        let child = spawn_ready(command, &socket)
            .await
            .map_err(|source| VmError::World { step: "start the host daemon", source })?;
        self.daemon = Some(child);
        Ok(())
    }

    fn socket(&self) -> PathBuf {
        self.dir.path().join("d.sock")
    }

    /// Clients on the host daemon, the real machines client included.
    async fn clients(&self) -> Result<Clients, VmError> {
        live_clients(self.socket(), true)
            .await
            .map_err(|source| VmError::World { step: "connect to the host daemon", source })
    }

    async fn host_channel(&self) -> Result<Channel, VmError> {
        let path = self.socket();
        Endpoint::from_static("http://[::]:50051")
            .connect_with_connector(tower::service_fn(move |_: Uri| {
                let path = path.clone();
                async move { Ok::<_, std::io::Error>(TokioIo::new(UnixStream::connect(path).await?)) }
            }))
            .await
            .map_err(|error| VmError::Link {
                step: "connect to the host's socket",
                detail: format!("{error:?}"),
            })
    }

    fn link_error(step: &'static str, detail: impl std::fmt::Debug) -> VmError {
        VmError::Link { step, detail: format!("{detail:?}") }
    }

    /// Adds the VM as a person does: the first add stops at the host key,
    /// the second confirms it. Returns once the host lists the machine
    /// online with a daemon version (what the link's handshake said).
    async fn add_machine(&self) -> Result<(), VmError> {
        let channel = self.host_channel().await?;
        let mut feed = pb::event_service_client::EventServiceClient::new(channel.clone())
            .subscribe(pb::SubscribeRequest {
                topics: vec![pb::Topic::Machines as i32],
                ..Default::default()
            })
            .await
            .map_err(|status| Self::link_error("open the machines feed", status))?
            .into_inner();
        let mut machines = pb::machine_service_client::MachineServiceClient::new(channel.clone());
        let request = |step: &str, fingerprint: &str| pb::AddSshMachineRequest {
            idempotency_key: Some(pb::IdempotencyKey {
                value: format!("add-{}-{step}", self.name),
            }),
            ssh_host: SSH_ALIAS.to_owned(),
            name: self.name.clone(),
            root: String::new(),
            trust_host_key_fingerprint: fingerprint.to_owned(),
        };
        let first = machines
            .add_ssh_machine(request("ask", ""))
            .await
            .map_err(|status| Self::link_error("start the add", status))?
            .into_inner();
        let asked = Self::operation_ended(&channel, &first).await?;
        let Some(pb::error::Kind::HostKeyUnknown(key)) = asked.error.clone().and_then(|error| error.kind) else {
            return Err(Self::link_error("the first add stops at the host key", asked));
        };
        let added = machines
            .add_ssh_machine(request("trust", &key.presented_fingerprint))
            .await
            .map_err(|status| Self::link_error("start the confirmed add", status))?
            .into_inner();
        let ended = Self::operation_ended(&channel, &added).await?;
        if ended.state != pb::OperationState::Succeeded as i32 {
            return Err(Self::link_error("the confirmed add succeeds", ended));
        }
        // Listed, and listed again each time the feed tells of a change.
        let deadline = tokio::time::Instant::now() + INSTALL;
        loop {
            let listed = machines
                .list_machines(pb::ListMachinesRequest::default())
                .await
                .map_err(|status| Self::link_error("list the machines", status))?
                .into_inner()
                .machines;
            let linked = listed.iter().any(|machine| {
                machine.name == self.name
                    && machine.reachability == pb::Reachability::Online as i32
                    && !machine.daemon_version.is_empty()
            });
            if linked {
                return Ok(());
            }
            match tokio::time::timeout_at(deadline, feed.message()).await {
                Ok(Ok(Some(_))) => {}
                other => {
                    return Err(Self::link_error(
                        "the host shows the VM linked",
                        format!("{other:?}; the host last listed {listed:?}"),
                    ));
                }
            }
        }
    }

    /// The operation as its watch last showed it, once the watch ends.
    async fn operation_ended(channel: &Channel, operation: &pb::Operation) -> Result<pb::Operation, VmError> {
        let mut watch = pb::operation_service_client::OperationServiceClient::new(channel.clone())
            .watch_operation(pb::WatchOperationRequest {
                id: operation.id.clone(),
                from_version: 0,
            })
            .await
            .map_err(|status| Self::link_error("watch an operation", status))?
            .into_inner();
        let deadline = tokio::time::Instant::now() + INSTALL;
        let mut last = operation.clone();
        loop {
            match tokio::time::timeout_at(deadline, watch.message()).await {
                Ok(Ok(Some(next))) => last = next,
                Ok(Ok(None)) => return Ok(last),
                other => {
                    return Err(Self::link_error(
                        "an operation ends",
                        format!("{other:?}; it last was {last:?}"),
                    ));
                }
            }
        }
    }

    /// Runs `script` on the VM as the throwaway user, in its home, and
    /// returns what it printed.
    fn on_vm(&self, script: &str) -> Result<String, VmError> {
        let inner = format!("sudo -u {} -H sh -c {}", quote(&self.user), quote(script));
        self.admin.run("run a command as the throwaway user", &inner)
    }

    /// Ends the host's link as a dropped connection does, and keeps its
    /// redials failing until [`Self::restore_link`]. The flag is set first,
    /// so no dial can get through. Only this run's link is killed: its
    /// command line has this run's ssh config and the machine daemon's
    /// `serve --stdio`.
    fn cut_link(&self) {
        let _ = std::fs::write(self.dir.path().join("hang"), b"");
        let pattern = format!("-F {}/ssh_config.*serve --stdio", self.dir.path().display());
        let _ = Command::new("pkill").args(["-f", "--", &pattern]).status();
    }

    /// Lets the host's dials reach the VM again; the redial is the host's own.
    fn restore_link(&self) {
        let _ = std::fs::remove_file(self.dir.path().join("hang"));
    }
}

impl Drop for Vm {
    fn drop(&mut self) {
        if let Some(mut child) = self.daemon.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        // The links and anything the daemon started carry its folder in
        // their command lines.
        let _ = Command::new("pkill").arg("-f").arg(self.dir.path()).status();
        let script = format!("{CLEANUP}h2e_cleanup {}", quote(&self.user));
        let _ = self.admin.run("clean up the throwaway user", &script);
    }
}

/// Whether to run: the VM's ssh config, or a printed reason to skip.
fn gate() -> Option<PathBuf> {
    match std::env::var_os("TERMINAL_LAB_SSH_CONFIG") {
        Some(path) => Some(PathBuf::from(path)),
        None => {
            eprintln!(
                "skipped: set TERMINAL_LAB_SSH_CONFIG to the h2-terminal-c1 Lima ssh.config (…/.lima/h2-terminal-c1/ssh.config) and run //apps/humata2/desktop:live_flows_ssh_test; see terminal/platform_test/SSH_DEMO.md"
            );
            None
        }
    }
}

/// The VM fixture, clients on its host, the VM's machine and the channel
/// holding it and This Mac.
struct Rig {
    vm: Vm,
    clients: Clients,
    machine: MachineId,
    #[allow(dead_code)]
    channel: ChannelId,
}

async fn rig(config: PathBuf) -> Rig {
    let vm = Vm::start(config).await.expect("set up the VM and add it as a machine");
    let clients = vm.clients().await.expect("connect to the host");
    let workspace = clients.daemon.load().await.expect("load");
    let find = |pick: &dyn Fn(&str) -> bool| -> MachineId {
        workspace
            .machines
            .iter()
            .find(|machine| pick(&machine.name))
            .map(|machine| machine.id.clone())
            .expect("the host lists the machine")
    };
    let machine = find(&|name| name == vm.name);
    let mac = find(&|name| name != vm.name);
    // No repos: a shell session needs none. If the daemon refuses a
    // channel without one, this is where it says so.
    let channel = clients
        .daemon
        .create_channel("vm", vec![], vec![mac, machine.clone()])
        .await
        .expect("create a channel with This Mac and the VM");
    Rig { vm, clients, machine, channel }
}

fn activate(app: &mut App<'_>) {
    app.cx
        .update_window(app.window, |_, window, _| window.activate_window())
        .expect("the window stays open");
    app.settle();
}

fn has(app: &mut App<'_>, id: &str) -> bool {
    app.view();
    app.cx
        .update_window(app.window, |_, window, _| window.try_find(SharedString::from(id.to_owned())).is_some())
        .expect("the window stays open")
}

fn click_id(app: &mut App<'_>, id: &str) {
    app.cx
        .update_window(app.window, |_, window, cx| window.click(SharedString::from(id.to_owned()), cx))
        .expect("the window stays open");
    app.settle();
}

fn toasts_saying(app: &App<'_>, what: &str) -> usize {
    app.ui
        .read(|state| state.toasts.iter().filter(|toast| toast.text.contains(what)).count())
}

/// Clicks the terminal, so keys go to it (what a person does on a tab that
/// opened before the window did).
fn focus_terminal(app: &mut App<'_>) {
    app.cx
        .update_window(app.window, |_, window, cx| {
            window
                .within("terminal-area")
                .click_with_options("terminal", ClickOptions::new(), cx)
        })
        .expect("the window stays open");
    app.settle();
}

/// The values printed as `<tag><digits and x>q`: the run after `tag` that
/// holds a digit first and ends at a `q`. The typed line has `$$` or `$(`
/// after the tag, never digits, so only the shell's answers match.
fn tagged(view: &str, tag: &str) -> Vec<String> {
    view.match_indices(tag)
        .filter_map(|(at, _)| {
            let rest = &view[at + tag.len()..];
            let run: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == 'x').collect();
            let ends = rest[run.len()..].starts_with('q');
            (run.starts_with(|c: char| c.is_ascii_digit()) && ends).then_some(run)
        })
        .collect()
}

/// Has the shell print its own process id as `<tag><pid>q` and returns it.
fn shell_pid(app: &mut App<'_>, tag: &str) -> u32 {
    run(app, &format!("echo {tag}$$q"));
    let label = format!("the shell to print its process id as {tag}<pid>q");
    app.wait_within(&label, LONG, |app| {
        let view = app.view();
        !tagged(&view, tag).is_empty()
    });
    let view = app.view();
    tagged(&view, tag)
        .last()
        .and_then(|run| run.parse().ok())
        .expect("a process id of digits")
}

/// Real/02 step 1: the VM's machine page's Open a shell starts a Terminal
/// session on the VM; returns it once its prompt shows. Keys typed before
/// the first screen are refused (never queued), so it waits for a prompt.
fn open_a_shell_on_the_vm(app: &mut App<'_>, vm: &MachineId) -> SessionId {
    app.wait_within("the VM to load online in the app", LONG, |app| {
        app.ui
            .read(|state| state.workspace.machines.iter().any(|m| &m.id == vm && m.online))
    });
    app.cx
        .update(|cx| app.ui.send(AppAction::Navigate(Route::Machine(vm.clone())), cx));
    app.settle();
    app.click(Role::Button, "Open a shell");
    app.wait_within("the sheet to open on the VM as a Terminal", LONG, |app| {
        app.sheet_holds(|sheet| sheet.agent == Agent::Shell && sheet.machine.as_ref() == Some(vm))
    });
    app.click(Role::Button, "Start session");
    app.wait_within("the Terminal session to be listed", LONG, |app| {
        app.ui.read(|state| {
            state
                .workspace
                .sessions
                .iter()
                .any(|session| session.agent == Agent::Shell)
        })
    });
    let session = app
        .ui
        .read(|state| {
            state
                .workspace
                .sessions
                .iter()
                .find(|session| session.agent == Agent::Shell)
                .cloned()
        })
        .expect("listed after the wait");
    assert_eq!(&session.machine, vm, "the shell runs on the VM, not on This Mac");
    let id = session.id;
    app.wait_within("the session page to show", LONG, |app| {
        app.ui.read(|state| state.route == Route::Session(id.clone()))
    });
    // The bash prompt ends in `$`; no label of the app holds one.
    wait_for(app, "$");
    id
}

/// Real/02 steps 1 to 5, 7, 9, 10 and 12 in the app over real ssh: the
/// shell is on the VM, vim takes Esc and arrows, ⌃C stops a program, less
/// searches and quits, a one-line paste runs, a multi-line paste waits for
/// the confirm, and a window resize reaches the pty of the same shell.
#[test]
fn a_shell_on_the_vm_in_the_app_takes_keys_vim_less_paste_and_resize() {
    let Some(config) = gate() else { return };
    let runtime = runtime();
    let Rig { vm, clients, machine, .. } = runtime.block_on(rig(config));
    let mut app = App::open(&runtime, clients);
    activate(&mut app);
    open_a_shell_on_the_vm(&mut app, &machine);

    // Step 2: the shell answers, and it is the VM's: arm64. The shell's own
    // process id is the one the resize must find again.
    answered(&mut app, "xqup");
    run(&mut app, "uname -m | tr a-z A-Z");
    wait_for(&mut app, "AARCH64");
    let pid = shell_pid(&mut app, "xqpid");

    // Steps 3 and 4: vim over ssh. Esc leaves insert mode and two Left
    // arrows (cursor-key mode) move the cursor onto `b`, which `x` deletes;
    // `:wq` saves and the prompt returns, as `cat` of the file proves.
    run(&mut app, "vim -u NONE -i NONE -N xqnotes.txt");
    wait_for(&mut app, "[New");
    key(&mut app, "i");
    wait_for(&mut app, "-- INSERT --");
    app.type_text("xqabcd");
    key(&mut app, "escape");
    wait_gone(&mut app, "-- INSERT --");
    key(&mut app, "left");
    key(&mut app, "left");
    app.type_text("x");
    run(&mut app, ":wq");
    run(&mut app, &format!("cat xqnotes.txt{UPPER}"));
    wait_for(&mut app, "XQACD");

    // Step 5: ⌃C is the byte 0x03 for the program, not a copy.
    run(&mut app, "sleep 1000");
    key(&mut app, "ctrl-c");
    answered(&mut app, "xqafterc");

    // Step 7: less opens at the top, `/` finds a later line, `q` leaves.
    run(&mut app, "seq -f 'xqline%g' 1 400 > xqnums; less xqnums");
    wait_for(&mut app, "xqline20");
    assert!(!shows(&mut app, "xqline310"), "less shows the first page");
    app.type_text("/xqline300");
    key(&mut app, "enter");
    wait_for(&mut app, "xqline310");
    app.type_text("q");
    wait_gone(&mut app, "xqline310");
    answered(&mut app, "xqafterless");

    // Step 9: a one-line paste is typed at the prompt; Enter runs it.
    app.play(&Action::Paste("echo xqpasted | tr a-z A-Z".to_owned()));
    key(&mut app, "enter");
    wait_for(&mut app, "XQPASTED");

    // Step 10: three lines for `cat`, which did not ask for bracketed
    // paste, wait for the confirm; once confirmed `cat` echoes them.
    run(&mut app, "cat");
    app.play(&Action::Paste("xqone\nxqtwo\nxqthree\n".to_owned()));
    app.wait_within("the paste prompt", LONG, |app| has(app, "paste-prompt"));
    assert!(!shows(&mut app, "xqtwo"), "nothing is sent before the answer");
    click_id(&mut app, "paste-confirm");
    app.wait_within("cat to echo the pasted lines", LONG, |app| {
        count(app, "xqthree") >= 2
    });
    key(&mut app, "ctrl-d");
    answered(&mut app, "xqcatdone");

    // Step 12: a foreground bash prints the pty's width, then again when
    // the window changes it (SIGWINCH). `s` prints `<rows>x<cols>`.
    run(
        &mut app,
        r#"bash -c 's() { stty size </dev/tty | tr \\040 x; }; trap "echo xqcols\$(s)q" WINCH; echo xqwatch$(s)q; while :; do sleep 1 & wait $!; done'"#,
    );
    app.wait_within("the watcher to print the pty's size", LONG, |app| {
        let view = app.view();
        !tagged(&view, "xqwatch").is_empty()
    });
    let view = app.view();
    let wide: u32 = tagged(&view, "xqwatch")
        .last()
        .and_then(|run| run.split('x').nth(1))
        .and_then(|cols| cols.parse().ok())
        .expect("the watcher printed rows and columns");
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.resize(size(px(720.), px(760.)));
            window.bounds_changed(cx);
        })
        .expect("the window stays open");
    app.settle();
    app.wait_within("the pty to report fewer columns", LONG, |app| {
        let view = app.view();
        tagged(&view, "xqcols")
            .last()
            .and_then(|run| run.split('x').nth(1))
            .and_then(|cols| cols.parse::<u32>().ok())
            .is_some_and(|narrow| narrow < wide)
    });
    key(&mut app, "ctrl-c");
    let again = shell_pid(&mut app, "xqsame");
    assert_eq!(again, pid, "the resize reached the same shell, not a new one");
    drop(app);
    drop(vm);
}

/// Real/03 over real ssh: with the link held down the tab says so, typed
/// keys are refused once and never arrive, the shell and a job on the VM
/// live on, the host redials by itself and shows what the job printed; then
/// the app quits and starts again and its tab reattaches to the same shell.
#[test]
fn a_dropped_vm_link_reconnects_without_replay_and_an_app_restart_keeps_the_same_shell() {
    let Some(config) = gate() else { return };
    let runtime = runtime();
    let Rig { vm, clients, machine, .. } = runtime.block_on(rig(config));
    let mut app = App::open(&runtime, clients);
    activate(&mut app);
    let id = open_a_shell_on_the_vm(&mut app, &machine);
    answered(&mut app, "xqup");
    let pid = shell_pid(&mut app, "xqpid");

    // A job that prints only once the test lets it (a file in the user's
    // home on the VM), and a half-typed line.
    run(
        &mut app,
        "(while [ ! -e ~/xqgo ]; do sleep 0.2; done; echo xqwhile | tr a-z A-Z) &",
    );
    answered(&mut app, "xqjob");
    app.type_text("echo xqhalf");
    wait_for(&mut app, "echo xqhalf");
    assert!(!shows(&mut app, "XQWHILE"), "the job has not printed yet");

    // Step 1: the link is cut; the tab says the connection is lost.
    vm.cut_link();
    wait_for(&mut app, LOST);

    // Step 3: keys typed now are refused, told once, and never sent.
    let told = toasts_saying(&app, "not sent");
    app.type_text("xqlost");
    app.settle();
    assert_eq!(toasts_saying(&app, "not sent"), told + 1, "one toast for the refused keys");

    // Step 2: on the VM the shell is alive; the job is let go meanwhile.
    vm.on_vm("touch ~/xqgo").expect("touch the job's gate file on the VM");
    vm.on_vm(&format!("ps -p {pid} -o pid="))
        .expect("the shell is still alive on the VM while the link is down");

    // Steps 5 and 6: the host redials by itself; what the job printed
    // meanwhile shows.
    vm.restore_link();
    wait_gone(&mut app, LOST);
    wait_for(&mut app, "XQWHILE");
    assert_eq!(count(&mut app, "XQWHILE"), 1, "what the job printed while down shows once");

    // Step 8: typing reaches the same shell; its line holds `echo xqhalf`
    // and none of what was refused.
    app.type_text(UPPER);
    key(&mut app, "enter");
    wait_for(&mut app, "XQHALF");
    assert!(!shows(&mut app, "XQLOST"), "refused keys never arrive late");

    // Steps 9, 10 and 12: the app quits (the shell does not) and starts
    // again on the same host; its tab shows what was printed before, and
    // the shell answers as the same process.
    drop(app);
    let clients = runtime.block_on(vm.clients()).expect("connect again");
    let mut app = App::open(&runtime, clients);
    activate(&mut app);
    app.show_session(&id);
    // The screen is in once what was printed before the restart shows;
    // keys typed earlier would be refused.
    wait_for(&mut app, "XQHALF");
    focus_terminal(&mut app);
    let again = shell_pid(&mut app, "xqsame");
    assert_eq!(again, pid, "the reopened tab is the same shell, not a new one");
    answered(&mut app, "xqagain");
    assert_eq!(count(&mut app, "XQAGAIN"), 1, "the line typed after the restart ran once");
    drop(app);
    drop(vm);
}

//! A machine whose host comes and goes, for the AWAY tests: a role-machine
//! daemon, the host's end of its peer link (a real `serve --stdio` link
//! process and the Uplink the host holds), and a terminal of the test's own
//! in which the real `humata` runs and the test types at its prompts.
//!
//! Use it as `mod common; #[path = "common/away.rs"] mod away;`.

#![allow(dead_code)]

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{BorrowedFd, OwnedFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

use hmac::{Hmac, KeyInit, Mac};
use humata2_grpc::humata::v1 as pb;
use rustix::fs::{Mode, OFlags};
use rustix::pty::OpenptFlags;
use rustix::termios::QueueSelector;
use sha2::Sha256;
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::Streaming;
use tonic::transport::Channel;

use crate::common::{self, Build, Daemon, WAIT, binary, server_info};

/// `humata`'s exit codes (CLI.md §7).
pub const OK: i32 = 0;
pub const NEEDS_USER: i32 = 5;
pub const RESOURCES: i32 = 10;

pub const PASSPHRASE: &str = "copper lantern drifts over harbor";
/// Not [`PASSPHRASE`].
pub const WRONG: &str = "copper lantern drifts over meadow";

const KEY: [u8; 32] = [0x5a; 32];
const HOST_LABEL: &[u8] = b"humata-peer-host-v1";
const HOST_NONCE: [u8; 32] = [7; 32];

/// How long a message from the machine may take.
const MESSAGE_WAIT: Duration = Duration::from_secs(10);

// ----- The machine. -----

fn write_key(dir: &Path) {
    let path = common::state_folder(dir).join("enrollment-key");
    let hex: String = KEY.iter().map(|byte| format!("{byte:02x}")).collect();
    std::fs::write(&path, format!("{hex}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

/// A role-machine daemon of `build` in `dir`, enrolled with the test's key.
pub async fn machine_in(build: Build, dir: TempDir) -> Daemon {
    write_key(dir.path());
    Daemon::start_with(build, dir, &["--role", "machine"]).await
}

/// The hash the host keeps of [`PASSPHRASE`].
pub fn verifier() -> String {
    humata2_passphrase::Verifier::hash(PASSPHRASE)
        .unwrap()
        .encoded()
        .to_owned()
}

/// Waits until the machine's view of its host is `want`.
pub async fn host_reachable_becomes(daemon: &Daemon, want: bool) {
    let deadline = Instant::now() + MESSAGE_WAIT;
    while server_info(daemon).await.host_reachable != want {
        assert!(
            Instant::now() < deadline,
            "the machine's view of its host never changed"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

// ----- The host's end of the link. -----

fn host_proof(machine_nonce: &[u8]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(&KEY).unwrap();
    mac.update(HOST_LABEL);
    mac.update(machine_nonce);
    mac.update(&HOST_NONCE);
    mac.finalize().into_bytes().to_vec()
}

/// The host's end of the peer link: the `serve --stdio` process and the
/// Uplink it holds. Dropping it is the host going away.
pub struct Host {
    _link: tokio::process::Child,
    to_machine: mpsc::Sender<pb::HostToMachine>,
    from_machine: Streaming<pb::MachineToHost>,
}

impl Host {
    /// Links to `daemon` as the host does, and reads the machine's Resume.
    /// Nothing is sent yet: the host's own Resume comes next, from
    /// [`Self::resume`].
    pub async fn link(daemon: &Daemon) -> Self {
        let mut host = Self::connect(daemon).await;
        let resume = host.next().await;
        assert!(
            matches!(resume, pb::machine_to_host::Message::Resume(_)),
            "the machine resumed first: {resume:?}"
        );
        host
    }

    /// Links to `daemon` and sends a policy copy carrying `verifier` (empty:
    /// no passphrase set on the host), returning once the machine acked it.
    /// The machine's other messages are skipped.
    pub async fn with_policy(daemon: &Daemon, verifier: &str) -> Self {
        let mut host = Self::connect(daemon).await;
        host.send_policy(
            3,
            pb::PolicySnapshot {
                approval_verifier: verifier.to_owned(),
                ..Default::default()
            },
        )
        .await;
        host.journal_until_policy_ack(3).await;
        host
    }

    async fn connect(daemon: &Daemon) -> Self {
        let mut link = tokio::process::Command::new(binary("HUMATAD2_RELEASE"))
            .args(["serve", "--stdio", "--state"])
            .arg(&daemon.state)
            .arg("--socket")
            .arg(&daemon.socket)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let channel: Channel =
            common::channel_over(link.stdout.take().unwrap(), link.stdin.take().unwrap()).await;
        let mut peer = pb::peer_service_client::PeerServiceClient::new(channel);
        let answer = peer
            .hello(pb::HelloRequest {
                expected_machine: None,
                host_api_major: 1,
                host_api_minor: 0,
                nonce: HOST_NONCE.to_vec(),
                sync_marker: String::new(),
                chunk_codecs: Vec::new(),
            })
            .await
            .unwrap()
            .into_inner();
        peer.prove_host(pb::ProveHostRequest {
            proof: host_proof(&answer.machine_nonce),
        })
        .await
        .unwrap();
        let (to_machine, outgoing) = mpsc::channel(16);
        let from_machine = peer
            .uplink(ReceiverStream::new(outgoing))
            .await
            .unwrap()
            .into_inner();
        Self {
            _link: link,
            to_machine,
            from_machine,
        }
    }

    /// Sends one message to the machine.
    pub async fn send(&self, message: pb::host_to_machine::Message) {
        self.to_machine
            .send(pb::HostToMachine {
                message: Some(message),
            })
            .await
            .unwrap();
    }

    /// The host's Resume: it holds the machine's journal through
    /// `journal_through` (GRPC.md §9.7).
    pub async fn resume(&self, journal_through: u64) {
        self.send(pb::host_to_machine::Message::Resume(pb::UplinkResume {
            from: Some(pb::UplinkAck {
                journal_through,
                ..Default::default()
            }),
        }))
        .await;
    }

    /// Acks the machine's journal through `seq`.
    pub async fn ack_journal(&self, seq: u64) {
        self.send(pb::host_to_machine::Message::Ack(pb::UplinkAck {
            journal_through: seq,
            ..Default::default()
        }))
        .await;
    }

    /// Sends `snapshot` as revision `revision`.
    pub async fn send_policy(&self, revision: u64, mut snapshot: pb::PolicySnapshot) {
        snapshot.revision = revision;
        self.send(pb::host_to_machine::Message::Policy(snapshot)).await;
    }

    /// The machine's next message.
    pub async fn next(&mut self) -> pb::machine_to_host::Message {
        tokio::time::timeout(MESSAGE_WAIT, async {
            loop {
                let message = self
                    .from_machine
                    .message()
                    .await
                    .unwrap()
                    .expect("the uplink stayed open");
                if let Some(message) = message.message {
                    return message;
                }
            }
        })
        .await
        .expect("the machine sent nothing")
    }

    /// The journal entries the machine sends before it acks policy
    /// revision `revision`, skipping its other messages (status, pointers,
    /// acks of other kinds).
    pub async fn journal_until_policy_ack(&mut self, revision: u64) -> Vec<pb::JournalEntry> {
        let mut entries = Vec::new();
        loop {
            match self.next().await {
                pb::machine_to_host::Message::Journal(entry) => entries.push(entry),
                pb::machine_to_host::Message::Ack(ack) if ack.policy_revision == revision => {
                    return entries;
                }
                _ => {}
            }
        }
    }

    /// The next `count` journal entries, skipping the machine's other
    /// messages.
    pub async fn journal(&mut self, count: usize) -> Vec<pb::JournalEntry> {
        let mut entries = Vec::new();
        while entries.len() < count {
            if let pb::machine_to_host::Message::Journal(entry) = self.next().await {
                entries.push(entry);
            }
        }
        entries
    }
}

// ----- The person's terminal. -----

/// A terminal of the test's own: `/bin/sh` reading commands from the test,
/// as the leader of a new terminal session whose controlling terminal is a
/// pty. HOME is a folder of its own, and the only HUMATA variables are the
/// daemon's socket and the binary.
///
/// Like a real terminal, it reads everything written to the screen as it
/// comes. Unread screen output would block a writer once the pty's queue is
/// full, and on macOS the exiting shell waits for it to drain, so dropping
/// the terminal would hang. Neither end of the pty leaks into children.
pub struct Terminal {
    shell: Child,
    commands: ChildStdin,
    /// The keyboard: what the test writes here is typed at the terminal.
    keyboard: File,
    /// The terminal's own end, to drop what was typed but never read.
    screen: File,
    home: TempDir,
    runs: usize,
}

impl Terminal {
    pub fn open(daemon: &Daemon) -> Self {
        let pty: OwnedFd = rustix::pty::openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).unwrap();
        rustix::io::fcntl_setfd(&pty, rustix::io::FdFlags::CLOEXEC).unwrap();
        rustix::pty::grantpt(&pty).unwrap();
        rustix::pty::unlockpt(&pty).unwrap();
        let name = rustix::pty::ptsname(&pty, Vec::new()).unwrap();
        let screen = rustix::fs::open(
            name.as_c_str(),
            OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap();
        let screen = File::from(screen);
        let mut output = File::from(rustix::io::fcntl_dupfd_cloexec(&pty, 0).unwrap());
        // Ends when the last screen end closes (the read fails or is empty).
        std::thread::spawn(move || {
            let mut shown = [0_u8; 4096];
            while matches!(output.read(&mut shown), Ok(read) if read > 0) {}
        });
        let home = tempfile::tempdir().unwrap();
        let mut command = Command::new("/bin/sh");
        command
            .current_dir(home.path())
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", home.path())
            .env("HUMATA_SOCKET", &daemon.socket)
            .env("HUMATA", binary("HUMATA"))
            .stdin(Stdio::piped())
            .stdout(Stdio::from(screen.try_clone().unwrap()))
            .stderr(Stdio::from(screen.try_clone().unwrap()));
        // SAFETY: only async-signal-safe calls between fork and exec.
        unsafe {
            command.pre_exec(|| {
                rustix::process::setsid()?;
                rustix::process::ioctl_tiocsctty(BorrowedFd::borrow_raw(2))?;
                Ok(())
            });
        }
        let mut shell = command.spawn().unwrap();
        let commands = shell.stdin.take().unwrap();
        Self {
            shell,
            commands,
            keyboard: File::from(pty),
            screen,
            home,
            runs: 0,
        }
    }

    /// Runs `line` (a shell command running `"$HUMATA"`), typing `typed`
    /// at the terminal, and returns its exit code. Whatever was typed but
    /// not read is dropped afterwards, so it can't answer a later prompt.
    pub async fn exit_code(&mut self, line: &str, typed: Option<&str>) -> i32 {
        let run = self.runs;
        self.runs += 1;
        writeln!(
            self.commands,
            "{line}; echo $? > code{run}.partial && mv code{run}.partial code{run}"
        )
        .unwrap();
        self.commands.flush().unwrap();
        if let Some(typed) = typed {
            writeln!(self.keyboard, "{typed}").unwrap();
            self.keyboard.flush().unwrap();
        }
        let code = read_when_written(&self.home.path().join(format!("code{run}"))).await;
        rustix::termios::tcflush(&self.screen, QueueSelector::IFlush).unwrap();
        code.trim().parse().unwrap()
    }

    /// `humata login` as a person runs it, typing `passphrase` at its
    /// prompt.
    pub async fn log_in(&mut self, passphrase: &str) -> i32 {
        self.exit_code(r#""$HUMATA" login < /dev/tty"#, Some(passphrase))
            .await
    }

    /// Whether this terminal is the user: a user-only call succeeds.
    pub async fn is_user(&mut self) -> bool {
        self.exit_code(r#""$HUMATA" --json logins > /dev/null 2>&1"#, None)
            .await
            == OK
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.shell.kill();
        let _ = self.shell.wait();
    }
}

pub async fn read_when_written(path: &Path) -> String {
    let deadline = Instant::now() + WAIT;
    loop {
        match std::fs::read_to_string(path) {
            Ok(text) if !text.is_empty() => return text,
            _ => {}
        }
        assert!(Instant::now() < deadline, "{} was never written", path.display());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

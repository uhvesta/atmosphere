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
use std::time::Duration;

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
    machine_with(build, dir, &[]).await
}

/// [`machine_in`] with more `serve` arguments.
pub async fn machine_with(build: Build, dir: TempDir, extra: &[&str]) -> Daemon {
    write_key(dir.path());
    let mut args = vec!["--role", "machine"];
    args.extend_from_slice(extra);
    Daemon::start_with(build, dir, &args).await
}

/// The hash the host keeps of [`PASSPHRASE`].
pub fn verifier() -> String {
    humata2_passphrase::Verifier::hash(PASSPHRASE)
        .unwrap()
        .encoded()
        .to_owned()
}

/// Waits until the machine's view of its host is `want`. The machine's feed
/// tells of each change in its link (HostLinkChanged, to the user only), so
/// the feed is opened first, the view is read once, and the view is read
/// again each time the feed tells of a change to `want`. Nothing sleeps.
pub async fn host_reachable_becomes(daemon: &Daemon, want: bool) {
    let mut feed = pb::event_service_client::EventServiceClient::new(daemon.channel().await)
        .subscribe(pb::SubscribeRequest {
            topics: vec![pb::Topic::Machines as i32],
            ..Default::default()
        })
        .await
        .expect("the machine's event feed opens")
        .into_inner();
    if server_info(daemon).await.host_reachable == want {
        return;
    }
    let deadline = tokio::time::Instant::now() + MESSAGE_WAIT;
    loop {
        match tokio::time::timeout_at(deadline, feed.message()).await {
            Err(_) => panic!(
                "the machine's view of its host never became {want} within {MESSAGE_WAIT:?}"
            ),
            Ok(Err(status)) => panic!(
                "the machine's event feed failed ({status:?}) before its view of its host became {want}"
            ),
            Ok(Ok(None)) => panic!(
                "the machine's event feed ended before its view of its host became {want}"
            ),
            Ok(Ok(Some(event))) => {
                let link_changed = matches!(
                    &event.payload,
                    Some(pb::event::Payload::HostLink(link)) if link.reachable == want
                );
                if link_changed && server_info(daemon).await.host_reachable == want {
                    return;
                }
            }
        }
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
                host_build: None,
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
    /// What the screen showed and no wait has taken yet.
    shown: Vec<u8>,
    /// The screen's output as the reader thread passes it on, as it comes.
    screen_output: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
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
        let (screen_sender, screen_output) = tokio::sync::mpsc::unbounded_channel();
        // Passes on what the screen shows, as it comes, so a wait can read
        // it. Keeps reading after the test has stopped waiting, so the shell
        // never blocks. Ends when the last screen end closes.
        std::thread::spawn(move || {
            let mut bytes = [0_u8; 4096];
            loop {
                match output.read(&mut bytes) {
                    Ok(read) if read > 0 => {
                        let _ = screen_sender.send(bytes[..read].to_vec());
                    }
                    _ => return,
                }
            }
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
            shown: Vec::new(),
            screen_output,
            home,
            runs: 0,
        }
    }

    /// Runs `line` (a shell command running `"$HUMATA"`), typing `typed`
    /// at the terminal, and returns its exit code. Whatever was typed but
    /// not read is dropped afterwards, so it can't answer a later prompt.
    pub async fn exit_code(&mut self, line: &str, typed: Option<&str>) -> i32 {
        let run = self.start(line);
        if let Some(typed) = typed {
            self.type_raw(format!("{typed}\n").as_bytes());
        }
        self.finished(run).await
    }

    /// Starts `line` as [`Self::exit_code`] runs it but returns at once with
    /// its number, for a command that reads the keyboard while it runs
    /// (`humata attach`): [`Self::type_raw`] types at it, and
    /// [`Self::finished`] waits for its exit code. The shell prints a done
    /// marker to the screen after it has written the exit code.
    pub fn start(&mut self, line: &str) -> usize {
        let run = self.runs;
        self.runs += 1;
        writeln!(
            self.commands,
            "{line}; echo $? > code{run}.partial && mv code{run}.partial code{run}; echo '<<done{run}>>'"
        )
        .unwrap();
        self.commands.flush().unwrap();
        run
    }

    /// Types `bytes` at the terminal as they are, with no newline added.
    pub fn type_raw(&mut self, bytes: &[u8]) {
        self.keyboard.write_all(bytes).unwrap();
        self.keyboard.flush().unwrap();
    }

    /// The exit code of the command [`Self::start`] numbered `run`, read
    /// once its done marker has come to the screen. Whatever was typed but
    /// not read is dropped, as [`Self::exit_code`] does.
    pub async fn finished(&mut self, run: usize) -> i32 {
        self.wait_for_screen(&format!("<<done{run}>>")).await;
        let code = std::fs::read_to_string(self.home.path().join(format!("code{run}")))
            .unwrap_or_else(|error| panic!("the exit code of command {run} was not kept: {error}"));
        rustix::termios::tcflush(&self.screen, QueueSelector::IFlush).unwrap();
        code.trim().parse().unwrap()
    }

    /// Waits until `text` has come to the screen, within [`WAIT`]. What came
    /// before it is dropped, so a later wait only sees what follows.
    pub async fn wait_for_screen(&mut self, text: &str) {
        let needle = text.as_bytes();
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            let found = self
                .shown
                .windows(needle.len())
                .position(|window| window == needle);
            if let Some(at) = found {
                self.shown.drain(..at + needle.len());
                return;
            }
            match tokio::time::timeout_at(deadline, self.screen_output.recv()).await {
                Ok(Some(bytes)) => self.shown.extend_from_slice(&bytes),
                Ok(None) => panic!("the terminal's screen closed before {text:?} came"),
                Err(_) => panic!("{text:?} never came to the terminal's screen within {WAIT:?}"),
            }
        }
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

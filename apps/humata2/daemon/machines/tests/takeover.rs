//! Taking over a logged-in shell (MP-14), against a real `/bin/sh` on this
//! computer with a temporary `$HOME`. A pipe stands in for the terminal;
//! it has no line discipline, so the Ctrl-U a terminal would act on is
//! dropped before writing.

use std::io::{Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use humata2_machines::{
    Builds, Exit, InstallProblem, InstalledFile, MachineName, Payload, Platform, Takeover,
    TakeoverAction, TakeoverError,
};

/// Long enough for a slow machine; only reached when the shell never
/// answers.
const GUARD: Duration = Duration::from_secs(60);

struct AnyPlatform(Vec<Payload>);

impl Builds for AnyPlatform {
    fn payloads(&self, _platform: Platform) -> Option<Vec<Payload>> {
        Some(self.0.clone())
    }
}

/// Short, so the tests that reach it don't wait long, but long enough for
/// the shell to install and start the fake humatad2 (over 300 ms under
/// Bazel).
const SHORT: Duration = Duration::from_secs(3);

/// A humatad2 whose bridge says it is in sync, then echoes the stream.
fn fake_files() -> Vec<Payload> {
    files_with_bridge("printf 'HUMATA_SYNC_%s\\n' \"$n\"\nexec cat")
}

/// The files, with a humatad2 that runs `body` with the sync nonce in `$n`.
fn files_with_bridge(body: &str) -> Vec<Payload> {
    let humatad = format!(
        "#!/bin/sh\nwhile [ \"$#\" -gt 0 ] && [ \"$1\" != --sync ]; do shift; done\nn=$2\n{body}\n"
    );
    vec![
        Payload::new(InstalledFile::Humatad, Arc::from(humatad.as_bytes())),
        Payload::new(InstalledFile::Holder, Arc::from(&b"#!/bin/sh\nexit 0\n"[..])),
    ]
}

fn machine() -> MachineName {
    MachineName::new("box-1").unwrap()
}

struct Shell {
    child: Child,
    stdin: ChildStdin,
    output: Receiver<Vec<u8>>,
}

impl Shell {
    fn start(home: &std::path::Path) -> Self {
        Self::spawn(home, &[])
    }

    /// `/bin/sh` with `args`, such as `-c` and a script that stands in for
    /// what a login ends at.
    fn spawn(home: &std::path::Path, args: &[&str]) -> Self {
        let mut child = Command::new("/bin/sh")
            .args(args)
            .env_clear()
            .env("HOME", home)
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let (sender, output) = mpsc::channel();
        thread::spawn(move || {
            let mut buffer = [0; 4096];
            while let Ok(n) = stdout.read(&mut buffer) {
                if n == 0 || sender.send(buffer[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            stdin,
            output,
        }
    }

    fn write(&mut self, bytes: &[u8]) {
        self.stdin.write_all(bytes).unwrap();
        self.stdin.flush().unwrap();
    }

    fn read(&self) -> Vec<u8> {
        self.output.recv_timeout(GUARD).expect("the shell answered")
    }

    /// The next output, or none once `deadline` passes; `Err` once the
    /// shell's output closed, with how it exited.
    fn next(&mut self, deadline: Option<Instant>) -> Result<Option<Vec<u8>>, Exit> {
        let wait = deadline.map_or(GUARD, |deadline| {
            deadline.saturating_duration_since(Instant::now())
        });
        match self.output.recv_timeout(wait) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(RecvTimeoutError::Timeout) if deadline.is_some() => Ok(None),
            Err(RecvTimeoutError::Timeout) => panic!("the shell didn't answer"),
            Err(RecvTimeoutError::Disconnected) => {
                use std::os::unix::process::ExitStatusExt;
                let status = self.child.wait().unwrap();
                Err(match status.code() {
                    Some(code) => Exit::Code(code),
                    None => Exit::Signal(status.signal().unwrap_or_default()),
                })
            }
        }
    }
}

/// What the takeover ended with, and the files it sent.
enum End {
    Ready(Platform, Vec<u8>),
    Failed(TakeoverError),
}

/// Plays the daemon: does what the takeover asks until it ends. `mangle`
/// may change a file on its way, as a misbehaving terminal would.
fn run(
    shell: &mut Shell,
    builds: &dyn Builds,
    mangle: impl Fn(&[u8]) -> Vec<u8>,
) -> (End, Vec<InstalledFile>) {
    run_takeover(Takeover::new(machine()), shell, builds, mangle)
}

fn run_takeover(
    mut takeover: Takeover,
    shell: &mut Shell,
    builds: &dyn Builds,
    mangle: impl Fn(&[u8]) -> Vec<u8>,
) -> (End, Vec<InstalledFile>) {
    let mut actions = takeover.logged_in();
    let mut sent = Vec::new();
    loop {
        for action in std::mem::take(&mut actions) {
            match action {
                TakeoverAction::Write(bytes) => {
                    let typed: Vec<u8> = bytes.into_iter().filter(|&b| b != 0x15).collect();
                    shell.write(&typed);
                }
                TakeoverAction::MakeLocalRaw => {}
                TakeoverAction::SendFile(payload) => {
                    sent.push(payload.file());
                    shell.write(&mangle(payload.bytes()));
                }
                TakeoverAction::Ready { platform, leftover } => {
                    return (End::Ready(platform, leftover), sent);
                }
                TakeoverAction::Failed(error) => return (End::Failed(error), sent),
            }
        }
        actions = match shell.next(takeover.deadline()) {
            Ok(Some(chunk)) => takeover.output(&chunk, builds),
            Ok(None) => takeover.time_passed(Instant::now()),
            Err(exit) => takeover.ended(Some(exit)),
        };
    }
}

#[test]
fn the_files_are_installed_and_the_shell_becomes_the_bridge() {
    let home = tempfile::tempdir().unwrap();
    let mut shell = Shell::start(home.path());
    let files = fake_files();

    let (end, sent) = run(&mut shell, &AnyPlatform(files.clone()), <[u8]>::to_vec);

    let End::Ready(_, mut received) = end else {
        panic!("the takeover failed");
    };
    assert_eq!(sent, [InstalledFile::Humatad, InstalledFile::Holder]);
    for payload in &files {
        let installed = home.path().join(".humata2/bin").join(payload.file().file_name());
        assert_eq!(std::fs::read(installed).unwrap(), &payload.bytes()[..]);
    }
    // Whatever goes in now reaches the bridge, and its answer comes back.
    shell.write(b"hello bridge\n");
    while !received.ends_with(b"hello bridge\n") {
        received.extend(shell.read());
    }
    assert_eq!(received, b"hello bridge\n");
    drop(shell.stdin);
    assert!(shell.child.wait().unwrap().success());
}

#[test]
fn files_already_there_are_not_sent_again() {
    let home = tempfile::tempdir().unwrap();
    let files = fake_files();
    let bin = home.path().join(".humata2/bin");
    std::fs::create_dir_all(&bin).unwrap();
    for payload in &files {
        let path = bin.join(payload.file().file_name());
        std::fs::write(&path, &payload.bytes()[..]).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let mut shell = Shell::start(home.path());

    let (end, sent) = run(&mut shell, &AnyPlatform(files), <[u8]>::to_vec);

    assert!(matches!(end, End::Ready(..)));
    assert!(sent.is_empty());
}

#[test]
fn a_file_changed_on_the_way_is_not_installed() {
    let home = tempfile::tempdir().unwrap();
    let mut shell = Shell::start(home.path());

    let (end, _) = run(&mut shell, &AnyPlatform(fake_files()), |bytes| {
        let mut changed = bytes.to_vec();
        changed[0] ^= 0x20;
        changed
    });

    let End::Failed(error) = end else {
        panic!("the takeover went ahead");
    };
    assert_eq!(
        error,
        TakeoverError::Install {
            file: InstalledFile::Humatad,
            problem: InstallProblem::ChecksumMismatch,
        }
    );
    assert!(!home.path().join(".humata2/bin/humatad2").exists());
}

#[test]
fn a_bridge_that_never_syncs_times_out_with_the_end_of_its_output() {
    let home = tempfile::tempdir().unwrap();
    let mut shell = Shell::start(home.path());
    let files = files_with_bridge("echo 'waiting for the socket'\nexec cat");
    let takeover = Takeover::new(machine()).with_sync_timeout(SHORT);

    let (end, _) = run_takeover(takeover, &mut shell, &AnyPlatform(files), <[u8]>::to_vec);

    let End::Failed(TakeoverError::SyncTimedOut {
        machine: timed_out,
        waited,
        output_tail,
    }) = end
    else {
        panic!("the takeover didn't time out");
    };
    assert_eq!(timed_out, machine());
    assert_eq!(waited, SHORT);
    assert!(output_tail.contains("waiting for the socket"), "{output_tail}");
}

#[test]
fn a_bridge_that_exits_fails_the_takeover_before_the_deadline() {
    let home = tempfile::tempdir().unwrap();
    let mut shell = Shell::start(home.path());
    let files = files_with_bridge("echo 'humatad2: cannot reach the daemon'\nexit 3");
    let takeover = Takeover::new(machine()).with_sync_timeout(GUARD);
    let started = Instant::now();

    let (end, _) = run_takeover(takeover, &mut shell, &AnyPlatform(files), <[u8]>::to_vec);

    let End::Failed(TakeoverError::TerminalEnded {
        exit, output_tail, ..
    }) = end
    else {
        panic!("the takeover didn't fail when the shell ended");
    };
    assert!(started.elapsed() < GUARD / 2, "{:?}", started.elapsed());
    assert_eq!(exit, Some(Exit::Code(3)));
    assert!(
        output_tail.contains("humatad2: cannot reach the daemon"),
        "{output_tail}"
    );
}

#[test]
fn only_the_whole_marker_line_for_this_nonce_counts() {
    let home = tempfile::tempdir().unwrap();
    let mut shell = Shell::start(home.path());
    // The echoed bridge line, another takeover's marker, this one's with
    // more after it, and with something before it.
    let files = files_with_bridge(
        "echo \"exec humatad2 bridge --sync $n\"\n\
         printf 'HUMATA_SYNC_0123456789abcdef\\n'\n\
         printf 'HUMATA_SYNC_%sx\\n' \"$n\"\n\
         printf 'HUMATA_SYNC_%s ready\\n' \"$n\"\n\
         printf 'say HUMATA_SYNC_%s\\n' \"$n\"\n\
         exec cat",
    );
    let takeover = Takeover::new(machine()).with_sync_timeout(SHORT);

    let (end, _) = run_takeover(takeover, &mut shell, &AnyPlatform(files), <[u8]>::to_vec);

    assert!(
        matches!(end, End::Failed(TakeoverError::SyncTimedOut { .. })),
        "the takeover took a marker that wasn't its own"
    );
}

#[test]
fn a_login_that_ends_in_a_menu_is_not_a_shell() {
    let home = tempfile::tempdir().unwrap();
    let menu = "echo '1) dev  2) prod'; while read -r line; do echo 'Pick 1-2:'; done";
    let mut shell = Shell::spawn(home.path(), &["-c", menu]);
    let takeover = Takeover::new(machine()).with_shell_timeout(SHORT);

    let (end, sent) = run_takeover(takeover, &mut shell, &AnyPlatform(fake_files()), <[u8]>::to_vec);

    let End::Failed(TakeoverError::NoShell {
        waited, output_tail, ..
    }) = end
    else {
        panic!("the takeover didn't give up on the menu");
    };
    assert_eq!(waited, SHORT);
    assert!(output_tail.contains("Pick 1-2:"), "{output_tail}");
    assert!(sent.is_empty());
}

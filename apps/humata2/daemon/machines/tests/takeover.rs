//! Taking over a logged-in shell (MP-14), against a real `/bin/sh` on this
//! computer with a temporary `$HOME`. A pipe stands in for the terminal;
//! it has no line discipline, so the Ctrl-U a terminal would act on is
//! dropped before writing.

use std::io::{Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use humata2_machines::{
    Builds, InstallProblem, InstalledFile, Payload, Platform, Takeover, TakeoverAction,
    TakeoverError,
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

/// A humatad2 whose bridge says it is in sync, then echoes the stream.
fn fake_files() -> Vec<Payload> {
    vec![
        Payload::new(
            InstalledFile::Humatad,
            Arc::from(&b"#!/bin/sh\nprintf 'HUMATA_SYNC_%s\\n' \"$3\"\nexec cat\n"[..]),
        ),
        Payload::new(InstalledFile::Holder, Arc::from(&b"#!/bin/sh\nexit 0\n"[..])),
    ]
}

struct Shell {
    child: Child,
    stdin: ChildStdin,
    output: Receiver<Vec<u8>>,
}

impl Shell {
    fn start(home: &std::path::Path) -> Self {
        let mut child = Command::new("/bin/sh")
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
    let mut takeover = Takeover::new();
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
        let chunk = shell.read();
        actions = takeover.output(&chunk, builds);
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

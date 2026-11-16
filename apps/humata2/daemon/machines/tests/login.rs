//! Interactive login (MP-13 to MP-16) against real pseudo-terminals and a
//! fake login command, a shell script in a temporary folder that asks for a
//! verification code and then becomes a shell. The takeover installs fake
//! humatad2 files into a temporary `$HOME`; nothing touches the real home,
//! `~/.ssh` or any network.
//!
//! Enabling it: move this file up one folder to `tests/login.rs` once
//! `login.rs` is wired into the crate (see the enable lines in the report).

use std::path::Path;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use humata2_machines::{
    Builds, BuildsError, Exit, InstalledFile, Invocation, LoginError, LoginEvent, LoginSession, LoginTerminal,
    MachineName, Observed, Payload, Plan, Platform, Purpose, TakeoverError, detect_prompt,
};

/// Long enough for a loaded computer; only reached when a test fails.
const GUARD: Duration = Duration::from_secs(60);

struct AnyPlatform(Vec<Payload>);

impl Builds for AnyPlatform {
    fn payloads(&self, _platform: Platform) -> Result<Vec<Payload>, BuildsError> {
        Ok(self.0.clone())
    }
}

/// A humatad2 whose `serve --stdio --sync <nonce>` says it is in sync,
/// then echoes the stream.
fn fake_files() -> Vec<Payload> {
    let humatad = "#!/bin/sh\nwhile [ \"$#\" -gt 0 ] && [ \"$1\" != --sync ]; do shift; done\nn=$2\nprintf 'HUMATA_SYNC_%s\\n' \"$n\"\nexec cat\n";
    vec![
        Payload::new(InstalledFile::Humatad, Arc::from(humatad.as_bytes())),
        Payload::new(InstalledFile::Holder, Arc::from(&b"#!/bin/sh\nexit 0\n"[..])),
        Payload::new(InstalledFile::Cli, Arc::from(&b"#!/bin/sh\nexit 0\n"[..])),
    ]
}

fn machine() -> MachineName {
    MachineName::new("box-1").unwrap()
}

/// `/bin/sh -c body` with a home of the test's own and a `pgrep` that finds
/// no holder (the computer's own would make the takeover stage each file).
fn login(home: &Path, body: &str) -> Invocation {
    use std::os::unix::fs::PermissionsExt;
    let fakes = home.join(".test-bin");
    std::fs::create_dir_all(&fakes).unwrap();
    let pgrep = fakes.join("pgrep");
    std::fs::write(&pgrep, "#!/bin/sh\nexit 1\n").unwrap();
    std::fs::set_permissions(&pgrep, std::fs::Permissions::from_mode(0o755)).unwrap();
    use humata2_machines::EnvValue::Value;
    Invocation {
        program: "/bin/sh".to_owned(),
        args: vec!["-c".to_owned(), body.to_owned()],
        env: vec![
            ("HOME".to_owned(), Value(home.display().to_string())),
            (
                "PATH".to_owned(),
                Value(format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", fakes.display())),
            ),
        ],
        stdin: None,
        timeout: None,
        purpose: Purpose::Login,
    }
}

/// Asks for a code, becomes a shell on the right one, ends with 3 otherwise.
const CODE_THEN_SHELL: &str = "printf 'Verification code: '; read code; \
    if [ \"$code\" = 123456 ]; then echo 'Welcome to box-1'; exec /bin/sh; else echo 'denied'; exit 3; fi";

/// Reads `events` until the output so far holds `needle`.
fn until(events: &Receiver<LoginEvent>, seen: &mut Vec<u8>, needle: &str) {
    let deadline = Instant::now() + GUARD;
    while !String::from_utf8_lossy(seen).contains(needle) {
        match events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(LoginEvent::Output(bytes)) => seen.extend(bytes),
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => panic!("never saw {needle:?}; saw {:?}", String::from_utf8_lossy(seen)),
            Err(RecvTimeoutError::Disconnected) => panic!("the terminal ended before {needle:?}"),
        }
    }
}

#[test]
fn the_terminal_takes_input_resizes_and_prints() {
    let home = tempfile::tempdir().unwrap();
    let (sender, events) = mpsc::channel();
    let body = "printf 'Verification code: '; read code; echo \"got:$code\"; read go; stty size; sleep 30";
    let mut terminal =
        LoginTerminal::spawn(&login(home.path(), body), &(), 80, 24, sender).expect("the terminal starts");
    let mut seen = Vec::new();

    until(&events, &mut seen, "Verification code:");
    terminal.write(b"123456\n").unwrap();
    until(&events, &mut seen, "got:123456");
    terminal.resize(100, 30).unwrap();
    terminal.write(b"\n").unwrap();
    until(&events, &mut seen, "30 100");
}

#[test]
fn dropping_the_terminal_stops_the_command_and_what_it_started() {
    let home = tempfile::tempdir().unwrap();
    let pid_file = home.path().join("pid");
    let (sender, events) = mpsc::channel();
    let body = format!("echo $$ > {}; echo started; exec sleep 300", pid_file.display());
    let terminal = LoginTerminal::spawn(&login(home.path(), &body), &(), 80, 24, sender).unwrap();
    let mut seen = Vec::new();
    until(&events, &mut seen, "started");
    let pid = std::fs::read_to_string(&pid_file).unwrap().trim().to_owned();
    let alive = || {
        std::process::Command::new("kill")
            .args(["-0", &pid])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    };
    assert!(alive());

    drop(terminal);

    let deadline = Instant::now() + GUARD;
    while alive() {
        assert!(Instant::now() < deadline, "the command kept running");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Runs a session on a thread; what the person would see arrives on
/// `shown`.
struct Running {
    handle: humata2_machines::LoginHandle,
    shown: Receiver<Observed>,
    result: std::thread::JoinHandle<Result<humata2_machines::LoginLink, LoginError>>,
}

fn start(home: &Path, body: &str, keys: Option<&Path>, files: Vec<Payload>) -> Running {
    let invocation = login(home, body);
    let (session, handle) = LoginSession::open(machine(), &invocation, &(), 80, 24).unwrap();
    let (sender, shown) = mpsc::channel();
    let keys = keys.map(Path::to_owned);
    let result = std::thread::spawn(move || {
        let builds = AnyPlatform(files);
        let plan = Plan {
            builds: &builds,
            enroll_into: keys.as_deref(),
            shell_timeout: None,
            sync_timeout: Some(Duration::from_secs(10)),
        };
        session.run(&plan, &mut |observed| sender.send(observed).is_ok())
    });
    Running { handle, shown, result }
}

impl Running {
    /// Reads what is shown until `needle` appears; returns whether a
    /// prompt was seen on the way.
    fn wait_for(&self, needle: &str) -> bool {
        let mut text = Vec::new();
        let mut prompted = false;
        let deadline = Instant::now() + GUARD;
        while !String::from_utf8_lossy(&text).contains(needle) {
            match self.shown.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(Observed::Output(bytes)) => text.extend(bytes),
                Ok(Observed::Prompt(_)) => prompted = true,
                Err(_) => panic!("never saw {needle:?}; saw {:?}", String::from_utf8_lossy(&text)),
            }
        }
        prompted
    }
}

#[test]
fn a_login_is_taken_over_and_the_terminal_becomes_the_peer_link() {
    let home = tempfile::tempdir().unwrap();
    let keys = tempfile::tempdir().unwrap();
    let running = start(home.path(), CODE_THEN_SHELL, Some(keys.path()), fake_files());

    assert!(running.wait_for("Verification code:"), "the prompt was reported");
    assert!(running.handle.input(b"123456\n".to_vec()));
    running.wait_for("Welcome to box-1");
    assert!(running.handle.done());
    let link = running.result.join().unwrap().expect("the takeover succeeded");

    assert!(link.enrolled());
    // The machine's key and the host's copy are the same private hex.
    let on_machine = std::fs::read(home.path().join(".humata2/state/enrollment-key")).unwrap();
    assert_eq!(std::fs::read(keys.path().join("box-1")).unwrap(), on_machine);
    assert_eq!(on_machine.len(), 64);
    for file in InstalledFile::ALL {
        assert!(home.path().join(".humata2/bin").join(file.file_name()).exists(), "{file:?}");
    }
    // What the host writes now reaches the bridge, and its answer returns.
    let (mut reader, mut writer) = link.split();
    use std::io::{Read, Write};
    writer.write_all(b"hello bridge\n").unwrap();
    writer.flush().unwrap();
    let mut got = Vec::new();
    let mut chunk = [0_u8; 256];
    while !got.ends_with(b"hello bridge\n") {
        let n = reader.read(&mut chunk).unwrap();
        assert!(n > 0, "the link closed early: {:?}", String::from_utf8_lossy(&got));
        got.extend_from_slice(&chunk[..n]);
    }
    assert_eq!(got, b"hello bridge\n");
}

#[test]
fn a_wrong_code_ends_the_login_with_the_end_of_the_terminal() {
    let home = tempfile::tempdir().unwrap();
    let running = start(home.path(), CODE_THEN_SHELL, None, fake_files());

    running.wait_for("Verification code:");
    assert!(running.handle.input(b"000000\n".to_vec()));
    let error = running.result.join().unwrap().err().expect("the login failed");

    let LoginError::Ended { exit, output_tail, .. } = error else {
        panic!("expected the terminal to have ended, got {error}");
    };
    assert_eq!(exit, Some(Exit::Code(3)));
    assert!(output_tail.contains("denied"), "{output_tail}");
    assert!(!home.path().join(".humata2").exists(), "nothing was installed");
}

#[test]
fn a_login_that_ends_at_no_shell_fails_the_takeover_and_installs_nothing() {
    let home = tempfile::tempdir().unwrap();
    // A menu-only tool: after the code it prints a menu and reads forever.
    let body = "printf 'Verification code: '; read code; echo 'Pick: 1) a 2) b'; sleep 300";
    let invocation = login(home.path(), body);
    let (session, handle) = LoginSession::open(machine(), &invocation, &(), 80, 24).unwrap();
    let thread = std::thread::spawn(move || {
        let builds = AnyPlatform(fake_files());
        let plan = Plan {
            builds: &builds,
            enroll_into: None,
            shell_timeout: Some(Duration::from_millis(500)),
            sync_timeout: None,
        };
        session.run(&plan, &mut |_| true)
    });
    handle.input(b"123456\n".to_vec());
    handle.done();

    let error = thread.join().unwrap().err().expect("no shell, no takeover");

    assert!(matches!(error, LoginError::Takeover(TakeoverError::NoShell { .. })), "{error}");
}

#[test]
fn cancelling_stops_the_terminal_and_leaves_nothing() {
    let home = tempfile::tempdir().unwrap();
    let running = start(home.path(), CODE_THEN_SHELL, None, fake_files());
    running.wait_for("Verification code:");

    assert!(running.handle.cancel());
    let error = running.result.join().unwrap().err().expect("cancelled");

    assert!(matches!(error, LoginError::Cancelled { .. }), "{error}");
    assert!(!home.path().join(".humata2").exists());
}

#[test]
fn prompts_are_found_in_what_a_command_printed() {
    assert!(detect_prompt(b"Verification code: ").is_some());
    assert!(detect_prompt(b"Welcome\n$ ").is_none());
}

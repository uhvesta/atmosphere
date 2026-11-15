//! `humata login` against a real daemon (SEC-11): the real `humata` runs
//! in a terminal of its own (a pty, its shell the session leader), the test
//! plays the app that confirms it, and every other caller is judged as the
//! release daemon judges it. A login holds for its terminal session only:
//! its token is refused from anywhere else, after logout, and once it
//! expires, and a session's tab can't log in at all.

mod common;

use std::fs::File;
use std::io::Write;
use std::os::fd::{BorrowedFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

use common::{Attach, Build, Daemon, WAIT, binary, folder, open, state_folder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{SessionRecord, Store};
use pb::cli_auth_service_client::CliAuthServiceClient;
use rustix::fs::{Mode, OFlags};
use rustix::pty::OpenptFlags;
use serde_json::Value as Json;
use tempfile::TempDir;
use tonic::metadata::MetadataValue;
use tonic::{Code, Request};

/// `humata`'s exit codes (CLI.md §7).
const OK: i32 = 0;
const DENIED: i32 = 4;
const NEEDS_USER: i32 = 5;

const LOGIN_HEADER: &str = "humata-cli-login";

/// One run of `humata` in the terminal: its exit code and JSON output.
struct Run {
    code: i32,
    json: Json,
}

/// A terminal of the test's own: `/bin/sh` reading commands from the test,
/// as the leader of a new terminal session whose controlling terminal is a
/// pty. HOME is a folder of its own, and the only HUMATA variables are the
/// daemon's socket and the binaries.
struct Terminal {
    shell: Child,
    commands: ChildStdin,
    /// Kept open: closing it hangs the terminal up.
    _pty: OwnedFd,
    home: TempDir,
    runs: usize,
}

impl Terminal {
    fn open(daemon: &Daemon) -> Self {
        let pty = rustix::pty::openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).unwrap();
        rustix::pty::grantpt(&pty).unwrap();
        rustix::pty::unlockpt(&pty).unwrap();
        let name = rustix::pty::ptsname(&pty, Vec::new()).unwrap();
        let screen = rustix::fs::open(
            name.as_c_str(),
            OFlags::RDWR | OFlags::NOCTTY,
            Mode::empty(),
        )
        .unwrap();
        let screen = File::from(screen);
        let home = tempfile::tempdir().unwrap();
        let mut command = Command::new("/bin/sh");
        command
            .current_dir(home.path())
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", home.path())
            .env("HUMATA_SOCKET", &daemon.socket)
            .env("HUMATA", binary("HUMATA"))
            .env("SETSID", binary("HUMATA2_SETSID"))
            .stdin(Stdio::piped())
            .stdout(Stdio::from(screen.try_clone().unwrap()))
            .stderr(Stdio::from(screen));
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
            _pty: pty,
            home,
            runs: 0,
        }
    }

    /// Where `humata login` keeps this terminal's login.
    fn saved_login(&self) -> PathBuf {
        self.home
            .path()
            .join(".humata2/cli-logins")
            .join(self.shell.id().to_string())
    }

    /// Starts `line` (a shell command running `"$HUMATA"`), its output
    /// going to files the test reads in [`Self::finish`].
    fn start(&mut self, line: &str) -> usize {
        let run = self.runs;
        self.runs += 1;
        writeln!(
            self.commands,
            "{line} > out{run} 2> err{run}; echo $? > code{run}.partial && mv code{run}.partial code{run}"
        )
        .unwrap();
        self.commands.flush().unwrap();
        run
    }

    async fn finish(&self, run: usize) -> Run {
        let code = read_when_written(&self.home.path().join(format!("code{run}"))).await;
        let out = std::fs::read_to_string(self.home.path().join(format!("out{run}"))).unwrap();
        let err = std::fs::read_to_string(self.home.path().join(format!("err{run}"))).unwrap();
        Run {
            code: code.trim().parse().unwrap(),
            json: serde_json::from_str(&out)
                .unwrap_or_else(|_| panic!("humata printed no JSON: {out}{err}")),
        }
    }

    async fn run(&mut self, line: &str) -> Run {
        let run = self.start(line);
        self.finish(run).await
    }

    /// `humata login`, confirmed (or not) in the app.
    async fn log_in(&mut self, daemon: &Daemon, allow: bool) -> Run {
        let run = self.start(r#""$HUMATA" --json login"#);
        answer_login(daemon, allow).await;
        self.finish(run).await
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.shell.kill();
        let _ = self.shell.wait();
    }
}

async fn read_when_written(path: &Path) -> String {
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

/// Answers the one login waiting for the app, as the app.
async fn answer_login(daemon: &Daemon, allow: bool) {
    let mut app = CliAuthServiceClient::new(daemon.channel().await);
    let deadline = Instant::now() + WAIT;
    let login = loop {
        let pending = app
            .list_pending_cli_logins(pb::ListPendingCliLoginsRequest {})
            .await
            .unwrap()
            .into_inner()
            .logins;
        if let Some(login) = pending.into_iter().next() {
            break login;
        }
        assert!(Instant::now() < deadline, "no login ever waited for the app");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert!(!login.terminal.is_empty());
    app.answer_cli_login(pb::AnswerCliLoginRequest {
        id: login.id,
        allow,
    })
    .await
    .unwrap();
}

async fn no_login_waits(daemon: &Daemon) -> bool {
    CliAuthServiceClient::new(daemon.channel().await)
        .list_pending_cli_logins(pb::ListPendingCliLoginsRequest {})
        .await
        .unwrap()
        .into_inner()
        .logins
        .is_empty()
}

/// Whether WhoAmI named the user, and could ask for presence.
fn is_user(whoami: &Run) -> bool {
    whoami.code == OK
        && !whoami.json["result"]["actor"]["user"].is_null()
        && whoami.json["result"]["presence_possible"] == Json::Bool(true)
}

const WHOAMI: &str = r#""$HUMATA" --json whoami"#;
/// A user-only call: refused (NEEDS_USER) without a usable login.
const LOGINS: &str = r#""$HUMATA" --json logins"#;

#[tokio::test(flavor = "multi_thread")]
async fn a_confirmed_login_makes_its_terminal_the_user_until_logout() {
    let daemon = Daemon::start_build(Build::AdminForThisProcess, folder()).await;
    let mut terminal = Terminal::open(&daemon);
    assert!(!is_user(&terminal.run(WHOAMI).await));
    assert_eq!(terminal.run(LOGINS).await.code, NEEDS_USER);

    let login = terminal.log_in(&daemon, true).await;
    assert_eq!(login.code, OK, "{}", login.json);
    let token = std::fs::read_to_string(terminal.saved_login()).unwrap();
    assert!(!login.json.to_string().contains(token.trim()));
    assert!(is_user(&terminal.run(WHOAMI).await));
    let listed = terminal.run(LOGINS).await;
    assert_eq!(listed.code, OK, "{}", listed.json);

    assert_eq!(terminal.run(r#""$HUMATA" --json logout"#).await.code, OK);
    assert!(!terminal.saved_login().exists());
    assert!(!is_user(&terminal.run(WHOAMI).await));
    assert_eq!(terminal.run(LOGINS).await.code, NEEDS_USER);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_login_denied_in_the_app_leaves_the_terminal_as_it_was() {
    let daemon = Daemon::start_build(Build::AdminForThisProcess, folder()).await;
    let mut terminal = Terminal::open(&daemon);
    assert_eq!(terminal.log_in(&daemon, false).await.code, DENIED);
    assert!(!terminal.saved_login().exists());
    assert_eq!(terminal.run(LOGINS).await.code, NEEDS_USER);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_logins_token_is_refused_outside_its_terminal_session() {
    let daemon = Daemon::start_build(Build::AdminForThisProcess, folder()).await;
    let mut terminal = Terminal::open(&daemon);
    assert_eq!(terminal.log_in(&daemon, true).await.code, OK);
    let token = std::fs::read_to_string(terminal.saved_login()).unwrap();

    // Sent from the test's own process, even though the daemon takes it
    // for the app when it sends nothing.
    let mut request = Request::new(pb::ListTabsRequest::default());
    request
        .metadata_mut()
        .insert(LOGIN_HEADER, MetadataValue::try_from(token.trim()).unwrap());
    let refused = daemon.terminals().await.list_tabs(request).await.unwrap_err();
    assert_eq!(refused.code(), Code::Unauthenticated, "{refused:?}");

    // Copied to where a process that left the terminal session would look.
    let session = terminal.shell.id();
    let escaped = terminal
        .run(&format!(
            r#""$SETSID" /bin/sh -c 'cp "$HOME/.humata2/cli-logins/{session}" "$HOME/.humata2/cli-logins/$$" && exec "$HUMATA" --json logins'"#
        ))
        .await;
    assert_eq!(escaped.code, NEEDS_USER, "{}", escaped.json);

    // The terminal itself is still logged in.
    assert_eq!(terminal.run(LOGINS).await.code, OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_login_ends_when_it_expires() {
    let daemon = Daemon::start_with(
        Build::AdminForThisProcess,
        folder(),
        &["--cli-login-seconds=1"],
    )
    .await;
    let mut terminal = Terminal::open(&daemon);
    assert_eq!(terminal.log_in(&daemon, true).await.code, OK);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(terminal.run(LOGINS).await.code, NEEDS_USER);
    // Logging out of an expired login just forgets it.
    assert_eq!(terminal.run(r#""$HUMATA" --json logout"#).await.code, OK);
    assert!(!terminal.saved_login().exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_saved_login_is_refused_in_another_terminal() {
    let daemon = Daemon::start_build(Build::AdminForThisProcess, folder()).await;
    let mut first = Terminal::open(&daemon);
    assert_eq!(first.log_in(&daemon, true).await.code, OK);
    let token = std::fs::read_to_string(first.saved_login()).unwrap();
    drop(first);

    // A new terminal that finds the old one's login under its own session
    // id, as it would if the old session's id were reused.
    let mut second = Terminal::open(&daemon);
    std::fs::create_dir_all(second.saved_login().parent().unwrap()).unwrap();
    std::fs::write(second.saved_login(), token).unwrap();
    assert_eq!(second.run(LOGINS).await.code, NEEDS_USER);
}

/// A session's tab can't log in, with its agent's token or without it:
/// anything typed there may come from the agent.
#[tokio::test(flavor = "multi_thread")]
async fn a_sessions_tab_cant_log_in() {
    const SESSION: &str = "se_login";
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    store
        .put_session(&SessionRecord {
            id: SESSION.to_owned(),
            channel: "ch_login".to_owned(),
            machine: String::new(),
            title: "login".to_owned(),
            agent: "sh".to_owned(),
            folder: dir.path().to_owned(),
            created_unix_ms: 0,
        })
        .unwrap();
    drop(store);
    let daemon = Daemon::start_build(Build::AdminForThisProcess, dir).await;
    let tab = daemon
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: None,
            session: Some(pb::SessionId {
                value: SESSION.to_owned(),
            }),
            repo: None,
        })
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap();
    let home = tempfile::tempdir().unwrap();
    let humata = binary("HUMATA");
    let mut attach = Attach::start(&daemon, open(&tab, 0)).await;
    for (run, unset) in [(0, ""), (1, "-u HUMATA_SESSION_TOKEN")] {
        attach
            .type_text(&format!(
                "env {unset} HOME={home} HUMATA_SOCKET={socket} {humata} --json login > {home}/out{run}; echo $? > {home}/code{run}\r",
                home = home.path().display(),
                socket = daemon.socket.display(),
                humata = humata.display(),
            ))
            .await;
        let code = read_when_written(&home.path().join(format!("code{run}"))).await;
        let out = std::fs::read_to_string(home.path().join(format!("out{run}"))).unwrap();
        let json: Json = serde_json::from_str(&out).unwrap();
        assert_eq!(code.trim(), NEEDS_USER.to_string(), "{json}");
        assert_eq!(
            json["error"]["details"]["caller_in_session_terminal"],
            Json::Bool(true),
            "{json}"
        );
        assert!(no_login_waits(&daemon).await);
    }
}

//! End-to-end `humata login` acceptance in the desktop app against a real
//! isolated `humatad2_admin` (using `--admin-pid` so only the app process acts
//! as the app, while the external PTY terminal is unauthenticated).
//!
//! Verifies that running `humata login` from an external terminal immediately
//! surfaces the approval banner in the rendered GPUI `Shell` (using
//! `FrameMode::Natural`, which only repaints dirty panes), clicking Approve
//! unblocks `humata login`, clears the banner, and authenticates subsequent
//! CLI commands in that terminal session.
#![cfg(target_os = "macos")]

use std::fs::File;
use std::io::Write as _;
use std::os::fd::{BorrowedFd, OwnedFd};
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::{
    AnyWindowHandle, Bounds, HeadlessAppContext, Role, WindowBounds, WindowOptions, point, px, size,
};
use humata2_app::{Load, Route};
use humata2_clients::Clients;
use humata2_clients_live::{LiveComments, LiveDaemon, LiveOptions};
use humata2_engine::Resources;
use humata2_flow::{Action, Driver as _, FrameMode, GpuiDriver, Target};
use humata2_ui::bridge::Ui;
use humata2_ui::record::Recorder;
use rustix::fs::{Mode, OFlags};
use rustix::pty::OpenptFlags;
use serde_json::Value as Json;
use tca::{MemoryStorage, SystemClock};
use tempfile::TempDir;
use tokio::runtime::Runtime;

const OK: i32 = 0;
const DENIED: i32 = 4;
const NEEDS_USER: i32 = 5;
const DEADLINE: Duration = Duration::from_secs(15);

fn binary(variable: &'static str) -> PathBuf {
    let path = std::env::var_os(variable)
        .unwrap_or_else(|| panic!("set {variable} to the binary's path"));
    std::fs::canonicalize(path).unwrap_or_else(|e| panic!("canonicalize {variable}: {e}"))
}

fn runtime() -> Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("tokio runtime")
}

struct TestDaemon {
    dir: TempDir,
    socket: PathBuf,
    child: Child,
}

impl TestDaemon {
    async fn start() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("h2cl_")
            .tempdir_in("/tmp")
            .expect("tempdir");
        let root = dir.path();
        std::fs::set_permissions(root, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .expect("chmod 700 tempdir");
        let socket = root.join("humatad.sock");
        let state = root.join("state");
        let channels = root.join("ch");
        let pid = std::process::id().to_string();
        let child = Command::new(binary("HUMATAD2"))
            .args(["serve", "--admin-pid", &pid, "--socket"])
            .arg(&socket)
            .arg("--root")
            .arg(root)
            .arg("--state")
            .arg(&state)
            .arg("--channels")
            .arg(&channels)
            .arg("--holder")
            .arg(binary("HUMATA2_HOLDER"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
            .expect("spawn humatad2_admin");
        let deadline = Instant::now() + Duration::from_secs(20);
        while tokio::net::UnixStream::connect(&socket).await.is_err() {
            assert!(
                Instant::now() < deadline,
                "humatad2_admin never listened on {}",
                socket.display()
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        Self { dir, socket, child }
    }

    async fn clients(&self) -> Clients {
        let daemon = LiveDaemon::connect(LiveOptions::new(self.socket.clone()))
            .await
            .expect("connect LiveDaemon");
        Clients {
            daemon: Arc::new(daemon.clone()),
            desk: Arc::new(daemon.clone()),
            comments: Arc::new(LiveComments::new(&daemon)),
            history: Arc::new(humata2_clients_live::history::LiveHistory::new(&daemon)),
            marks: Arc::new(humata2_clients_live::marks::LiveMarks::new(&daemon)),
            views: Arc::new(humata2_clients_live::LiveViews::new(&daemon)),
            pull_requests: Arc::new(humata2_clients_live::LivePullRequests::new(&daemon)),
            review_chat: Arc::new(humata2_clients_live::LiveReviewChat::new(&daemon)),
            todo: Arc::new(humata2_clients_live::todo::LiveTodo::new(&daemon)),
            terminals: Arc::new(daemon),
            clock: Arc::new(SystemClock::new()),
            clipboard: Arc::new(humata2_clients::FakeClipboard::default()),
            reveal: Arc::new(humata2_clients::FakeReveal::default()),
            files: Arc::new(humata2_clients::FakeFileSystem::default()),
            file_manager: Arc::new(humata2_clients::FakeFileManager::default()),
            keychain: Arc::new(humata2_clients::FakeKeychain::default()),
            local_daemon: Arc::new(humata2_clients::FakeLocalDaemon::default()),
            machines: Arc::new(humata2_clients::machines::FakeMachines::new()),
            notifier: Arc::new(humata2_clients::NoopNotifier),
            storage: Arc::new(MemoryStorage::new()),
        }
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let holders = self.dir.path().join("state").join("holders");
        if std::fs::read_dir(&holders).is_ok_and(|mut entries| entries.next().is_some()) {
            let _ = Command::new("pkill")
                .args(["-9", "-f"])
                .arg(self.dir.path())
                .status();
        }
    }
}

struct ExternalTerminal {
    shell: Child,
    commands: ChildStdin,
    _pty: OwnedFd,
    home: TempDir,
    runs: usize,
}

struct CliRun {
    code: i32,
    json: Json,
}

impl ExternalTerminal {
    fn open(socket: &Path) -> Self {
        let pty = rustix::pty::openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).expect("openpt");
        rustix::pty::grantpt(&pty).expect("grantpt");
        rustix::pty::unlockpt(&pty).expect("unlockpt");
        let name = rustix::pty::ptsname(&pty, Vec::new()).expect("ptsname");
        let screen = rustix::fs::open(
            name.as_c_str(),
            OFlags::RDWR | OFlags::NOCTTY,
            Mode::empty(),
        )
        .expect("open slave pty");
        let screen = File::from(screen);
        let home = tempfile::tempdir().expect("terminal home");
        let mut command = Command::new("/bin/sh");
        command
            .current_dir(home.path())
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", home.path())
            .env("HUMATA_SOCKET", socket)
            .env("HUMATA", binary("HUMATA"))
            .stdin(Stdio::piped())
            .stdout(Stdio::from(screen.try_clone().expect("clone pty")))
            .stderr(Stdio::from(screen));
        // SAFETY: only async-signal-safe calls between fork and exec.
        unsafe {
            command.pre_exec(|| {
                rustix::process::setsid()?;
                rustix::process::ioctl_tiocsctty(BorrowedFd::borrow_raw(2))?;
                Ok(())
            });
        }
        let mut shell = command.spawn().expect("spawn external sh");
        let commands = shell.stdin.take().expect("shell stdin");
        Self {
            shell,
            commands,
            _pty: pty,
            home,
            runs: 0,
        }
    }

    fn start(&mut self, line: &str) -> usize {
        let run = self.runs;
        self.runs += 1;
        writeln!(
            self.commands,
            "{line} > out{run} 2> err{run}; echo $? > code{run}.partial && mv code{run}.partial code{run}"
        )
        .expect("write shell command");
        self.commands.flush().expect("flush shell command");
        run
    }

    fn try_finish(&self, run: usize) -> Option<CliRun> {
        let code_text = std::fs::read_to_string(self.home.path().join(format!("code{run}"))).ok()?;
        if code_text.trim().is_empty() {
            return None;
        }
        let out = std::fs::read_to_string(self.home.path().join(format!("out{run}"))).unwrap();
        let err = std::fs::read_to_string(self.home.path().join(format!("err{run}"))).unwrap();
        Some(CliRun {
            code: code_text.trim().parse().expect("numeric exit code"),
            json: serde_json::from_str(&out)
                .unwrap_or_else(|_| panic!("humata printed non-JSON: stdout={out:?} stderr={err:?}")),
        })
    }

    fn run_blocking(&mut self, line: &str) -> CliRun {
        let run = self.start(line);
        let deadline = Instant::now() + DEADLINE;
        loop {
            if let Some(done) = self.try_finish(run) {
                return done;
            }
            assert!(Instant::now() < deadline, "command timed out: {line}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for ExternalTerminal {
    fn drop(&mut self) {
        let _ = self.shell.kill();
        let _ = self.shell.wait();
    }
}

struct HeadlessApp<'a> {
    runtime: &'a Runtime,
    cx: HeadlessAppContext,
    window: AnyWindowHandle,
    ui: Rc<Ui>,
    main_notifies: Arc<AtomicUsize>,
}

impl<'a> HeadlessApp<'a> {
    fn open(runtime: &'a Runtime, clients: Clients) -> Self {
        let mut cx = humata2_engine::headless(&Resources::new(humata2_ui::Assets))
            .expect("headless context");
        cx.update(humata2_ui::init);
        let ui = Ui::new(runtime.handle(), clients, Recorder::new());
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(0.), px(0.)),
                size(px(humata2_ui::WINDOW.0), px(humata2_ui::WINDOW.1)),
            ))),
            show: false,
            focus: false,
            ..WindowOptions::default()
        };
        let main_notifies = Arc::new(AtomicUsize::new(0));
        let counter = main_notifies.clone();
        let (window, _shell) = cx.update(|cx| {
            let (window, shell) = humata2_ui::open(cx, &ui, options).expect("open window");
            cx.observe(&shell, move |_, _| {
                counter.fetch_add(1, Ordering::SeqCst);
            })
            .detach();
            (window, shell)
        });
        let mut app = Self {
            runtime,
            cx,
            window,
            ui,
            main_notifies,
        };
        app.settle();
        app
    }

    fn main_notifies(&self) -> usize {
        self.main_notifies.load(Ordering::SeqCst)
    }

    fn settle(&mut self) {
        for _ in 0..4 {
            self.runtime.block_on(async {
                for _ in 0..8 {
                    tokio::task::yield_now().await;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            });
            self.cx.run_until_parked();
        }
        self.cx
            .update_window(self.window, |_, window, cx| {
                humata2_engine::frame::natural_frame(window, cx);
            })
            .expect("window stays open");
    }

    fn view(&mut self) -> String {
        GpuiDriver::with_frames(&mut self.cx, self.window, FrameMode::Natural)
            .view()
            .expect("read view")
    }

    fn click(&mut self, role: Role, label: &str) {
        let action = Action::Click(Target::new(role, label));
        GpuiDriver::with_frames(&mut self.cx, self.window, FrameMode::Natural)
            .perform(&action)
            .unwrap_or_else(|e| panic!("{action} failed: {e}\nview:\n{}", self.view()));
        self.settle();
    }

    fn wait_until(&mut self, what: &str, mut predicate: impl FnMut(&mut Self) -> bool) {
        let deadline = Instant::now() + DEADLINE;
        loop {
            self.settle();
            if predicate(self) {
                return;
            }
            if Instant::now() >= deadline {
                let pending = self.ui.read(|s| s.pending_cli_logins.clone());
                let view = self.view();
                panic!(
                    "timed out waiting for {what}\npending_cli_logins in store: {pending:?}\nrendered view:\n{view}"
                );
            }
        }
    }
}

#[test]
fn external_humata_login_surfaces_banner_and_approves_in_desktop_app() {
    let runtime = runtime();
    let (daemon, clients) = runtime.block_on(async {
        let daemon = TestDaemon::start().await;
        let clients = daemon.clients().await;
        (daemon, clients)
    });
    let mut app = HeadlessApp::open(&runtime, clients);
    app.wait_until("app workspace to load", |app| {
        app.ui.read(|s| s.load == Load::Ready && s.route == Route::Home)
    });

    let mut term = ExternalTerminal::open(&daemon.socket);
    // Before login, user-only calls fail with presence_required (exit 5).
    let before = term.run_blocking(r#""$HUMATA" --json logins"#);
    assert_eq!(before.code, NEEDS_USER, "before login: {}", before.json);

    // Start `humata login` from the external terminal.
    let notifies_before_login = app.main_notifies();
    let login_run = term.start(r#""$HUMATA" --json login"#);

    // Wait for the pending CLI login event to reach the app store, and verify
    // that Part::Main (`Shell`) was actually notified to repaint (without
    // relying on an external forced frame).
    app.wait_until("pending_cli_logins to populate in app store", |app| {
        app.ui.read(|s| !s.pending_cli_logins.is_empty())
    });
    assert!(
        app.main_notifies() > notifies_before_login,
        "Shell (Part::Main) was never notified when Event::PendingCliLogin arrived; \
         the real desktop app will never repaint to show the CLI login banner!"
    );

    // Without navigating or triggering any unrelated redraw, the CLI login
    // approval banner must paint in the natural GPUI frame.
    app.wait_until("CLI login Approve button to render in banner", |app| {
        let view = app.view();
        view.contains("CLI login request") && view.contains(r#"Button "Approve""#)
    });

    // Click Approve in the desktop app banner.
    let notifies_before_approve = app.main_notifies();
    app.click(Role::Button, "Approve");

    // The waiting `humata login` completes with exit 0 and saves the token.
    let mut login_result = None;
    app.wait_until("humata login process to exit", |_| {
        login_result = term.try_finish(login_run);
        login_result.is_some()
    });
    let login_result = login_result.unwrap();
    assert_eq!(login_result.code, OK, "login failed: {}", login_result.json);
    assert!(
        app.main_notifies() > notifies_before_approve,
        "Shell (Part::Main) was never notified after approving the CLI login"
    );

    // The banner disappears from the rendered view without manual navigation.
    app.wait_until("CLI login banner to clear after approval", |app| {
        !app.view().contains("CLI login request")
    });

    // The terminal is now authenticated for user-only CLI calls.
    let after = term.run_blocking(r#""$HUMATA" --json logins"#);
    assert_eq!(after.code, OK, "after login: {}", after.json);

    // Also verify Deny on the Inbox route.
    app.click(Role::Link, "Agent inbox");
    app.wait_until("Inbox route", |app| {
        app.ui.read(|s| s.route == Route::Inbox)
    });
    let mut second = ExternalTerminal::open(&daemon.socket);
    let denied_run = second.start(r#""$HUMATA" --json login"#);
    app.wait_until("CLI login Deny button to render in Inbox", |app| {
        let view = app.view();
        view.contains("CLI login request") && view.contains(r#"Button "Deny""#)
    });
    app.click(Role::Button, "Deny");
    let mut denied_result = None;
    app.wait_until("denied humata login to exit", |_| {
        denied_result = second.try_finish(denied_run);
        denied_result.is_some()
    });
    assert_eq!(denied_result.unwrap().code, DENIED);
    app.wait_until("CLI login card to clear after deny", |app| {
        !app.view().contains("CLI login request")
    });

    drop(app);
    drop(daemon);
}

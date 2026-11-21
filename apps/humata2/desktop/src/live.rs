//! How the desktop app starts on the real daemon: where the host tools it
//! carries are, which backend to run on, installing the tools, and waiting
//! for the daemon's socket.

#[path = "notifier.rs"]
pub(crate) mod notifier;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use humata2_clients::{
    Clipboard, Clients, DaemonError, FakeFileSystem, FakeKeychain, LocalDaemon, Reply,
};
use humata2_clients_live::{
    ConnectError, DaemonFacts, LiveComments, LiveDaemon, LiveMachines, LiveOptions, Observer,
};
use humata2_install::{AGENT_LABEL, Bundle, Installed, Layout, SystemLaunchd};
use tca::SystemClock;

/// What the app should run on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Backend {
    /// The demo workspace.
    Fakes,
    /// The daemon at this socket.
    Live { socket: PathBuf },
}

/// Why the app couldn't start on the real daemon.
#[derive(Debug, thiserror::Error)]
pub(crate) enum LiveStartError {
    /// Installing the host tools failed.
    #[error("Humata couldn't install its host tools: {source}")]
    Install {
        /// What the installer said.
        #[source]
        source: humata2_install::InstallError,
    },
    /// The install's thread stopped before it finished.
    #[error(
        "Humata's tool install stopped before it finished. Open Humata again; if it keeps stopping, report a bug with the log in ~/.humata2/logs."
    )]
    InstallStopped {
        /// What the runtime said.
        #[source]
        source: tokio::task::JoinError,
    },
    /// The socket never accepted.
    #[error(
        "Humata's daemon didn't come up at {} within {} s. Look at ~/.humata2/logs/launchd.log for why it stopped, then restart it with `launchctl kickstart -k gui/$(id -u)/{AGENT_LABEL}` and open Humata again.",
        socket.display(),
        waited.as_secs()
    )]
    NoDaemon {
        /// The socket waited on.
        socket: PathBuf,
        /// How long the app waited.
        waited: Duration,
    },
    /// The daemon is there but the app couldn't talk to it.
    #[error("{source}")]
    Connect {
        /// Why connecting failed.
        #[source]
        source: ConnectError,
    },
}

/// Where the app finds the host tools it carries:
/// `<App>.app/Contents/Helpers/{humatad2,humata2_holder,humata,stack,humata2_fusefs}` when the
/// running executable is `<App>.app/Contents/MacOS/<exe>`; None otherwise (a
/// bazel-run dev binary). The release they belong to is named by
/// `Contents/Resources/manifest.json` when the app ships one (UP-1, D281);
/// it sits outside Helpers, which holds only signed executables.
pub(crate) fn bundled_tools(exe: &Path) -> Option<Bundle> {
    let macos = exe.parent()?;
    if macos.file_name()? != "MacOS" {
        return None;
    }
    let contents = macos.parent()?;
    if contents.file_name()? != "Contents" {
        return None;
    }
    let app = contents.parent()?;
    if app.extension()? != "app" {
        return None;
    }
    let helpers = contents.join("Helpers");
    let bundle = Bundle::new(
        helpers.join("humatad2"),
        helpers.join("humata2_holder"),
        helpers.join("humata"),
        helpers.join("stack"),
        helpers.join("humata2_fusefs"),
    );
    let manifest = contents.join("Resources").join("manifest.json");
    if !manifest.is_file() {
        return Some(bundle);
    }
    // The Linux tools Add machine installs from (TS-2), when the app carries them.
    let machine_tools = contents.join("Resources").join("machine-tools");
    let bundle = bundle.with_manifest(manifest);
    Some(if machine_tools.join("manifest.json").is_file() { bundle.with_machine_tools(machine_tools) } else { bundle })
}

/// This Mac's own daemon for the one thing a daemon too old to hold a Migrate
/// item can't do itself (D374): swap the staged release in and restart.
/// Wraps `humata2_install::migrate_old_daemon` on a blocking task.
struct LiveLocalDaemon {
    layout: Layout,
    /// The daemon's socket, which a restart removes and makes again.
    socket: PathBuf,
}

impl LiveLocalDaemon {
    const WHAT: &'static str = "Migrating the daemon on This Mac";

    /// How long one look at the socket may take. A daemon that accepts but
    /// doesn't answer is not back; the next filesystem event looks again.
    const LOOK: Duration = Duration::from_secs(5);

    fn refused(reason: impl Into<String>) -> DaemonError {
        DaemonError::Refused { what: Self::WHAT.to_owned(), reason: reason.into() }
    }

    /// Whether the daemon at `socket` answers ServerInfo with `bundle`.
    async fn answers_at(socket: &Path, bundle: &str) -> bool {
        let look = LiveDaemon::connect(LiveOptions::new(socket.to_path_buf()));
        matches!(
            tokio::time::timeout(Self::LOOK, look).await,
            Ok(Ok(daemon)) if daemon.facts().bundle == bundle
        )
    }
}

impl LocalDaemon for LiveLocalDaemon {
    fn migrate_old_daemon(&self, staged_bundle: &str) -> Reply<()> {
        let layout = self.layout.clone();
        let shown = staged_bundle.to_owned();
        Box::pin(async move {
            let done = tokio::task::spawn_blocking(move || -> Result<(), DaemonError> {
                // Migrate what the card showed, not a release staged since.
                let staged = humata2_install::staged_release(&layout)
                    .map_err(|error| LiveLocalDaemon::refused(error.to_string()))?;
                if staged.as_deref() != Some(shown.as_str()) {
                    return Err(LiveLocalDaemon::refused(
                        "the release staged beside the daemon is not the one the Migrate item showed. Quit Humata and open it again to see the current Migrate item; nothing was changed",
                    ));
                }
                humata2_install::migrate_old_daemon(&layout, &SystemLaunchd)
                    .map_err(|error| LiveLocalDaemon::refused(error.to_string()))
            })
            .await;
            match done {
                Ok(result) => result,
                Err(error) => Err(LiveLocalDaemon::refused(format!(
                    "the migration's thread stopped before it finished ({error}). Open Humata again; if the old daemon is still running, click Migrate again, and report a bug with the log in ~/.humata2/logs if it keeps stopping"
                ))),
            }
        })
    }

    fn back_at(&self, staged_bundle: &str) -> Reply<()> {
        let socket = self.socket.clone();
        let bundle = staged_bundle.to_owned();
        Box::pin(async move {
            let Some(dir) = socket.parent().map(Path::to_path_buf) else {
                return Err(Self::refused(format!(
                    "{} has no folder to watch for the new daemon. Quit Humata and open it again",
                    socket.display()
                )));
            };
            // Each change in the socket's folder wakes one look; a change
            // that lands while a look runs is kept for the next one.
            let changed = Arc::new(tokio::sync::Notify::new());
            let wake = changed.clone();
            let mut watcher = notify::recommended_watcher(move |_event: notify::Result<notify::Event>| {
                wake.notify_one();
            })
            .map_err(|error| {
                Self::refused(format!(
                    "Humata couldn't watch {} for the new daemon ({error}). Quit Humata and open it again",
                    dir.display()
                ))
            })?;
            notify::Watcher::watch(&mut watcher, &dir, notify::RecursiveMode::NonRecursive)
                .map_err(|error| {
                    Self::refused(format!(
                        "Humata couldn't watch {} for the new daemon ({error}). Quit Humata and open it again",
                        dir.display()
                    ))
                })?;
            // The watch is set before the first look, so a daemon that came
            // up already is seen and one that comes up next is not missed.
            loop {
                if Self::answers_at(&socket, &bundle).await {
                    return Ok(());
                }
                changed.notified().await;
            }
        })
    }
}

/// What installing the host tools did, and the bundle id of the app's
/// release when it is staged beside an older daemon, waiting for Migrate.
pub(crate) type InstallReport = (Installed, Option<String>);

/// What the app starts with: its clients, what the install did, and what
/// the daemon said about itself.
pub(crate) type Started = (Clients, Option<InstallReport>, DaemonFacts);

/// Fakes unless `--live` was given or the app runs from a bundle;
/// `--socket` overrides `~/.humata2/humatad.sock`.
pub(crate) fn backend(
    live_flag: bool,
    socket_flag: Option<PathBuf>,
    exe: &Path,
    home: &Path,
) -> Backend {
    if !live_flag && socket_flag.is_none() && bundled_tools(exe).is_none() {
        return Backend::Fakes;
    }
    let socket = socket_flag.unwrap_or_else(|| Layout::for_home(home).socket().to_path_buf());
    Backend::Live { socket }
}

/// Where start-up is, for the window shown while it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    /// Copying the host tools in and loading the daemon's launchd agent.
    Installing,
    /// Waiting for the daemon's socket to accept.
    Waiting,
}

/// The phase start-up begins in: only a bundle has tools to install.
pub(crate) fn first_phase(bundle_present: bool) -> Phase {
    if bundle_present { Phase::Installing } else { Phase::Waiting }
}

/// Writes copied text to the macOS pasteboard via `/usr/bin/pbcopy`.
#[derive(Clone, Copy, Debug, Default)]
struct SystemClipboard;

impl SystemClipboard {
    const fn new() -> Self {
        Self
    }
}

fn pipe_to_command(program: &str, args: &[&str], text: &str) -> std::io::Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes())?;
    }
    let _ = child.wait()?;
    Ok(())
}

impl Clipboard for SystemClipboard {
    fn copy(&self, text: &str) {
        #[cfg(target_os = "macos")]
        {
            let _ = pipe_to_command("/usr/bin/pbcopy", &[], text);
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = text;
        }
    }
}

/// Starts on the real daemon: from a bundle, installs the host tools first
/// (TS-0, PER-20); then waits up to `wait` for the socket to accept and
/// connects. `progress` hears each phase as it begins, so a window can show
/// it. The file system and keychain are the fakes the demo uses, because no
/// real ones exist yet.
///
/// An install that finds an older daemon stages the app's release instead of
/// swapping it in (D301); the report then names that release, so the app
/// can post "Migrate the daemon on This Mac".
pub(crate) async fn start_live(
    socket: &Path,
    bundle: Option<&Bundle>,
    home: &Path,
    storage: Arc<dyn tca::Storage>,
    wait: Duration,
    calls: Option<Observer>,
    progress: impl Fn(Phase),
) -> Result<Started, LiveStartError> {
    let installer = bundle.map(|bundle| {
        let bundle = bundle.clone();
        let layout = Layout::for_home(home);
        move || -> Result<InstallReport, humata2_install::InstallError> {
            let installed = humata2_install::install(&bundle, &layout, &SystemLaunchd)?;
            let staged = humata2_install::staged_release(&layout)?;
            Ok((installed, staged))
        }
    });
    let local: Arc<dyn LocalDaemon> = Arc::new(LiveLocalDaemon {
        layout: Layout::for_home(home),
        socket: socket.to_path_buf(),
    });
    start_with(installer, socket, storage, local, wait, calls, progress).await
}

/// [`start_live`] with the install passed in, so a test can stand in for the
/// real one (which writes under HOME and runs launchctl).
async fn start_with<R: Send + 'static>(
    installer: Option<impl FnOnce() -> Result<R, humata2_install::InstallError> + Send + 'static>,
    socket: &Path,
    storage: Arc<dyn tca::Storage>,
    local_daemon: Arc<dyn LocalDaemon>,
    wait: Duration,
    calls: Option<Observer>,
    progress: impl Fn(Phase),
) -> Result<(Clients, Option<R>, DaemonFacts), LiveStartError> {
    let installed = match installer {
        Some(installer) => {
            progress(Phase::Installing);
            // File copies, hashing and launchctl block, so they stay off the
            // runtime's workers.
            let done = tokio::task::spawn_blocking(installer)
                .await
                .map_err(|source| LiveStartError::InstallStopped { source })?;
            Some(done.map_err(|source| LiveStartError::Install { source })?)
        }
        None => None,
    };
    progress(Phase::Waiting);
    wait_for_socket(socket, wait).await?;
    let mut options = LiveOptions::new(socket.to_path_buf());
    if let Some(calls) = calls {
        options = options.observe_calls(move |call| calls(call));
    }
    let daemon = LiveDaemon::connect(options)
        .await
        .map_err(|source| LiveStartError::Connect { source })?;
    let file_manager = humata2_clients_live::LiveFileManager::new(&daemon);
    let facts = daemon.facts().clone();
    let (notifier, _notifier_bridge) = notifier::LiveNotifier::new();
    let clients = Clients {
        daemon: Arc::new(daemon.clone()),
        desk: Arc::new(daemon.clone()),
        // Machines, providers and skills over the same connection (HO-1, MP-2, SK-2).
        machines: Arc::new(LiveMachines::new(&daemon)),
        comments: Arc::new(LiveComments::new(&daemon)),
        history: Arc::new(humata2_clients_live::history::LiveHistory::new(&daemon)),
        marks: Arc::new(humata2_clients_live::marks::LiveMarks::new(&daemon)),
        todo: Arc::new(humata2_clients_live::todo::LiveTodo::new(&daemon)),
        views: Arc::new(humata2_clients_live::LiveViews::new(&daemon)),
        pull_requests: Arc::new(humata2_clients_live::LivePullRequests::new(&daemon)),
        review_chat: Arc::new(humata2_clients_live::LiveReviewChat::new(&daemon)),
        terminals: Arc::new(daemon),
        clock: Arc::new(SystemClock::new()),
        clipboard: Arc::new(SystemClipboard::new()),
        reveal: Arc::new(humata2_clients::FinderReveal::new()),
        files: Arc::new(FakeFileSystem::default()),
        file_manager: Arc::new(file_manager),
        keychain: Arc::new(FakeKeychain::default()),
        local_daemon,
        notifier: Arc::new(notifier),
        storage,
    };
    Ok((clients, installed, facts))
}

async fn wait_for_socket(socket: &Path, wait: Duration) -> Result<(), LiveStartError> {
    let start = Instant::now();
    loop {
        if tokio::net::UnixStream::connect(socket).await.is_ok() {
            return Ok(());
        }
        if start.elapsed() >= wait {
            return Err(LiveStartError::NoDaemon {
                socket: socket.to_path_buf(),
                waited: wait,
            });
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Whether the person's own login shell finds `humata` (D137): runs
/// `$SHELL -ilc 'command -v humata'` with a 3 s timeout; None if it couldn't
/// tell.
pub(crate) fn shell_finds_humata(shell: &Path) -> Option<bool> {
    let mut child = Command::new(shell)
        .args(["-ilc", "command -v humata"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status.success()),
            Ok(None) if start.elapsed() < Duration::from_secs(3) => {
                std::thread::sleep(Duration::from_millis(25));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP_EXE: &str = "/Applications/Humata 2.app/Contents/MacOS/humata2";

    #[test]
    fn bundled_tools_found_inside_an_app() {
        // A folder that never exists: the real /Applications copy may ship a
        // manifest, which this case must not read.
        let bundle = bundled_tools(Path::new("/humata2-test-absent/Humata 2.app/Contents/MacOS/humata2"));
        let helpers = "/humata2-test-absent/Humata 2.app/Contents/Helpers";
        let want = Bundle::new(
            PathBuf::from(format!("{helpers}/humatad2")),
            PathBuf::from(format!("{helpers}/humata2_holder")),
            PathBuf::from(format!("{helpers}/humata")),
            PathBuf::from(format!("{helpers}/stack")),
            PathBuf::from(format!("{helpers}/humata2_fusefs")),
        );
        assert_eq!(format!("{bundle:?}"), format!("{:?}", Some(want)));
    }

    #[test]
    fn bundled_tools_name_the_release_the_app_ships() {
        let root = std::env::temp_dir().join(format!("humata2-live-test-manifest-{}", std::process::id()));
        let contents = root.join("Humata 2.app/Contents");
        std::fs::create_dir_all(contents.join("MacOS")).expect("MacOS");
        std::fs::create_dir_all(contents.join("Resources")).expect("Resources");
        let exe = contents.join("MacOS/humata2");
        let without = bundled_tools(&exe).expect("an app's tools");
        let manifest = contents.join("Resources/manifest.json");
        std::fs::write(&manifest, "{}").expect("manifest");
        let with = bundled_tools(&exe).expect("an app's tools");
        let helpers = contents.join("Helpers");
        let tools = || {
            Bundle::new(
                helpers.join("humatad2"),
                helpers.join("humata2_holder"),
                helpers.join("humata"),
                helpers.join("stack"),
                helpers.join("humata2_fusefs"),
            )
        };
        assert_eq!(format!("{without:?}"), format!("{:?}", tools()));
        assert_eq!(format!("{with:?}"), format!("{:?}", tools().with_manifest(manifest)));
        std::fs::remove_dir_all(&root).expect("clean up");
    }

    #[test]
    fn bundled_tools_carry_the_linux_tools_an_app_ships() {
        let root = std::env::temp_dir().join(format!("humata2-live-test-machine-tools-{}", std::process::id()));
        let contents = root.join("Humata 2.app/Contents");
        let folder = contents.join("Resources/machine-tools");
        std::fs::create_dir_all(contents.join("MacOS")).expect("MacOS");
        std::fs::create_dir_all(&folder).expect("machine-tools");
        let exe = contents.join("MacOS/humata2");
        let manifest = contents.join("Resources/manifest.json");
        std::fs::write(&manifest, "{}").expect("manifest");
        let helpers = contents.join("Helpers");
        let tools = || {
            Bundle::new(
                helpers.join("humatad2"),
                helpers.join("humata2_holder"),
                helpers.join("humata"),
                helpers.join("stack"),
                helpers.join("humata2_fusefs"),
            )
            .with_manifest(manifest.clone())
        };
        let empty = bundled_tools(&exe).expect("an app's tools");
        std::fs::write(folder.join("manifest.json"), "{}").expect("machine-tools manifest");
        let with = bundled_tools(&exe).expect("an app's tools");
        assert_eq!(format!("{empty:?}"), format!("{:?}", tools()));
        assert_eq!(format!("{with:?}"), format!("{:?}", tools().with_machine_tools(folder)));
        std::fs::remove_dir_all(&root).expect("clean up");
    }

    #[test]
    fn bundled_tools_absent_for_a_dev_binary() {
        assert!(bundled_tools(Path::new("/tmp/bazel-out/humata2")).is_none());
    }

    #[test]
    fn backend_choice() {
        let home = Path::new("/home/x");
        let dev = Path::new("/tmp/bazel-out/humata2");
        assert_eq!(backend(false, None, dev, home), Backend::Fakes);
        assert_eq!(
            backend(false, None, Path::new(APP_EXE), home),
            Backend::Live { socket: PathBuf::from("/home/x/.humata2/humatad.sock") }
        );
        assert_eq!(
            backend(true, None, dev, home),
            Backend::Live { socket: PathBuf::from("/home/x/.humata2/humatad.sock") }
        );
        assert_eq!(
            backend(true, Some(PathBuf::from("/tmp/s.sock")), Path::new(APP_EXE), home),
            Backend::Live { socket: PathBuf::from("/tmp/s.sock") }
        );
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
    }

    fn installed() -> Installed {
        Installed {
            humatad2: humata2_install::BinaryChange::Unchanged,
            holder: humata2_install::BinaryChange::Unchanged,
            humata: humata2_install::BinaryChange::Unchanged,
            stack: humata2_install::BinaryChange::Unchanged,
            fusefs: humata2_install::BinaryChange::Unchanged,
            agent: humata2_install::AgentChange::Unchanged,
            path_hint: None,
            path_clash: None,
        }
    }

    #[test]
    fn no_listener_means_no_daemon() {
        let socket = std::env::temp_dir().join("humata2-live-test-nobody.sock");
        let result = runtime().block_on(start_live(
            &socket,
            None,
            Path::new("/nonexistent-home"),
            Arc::new(tca::MemoryStorage::new()),
            Duration::from_millis(250),
            None,
            |_| {},
        ));
        assert!(matches!(result, Err(LiveStartError::NoDaemon { .. })));
    }

    #[test]
    fn a_bundle_installs_then_waits() {
        let socket = std::env::temp_dir().join("humata2-live-test-phases.sock");
        let heard = std::sync::Mutex::new(Vec::new());
        let result = runtime().block_on(start_with(
            Some(|| Ok(installed())),
            &socket,
            Arc::new(tca::MemoryStorage::new()),
            Arc::new(humata2_clients::FakeLocalDaemon::default()),
            Duration::from_millis(100),
            None,
            |phase| heard.lock().expect("lock").push(phase),
        ));
        assert!(matches!(result, Err(LiveStartError::NoDaemon { .. })));
        assert_eq!(*heard.lock().expect("lock"), [Phase::Installing, Phase::Waiting]);
    }

    #[test]
    fn no_bundle_goes_straight_to_waiting() {
        let socket = std::env::temp_dir().join("humata2-live-test-nobundle.sock");
        let heard = std::sync::Mutex::new(Vec::new());
        let result = runtime().block_on(start_with(
            None::<fn() -> Result<Installed, humata2_install::InstallError>>,
            &socket,
            Arc::new(tca::MemoryStorage::new()),
            Arc::new(humata2_clients::FakeLocalDaemon::default()),
            Duration::from_millis(100),
            None,
            |phase| heard.lock().expect("lock").push(phase),
        ));
        assert!(matches!(result, Err(LiveStartError::NoDaemon { .. })));
        assert_eq!(*heard.lock().expect("lock"), [Phase::Waiting]);
        assert_eq!(first_phase(false), Phase::Waiting);
        assert_eq!(first_phase(true), Phase::Installing);
    }

    #[test]
    fn an_install_that_dies_never_waits() {
        let socket = std::env::temp_dir().join("humata2-live-test-badinstall.sock");
        let heard = std::sync::Mutex::new(Vec::new());
        let result = runtime().block_on(start_with(
            Some(|| -> Result<Installed, humata2_install::InstallError> { panic!("install blew up") }),
            &socket,
            Arc::new(tca::MemoryStorage::new()),
            Arc::new(humata2_clients::FakeLocalDaemon::default()),
            Duration::from_millis(100),
            None,
            |phase| heard.lock().expect("lock").push(phase),
        ));
        assert!(matches!(result, Err(LiveStartError::InstallStopped { .. })));
        assert_eq!(*heard.lock().expect("lock"), [Phase::Installing]);
    }

    #[test]
    fn a_shell_can_be_asked() {
        assert!(shell_finds_humata(Path::new("/bin/sh")).is_some());
    }

    #[test]
    fn clipboard_pipe_writes_utf8_text_to_command_stdin() {
        let out = std::env::temp_dir().join(format!(
            "humata2-live-test-clipboard-{}.txt",
            std::process::id()
        ));
        let path = out.to_string_lossy().into_owned();
        let sample = "copied from terminal \u{2713}\nline two";
        pipe_to_command("/bin/sh", &["-c", "cat > \"$1\"", "_", &path], sample)
            .expect("pipe to command");
        let written = std::fs::read_to_string(&out).expect("read piped output");
        let _ = std::fs::remove_file(&out);
        assert_eq!(written, sample);
    }
}

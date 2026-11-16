//! How the desktop app starts on the real daemon: where the host tools it
//! carries are, which backend to run on, installing the tools, and waiting
//! for the daemon's socket.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use humata2_clients::{
    Clients, FakeClipboard, FakeFileSystem, FakeKeychain,
};
use humata2_clients_live::{ConnectError, LiveDaemon, LiveOptions};
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
/// `<App>.app/Contents/Helpers/{humatad2,humata2_holder,humata}` when the
/// running executable is `<App>.app/Contents/MacOS/<exe>`; None otherwise (a
/// bazel-run dev binary).
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
    Some(Bundle::new(
        helpers.join("humatad2"),
        helpers.join("humata2_holder"),
        helpers.join("humata"),
    ))
}

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

/// Starts on the real daemon: from a bundle, installs the host tools first
/// (TS-0, PER-20); then waits up to `wait` for the socket to accept and
/// connects. The clock, clipboard, file system and keychain are the fakes
/// the demo uses, because no real ones exist yet.
pub(crate) async fn start_live(
    socket: &Path,
    bundle: Option<&Bundle>,
    home: &Path,
    storage: Arc<dyn tca::Storage>,
    wait: Duration,
) -> Result<(Clients, Option<Installed>), LiveStartError> {
    let installed = match bundle {
        Some(bundle) => {
            let layout = Layout::for_home(home);
            Some(
                humata2_install::install(bundle, &layout, &SystemLaunchd)
                    .map_err(|source| LiveStartError::Install { source })?,
            )
        }
        None => None,
    };
    wait_for_socket(socket, wait).await?;
    let daemon = LiveDaemon::connect(LiveOptions::new(socket.to_path_buf()))
        .await
        .map_err(|source| LiveStartError::Connect { source })?;
    let clients = Clients {
        daemon: Arc::new(daemon.clone()),
        desk: Arc::new(daemon.clone()),
        terminals: Arc::new(daemon),
        clock: Arc::new(SystemClock::new()),
        clipboard: Arc::new(FakeClipboard::default()),
        files: Arc::new(FakeFileSystem::default()),
        file_manager: Arc::new(humata2_clients::FakeFileManager::default()),
        keychain: Arc::new(FakeKeychain::default()),
        storage,
    };
    Ok((clients, installed))
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
        let bundle = bundled_tools(Path::new(APP_EXE));
        let helpers = "/Applications/Humata 2.app/Contents/Helpers";
        let want = Bundle::new(
            PathBuf::from(format!("{helpers}/humatad2")),
            PathBuf::from(format!("{helpers}/humata2_holder")),
            PathBuf::from(format!("{helpers}/humata")),
        );
        assert_eq!(format!("{bundle:?}"), format!("{:?}", Some(want)));
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

    #[test]
    fn no_listener_means_no_daemon() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let socket = std::env::temp_dir().join("humata2-live-test-nobody.sock");
        let result = runtime.block_on(start_live(
            &socket,
            None,
            Path::new("/nonexistent-home"),
            Arc::new(tca::MemoryStorage::new()),
            Duration::from_millis(250),
        ));
        assert!(matches!(result, Err(LiveStartError::NoDaemon { .. })));
    }

    #[test]
    fn a_shell_can_be_asked() {
        assert!(shell_finds_humata(Path::new("/bin/sh")).is_some());
    }
}

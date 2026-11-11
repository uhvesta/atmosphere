use std::path::PathBuf;

/// Overrides where the daemon listens and clients connect.
pub const SOCKET_ENV: &str = "KHODR_SOCKET";

/// Where the daemon listens unless told otherwise: `$KHODR_SOCKET`, else
/// `~/Library/Application Support/Khodr/daemon.sock` on macOS and
/// `$XDG_RUNTIME_DIR/khodr/daemon.sock` (or `~/.local/state/khodr/daemon.sock`)
/// elsewhere. None if there is no home folder to put it in.
pub fn default_socket() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(SOCKET_ENV).filter(|path| !path.is_empty()) {
        return Some(PathBuf::from(path));
    }
    if cfg!(target_os = "macos") {
        return home().map(|home| home.join("Library/Application Support/Khodr/daemon.sock"));
    }
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR").filter(|path| !path.is_empty()) {
        return Some(PathBuf::from(runtime).join("khodr/daemon.sock"));
    }
    home().map(|home| home.join(".local/state/khodr/daemon.sock"))
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

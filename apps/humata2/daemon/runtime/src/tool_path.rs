//! The `PATH` the daemon finds its tools on (`gh`, `git`, the agent CLIs).
//!
//! launchd starts the daemon with `/usr/bin:/bin:/usr/sbin:/sbin`, so `gh`,
//! `copilot` and `codex` from Homebrew (`/opt/homebrew/bin`) and `claude`
//! from `~/.local/bin` were "not installed" (owner, 0.2.1). With
//! `HUMATA2_LOGIN_PATH=1`, which the launch agent sets, `serve` asks the
//! person's login shell for its `PATH` once, at start, before any thread
//! runs, and adds the usual install folders after it. Every caller, the
//! session tabs it starts too, then finds what the person's own shell finds.

use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long the login shell may take to say its `PATH`. A slow profile
/// costs the daemon's start this long once; past it the usual folders
/// still apply.
const SHELL_TIMEOUT: Duration = Duration::from_secs(5);

/// What the shell prints just before its `PATH`, so a profile that prints
/// a banner doesn't end up in it.
const MARK: &str = "__HUMATA2_PATH__";

/// Folders tools are installed into when the login shell doesn't say:
/// Homebrew's on Apple silicon and Intel, then the per-user ones.
const USUAL: [&str; 3] = ["/opt/homebrew/bin", "/opt/homebrew/sbin", "/usr/local/bin"];
const USUAL_IN_HOME: [&str; 2] = [".local/bin", "bin"];

/// The `gh` every caller runs: `HUMATA2_GH` in an admin build (the
/// tests' fake), else `gh`, found on the widened `PATH`.
pub fn gh_program() -> OsString {
    if humata2_rpc_kit::admin::is_admin_mode()
        && let Some(program) = std::env::var_os("HUMATA2_GH")
    {
        return program;
    }
    OsString::from("gh")
}

/// The variable the launch agent sets. An environment variable, not a
/// flag: an older daemon a new launch agent starts ignores it, where it
/// would refuse an unknown flag and not start.
const ASKED: &str = "HUMATA2_LOGIN_PATH";

/// Whether this process was asked to widen its `PATH`.
pub fn asked() -> bool {
    std::env::var_os(ASKED).is_some_and(|value| value == "1")
}

/// Widens this process's `PATH` as the module says. Call it only while the
/// process has one thread: it sets an environment variable.
pub fn widen_from_login_shell() {
    let home = std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from);
    let shell = std::env::var_os("SHELL")
        .filter(|shell| !shell.is_empty())
        .unwrap_or_else(|| {
            OsString::from(if cfg!(target_os = "macos") {
                "/bin/zsh"
            } else {
                "/bin/sh"
            })
        });
    let login = shell_path(Path::new(&shell));
    if login.is_none() {
        humata2_rpc_kit::log::say!(
            "humatad2: couldn't learn {}'s PATH, so tools are looked for in the usual folders too; if gh or an agent CLI still isn't found, check that `{} -l -i -c 'echo $PATH'` finishes and prints it",
            Path::new(&shell).display(),
            Path::new(&shell).display()
        );
    }
    let current = std::env::var_os("PATH");
    let path = widened(current.as_deref(), login.as_deref(), home.as_deref());
    // SAFETY: main calls this before the runtime starts, while this
    // process has one thread, so nothing reads the environment as it
    // changes. The ask isn't passed on to the tabs and tools started.
    unsafe {
        std::env::remove_var(ASKED);
        if let Some(path) = path {
            std::env::set_var("PATH", path);
        }
    }
}

/// The `PATH` `shell` has as a login, interactive shell; none when it
/// fails, says nothing, or takes longer than `SHELL_TIMEOUT`.
fn shell_path(shell: &Path) -> Option<OsString> {
    let mut child = Command::new(shell)
        .args(["-l", "-i", "-c", &format!("printf '{MARK}%s' \"$PATH\"")])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = (&mut stdout).take(1 << 16).read_to_end(&mut bytes);
        bytes
    });
    let deadline = Instant::now() + SHELL_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let printed = String::from_utf8(reader.join().ok()?).ok()?;
    let path = printed.rsplit_once(MARK)?.1.trim();
    (!path.is_empty()).then(|| OsString::from(path))
}

/// `login` (the shell's `PATH`) first, as the person's shell orders it,
/// then `current`, then the usual install folders, each folder once. None
/// when the result can't be a `PATH` (a folder holds a `:`).
fn widened(
    current: Option<&OsStr>,
    login: Option<&OsStr>,
    home: Option<&Path>,
) -> Option<OsString> {
    let mut folders: Vec<PathBuf> = Vec::new();
    let mut add = |folder: PathBuf| {
        if !folder.as_os_str().is_empty() && !folders.contains(&folder) {
            folders.push(folder);
        }
    };
    for path in [login, current].into_iter().flatten() {
        std::env::split_paths(path).for_each(&mut add);
    }
    USUAL.iter().map(PathBuf::from).for_each(&mut add);
    if let Some(home) = home {
        USUAL_IN_HOME
            .iter()
            .map(|folder| home.join(folder))
            .for_each(&mut add);
    }
    std::env::join_paths(folders).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::os::unix::fs::PermissionsExt;

    fn folders(path: &OsStr) -> Vec<String> {
        std::env::split_paths(path)
            .map(|folder| folder.display().to_string())
            .collect()
    }

    #[test]
    fn launchds_bare_path_gains_the_shells_folders_first_then_the_usual_ones() {
        let path = widened(
            Some(OsStr::new("/usr/bin:/bin:/usr/sbin:/sbin")),
            Some(OsStr::new(
                "/opt/homebrew/bin:/Users/me/.local/bin:/usr/bin:/bin",
            )),
            Some(Path::new("/Users/me")),
        )
        .unwrap();
        assert_eq!(
            folders(&path),
            [
                "/opt/homebrew/bin",
                "/Users/me/.local/bin",
                "/usr/bin",
                "/bin",
                "/usr/sbin",
                "/sbin",
                "/opt/homebrew/sbin",
                "/usr/local/bin",
                "/Users/me/bin",
            ]
        );
    }

    #[test]
    fn without_the_shells_path_the_usual_folders_still_come_after_the_current_ones() {
        let path = widened(
            Some(OsStr::new("/usr/bin:/bin")),
            None,
            Some(Path::new("/home/me")),
        )
        .unwrap();
        let found = folders(&path);
        assert_eq!(found[..2], ["/usr/bin", "/bin"]);
        assert!(found.contains(&"/opt/homebrew/bin".to_owned()));
        assert!(found.contains(&"/home/me/.local/bin".to_owned()));
    }

    #[test]
    fn the_shells_path_is_what_follows_the_mark_even_after_a_banner() {
        let dir = tempfile::tempdir().unwrap();
        let shell = dir.path().join("shell");
        std::fs::write(
            &shell,
            "#!/bin/sh\necho 'Welcome back'\nprintf '__HUMATA2_PATH__/opt/tools/bin:/usr/bin'\n",
        )
        .unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            shell_path(&shell),
            Some(OsString::from("/opt/tools/bin:/usr/bin"))
        );
    }

    #[test]
    fn a_shell_that_fails_gives_no_path() {
        let dir = tempfile::tempdir().unwrap();
        let shell = dir.path().join("shell");
        std::fs::write(&shell, "#!/bin/sh\nexit 3\n").unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(shell_path(&shell), None);
        assert_eq!(shell_path(&dir.path().join("missing")), None);
    }
}

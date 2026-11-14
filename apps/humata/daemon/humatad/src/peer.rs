//! Who is on the other end of a connection to the daemon: the Humata app,
//! whose admin actions run at once, or another client, as `humata` or
//! `humata mcp`, whose admin actions wait for the person's approval and
//! whose requests go by the Agent Access settings, under the kind of
//! client its hello says it is.
//!
//! This keeps well-meaning agents from acting as the person; it is no wall
//! against a program set on getting round it, which, running as the same
//! user, could change Humata's files itself.

use std::path::{Path, PathBuf};
use std::process::Command;

use humata_wire::pb;
use tokio::net::UnixStream;

/// The client on one connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Peer {
    /// The Humata app this daemon came with.
    App,
    /// Any other program, with what it ran as and what started it, for the
    /// person deciding its approvals, and the surface its changes came from.
    Client {
        program: String,
        parent: String,
        surface: pb::ChangeSurface,
    },
}

impl Peer {
    /// The client connected on `stream`; one Humata can't make out counts
    /// as another client, never as the app.
    pub async fn of(stream: &UnixStream) -> Self {
        let pid = stream.peer_cred().ok().and_then(|cred| cred.pid());
        // It runs `ps`, so off the async threads.
        tokio::task::spawn_blocking(move || Self::of_process(pid))
            .await
            .unwrap_or_else(|_| Self::unknown())
    }

    fn of_process(pid: Option<i32>) -> Self {
        let Some(pid) = pid.filter(|pid| *pid > 0) else {
            return Self::unknown();
        };
        let exe = executable(pid);
        if exe.as_deref().is_some_and(is_app) {
            return Self::App;
        }
        let (program, parent) = described(pid);
        let surface = if program.split_whitespace().nth(1) == Some("mcp") {
            pb::ChangeSurface::Mcp
        } else {
            pb::ChangeSurface::CommandLine
        };
        Self::Client {
            program: if program.is_empty() {
                exe.map(|exe| exe.display().to_string()).unwrap_or_default()
            } else {
                program
            },
            parent,
            surface,
        }
    }

    fn unknown() -> Self {
        Self::Client {
            program: String::new(),
            parent: String::new(),
            surface: pb::ChangeSurface::CommandLine,
        }
    }
}

/// Whether `exe` is the app: the program in `Contents/MacOS` of the bundle
/// this humatad runs from, in `Contents/Helpers`.
fn is_app(exe: &Path) -> bool {
    if e2e_app().is_some_and(|app| app == exe) {
        return true;
    }
    let Some(contents) = std::env::current_exe()
        .ok()
        .and_then(|ours| ours.canonicalize().ok())
        .and_then(|ours| Some(ours.parent()?.parent()?.to_owned()))
    else {
        return false;
    };
    contents.ends_with("Contents") && is_app_of(&contents, exe)
}

/// Whether `exe` is the app of the bundle whose `Contents` is `contents`:
/// the program in its `Contents/MacOS`, or that same program run from
/// another path. macOS runs an app opened where it was downloaded from a
/// read-only copy elsewhere (App Translocation), while launchd runs the
/// bundle's login agent, this humatad, from where the bundle really is.
fn is_app_of(contents: &Path, exe: &Path) -> bool {
    let programs = contents.join("MacOS");
    if exe.parent() == Some(programs.as_path()) {
        return true;
    }
    let Some(name) = exe.file_name() else {
        return false;
    };
    exe.parent()
        .is_some_and(|folder| folder.ends_with("Contents/MacOS"))
        && same_contents(&programs.join(name), exe)
}

/// Whether files `ours` and `theirs` hold the same bytes; false if either
/// can't be read.
fn same_contents(ours: &Path, theirs: &Path) -> bool {
    use std::io::BufRead;
    const CHUNK: usize = 64 * 1024;
    let (Ok(our_file), Ok(their_file)) = (std::fs::File::open(ours), std::fs::File::open(theirs))
    else {
        return false;
    };
    let same_size = match (our_file.metadata(), their_file.metadata()) {
        (Ok(ours), Ok(theirs)) => ours.is_file() && theirs.is_file() && ours.len() == theirs.len(),
        _ => false,
    };
    if !same_size {
        return false;
    }
    let mut ours = std::io::BufReader::with_capacity(CHUNK, our_file);
    let mut theirs = std::io::BufReader::with_capacity(CHUNK, their_file);
    loop {
        let (Ok(ours_read), Ok(theirs_read)) = (ours.fill_buf(), theirs.fill_buf()) else {
            return false;
        };
        if ours_read.is_empty() || theirs_read.is_empty() {
            return ours_read.is_empty() && theirs_read.is_empty();
        }
        let shared = ours_read.len().min(theirs_read.len());
        if ours_read[..shared] != theirs_read[..shared] {
            return false;
        }
        ours.consume(shared);
        theirs.consume(shared);
    }
}

/// In end-to-end tests only, the program that stands in for the app.
#[cfg(feature = "e2e")]
fn e2e_app() -> Option<PathBuf> {
    std::env::var_os("HUMATA_E2E_APP").and_then(|path| PathBuf::from(path).canonicalize().ok())
}

#[cfg(not(feature = "e2e"))]
fn e2e_app() -> Option<PathBuf> {
    None
}

/// The program process `pid` runs, as the system has it rather than as it
/// names itself.
#[cfg(target_os = "macos")]
fn executable(pid: i32) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    // PROC_PIDPATHINFO_MAXSIZE.
    const LONGEST: usize = 4 * 1024;
    unsafe extern "C" {
        fn proc_pidpath(pid: i32, buffer: *mut std::ffi::c_void, size: u32) -> i32;
    }
    let mut buffer = vec![0u8; LONGEST];
    // SAFETY: the buffer is LONGEST bytes, as told, and outlives the call.
    let written = unsafe { proc_pidpath(pid, buffer.as_mut_ptr().cast(), LONGEST as u32) };
    let written = usize::try_from(written).ok().filter(|n| *n > 0)?;
    buffer.truncate(written);
    let path = PathBuf::from(std::ffi::OsStr::from_bytes(&buffer));
    Some(path.canonicalize().unwrap_or(path))
}

#[cfg(not(target_os = "macos"))]
fn executable(pid: i32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/exe")).ok()
}

/// What process `pid` ran as, with its arguments, and the name of the
/// program that started it, as `ps` tells them; empty where it can't.
fn described(pid: i32) -> (String, String) {
    let ps = |format: &str, pid: &str| {
        Command::new("/bin/ps")
            .args(["-o", format, "-p", pid])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .unwrap_or_default()
    };
    let line = ps("ppid=,args=", &pid.to_string());
    let (ppid, args) = line
        .trim_start()
        .split_once(char::is_whitespace)
        .unwrap_or((line.as_str(), ""));
    let program = shortened(args.trim());
    let parent = if ppid.is_empty() {
        String::new()
    } else {
        let comm = ps("comm=", ppid);
        Path::new(&comm)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or(comm)
    };
    (program, parent)
}

/// `args` with the program named by its file name, not its whole path.
fn shortened(args: &str) -> String {
    let (program, rest) = args.split_once(' ').unwrap_or((args, ""));
    let name = Path::new(program)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| program.to_owned());
    if rest.is_empty() {
        name
    } else {
        format!("{name} {rest}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_is_named_by_its_file_name() {
        assert_eq!(
            shortened("/Applications/Humata.app/Contents/Helpers/humata mcp"),
            "humata mcp"
        );
        assert_eq!(shortened("humata"), "humata");
    }

    #[test]
    fn a_program_outside_the_bundle_is_not_the_app() {
        assert!(!is_app(Path::new("/usr/local/bin/humata")));
    }

    /// A bundle's `Contents` under `root`, with `program` as its app.
    fn bundle(root: &Path, program: &[u8]) -> PathBuf {
        let contents = root.join("Humata.app/Contents");
        std::fs::create_dir_all(contents.join("MacOS")).unwrap_or_else(|error| panic!("{error}"));
        std::fs::write(contents.join("MacOS/Humata"), program)
            .unwrap_or_else(|error| panic!("{error}"));
        contents
    }

    #[test]
    fn the_app_run_from_a_copy_of_its_bundle_elsewhere_is_the_app() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let program = vec![7u8; 200_000];
        let ours = bundle(&dir.path().join("Downloads"), &program);
        let moved = bundle(&dir.path().join("AppTranslocation/1234/d"), &program);
        assert!(is_app_of(&ours, &ours.join("MacOS/Humata")));
        assert!(is_app_of(&ours, &moved.join("MacOS/Humata")));
    }

    #[test]
    fn another_program_in_a_bundle_elsewhere_is_not_the_app() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let ours = bundle(&dir.path().join("Downloads"), &[7u8; 200_000]);
        let mut changed = vec![7u8; 200_000];
        changed[150_000] = 8;
        let other = bundle(&dir.path().join("other"), &changed);
        let shorter = bundle(&dir.path().join("shorter"), &[7u8; 100]);
        assert!(!is_app_of(&ours, &other.join("MacOS/Humata")));
        assert!(!is_app_of(&ours, &shorter.join("MacOS/Humata")));
        // The same bytes, but not run as a bundle's app.
        let loose = dir.path().join("Humata");
        std::fs::write(&loose, [7u8; 200_000]).unwrap_or_else(|error| panic!("{error}"));
        assert!(!is_app_of(&ours, &loose));
    }

    #[tokio::test]
    async fn this_test_on_a_socket_is_another_client() {
        let (ours, _theirs) = UnixStream::pair().unwrap_or_else(|error| panic!("{error}"));
        assert!(matches!(Peer::of(&ours).await, Peer::Client { .. }));
    }
}

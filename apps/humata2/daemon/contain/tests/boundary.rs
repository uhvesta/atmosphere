//! SEC-6 as a boundary suite on macOS: a contained command really is held
//! to its session folder, `shared/`, its temp folder, its agent state and the
//! daemon's socket. Each probe is a plain shell command run under the wrapped
//! argv; its exit status says whether the operating system let it through.
#![cfg(target_os = "macos")]

use std::ffi::OsString;
use std::io::Read;
use std::net::TcpListener;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::Command;

use humata2_contain::Containment;
use tempfile::TempDir;

/// A Humata layout in a temp folder: two sessions of one channel, the
/// channel's shared folder, Humata's own state, a home with SSH keys and a
/// gh config, a folder outside everything, and the daemon's socket.
struct Layout {
    _dir: TempDir,
    root: PathBuf,
    own: PathBuf,
    other: PathBuf,
    shared: PathBuf,
    state: PathBuf,
    ssh: PathBuf,
    gh: PathBuf,
    outside: PathBuf,
    socket: PathBuf,
    _listener: UnixListener,
}

impl Layout {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("a temp folder");
        // Left as the temp folder gives it (on macOS under the /var
        // symlink): the wrap must resolve paths itself.
        let root = dir.path().to_path_buf();
        let channels = root.join("Humata/channels/acme");
        let own = channels.join("fix-login");
        let other = channels.join("rate-limiter");
        let shared = channels.join("shared");
        let state = root.join(".humata2/state");
        let ssh = root.join("home/.ssh");
        let gh = root.join("home/.config/gh");
        let outside = root.join("elsewhere");
        for folder in [&own, &other, &shared, &state, &ssh, &gh, &outside] {
            std::fs::create_dir_all(folder).expect("a folder");
        }
        for (folder, name) in [
            (&own, "notes.md"),
            (&other, "secret.md"),
            (&shared, "plan.md"),
            (&state, "store.db"),
            (&ssh, "id_ed25519"),
            (&gh, "hosts.yml"),
            (&outside, "readme"),
        ] {
            std::fs::write(folder.join(name), "text\n").expect("a file");
        }
        let socket = root.join(".humata2/humatad.sock");
        let listener = UnixListener::bind(&socket).expect("a socket");
        Self {
            _dir: dir,
            own,
            other,
            shared,
            state,
            ssh,
            gh,
            outside,
            socket,
            _listener: listener,
            root,
        }
    }

    /// What a session's daemon would contain this session with.
    fn containment(&self) -> Containment {
        Containment {
            session_folder: self.own.clone(),
            shared: self.shared.clone(),
            also_writable: Vec::new(),
            hidden: vec![
                self.root.join("Humata/channels"),
                self.root.join(".humata2"),
                self.ssh.clone(),
                self.gh.clone(),
            ],
            also_readable: Vec::new(),
            agent_state: Vec::new(),
            read_only: Vec::new(),
            temp: self.own.join(".tmp"),
            socket: self.socket.clone(),
        }
    }
}

/// Runs `script` with `/bin/sh` under `containment`; true when it exits 0.
fn allowed(containment: &Containment, script: &str) -> bool {
    let argv: Vec<OsString> = vec!["/bin/sh".into(), "-c".into(), script.into()];
    let wrapped = containment.wrap(&argv).expect("a wrapped command");
    let (program, args) = wrapped.split_first().expect("a program");
    Command::new(program)
        .args(args)
        .output()
        .expect("the wrapped command starts")
        .status
        .success()
}

fn write(path: &Path) -> String {
    format!("echo changed > '{}'", path.display())
}

fn read(path: &Path) -> String {
    format!("cat '{}' > /dev/null", path.display())
}

#[test]
fn a_contained_agent_writes_in_its_session_folder_and_shared() {
    let layout = Layout::new();
    let contained = layout.containment();

    assert!(allowed(&contained, &write(&layout.own.join("notes.md"))));
    assert!(allowed(&contained, &write(&layout.own.join("new.md"))));
    assert!(allowed(&contained, &write(&layout.shared.join("plan.md"))));
    assert!(allowed(&contained, "echo ok > /dev/null"), "a program needs /dev/null");
    assert_eq!(
        std::fs::read_to_string(layout.shared.join("plan.md")).expect("the file"),
        "changed\n"
    );
}

#[test]
fn a_contained_agent_can_not_write_anywhere_else() {
    let layout = Layout::new();
    let contained = layout.containment();

    for path in [
        layout.other.join("secret.md"),
        layout.outside.join("readme"),
        layout.outside.join("new"),
        layout.state.join("store.db"),
        layout.root.join("home/.profile"),
    ] {
        assert!(!allowed(&contained, &write(&path)), "{} was written", path.display());
    }
    assert_eq!(
        std::fs::read_to_string(layout.other.join("secret.md")).expect("the file"),
        "text\n"
    );
    assert!(!layout.outside.join("new").exists());
}

#[test]
fn reads_of_other_sessions_humata_ssh_and_gh_fail_and_its_own_work() {
    let layout = Layout::new();
    let contained = layout.containment();

    assert!(allowed(&contained, &read(&layout.own.join("notes.md"))));
    assert!(allowed(&contained, &read(&layout.shared.join("plan.md"))));
    assert!(allowed(&contained, &read(&layout.outside.join("readme"))));
    for path in [
        layout.other.join("secret.md"),
        layout.state.join("store.db"),
        layout.ssh.join("id_ed25519"),
        layout.gh.join("hosts.yml"),
    ] {
        assert!(!allowed(&contained, &read(&path)), "{} was read", path.display());
    }
    assert!(
        !allowed(&contained, &format!("ls '{}'", layout.other.display())),
        "another session's folder was listed"
    );
}

#[test]
fn nothing_on_this_machine_but_the_daemons_socket_is_reachable() {
    let layout = Layout::new();
    let contained = layout.containment();
    let tcp = TcpListener::bind("127.0.0.1:0").expect("a TCP listener");
    let port = tcp.local_addr().expect("its address").port();
    let other_socket = layout.outside.join("other.sock");
    let _other = UnixListener::bind(&other_socket).expect("another socket");
    // The daemon reads one message from each connection and hangs up, so an
    // allowed probe ends; `-w 2` makes a wrongly allowed one fail, not hang.
    let daemon = layout._listener.try_clone().expect("the daemon's socket");
    std::thread::spawn(move || {
        for stream in daemon.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut buffer = [0u8; 64];
            let _ = stream.read(&mut buffer);
        }
    });

    assert!(allowed(
        &contained,
        &format!("echo hi | /usr/bin/nc -w 2 -U '{}'", layout.socket.display())
    ));
    assert!(!allowed(&contained, &format!("echo hi | /usr/bin/nc -w 2 127.0.0.1 {port}")));
    assert!(!allowed(
        &contained,
        &format!("echo hi | /usr/bin/nc -w 2 -U '{}'", other_socket.display())
    ));
    tcp.set_nonblocking(true).expect("non-blocking");
    let mut got = String::new();
    if let Ok((mut stream, _)) = tcp.accept() {
        let _ = stream.read_to_string(&mut got);
    }
    assert!(got.is_empty(), "the TCP listener heard {got:?}");
}

#[test]
fn paths_it_is_also_given_are_writable_as_a_context_forks_parent_clones_are() {
    let layout = Layout::new();
    let parent = layout.other.join("api");
    std::fs::create_dir_all(&parent).expect("a folder");
    let contained = Containment {
        also_writable: vec![parent.clone()],
        ..layout.containment()
    };

    assert!(allowed(&contained, &write(&parent.join("main.rs"))));
    assert!(allowed(&contained, &read(&parent.join("main.rs"))));
    assert!(!allowed(&contained, &read(&layout.other.join("secret.md"))));
}

#[test]
fn a_path_with_quotes_in_it_is_contained_as_it_is() {
    let layout = Layout::new();
    let odd = layout.root.join("Humata/channels/acme/say \"hi\" \\ there");
    std::fs::create_dir_all(&odd).expect("a folder");
    let contained = Containment {
        session_folder: odd.clone(),
        ..layout.containment()
    };

    assert!(allowed(&contained, &write(&odd.join("x"))));
    assert!(!allowed(&contained, &write(&layout.own.join("notes.md"))));
}

#[test]
fn a_session_folder_that_does_not_exist_is_an_error_not_a_looser_sandbox() {
    let layout = Layout::new();
    let contained = Containment {
        session_folder: layout.root.join("missing"),
        ..layout.containment()
    };

    let argv: Vec<OsString> = vec!["/bin/sh".into()];
    assert!(contained.wrap(&argv).is_err());
}

#[test]
fn an_agent_state_folder_inside_a_hidden_folder_is_readable_and_writable() {
    let layout = Layout::new();
    let state = layout.root.join(".humata2/agent");
    std::fs::create_dir_all(&state).expect("a folder");
    std::fs::write(state.join("settings.json"), "{}\n").expect("a file");
    let contained = Containment {
        agent_state: vec![state.clone()],
        ..layout.containment()
    };

    assert!(allowed(&contained, &read(&state.join("settings.json"))));
    assert!(allowed(&contained, &write(&state.join("session.json"))));
    assert!(!allowed(&contained, &read(&layout.state.join("store.db"))));
}

#[test]
fn an_agent_state_file_covers_the_temp_file_it_is_replaced_through() {
    let layout = Layout::new();
    let state = layout.root.join("home/.agent.json");
    std::fs::write(&state, "{}\n").expect("a file");
    let contained = Containment {
        agent_state: vec![state.clone()],
        ..layout.containment()
    };
    let temp_file = layout.root.join("home/.agent.json.tmp.123");

    let replace = format!(
        "echo changed > '{}' && mv '{}' '{}'",
        temp_file.display(),
        temp_file.display(),
        state.display()
    );
    assert!(allowed(&contained, &replace));
    assert_eq!(std::fs::read_to_string(&state).expect("the file"), "changed\n");
}

#[test]
fn a_state_path_is_matched_literally_not_as_a_pattern() {
    let layout = Layout::new();
    let state = layout.root.join("home/.agent.json");
    std::fs::write(&state, "{}\n").expect("a file");
    let contained = Containment {
        agent_state: vec![state],
        ..layout.containment()
    };

    // An unescaped dot would match any character, so this name would pass.
    assert!(!allowed(&contained, &write(&layout.root.join("home/.agentXjson"))));
}

#[test]
fn a_read_only_file_in_agent_state_is_readable_but_not_writable() {
    let layout = Layout::new();
    let state = layout.root.join(".humata2/agent");
    let settings = state.join("settings.json");
    std::fs::create_dir_all(&state).expect("a folder");
    std::fs::write(&settings, "{}\n").expect("a file");
    let contained = Containment {
        agent_state: vec![state.clone()],
        read_only: vec![settings.clone()],
        ..layout.containment()
    };

    assert!(allowed(&contained, &read(&settings)));
    assert!(!allowed(&contained, &write(&settings)), "a read-only file was written");
    // Replacing the file through a temp file in the same folder is a write too.
    let temp = state.join("settings.tmp");
    let replace = format!(
        "echo changed > '{}' && mv '{}' '{}'",
        temp.display(),
        temp.display(),
        settings.display()
    );
    assert!(!allowed(&contained, &replace), "a read-only file was replaced");
    assert_eq!(std::fs::read_to_string(&settings).expect("the file"), "{}\n");
    assert!(allowed(&contained, &write(&state.join("session.json"))));
}

#[test]
fn the_contained_command_sees_its_temp_folder_as_tmpdir() {
    let layout = Layout::new();
    let contained = layout.containment();
    let temp = layout.own.join(".tmp");
    assert!(!temp.exists(), "the temp folder existed before the wrap");

    assert!(allowed(
        &contained,
        r#"test -n "$TMPDIR" && echo x > "$TMPDIR/probe""#
    ));
    assert!(temp.is_dir(), "wrap didn't create the temp folder");
    assert!(temp.join("probe").is_file(), "TMPDIR didn't point at the temp folder");
}

#[test]
fn a_hook_file_inside_a_hidden_folder_is_readable_but_not_writable() {
    let layout = Layout::new();
    let hooks = layout.state.join("hooks.json");
    std::fs::write(&hooks, "{}\n").expect("a file");
    let contained = Containment {
        also_readable: vec![hooks.clone()],
        ..layout.containment()
    };

    assert!(allowed(&contained, &read(&hooks)));
    assert!(!allowed(&contained, &write(&hooks)));
    assert!(!allowed(&contained, &read(&layout.state.join("store.db"))));
}

//! A live session on the Lima VM `h2-terminal-c1` survives a holder bump
//! there (LIVEHOLDER): a holder running on the VM is moved in place onto a
//! newer build on the VM, and its program keeps its pid, never sees a
//! hangup, keeps its screen, and still takes typing.
//!
//! Manual and slow tier: it needs the VM and a Linux build of the holder.
//! Set `TERMINAL_LAB_SSH_CONFIG` (the lab's ssh config),
//! `TERMINAL_VM_LINUX_HOLDER` (an aarch64 Linux `humata2_holder` on this
//! Mac) and, if the host alias differs, `TERMINAL_LAB_SSH_HOST`.
//!
//! Everything it starts lives under one unique `/var/tmp/h2-bump-*` folder
//! on the VM, and cleanup only touches processes whose argv names it.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use humata2_holder::{Event, Events, attach, move_to};

const WAIT: Duration = Duration::from_secs(30);
const PREFIX: &str = "h2-bump-";

fn required_env(name: &str, what: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("set {name} to {what} to run this test"))
}

struct Vm {
    config: PathBuf,
    host: String,
    root: String,
    local: tempfile::TempDir,
    forward: Option<Child>,
}

impl Vm {
    fn new() -> Self {
        let trusted = std::fs::canonicalize(required_env(
            "TERMINAL_LAB_SSH_CONFIG",
            "the dedicated Lima ssh config of h2-terminal-c1",
        ))
        .expect("TERMINAL_LAB_SSH_CONFIG must name an existing file");
        let host =
            std::env::var("TERMINAL_LAB_SSH_HOST").unwrap_or_else(|_| "lima-h2-terminal-c1".into());
        let local = tempfile::Builder::new()
            .prefix(PREFIX)
            .tempdir_in("/tmp")
            .expect("a local temp folder");
        let config = local.path().join("ssh_config");
        std::fs::write(&config, format!("Include \"{}\"\n", trusted.display()))
            .expect("write the ssh config");
        let name = local.path().file_name().unwrap().to_string_lossy().into_owned();
        let root = format!("/var/tmp/{name}");
        assert!(root.starts_with(&format!("/var/tmp/{PREFIX}")) && root.len() > 20);
        Self { config, host, root, local, forward: None }
    }

    fn ssh(&self) -> Command {
        let mut command = Command::new("ssh");
        command
            .arg("-F")
            .arg(&self.config)
            .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=10"]);
        command
    }

    /// Runs `script` on the VM with `stdin` as its input; panics if it fails.
    fn run_with(&self, script: &str, stdin: Option<&Path>) -> String {
        let mut command = self.ssh();
        command.arg(&self.host).arg(script).stdout(Stdio::piped()).stderr(Stdio::piped());
        command.stdin(match stdin {
            Some(path) => Stdio::from(std::fs::File::open(path).expect("open the file to send")),
            None => Stdio::null(),
        });
        let out = command.output().expect("run ssh");
        assert!(
            out.status.success(),
            "ssh command failed ({script}): {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    fn run(&self, script: &str) -> String {
        self.run_with(script, None)
    }

    /// Writes `text` to `remote` on the VM.
    fn put_text(&self, remote: &str, text: &str) {
        let staged = self.local.path().join("staged");
        std::fs::write(&staged, text).expect("stage a file");
        self.run_with(&format!("cat > '{remote}'"), Some(&staged));
    }

    fn pid_of(&self, file: &str) -> u32 {
        self.run(&format!("cat '{}/{file}'", self.root))
            .parse()
            .expect("a pid in the file")
    }

    /// True once `check` (a shell test) holds on the VM, polled there.
    fn wait_remote(&self, check: &str) {
        let script = format!(
            "i=0; while [ $i -lt 15000 ]; do if {check}; then exit 0; fi; sleep 0.002; i=$((i+1)); done; exit 1"
        );
        self.run(&script);
    }
}

impl Drop for Vm {
    fn drop(&mut self) {
        if let Some(mut child) = self.forward.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if !self.root.starts_with(&format!("/var/tmp/{PREFIX}")) || self.root.len() <= 20 {
            return;
        }
        // Only processes whose argv names this unique folder, never the
        // cleanup shell itself.
        let _ = self
            .ssh()
            .arg(&self.host)
            .arg(format!(
                "for d in /proc/[0-9]*; do p=${{d#/proc/}}; [ \"$p\" = \"$$\" ] && continue; \
                 c=$(tr '\\000' ' ' <$d/cmdline 2>/dev/null); \
                 case \"$c\" in *'{r}/'*) kill -KILL $p 2>/dev/null;; esac; done; rm -rf '{r}'",
                r = self.root
            ))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .stdin(Stdio::null())
            .status();
    }
}

/// Reads output until `needle` shows or the wait runs out.
async fn wait_for(events: &mut Events, needle: &str) -> bool {
    let mut seen = Vec::new();
    tokio::time::timeout(WAIT, async {
        loop {
            match events.next().await {
                Ok(Some(Event::Output { data, .. })) => {
                    seen.extend(data);
                    if String::from_utf8_lossy(&seen).contains(needle) {
                        return true;
                    }
                }
                Ok(Some(_)) => {}
                _ => return false,
            }
        }
    })
    .await
    .unwrap_or(false)
}

/// Everything the holder kept from offset 0 up to the end it reported on attach.
async fn everything(socket: &Path) -> Vec<u8> {
    let attached = attach(socket, 0).await.expect("attach from 0");
    let mut events = attached.events;
    let mut seen = Vec::new();
    let end = attached.welcome.end;
    let _ = tokio::time::timeout(WAIT, async {
        while let Ok(Some(event)) = events.next().await {
            if let Event::Output { offset, data } = event {
                let reached = offset + data.len() as u64;
                seen.extend(data);
                if reached >= end {
                    break;
                }
            }
        }
    })
    .await;
    seen
}

/// Types `line` and waits for `needle` in the output.
async fn run(socket: &Path, line: &str, needle: &str) -> bool {
    let attached = attach(socket, u64::MAX).await.expect("attach live");
    let mut events = attached.events;
    let mut control = attached.control;
    control.input(1, line.as_bytes()).await.expect("send input");
    wait_for(&mut events, needle).await
}

#[tokio::test]
async fn a_live_ssh_session_survives_a_holder_bump_on_the_vm() {
    let linux_holder = std::fs::canonicalize(required_env(
        "TERMINAL_VM_LINUX_HOLDER",
        "an aarch64 Linux humata2_holder binary",
    ))
    .expect("TERMINAL_VM_LINUX_HOLDER must name an existing file");
    let mut vm = Vm::new();
    let root = vm.root.clone();

    // Two copies of the Linux holder: the running build and the newer one.
    vm.run(&format!("mkdir -p '{root}/old' '{root}/new'"));
    for build in ["old", "new"] {
        vm.run_with(
            &format!("cat > '{root}/{build}/humata2_holder' && chmod 755 '{root}/{build}/humata2_holder'"),
            Some(&linux_holder),
        );
    }

    // The program notes a hangup in hup.log and writes its pid.
    vm.put_text(
        &format!("{root}/prog.sh"),
        &format!(
            "trap 'echo hup >> {root}/hup.log' HUP; echo $$ > {root}/program.pid; exec 2>&1; \
             while IFS= read -r line; do eval \"$line\"; done\n"
        ),
    );
    vm.put_text(
        &format!("{root}/start.sh"),
        &format!(
            "cd '{root}'\n\
             PS1='$ ' ENV=/dev/null nohup '{root}/old/humata2_holder' --socket '{root}/t.sock' \
             --cwd '{root}' -- /bin/sh '{root}/prog.sh' >/dev/null 2>'{root}/holder.err' </dev/null &\n\
             echo $! > '{root}/holder.pid'\n"
        ),
    );
    vm.run(&format!("sh '{root}/start.sh'"));
    vm.wait_remote(&format!("[ -S '{root}/t.sock' ] && [ -s '{root}/program.pid' ]"));

    // Reach the VM's socket from here: each local connection is a new
    // remote one.
    let local_socket = vm.local.path().join("f.sock");
    let forward = vm
        .ssh()
        .args(["-o", "ExitOnForwardFailure=yes", "-o", "StreamLocalBindUnlink=yes", "-N", "-L"])
        .arg(format!("{}:{root}/t.sock", local_socket.display()))
        .arg(&vm.host)
        .stdin(Stdio::null())
        .spawn()
        .expect("start the ssh forward");
    vm.forward = Some(forward);
    let deadline = Instant::now() + WAIT;
    while !local_socket.exists() {
        assert!(Instant::now() < deadline, "the ssh forward never came up");
        std::thread::sleep(Duration::from_millis(2));
    }

    let holder_pid = vm.pid_of("holder.pid");
    let program_pid = vm.pid_of("program.pid");
    let old_exe = vm.run(&format!("readlink /proc/{holder_pid}/exe"));
    assert_eq!(old_exe, format!("{root}/old/humata2_holder"), "not running the old build");

    assert!(run(&local_socket, "echo before-$((6*7))\n", "before-42").await);
    let before = everything(&local_socket).await;
    assert!(String::from_utf8_lossy(&before).contains("before-42"));

    move_to(&local_socket, Path::new(&format!("{root}/new/humata2_holder")))
        .await
        .expect("the move onto the newer build");

    assert_eq!(vm.pid_of("holder.pid"), holder_pid);
    assert_eq!(
        vm.run(&format!("readlink /proc/{holder_pid}/exe")),
        format!("{root}/new/humata2_holder"),
        "the holder still runs the old build"
    );
    assert_eq!(vm.pid_of("program.pid"), program_pid);
    assert_eq!(
        vm.run(&format!("ps -o ppid= -p {program_pid}")),
        holder_pid.to_string(),
        "the program lost its parent or was ended"
    );
    assert_eq!(
        vm.run(&format!("test -e '{root}/hup.log' && echo yes || echo no")),
        "no",
        "the program saw a hangup"
    );

    let after = everything(&local_socket).await;
    assert!(after.starts_with(&before), "the screen did not carry over the move");

    assert!(run(&local_socket, "echo moved-$((6*7))\n", "moved-42").await);

    let attached = attach(&local_socket, u64::MAX).await.expect("attach to exit");
    let mut events = attached.events;
    let mut control = attached.control;
    control.input(2, b"exit 7\n").await.expect("send exit");
    let code = tokio::time::timeout(WAIT, async {
        loop {
            match events.next().await {
                Ok(Some(Event::Exited { code })) => return Some(code),
                Ok(Some(_)) => {}
                _ => return None,
            }
        }
    })
    .await
    .ok()
    .flatten();
    assert_eq!(code, Some(7));
}

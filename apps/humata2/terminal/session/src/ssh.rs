use crate::{Endpoint, Error, SessionSpec};
use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

pub(crate) struct Tunnel {
    child: Child,
    socket: PathBuf,
}
impl Drop for Tunnel {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.socket);
    }
}
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
fn path(p: &Path) -> String {
    quote(&p.to_string_lossy())
}
fn command(config: &Path, destination: &str) -> Command {
    let mut c = Command::new("ssh");
    c.args(["-F"]).arg(config).args([
        "-o",
        "ControlMaster=no",
        "-o",
        "ControlPath=none",
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=8",
        "-o",
        "ServerAliveInterval=5",
        "-o",
        "ServerAliveCountMax=2",
    ]);
    c.arg(destination);
    c
}
pub(crate) fn start(
    endpoint: &Endpoint,
    socket: &str,
    spec: &SessionSpec,
    memory_only: bool,
) -> Result<(), Error> {
    let Endpoint::Ssh {
        config,
        destination,
        holder,
        root,
    } = endpoint
    else {
        unreachable!()
    };
    let remote = format!("{root}/{socket}");
    let mut args = format!(
        "{} --socket {} --cwd {} --cols {} --rows {} --scrollback-bytes {} {} -- {}",
        path(holder),
        quote(&remote),
        path(&spec.cwd),
        spec.cols,
        spec.rows,
        16 << 20,
        if memory_only { "--memory-only" } else { "" },
        path(&spec.program)
    );
    for arg in &spec.args {
        args.push(' ');
        args.push_str(&quote(arg));
    }
    // Keep the holder independent of the SSH connection. Its stdout is never the SSH pipe.
    let shell = format!(
        "umask 077; mkdir -p {} || exit 1; test ! -S {} || exit 1; nohup env TERM=xterm-256color COLORTERM=truecolor {args} </dev/null >{} 2>{} & echo started; exit 0",
        quote(root),
        quote(&remote),
        quote(&format!("{remote}.ready")),
        quote(&format!("{remote}.log"))
    );
    let mut child = command(config, destination)
        .arg(shell)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|source| Error::Ssh {
            destination: destination.clone(),
            source,
        })?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(12);
    loop {
        if let Some(status) = child.try_wait().map_err(|source| Error::Ssh {
            destination: destination.clone(),
            source,
        })? {
            return if status.success() {
                Ok(())
            } else {
                Err(Error::Remote {
                    destination: destination.clone(),
                    detail: format!("remote launch exited with {status}"),
                })
            };
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Remote{destination:destination.clone(),detail:"launch timed out; inspect the remote terminal log before retrying so a running shell is not duplicated".into()});
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}
pub(crate) fn tunnel(endpoint: &Endpoint, name: &str, local: &Path) -> Result<Tunnel, Error> {
    let Endpoint::Ssh {
        config,
        destination,
        root,
        ..
    } = endpoint
    else {
        unreachable!()
    };
    // Forward only this terminal's Unix socket. No TCP listener or remote daemon API.
    let mut c = Command::new("ssh");
    c.arg("-F")
        .arg(config)
        .args([
            "-o",
            "ControlMaster=no",
            "-o",
            "ControlPath=none",
            "-o",
            "BatchMode=yes",
            "-o",
            "ExitOnForwardFailure=yes",
            "-o",
            "ConnectTimeout=8",
            "-o",
            "ServerAliveInterval=5",
            "-o",
            "ServerAliveCountMax=2",
            "-o",
            "StdinNull=no",
            "-o",
            "SessionType=default",
            "-T",
            "-L",
        ])
        .arg(format!("{}:{root}/{name}", local.display()))
        .arg(destination)
        // This command is an attachment lifeline, not the terminal process.
        // Child retains the pipe's writer. Frontend SIGKILL closes it; EOF
        // ends cat and the SSH session even though Rust Drop never runs.
        .arg("exec /bin/cat >/dev/null")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = c.spawn().map_err(|source| Error::Ssh {
        destination: destination.clone(),
        source,
    })?;
    Ok(Tunnel {
        child,
        socket: local.into(),
    })
}

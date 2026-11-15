//! The one way Humata reaches a machine: the provider's `connect` command
//! (MP-7) or SSH with the identity's details (MP-8, MP-10). Every step and
//! the bridge run through it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::error::QuoteError;
use crate::identity::{SshAccess, SshTarget};
use crate::known_hosts;
use crate::name::MachineName;
use crate::quote::{self, sh_command};
use crate::runner::{EnvValue, Invocation, Purpose};
use crate::service;

/// Where humatad2 and its holder live on a machine, under `$HOME`.
pub(crate) const BIN_DIR: &str = ".humata2/bin";

/// Learns a machine's host key: ssh records what the machine presents in a
/// fresh file, which is printed and removed. The `-o` given first wins, so
/// nothing in the user's ssh config can point it elsewhere.
const SCAN_SCRIPT: &str = r#"t=$(mktemp "${TMPDIR:-/tmp}/humata2-hostkey.XXXXXX") || exit 70
trap 'rm -f "$t"' EXIT
ssh -o UserKnownHostsFile="$t" "$@" >/dev/null
s=$?
cat "$t"
exit "$s"
"#;

/// How an ssh command is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Non-interactive, for every step and the bridge.
    Batch,
    /// In a real terminal, for a login (MP-14).
    Interactive,
    /// Up to the host key and no further: no credential is offered.
    Scan,
}

/// How a machine is reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    machine: MachineName,
    how: How,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum How {
    Connect {
        template: String,
        env: Vec<(String, EnvValue)>,
    },
    Ssh {
        target: SshTarget,
        known_hosts: PathBuf,
    },
}

impl Link {
    pub(crate) fn connect(machine: MachineName, template: String, env: Vec<(String, EnvValue)>) -> Self {
        Self {
            machine,
            how: How::Connect { template, env },
        }
    }

    /// SSH to `target`, checking host keys only against Humata's own
    /// `known_hosts` file, never `~/.ssh/known_hosts` (MP-9).
    pub fn ssh(machine: MachineName, target: SshTarget, known_hosts: &Path) -> Self {
        Self {
            machine,
            how: How::Ssh {
                target,
                known_hosts: known_hosts.to_owned(),
            },
        }
    }

    pub fn machine(&self) -> &MachineName {
        &self.machine
    }

    /// Whether this is a provider's `connect` command, which the user can
    /// also open interactively (MP-12).
    pub fn is_connect(&self) -> bool {
        matches!(self.how, How::Connect { .. })
    }

    /// The line to add to Humata's `known_hosts` for a pinned host key
    /// (MP-9), when the identity gave one.
    pub fn known_hosts_entry(&self) -> Option<String> {
        match &self.how {
            How::Ssh { target, .. } => target.host_key.as_ref().map(|key| {
                format!(
                    "{} {} {}",
                    known_hosts::alias(&self.machine),
                    key.algorithm(),
                    key.key()
                )
            }),
            How::Connect { .. } => None,
        }
    }

    /// Humata's `known_hosts` file, for SSH it runs itself; none for a
    /// provider's `connect`, whose tool checks keys on its own.
    pub(crate) fn known_hosts(&self) -> Option<&Path> {
        match &self.how {
            How::Ssh { known_hosts, .. } => Some(known_hosts.as_path()),
            How::Connect { .. } => None,
        }
    }

    /// Connects only far enough to learn the host key the machine
    /// presents, offering no key, password or agent, and prints it in
    /// `known_hosts` form. None for a provider's `connect`.
    pub(crate) fn scan_host_key(&self) -> Option<Invocation> {
        let How::Ssh { target, .. } = &self.how else {
            return None;
        };
        let mut args = vec![
            "-c".to_owned(),
            SCAN_SCRIPT.to_owned(),
            "humata2-scan".to_owned(),
        ];
        args.extend(ssh_args(&self.machine, target, None, Mode::Scan, "true"));
        Some(Invocation {
            program: "/bin/sh".to_owned(),
            args,
            env: Vec::new(),
            stdin: None,
            timeout: Some(Duration::from_secs(60)),
            purpose: Purpose::ScanHostKey,
        })
    }

    /// Runs `script` with `sh` on the machine, non-interactively.
    pub(crate) fn script(
        &self,
        script: &str,
        purpose: Purpose,
        timeout: Option<Duration>,
        stdin: Option<Arc<[u8]>>,
    ) -> Result<Invocation, QuoteError> {
        self.invocation(&sh_command(script)?, false, purpose, timeout, stdin)
    }

    /// The command that carries the gRPC stream to the machine's humatad2
    /// (TR-2, D15). It runs until the daemon stops it.
    pub fn bridge(&self) -> Result<Invocation, QuoteError> {
        self.script(
            &format!("exec \"$HOME/{BIN_DIR}/humatad2\" bridge"),
            Purpose::Bridge,
            None,
            None,
        )
    }

    /// The command that carries the host's peer link: `humatad2 serve
    /// --stdio` on the machine, speaking gRPC on the command's stdio
    /// (MP-7, GRPC.md §8). It runs until the link ends.
    pub fn peer(&self) -> Result<Invocation, QuoteError> {
        self.script(
            &format!("exec \"$HOME/{BIN_DIR}/humatad2\" serve --stdio"),
            Purpose::PeerLink,
            None,
            None,
        )
    }

    /// The command to run in a real terminal for an interactive login
    /// (MP-14): the link with an interactive shell on the far end, ready for
    /// [`crate::Takeover`].
    pub fn interactive(&self) -> Result<Invocation, QuoteError> {
        self.invocation(&sh_command("exec sh -i")?, true, Purpose::Login, None, None)
    }

    /// Stops humatad2 and removes its user service (or crontab entry) and
    /// its files from `~/.humata2/bin`, for cancelling Add machine or
    /// removing a machine. It leaves Humata's folders, state and lingering
    /// alone, and succeeds when there was nothing to remove.
    pub fn remove_daemon(&self) -> Result<Invocation, QuoteError> {
        self.script(
            &service::remove_script(),
            Purpose::Uninstall,
            Some(Duration::from_secs(2 * 60)),
            None,
        )
    }

    /// Adds a public key to the machine's `authorized_keys`, once, so later
    /// connects need no login (MP-17).
    pub fn authorize_key(&self, public_key: &str) -> Result<Invocation, QuoteError> {
        let key = quote::quote("the public key", public_key)?;
        let script = format!(
            "umask 077 && mkdir -p \"$HOME/.ssh\" && touch \"$HOME/.ssh/authorized_keys\" && \
             {{ grep -qxF {key} \"$HOME/.ssh/authorized_keys\" || printf '%s\\n' {key} >> \"$HOME/.ssh/authorized_keys\"; }}"
        );
        self.script(&script, Purpose::AuthorizeKey, Some(Duration::from_secs(60)), None)
    }

    /// `remote` is a command line for the machine's login shell.
    fn invocation(
        &self,
        remote: &str,
        interactive: bool,
        purpose: Purpose,
        timeout: Option<Duration>,
        stdin: Option<Arc<[u8]>>,
    ) -> Result<Invocation, QuoteError> {
        let (program, args, env) = match &self.how {
            How::Connect { template, env } => {
                let cmd = quote::quote("the command for the machine", remote)?;
                (
                    "/bin/sh".to_owned(),
                    vec![
                        "-c".to_owned(),
                        quote::render(template, self.machine.as_str(), Some(&cmd)),
                    ],
                    env.clone(),
                )
            }
            How::Ssh {
                target,
                known_hosts,
            } => (
                "ssh".to_owned(),
                ssh_args(
                    &self.machine,
                    target,
                    Some(known_hosts.as_path()),
                    if interactive { Mode::Interactive } else { Mode::Batch },
                    remote,
                ),
                Vec::new(),
            ),
        };
        Ok(Invocation {
            program,
            args,
            env,
            stdin,
            timeout,
            purpose,
        })
    }
}

/// The arguments to `ssh`. `known_hosts_file` is none only for a scan,
/// whose file the scan script gives first.
fn ssh_args(
    machine: &MachineName,
    target: &SshTarget,
    known_hosts_file: Option<&Path>,
    mode: Mode,
    remote: &str,
) -> Vec<String> {
    let mut args = Vec::new();
    let config = match &target.access {
        SshAccess::Config { ssh_config } => ssh_config.as_str(),
        // Nothing from ~/.ssh/config: the identity says everything.
        SshAccess::Direct { .. } => "/dev/null",
    };
    args.extend(["-F".to_owned(), config.to_owned()]);
    let mut options = vec![
        format!("BatchMode={}", if mode == Mode::Interactive { "no" } else { "yes" }),
        "GlobalKnownHostsFile=/dev/null".to_owned(),
        format!(
            "StrictHostKeyChecking={}",
            if mode == Mode::Scan { "accept-new" } else { "yes" }
        ),
        format!("HostKeyAlias={}", known_hosts::alias(machine)),
        "UpdateHostKeys=no".to_owned(),
        // Hashed names wouldn't match the alias Humata looks for.
        "HashKnownHosts=no".to_owned(),
        // A shared connection would skip the host key check against
        // Humata's file.
        "ControlPath=none".to_owned(),
        // A file streamed during takeover may hold `\n~.`.
        "EscapeChar=none".to_owned(),
        "ConnectTimeout=10".to_owned(),
        "ServerAliveInterval=30".to_owned(),
        "ServerAliveCountMax=4".to_owned(),
    ];
    if let Some(file) = known_hosts_file {
        options.push(format!("UserKnownHostsFile=\"{}\"", file.display()));
    }
    if mode == Mode::Scan {
        // An unverified machine gets no credential and no forwarding.
        options.extend(
            [
                "PubkeyAuthentication=no",
                "PasswordAuthentication=no",
                "KbdInteractiveAuthentication=no",
                "GSSAPIAuthentication=no",
                "HostbasedAuthentication=no",
                "ForwardAgent=no",
                "ClearAllForwardings=yes",
            ]
            .map(str::to_owned),
        );
    }
    for option in options {
        args.push("-o".to_owned());
        args.push(option);
    }
    args.push(if mode == Mode::Interactive { "-tt" } else { "-T" }.to_owned());
    if let SshAccess::Direct {
        port,
        user,
        identity_file,
    } = &target.access
    {
        args.extend(["-p".to_owned(), port.to_string(), "-l".to_owned(), user.clone()]);
        if let (Some(identity_file), false) = (identity_file, mode == Mode::Scan) {
            args.extend([
                "-i".to_owned(),
                identity_file.clone(),
                "-o".to_owned(),
                "IdentitiesOnly=yes".to_owned(),
            ]);
        }
    }
    args.extend(["--".to_owned(), target.host.clone(), remote.to_owned()]);
    args
}

//! The one way Humata reaches a machine: the provider's `connect` command
//! (MP-7) or SSH with the identity's details (MP-8, MP-10). Every step and
//! the bridge run through it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::error::QuoteError;
use crate::identity::{SshAccess, SshTarget};
use crate::name::MachineName;
use crate::quote::{self, sh_command};
use crate::runner::{EnvValue, Invocation, Purpose};

/// Where humatad2 and its holder live on a machine, under `$HOME`.
pub(crate) const BIN_DIR: &str = ".humata2/bin";

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
            How::Ssh { target, .. } => target
                .host_key
                .as_ref()
                .map(|key| format!("{} {} {}", host_key_alias(&self.machine), key.algorithm, key.key)),
            How::Connect { .. } => None,
        }
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

    /// The command to run in a real terminal for an interactive login
    /// (MP-14): the link with an interactive shell on the far end, ready for
    /// [`crate::Takeover`].
    pub fn interactive(&self) -> Result<Invocation, QuoteError> {
        self.invocation(&sh_command("exec sh -i")?, true, Purpose::Login, None, None)
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
                ssh_args(&self.machine, target, known_hosts, interactive, remote),
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

/// The name Humata's `known_hosts` keys a machine's host key under, so the
/// pin holds whatever address or alias reaches it.
fn host_key_alias(machine: &MachineName) -> String {
    format!("humata2.{machine}")
}

fn ssh_args(
    machine: &MachineName,
    target: &SshTarget,
    known_hosts: &Path,
    interactive: bool,
    remote: &str,
) -> Vec<String> {
    let mut args = Vec::new();
    let config = match &target.access {
        SshAccess::Config { ssh_config } => ssh_config.as_str(),
        // Nothing from ~/.ssh/config: the identity says everything.
        SshAccess::Direct { .. } => "/dev/null",
    };
    args.extend(["-F".to_owned(), config.to_owned()]);
    let options = [
        format!("BatchMode={}", if interactive { "no" } else { "yes" }),
        format!("UserKnownHostsFile=\"{}\"", known_hosts.display()),
        "GlobalKnownHostsFile=/dev/null".to_owned(),
        "StrictHostKeyChecking=yes".to_owned(),
        format!("HostKeyAlias={}", host_key_alias(machine)),
        "UpdateHostKeys=no".to_owned(),
        // A file streamed during takeover may hold `\n~.`.
        "EscapeChar=none".to_owned(),
        "ConnectTimeout=10".to_owned(),
        "ServerAliveInterval=15".to_owned(),
        "ServerAliveCountMax=4".to_owned(),
    ];
    for option in options {
        args.push("-o".to_owned());
        args.push(option);
    }
    args.push(if interactive { "-tt" } else { "-T" }.to_owned());
    if let SshAccess::Direct {
        port,
        user,
        identity_file,
    } = &target.access
    {
        args.extend(["-p".to_owned(), port.to_string(), "-l".to_owned(), user.clone()]);
        if let Some(identity_file) = identity_file {
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

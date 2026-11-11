//! The hosts the daemon reaches: the hosts file, and each host's connection
//! over a command's stdin and stdout, in practice `ssh <host> khodrd host
//! --stdio`. A host added by its SSH destination is managed: the daemon
//! installs khodrd there, keeps it at its own build and sends it its
//! variables and setup script on each connection.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use khodr_wire::{Client, pb};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};

use crate::error::{DaemonError, StartError};

/// The hosts file: each host by name.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Hosts {
    #[serde(default)]
    pub hosts: BTreeMap<String, HostConfig>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HostConfig {
    /// For a host reached by a command of its own: the program and its
    /// arguments, for example `["ssh", "build", "khodrd", "host", "--stdio"]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,
    /// For a managed host: how `ssh` reaches it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh: Option<Ssh>,
    /// Variables for its agents, such as `GH_TOKEN`. Sent to the host on
    /// each connection and kept only in its memory there.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Run on the host when it changes, or until it succeeds.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub setup_script: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Ssh {
    /// What `ssh` connects to: a `Host` in the SSH config, or `user@host`.
    pub destination: String,
    /// An SSH config file other than `~/.ssh/config`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<PathBuf>,
    /// The folder on the host for khodrd and its sessions, relative to the
    /// home folder there unless absolute.
    pub folder: String,
}

/// Where a managed host keeps khodrd unless told otherwise.
pub const DEFAULT_FOLDER: &str = ".khodr";

impl Hosts {
    /// The hosts in `path`; none if there is no such file.
    pub fn load(path: &Path) -> Result<Self, StartError> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|source| StartError::BadHosts {
                path: path.to_owned(),
                source,
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(StartError::ReadHosts {
                path: path.to_owned(),
                source,
            }),
        }
    }

    /// Writes the hosts to `path`, readable only by you since it holds the
    /// hosts' variables, replacing it whole so a crash leaves the old one.
    pub fn save(&self, path: &Path) -> Result<(), DaemonError> {
        use std::os::unix::fs::OpenOptionsExt as _;
        let failed = |source| DaemonError::SaveHosts {
            path: path.to_owned(),
            source,
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(failed)?;
        }
        let json = serde_json::to_vec_pretty(self).map_err(|error| failed(error.into()))?;
        let temporary = path.with_extension("json.new");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(failed)?;
        file.write_all(&json)
            .and_then(|()| file.sync_all())
            .map_err(failed)?;
        std::fs::rename(&temporary, path).map_err(failed)
    }
}

impl HostConfig {
    pub fn info(&self, name: &str) -> pb::HostInfo {
        pb::HostInfo {
            name: name.to_owned(),
            ssh_destination: self
                .ssh
                .as_ref()
                .map(|ssh| ssh.destination.clone())
                .unwrap_or_default(),
            env_names: self.env.keys().cloned().collect(),
            setup_script: self.setup_script.clone(),
        }
    }

    fn configure(&self) -> pb::ConfigureHost {
        pb::ConfigureHost {
            env: self
                .env
                .iter()
                .map(|(name, value)| pb::EnvVar {
                    name: name.clone(),
                    value: value.clone(),
                })
                .collect(),
            setup_script: self.setup_script.clone(),
        }
    }

    /// The command that reaches the host's `khodrd host --stdio`.
    fn command(&self) -> Vec<String> {
        let Some(ssh) = &self.ssh else {
            return self.command.clone();
        };
        let folder = &ssh.folder;
        let remote = format!(
            "{} host --stdio --state {} --cache {}",
            quoted(&format!("{folder}/bin/khodrd")),
            quoted(&format!("{folder}/state")),
            quoted(&format!("{folder}/cache")),
        );
        let mut command = ssh.base();
        command.push(remote);
        command
    }
}

impl Ssh {
    /// `ssh`, its options and the destination, for a command to follow.
    fn base(&self) -> Vec<String> {
        let mut command = vec!["ssh".to_owned()];
        if let Some(config) = &self.config {
            command.push("-F".to_owned());
            command.push(config.display().to_string());
        }
        command.extend(
            [
                // Never stops to ask for a password the daemon can't type.
                "-o",
                "BatchMode=yes",
                // Notices a dead connection, so its turns are closed.
                "-o",
                "ServerAliveInterval=15",
                "-o",
                "ServerAliveCountMax=3",
                "--",
                &self.destination,
            ]
            .map(str::to_owned),
        );
        command
    }
}

/// Quotes `text` for the remote shell, which SSH runs the command with.
fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// What the command printed on stderr, kept for the error if it fails.
const STDERR_KEPT: usize = 4096;

/// How long a command over SSH to set up a host may take.
const SSH_TIMEOUT: Duration = Duration::from_secs(120);

/// A host reached through a command, connected on first use and again after
/// the connection drops.
pub struct Remote {
    name: String,
    config: Mutex<HostConfig>,
    connection: tokio::sync::Mutex<Option<(Arc<Client>, Child)>>,
}

impl Remote {
    pub fn new(name: String, config: HostConfig) -> Self {
        Self {
            name,
            config: Mutex::new(config),
            connection: tokio::sync::Mutex::new(None),
        }
    }

    pub fn config(&self) -> HostConfig {
        self.config
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Takes `config` for the next connection, and sends the host its
    /// variables and setup script now if it is connected.
    pub async fn reconfigure(&self, config: HostConfig) -> Result<(), DaemonError> {
        let configure = config.configure();
        *self.config.lock().unwrap_or_else(PoisonError::into_inner) = config;
        let connection = self.connection.lock().await;
        if let Some((client, _)) = connection.as_ref()
            && client.is_connected()
        {
            self.send_configure(client, configure).await?;
        }
        Ok(())
    }

    async fn send_configure(
        &self,
        client: &Client,
        configure: pb::ConfigureHost,
    ) -> Result<(), DaemonError> {
        client
            .call(pb::request::Kind::ConfigureHost(configure))
            .await
            .map(drop)
            .map_err(|source| DaemonError::Host {
                host: self.name.clone(),
                source,
            })
    }

    /// The connection to the host, made if there is none. A managed host
    /// running another build of khodrd than this one is updated first, if it
    /// is the same platform as this Mac.
    pub async fn client(&self, hosts: &Path, build: &str) -> Result<Arc<Client>, DaemonError> {
        let mut connection = self.connection.lock().await;
        if let Some((client, _)) = connection.as_ref()
            && client.is_connected()
        {
            return Ok(client.clone());
        }
        *connection = None;
        let config = self.config();
        let (mut client, mut child) = self.connect(&config, hosts, build).await?;
        if let Some(ssh) = &config.ssh
            && client.peer().program_version != build
            && self.platform(ssh).await.ok() == local_platform().await
        {
            drop(client);
            let _ = child.kill().await;
            self.install(ssh, None).await?;
            (client, child) = self.connect(&config, hosts, build).await?;
        }
        let client = Arc::new(client);
        self.send_configure(&client, config.configure()).await?;
        *connection = Some((client.clone(), child));
        Ok(client)
    }

    /// Drops the connection, so the next request makes a new one.
    pub async fn disconnect(&self) {
        if let Some((_, mut child)) = self.connection.lock().await.take() {
            let _ = child.kill().await;
        }
    }

    async fn connect(
        &self,
        config: &HostConfig,
        hosts: &Path,
        build: &str,
    ) -> Result<(Client, Child), DaemonError> {
        let command = config.command();
        let Some((program, arguments)) = command.split_first() else {
            return Err(DaemonError::NoCommand {
                host: self.name.clone(),
                hosts: hosts.to_owned(),
            });
        };
        let mut child = Command::new(program)
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|source| DaemonError::Spawn {
                host: self.name.clone(),
                program: program.clone(),
                source,
            })?;
        let (Some(stdin), Some(stdout), Some(mut stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            let source = std::io::Error::other("its stdin, stdout and stderr weren't connected");
            return Err(DaemonError::Spawn {
                host: self.name.clone(),
                program: program.clone(),
                source,
            });
        };
        let said = Arc::new(Mutex::new(String::new()));
        tokio::spawn({
            let said = said.clone();
            async move {
                let mut buffer = [0_u8; 1024];
                while let Ok(read) = stderr.read(&mut buffer).await
                    && read > 0
                {
                    let mut said = said.lock().unwrap_or_else(PoisonError::into_inner);
                    if said.len() < STDERR_KEPT {
                        said.push_str(&String::from_utf8_lossy(&buffer[..read]));
                    }
                }
            }
        });
        match Client::connect(stdout, stdin, pb::Role::Daemon, build).await {
            Ok(client) => Ok((client, child)),
            Err(source) => {
                // Lets the command finish printing why it stopped.
                let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
                let stderr = said.lock().unwrap_or_else(PoisonError::into_inner).clone();
                Err(DaemonError::Connect {
                    host: self.name.clone(),
                    managed: config.ssh.is_some(),
                    stderr,
                    source,
                })
            }
        }
    }

    /// Copies khodrd to the host: `binary`, or this one if the host is the
    /// same platform as this Mac. Takes effect on the next connection.
    pub async fn install(&self, ssh: &Ssh, binary: Option<PathBuf>) -> Result<(), DaemonError> {
        let binary = match binary {
            Some(binary) => binary,
            None => {
                let theirs = self.platform(ssh).await?;
                let ours = local_platform().await.unwrap_or_default();
                if theirs != ours {
                    return Err(DaemonError::OtherPlatform {
                        host: self.name.clone(),
                        theirs,
                        ours,
                    });
                }
                std::env::current_exe().map_err(|source| DaemonError::ReadBinary {
                    path: PathBuf::from("khodrd"),
                    source,
                })?
            }
        };
        let bytes = tokio::fs::read(&binary)
            .await
            .map_err(|source| DaemonError::ReadBinary {
                path: binary.clone(),
                source,
            })?;
        let folder = quoted(&format!("{}/bin", ssh.folder));
        let script = format!(
            "mkdir -p {folder} && cat > {folder}/khodrd.new && chmod 755 {folder}/khodrd.new && mv {folder}/khodrd.new {folder}/khodrd"
        );
        self.ssh(ssh, &script, Some(bytes)).await.map(drop)
    }

    /// `uname -sm` on the host.
    async fn platform(&self, ssh: &Ssh) -> Result<String, DaemonError> {
        Ok(self.ssh(ssh, "uname -sm", None).await?.trim().to_owned())
    }

    /// Runs `script` on the host over SSH, with `input` on its stdin, and
    /// returns what it printed.
    async fn ssh(
        &self,
        ssh: &Ssh,
        script: &str,
        input: Option<Vec<u8>>,
    ) -> Result<String, DaemonError> {
        let mut command = ssh.base();
        command.push(script.to_owned());
        let failed = |output: String| DaemonError::Ssh {
            host: self.name.clone(),
            script: script.to_owned(),
            output,
        };
        let mut child = Command::new(&command[0])
            .args(&command[1..])
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|source| DaemonError::Spawn {
                host: self.name.clone(),
                program: "ssh".to_owned(),
                source,
            })?;
        if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
            stdin
                .write_all(&input)
                .await
                .map_err(|error| failed(format!("the copy stopped: {error}")))?;
        }
        let output = tokio::time::timeout(SSH_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| failed(format!("no answer in {} seconds", SSH_TIMEOUT.as_secs())))?
            .map_err(|error| failed(error.to_string()))?;
        if !output.status.success() {
            return Err(failed(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Answers `request` over the host's connection.
    pub async fn call(
        &self,
        client: &Client,
        request: pb::request::Kind,
    ) -> Result<pb::response::Kind, DaemonError> {
        client
            .call(request)
            .await
            .map_err(|source| DaemonError::Host {
                host: self.name.clone(),
                source,
            })
    }
}

/// `uname -sm` on this Mac.
async fn local_platform() -> Option<String> {
    let output = Command::new("uname").arg("-sm").output().await.ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// The hosts named in an SSH config: each `Host` that isn't a pattern.
pub fn ssh_hosts(config: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in config.lines() {
        let line = line.trim();
        let mut words = line
            .split(|c: char| c.is_whitespace() || c == '=')
            .filter(|word| !word.is_empty());
        if !words
            .next()
            .is_some_and(|keyword| keyword.eq_ignore_ascii_case("host"))
        {
            continue;
        }
        for name in words {
            if name.starts_with('#') {
                break;
            }
            if !name.contains(['*', '?', '!']) && !names.iter().any(|known| known == name) {
                names.push(name.to_owned());
            }
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ssh_config_names_its_hosts_but_not_its_patterns() {
        let config = "\
Host *
  ServerAliveInterval 30
Host build gpu # the big ones
  User me
host=laptop
Match host build
Host !bad *.internal build
";
        assert_eq!(ssh_hosts(config), ["build", "gpu", "laptop"]);
    }

    #[test]
    fn a_managed_host_is_reached_over_ssh_at_its_folder() {
        let config = HostConfig {
            ssh: Some(Ssh {
                destination: "build".to_owned(),
                config: None,
                folder: "it's here".to_owned(),
            }),
            ..HostConfig::default()
        };
        let command = config.command();
        assert_eq!(command[0], "ssh");
        assert_eq!(command[command.len() - 2], "build");
        let remote = &command[command.len() - 1];
        assert!(
            remote.starts_with(r"'it'\''s here/bin/khodrd' host --stdio"),
            "{remote}"
        );
    }

    #[test]
    fn the_hosts_file_keeps_what_was_saved_and_only_you_can_read_it() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let path = dir.path().join("hosts.json");
        let mut hosts = Hosts::default();
        hosts.hosts.insert(
            "build".to_owned(),
            HostConfig {
                ssh: Some(Ssh {
                    destination: "me@build".to_owned(),
                    config: None,
                    folder: DEFAULT_FOLDER.to_owned(),
                }),
                env: BTreeMap::from([("GH_TOKEN".to_owned(), "secret".to_owned())]),
                ..HostConfig::default()
            },
        );

        hosts.save(&path).unwrap_or_else(|error| panic!("{error}"));
        let loaded = Hosts::load(&path).unwrap_or_else(|error| panic!("{error}"));
        let mode = std::fs::metadata(&path)
            .map(|metadata| metadata.permissions().mode() & 0o777)
            .unwrap_or_default();

        assert_eq!(loaded.hosts, hosts.hosts);
        assert_eq!(mode, 0o600);
    }
}

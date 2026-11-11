//! An SSH server on loopback for end-to-end tests: `test_sshd <folder>`.
//!
//! It makes a host key and a client key, then writes into the folder what
//! OpenSSH's `ssh` needs to reach it as the host `humata-test` and nothing
//! else: `ssh_config`, `known_hosts` and `id_ed25519`. `ssh_config` is
//! written last, so once it exists the server is listening. It accepts only
//! that client key, and runs each command it is asked to with `/bin/sh -c`,
//! as the user running this. It stops when killed.

use std::collections::HashMap;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use russh::keys::{Algorithm, PrivateKey, PublicKey, ssh_key::LineEnding};
use russh::server::{Auth, Msg, Server as _, Session, VerifiedPublicKey};
use russh::{Channel, ChannelId, ChannelMsg};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::process::Command;

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("usage: test_sshd <folder>; it writes ssh_config there for `ssh -F`")]
    Usage,

    #[error("couldn't make an SSH key; the system's random source failed")]
    Key {
        #[source]
        source: russh::keys::ssh_key::Error,
    },

    #[error("couldn't write {path}; check the folder exists and is writable")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("couldn't listen on loopback; check that the test may use the network")]
    Listen {
        #[source]
        source: std::io::Error,
    },

    #[error("the SSH server stopped")]
    Serve {
        #[source]
        source: std::io::Error,
    },
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        let mut message = error.to_string();
        let mut source = std::error::Error::source(&error);
        while let Some(cause) = source {
            message.push_str(": ");
            message.push_str(&cause.to_string());
            source = cause.source();
        }
        eprintln!("test_sshd: {message}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Error> {
    let mut arguments = std::env::args_os().skip(1);
    let (Some(folder), None) = (arguments.next(), arguments.next()) else {
        return Err(Error::Usage);
    };
    let folder = PathBuf::from(folder);

    let mut rng = rand::rng();
    let host_key =
        PrivateKey::random(&mut rng, Algorithm::Ed25519).map_err(|source| Error::Key { source })?;
    let client_key =
        PrivateKey::random(&mut rng, Algorithm::Ed25519).map_err(|source| Error::Key { source })?;

    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|source| Error::Listen { source })?;
    let port = listener
        .local_addr()
        .map_err(|source| Error::Listen { source })?
        .port();

    let identity = folder.join("id_ed25519");
    let client_openssh = client_key
        .to_openssh(LineEnding::LF)
        .map_err(|source| Error::Key { source })?;
    write(&identity, client_openssh.as_bytes(), 0o600)?;
    let host_public = host_key
        .public_key()
        .to_openssh()
        .map_err(|source| Error::Key { source })?;
    let known_hosts = folder.join("known_hosts");
    write(
        &known_hosts,
        format!("[127.0.0.1]:{port} {host_public}\n").as_bytes(),
        0o600,
    )?;
    let config = format!(
        "Host humata-test\n\
         \tHostName 127.0.0.1\n\
         \tPort {port}\n\
         \tUser humata\n\
         \tIdentityFile {identity}\n\
         \tIdentitiesOnly yes\n\
         \tIdentityAgent none\n\
         \tUserKnownHostsFile {known_hosts}\n\
         \tStrictHostKeyChecking yes\n\
         \tBatchMode yes\n\
         \tLogLevel ERROR\n",
        identity = identity.display(),
        known_hosts = known_hosts.display(),
    );
    // Written under another name and renamed, so a test that sees it sees
    // all of it.
    let partial = folder.join("ssh_config.partial");
    write(&partial, config.as_bytes(), 0o600)?;
    let config_path = folder.join("ssh_config");
    std::fs::rename(&partial, &config_path).map_err(|source| Error::Write {
        path: config_path,
        source,
    })?;

    let config = russh::server::Config {
        keys: vec![host_key],
        auth_rejection_time: std::time::Duration::from_millis(10),
        auth_rejection_time_initial: Some(std::time::Duration::ZERO),
        ..Default::default()
    };
    let mut server = Server {
        client: client_key.public_key().clone(),
    };
    server
        .run_on_socket(Arc::new(config), &listener)
        .await
        .map_err(|source| Error::Serve { source })
}

fn write(path: &Path, contents: &[u8], mode: u32) -> Result<(), Error> {
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(mode)
        .open(path)
        .and_then(|mut file| std::io::Write::write_all(&mut file, contents));
    written.map_err(|source| Error::Write {
        path: path.to_owned(),
        source,
    })
}

struct Server {
    client: PublicKey,
}

impl russh::server::Server for Server {
    type Handler = Connection;

    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Connection {
        Connection {
            client: self.client.clone(),
            channels: HashMap::new(),
        }
    }
}

struct Connection {
    client: PublicKey,
    /// Session channels opened and not yet running a command.
    channels: HashMap<ChannelId, Channel<Msg>>,
}

impl russh::server::Handler for Connection {
    type Error = russh::Error;

    async fn auth_publickey(
        &mut self,
        _user: &str,
        key: VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
        Ok(if key.public_key.key_data() == self.client.key_data() {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: russh::server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let Some(opened) = self.channels.remove(&channel) else {
            session.channel_failure(channel)?;
            return Ok(());
        };
        session.channel_success(channel)?;
        let command = String::from_utf8_lossy(data).into_owned();
        tokio::spawn(execute(opened, command));
        Ok(())
    }
}

/// Runs `command` with the channel as its stdin, stdout and stderr, then
/// sends its exit status and closes the channel.
async fn execute(channel: Channel<Msg>, command: String) {
    let (mut reader, writer) = channel.split();
    let child = Command::new("/bin/sh")
        .arg("-c")
        .arg(&command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(error) => {
            let message = format!("test_sshd: couldn't run {command}: {error}\n");
            let _ = writer.extended_data_bytes(1, message.into_bytes()).await;
            let _ = writer.exit_status(127).await;
            let _ = writer.eof().await;
            let _ = writer.close().await;
            return;
        }
    };

    let mut stdin = child.stdin.take();
    let input = tokio::spawn(async move {
        while let Some(message) = reader.wait().await {
            match message {
                ChannelMsg::Data { data } => {
                    let Some(pipe) = stdin.as_mut() else { continue };
                    if pipe.write_all(&data).await.is_err() {
                        stdin = None;
                    }
                }
                // Dropping stdin closes it, which the command reads as EOF.
                ChannelMsg::Eof | ChannelMsg::Close => stdin = None,
                _ => {}
            }
        }
    });

    let mut stdout_writer = writer.make_writer();
    let mut stderr_writer = writer.make_writer_ext(Some(1));
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let copied = async {
        if let Some(mut stdout) = stdout {
            let _ = tokio::io::copy(&mut stdout, &mut stdout_writer).await;
            let _ = stdout_writer.flush().await;
        }
    };
    let errors = async {
        if let Some(mut stderr) = stderr {
            let _ = tokio::io::copy(&mut stderr, &mut stderr_writer).await;
            let _ = stderr_writer.flush().await;
        }
    };
    tokio::join!(copied, errors);
    let status = child
        .wait()
        .await
        .ok()
        .and_then(|status| status.code())
        .and_then(|code| u32::try_from(code).ok())
        .unwrap_or(255);
    let _ = writer.exit_status(status).await;
    let _ = writer.eof().await;
    let _ = writer.close().await;
    input.abort();
}

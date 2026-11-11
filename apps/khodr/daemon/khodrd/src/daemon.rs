//! The daemon role: one place the app and the CLI talk to, which runs
//! sessions on this Mac or passes them to a host it reaches over a command's
//! stdin and stdout (in practice `ssh <host> khodrd host --stdio`).

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex, PoisonError};

use khodr_wire::{
    Client, ClientError, Reply, Service, ServiceFuture, describe, error, pb, streams,
};
use serde::Deserialize;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use crate::error::{DaemonError, StartError};
use crate::host::Host;
use crate::store::Store;
use crate::{VERSION, convert};

/// The hosts file: each host by name, and the command that reaches its
/// `khodrd host --stdio`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hosts {
    #[serde(default)]
    pub hosts: BTreeMap<String, HostConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostConfig {
    /// The program and its arguments, for example
    /// `["ssh", "build", "khodrd", "host", "--stdio"]`.
    pub command: Vec<String>,
}

impl Hosts {
    /// The hosts in `path`; none if there is no such file.
    pub fn load(path: &std::path::Path) -> Result<Self, StartError> {
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
}

/// A host reached through a command, connected on first use and again after
/// the connection drops.
struct Remote {
    name: String,
    command: Vec<String>,
    connection: tokio::sync::Mutex<Option<(Arc<Client>, Child)>>,
}

/// What the command printed on stderr, kept for the error if it fails.
const STDERR_KEPT: usize = 4096;

impl Remote {
    async fn client(&self, hosts: &std::path::Path) -> Result<Arc<Client>, DaemonError> {
        let mut connection = self.connection.lock().await;
        if let Some((client, _)) = connection.as_ref()
            && client.is_connected()
        {
            return Ok(client.clone());
        }
        *connection = None;
        let Some((program, arguments)) = self.command.split_first() else {
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
        match Client::connect(stdout, stdin, pb::Role::Daemon, VERSION).await {
            Ok(client) => {
                let client = Arc::new(client);
                *connection = Some((client.clone(), child));
                Ok(client)
            }
            Err(source) => {
                // Lets the command finish printing why it stopped.
                let _ = tokio::time::timeout(std::time::Duration::from_secs(2), child.wait()).await;
                let stderr = said.lock().unwrap_or_else(PoisonError::into_inner).clone();
                Err(DaemonError::Connect {
                    host: self.name.clone(),
                    stderr,
                    source,
                })
            }
        }
    }
}

/// Answers clients: sessions on this Mac go to the local host, others to the
/// host they run on. Projects, and where each session is, are the daemon's.
pub struct Daemon {
    local: Arc<Host>,
    remotes: HashMap<String, Remote>,
    hosts_file: PathBuf,
    store: Store,
}

impl Daemon {
    pub fn new(local: Host, store: Store, hosts: Hosts, hosts_file: PathBuf) -> Self {
        let remotes = hosts
            .hosts
            .into_iter()
            .map(|(name, config)| {
                let remote = Remote {
                    name: name.clone(),
                    command: config.command,
                    connection: tokio::sync::Mutex::new(None),
                };
                (name, remote)
            })
            .collect();
        Self {
            local: Arc::new(local),
            remotes,
            hosts_file,
            store,
        }
    }

    fn remote(&self, host: &str) -> Result<&Remote, DaemonError> {
        self.remotes
            .get(host)
            .ok_or_else(|| DaemonError::UnknownHost {
                host: host.to_owned(),
                hosts: self.hosts_file.clone(),
            })
    }

    /// Sends `request` to `host` ("" for this Mac).
    async fn forward(&self, host: &str, request: pb::request::Kind) -> Result<Reply, DaemonError> {
        if host.is_empty() {
            return Ok(self.local.clone().handle(request).await);
        }
        let client = self.remote(host)?.client(&self.hosts_file).await?;
        let failed = |source: ClientError| match source {
            ClientError::Refused { code, message } => Ok(Reply::Response(error(code, message))),
            source => Err(DaemonError::Host {
                host: host.to_owned(),
                source,
            }),
        };
        if !streams(&request) {
            return client
                .call(request)
                .await
                .map(Reply::Response)
                .or_else(failed);
        }
        let (response, mut stream) = match client.stream(request).await {
            Ok(answer) => answer,
            Err(source) => return failed(source),
        };
        let (items, receiver) = mpsc::channel(64);
        tokio::spawn(async move {
            loop {
                let next = tokio::select! {
                    next = stream.next() => next,
                    () = items.closed() => return,
                };
                let Some(item) = next else { return };
                if items.send(item).await.is_err() {
                    return;
                }
            }
        });
        Ok(Reply::Stream {
            response,
            items: receiver,
        })
    }

    /// The host `session` runs on, asking every host if it isn't known yet.
    async fn host_of(&self, session: &str) -> Result<String, DaemonError> {
        if let Some(placement) = self.store.placement(session)? {
            return Ok(placement.host);
        }
        self.list("").await?;
        Ok(self
            .store
            .placement(session)?
            .map(|placement| placement.host)
            .unwrap_or_default())
    }

    /// Records that `session` runs on `host`, in `project` if given, and
    /// fills in what the daemon knows of it.
    fn own(
        &self,
        session: &mut pb::SessionInfo,
        host: &str,
        project: Option<&str>,
    ) -> Result<(), DaemonError> {
        self.store.place(&session.id, host, project)?;
        session.host = host.to_owned();
        session.project_id = self
            .store
            .placement(&session.id)?
            .and_then(|placement| placement.project)
            .unwrap_or_default();
        Ok(())
    }

    /// Every host's sessions, or only `project`'s if it isn't empty. A host
    /// that can't be reached is left out, so one machine being off doesn't
    /// hide the rest.
    async fn list(&self, project: &str) -> Result<pb::Sessions, DaemonError> {
        let mut all = Vec::new();
        let hosts = std::iter::once("").chain(self.remotes.keys().map(String::as_str));
        for host in hosts {
            let request = pb::request::Kind::ListSessions(pb::ListSessions::default());
            if let Ok(Reply::Response(pb::response::Kind::Sessions(sessions))) =
                self.forward(host, request).await
            {
                for mut session in sessions.sessions {
                    self.own(&mut session, host, None)?;
                    if project.is_empty() || session.project_id == project {
                        all.push(session);
                    }
                }
            }
        }
        Ok(pb::Sessions { sessions: all })
    }

    fn create_project(&self, create: pb::CreateProject) -> Result<pb::Project, DaemonError> {
        let repository = PathBuf::from(create.repository);
        let not_a_folder = |source| DaemonError::NotAFolder {
            repository: repository.clone(),
            source,
        };
        if !repository.is_absolute() {
            return Err(not_a_folder(None));
        }
        let metadata =
            std::fs::metadata(&repository).map_err(|source| not_a_folder(Some(source)))?;
        if !metadata.is_dir() {
            return Err(not_a_folder(None));
        }
        let project = self
            .store
            .add_project(create.name, repository, create.instructions)?;
        Ok(convert::project(project))
    }

    /// Fills in what `start` leaves to its project, and returns the project.
    fn from_project(&self, start: &mut pb::StartSession) -> Result<Option<String>, DaemonError> {
        let project_id = std::mem::take(&mut start.project_id);
        if project_id.is_empty() {
            return Ok(None);
        }
        let project = self.store.project(&project_id)?;
        if start.working_directory.is_empty() {
            if !start.host.is_empty() {
                return Err(DaemonError::NoRemoteDirectory {
                    project: project.name,
                    host: start.host.clone(),
                });
            }
            start.working_directory = project.repository.display().to_string();
        }
        if start.instructions.is_empty() {
            start.instructions = project.instructions;
        }
        Ok(Some(project_id))
    }

    async fn answer(&self, request: pb::request::Kind) -> Result<Reply, DaemonError> {
        use pb::request::Kind as Request;
        use pb::response::Kind as Response;
        let mut request = request;
        let mut project = None;
        let host = match &mut request {
            Request::ListSessions(list) => {
                let sessions = self.list(&list.project_id).await?;
                return Ok(Reply::Response(Response::Sessions(sessions)));
            }
            Request::CreateProject(create) => {
                let project = self.create_project(std::mem::take(create))?;
                return Ok(Reply::Response(Response::ProjectCreated(
                    pb::ProjectCreated {
                        project: Some(project),
                    },
                )));
            }
            Request::ListProjects(_) => {
                let projects = self
                    .store
                    .projects()?
                    .into_iter()
                    .map(convert::project)
                    .collect();
                return Ok(Reply::Response(Response::Projects(pb::Projects {
                    projects,
                })));
            }
            Request::StartSession(start) => {
                project = self.from_project(start)?;
                // The host runs it on itself; only the daemon knows hosts by
                // name.
                std::mem::take(&mut start.host)
            }
            Request::ForkSession(fork) => {
                // A fork stays in its parent's project.
                let host = self.host_of(&fork.session_id).await?;
                project = self
                    .store
                    .placement(&fork.session_id)?
                    .and_then(|placement| placement.project);
                host
            }
            Request::SendMessage(message) => self.host_of(&message.session_id).await?,
            Request::Approve(approve) => self.host_of(&approve.session_id).await?,
            Request::ReadHistory(read) => self.host_of(&read.session_id).await?,
            Request::WatchSession(watch) => self.host_of(&watch.session_id).await?,
            Request::AbortTurn(abort) => self.host_of(&abort.session_id).await?,
            Request::SetApprovalMode(set) => self.host_of(&set.session_id).await?,
        };
        let mut reply = self.forward(&host, request).await?;
        if let Reply::Response(
            Response::SessionStarted(pb::SessionStarted {
                session: Some(session),
            })
            | Response::SessionForked(pb::SessionForked {
                session: Some(session),
            }),
        ) = &mut reply
        {
            self.own(session, &host, project.as_deref())?;
        }
        Ok(reply)
    }
}

impl Service for Daemon {
    fn handle(self: Arc<Self>, request: pb::request::Kind) -> ServiceFuture {
        Box::pin(async move {
            match self.answer(request).await {
                Ok(reply) => reply,
                Err(failure) => Reply::Response(error(failure.code(), describe(&failure))),
            }
        })
    }
}

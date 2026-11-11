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

use crate::board::{self, Item, ItemFields, ItemPatch, Priority, Status};
use crate::error::StoreError;
use crate::error::{DaemonError, StartError};
use crate::host::Host;
use crate::store::Store;
use crate::{VERSION, convert};
use khodr_agent::Repository;

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

    /// Records that `session` runs on `host`, in `project` and on `item` if
    /// given, and fills in what the daemon knows of it.
    fn own(
        &self,
        session: &mut pb::SessionInfo,
        host: &str,
        project: Option<&str>,
        item: Option<&str>,
    ) -> Result<(), DaemonError> {
        self.store.place(&session.id, host, project, item)?;
        session.host = host.to_owned();
        let placement = self.store.placement(&session.id)?;
        let (project, item) = placement
            .map(|placement| (placement.project, placement.item))
            .unwrap_or_default();
        session.project_id = project.unwrap_or_default();
        session.item_id = item.unwrap_or_default();
        Ok(())
    }

    /// `project`'s items, each with the status its sessions give it.
    async fn shown_items(&self, project: &str) -> Result<Vec<pb::Item>, DaemonError> {
        let mut items: Vec<pb::Item> = self.store.items(project)?.iter().map(Item::wire).collect();
        let sessions = self.list(project).await?;
        board::show(&mut items, &sessions.sessions);
        Ok(items)
    }

    /// One item, with the status its sessions give it.
    async fn shown_item(&self, item: &Item) -> Result<pb::Item, DaemonError> {
        let items = self.shown_items(&item.project_id).await?;
        Ok(items
            .into_iter()
            .find(|shown| shown.id == item.id)
            .unwrap_or_else(|| item.wire()))
    }

    /// Every session waiting on you, across projects.
    async fn inbox(&self) -> Result<pb::Inbox, DaemonError> {
        let projects = self.store.projects()?;
        let mut items: HashMap<String, pb::Item> = HashMap::new();
        for project in &projects {
            for item in self.shown_items(&project.id).await? {
                items.insert(item.id.clone(), item);
            }
        }
        let names: HashMap<&str, &str> = projects
            .iter()
            .map(|project| (project.id.as_str(), project.name.as_str()))
            .collect();
        let mut entries: Vec<pb::InboxEntry> = self
            .list("")
            .await?
            .sessions
            .into_iter()
            .filter_map(|session| {
                let item = items.get(&session.item_id);
                let reason = board::inbox_reason(&session, item)?;
                Some(pb::InboxEntry {
                    project_name: names
                        .get(session.project_id.as_str())
                        .map(|name| (*name).to_owned())
                        .unwrap_or_default(),
                    item_title: item.map(|item| item.title.clone()).unwrap_or_default(),
                    reason: reason.into(),
                    session: Some(session),
                })
            })
            .collect();
        let rank = |entry: &pb::InboxEntry| match entry.reason() {
            pb::InboxReason::Approval => 0,
            pb::InboxReason::Failed => 1,
            pb::InboxReason::Reply | pb::InboxReason::Unspecified => 2,
        };
        entries.sort_by_key(rank);
        Ok(pb::Inbox { entries })
    }

    fn create_item(&self, create: pb::CreateItem) -> Result<Item, DaemonError> {
        let fields = ItemFields {
            parent_id: convert::optional(create.parent_id.clone()),
            status: Status::from_wire(create.status()),
            priority: Priority::from_wire(create.priority()),
            title: create.title,
            spec: create.spec,
        };
        Ok(self.store.add_item(&create.project_id, fields)?)
    }

    fn update_item(&self, update: pb::UpdateItem) -> Result<Item, DaemonError> {
        let patch = ItemPatch {
            status: update.status.map(|_| Status::from_wire(update.status())),
            priority: update
                .priority
                .map(|_| Priority::from_wire(update.priority())),
            title: update.title,
            spec: update.spec,
        };
        Ok(self.store.update_item(&update.item_id, patch)?)
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
                    self.own(&mut session, host, None, None)?;
                    if project.is_empty() || session.project_id == project {
                        all.push(session);
                    }
                }
            }
        }
        Ok(pb::Sessions { sessions: all })
    }

    fn create_project(&self, create: pb::CreateProject) -> Result<pb::Project, DaemonError> {
        if create.repositories.is_empty() {
            return Err(DaemonError::NoRepositories);
        }
        let mut repositories: Vec<Repository> = Vec::new();
        for repository in create.repositories {
            let path = PathBuf::from(repository.path);
            let not_a_folder = |source| DaemonError::NotAFolder {
                repository: path.clone(),
                source,
            };
            if !path.is_absolute() {
                return Err(not_a_folder(None));
            }
            let metadata = std::fs::metadata(&path).map_err(|source| not_a_folder(Some(source)))?;
            if !metadata.is_dir() {
                return Err(not_a_folder(None));
            }
            let wanted = Some(repository.name)
                .filter(|name| !name.is_empty())
                .or_else(|| {
                    path.file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                });
            let taken: Vec<&str> = repositories
                .iter()
                .map(|known| known.name.as_str())
                .collect();
            let name = folder_name(wanted.as_deref(), &taken);
            repositories.push(Repository { name, path });
        }
        let project = self
            .store
            .add_project(create.name, repositories, create.instructions)?;
        Ok(convert::project(project))
    }

    /// Fills in what `start` leaves to its project and item, and returns
    /// them.
    fn from_project(
        &self,
        start: &mut pb::StartSession,
    ) -> Result<(Option<String>, Option<String>), DaemonError> {
        let mut project_id = std::mem::take(&mut start.project_id);
        let item = match convert::optional(std::mem::take(&mut start.item_id)) {
            Some(id) => Some(self.store.item(&id)?),
            None => None,
        };
        if let Some(item) = &item {
            if project_id.is_empty() {
                project_id.clone_from(&item.project_id);
            } else if project_id != item.project_id {
                return Err(StoreError::ItemInAnotherProject {
                    title: item.fields.title.clone(),
                }
                .into());
            }
        }
        if project_id.is_empty() {
            return Ok((None, None));
        }
        let project = self.store.project(&project_id)?;
        if start.working_directory.is_empty() && start.repositories.is_empty() {
            if !start.host.is_empty() {
                return Err(DaemonError::NoRemoteDirectory {
                    project: project.name,
                    host: start.host.clone(),
                });
            }
            start.repositories = project
                .repositories
                .iter()
                .map(convert::wire_repository)
                .collect();
        }
        if start.instructions.is_empty() {
            start.instructions = project.instructions;
        }
        if let Some(item) = &item {
            let parent = match (&item.fields.parent_id, start.specs()) {
                (Some(parent), pb::SpecScope::WithParents) => Some(self.store.item(parent)?),
                _ => None,
            };
            start.instructions = with_specs(&start.instructions, item, parent.as_ref());
        }
        Ok((Some(project_id), item.map(|item| item.id)))
    }

    async fn answer(&self, request: pb::request::Kind) -> Result<Reply, DaemonError> {
        use pb::request::Kind as Request;
        use pb::response::Kind as Response;
        let mut request = request;
        let mut project = None;
        let mut item = None;
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
            Request::CreateItem(create) => {
                let created = self.create_item(std::mem::take(create))?;
                let item = self.shown_item(&created).await?;
                return Ok(Reply::Response(Response::ItemCreated(pb::ItemCreated {
                    item: Some(item),
                })));
            }
            Request::UpdateItem(update) => {
                let updated = self.update_item(std::mem::take(update))?;
                let item = self.shown_item(&updated).await?;
                return Ok(Reply::Response(Response::ItemUpdated(pb::ItemUpdated {
                    item: Some(item),
                })));
            }
            Request::ListItems(list) => {
                let items = self.shown_items(&list.project_id).await?;
                return Ok(Reply::Response(Response::Items(pb::Items { items })));
            }
            Request::ReadBoardLog(read) => {
                let changes = self.store.board_log(&read.project_id)?;
                return Ok(Reply::Response(Response::BoardLog(pb::BoardLog {
                    changes: changes.iter().map(board::Change::wire).collect(),
                })));
            }
            Request::RevertBoardChange(revert) => {
                let item = match self.store.revert(revert.change_id)? {
                    Some(item) => Some(self.shown_item(&item).await?),
                    None => None,
                };
                return Ok(Reply::Response(Response::BoardChangeReverted(
                    pb::BoardChangeReverted { item },
                )));
            }
            Request::ListInbox(_) => {
                let inbox = self.inbox().await?;
                return Ok(Reply::Response(Response::Inbox(inbox)));
            }
            Request::StartSession(start) => {
                (project, item) = self.from_project(start)?;
                // The host runs it on itself; only the daemon knows hosts by
                // name.
                std::mem::take(&mut start.host)
            }
            Request::ForkSession(fork) => {
                // A fork stays in its parent's project, on its item.
                let host = self.host_of(&fork.session_id).await?;
                if let Some(placement) = self.store.placement(&fork.session_id)? {
                    project = placement.project;
                    item = placement.item;
                }
                host
            }
            Request::SendMessage(message) => self.host_of(&message.session_id).await?,
            Request::Approve(approve) => self.host_of(&approve.session_id).await?,
            Request::ReadHistory(read) => self.host_of(&read.session_id).await?,
            Request::WatchSession(watch) => self.host_of(&watch.session_id).await?,
            Request::AbortTurn(abort) => self.host_of(&abort.session_id).await?,
            Request::SetApprovalMode(set) => self.host_of(&set.session_id).await?,
            Request::UpdateSession(update) => {
                let host = self.host_of(&update.session_id).await?;
                if let Some(moved) = update.item_id.take() {
                    let moved = convert::optional(moved);
                    self.store
                        .move_session(&update.session_id, moved.as_deref())?;
                }
                host
            }
            Request::ReadChanges(read) => self.host_of(&read.session_id).await?,
        };
        let mut reply = self.forward(&host, request).await?;
        if let Reply::Response(
            Response::SessionStarted(pb::SessionStarted {
                session: Some(session),
            })
            | Response::SessionForked(pb::SessionForked {
                session: Some(session),
            })
            | Response::SessionUpdated(pb::SessionUpdated {
                session: Some(session),
            }),
        ) = &mut reply
        {
            self.own(session, &host, project.as_deref(), item.as_deref())?;
        }
        Ok(reply)
    }
}

/// `instructions`, followed by the specs a session on `item` reads: its
/// task's first if given, then its own.
fn with_specs(instructions: &str, item: &Item, parent: Option<&Item>) -> String {
    let mut text = instructions.trim_end().to_owned();
    let mut section = |heading: &str, item: &Item| {
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(&format!("# {heading}: {}", item.fields.title));
        let spec = item.fields.spec.trim();
        if !spec.is_empty() {
            text.push_str("\n\n");
            text.push_str(spec);
        }
    };
    if let Some(parent) = parent {
        section("The task this is part of", parent);
    }
    let heading = if item.fields.parent_id.is_some() {
        "Your subtask"
    } else {
        "Your task"
    };
    section(heading, item);
    text
}

/// A name for a repository's folder in a session's copy: `wanted` if it is
/// one folder's name and not `taken`, else made so.
fn folder_name(wanted: Option<&str>, taken: &[&str]) -> String {
    let base: String = wanted
        .unwrap_or_default()
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c.is_control() {
                '-'
            } else {
                c
            }
        })
        .collect();
    let base = match base.trim() {
        "" | "." | ".." => "repository".to_owned(),
        trimmed => trimmed.to_owned(),
    };
    let mut name = base.clone();
    let mut count = 1;
    while taken.contains(&name.as_str()) {
        count += 1;
        name = format!("{base}-{count}");
    }
    name
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

#[cfg(test)]
mod specs_tests {
    use super::*;

    fn item(title: &str, parent: Option<&str>, spec: &str) -> Item {
        Item {
            id: title.to_owned(),
            project_id: "p".to_owned(),
            fields: ItemFields {
                parent_id: parent.map(str::to_owned),
                title: title.to_owned(),
                spec: spec.to_owned(),
                status: Status::Todo,
                priority: Priority::None,
            },
            created_unix_ms: 0,
            updated_unix_ms: 0,
            session_ids: Vec::new(),
        }
    }

    #[test]
    fn a_session_reads_its_specs_after_its_instructions_parents_first() {
        let task = item("login", None, "Let people sign in.");
        let subtask = item("form", Some("login"), "The form.");
        let alone = with_specs("be brief", &subtask, None);
        assert!(alone.starts_with("be brief"), "{alone}");
        assert!(alone.contains("The form."), "{alone}");
        assert!(!alone.contains("Let people sign in."), "{alone}");

        let both = with_specs("", &subtask, Some(&task));
        let parent = both.find("Let people sign in.");
        let own = both.find("The form.");
        assert!(parent.is_some() && parent < own, "{both}");
    }
}

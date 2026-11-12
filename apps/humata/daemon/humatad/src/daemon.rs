//! The daemon role: one place the app and the CLI talk to, which runs
//! sessions on this Mac or passes them to a host it reaches over a command's
//! stdin and stdout (in practice `ssh <host> humatad host --stdio`).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock};

use humata_wire::{ClientError, Reply, Service, ServiceFuture, describe, error, pb, streams};
use tokio::sync::mpsc;

use crate::board::{self, Item, ItemFields, ItemPatch, Priority, Status};
use crate::convert;
use crate::error::DaemonError;
use crate::error::StoreError;
use crate::host::Host;
use crate::hosts::{self, HostConfig, Hosts, Remote, Ssh};
use crate::store::Store;
use humata_agent::Repository;

mod review;

/// Answers clients: sessions on this Mac go to the local host, others to the
/// host they run on. Projects, and where each session is, are the daemon's.
pub struct Daemon {
    local: Arc<Host>,
    remotes: RwLock<BTreeMap<String, Arc<Remote>>>,
    /// Held while the hosts file is changed, so changes don't race.
    editing: tokio::sync::Mutex<()>,
    hosts_file: PathBuf,
    store: Store,
    /// This humatad's build, which managed hosts are kept at.
    build: String,
}

impl Daemon {
    pub fn new(
        local: Host,
        store: Store,
        hosts: Hosts,
        hosts_file: PathBuf,
        build: String,
    ) -> Self {
        let remotes = hosts
            .hosts
            .into_iter()
            .map(|(name, config)| (name.clone(), Arc::new(Remote::new(name, config))))
            .collect();
        Self {
            local: Arc::new(local),
            remotes: RwLock::new(remotes),
            editing: tokio::sync::Mutex::new(()),
            hosts_file,
            store,
            build,
        }
    }

    fn remote(&self, host: &str) -> Result<Arc<Remote>, DaemonError> {
        self.remotes
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(host)
            .cloned()
            .ok_or_else(|| DaemonError::UnknownHost {
                host: host.to_owned(),
                hosts: self.hosts_file.clone(),
            })
    }

    fn remote_names(&self) -> Vec<String> {
        self.remotes
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .cloned()
            .collect()
    }

    /// The hosts as they are now, for the hosts file.
    fn hosts(&self) -> Hosts {
        let remotes = self.remotes.read().unwrap_or_else(PoisonError::into_inner);
        Hosts {
            hosts: remotes
                .iter()
                .map(|(name, remote)| (name.clone(), remote.config()))
                .collect(),
        }
    }

    fn add_host(&self, add: pb::AddHost) -> Result<pb::HostInfo, DaemonError> {
        let name = add.name.trim().to_owned();
        let named = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        if !named {
            return Err(DaemonError::BadHostName { name });
        }
        let destination = add.ssh_destination.trim().to_owned();
        if destination.is_empty() {
            return Err(DaemonError::NoDestination);
        }
        let folder = Some(add.folder.trim().trim_end_matches('/'))
            .filter(|folder| !folder.is_empty())
            .unwrap_or(hosts::DEFAULT_FOLDER)
            .to_owned();
        let config = HostConfig {
            ssh: Some(Ssh {
                destination,
                config: convert::optional(add.ssh_config).map(PathBuf::from),
                folder,
            }),
            ..HostConfig::default()
        };
        let mut hosts = self.hosts();
        if hosts.hosts.contains_key(&name) {
            return Err(DaemonError::HostExists { host: name });
        }
        hosts.hosts.insert(name.clone(), config.clone());
        hosts.save(&self.hosts_file)?;
        let info = config.info(&name);
        self.remotes
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name.clone(), Arc::new(Remote::new(name, config)));
        Ok(info)
    }

    async fn update_host(&self, update: pb::UpdateHost) -> Result<pb::HostInfo, DaemonError> {
        let remote = self.remote(&update.name)?;
        let mut config = remote.config();
        for var in update.set_env {
            config.env.insert(var.name, var.value);
        }
        for name in update.unset_env {
            config.env.remove(&name);
        }
        if let Some(script) = update.setup_script {
            config.setup_script = script;
        }
        let mut hosts = self.hosts();
        hosts.hosts.insert(update.name.clone(), config.clone());
        hosts.save(&self.hosts_file)?;
        let info = config.info(&update.name);
        remote.reconfigure(config).await?;
        Ok(info)
    }

    async fn remove_host(&self, name: &str) -> Result<(), DaemonError> {
        let remote = self.remote(name)?;
        let mut hosts = self.hosts();
        hosts.hosts.remove(name);
        hosts.save(&self.hosts_file)?;
        self.remotes
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(name);
        remote.disconnect().await;
        self.store.forget_host(name)?;
        Ok(())
    }

    fn managed(&self, name: &str) -> Result<(Arc<Remote>, Ssh), DaemonError> {
        let remote = self.remote(name)?;
        let ssh = remote.config().ssh.ok_or_else(|| DaemonError::NotManaged {
            host: name.to_owned(),
        })?;
        Ok((remote, ssh))
    }

    async fn install_host(
        &self,
        install: pb::InstallHost,
    ) -> Result<pb::HostInstalled, DaemonError> {
        let (remote, ssh) = self.managed(&install.name)?;
        remote
            .install(&ssh, convert::optional(install.binary).map(PathBuf::from))
            .await?;
        remote.disconnect().await;
        remote.retry_now();
        let client = remote.client(&self.hosts_file, &self.build).await?;
        Ok(pb::HostInstalled {
            program_version: client.peer().program_version.clone(),
        })
    }

    async fn check_host(&self, name: &str) -> Result<pb::HostHealth, DaemonError> {
        let remote = self.remote(name)?;
        remote.retry_now();
        let status = match remote.client(&self.hosts_file, &self.build).await {
            Ok(client) => {
                remote
                    .call(
                        &client,
                        pb::request::Kind::ReadHostStatus(pb::ReadHostStatus {}),
                    )
                    .await
            }
            Err(failure) => Err(failure),
        };
        Ok(match status {
            Ok(pb::response::Kind::HostStatus(status)) => pb::HostHealth {
                name: name.to_owned(),
                reachable: true,
                problem: String::new(),
                status: Some(status),
            },
            Ok(_) => pb::HostHealth {
                name: name.to_owned(),
                reachable: true,
                problem: "it answered with something else than its status; install this build of humatad on it".to_owned(),
                status: None,
            },
            Err(failure) => pb::HostHealth {
                name: name.to_owned(),
                reachable: false,
                problem: describe(&failure),
                status: None,
            },
        })
    }

    /// `project`'s repositories as cloned on `host`, cloning or updating
    /// them there from each one's `origin`.
    async fn clone_on(
        &self,
        host: &str,
        project: &crate::store::Project,
    ) -> Result<Vec<pb::Repository>, DaemonError> {
        let mut repositories = Vec::new();
        for repository in &project.repositories {
            repositories.push(pb::RemoteRepository {
                name: repository.name.clone(),
                url: origin(host, &repository.path).await?,
            });
        }
        let request = pb::request::Kind::CloneRepositories(pb::CloneRepositories {
            folder: project.id.clone(),
            repositories,
        });
        let remote = self.remote(host)?;
        let client = remote.client(&self.hosts_file, &self.build).await?;
        match remote.call(&client, request).await? {
            pb::response::Kind::RepositoriesCloned(cloned) => Ok(cloned.repositories),
            _ => Err(DaemonError::Host {
                host: host.to_owned(),
                source: ClientError::Unexpected {
                    expected: "the repositories cloned",
                    got: "another answer".to_owned(),
                },
            }),
        }
    }

    /// Sends `request` to `host` ("" for this Mac).
    async fn forward(&self, host: &str, request: pb::request::Kind) -> Result<Reply, DaemonError> {
        if host.is_empty() {
            return Ok(self.local.clone().handle(request).await);
        }
        let client = self
            .remote(host)?
            .client(&self.hosts_file, &self.build)
            .await?;
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
        let (project, item, seen) = placement
            .map(|placement| (placement.project, placement.item, placement.seen_turns))
            .unwrap_or_default();
        session.project_id = project.unwrap_or_default();
        session.item_id = item.unwrap_or_default();
        session.seen_turns = seen;
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
        let sessions = self.list("").await?.sessions;
        let mut items: HashMap<String, pb::Item> = HashMap::new();
        for project in &projects {
            let mut shown: Vec<pb::Item> = self
                .store
                .items(&project.id)?
                .iter()
                .map(Item::wire)
                .collect();
            board::show(&mut shown, &sessions);
            items.extend(shown.into_iter().map(|item| (item.id.clone(), item)));
        }
        let names: HashMap<&str, &str> = projects
            .iter()
            .map(|project| (project.id.as_str(), project.name.as_str()))
            .collect();
        let mut entries: Vec<pb::InboxEntry> = sessions
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
    /// that can't be reached shows what it last had, marked offline, so one
    /// machine being off doesn't hide the rest.
    async fn list(&self, project: &str) -> Result<pb::Sessions, DaemonError> {
        let mut all = Vec::new();
        // Every host at once, so a slow one costs its own time only.
        let mut asking = tokio::task::JoinSet::new();
        for (index, host) in self.remote_names().into_iter().enumerate() {
            let remote = self.remote(&host)?;
            let (hosts_file, build) = (self.hosts_file.clone(), self.build.clone());
            asking.spawn(async move {
                let request = pb::request::Kind::ListSessions(pb::ListSessions::default());
                let answer = match remote.client(&hosts_file, &build).await {
                    Ok(client) => remote.call(&client, request).await.ok(),
                    Err(_) => None,
                };
                (index, host, answer)
            });
        }
        let local = self
            .forward(
                "",
                pb::request::Kind::ListSessions(pb::ListSessions::default()),
            )
            .await;
        let mut answers = vec![(
            String::new(),
            match local {
                Ok(Reply::Response(kind)) => Some(kind),
                _ => None,
            },
        )];
        let mut remotes = asking.join_all().await;
        remotes.sort_by_key(|(index, _, _)| *index);
        answers.extend(remotes.into_iter().map(|(_, host, answer)| (host, answer)));
        for (host, answer) in answers {
            let sessions = match answer {
                Some(pb::response::Kind::Sessions(sessions)) => {
                    if !host.is_empty() {
                        self.store.cache_sessions(&host, &sessions.sessions)?;
                    }
                    sessions.sessions
                }
                _ if host.is_empty() => Vec::new(),
                _ => {
                    let mut cached = self.store.cached_sessions(&host)?;
                    for session in &mut cached {
                        session.host_offline = true;
                    }
                    cached
                }
            };
            for mut session in sessions {
                self.own(&mut session, &host, None, None)?;
                if project.is_empty() || session.project_id == project {
                    all.push(session);
                }
            }
        }
        Ok(pb::Sessions { sessions: all })
    }

    /// `session`'s history from its host, kept for when it is offline, or as
    /// last kept if it is.
    async fn history(&self, host: &str, read: pb::ReadHistory) -> Result<Reply, DaemonError> {
        let session = read.session_id.clone();
        match self
            .forward(host, pb::request::Kind::ReadHistory(read))
            .await
        {
            Ok(Reply::Response(pb::response::Kind::History(history))) => {
                if !host.is_empty() {
                    self.store.cache_history(&session, &history)?;
                }
                Ok(Reply::Response(pb::response::Kind::History(history)))
            }
            Err(failure) if failure.code() == pb::ErrorCode::HostUnavailable => {
                match self.store.cached_history(&session)? {
                    Some(history) => Ok(Reply::Response(pb::response::Kind::History(history))),
                    None => Err(failure),
                }
            }
            other => other,
        }
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
    async fn from_project(
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
            start.repositories = if start.host.is_empty() {
                project
                    .repositories
                    .iter()
                    .map(convert::wire_repository)
                    .collect()
            } else {
                self.clone_on(&start.host, &project).await?
            };
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
                (project, item) = self.from_project(start).await?;
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
            Request::ReadHistory(read) => {
                let host = self.host_of(&read.session_id).await?;
                return self.history(&host, std::mem::take(read)).await;
            }
            Request::ListHosts(_) => {
                let hosts = self
                    .hosts()
                    .hosts
                    .iter()
                    .map(|(name, config)| config.info(name))
                    .collect();
                return Ok(Reply::Response(Response::Hosts(pb::Hosts { hosts })));
            }
            Request::AddHost(add) => {
                let _editing = self.editing.lock().await;
                let host = self.add_host(std::mem::take(add))?;
                return Ok(Reply::Response(Response::HostSaved(pb::HostSaved {
                    host: Some(host),
                })));
            }
            Request::UpdateHost(update) => {
                let _editing = self.editing.lock().await;
                let host = self.update_host(std::mem::take(update)).await?;
                return Ok(Reply::Response(Response::HostSaved(pb::HostSaved {
                    host: Some(host),
                })));
            }
            Request::RemoveHost(remove) => {
                let _editing = self.editing.lock().await;
                self.remove_host(&remove.name).await?;
                return Ok(Reply::Response(Response::HostRemoved(pb::HostRemoved {})));
            }
            Request::CheckHost(check) => {
                let health = self.check_host(&check.name).await?;
                return Ok(Reply::Response(Response::HostHealth(health)));
            }
            Request::InstallHost(install) => {
                let installed = self.install_host(std::mem::take(install)).await?;
                return Ok(Reply::Response(Response::HostInstalled(installed)));
            }
            Request::ListSshHosts(_) => {
                return Ok(Reply::Response(Response::SshHosts(pb::SshHosts {
                    names: ssh_config_hosts(),
                })));
            }
            Request::ConfigureHost(_)
            | Request::ReadHostStatus(_)
            | Request::CloneRepositories(_) => {
                return Err(DaemonError::ForHostsOnly);
            }
            Request::WatchSession(watch) => self.host_of(&watch.session_id).await?,
            Request::AbortTurn(abort) => self.host_of(&abort.session_id).await?,
            Request::Unqueue(unqueue) => self.host_of(&unqueue.session_id).await?,
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
            Request::ReadChanges(read) => {
                let host = self.host_of(&read.session_id).await?;
                return self.read_changes(&host, std::mem::take(read)).await;
            }
            Request::EditChanges(edit) => self.host_of(&edit.session_id).await?,
            Request::ListFiles(list) => self.host_of(&list.session_id).await?,
            Request::ReadFile(read) => self.host_of(&read.session_id).await?,
            Request::MarkSeen(mark) => {
                self.store.mark_seen(&mark.session_id, mark.turns)?;
                return Ok(Reply::Response(Response::Seen(pb::Seen {})));
            }
            Request::MarkViewed(mark) => {
                self.mark_viewed(mark)?;
                return Ok(Reply::Response(Response::FileViewed(pb::FileViewed {})));
            }
            Request::CreateThread(create) => {
                let thread = self.create_thread(std::mem::take(create)).await?;
                return Ok(Reply::Response(Response::ThreadSaved(pb::ThreadSaved {
                    thread: Some(thread),
                })));
            }
            Request::ReplyToThread(reply) => {
                let thread = self.reply_to_thread(reply).await?;
                return Ok(Reply::Response(Response::ThreadSaved(pb::ThreadSaved {
                    thread: Some(thread),
                })));
            }
            Request::ResolveThread(resolve) => {
                let thread = self.resolve_thread(resolve).await?;
                return Ok(Reply::Response(Response::ThreadSaved(pb::ThreadSaved {
                    thread: Some(thread),
                })));
            }
            Request::DeleteComment(delete) => {
                self.delete_comment(delete)?;
                return Ok(Reply::Response(Response::CommentDeleted(
                    pb::CommentDeleted {},
                )));
            }
            Request::ListThreads(list) => {
                let threads = self.threads(&list.session_id).await?;
                return Ok(Reply::Response(Response::Threads(pb::Threads { threads })));
            }
            Request::SendReview(send) => {
                let comments = self.send_review(&send.session_id).await?;
                return Ok(Reply::Response(Response::ReviewSent(pb::ReviewSent {
                    comments,
                })));
            }
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

/// The hosts in `~/.ssh/config`; none if it can't be read.
fn ssh_config_hosts() -> Vec<String> {
    std::env::var_os("HOME")
        .map(|home| Path::new(&home).join(".ssh/config"))
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|config| hosts::ssh_hosts(&config))
        .unwrap_or_default()
}

/// The URL `host` clones the repository at `path` from: its `origin`.
async fn origin(host: &str, path: &Path) -> Result<String, DaemonError> {
    let no_origin = |source| DaemonError::NoOrigin {
        host: host.to_owned(),
        path: path.to_owned(),
        source,
    };
    let output = tokio::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["remote", "get-url", "origin"])
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|source| no_origin(Some(source)))?;
    let url = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !output.status.success() || url.is_empty() {
        return Err(no_origin(None));
    }
    Ok(url)
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

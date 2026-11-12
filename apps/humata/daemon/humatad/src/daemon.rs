//! The daemon role: one place the app and the CLI talk to, which runs
//! sessions on this Mac or passes them to a host it reaches over a command's
//! stdin and stdout (in practice `ssh <host> humatad host --stdio`).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock, Weak};

use humata_wire::{ClientError, Reply, Service, ServiceFuture, describe, error, pb, streams};
use tokio::sync::mpsc;

use crate::board::{self, Item, ItemFields, ItemPatch, Priority, Status};
use crate::convert;
use crate::error::DaemonError;
use crate::error::StoreError;
use crate::host::Host;
use crate::hosts::{self, HostConfig, Hosts, Remote, Ssh, ToolCall};
use crate::store::{Placement, Store};
use humata_agent::Repository;

/// How long a watch whose host dropped waits before watching again, at
/// first and at most.
const REWATCH_FIRST_WAIT: std::time::Duration = std::time::Duration::from_secs(1);
const REWATCH_LONGEST_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

mod forwards;
mod planning;
mod recovery;
mod review;

/// Answers clients: sessions on this Mac go to the local host, others to the
/// host they run on. Projects, and where each session is, are the daemon's.
pub struct Daemon {
    local: Arc<Host>,
    remotes: RwLock<BTreeMap<String, Arc<Remote>>>,
    /// Held while the hosts file is changed, so changes don't race.
    editing: tokio::sync::Mutex<()>,
    hosts_file: PathBuf,
    /// The variables for the coordinator's own agents, those on the Mac the
    /// daemon runs on, as saved in the hosts file.
    coordinator_env: std::sync::Mutex<BTreeMap<String, String>>,
    store: Arc<Store>,
    /// This humatad's build, which managed hosts are kept at.
    build: String,
    /// Hosts' agents' calls to the humata tools, for [`Daemon::answer_tool_calls`].
    tool_calls: mpsc::UnboundedSender<ToolCall>,
    /// This daemon, for work that outlives the request that started it.
    me: Weak<Daemon>,
    /// Where recovery copies' code is kept, a mirror per repository.
    recovery: PathBuf,
    /// The sessions being backed up, each with whether to back it up again
    /// once done.
    backing_up: std::sync::Mutex<HashMap<String, bool>>,
    /// The ports forwarded to hosts, by id.
    forwards: std::sync::Mutex<BTreeMap<String, forwards::Forwarding>>,
    next_forward: std::sync::atomic::AtomicU64,
}

impl Daemon {
    /// The daemon, answering its hosts' agents' tool calls from now on.
    pub fn new(
        local: Host,
        store: Store,
        hosts: Hosts,
        hosts_file: PathBuf,
        recovery: PathBuf,
        build: String,
    ) -> Arc<Self> {
        let (tool_calls, calls) = mpsc::unbounded_channel();
        local.set_env(hosts.coordinator_env.clone());
        let coordinator_env = std::sync::Mutex::new(hosts.coordinator_env);
        let remotes = hosts
            .hosts
            .into_iter()
            .map(|(name, config)| {
                let remote = Remote::new(name.clone(), config, tool_calls.clone());
                (name, Arc::new(remote))
            })
            .collect();
        let daemon = Arc::new_cyclic(|me| Self {
            me: me.clone(),
            local: Arc::new(local),
            remotes: RwLock::new(remotes),
            editing: tokio::sync::Mutex::new(()),
            hosts_file,
            coordinator_env,
            store: Arc::new(store),
            build,
            tool_calls,
            recovery,
            backing_up: std::sync::Mutex::default(),
            forwards: std::sync::Mutex::default(),
            next_forward: std::sync::atomic::AtomicU64::new(0),
        });
        tokio::spawn(answer_tool_calls(Arc::downgrade(&daemon), calls));
        daemon
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
            coordinator_env: self
                .coordinator_env
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        }
    }

    /// The coordinator as a host: the names of its agents' variables, and no
    /// name.
    fn coordinator(&self) -> pb::HostInfo {
        pb::HostInfo {
            env_names: self
                .coordinator_env
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .keys()
                .cloned()
                .collect(),
            ..Default::default()
        }
    }

    /// Sets or unsets variables for the coordinator's agents. It has no
    /// setup script: the person sets their Mac up themselves.
    fn update_coordinator(&self, update: pb::UpdateHost) -> Result<pb::HostInfo, DaemonError> {
        if update.setup_script.is_some() {
            return Err(DaemonError::NoSetupHere);
        }
        let mut env = self
            .coordinator_env
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        for var in update.set_env {
            env.insert(var.name, var.value);
        }
        for name in update.unset_env {
            env.remove(&name);
        }
        let mut hosts = self.hosts();
        hosts.coordinator_env = env.clone();
        hosts.save(&self.hosts_file)?;
        self.local.set_env(env.clone());
        *self
            .coordinator_env
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = env;
        Ok(self.coordinator())
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
            .insert(
                name.clone(),
                Arc::new(Remote::new(name, config, self.tool_calls.clone())),
            );
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
    ) -> Result<pb::RepositoriesCloned, DaemonError> {
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
            pb::response::Kind::RepositoriesCloned(cloned) => Ok(cloned),
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

    /// Sends a request that answers with terminals to `host`, and names
    /// the host in them.
    async fn terminals(
        &self,
        host: &str,
        request: pb::request::Kind,
    ) -> Result<Reply, DaemonError> {
        let mut reply = self.forward(host, request).await?;
        match &mut reply {
            Reply::Response(pb::response::Kind::TerminalOpened(pb::TerminalOpened {
                terminal: Some(terminal),
            })) => host.clone_into(&mut terminal.host),
            Reply::Response(pb::response::Kind::Terminals(pb::Terminals { terminals })) => {
                for terminal in terminals {
                    host.clone_into(&mut terminal.host);
                }
            }
            _ => {}
        }
        Ok(reply)
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
        session.planning = self.store.planning(&session.id)?;
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
        let requests = self.store.open_requests()?;
        let asking: std::collections::HashSet<&str> = requests
            .iter()
            .map(|request| request.session_id.as_str())
            .collect();
        let project_name = |session: &pb::SessionInfo| {
            names
                .get(session.project_id.as_str())
                .map(|name| (*name).to_owned())
                .unwrap_or_default()
        };
        let mut entries: Vec<pb::InboxEntry> = Vec::new();
        for request in &requests {
            let Some(session) = sessions
                .iter()
                .find(|session| session.id == request.session_id && !session.archived)
            else {
                continue;
            };
            let item = items
                .get(&request.item_id)
                .or_else(|| items.get(&session.item_id));
            entries.push(pb::InboxEntry {
                project_name: project_name(session),
                item_title: item.map(|item| item.title.clone()).unwrap_or_default(),
                reason: match request.kind() {
                    pb::AgentRequestKind::Move => pb::InboxReason::Move,
                    _ => pb::InboxReason::Question,
                }
                .into(),
                session: Some(session.clone()),
                request: Some(request.clone()),
            });
        }
        for session in &sessions {
            let item = items.get(&session.item_id);
            let Some(reason) = board::inbox_reason(session, item) else {
                continue;
            };
            // What it asks says more than that it replied.
            if reason == pb::InboxReason::Reply && asking.contains(session.id.as_str()) {
                continue;
            }
            entries.push(pb::InboxEntry {
                project_name: project_name(session),
                item_title: item.map(|item| item.title.clone()).unwrap_or_default(),
                reason: reason.into(),
                session: Some(session.clone()),
                request: None,
            });
        }
        let rank = |entry: &pb::InboxEntry| match entry.reason() {
            pb::InboxReason::Approval => 0,
            pb::InboxReason::Question | pb::InboxReason::Move => 1,
            pb::InboxReason::Failed => 2,
            pb::InboxReason::Reply | pb::InboxReason::Unspecified => 3,
        };
        entries.sort_by_key(rank);
        Ok(pb::Inbox { entries })
    }

    /// Where `session` is, found on its host if the daemon hasn't seen it.
    async fn placement(&self, session: &str) -> Result<Placement, DaemonError> {
        self.host_of(session).await?;
        self.store.placement(session)?.ok_or_else(|| {
            StoreError::UnknownSession {
                id: session.to_owned(),
            }
            .into()
        })
    }

    async fn ask_person(&self, ask: pb::AskPerson) -> Result<pb::AgentRequest, DaemonError> {
        let text = ask.text.trim();
        if text.is_empty() {
            return Err(DaemonError::NoRequestText);
        }
        self.placement(&ask.session_id).await?;
        Ok(self.store.add_request(pb::AgentRequest {
            session_id: ask.session_id,
            kind: pb::AgentRequestKind::Question.into(),
            text: text.to_owned(),
            ..Default::default()
        })?)
    }

    async fn request_move(&self, asked: pb::RequestMove) -> Result<pb::AgentRequest, DaemonError> {
        let summary = asked.summary.trim();
        if summary.is_empty() {
            return Err(DaemonError::NoRequestText);
        }
        let status = asked.status();
        if !matches!(
            status,
            pb::ItemStatus::InReview | pb::ItemStatus::Done | pb::ItemStatus::WontDo
        ) {
            return Err(DaemonError::NotAMove);
        }
        let placement = self.placement(&asked.session_id).await?;
        let item = self.store.item(&asked.item_id)?;
        if placement.project.as_deref() != Some(item.project_id.as_str()) {
            return Err(StoreError::ItemInAnotherProject {
                title: item.fields.title,
            }
            .into());
        }
        Ok(self.store.add_request(pb::AgentRequest {
            session_id: asked.session_id,
            kind: pb::AgentRequestKind::Move.into(),
            text: summary.to_owned(),
            item_id: item.id,
            status: status.into(),
            ..Default::default()
        })?)
    }

    /// Does what the person answered, then takes the request out of the
    /// inbox; if doing it fails, the request stays.
    async fn answer_request(&self, answer: pb::AnswerRequest) -> Result<(), DaemonError> {
        use pb::answer_request::Answer;
        let request = self.store.open_request(&answer.request_id)?;
        let move_to = |status: pb::ItemStatus| {
            pb::request::Kind::UpdateItem(pb::UpdateItem {
                item_id: request.item_id.clone(),
                status: Some(status.into()),
                ..Default::default()
            })
        };
        let doing = match answer.answer {
            Some(Answer::Reply(text)) => {
                let text = text.trim();
                if text.is_empty() {
                    return Err(DaemonError::NoReply);
                }
                Some(pb::request::Kind::SendMessage(pb::SendMessage {
                    session_id: request.session_id.clone(),
                    text: text.to_owned(),
                    ..Default::default()
                }))
            }
            Some(Answer::Approve(_)) if request.kind() == pb::AgentRequestKind::Move => {
                Some(move_to(request.status()))
            }
            Some(Answer::Approve(_)) => return Err(DaemonError::NothingToApprove),
            Some(Answer::MoveTo(status)) if request.kind() == pb::AgentRequestKind::Move => Some(
                move_to(pb::ItemStatus::try_from(status).unwrap_or_default()),
            ),
            Some(Answer::MoveTo(_)) => return Err(DaemonError::NothingToApprove),
            Some(Answer::Dismiss(_)) | None => None,
        };
        if let Some(doing) = doing {
            Box::pin(self.answer(doing)).await?;
        }
        Ok(self.store.answer_request(&request.id)?)
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
    async fn delete_project(&self, id: &str) -> Result<(), DaemonError> {
        let project = self.store.project(id)?;
        let working = self
            .list(id)
            .await?
            .sessions
            .iter()
            .filter(|session| !session.archived)
            .count();
        if working > 0 {
            return Err(DaemonError::ProjectInUse {
                name: project.name,
                working,
            });
        }
        Ok(self.store.delete_project(id)?)
    }

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
                if !host.is_empty() && self.store.moved_away(&session.id, &host)? {
                    continue;
                }
                if !host.is_empty() && !session.host_offline {
                    self.back_up_soon(&host, &session.id, Some(session.turns));
                }
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
        let project = self.store.add_project(
            create.name,
            repositories,
            create.instructions,
            convert::skills(create.skills),
        )?;
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
        // A planning session gets an empty folder of its own from its host.
        let checkout = !start.planning;
        if checkout && start.working_directory.is_empty() && start.repositories.is_empty() {
            start.repositories = if start.host.is_empty() {
                project
                    .repositories
                    .iter()
                    .map(convert::wire_repository)
                    .collect()
            } else {
                self.clone_on(&start.host, &project).await?.repositories
            };
        }
        let parent = match (
            item.as_ref()
                .and_then(|item| item.fields.parent_id.as_ref()),
            start.specs(),
        ) {
            (Some(parent), pb::SpecScope::WithParents) => Some(self.store.item(parent)?),
            _ => None,
        };
        start.instructions = planning::instructions(
            &project,
            &start.instructions,
            item.as_ref(),
            parent.as_ref(),
        );
        Ok((Some(project_id), item.map(|item| item.id)))
    }

    /// Watches `session` on its host, keeping the Mac's copy of its history
    /// up to date as each turn ends. While the host can't be reached, the
    /// copy is sent instead and the stream ends, so the session can still
    /// be read and the app knows to watch again.
    async fn watch(&self, host: &str, watch: pb::WatchSession) -> Result<Reply, DaemonError> {
        let session = watch.session_id.clone();
        let request = pb::request::Kind::WatchSession(watch);
        if host.is_empty() {
            return self.forward(host, request).await;
        }
        let (response, mut upstream) = match self.forward(host, request).await {
            Ok(Reply::Stream { response, items }) => (response, items),
            Err(failure) if failure.code() == pb::ErrorCode::HostUnavailable => {
                let Some(history) = self.store.cached_history(&session)? else {
                    return Err(failure);
                };
                let (items, receiver) = mpsc::channel(history.events.len().max(1));
                for event in history.events {
                    let item = pb::StreamItem {
                        kind: Some(pb::stream_item::Kind::SessionEvent(event)),
                    };
                    // The channel holds every event, so this can't wait.
                    let _ = items.try_send(Ok(item));
                }
                return Ok(Reply::Stream {
                    response: pb::response::Kind::Watching(pb::Watching {}),
                    items: receiver,
                });
            }
            other => return other,
        };
        let store = self.store.clone();
        // Histories only grow, so a copy is written only when it has more
        // than the last, never part of the replay over a fuller one.
        let mut kept = self
            .store
            .cached_history(&session)?
            .map_or(0, |history| history.events.len());
        let (items, receiver) = mpsc::channel(64);
        let (daemon, host) = (self.me.clone(), host.to_owned());
        tokio::spawn(async move {
            let mut events = Vec::new();
            // Events of a replay already passed on, after re-watching.
            let mut skip = 0;
            let mut keep = |events: &Vec<pb::SessionEvent>| {
                if events.len() <= kept {
                    return;
                }
                let history = pb::History {
                    events: events.clone(),
                };
                // A copy that couldn't be written stays as it was; the next
                // turn tries again.
                if store.cache_history(&session, &history).is_ok() {
                    kept = events.len();
                }
            };
            loop {
                let next = tokio::select! {
                    next = upstream.recv() => next,
                    () = items.closed() => break,
                };
                let Some(item) = next else {
                    // The host's connection dropped, or it ended the watch.
                    // Its sessions outlive the connection, so this watches
                    // again from the start of the history and passes on
                    // only what is past the events already sent.
                    let Some(daemon) = daemon.upgrade() else {
                        break;
                    };
                    match daemon.rewatch(&host, &session, &items).await {
                        Some(stream) => {
                            upstream = stream;
                            skip = events.len();
                            continue;
                        }
                        None => break,
                    }
                };
                if let Ok(pb::StreamItem {
                    kind: Some(pb::stream_item::Kind::SessionEvent(event)),
                }) = &item
                {
                    if skip > 0 {
                        skip -= 1;
                        continue;
                    }
                    events.push(event.clone());
                    if matches!(
                        event.kind,
                        Some(pb::session_event::Kind::Idle(_))
                            | Some(pb::session_event::Kind::Checkpoint(_))
                    ) {
                        keep(&events);
                    }
                    if matches!(event.kind, Some(pb::session_event::Kind::Saved(_))) {
                        if let Some(daemon) = daemon.upgrade() {
                            daemon.back_up_soon(&host, &session, None);
                        }
                    }
                    if matches!(event.kind, Some(pb::session_event::Kind::Idle(_))) {
                        if let Some(daemon) = daemon.upgrade() {
                            let turns = events
                                .iter()
                                .filter(|event| {
                                    matches!(
                                        event.kind,
                                        Some(pb::session_event::Kind::TurnEnded(_))
                                    )
                                })
                                .count();
                            let turns = u32::try_from(turns).unwrap_or(u32::MAX);
                            daemon.back_up_soon(&host, &session, Some(turns));
                        }
                    }
                }
                if items.send(item).await.is_err() {
                    break;
                }
            }
            keep(&events);
        });
        Ok(Reply::Stream {
            response,
            items: receiver,
        })
    }

    /// Watches `session` on `host` again, waiting longer between tries while
    /// the host is unreachable, until it answers or `client` stops
    /// listening. None if the host no longer has the session.
    async fn rewatch(
        &self,
        host: &str,
        session: &str,
        client: &mpsc::Sender<Result<pb::StreamItem, pb::Error>>,
    ) -> Option<mpsc::Receiver<Result<pb::StreamItem, pb::Error>>> {
        let mut wait = REWATCH_FIRST_WAIT;
        loop {
            let request = pb::request::Kind::WatchSession(pb::WatchSession {
                session_id: session.to_owned(),
            });
            match self.forward(host, request).await {
                Ok(Reply::Stream { items, .. }) => return Some(items),
                Err(failure) if failure.code() == pb::ErrorCode::HostUnavailable => {}
                _ => return None,
            }
            tokio::select! {
                () = tokio::time::sleep(wait) => {}
                () = client.closed() => return None,
            }
            wait = (wait * 2).min(REWATCH_LONGEST_WAIT);
        }
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
            Request::DeleteProject(delete) => {
                self.delete_project(&delete.project_id).await?;
                return Ok(Reply::Response(Response::ProjectDeleted(
                    pb::ProjectDeleted {},
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
                let spec_changed = update.spec.is_some();
                let updated = self.update_item(std::mem::take(update))?;
                if spec_changed {
                    self.spec_changed(&updated).await;
                }
                let item = self.shown_item(&updated).await?;
                return Ok(Reply::Response(Response::ItemUpdated(pb::ItemUpdated {
                    item: Some(item),
                })));
            }
            Request::ListItems(list) => {
                let items = self.shown_items(&list.project_id).await?;
                return Ok(Reply::Response(Response::Items(pb::Items { items })));
            }
            Request::ListProposals(list) => {
                let proposals = self.proposals(&list.session_id).await?;
                return Ok(Reply::Response(Response::Proposals(pb::Proposals {
                    proposals,
                })));
            }
            Request::ApplyProposal(apply) => {
                let applied = self.apply_proposal(std::mem::take(apply)).await?;
                return Ok(Reply::Response(Response::ProposalApplied(applied)));
            }
            Request::DeclineProposal(decline) => {
                let proposal = self.decline_proposal(decline).await?;
                return Ok(Reply::Response(Response::ProposalDeclined(
                    pb::ProposalDeclined {
                        proposal: Some(proposal),
                    },
                )));
            }
            Request::UpdateProject(update) => {
                let project = self.update_project(std::mem::take(update))?;
                return Ok(Reply::Response(Response::ProjectUpdated(
                    pb::ProjectUpdated {
                        project: Some(project),
                    },
                )));
            }
            Request::ReadSpec(read) => {
                let spec = self.read_spec(read)?;
                return Ok(Reply::Response(Response::Spec(spec)));
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
            Request::AskPerson(ask) => {
                let request = self.ask_person(std::mem::take(ask)).await?;
                return Ok(Reply::Response(Response::RequestMade(pb::RequestMade {
                    request: Some(request),
                })));
            }
            Request::RequestMove(asked) => {
                let request = self.request_move(std::mem::take(asked)).await?;
                return Ok(Reply::Response(Response::RequestMade(pb::RequestMade {
                    request: Some(request),
                })));
            }
            Request::AnswerRequest(answer) => {
                self.answer_request(std::mem::take(answer)).await?;
                return Ok(Reply::Response(Response::RequestAnswered(
                    pb::RequestAnswered {},
                )));
            }
            Request::StartSession(start) => {
                (project, item) = self.from_project(start).await?;
                // The host runs it on itself; only the daemon knows hosts by
                // name.
                std::mem::take(&mut start.host)
            }
            Request::ListImportable(list) => std::mem::take(&mut list.host),
            Request::ListModels(list) => std::mem::take(&mut list.host),
            Request::ListSkipped(list) => std::mem::take(&mut list.host),
            Request::RemoveSkipped(remove) => std::mem::take(&mut remove.host),
            Request::ImportSession(import) => {
                if let Some(id) = convert::optional(std::mem::take(&mut import.project_id)) {
                    project = Some(self.store.project(&id)?.id);
                }
                std::mem::take(&mut import.host)
            }
            Request::ForkSession(fork) => {
                // A fork stays in its parent's project, on its item.
                let host = self.host_of(&fork.session_id).await?;
                if let Some(placement) = self.store.placement(&fork.session_id)? {
                    project = placement.project;
                    item = placement.item;
                }
                // Or onto the item asked for, reading its specs.
                if let Some(onto) = convert::optional(std::mem::take(&mut fork.item_id)) {
                    project = Some(
                        self.fork_onto(fork, &host, &onto, project.as_deref())
                            .await?,
                    );
                    item = Some(onto);
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
                return Ok(Reply::Response(Response::Hosts(pb::Hosts {
                    hosts,
                    coordinator: Some(self.coordinator()),
                })));
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
                let update = std::mem::take(update);
                let host = if update.name.is_empty() {
                    self.update_coordinator(update)?
                } else {
                    self.update_host(update).await?
                };
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
            Request::ListRecoveries(_) => {
                let recoveries = self.recoveries().await?;
                return Ok(Reply::Response(Response::Recoveries(recoveries)));
            }
            Request::RestoreSession(restore) => {
                let session = self
                    .restore_session(&restore.session_id, &restore.host)
                    .await?;
                return Ok(Reply::Response(Response::SessionRestored(
                    pb::SessionRestored {
                        session: Some(session),
                    },
                )));
            }
            Request::RestoreHost(restore) => {
                let restored = self.restore_host(&restore.host).await?;
                return Ok(Reply::Response(Response::HostRestored(restored)));
            }
            Request::ListSshHosts(_) => {
                return Ok(Reply::Response(Response::SshHosts(pb::SshHosts {
                    names: ssh_config_hosts(),
                })));
            }
            Request::ConfigureHost(_)
            | Request::ReadHostStatus(_)
            | Request::CloneRepositories(_)
            | Request::WatchToolCalls(_)
            | Request::AnswerToolCall(_)
            | Request::StopHost(_)
            | Request::BackupSession(_)
            | Request::RestoreBackup(_)
            | Request::ConnectPort(_)
            | Request::WritePort(_)
            | Request::ClosePort(_)
            | Request::SetSpecs(_) => {
                return Err(DaemonError::ForHostsOnly);
            }
            Request::ForwardPort(forward) => {
                return self.forward_port(std::mem::take(forward)).await;
            }
            Request::ListForwards(_) => {
                return Ok(Reply::Response(Response::Forwards(pb::Forwards {
                    forwards: self.forwards_list(),
                })));
            }
            Request::StopForward(stop) => {
                self.stop_forward(&stop.id)?;
                return Ok(Reply::Response(Response::ForwardStopped(
                    pb::ForwardStopped {},
                )));
            }
            Request::WatchSession(watch) => {
                let host = self.host_of(&watch.session_id).await?;
                // A host that restarted has lost its copy of the specs.
                if self
                    .store
                    .placement(&watch.session_id)?
                    .is_some_and(|placed| placed.item.is_some())
                {
                    let _ = self.push_specs(&watch.session_id, &host).await;
                }
                return self.watch(&host, std::mem::take(watch)).await;
            }
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
            Request::OpenTerminal(open) => {
                let host = if open.session_id.is_empty() {
                    open.host.clone()
                } else {
                    self.host_of(&open.session_id).await?
                };
                return self.terminals(&host, request).await;
            }
            Request::AttachTerminal(attach) => {
                let host = attach.host.clone();
                return self.forward(&host, request).await;
            }
            Request::TerminalInput(input) => input.host.clone(),
            Request::ResizeTerminal(resize) => resize.host.clone(),
            Request::CloseTerminal(close) => close.host.clone(),
            Request::ListTerminals(list) => {
                let host = list.host.clone();
                return self.terminals(&host, request).await;
            }
            Request::ListFiles(list) => self.host_of(&list.session_id).await?,
            Request::ReadFile(read) if read.session_id.is_empty() => read.host.clone(),
            Request::ReadFile(read) => self.host_of(&read.session_id).await?,
            Request::ListFolder(list) if list.session_id.is_empty() => list.host.clone(),
            Request::ListFolder(list) => self.host_of(&list.session_id).await?,
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
        // What happened outside Humata isn't news: it was read there.
        let imported = matches!(request, Request::ImportSession(_));
        let planning = matches!(&request, Request::StartSession(start) if start.planning);
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
            if planning {
                self.store
                    .place(&session.id, &host, project.as_deref(), item.as_deref())?;
                self.store.set_planning(&session.id)?;
            }
            self.own(session, &host, project.as_deref(), item.as_deref())?;
            if item.is_some() {
                // Its agent reads them there while the Mac is away; one
                // that can't have them reads them from here.
                let _ = self.push_specs(&session.id, &host).await;
            }
            if imported {
                self.store.mark_seen(&session.id, session.turns)?;
                session.seen_turns = session.turns;
            }
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

/// Answers hosts' agents' tool calls, each on its own task, until the
/// daemon is dropped.
async fn answer_tool_calls(daemon: Weak<Daemon>, mut calls: mpsc::UnboundedReceiver<ToolCall>) {
    while let Some(call) = calls.recv().await {
        let Some(daemon) = daemon.upgrade() else {
            return;
        };
        tokio::spawn(async move {
            let response = match daemon.tool_call(&call.host, call.request).await {
                Ok(response) => response,
                Err(failure) => error(failure.code(), describe(&failure)),
            };
            let answer = pb::AnswerToolCall {
                id: call.id,
                response: Some(pb::Response {
                    kind: Some(response),
                }),
            };
            // A host gone since has no agent left waiting.
            let _ = call
                .client
                .call(pb::request::Kind::AnswerToolCall(answer))
                .await;
        });
    }
}

impl Daemon {
    /// Answers a tool call from an agent on `host`, which reaches only the
    /// sessions there and the boards of their projects.
    pub(crate) async fn tool_call(
        &self,
        host: &str,
        request: Option<pb::request::Kind>,
    ) -> Result<pb::response::Kind, DaemonError> {
        use pb::request::Kind as Request;
        let out_of_scope = |asked| DaemonError::OutOfScope {
            host: host.to_owned(),
            asked,
        };
        let mut sessions = self.list("").await?;
        sessions.sessions.retain(|session| session.host == host);
        let in_project = |project: &str| {
            !project.is_empty()
                && sessions
                    .sessions
                    .iter()
                    .any(|session| session.project_id == project)
        };
        let allowed = match &request {
            Some(Request::ListSessions(_)) => {
                return Ok(pb::response::Kind::Sessions(sessions));
            }
            Some(Request::ListItems(list)) => in_project(&list.project_id),
            Some(Request::CreateItem(create)) => in_project(&create.project_id),
            Some(Request::UpdateItem(update)) => self
                .store
                .item(&update.item_id)
                .is_ok_and(|item| in_project(&item.project_id)),
            Some(Request::ListThreads(list)) => sessions
                .sessions
                .iter()
                .any(|session| session.id == list.session_id),
            Some(Request::AskPerson(ask)) => sessions
                .sessions
                .iter()
                .any(|session| session.id == ask.session_id),
            Some(Request::RequestMove(asked)) => sessions
                .sessions
                .iter()
                .any(|session| session.id == asked.session_id),
            _ => return Err(out_of_scope("a request that isn't a humata tool's")),
        };
        let Some(request) = request.filter(|_| allowed) else {
            return Err(out_of_scope("another project's board or session"));
        };
        match self.answer(request).await? {
            Reply::Response(response) => Ok(response),
            Reply::Stream { .. } => Err(out_of_scope("a stream")),
        }
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

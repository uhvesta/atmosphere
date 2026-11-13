//! The daemon role: one place the app and the CLI talk to, which runs
//! sessions on this Mac or passes them to a host it reaches over a command's
//! stdin and stdout (in practice `ssh <host> humatad host --stdio`).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock, Weak};

use humata_wire::{ClientError, Reply, Service, ServiceFuture, describe, error, pb, streams};
use tokio::sync::mpsc;

use crate::board::{self, Author, Item, ItemFields, ItemPatch, Priority, Status};
use crate::convert;
use crate::error::DaemonError;
use crate::error::StoreError;
use crate::host::Host;
use crate::hosts::{self, HostConfig, Hosts, Remote, Ssh, ToolCall};
use crate::store::{Placement, Store};

/// How long a watch whose host dropped waits before watching again, at
/// first and at most.
const REWATCH_FIRST_WAIT: std::time::Duration = std::time::Duration::from_secs(1);
const REWATCH_LONGEST_WAIT: std::time::Duration = std::time::Duration::from_secs(30);
/// How long a list waits for each host before showing it as it last was.
const HOST_LIST_WAIT: std::time::Duration = std::time::Duration::from_secs(3);
/// How long a host's listing of its sessions is given to every caller
/// after it was made, as the app, the board and search list at once.
const LISTING_KEPT: std::time::Duration = std::time::Duration::from_secs(1);

/// A host's listing of its sessions: None until it's made, then when it
/// was and what the host said, None if it didn't in time.
type Listed = Option<(std::time::Instant, Option<pb::Sessions>)>;

/// A host's listing of its sessions, shared by every caller meanwhile.
struct Listing {
    answer: tokio::sync::watch::Receiver<Listed>,
}
/// The least time between copies off a host made for saves part way
/// through a turn.
const SAVE_BACKUP_EVERY: std::time::Duration = std::time::Duration::from_secs(60);

mod approvals;
pub use approvals::Connection;
mod forwards;
mod planning;
mod pulls;
mod recovery;
mod repositories;
pub use repositories::Clones;
pub use storage::Places;
mod plugins;
mod review;
mod search;
mod skills;
mod storage;

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
    /// Where the Mac's clones of repositories added by their URLs go, and
    /// where older daemons put them, which are still used.
    clones: Clones,
    /// The sessions being backed up, each with whether to back it up again
    /// once done.
    backing_up: std::sync::Mutex<HashMap<String, bool>>,
    /// Backups that failed, left alone for a while.
    backup_failures: std::sync::Mutex<recovery::BackupFailures>,
    /// The ports forwarded to hosts, by id.
    forwards: std::sync::Mutex<BTreeMap<String, forwards::Forwarding>>,
    next_forward: std::sync::atomic::AtomicU64,
    /// Held while the search index catches up, so two searches don't index
    /// one session's events twice.
    indexing: tokio::sync::Mutex<()>,
    /// Told when the search index caught up, while it catches up in the
    /// background, so a search waits on that one instead of starting another.
    catching_up: std::sync::Mutex<Option<tokio::sync::watch::Receiver<bool>>>,
    /// Each host's listing of its sessions, under way or just made, shared
    /// by every caller meanwhile.
    listings: std::sync::Mutex<HashMap<String, Listing>>,
    /// Told when an approval is asked for or decided.
    approvals_changed: tokio::sync::Notify,
    /// Where its files are on the Mac, as Settings › Storage shows them.
    places: storage::Places,
    cleaning: std::sync::Mutex<storage::Cleaning>,
}

impl Daemon {
    /// The daemon, answering its hosts' agents' tool calls from now on.
    pub fn new(
        local: Host,
        store: Store,
        hosts: Hosts,
        hosts_file: PathBuf,
        recovery: PathBuf,
        clones: Clones,
        places: storage::Places,
        build: String,
    ) -> Arc<Self> {
        let (tool_calls, calls) = mpsc::unbounded_channel();
        local.set_env(hosts.coordinator_env.clone());
        let coordinator_env = std::sync::Mutex::new(hosts.coordinator_env);
        let model_server = local.model_server();
        let remotes = hosts
            .hosts
            .into_iter()
            .map(|(name, config)| {
                let remote = Remote::new(
                    name.clone(),
                    config,
                    tool_calls.clone(),
                    model_server.clone(),
                );
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
            clones,
            backing_up: std::sync::Mutex::default(),
            backup_failures: std::sync::Mutex::default(),
            forwards: std::sync::Mutex::default(),
            next_forward: std::sync::atomic::AtomicU64::new(0),
            indexing: tokio::sync::Mutex::new(()),
            catching_up: std::sync::Mutex::default(),
            listings: std::sync::Mutex::default(),
            approvals_changed: tokio::sync::Notify::new(),
            places,
            cleaning: std::sync::Mutex::default(),
        });
        if let Err(error) = daemon.interrupt_unfinished_approvals() {
            eprintln!(
                "humatad: couldn't mark the admin actions cut short when it last stopped: {}",
                describe(&error)
            );
        }
        tokio::spawn(answer_tool_calls(Arc::downgrade(&daemon), calls));
        tokio::spawn(pulls::follow(Arc::downgrade(&daemon)));
        daemon
    }

    /// Points local-model sessions here and on the hosts at the app's
    /// model server on `port`. A host reached before forwards the old
    /// port, so a change reconnects it for the new forward, once no turn
    /// or terminal there would be cut off by it.
    async fn set_model_server(&self, port: u32) -> Result<(), DaemonError> {
        let port = u16::try_from(port)
            .ok()
            .filter(|port| *port != 0)
            .ok_or(DaemonError::ModelServerPort { port })?;
        let changed = self.local.model_server().port() != Some(port);
        self.local.set_model_server(port);
        if changed {
            let remotes: Vec<_> = self
                .remotes
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .values()
                .cloned()
                .collect();
            for remote in remotes {
                tokio::spawn(async move { remote.reconnect_when_idle().await });
            }
        }
        Ok(())
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
                Arc::new(Remote::new(
                    name,
                    config,
                    self.tool_calls.clone(),
                    self.local.model_server(),
                )),
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
                url: match project.urls.get(&repository.name) {
                    Some(url) => url.clone(),
                    None => origin(host, &repository.path).await?,
                },
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
        let known = self.store.known(&session.id)?;
        self.own_known(session, host, project, item, known)
    }

    /// [`Daemon::own`], given what the daemon knows of `session`, read
    /// before.
    fn own_known(
        &self,
        session: &mut pb::SessionInfo,
        host: &str,
        project: Option<&str>,
        item: Option<&str>,
        known: Option<crate::store::Known>,
    ) -> Result<(), DaemonError> {
        // Sessions are listed every two seconds and are seldom placed anew,
        // so the placement is written only when it would change.
        let placed = known.as_ref().is_some_and(|known| {
            known.placement.host == host
                && (project.is_none() || known.placement.project.is_some())
                && (item.is_none() || known.placement.item.is_some())
        });
        let known = if placed {
            known
        } else {
            self.store.place(&session.id, host, project, item)?;
            self.store.known(&session.id)?
        };
        session.host = host.to_owned();
        let Some(known) = known else {
            return Ok(());
        };
        session.project_id = known.placement.project.unwrap_or_default();
        // One deleted since shows no title.
        session.item_title = known.item_title.unwrap_or_default();
        session.item_id = known.placement.item.unwrap_or_default();
        session.seen_turns = known.placement.seen_turns;
        session.planning = known.planning;
        session.asking = known.asking;
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
        // Their titles and statuses only, not their specs.
        let mut shown: Vec<pb::Item> = self.store.item_heads()?.iter().map(Item::wire).collect();
        board::show(&mut shown, &sessions);
        let items: HashMap<String, pb::Item> = shown
            .into_iter()
            .map(|item| (item.id.clone(), item))
            .collect();
        let names: HashMap<&str, &str> = projects
            .iter()
            .map(|project| (project.id.as_str(), project.name.as_str()))
            .collect();
        let requests = self.store.open_requests()?;
        let asking: HashSet<&str> = requests
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
        // Within a rank, what has waited longest comes first.
        entries.sort_by_key(|entry| {
            let since = entry
                .session
                .as_ref()
                .map_or(0, |session| session.last_active_unix_ms);
            (rank(entry), since)
        });
        Ok(pb::Inbox {
            entries,
            approvals: self.pending_approvals()?,
        })
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
        Ok(self.store.add_request(
            pb::AgentRequest {
                session_id: ask.session_id,
                kind: pb::AgentRequestKind::Question.into(),
                text: text.to_owned(),
                ..Default::default()
            },
            &ask.client_id,
        )?)
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
        let item_id = match (asked.item_id.as_str(), &placement.item) {
            ("", Some(placed)) => placed.as_str(),
            ("", None) => return Err(DaemonError::NoTaskToMove),
            (given, _) => given,
        };
        let item = self.store.item(item_id)?;
        if placement.project.as_deref() != Some(item.project_id.as_str()) {
            return Err(StoreError::ItemInAnotherProject {
                title: item.fields.title,
            }
            .into());
        }
        Ok(self.store.add_request(
            pb::AgentRequest {
                session_id: asked.session_id,
                kind: pb::AgentRequestKind::Move.into(),
                text: summary.to_owned(),
                item_id: item.id,
                status: status.into(),
                ..Default::default()
            },
            &asked.client_id,
        )?)
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
                // The session asked for it; the journal says so.
                by_session_id: request.session_id.clone(),
                ..Default::default()
            })
        };
        let doing = match answer.answer {
            Some(Answer::Reply(text)) => {
                let text = text.trim();
                if text.is_empty() {
                    return Err(DaemonError::NoReply);
                }
                // Straight to the session: its other questions stay open.
                // Named by the request, so answering it again, as after a
                // crash before it was marked answered, sends it once.
                let reply = self
                    .send_message(pb::SendMessage {
                        session_id: request.session_id.clone(),
                        text: text.to_owned(),
                        client_id: format!("answer-{}", request.id),
                        ..Default::default()
                    })
                    .await?;
                if let Some(refused) = refusal(&reply) {
                    return Err(refused);
                }
                None
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
            let reply = Box::pin(self.answer(doing)).await?;
            if let Some(refused) = refusal(&reply) {
                return Err(refused);
            }
        }
        Ok(self.store.answer_request(&request.id)?)
    }

    /// Sends `message` to its session with the spec notes kept for it, which
    /// are forgotten only once its host took the message.
    async fn send_message(&self, mut message: pb::SendMessage) -> Result<Reply, DaemonError> {
        let host = self.host_of(&message.session_id).await?;
        let notes = self.store.spec_notes(&message.session_id)?;
        if !notes.is_empty() {
            let told: Vec<&str> = notes.iter().map(|(_, note)| note.as_str()).collect();
            message.text = format!("{}\n\n{}", told.join("\n\n"), message.text);
        }
        let session = message.session_id.clone();
        let reply = self
            .forward(&host, pb::request::Kind::SendMessage(message))
            .await?;
        if refusal(&reply).is_none() && !notes.is_empty() {
            self.store.clear_spec_notes(&session, &notes)?;
        }
        Ok(reply)
    }

    fn create_item(&self, create: pb::CreateItem) -> Result<Item, DaemonError> {
        let author = Author::on(create.surface());
        let fields = ItemFields {
            parent_id: convert::optional(create.parent_id.clone()),
            status: Status::from_wire(create.status()),
            priority: Priority::from_wire(create.priority()),
            title: create.title,
            spec: create.spec,
        };
        Ok(self.store.add_item(&create.project_id, fields, &author)?)
    }

    fn update_item(&self, update: pb::UpdateItem) -> Result<Item, DaemonError> {
        let author = Author::session(&update.by_session_id, update.surface());
        let patch = ItemPatch {
            status: update.status.map(|_| Status::from_wire(update.status())),
            priority: update
                .priority
                .map(|_| Priority::from_wire(update.priority())),
            title: update.title,
            spec: update.spec,
        };
        Ok(self.store.update_item(&update.item_id, patch, &author)?)
    }

    /// Deletes item `id`, refused while sessions on it aren't archived. Its
    /// archived sessions move to its project.
    async fn delete_item(&self, id: &str, author: &Author) -> Result<(), DaemonError> {
        let item = self.store.item(id)?;
        let on_it: Vec<pb::SessionInfo> = self
            .list(&item.project_id)
            .await?
            .sessions
            .into_iter()
            .filter(|session| session.item_id == id)
            .collect();
        let live = on_it.iter().filter(|session| !session.archived).count();
        if live > 0 {
            return Err(DaemonError::ItemHasSessions {
                title: item.fields.title,
                live,
            });
        }
        let archived: Vec<String> = on_it.into_iter().map(|session| session.id).collect();
        Ok(self.store.delete_item(id, author, &archived)?)
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

    /// Reconnects `host` when it waits to be and none of its `sessions` is
    /// busy any more, which updates it if it's outdated.
    fn update_once_idle(
        &self,
        host: &str,
        sessions: &[pb::SessionInfo],
    ) -> Result<(), DaemonError> {
        let remote = self.remote(host)?;
        let busy = sessions.iter().any(|session| {
            matches!(
                session.state(),
                pb::SessionState::Working | pb::SessionState::WaitingForApproval
            )
        });
        if remote.reconnect_pending() && !busy {
            tokio::spawn(async move { remote.reconnect_when_idle().await });
        }
        Ok(())
    }

    /// `host`'s listing of its sessions: the one under way or made in the
    /// last [`LISTING_KEPT`], or a new one. The listing gives up at
    /// [`HOST_LIST_WAIT`], and keeps what it got as the host's sessions.
    fn listing(&self, host: &str) -> Result<tokio::sync::watch::Receiver<Listed>, DaemonError> {
        let mut listings = self.listings.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(listing) = listings.get(host) {
            let fresh = match &*listing.answer.borrow() {
                None => true,
                Some((at, _)) => at.elapsed() < LISTING_KEPT,
            };
            if fresh {
                return Ok(listing.answer.clone());
            }
        }
        let remote = self.remote(host)?;
        let (hosts_file, build) = (self.hosts_file.clone(), self.build.clone());
        let store = self.store.clone();
        let named = host.to_owned();
        let (told, answer) = tokio::sync::watch::channel(None);
        tokio::spawn(async move {
            let deadline = tokio::time::Instant::now() + HOST_LIST_WAIT;
            // Connecting goes on past the deadline, so the next listing
            // may have the host; asking it doesn't.
            let connecting = {
                let remote = remote.clone();
                tokio::spawn(async move { remote.client(&hosts_file, &build).await })
            };
            let asked = async {
                let client = connecting.await.ok()?.ok()?;
                let request = pb::request::Kind::ListSessions(pb::ListSessions::default());
                match remote.call(&client, request).await.ok()? {
                    pb::response::Kind::Sessions(sessions) => Some(sessions),
                    _ => None,
                }
            };
            let sessions = tokio::time::timeout_at(deadline, asked)
                .await
                .ok()
                .flatten();
            if let Some(sessions) = &sessions {
                let (kept, host) = (sessions.sessions.clone(), named.clone());
                if let Err(error) = store
                    .apart(move |store| store.cache_sessions(&host, &kept))
                    .await
                {
                    eprintln!(
                        "humatad: couldn't keep {named}'s sessions for while it's offline: {}",
                        describe(&error)
                    );
                }
            }
            told.send_replace(Some((std::time::Instant::now(), sessions)));
        });
        listings.insert(
            host.to_owned(),
            Listing {
                answer: answer.clone(),
            },
        );
        Ok(answer)
    }

    /// Drops `host`'s kept listing, so the next one asks it again.
    fn forget_listing(&self, host: &str) {
        if host.is_empty() {
            return;
        }
        self.listings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(host);
    }

    async fn list(&self, project: &str) -> Result<pb::Sessions, DaemonError> {
        let mut all = Vec::new();
        // Every host at once, each given a little while: a slow one shows
        // as it last was, offline, and its connection goes on meanwhile,
        // so the next list may have it.
        let mut asking = Vec::new();
        for host in self.remote_names() {
            let listing = self.listing(&host)?;
            asking.push((host, listing));
        }
        let deadline = tokio::time::Instant::now() + HOST_LIST_WAIT;
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
        for (host, mut listing) in asking {
            let answer = tokio::time::timeout_at(deadline, listing.wait_for(Option::is_some))
                .await
                .ok()
                .and_then(Result::ok)
                .and_then(|listed| listed.as_ref().and_then(|(_, sessions)| sessions.clone()));
            answers.push((host, answer.map(pb::response::Kind::Sessions)));
        }
        let mut listed = Vec::new();
        for (host, answer) in answers {
            let sessions = match answer {
                Some(pb::response::Kind::Sessions(sessions)) => {
                    if !host.is_empty() {
                        self.update_once_idle(&host, &sessions.sessions)?;
                    }
                    Some(sessions.sessions)
                }
                _ if host.is_empty() => Some(Vec::new()),
                // Shown as it last was, read with the rest below.
                _ => None,
            };
            listed.push((host, sessions));
        }
        // What the daemon knows of them, read at once for all of them and
        // off the executor, as the app lists every two seconds.
        let (listed, known, mut moved) = self
            .store
            .apart(move |store| {
                let mut all = Vec::with_capacity(listed.len());
                let mut moved = HashMap::new();
                for (host, sessions) in listed {
                    let sessions = match sessions {
                        Some(sessions) => sessions,
                        None => {
                            let mut cached = store.cached_sessions(&host)?;
                            for session in &mut cached {
                                session.host_offline = true;
                            }
                            cached
                        }
                    };
                    moved.insert(host.clone(), store.moved_away_from(&host)?);
                    all.push((host, sessions));
                }
                let ids: Vec<&str> = all
                    .iter()
                    .flat_map(|(_, sessions)| sessions.iter().map(|session| session.id.as_str()))
                    .collect();
                let known = store.known_many(&ids)?;
                Ok((all, known, moved))
            })
            .await?;
        for (host, sessions) in listed {
            let moved = moved.remove(&host).unwrap_or_default();
            for mut session in sessions {
                if moved.contains(&session.id) {
                    continue;
                }
                if !host.is_empty() && !session.host_offline {
                    self.back_up_soon(&host, &session.id, Some(session.turns));
                }
                let was = known.get(&session.id).cloned();
                self.own_known(&mut session, &host, None, None, was)?;
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
                // Kept again only when it has changed since, and written
                // apart: compressing a long history takes a while.
                if !host.is_empty()
                    && self.store.cached_history_len(&session)? != history.events.len()
                {
                    let kept = history.clone();
                    self.store
                        .apart(move |store| store.cache_history(&session, &kept))
                        .await?;
                }
                Ok(Reply::Response(pb::response::Kind::History(history)))
            }
            Err(failure) if failure.code() == pb::ErrorCode::HostUnavailable => {
                // Decompressed apart: a long history takes a while.
                let kept = self
                    .store
                    .apart(move |store| store.cached_history(&session))
                    .await?;
                match kept {
                    Some(history) => Ok(Reply::Response(pb::response::Kind::History(history))),
                    None => Err(failure),
                }
            }
            other => other,
        }
    }

    async fn create_project(&self, create: pb::CreateProject) -> Result<pb::Project, DaemonError> {
        let (repositories, urls) = self.resolve_repositories(create.repositories).await?;
        let project = self.store.add_project(
            create.name,
            repositories,
            urls,
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
            if start.host.is_empty() {
                start.notices = self.refresh(&project).await;
                start.repositories = project
                    .repositories
                    .iter()
                    .map(convert::wire_repository)
                    .collect();
            } else {
                let cloned = self.clone_on(&start.host, &project).await?;
                start.notices = cloned.notices;
                start.repositories = cloned.repositories;
            }
        }
        let parent = match (
            item.as_ref()
                .and_then(|item| item.fields.parent_id.as_ref()),
            start.specs(),
        ) {
            (Some(parent), pb::SpecScope::WithParents) => Some(self.store.item(parent)?),
            _ => None,
        };
        let skills = self
            .session_skills(
                &project,
                &start.host,
                &start.repositories,
                &start.working_directory,
            )
            .await?;
        start.instructions = planning::instructions(
            &project,
            &start.instructions,
            &skills,
            item.as_ref(),
            parent.as_ref(),
        );
        start.plugins.clone_from(&project.plugins);
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
                let kept = {
                    let session = session.clone();
                    self.store
                        .apart(move |store| store.cached_history(&session))
                        .await?
                };
                let Some(history) = kept else {
                    return Err(failure);
                };
                let (items, receiver) = mpsc::channel(history.events.len() + 1);
                for event in history.events {
                    let item = pb::StreamItem {
                        kind: Some(pb::stream_item::Kind::SessionEvent(event)),
                    };
                    // The channel holds every event, so this can't wait.
                    let _ = items.try_send(Ok(item));
                }
                // The copy is all there is until the host is back.
                let _ = items.try_send(Ok(pb::StreamItem {
                    kind: Some(pb::stream_item::Kind::HistoryLoaded(pb::HistoryLoaded {})),
                }));
                return Ok(Reply::Stream {
                    response: pb::response::Kind::Watching(pb::Watching {}),
                    items: receiver,
                });
            }
            other => return other,
        };
        let store = self.store.clone();
        // A history grows by a turn at a time, so a copy is written only
        // when it has more than the last, never part of the replay over a
        // fuller one.
        let mut kept = self.store.cached_history_len(&session)?;
        // The events the host sends first that this Mac already has: their
        // saves and turns were backed up when they happened, so replaying
        // them must not send the whole session over again.
        let known = kept;
        let (items, receiver) = mpsc::channel(64);
        let (daemon, host) = (self.me.clone(), host.to_owned());
        tokio::spawn(async move {
            // The history so far, without the parts of replies since
            // made whole, as the host keeps it.
            let mut events: Vec<pb::SessionEvent> = Vec::new();
            // Where the parts of the reply being streamed start in `events`.
            let mut streaming: Option<usize> = None;
            // Every event passed on, so a replay after re-watching passes on
            // only those not sent before. A history drops the parts of a
            // reply once it is whole, so it can't be skipped by count.
            let mut sent = HashSet::new();
            // Saves part way through a turn come after each change; their
            // copies off the host are made at most once a minute, and the
            // turn's end makes one anyway.
            let mut backed_up_save: Option<std::time::Instant> = None;
            // Copies of the history are compressed and written on a thread
            // for blocking work, so a long one doesn't hold up an executor
            // thread; one at a time, in order, skipping to the newest when
            // several wait. It stops when the watch does.
            let (copies, mut waiting) = mpsc::unbounded_channel::<pb::History>();
            let writer = (store.clone(), session.clone());
            tokio::task::spawn_blocking(move || {
                let (store, session) = writer;
                while let Some(mut history) = waiting.blocking_recv() {
                    while let Ok(newer) = waiting.try_recv() {
                        history = newer;
                    }
                    if let Err(error) = store.cache_history(&session, &history) {
                        eprintln!("humatad: couldn't keep a copy of {session}'s history: {error}");
                    }
                }
            });
            let mut keep = |events: &Vec<pb::SessionEvent>| {
                if events.len() <= kept {
                    return;
                }
                let history = pb::History {
                    events: events.clone(),
                };
                // Compressed and written by the writer apart. One that
                // couldn't be written stays as it was; the next turn's copy
                // has it all.
                kept = events.len();
                let _ = copies.send(history);
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
                            continue;
                        }
                        None => break,
                    }
                };
                if let Ok(pb::StreamItem {
                    kind: Some(pb::stream_item::Kind::SessionEvent(event)),
                }) = &item
                {
                    if !sent.insert(event.id.clone()) {
                        continue;
                    }
                    match event.kind {
                        Some(pb::session_event::Kind::AssistantDelta(_)) => {
                            streaming.get_or_insert(events.len());
                        }
                        Some(
                            pb::session_event::Kind::AssistantMessage(_)
                            | pb::session_event::Kind::TurnEnded(_),
                        ) => {
                            if let Some(from) = streaming.take() {
                                let tail = events.split_off(from);
                                events.extend(tail.into_iter().filter(|kept| {
                                    let part = matches!(
                                        kept.kind,
                                        Some(pb::session_event::Kind::AssistantDelta(_))
                                    );
                                    // The host's history drops the parts of
                                    // a whole reply, so a replay never sends
                                    // them again: they needn't be kept.
                                    if part {
                                        sent.remove(&kept.id);
                                    }
                                    !part
                                }));
                            }
                        }
                        _ => {}
                    }
                    events.push(event.clone());
                    // Once a turn, at its end: its checkpoint comes just
                    // before, and a copy cut short of it is kept at the
                    // watch's end.
                    if matches!(event.kind, Some(pb::session_event::Kind::Idle(_))) {
                        keep(&events);
                    }
                    let replayed = events.len() <= known;
                    if !replayed
                        && matches!(event.kind, Some(pb::session_event::Kind::Saved(_)))
                        && backed_up_save.is_none_or(|at| at.elapsed() >= SAVE_BACKUP_EVERY)
                    {
                        backed_up_save = Some(std::time::Instant::now());
                        if let Some(daemon) = daemon.upgrade() {
                            daemon.back_up_soon(&host, &session, None);
                        }
                    }
                    if !replayed && matches!(event.kind, Some(pb::session_event::Kind::Idle(_))) {
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
                let project = self.create_project(std::mem::take(create)).await?;
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
                let by = update.by_session_id.clone();
                let moved = update.status.is_some() && by.is_empty();
                let before = self.store.item(&update.item_id)?.fields.spec;
                let updated = self.update_item(std::mem::take(update))?;
                if moved {
                    // The person moved it by hand, which answers the moves
                    // agents asked for.
                    self.store.answer_moves(&updated.id)?;
                }
                if updated.fields.spec != before {
                    self.spec_changed(&updated, &by).await;
                }
                let item = self.shown_item(&updated).await?;
                return Ok(Reply::Response(Response::ItemUpdated(pb::ItemUpdated {
                    item: Some(item),
                })));
            }
            Request::DeleteItem(delete) => {
                self.delete_item(&delete.item_id, &Author::on(delete.surface()))
                    .await?;
                return Ok(Reply::Response(Response::ItemDeleted(pb::ItemDeleted {})));
            }
            Request::ListItems(list) => {
                let items = self.shown_items(&list.project_id).await?;
                return Ok(Reply::Response(Response::Items(pb::Items { items })));
            }
            Request::ListProposals(list) => {
                let proposals = if list.session_id.is_empty() {
                    self.outside_proposals(&list.project_id)?
                } else {
                    self.proposals(&list.session_id).await?
                };
                return Ok(Reply::Response(Response::Proposals(pb::Proposals {
                    proposals,
                })));
            }
            Request::Propose(propose) => {
                let proposal = self.propose(std::mem::take(propose))?;
                return Ok(Reply::Response(Response::Proposed(pb::Proposed {
                    proposal: Some(proposal),
                })));
            }
            Request::ApplyProposal(apply) => {
                let applied = self.apply_proposal(std::mem::take(apply)).await?;
                return Ok(Reply::Response(Response::ProposalApplied(applied)));
            }
            Request::WaitForApproval(wait) => {
                let approval = self
                    .wait_for_approval(
                        &wait.id,
                        std::time::Duration::from_secs(wait.wait_seconds.into()),
                    )
                    .await?;
                return Ok(Reply::Response(Response::ApprovalStatus(Box::new(
                    pb::ApprovalStatus {
                        approval: Some(Box::new(approval)),
                    },
                ))));
            }
            Request::ListSkills(_) => {
                return Ok(Reply::Response(Response::SkillList(
                    self.list_skills().await?,
                )));
            }
            Request::PickSkill(pick) => {
                self.pick_skill(pick)?;
                return Ok(Reply::Response(Response::SkillPicked(pb::SkillPicked {})));
            }
            Request::ListPlugins(_) => {
                return Ok(Reply::Response(Response::PluginList(
                    self.list_plugins().await?,
                )));
            }
            Request::GetSessionDefaults(_) => {
                return Ok(Reply::Response(self.get_session_defaults()?));
            }
            Request::SetSessionDefaults(set) => {
                return Ok(Reply::Response(self.set_session_defaults(set)?));
            }
            Request::SetProjectPlugin(set) => {
                let project = self.set_project_plugin(set)?;
                return Ok(Reply::Response(Response::ProjectUpdated(
                    pb::ProjectUpdated {
                        project: Some(project),
                    },
                )));
            }
            Request::ListApprovals(_) => {
                return Ok(Reply::Response(Response::Approvals(self.list_approvals()?)));
            }
            Request::DecideApprovals(decide) => {
                let decided = self.decide_approvals(std::mem::take(decide)).await?;
                return Ok(Reply::Response(Response::ApprovalsDecided(decided)));
            }
            Request::RevokeGrant(revoke) => {
                self.revoke_grant(&revoke.id)?;
                return Ok(Reply::Response(Response::GrantRevoked(pb::GrantRevoked {})));
            }
            Request::GetStorage(_) => {
                return Ok(Reply::Response(Response::Storage(self.storage().await?)));
            }
            Request::SetStorageSettings(set) => {
                self.set_storage_settings(&set.settings.clone().unwrap_or_default())?;
                return Ok(Reply::Response(Response::Storage(self.storage().await?)));
            }
            Request::CleanUpStorage(clean) => {
                self.start_cleanup(std::mem::take(clean))?;
                return Ok(Reply::Response(Response::Storage(self.storage().await?)));
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
                let project = self.update_project(std::mem::take(update)).await?;
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
            Request::ReadSkill(read) => {
                let skill = self.read_skill(read)?;
                return Ok(Reply::Response(Response::Skill(skill)));
            }
            Request::Search(search) => {
                let results = self.search(std::mem::take(search), None).await?;
                return Ok(Reply::Response(Response::SearchResults(results)));
            }
            Request::SetModelServer(set) => {
                self.set_model_server(set.port).await?;
                return Ok(Reply::Response(Response::ModelServerSet(
                    pb::ModelServerSet {},
                )));
            }
            Request::LinkCommands(link) => {
                let linked = crate::commands::link_commands(link.check)?;
                return Ok(Reply::Response(Response::CommandsLinked(linked)));
            }
            Request::ReadBoardLog(read) => {
                let changes = self.store.board_log(&read.project_id)?;
                return Ok(Reply::Response(Response::BoardLog(pb::BoardLog {
                    changes: changes.iter().map(board::Change::wire).collect(),
                })));
            }
            Request::RevertBoardChange(revert) => {
                let before = self.store.changed_item(revert.change_id)?;
                let author = Author::on(revert.surface());
                let reverted = self.store.revert(revert.change_id, &author)?;
                let item = match reverted.item {
                    Some(item) => {
                        if before.is_some_and(|before| before.fields.spec != item.fields.spec) {
                            self.spec_changed(&item, "").await;
                        }
                        Some(self.shown_item(&item).await?)
                    }
                    None => None,
                };
                return Ok(Reply::Response(Response::BoardChangeReverted(
                    pb::BoardChangeReverted {
                        item,
                        kept: reverted.kept.into_iter().map(Into::into).collect(),
                    },
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
                Box::pin(self.refuse_on_device_on_host(start)).await?;
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
            Request::SendMessage(message) => {
                let session = message.session_id.clone();
                let reply = self.send_message(std::mem::take(message)).await?;
                if refusal(&reply).is_none() {
                    // Writing to it answers what it asked.
                    self.store.answer_questions(&session)?;
                }
                return Ok(reply);
            }
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
                    .await;
                self.forget_listing(&restore.host);
                let session = session?;
                return Ok(Reply::Response(Response::SessionRestored(
                    pb::SessionRestored {
                        session: Some(session),
                    },
                )));
            }
            Request::RestoreHost(restore) => {
                let restored = self.restore_host(&restore.host).await;
                self.forget_listing(&restore.host);
                let restored = restored?;
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
            | Request::ScanSkills(_)
            | Request::ScanPlugins(_)
            | Request::WatchToolCalls(_)
            | Request::AnswerToolCall(_)
            | Request::StopHost(_)
            | Request::BackupSession(_)
            | Request::RestoreBackup(_)
            | Request::ConnectPort(_)
            | Request::WritePort(_)
            | Request::ClosePort(_)
            | Request::SetSpecs(_)
            | Request::DeleteBranches(_) => {
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
            Request::RewindSession(rewind) => self.host_of(&rewind.session_id).await?,
            Request::UndoRewind(undo) => self.host_of(&undo.session_id).await?,
            Request::Unqueue(unqueue) => self.host_of(&unqueue.session_id).await?,
            Request::EditQueued(edit) => self.host_of(&edit.session_id).await?,
            Request::ReadImage(read) => self.host_of(&read.session_id).await?,
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
            Request::DeleteSession(delete) => {
                let deleted = self
                    .delete_session(&delete.session_id, delete.only_if_held)
                    .await?;
                return Ok(Reply::Response(Response::SessionDeleted(
                    pb::SessionDeleted { kept: !deleted },
                )));
            }
            Request::ReadChanges(read) => {
                let host = self.host_of(&read.session_id).await?;
                return self.read_changes(&host, std::mem::take(read)).await;
            }
            Request::EditChanges(edit) => self.host_of(&edit.session_id).await?,
            Request::LandSession(land) => return self.land(std::mem::take(land)).await,
            Request::UpdateFromBase(update) => self.host_of(&update.session_id).await?,
            Request::ListPullRequests(list) => {
                let pulls = self.list_pulls(list).await?;
                return Ok(Reply::Response(Response::PullRequests(pulls)));
            }
            Request::ImportIssues(import) => {
                let imported = self.import_issues(&import.project_id).await?;
                return Ok(Reply::Response(Response::IssuesImported(imported)));
            }
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
            Request::ListFileVersions(list) => self.host_of(&list.session_id).await?,
            Request::RestoreFileVersion(restore) => self.host_of(&restore.session_id).await?,
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
        let forked = matches!(request, Request::ForkSession(_));
        let planning = matches!(&request, Request::StartSession(start) if start.planning);
        let mut reply = self.forward(&host, request).await?;
        // What was sent may have changed the host's sessions.
        self.forget_listing(&host);
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
            if forked && !session.parent_id.is_empty() {
                self.store.copy_decisions(&session.parent_id, &session.id)?;
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
        let every = sessions.sessions.clone();
        sessions.sessions.retain(|session| session.host == host);
        let here = |id: &str| sessions.sessions.iter().any(|session| session.id == id);
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
            Some(Request::ReadSpec(read)) => here(&read.session_id),
            Some(Request::ReadSkill(read)) => here(&read.session_id),
            // A session another host runs, in a project of this host's, as
            // search finds them.
            Some(Request::ReadHistory(read)) => every
                .iter()
                .any(|session| session.id == read.session_id && in_project(&session.project_id)),
            Some(Request::Search(search)) => {
                if !search.project_id.is_empty() && !in_project(&search.project_id) {
                    return Err(out_of_scope("another project's sessions"));
                }
                let mut scope: Vec<String> = sessions
                    .sessions
                    .iter()
                    .map(|session| session.project_id.clone())
                    .filter(|project| !project.is_empty())
                    .collect();
                scope.sort();
                scope.dedup();
                let results = self.search(search.clone(), Some(scope)).await?;
                return Ok(pb::response::Kind::SearchResults(results));
            }
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

/// The host's refusal in `reply`, as an error.
fn refusal(reply: &Reply) -> Option<DaemonError> {
    match reply {
        Reply::Response(pb::response::Kind::Error(refused)) => Some(DaemonError::Refused {
            code: refused.code(),
            message: refused.message.clone(),
        }),
        _ => None,
    }
}

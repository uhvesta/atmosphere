//! [`Daemon`] over the daemon's gRPC services.

use std::sync::Arc;
use std::time::Duration;

use humata2_clients::model::{
    Agent, Answer, Archive, Capability, ChannelId, ForkKind, GrantId, ItemId,
    ItemStatus, MachineId, Recovery, RequestId, Restart, RuleSetting, Scope, SessionId,
    SessionOptions, SetupId, TransferId,
};
use humata2_clients::{Daemon, DaemonError, Event, Events, Reply, Workspace};
use humata2_grpc::humata::v1 as pb;

use crate::connection::{LiveDaemon, Shared, fresh_key, not_in_this_release};
use crate::{map, setup};

const PAGE_SIZE: u32 = 200;
const POLL: Duration = Duration::from_millis(100);

impl Shared {
    /// Waits for an operation to stop running; a failed or cancelled one is
    /// refused with the daemon's words.
    async fn finish(&self, mut operation: pb::Operation, what: &str) -> Result<pb::Operation, DaemonError> {
        while operation.state == pb::OperationState::Running as i32 {
            // On the client's runtime: the caller may be polled elsewhere,
            // and a tokio sleep needs the runtime from when it is made.
            let _ = self.spawn(async { tokio::time::sleep(POLL).await }).await;
            let Some(id) = operation.id.clone() else {
                break;
            };
            operation = self
                .operations()
                .get_operation(pb::GetOperationRequest { id: Some(id) })
                .await
                .map_err(|status| self.error(&status, what))?
                .into_inner();
        }
        if operation.state == pb::OperationState::Succeeded as i32 {
            return Ok(operation);
        }
        let reason = match operation.error {
            Some(error) if !error.fix.is_empty() => format!("{} {}", error.message, error.fix),
            Some(error) => error.message,
            None => "it didn't finish. Try again".to_owned(),
        };
        Err(DaemonError::Refused {
            what: what.to_owned(),
            reason,
        })
    }

    /// The daemon's last start: `None` while it isn't known yet (the
    /// daemon is still bringing sessions back, or couldn't be read), then
    /// the restart to show, if the start brought anything back or lost
    /// anything. A clean start with nothing to bring back shows none.
    async fn read_restart(&self) -> Option<Option<Restart>> {
        let info = self
            .daemon_info()
            .get_server_info(pb::GetServerInfoRequest {})
            .await
            .ok()?
            .into_inner();
        let last = info.last_start.as_ref()?;
        Some(map::restart(&self.machine, Some(last)).filter(|restart| {
            !restart.resumed.is_empty()
                || !restart.not_brought_back.is_empty()
                || last.restored_copy_taken_unix_ms != 0
        }))
    }

    /// Reads the restart report once if none is known yet: the daemon
    /// publishes it only after it has brought its sessions back, so the
    /// first session event after that is the cue. At most one read runs.
    pub(crate) fn check_restart(self: &Arc<Self>) {
        {
            let mut state = self.state();
            if state.restart_known || state.restart_reading {
                return;
            }
            state.restart_reading = true;
        }
        let shared = self.clone();
        self.spawn(async move {
            let restart = shared.read_restart().await;
            {
                let mut state = shared.state();
                state.restart_reading = false;
                state.restart_known |= restart.is_some();
            }
            if let Some(Some(restart)) = restart {
                shared.publish(Event::Restart(restart));
            }
        });
    }

    /// Shows a setup's operation to the app.
    fn publish_setup(&self, operation: &pb::Operation) {
        let id = operation.id.as_ref().map(|id| id.value.clone()).unwrap_or_default();
        let name = self.state().setup_names.get(&id).cloned().unwrap_or_default();
        self.publish(Event::ChannelSetup(setup::setup(&name, operation, &self.machine)));
    }

    /// Follows a channel setup until it stops running, pushing each change.
    /// A failed or cancelled one is refused with the daemon's words.
    async fn watch_setup(&self, operation: pb::Operation) -> Result<pb::Operation, DaemonError> {
        const WHAT: &str = "Setting up the channel";
        self.publish_setup(&operation);
        let mut last = operation;
        if last.state == pb::OperationState::Running as i32 {
            if let Some(id) = last.id.clone() {
                let opened = self
                    .operations()
                    .watch_operation(pb::WatchOperationRequest {
                        id: Some(id),
                        from_version: last.version,
                    })
                    .await
                    .map_err(|status| self.error(&status, WHAT))?;
                let mut stream = opened.into_inner();
                while last.state == pb::OperationState::Running as i32 {
                    match stream.message().await {
                        Ok(Some(update)) => {
                            self.publish_setup(&update);
                            last = update;
                        }
                        // The stream ended early: `finish` asks once more.
                        Ok(None) => break,
                        Err(status) => return Err(self.error(&status, WHAT)),
                    }
                }
            }
        }
        let done = self.finish(last, WHAT).await;
        if let Ok(operation) = &done {
            self.publish_setup(operation);
        }
        done
    }

    /// This machine, then the others the daemon lists (HO-3). A daemon that doesn't build the
    /// call yet lists only this machine.
    async fn load_machines(&self) -> Result<Vec<humata2_clients::model::Machine>, DaemonError> {
        let mut machines = vec![map::this_machine(self.machine.clone())];
        let mut token = String::new();
        loop {
            let reply = match humata2_grpc::humata::v1::machine_service_client::MachineServiceClient::new(
                self.wire.clone(),
            )
            .list_machines(pb::ListMachinesRequest {
                page: Some(pb::PageRequest {
                    page_size: PAGE_SIZE,
                    page_token: token,
                }),
            })
            .await
            {
                Ok(reply) => reply.into_inner(),
                Err(status) if status.code() == tonic::Code::Unimplemented => break,
                Err(status) => return Err(self.error(&status, "The machines")),
            };
            for listed in &reply.machines {
                let mapped = map::machine(listed);
                if mapped.id != self.machine
                    && mapped.kind != humata2_clients::model::MachineKind::ThisMac
                    && !machines.iter().any(|known| known.id == mapped.id)
                {
                    machines.push(mapped);
                }
            }
            token = reply.page.map(|page| page.next_page_token).unwrap_or_default();
            if token.is_empty() {
                break;
            }
        }
        Ok(machines)
    }

    async fn load(&self) -> Result<Workspace, DaemonError> {
        let mut channels = Vec::new();
        let mut token = String::new();
        loop {
            let reply = self
                .channels()
                .list_channels(pb::ListChannelsRequest {
                    page: Some(pb::PageRequest {
                        page_size: PAGE_SIZE,
                        page_token: token,
                    }),
                })
                .await
                .map_err(|status| self.error(&status, "The channels"))?
                .into_inner();
            channels.extend(reply.channels.iter().map(map::channel));
            token = reply.page.map(|page| page.next_page_token).unwrap_or_default();
            if token.is_empty() {
                break;
            }
        }
        let mut sessions = Vec::new();
        loop {
            let reply = self
                .sessions()
                .list_sessions(pb::ListSessionsRequest {
                    channel: None,
                    machine: None,
                    include_archived: false,
                    page: Some(pb::PageRequest {
                        page_size: PAGE_SIZE,
                        page_token: token,
                    }),
                })
                .await
                .map_err(|status| self.error(&status, "The sessions"))?
                .into_inner();
            for session in &reply.sessions {
                let tabs = match session.id.as_ref() {
                    Some(id) => self.refresh_tabs(id).await.unwrap_or_default(),
                    None => Vec::new(),
                };
                sessions.push(map::session(session, &tabs, &self.agents));
            }
            token = reply.page.map(|page| page.next_page_token).unwrap_or_default();
            if token.is_empty() {
                break;
            }
        }
        let requests = self.load_requests().await?;
        let ids: Vec<ChannelId> = channels.iter().map(|channel| channel.id.clone()).collect();
        let rules = self.load_rules(&ids).await?;
        let restart = self.read_restart().await;
        {
            let mut state = self.state();
            state.restart_known = restart.is_some();
        }
        let restart = restart.flatten();
        let machines = self.load_machines().await?;
        Ok(Workspace {
            machines,
            channels,
            sessions,
            restarts: restart.into_iter().collect(),
            requests,
            rules,
            ..Workspace::default()
        })
    }

    async fn create_channel(
        &self,
        name: String,
        repos: Vec<String>,
        machines: Vec<MachineId>,
    ) -> Result<ChannelId, DaemonError> {
        let shown_name = name.clone();
        let repos = repos
            .iter()
            .map(|text| map::parse_repo(text).ok_or_else(|| bad_repo(text)))
            .collect::<Result<Vec<_>, _>>()?;
        let machines = if machines.is_empty() {
            vec![self.machine.clone()]
        } else {
            machines
        };
        let reply = self
            .channels()
            .create_channel(pb::CreateChannelRequest {
                idempotency_key: fresh_key(),
                name,
                repos,
                machines: machines
                    .iter()
                    .map(|machine| pb::MachineId {
                        value: machine.as_str().to_owned(),
                    })
                    .collect(),
            })
            .await
            .map_err(|status| self.error(&status, "The channel"))?
            .into_inner();
        let id = reply.channel.and_then(|channel| channel.id).ok_or_else(|| DaemonError::Refused {
            what: "The channel".to_owned(),
            reason: "the daemon didn't name the channel it made. Reload to see whether it exists".to_owned(),
        })?;
        let id = ChannelId::new(id.value);
        if let Some(setup) = reply.setup {
            if let Some(operation) = &setup.id {
                let mut state = self.state();
                state.setup_names.insert(operation.value.clone(), shown_name);
                state.setup_channels.insert(operation.value.clone(), id.clone());
            }
            self.watch_setup(setup).await?;
        }
        Ok(id)
    }

    async fn create_session(
        &self,
        channel: ChannelId,
        agent: Agent,
        machine: MachineId,
        title: String,
        options: SessionOptions,
    ) -> Result<SessionId, DaemonError> {
        let channel_id = pb::ChannelId {
            value: channel.as_str().to_owned(),
        };
        let known = self
            .channels()
            .get_channel(pb::GetChannelRequest {
                id: Some(channel_id.clone()),
            })
            .await
            .map_err(|status| self.error(&status, "The channel"))?
            .into_inner();
        let mut repos = Vec::new();
        for text in &options.repos {
            let repo = map::parse_repo(text).ok_or_else(|| bad_repo(text))?;
            repos.push(pb::RepoChoice {
                repo: Some(repo),
                include: true,
            });
        }
        for repo in known.repos {
            if !repos.iter().any(|choice| choice.repo.as_ref() == Some(&repo)) {
                repos.push(pb::RepoChoice {
                    repo: Some(repo),
                    include: false,
                });
            }
        }
        let reply = self
            .sessions()
            .start_session(pb::StartSessionRequest {
                idempotency_key: fresh_key(),
                channel: Some(channel_id),
                title,
                agent: Some(pb::AgentId {
                    value: self.agents.name(agent),
                }),
                model: options.model.unwrap_or_default(),
                machine: Some(pb::MachineId {
                    value: machine.as_str().to_owned(),
                }),
                repos,
                first_message: options.first_message,
                board_item: None,
            })
            .await
            .map_err(|status| self.error(&status, "The new session"))?
            .into_inner();
        let id = reply.session.ok_or_else(|| DaemonError::Refused {
            what: "The new session".to_owned(),
            reason: "the daemon didn't name the session it started. Reload to see whether it exists".to_owned(),
        })?;
        Ok(SessionId::new(id.value))
    }

    async fn resume(&self, session: SessionId) -> Result<(), DaemonError> {
        let id = pb::SessionId {
            value: session.as_str().to_owned(),
        };
        let operation = self
            .sessions()
            .resume_session(pb::ResumeSessionRequest {
                idempotency_key: fresh_key(),
                id: Some(id),
                mode: pb::resume_session_request::Mode::Resume as i32,
            })
            .await
            .map_err(|status| self.error(&status, "Resuming the session"))?
            .into_inner();
        self.finish(operation, "Resuming the session").await.map(|_| ())
    }

    async fn start_fresh(&self, session: SessionId) -> Result<(), DaemonError> {
        let id = pb::SessionId {
            value: session.as_str().to_owned(),
        };
        let operation = self
            .sessions()
            .resume_session(pb::ResumeSessionRequest {
                idempotency_key: fresh_key(),
                id: Some(id),
                mode: pb::resume_session_request::Mode::StartFreshWithSummary as i32,
            })
            .await
            .map_err(|status| self.error(&status, "Starting the session fresh"))?
            .into_inner();
        self.finish(operation, "Starting the session fresh").await.map(|_| ())
    }

    async fn cancel_setup(&self, setup: SetupId) -> Result<(), DaemonError> {
        let operation = self
            .operations()
            .cancel_operation(pb::CancelOperationRequest {
                id: Some(pb::OperationId { value: setup.0 }),
                idempotency_key: fresh_key(),
            })
            .await
            .map_err(|status| self.error(&status, "Stopping the setup"))?
            .into_inner();
        self.publish_setup(&operation);
        Ok(())
    }

    /// Runs a failed setup's failed steps again; the caller returns as soon
    /// as the daemon has taken it, and the progress arrives as events.
    /// Runs a setup's failed steps again and follows it to the end; the
    /// channel it makes, once it is done.
    async fn retry_setup(&self, setup: SetupId) -> Result<ChannelId, DaemonError> {
        let channel = self.state().setup_channels.get(&setup.0).cloned();
        let operation = self
            .operations()
            .retry_operation(pb::RetryOperationRequest {
                id: Some(pb::OperationId { value: setup.0 }),
                idempotency_key: fresh_key(),
            })
            .await
            .map_err(|status| self.error(&status, "Trying the setup again"))?
            .into_inner();
        self.watch_setup(operation).await?;
        channel.ok_or_else(|| DaemonError::Refused {
            what: "The channel".to_owned(),
            reason: "its setup finished, but this app didn't start it, so it can't tell which channel it made. Reload to see it".to_owned(),
        })
    }

    async fn send(&self, session: SessionId, text: String) -> Result<(), DaemonError> {
        let id = pb::SessionId {
            value: session.as_str().to_owned(),
        };
        let tab = self.tab(&session, 0).await?;
        self.terminal()
            .r#type(pb::TypeRequest {
                idempotency_key: fresh_key(),
                session: Some(id),
                tab: tab.id,
                text: format!("{text}\r"),
                when: pb::type_request::When::Now as i32,
            })
            .await
            .map_err(|status| self.error(&status, "The message"))?;
        Ok(())
    }
}

fn bad_repo(text: &str) -> DaemonError {
    DaemonError::Refused {
        what: format!("The repository {text:?}"),
        reason: "write it as owner/name, such as acme/app".to_owned(),
    }
}

/// An answer that is not in this release.
fn later<T: Send + 'static>(what: &'static str) -> Reply<T> {
    Box::pin(async move { Err(not_in_this_release(what)) })
}

impl Daemon for LiveDaemon {
    fn read_file(&self, _session: &SessionId, _repo: &str, _path: &str) -> Reply<String> {
        later("Reading a file")
    }

    fn load(&self) -> Reply<Workspace> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.load().await })
    }

    fn events(&self) -> Reply<Events> {
        let shared: Arc<Shared> = self.shared.clone();
        Box::pin(async move { shared.subscribe().await })
    }

    fn send(&self, session: &SessionId, text: &str) -> Reply<()> {
        let shared = self.shared.clone();
        let (session, text) = (session.clone(), text.to_owned());
        Box::pin(async move { shared.send(session, text).await })
    }

    fn answer(&self, request: &RequestId, answer: Answer) -> Reply<()> {
        let shared = self.shared.clone();
        let request = request.clone();
        Box::pin(async move { shared.answer_request(request, answer).await })
    }

    fn reconnect(&self, _machine: &MachineId) -> Reply<Vec<(SessionId, Recovery)>> {
        later("Reconnecting a host")
    }

    fn resume(&self, session: &SessionId) -> Reply<()> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move { shared.resume(session).await })
    }

    fn start_fresh(&self, session: &SessionId) -> Reply<()> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move { shared.start_fresh(session).await })
    }

    fn cancel_setup(&self, setup: &SetupId) -> Reply<()> {
        let shared = self.shared.clone();
        let setup = setup.clone();
        Box::pin(async move { shared.cancel_setup(setup).await })
    }

    fn retry_setup(&self, setup: &SetupId) -> Reply<ChannelId> {
        let shared = self.shared.clone();
        let setup = setup.clone();
        Box::pin(async move { shared.retry_setup(setup).await })
    }

    fn fork(&self, _session: &SessionId, _kind: ForkKind, _title: &str) -> Reply<SessionId> {
        later("Forking a session")
    }

    fn archive(&self, _session: &SessionId) -> Reply<Archive> {
        later("Archiving a session")
    }

    fn add_machine(&self, _address: &str) -> Reply<MachineId> {
        later("Adding a host")
    }

    fn restore_session(&self, _session: &SessionId) -> Reply<()> {
        later("Restoring a session")
    }

    fn set_pinned(&self, _session: &SessionId, _pinned: bool) -> Reply<()> {
        later("Pinning a session")
    }

    fn set_vip(&self, _session: &SessionId, _vip: bool) -> Reply<()> {
        later("Marking a session very important")
    }

    fn mark_read(&self, _session: &SessionId) -> Reply<()> {
        later("Marking a session read")
    }

    fn create_session(
        &self,
        channel: &ChannelId,
        agent: Agent,
        machine: &MachineId,
        title: &str,
        options: SessionOptions,
    ) -> Reply<SessionId> {
        let shared = self.shared.clone();
        let (channel, machine, title) = (channel.clone(), machine.clone(), title.to_owned());
        Box::pin(async move {
            shared
                .create_session(channel, agent, machine, title, options)
                .await
        })
    }

    fn create_channel(
        &self,
        name: &str,
        repos: Vec<String>,
        machines: Vec<MachineId>,
    ) -> Reply<ChannelId> {
        let shared = self.shared.clone();
        let name = name.to_owned();
        Box::pin(async move { shared.create_channel(name, repos, machines).await })
    }

    fn move_item(&self, _item: &ItemId, _status: ItemStatus) -> Reply<()> {
        later("Moving a board item")
    }

    fn create_item(&self, _scope: Scope, _title: &str) -> Reply<ItemId> {
        later("Adding a board item")
    }

    fn add_dependency(&self, _item: &ItemId, _on: &ItemId) -> Reply<()> {
        later("Adding a dependency")
    }

    fn retry_transfer(&self, _transfer: &TransferId) -> Reply<()> {
        later("Retrying a transfer")
    }

    fn set_rule(
        &self,
        channel: &ChannelId,
        capability: &Capability,
        setting: RuleSetting,
    ) -> Reply<()> {
        let shared = self.shared.clone();
        let (channel, capability) = (channel.clone(), capability.clone());
        Box::pin(async move { shared.change_rule(channel, capability, setting).await })
    }

    fn remove_grant(&self, grant: &GrantId) -> Reply<()> {
        let shared = self.shared.clone();
        let grant = grant.clone();
        Box::pin(async move { shared.revoke(grant).await })
    }
}

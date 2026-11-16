//! [`Daemon`] over the daemon's gRPC services.

use std::sync::Arc;
use std::time::Duration;

use humata2_clients::model::{
    Agent, Answer, Capability, ChannelId, Comment, ForkKind, GrantId, ItemId,
    ItemStatus, MachineId, Recovery, RequestId, RuleSetting, Scope, SessionId, SessionOptions,
    TransferId,
};
use humata2_clients::{Daemon, DaemonError, Events, Reply, Workspace};
use humata2_grpc::humata::v1 as pb;

use crate::connection::{LiveDaemon, Shared, fresh_key, not_in_this_release};
use crate::map;

const PAGE_SIZE: u32 = 200;
const POLL: Duration = Duration::from_millis(100);

impl Shared {
    /// Waits for an operation to stop running; a failed or cancelled one is
    /// refused with the daemon's words.
    async fn finish(&self, mut operation: pb::Operation, what: &str) -> Result<pb::Operation, DaemonError> {
        while operation.state == pb::OperationState::Running as i32 {
            // On the client's runtime: the caller may be polled elsewhere.
            let _ = self.spawn(tokio::time::sleep(POLL)).await;
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
        Ok(Workspace {
            machines: vec![map::this_machine(self.machine.clone())],
            channels,
            sessions,
            ..Workspace::default()
        })
    }

    async fn create_channel(
        &self,
        name: String,
        repos: Vec<String>,
        machines: Vec<MachineId>,
    ) -> Result<ChannelId, DaemonError> {
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
        if let Some(setup) = reply.setup {
            self.finish(setup, "Setting up the channel").await?;
        }
        let id = reply.channel.and_then(|channel| channel.id).ok_or_else(|| DaemonError::Refused {
            what: "The channel".to_owned(),
            reason: "the daemon didn't name the channel it made. Reload to see whether it exists".to_owned(),
        })?;
        Ok(ChannelId::new(id.value))
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

    fn answer(&self, _request: &RequestId, _answer: Answer) -> Reply<()> {
        later("Answering a request")
    }

    fn reconnect(&self, _machine: &MachineId) -> Reply<Vec<(SessionId, Recovery)>> {
        later("Reconnecting a host")
    }

    fn resume(&self, session: &SessionId) -> Reply<()> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move { shared.resume(session).await })
    }

    fn fork(&self, _session: &SessionId, _kind: ForkKind, _title: &str) -> Reply<SessionId> {
        later("Forking a session")
    }

    fn archive(&self, _session: &SessionId) -> Reply<()> {
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

    fn send_comments(&self, _session: &SessionId, _comments: Vec<Comment>) -> Reply<()> {
        later("Sending review comments")
    }

    fn retry_transfer(&self, _transfer: &TransferId) -> Reply<()> {
        later("Retrying a transfer")
    }

    fn set_rule(
        &self,
        _channel: &ChannelId,
        _capability: &Capability,
        _setting: RuleSetting,
    ) -> Reply<()> {
        later("Changing an access rule")
    }

    fn remove_grant(&self, _grant: &GrantId) -> Reply<()> {
        later("Removing a grant")
    }
}

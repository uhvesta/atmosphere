//! [`Daemon`] over the daemon's gRPC services.

use std::sync::Arc;

use humata2_clients::model::{
    Agent, Answer, Archive, Capability, ChannelId, DiffBase, ForkKind, GrantId, ItemId, ItemStatus,
    MachineId, PushChoice, PushPlan, Recovery, RepoBases, RepoDiff, RequestId, Restart,
    RuleSetting, Scope, SessionId, SessionOptions, SetupId, TransferId,
};
use humata2_clients::{
    AgentSwitch, ArchivePlan, ChannelSessionWork, Daemon, DaemonError, DeleteChannelPlan,
    DeleteChannelRequest, DeleteChannelResponse, Event, Events, ForkOptions, HolderTab, PastTurn,
    Reply, RunningHolder, SwitchChange, UnpushedWork, Workspace,
};
use humata2_clients::mass_delete::{DeleteScope, DeleteSessionsOutcome, DeleteSessionsPlan};
use crate::humata2_grpc::humata::v1 as pb;

use crate::connection::{LiveDaemon, Shared, fresh_key, key_of, not_in_this_release};
use crate::{map, message, setup};

const PAGE_SIZE: u32 = 200;

impl Shared {
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
        if !self.state().restart_cue() {
            return;
        }
        let shared = self.clone();
        self.spawn(async move {
            loop {
                let restart = shared.read_restart().await;
                let again = shared.state().restart_read_ended(restart.is_some());
                if let Some(Some(restart)) = restart {
                    shared.publish(Event::Restart(restart));
                }
                if !again {
                    break;
                }
            }
        });
    }

    /// Shows a setup's operation to the app.
    fn publish_setup(&self, operation: &pb::Operation) {
        let id = operation
            .id
            .as_ref()
            .map(|id| id.value.clone())
            .unwrap_or_default();
        let name = self
            .state()
            .setup_names
            .get(&id)
            .cloned()
            .unwrap_or_default();
        self.publish(Event::ChannelSetup(setup::setup(
            &name,
            operation,
            &self.machine,
        )));
    }

    /// Follows a channel setup until it stops running, pushing each change.
    /// A failed or cancelled one is refused with the daemon's words.
    async fn watch_setup(&self, operation: pb::Operation) -> Result<pb::Operation, DaemonError> {
        self.publish_setup(&operation);
        self.follow_setup(operation).await
    }

    /// Streams updates for an already-published channel setup operation until
    /// it reaches a terminal state, publishing each change.
    async fn follow_setup(&self, mut last: pb::Operation) -> Result<pb::Operation, DaemonError> {
        const WHAT: &str = "Setting up the channel";
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
            let reply =
                match crate::humata2_grpc::humata::v1::machine_service_client::MachineServiceClient::new(
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
            let now = crate::local_time::LocalNow::read();
            for listed in &reply.machines {
                let mapped = map::machine(listed, &now);
                if mapped.id != self.machine
                    && mapped.kind != humata2_clients::model::MachineKind::ThisMac
                    && !machines.iter().any(|known| known.id == mapped.id)
                {
                    machines.push(mapped);
                }
            }
            token = reply
                .page
                .map(|page| page.next_page_token)
                .unwrap_or_default();
            if token.is_empty() {
                break;
            }
        }
        Ok(machines)
    }

    async fn load(self: &Arc<Self>) -> Result<Workspace, DaemonError> {
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
            channels.extend(reply.channels.iter().map(|channel| map::channel(channel, &self.agents)));
            token = reply
                .page
                .map(|page| page.next_page_token)
                .unwrap_or_default();
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
                    Some(id) => self.listed_tabs(id).await,
                    None => Vec::new(),
                };
                sessions.push(self.map_session(session, &tabs).await);
            }
            token = reply
                .page
                .map(|page| page.next_page_token)
                .unwrap_or_default();
            if token.is_empty() {
                break;
            }
        }
        let (requests, allowed_without_asking_today) = self.load_requests().await?;
        let ids: Vec<ChannelId> = channels.iter().map(|channel| channel.id.clone()).collect();
        let rules = self.load_rules(&ids).await?;
        let restart = self.read_restart().await;
        {
            let mut state = self.state();
            state.restart_known = restart.is_some();
        }
        let restart = restart.flatten();
        let machines = self.load_machines().await?;
        let (items, item_details) = self.load_items(&ids).await?;
        let desk = humata2_clients_live_desk::load_desk(&self.conn, &ids).await?;
        Ok(Workspace {
            items,
            item_details,
            machines,
            channels,
            sessions,
            restarts: restart.into_iter().collect(),
            requests,
            allowed_without_asking_today,
            rules,
            desk,
            ..Workspace::default()
        })
    }

    async fn create_channel(
        self: &Arc<Self>,
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
        let id = reply
            .channel
            .and_then(|channel| channel.id)
            .ok_or_else(|| DaemonError::Refused {
                what: "The channel".to_owned(),
                reason:
                    "the daemon didn't name the channel it made. Reload to see whether it exists"
                        .to_owned(),
            })?;
        let id = ChannelId::new(id.value);
        if let Some(setup) = reply.setup {
            if let Some(operation) = &setup.id {
                let mut state = self.state();
                state
                    .setup_names
                    .insert(operation.value.clone(), shown_name);
                state
                    .setup_channels
                    .insert(operation.value.clone(), id.clone());
            }
            // Return as soon as the daemon has created the channel so the
            // Create Channel sheet closes immediately while clone progress
            // streams asynchronously via `Event::ChannelSetup` (SMB-001).
            self.publish_setup(&setup);
            if setup.state == pb::OperationState::Running as i32 {
                let shared = self.clone();
                self.spawn(async move {
                    let _ = shared.follow_setup(setup).await;
                });
            }
        }
        Ok(id)
    }

    /// The GitHub repositories `gh` sees on the host, as `owner/name`, that
    /// match `query` (CH-3). Reads the daemon's pages until it has no more,
    /// up to a fixed number of pages.
    async fn github_repos(&self, query: String) -> Result<Vec<String>, DaemonError> {
        // 10 pages of 100 hold the daemon's whole list (at most 1000, D272).
        const REPO_PAGE_SIZE: u32 = 100;
        const REPO_PAGES: usize = 10;
        let mut repos = Vec::new();
        let mut token = String::new();
        for _ in 0..REPO_PAGES {
            let reply = self
                .channels()
                .search_git_hub_repos(pb::SearchGitHubReposRequest {
                    query: query.clone(),
                    page: Some(pb::PageRequest {
                        page_size: REPO_PAGE_SIZE,
                        page_token: token,
                    }),
                })
                .await
                .map_err(|status| self.error(&status, "The GitHub repositories"))?
                .into_inner();
            repos.extend(reply.repos.iter().filter_map(|listed| {
                listed
                    .repo
                    .as_ref()
                    .map(|repo| format!("{}/{}", repo.owner, repo.name))
            }));
            token = reply
                .page
                .map(|page| page.next_page_token)
                .unwrap_or_default();
            if token.is_empty() {
                break;
            }
        }
        Ok(repos)
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
            if !repos
                .iter()
                .any(|choice| choice.repo.as_ref() == Some(&repo))
            {
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
                branch: String::new(),
                rebuild: None,
                review: None,
            })
            .await
            .map_err(|status| self.error(&status, "The new session"))?
            .into_inner();
        let id = reply.session.ok_or_else(|| DaemonError::Refused {
            what: "The new session".to_owned(),
            reason:
                "the daemon didn't name the session it started. Reload to see whether it exists"
                    .to_owned(),
        })?;
        let started = SessionId::new(id.value);
        if let Some(item) = &options.item {
            self.start_item_work(item, &started).await;
        }
        Ok(started)
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
        self.finish(operation, "Resuming the session")
            .await
            .map(|_| ())
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
        self.finish(operation, "Starting the session fresh")
            .await
            .map(|_| ())
    }

    async fn fork(
        &self,
        session: SessionId,
        options: ForkOptions,
    ) -> Result<SessionId, DaemonError> {
        let kind = match options.kind {
            ForkKind::Worktree => pb::ForkKind::CodeAndContext,
            ForkKind::Shared => pb::ForkKind::CodeOnly,
            ForkKind::Conversation => pb::ForkKind::ContextOnly,
        };
        let operation = self
            .sessions()
            .fork_session(pb::ForkSessionRequest {
                idempotency_key: fresh_key(),
                parent: Some(pb::SessionId {
                    value: session.as_str().to_owned(),
                }),
                kind: kind as i32,
                before_turn: options.before_turn.unwrap_or(0),
                title: options.title,
                agent: options.agent.map(|agent| pb::AgentId {
                    value: self.agents.name(agent),
                }),
                model: String::new(),
                machine: options.machine.map(|machine| pb::MachineId {
                    value: machine.as_str().to_owned(),
                }),
                // A context-only fork works in its parent's clones, so it has
                // no branch to name and the daemon refuses one; the dialog's
                // branch draft is for the other kinds.
                branch: match options.kind {
                    ForkKind::Conversation => String::new(),
                    ForkKind::Worktree | ForkKind::Shared => options.branch,
                },
                reason: String::new(),
            })
            .await
            .map_err(|status| self.error(&status, "Forking the session"))?
            .into_inner();
        let done = self.finish(operation, "Forking the session").await?;
        match done.result {
            Some(pb::operation::Result::Session(id)) => Ok(SessionId::new(id.value)),
            _ => Err(DaemonError::Refused {
                what: "Forking the session".to_owned(),
                reason: "the daemon didn't name the fork it made. Reload to see whether it exists"
                    .to_owned(),
            }),
        }
    }

    async fn list_turns(&self, session: SessionId) -> Result<Vec<PastTurn>, DaemonError> {
        let mut turns = Vec::new();
        let mut token = String::new();
        loop {
            let reply = self
                .sessions()
                .list_turns(pb::ListTurnsRequest {
                    session: Some(pb::SessionId {
                        value: session.as_str().to_owned(),
                    }),
                    page: Some(pb::PageRequest {
                        page_size: PAGE_SIZE,
                        page_token: token,
                    }),
                })
                .await
                .map_err(|status| self.error(&status, "The turns of this session"))?
                .into_inner();
            turns.extend(reply.turns.into_iter().map(|turn| PastTurn {
                index: turn.index,
                text: turn.user_text,
            }));
            token = reply
                .page
                .map(|page| page.next_page_token)
                .unwrap_or_default();
            if token.is_empty() {
                return Ok(turns);
            }
        }
    }

    async fn move_session(&self, session: SessionId, to: MachineId) -> Result<(), DaemonError> {
        let operation = self
            .sessions()
            .move_session(pb::MoveSessionRequest {
                idempotency_key: fresh_key(),
                id: Some(pb::SessionId {
                    value: session.as_str().to_owned(),
                }),
                to: Some(pb::MachineId {
                    value: to.as_str().to_owned(),
                }),
                ..Default::default()
            })
            .await
            .map_err(|status| self.error(&status, "Moving the session"))?
            .into_inner();
        self.finish(operation, "Moving the session")
            .await
            .map(|_| ())
    }

    async fn continue_in_channel(
        &self,
        session: SessionId,
        channel: ChannelId,
        before_turn: Option<u32>,
    ) -> Result<SessionId, DaemonError> {
        const WHAT: &str = "Continuing the session in another channel";
        let reply = self
            .sessions()
            .continue_in_channel(pb::ContinueInChannelRequest {
                idempotency_key: fresh_key(),
                session: Some(pb::SessionId {
                    value: session.as_str().to_owned(),
                }),
                channel: Some(pb::ChannelId {
                    value: channel.as_str().to_owned(),
                }),
                // 0 is the whole conversation, from now.
                before_turn: before_turn.unwrap_or(0),
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        let id = reply.session.ok_or_else(|| DaemonError::Refused {
            what: WHAT.to_owned(),
            reason:
                "the daemon didn't name the session it started. Reload to see whether it exists"
                    .to_owned(),
        })?;
        if let Some(operation) = reply.operation {
            self.finish(operation, WHAT).await?;
        }
        Ok(SessionId::new(id.value))
    }

    async fn rename_session(
        &self,
        session: SessionId,
        title: String,
        rename_branch: bool,
    ) -> Result<(), DaemonError> {
        self.sessions()
            .rename_session(pb::RenameSessionRequest {
                idempotency_key: fresh_key(),
                id: Some(pb::SessionId {
                    value: session.as_str().to_owned(),
                }),
                title,
                expected_revision: None,
                rename_branch,
            })
            .await
            .map_err(|status| self.error(&status, "Renaming the session"))?;
        Ok(())
    }

    async fn stop_session(&self, session: SessionId) -> Result<(), DaemonError> {
        self.sessions()
            .stop_session(pb::StopSessionRequest {
                idempotency_key: fresh_key(),
                id: Some(pb::SessionId {
                    value: session.as_str().to_owned(),
                }),
                for_move: false,
            })
            .await
            .map_err(|status| self.error(&status, "Stopping the agent"))?;
        Ok(())
    }

    async fn read_archive_plan(&self, session: &SessionId) -> Result<pb::ArchivePlan, DaemonError> {
        self.sessions()
            .plan_archive(pb::PlanArchiveRequest {
                id: Some(pb::SessionId {
                    value: session.as_str().to_owned(),
                }),
            })
            .await
            .map_err(|status| self.error(&status, "Checking what archiving would lose"))
            .map(tonic::Response::into_inner)
    }

    async fn plan_archive(&self, session: SessionId) -> Result<ArchivePlan, DaemonError> {
        let plan = self.read_archive_plan(&session).await?;
        Ok(ArchivePlan {
            forks_keep_running: plan.forks_keep_running,
            unpushed: plan.unpushed.iter().map(unpushed).collect(),
            frees: None,
            unsubmitted_drafts: plan.unsubmitted_drafts,
        })
    }

    async fn archive(
        &self,
        session: SessionId,
        push_first: bool,
        end: bool,
    ) -> Result<Archive, DaemonError> {
        let plan = self.read_archive_plan(&session).await?;
        let choice = if push_first {
            pb::ArchiveChoice::CommitPushPrThenArchive
        } else if plan.unpushed.is_empty() {
            pb::ArchiveChoice::Archive
        } else {
            pb::ArchiveChoice::ArchiveAndDeleteWork
        };
        let operation = self
            .sessions()
            .archive_session(pb::ArchiveSessionRequest {
                idempotency_key: fresh_key(),
                id: Some(pb::SessionId {
                    value: session.as_str().to_owned(),
                }),
                choice: choice as i32,
                plan: plan.token,
                end: end.then_some(true),
            })
            .await
            .map_err(|status| self.error(&status, "Archiving the session"))?
            .into_inner();
        self.finish(operation, "Archiving the session").await?;
        Ok(Archive { frees: None })
    }

    /// Brings an archived session back where it was (the Undo after an
    /// archive), and waits for the daemon to finish.
    async fn restore_session(&self, session: SessionId) -> Result<(), DaemonError> {
        self.restore_session_as(session, false).await
    }

    /// Restores an archived session, `stopped` to leave its agent unstarted
    /// (D784).
    async fn restore_session_as(&self, session: SessionId, stopped: bool) -> Result<(), DaemonError> {
        let operation = self
            .sessions()
            .restore_session(pb::RestoreSessionRequest {
                idempotency_key: fresh_key(),
                id: Some(pb::SessionId {
                    value: session.as_str().to_owned(),
                }),
                machine: None,
                stopped,
            })
            .await
            .map_err(|status| self.error(&status, "Restoring the session"))?
            .into_inner();
        self.finish(operation, "Restoring the session").await?;
        Ok(())
    }

    /// Sets a session's pin or VIP mark on the host, the same marks the
    /// sidebar's Pinned list reads.
    async fn set_session_marks(
        &self,
        session: SessionId,
        pinned: Option<bool>,
        vip: Option<bool>,
        what: &str,
    ) -> Result<(), DaemonError> {
        use pb::entity_ref::Entity;
        pb::mark_service_client::MarkServiceClient::new(self.wire.clone())
            .set_marks(pb::SetMarksRequest {
                idempotency_key: fresh_key(),
                entity: Some(pb::EntityRef {
                    entity: Some(Entity::Session(pb::SessionId {
                        value: session.as_str().to_owned(),
                    })),
                }),
                pinned,
                vip,
            })
            .await
            .map_err(|status| self.error(&status, what))?;
        Ok(())
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

    async fn send(
        self: &Arc<Self>,
        session: SessionId,
        tab: usize,
        text: String,
        key: Option<String>,
    ) -> Result<(), DaemonError> {
        let id = pb::SessionId {
            value: session.as_str().to_owned(),
        };
        let tab = self.tab(&session, tab).await?;
        self.terminal()
            .r#type(pb::TypeRequest {
                idempotency_key: key.map_or_else(fresh_key, key_of),
                session: Some(id),
                tab: tab.id,
                text: message::typed(&text),
                when: pb::type_request::When::Now as i32,
            })
            .await
            .map_err(|status| self.error(&status, "The message"))?;
        Ok(())
    }

    async fn plan_delete_channel(
        &self,
        channel: ChannelId,
    ) -> Result<DeleteChannelPlan, DaemonError> {
        const WHAT: &str = "Planning the channel delete";
        let plan = self
            .channels()
            .plan_delete_channel(pb::PlanDeleteChannelRequest {
                channel: Some(pb::ChannelId {
                    value: channel.as_str().to_owned(),
                }),
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        let token = plan
            .token
            .map(|token| token.value)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| DaemonError::Refused {
                what: WHAT.to_owned(),
                reason: "the daemon didn't give a token for the plan. Plan the delete again"
                    .to_owned(),
            })?;
        Ok(DeleteChannelPlan {
            channel: plan
                .channel
                .as_ref()
                .map(|id| map::channel_id(Some(id)))
                .unwrap_or(channel),
            live_sessions: plan
                .live_sessions
                .into_iter()
                .map(|work| ChannelSessionWork {
                    session: map::session_id(work.session.as_ref()),
                    title: work.title,
                    dirty_files: work.dirty_files,
                    unpushed_commits: work.unpushed_commits,
                })
                .collect(),
            archived_sessions: plan.archived_sessions,
            board_items: plan.board_items,
            pending_requests: plan.pending_requests,
            token,
            moves_to_desk: plan
                .moves_to_desk
                .into_iter()
                .map(|id| map::session_id(Some(&id)))
                .collect(),
            cannot_move_count: u32::try_from(plan.cannot_move.len()).unwrap_or(u32::MAX),
            offline_machines: plan
                .offline_machines
                .into_iter()
                .map(|id| map::machine_id(Some(&id)))
                .collect(),
        })
    }

    async fn delete_channel(
        &self,
        request: DeleteChannelRequest,
    ) -> Result<DeleteChannelResponse, DaemonError> {
        const WHAT: &str = "Deleting the channel";
        let reply = self
            .channels()
            .delete_channel(pb::DeleteChannelRequest {
                idempotency_key: fresh_key(),
                channel: Some(pb::ChannelId {
                    value: request.channel.as_str().to_owned(),
                }),
                plan: Some(pb::PlanToken {
                    value: request.plan_token,
                }),
                presence: None,
                delete_unpushed_work: request.delete_work,
                end_live: request.end_live,
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        if let Some(operation) = reply.operation {
            self.finish(operation, WHAT).await?;
        }
        Ok(DeleteChannelResponse {
            trash_item: reply
                .trash_item
                .map(|item| item.value)
                .unwrap_or_default(),
        })
    }
}

pub(crate) use humata2_clients_live_core::bad_repo;

/// An answer that is not in this release.
/// One repository's unpushed work, as a sentence for the archive dialog.
pub(crate) fn unpushed(repo: &pb::UnpushedRepo) -> UnpushedWork {
    let name = repo
        .repo
        .as_ref()
        .map_or_else(String::new, |r| r.name.clone());
    let mut parts = Vec::new();
    if repo.dirty_files > 0 {
        let noun = if repo.dirty_files == 1 {
            "file"
        } else {
            "files"
        };
        parts.push(format!("{} uncommitted {noun}", repo.dirty_files));
    }
    if repo.unpushed_commits > 0 {
        let noun = if repo.unpushed_commits == 1 {
            "commit"
        } else {
            "commits"
        };
        parts.push(format!("{} {noun}", repo.unpushed_commits));
    }
    let on = format!("on {}", repo.branch);
    let tail = if repo.never_pushed {
        "never pushed"
    } else {
        "not pushed"
    };
    UnpushedWork {
        repo: name,
        what: format!("{} {on}, {tail}.", parts.join(" and ")),
    }
}

fn later<T: Send + 'static>(what: &'static str) -> Reply<T> {
    Box::pin(async move { Err(not_in_this_release(what)) })
}

impl Daemon for LiveDaemon {
    fn read_file(&self, session: &SessionId, repo: &str, path: &str) -> Reply<String> {
        let conn = self.shared.conn.clone();
        let (session, repo, path) = (session.clone(), repo.to_owned(), path.to_owned());
        Box::pin(async move {
            humata2_clients_live_diff::read_session_file(&conn, &session, &repo, &path).await
        })
    }

    fn find_files(&self, session: &SessionId) -> Reply<Vec<humata2_clients::model::SessionFile>> {
        let conn = self.shared.conn.clone();
        let session = session.clone();
        Box::pin(async move { humata2_clients_live_diff::find_session_files(&conn, &session).await })
    }

    fn diffs(&self, session: &SessionId, base: &DiffBase) -> Reply<Vec<RepoDiff>> {
        let conn = self.shared.conn.clone();
        let (session, base) = (session.clone(), base.clone());
        Box::pin(async move { humata2_clients_live_diff::session_diffs(&conn, &session, &base).await })
    }

    fn repo_diff(&self, session: &SessionId, base: &DiffBase, repo: &str) -> Reply<Vec<RepoDiff>> {
        let conn = self.shared.conn.clone();
        let (session, base, repo) = (session.clone(), base.clone(), repo.to_owned());
        Box::pin(async move {
            humata2_clients_live_diff::repo_diff(&conn, &session, &base, &repo).await
        })
    }

    fn bases(&self, session: &SessionId, repo: &str) -> Reply<RepoBases> {
        let conn = self.shared.conn.clone();
        let (session, repo) = (session.clone(), repo.to_owned());
        Box::pin(async move { humata2_clients_live_diff::repo_bases(&conn, &session, &repo).await })
    }

    fn find_default_branch(&self, session: &SessionId, repo: &str) -> Reply<String> {
        let conn = self.shared.conn.clone();
        let (session, repo) = (session.clone(), repo.to_owned());
        Box::pin(async move {
            humata2_clients_live_diff::find_default_branch(&conn, &session, &repo).await
        })
    }

    fn mark_reviewed(&self, session: &SessionId) -> Reply<()> {
        let conn = self.shared.conn.clone();
        let session = session.clone();
        Box::pin(async move { humata2_clients_live_diff::mark_reviewed(&conn, &session).await })
    }

    fn looks(&self, session: &SessionId) -> Reply<Vec<humata2_clients::model::LookedFile>> {
        let conn = self.shared.conn.clone();
        let session = session.clone();
        Box::pin(async move { humata2_clients_live_diff::looks(&conn, &session).await })
    }

    fn save_looks(
        &self,
        session: &SessionId,
        looks: Vec<humata2_clients::model::LookedFile>,
    ) -> Reply<()> {
        let conn = self.shared.conn.clone();
        let session = session.clone();
        Box::pin(async move { humata2_clients_live_diff::save_looks(&conn, &session, looks).await })
    }

    fn plan_push(&self, session: &SessionId) -> Reply<PushPlan> {
        let conn = self.shared.conn.clone();
        let session = session.clone();
        Box::pin(async move { humata2_clients_live_diff::plan_push(&conn, &session).await })
    }

    fn push(
        &self,
        session: &SessionId,
        token: &str,
        choices: Vec<PushChoice>,
    ) -> Reply<Vec<humata2_clients::model::PushedPullRequest>> {
        let conn = self.shared.conn.clone();
        let session = session.clone();
        let token = token.to_owned();
        Box::pin(async move {
            humata2_clients_live_diff::push(&conn, &session, &token, choices).await
        })
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
        Box::pin(async move { shared.send(session, 0, text, None).await })
    }

    fn send_keyed(&self, session: &SessionId, text: &str, key: &str) -> Reply<()> {
        let shared = self.shared.clone();
        let (session, text, key) = (session.clone(), text.to_owned(), key.to_owned());
        Box::pin(async move { shared.send(session, 0, text, Some(key)).await })
    }

    fn send_to_tab(&self, session: &SessionId, tab: usize, text: &str, key: &str) -> Reply<()> {
        let shared = self.shared.clone();
        let (session, text, key) = (session.clone(), text.to_owned(), key.to_owned());
        Box::pin(async move { shared.send(session, tab, text, Some(key)).await })
    }

    fn answer(&self, request: &RequestId, answer: Answer) -> Reply<()> {
        let shared = self.shared.clone();
        let request = request.clone();
        Box::pin(async move { shared.answer_request(request, answer).await })
    }

    fn ask_why(&self, request: &RequestId) -> Reply<humata2_clients::model::Request> {
        let shared = self.shared.clone();
        let request = request.clone();
        Box::pin(async move { shared.ask_why(request).await })
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

    fn fork(&self, session: &SessionId, options: ForkOptions) -> Reply<SessionId> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move { shared.fork(session, options).await })
    }

    fn list_turns(&self, session: &SessionId) -> Reply<Vec<PastTurn>> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move { shared.list_turns(session).await })
    }

    fn move_session(&self, session: &SessionId, to: &MachineId) -> Reply<()> {
        let shared = self.shared.clone();
        let (session, to) = (session.clone(), to.clone());
        Box::pin(async move { shared.move_session(session, to).await })
    }

    fn continue_in_channel(
        &self,
        session: &SessionId,
        channel: &ChannelId,
        before_turn: Option<u32>,
    ) -> Reply<SessionId> {
        let shared = self.shared.clone();
        let (session, channel) = (session.clone(), channel.clone());
        Box::pin(async move { shared.continue_in_channel(session, channel, before_turn).await })
    }

    fn rename_session(&self, session: &SessionId, title: &str, rename_branch: bool) -> Reply<()> {
        let shared = self.shared.clone();
        let (session, title) = (session.clone(), title.to_owned());
        Box::pin(async move { shared.rename_session(session, title, rename_branch).await })
    }

    fn add_repo_to_session(&self, session: &SessionId, repo: &str) -> Reply<()> {
        let shared = self.shared.clone();
        let (session, repo) = (session.clone(), repo.to_owned());
        Box::pin(async move { shared.add_repo_to_session(session, repo).await })
    }

    fn stop_agent(&self, session: &SessionId) -> Reply<()> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move { shared.stop_session(session).await })
    }

    fn plan_archive(&self, session: &SessionId) -> Reply<ArchivePlan> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move { shared.plan_archive(session).await })
    }

    fn plan_delete_sessions(&self, scope: &DeleteScope) -> Reply<DeleteSessionsPlan> {
        let shared = self.shared.clone();
        let scope = scope.clone();
        Box::pin(async move { shared.plan_delete_sessions(scope).await })
    }

    fn delete_sessions(
        &self,
        token: &str,
        delete_unpushed_work: bool,
        typed_count: u32,
    ) -> Reply<DeleteSessionsOutcome> {
        let shared = self.shared.clone();
        let token = token.to_owned();
        Box::pin(async move { shared.delete_sessions(&token, delete_unpushed_work, typed_count).await })
    }

    fn archive(&self, session: &SessionId, push_first: bool) -> Reply<Archive> {
        self.archive_with_end(session, push_first, false)
    }

    fn archive_with_end(&self, session: &SessionId, push_first: bool, end: bool) -> Reply<Archive> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move { shared.archive(session, push_first, end).await })
    }

    fn add_machine(&self, _address: &str) -> Reply<MachineId> {
        later("Adding a host")
    }

    fn restore_session(&self, session: &SessionId) -> Reply<()> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move { shared.restore_session(session).await })
    }

    fn restore_session_stopped(&self, session: &SessionId) -> Reply<()> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move { shared.restore_session_as(session, true).await })
    }

    fn running_holders(&self, machine: &MachineId) -> Reply<Vec<RunningHolder>> {
        let shared = self.shared.clone();
        let machine = machine.clone();
        Box::pin(async move { shared.running_holders(machine).await })
    }

    fn stop_running_holder(&self, machine: &MachineId, tab: &HolderTab) -> Reply<()> {
        let shared = self.shared.clone();
        let (machine, tab) = (machine.clone(), tab.clone());
        Box::pin(async move { shared.stop_running_holder(machine, tab).await })
    }

    fn resume_running_holder(&self, machine: &MachineId, tab: &HolderTab) -> Reply<()> {
        let shared = self.shared.clone();
        let (machine, tab) = (machine.clone(), tab.clone());
        Box::pin(async move { shared.resume_running_holder(machine, tab).await })
    }

    fn end_running_holder(&self, machine: &MachineId, tab: &HolderTab) -> Reply<()> {
        let shared = self.shared.clone();
        let (machine, tab) = (machine.clone(), tab.clone());
        Box::pin(async move { shared.end_running_holder(machine, tab).await })
    }

    fn adopt_running_holder(
        &self,
        machine: &MachineId,
        tab: &str,
        channel: Option<&humata2_clients::model::ChannelId>,
    ) -> Reply<SessionId> {
        let shared = self.shared.clone();
        let (machine, tab, channel) = (machine.clone(), tab.to_owned(), channel.cloned());
        Box::pin(async move { shared.adopt_running_holder(machine, tab, channel).await })
    }

    fn set_pinned(&self, session: &SessionId, pinned: bool) -> Reply<()> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move {
            shared
                .set_session_marks(session, Some(pinned), None, "Pinning the session")
                .await
        })
    }

    fn set_vip(&self, session: &SessionId, vip: bool) -> Reply<()> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move {
            shared
                .set_session_marks(session, None, Some(vip), "Marking the session very important")
                .await
        })
    }

    /// Views the session on the daemon, which clears its unread mark (D392).
    /// A session that couldn't resume keeps its recovery until it is
    /// resumed or started fresh: the daemon has no note to clear.
    fn mark_read(&self, session: &SessionId) -> Reply<()> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move { shared.view(Some(&session)).await })
    }

    fn view(&self, session: Option<&SessionId>) -> Reply<()> {
        let shared = self.shared.clone();
        let session = session.cloned();
        Box::pin(async move { shared.view(session.as_ref()).await })
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

    fn plan_delete_channel(&self, channel: &ChannelId) -> Reply<DeleteChannelPlan> {
        let shared = self.shared.clone();
        let channel = channel.clone();
        Box::pin(async move { shared.plan_delete_channel(channel).await })
    }

    fn delete_channel(&self, request: DeleteChannelRequest) -> Reply<DeleteChannelResponse> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.delete_channel(request).await })
    }

    fn add_repo_to_channel(&self, channel: &ChannelId, repo: &str) -> Reply<()> {
        let shared = self.shared.clone();
        let (channel, repo) = (channel.clone(), repo.to_owned());
        Box::pin(async move { shared.add_repo_to_channel(channel, repo).await })
    }

    fn github_repos(&self, query: &str) -> Reply<Vec<String>> {
        let shared = self.shared.clone();
        let query = query.to_owned();
        Box::pin(async move { shared.github_repos(query).await })
    }

    // REPOS (RL-5): the repo library and the one repo search, over RepoService.
    fn search_repos(
        &self,
        query: &str,
        scope: Option<humata2_clients::repos::RepoScope>,
    ) -> Reply<humata2_clients::repos::RepoSearch> {
        let shared = self.shared.clone();
        let query = query.to_owned();
        Box::pin(async move { shared.search_repos(query, scope).await })
    }

    fn repo_library(&self) -> Reply<Vec<humata2_clients::repos::LibraryRepo>> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.repo_library().await })
    }

    fn add_to_repo_library(&self, repo: &str) -> Reply<Vec<humata2_clients::repos::LibraryRepo>> {
        let shared = self.shared.clone();
        let repo = repo.to_owned();
        Box::pin(async move { shared.add_to_repo_library(&repo).await })
    }

    fn remove_from_repo_library(&self, repo: &str) -> Reply<Vec<humata2_clients::repos::LibraryRepo>> {
        let shared = self.shared.clone();
        let repo = repo.to_owned();
        Box::pin(async move { shared.remove_from_repo_library(&repo).await })
    }

    fn forget_repo_library(&self) -> Reply<u32> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.forget_repo_library().await })
    }

    fn repo_scope(&self) -> Reply<humata2_clients::repos::RepoScope> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.repo_scope().await })
    }

    fn save_repo_scope(
        &self,
        scope: humata2_clients::repos::RepoScope,
    ) -> Reply<humata2_clients::repos::RepoScope> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.save_repo_scope(scope).await })
    }

    fn repo_owners(&self) -> Reply<Vec<String>> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.repo_owners().await })
    }

    fn move_item(&self, item: &ItemId, status: ItemStatus, seen: Option<u64>) -> Reply<()> {
        let shared = self.shared.clone();
        let item = item.clone();
        Box::pin(async move { shared.move_item(&item, status, seen).await })
    }

    fn create_item_in(&self, scope: Scope, title: &str, status: ItemStatus) -> Reply<ItemId> {
        let shared = self.shared.clone();
        let title = title.to_owned();
        Box::pin(async move { shared.create_item(&scope, &title, status).await })
    }

    fn add_dependency(&self, item: &ItemId, on: &ItemId) -> Reply<()> {
        let shared = self.shared.clone();
        let (item, on) = (item.clone(), on.clone());
        Box::pin(async move { shared.set_dependency(&item, &on, true).await })
    }

    fn remove_dependency(&self, item: &ItemId, on: &ItemId) -> Reply<()> {
        let shared = self.shared.clone();
        let (item, on) = (item.clone(), on.clone());
        Box::pin(async move { shared.set_dependency(&item, &on, false).await })
    }

    fn set_item_session(&self, item: &ItemId, session: &SessionId, working: bool) -> Reply<()> {
        let shared = self.shared.clone();
        let (item, session) = (item.clone(), session.clone());
        Box::pin(async move { shared.set_item_session(&item, &session, working).await })
    }

    fn item_history(&self, item: &ItemId) -> Reply<Vec<String>> {
        let shared = self.shared.clone();
        let item = item.clone();
        Box::pin(async move { shared.item_history(&item).await })
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


    fn codex_needs_trust(&self) -> Reply<bool> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.codex_needs_trust().await })
    }

    fn set_default_agent(&self, agent: Agent) -> Reply<()> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.change_default_agent(agent).await })
    }


    fn agent_switches(&self) -> Reply<Vec<AgentSwitch>> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.agent_switches().await })
    }

    fn set_agent_switch(&self, kind: String, change: SwitchChange) -> Reply<AgentSwitch> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.set_agent_switch(kind, change).await })
    }

    fn sync_settings(&self) -> Reply<humata2_clients::daemon::SyncSnapshot> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.sync_snapshot().await })
    }

    fn set_sync_settings(
        &self,
        change: humata2_clients::daemon::SyncChange,
    ) -> Reply<humata2_clients::daemon::SyncSnapshot> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.change_sync_settings(change).await })
    }

    fn web_todo_settings(&self) -> Reply<humata2_clients::daemon::WebTodoSnapshot> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.web_todo_snapshot().await })
    }

    fn set_web_todo_enabled(&self, enabled: bool) -> Reply<humata2_clients::daemon::WebTodoSnapshot> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.change_web_todo_enabled(enabled).await })
    }

    fn todo_automation(&self) -> Reply<humata2_clients::daemon::TodoAutomation> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.todo_automation().await })
    }

    fn set_todo_automation(
        &self,
        section: humata2_clients::daemon::TodoAutomation,
    ) -> Reply<humata2_clients::daemon::TodoAutomation> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.change_todo_automation(section).await })
    }

    fn full_settings(&self) -> Reply<humata2_clients::daemon::FullSettings> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.full_settings().await })
    }

    fn update_settings(
        &self,
        patch: humata2_clients::daemon::SettingsPatch,
    ) -> Reply<humata2_clients::daemon::FullSettings> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.update_settings(patch).await })
    }

    fn refresh_gh_login(&self) -> Reply<humata2_clients::daemon::FullSettings> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.refresh_gh_login().await })
    }

    fn channel_settings(
        &self,
        channel: &ChannelId,
    ) -> Reply<humata2_clients::daemon::ChannelSettings> {
        let shared = self.shared.clone();
        let channel = channel.clone();
        Box::pin(async move { shared.channel_settings(channel).await })
    }

    fn update_channel_settings(
        &self,
        channel: &ChannelId,
        patch: humata2_clients::daemon::ChannelSettingsPatch,
    ) -> Reply<humata2_clients::daemon::ChannelSettings> {
        let shared = self.shared.clone();
        let channel = channel.clone();
        Box::pin(async move { shared.update_channel_settings(channel, patch).await })
    }

    fn list_detected_browsers(&self) -> Reply<Vec<humata2_clients::model::DetectedBrowser>> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.list_detected_browsers().await })
    }

    fn install_browser_host(
        &self,
        kinds: Vec<humata2_clients::model::BrowserKind>,
    ) -> Reply<Vec<humata2_clients::model::DetectedBrowser>> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.install_browser_host(kinds).await })
    }

    fn uninstall_browser_host(
        &self,
        kinds: Vec<humata2_clients::model::BrowserKind>,
    ) -> Reply<Vec<humata2_clients::model::DetectedBrowser>> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.uninstall_browser_host(kinds).await })
    }

    fn browser_bridge_status(&self) -> Reply<humata2_clients::model::BrowserBridgeStatus> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.browser_bridge_status().await })
    }

    fn list_pending_cli_logins(&self) -> Reply<Vec<humata2_clients::PendingCliLogin>> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.list_pending_cli_logins().await })
    }

    fn answer_cli_login(&self, id: String, allow: bool) -> Reply<()> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.answer_cli_login(id, allow).await })
    }

    fn list_cli_sessions(&self) -> Reply<Vec<humata2_clients::CliSession>> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.list_cli_sessions().await })
    }

    fn end_cli_session(&self, id: String) -> Reply<()> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.end_cli_session(id).await })
    }

    fn list_script_tokens(&self) -> Reply<Vec<humata2_clients::ScriptToken>> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.list_script_tokens().await })
    }

    fn revoke_script_token(&self, id: String) -> Reply<()> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.revoke_script_token(id).await })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use humata2_clients::model::{ChannelId, MachineId, SetupState};
    use humata2_clients::{Daemon, Event};
    use tokio::net::UnixListener;
    use tokio::sync::{Notify, mpsc};
    use tokio_stream::wrappers::{ReceiverStream, UnixListenerStream};
    use tonic::{Request, Response, Status, Streaming};

    use crate::connection::{LiveDaemon, LiveOptions};
    use crate::humata2_grpc::humata::v1 as pb;
    use crate::humata2_grpc::humata::v1::channel_service_server::{
        ChannelService, ChannelServiceServer,
    };
    use crate::humata2_grpc::humata::v1::daemon_service_server::{
        DaemonService, DaemonServiceServer,
    };
    use crate::humata2_grpc::humata::v1::event_service_server::{
        EventService, EventServiceServer,
    };
    use crate::humata2_grpc::humata::v1::operation_service_server::{
        OperationService, OperationServiceServer,
    };

    struct SocketGuard(PathBuf);

    impl Drop for SocketGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[derive(Clone)]
    struct MockDaemon {
        watch_opened: Arc<Notify>,
        release_clone: Arc<Notify>,
    }

    fn setup_operation(state: pb::OperationState, clone_state: pb::StepState, version: u64) -> pb::Operation {
        pb::Operation {
            id: Some(pb::OperationId {
                value: "op_setup_1".to_owned(),
            }),
            state: state as i32,
            version,
            cancellable: state == pb::OperationState::Running,
            steps: vec![
                pb::OperationStep {
                    key: "record".to_owned(),
                    title: "Saving the channel".to_owned(),
                    state: pb::StepState::Done as i32,
                    ..Default::default()
                },
                pb::OperationStep {
                    key: "clone_main:acme/app".to_owned(),
                    title: "Cloning acme/app".to_owned(),
                    state: clone_state as i32,
                    done_units: u64::from(clone_state == pb::StepState::Done),
                    total_units: 1,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    #[tonic::async_trait]
    impl DaemonService for MockDaemon {
        async fn get_server_info(
            &self,
            _request: Request<pb::GetServerInfoRequest>,
        ) -> Result<Response<pb::ServerInfo>, Status> {
            Ok(Response::new(pb::ServerInfo {
                machine: Some(pb::MachineId {
                    value: "ma_test".to_owned(),
                }),
                ..Default::default()
            }))
        }

        async fn who_am_i(
            &self,
            _request: Request<pb::WhoAmIRequest>,
        ) -> Result<Response<pb::WhoAmIResponse>, Status> {
            Err(Status::unimplemented(""))
        }

        async fn begin_relay(
            &self,
            _request: Request<pb::BeginRelayRequest>,
        ) -> Result<Response<pb::BeginRelayResponse>, Status> {
            Err(Status::unimplemented(""))
        }

        async fn complete_relay(
            &self,
            _request: Request<pb::CompleteRelayRequest>,
        ) -> Result<Response<pb::CompleteRelayResponse>, Status> {
            Err(Status::unimplemented(""))
        }

        type PeerUplinkStream = ReceiverStream<Result<pb::MachineToHost, Status>>;

        async fn peer_uplink(
            &self,
            _request: Request<Streaming<pb::HostToMachine>>,
        ) -> Result<Response<Self::PeerUplinkStream>, Status> {
            Err(Status::unimplemented(""))
        }

        async fn repair_restart(
            &self,
            _request: Request<pb::RepairRestartRequest>,
        ) -> Result<Response<pb::RepairRestartResponse>, Status> {
            Err(Status::unimplemented(""))
        }

        async fn stop_sessions(
            &self,
            _request: Request<pb::StopSessionsRequest>,
        ) -> Result<Response<pb::StopSessionsResponse>, Status> {
            Err(Status::unimplemented(""))
        }

        async fn start_sessions(
            &self,
            _request: Request<pb::StartSessionsRequest>,
        ) -> Result<Response<pb::StartSessionsResponse>, Status> {
            Err(Status::unimplemented(""))
        }

        async fn upgrade(
            &self,
            _request: Request<pb::UpgradeRequest>,
        ) -> Result<Response<pb::UpgradeResponse>, Status> {
            Err(Status::unimplemented(""))
        }
    }

    #[tonic::async_trait]
    impl ChannelService for MockDaemon {
        async fn create_channel(
            &self,
            request: Request<pb::CreateChannelRequest>,
        ) -> Result<Response<pb::CreateChannelResponse>, Status> {
            let name = request.into_inner().name;
            Ok(Response::new(pb::CreateChannelResponse {
                channel: Some(pb::Channel {
                    id: Some(pb::ChannelId {
                        value: "ch_1".to_owned(),
                    }),
                    name,
                    folder: "q4-billing".to_owned(),
                    ..Default::default()
                }),
                setup: Some(setup_operation(
                    pb::OperationState::Running,
                    pb::StepState::Running,
                    1,
                )),
            }))
        }

        async fn get_channel(
            &self,
            _request: Request<pb::GetChannelRequest>,
        ) -> Result<Response<pb::Channel>, Status> {
            Err(Status::unimplemented(""))
        }

        async fn list_channels(
            &self,
            _request: Request<pb::ListChannelsRequest>,
        ) -> Result<Response<pb::ListChannelsResponse>, Status> {
            Err(Status::unimplemented(""))
        }

        async fn rename_channel(
            &self,
            _request: Request<pb::RenameChannelRequest>,
        ) -> Result<Response<pb::Channel>, Status> {
            Err(Status::unimplemented(""))
        }

        async fn search_git_hub_repos(
            &self,
            _request: Request<pb::SearchGitHubReposRequest>,
        ) -> Result<Response<pb::SearchGitHubReposResponse>, Status> {
            Err(Status::unimplemented(""))
        }

        async fn add_repo(
            &self,
            _request: Request<pb::AddRepoRequest>,
        ) -> Result<Response<pb::Operation>, Status> {
            Err(Status::unimplemented(""))
        }

        async fn plan_remove_repo(
            &self,
            _request: Request<pb::PlanRemoveRepoRequest>,
        ) -> Result<Response<pb::RemoveRepoPlan>, Status> {
            Err(Status::unimplemented(""))
        }

        async fn remove_repo(
            &self,
            _request: Request<pb::RemoveRepoRequest>,
        ) -> Result<Response<pb::Channel>, Status> {
            Err(Status::unimplemented(""))
        }

        async fn add_machine(
            &self,
            _request: Request<pb::AddChannelMachineRequest>,
        ) -> Result<Response<pb::Operation>, Status> {
            Err(Status::unimplemented(""))
        }

        async fn plan_remove_machine(
            &self,
            _request: Request<pb::PlanRemoveChannelMachineRequest>,
        ) -> Result<Response<pb::RemoveChannelMachinePlan>, Status> {
            Err(Status::unimplemented(""))
        }

        async fn remove_machine(
            &self,
            _request: Request<pb::RemoveChannelMachineRequest>,
        ) -> Result<Response<pb::Operation>, Status> {
            Err(Status::unimplemented(""))
        }

        async fn get_grid(
            &self,
            _request: Request<pb::GetGridRequest>,
        ) -> Result<Response<pb::ChannelGrid>, Status> {
            Err(Status::unimplemented(""))
        }

        async fn plan_delete_channel(
            &self,
            _request: Request<pb::PlanDeleteChannelRequest>,
        ) -> Result<Response<pb::DeleteChannelPlan>, Status> {
            Err(Status::unimplemented(""))
        }

        async fn delete_channel(
            &self,
            _request: Request<pb::DeleteChannelRequest>,
        ) -> Result<Response<pb::DeleteChannelResponse>, Status> {
            Err(Status::unimplemented(""))
        }
    }

    #[tonic::async_trait]
    impl OperationService for MockDaemon {
        async fn get_operation(
            &self,
            _request: Request<pb::GetOperationRequest>,
        ) -> Result<Response<pb::Operation>, Status> {
            Ok(Response::new(setup_operation(
                pb::OperationState::Succeeded,
                pb::StepState::Done,
                2,
            )))
        }

        async fn find_operation(
            &self,
            _request: Request<pb::FindOperationRequest>,
        ) -> Result<Response<pb::Operation>, Status> {
            Err(Status::unimplemented(""))
        }

        async fn list_operations(
            &self,
            _request: Request<pb::ListOperationsRequest>,
        ) -> Result<Response<pb::ListOperationsResponse>, Status> {
            Err(Status::unimplemented(""))
        }

        type WatchOperationStream = ReceiverStream<Result<pb::Operation, Status>>;

        async fn watch_operation(
            &self,
            _request: Request<pb::WatchOperationRequest>,
        ) -> Result<Response<Self::WatchOperationStream>, Status> {
            let (tx, rx) = mpsc::channel(4);
            let watch_opened = self.watch_opened.clone();
            let release_clone = self.release_clone.clone();
            tokio::spawn(async move {
                watch_opened.notify_one();
                release_clone.notified().await;
                let _ = tx
                    .send(Ok(setup_operation(
                        pb::OperationState::Succeeded,
                        pb::StepState::Done,
                        2,
                    )))
                    .await;
            });
            Ok(Response::new(ReceiverStream::new(rx)))
        }

        async fn cancel_operation(
            &self,
            _request: Request<pb::CancelOperationRequest>,
        ) -> Result<Response<pb::Operation>, Status> {
            Err(Status::unimplemented(""))
        }

        async fn retry_operation(
            &self,
            _request: Request<pb::RetryOperationRequest>,
        ) -> Result<Response<pb::Operation>, Status> {
            Err(Status::unimplemented(""))
        }
    }

    #[tonic::async_trait]
    impl EventService for MockDaemon {
        type SubscribeStream = ReceiverStream<Result<pb::Event, Status>>;

        async fn subscribe(
            &self,
            _request: Request<pb::SubscribeRequest>,
        ) -> Result<Response<Self::SubscribeStream>, Status> {
            let (tx, rx) = mpsc::channel(4);
            // Hold the sender alive so the subscription stream stays open.
            tokio::spawn(async move {
                let _keep = tx;
                std::future::pending::<()>().await;
            });
            Ok(Response::new(ReceiverStream::new(rx)))
        }
    }

    async fn next_setup(
        events: &mut humata2_clients::Events,
    ) -> humata2_clients::model::ChannelSetup {
        tokio::time::timeout(Duration::from_secs(2), async {
            while let Some(item) = events.recv().await {
                match item.expect("event ok") {
                    Event::ChannelSetup(setup) => return setup,
                    Event::Resync => continue,
                    other => panic!("unexpected event while waiting for ChannelSetup: {other:?}"),
                }
            }
            panic!("event stream closed before ChannelSetup arrived");
        })
        .await
        .expect("timed out waiting for ChannelSetup event")
    }

    #[tokio::test]
    async fn create_channel_returns_without_blocking_on_clone_completion() {
        let socket_path = std::env::temp_dir().join(format!(
            "humata-live-daemon-create-channel-{}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&socket_path);
        let _guard = SocketGuard(socket_path.clone());
        let listener = UnixListener::bind(&socket_path).expect("bind test unix socket");

        let watch_opened = Arc::new(Notify::new());
        let release_clone = Arc::new(Notify::new());
        let service = MockDaemon {
            watch_opened: watch_opened.clone(),
            release_clone: release_clone.clone(),
        };
        let server = tokio::spawn(async move {
            let _ = tonic::transport::Server::builder()
                .add_service(DaemonServiceServer::new(service.clone()))
                .add_service(ChannelServiceServer::new(service.clone()))
                .add_service(EventServiceServer::new(service.clone()))
                .add_service(OperationServiceServer::new(service))
                .serve_with_incoming(UnixListenerStream::new(listener))
                .await;
        });

        let daemon = LiveDaemon::connect(LiveOptions::new(socket_path))
            .await
            .expect("connect to mock daemon");
        let mut events = daemon.events().await.expect("subscribe to daemon events");

        // `release_clone` has NOT been notified yet: `create_channel` must still
        // return the new channel id immediately without waiting for the clone.
        let id = tokio::time::timeout(
            Duration::from_secs(2),
            daemon.create_channel(
                "Q4 Billing",
                vec!["acme/app".to_owned()],
                vec![MachineId::new("ma_test")],
            ),
        )
        .await
        .expect("create_channel must not block on clone completion")
        .expect("create_channel succeeds");
        assert_eq!(id, ChannelId::new("ch_1"));

        // The initial running setup event is published right away.
        let initial = next_setup(&mut events).await;
        assert_eq!(initial.name, "Q4 Billing");
        assert!(matches!(
            initial.steps.get(1).map(|step| &step.state),
            Some(SetupState::Running { .. })
        ));

        // The background watcher opens `WatchOperation` and streams completion
        // asynchronously once the clone finishes.
        tokio::time::timeout(Duration::from_secs(2), watch_opened.notified())
            .await
            .expect("background task opens WatchOperation");
        release_clone.notify_one();

        let done = next_setup(&mut events).await;
        assert_eq!(done.name, "Q4 Billing");
        assert_eq!(
            done.steps.get(1).map(|step| &step.state),
            Some(&SetupState::Done)
        );

        server.abort();
    }
}

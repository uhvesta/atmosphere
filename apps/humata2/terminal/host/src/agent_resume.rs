//! Opt-in conversation recovery after confirmed process loss, never SSH guessing.
use super::*;
use std::sync::Arc;
use terminal_provider::{ResumeIdentity, ResumePolicy, ResumeRegistry};
use tokio::sync::broadcast;

const RESERVATION_PREFIX: &str = "agent-resume:";
const SOURCE_ATTRIBUTE: &str = "terminal.resume.source";
const PROFILE_ATTRIBUTE: &str = "terminal.resume.profile";

pub(super) fn saved_profile(saved: &terminal_session::SavedSession) -> Option<String> {
    if saved
        .startup_profile
        .as_deref()
        .is_some_and(|key| key.starts_with(RESERVATION_PREFIX))
    {
        saved.attributes.get(PROFILE_ATTRIBUTE).cloned()
    } else {
        saved.startup_profile.clone()
    }
}

pub(super) fn saved_history(
    saved: &terminal_session::SavedSession,
    actor: &Actor,
) -> HistoryPolicy {
    // A crash may precede the actor's separate settings save. The atomic
    // reservation still identifies its parent and trusted profile; never
    // replace the parent's explicit Off policy with a global Recent default.
    if saved
        .startup_profile
        .as_deref()
        .is_some_and(|key| key.starts_with(RESERVATION_PREFIX))
    {
        if let Some(source) = saved.attributes.get(SOURCE_ATTRIBUTE) {
            if let Some(setting) = actor
                .store
                .as_ref()
                .and_then(|store| {
                    store
                        .sessions()
                        .iter()
                        .find(|parent| &parent.uuid == source)
                })
                .and_then(|parent| actor.settings.get(&parent.id))
            {
                return setting.history.clone();
            }
        }
        // Missing lineage is not permission to start persisting output.
        return HistoryPolicy::Off;
    }
    saved_profile(saved)
        .and_then(|name| actor.profiles.get(&name))
        .map(|profile| profile.history.clone())
        .unwrap_or_else(|| actor.default_history.clone())
}

/// Trusted configuration supplied again when the native host starts. Launch
/// arguments and executable paths are never taken from a provider event.
#[derive(Default)]
pub struct AgentResumeConfig {
    pub registry: Arc<ResumeRegistry>,
    pub profiles: BTreeMap<String, ResumePolicy>,
}

/// A process launch/attachment receipt, not proof that the provider accepted
/// the conversation or completed a turn. Confirm that through structured events.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentResumeLaunch {
    pub source_uuid: String,
    pub terminal_uuid: String,
    pub terminal_id: u64,
    pub provider: String,
    pub session_id: String,
    /// A previous durable reservation was found. It was not launched again;
    /// inspect this terminal's state before treating it as a live process.
    pub existing: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum AgentResumeError {
    #[error(
        "Terminal {uuid} does not exist on this host. Select its owning host before recovering the agent."
    )]
    Missing { uuid: String },
    #[error(
        "Terminal {uuid} is still live or stopping. Reattach to it; do not start a duplicate agent."
    )]
    Live { uuid: String },
    #[error(
        "Terminal {uuid} has no saved provider session ID. Supply a structured session-start event before enabling conversation recovery; a display checkpoint cannot identify a conversation."
    )]
    MissingIdentity { uuid: String },
    #[error(
        "Terminal {uuid} has no enabled, configured recovery profile. Configure explicit resume-only arguments for its trusted launcher before retrying."
    )]
    Disabled { uuid: String },
    #[error(
        "Recovery profile {profile} now names another machine. Restore the original endpoint or explicitly migrate its provider state before recovering."
    )]
    EndpointChanged { profile: String },
    #[error(
        "Saved agent directory {path:?} must be absolute and valid on the original machine. Correct its explicit recovery metadata; no replacement was launched."
    )]
    Directory { path: String },
    #[error(
        "The native host lost its workspace owner after a failed recovery worker. Inspect the durable reservation and restart the native host; reconnecting only the viewer cannot repair this state."
    )]
    StoreUnavailable,
    #[error(
        "An agent launch is already in progress. Wait for its outcome before requesting another recovery."
    )]
    Busy,
    #[error(
        "Terminal {uuid} already has recovery reservation {id}, whose process or saved conversation identity is not confirmed. Inspect that record; it will not be launched a second time."
    )]
    Reservation { uuid: String, id: u64 },
    #[error(
        "The host closed during recovery. Inspect its durable terminal records after reconnecting; do not automatically retry an uncertain launch."
    )]
    Closed,
    #[error(
        "Agent recovery arguments were rejected before launch. Check the exact saved session ID and configured resume adapter."
    )]
    Plan {
        #[source]
        source: terminal_provider::ResumeError,
    },
    #[error(
        "Agent recovery failed during {stage} for reserved terminal {id:?}. Inspect that durable reservation before retrying; a process may already exist."
    )]
    Session {
        id: Option<u64>,
        stage: &'static str,
        #[source]
        source: terminal_session::Error,
    },
    #[error(
        "Agent recovery terminal {id} could not initialize its display or save recovery settings. Preserve its record and fix the underlying problem before retrying."
    )]
    Host {
        id: u64,
        #[source]
        source: Error,
    },
    #[error(
        "The recovery worker failed. Inspect the reserved terminal and restart the native host; reconnecting only the viewer cannot restore workspace ownership."
    )]
    Worker {
        #[source]
        source: tokio::task::JoinError,
    },
}

#[derive(Clone, Debug)]
pub enum AgentResumeObservation {
    Launched(AgentResumeLaunch),
    Failed {
        source_uuid: String,
        error: Arc<AgentResumeError>,
    },
    Lagged {
        dropped: u64,
    },
}
pub struct AgentResumeObserver {
    receiver: broadcast::Receiver<AgentResumeObservation>,
}
impl AgentResumeObserver {
    pub async fn recv(&mut self) -> Option<AgentResumeObservation> {
        match self.receiver.recv().await {
            Ok(event) => Some(event),
            Err(broadcast::error::RecvError::Lagged(dropped)) => {
                Some(AgentResumeObservation::Lagged { dropped })
            }
            Err(broadcast::error::RecvError::Closed) => None,
        }
    }
}

pub(super) struct AgentResumes {
    config: AgentResumeConfig,
    attempted: BTreeSet<u64>,
    events: broadcast::Sender<AgentResumeObservation>,
    closing: bool,
    in_flight: bool,
    store_failed: bool,
}
impl AgentResumes {
    pub(super) fn new(
        config: AgentResumeConfig,
        events: broadcast::Sender<AgentResumeObservation>,
    ) -> Self {
        Self {
            config,
            attempted: BTreeSet::new(),
            events,
            closing: false,
            in_flight: false,
            store_failed: false,
        }
    }
    pub(super) fn close(&mut self) {
        self.closing = true;
    }
}

pub(super) struct Launch {
    source_uuid: String,
    profile: Profile,
    hint: RecoveryHint,
    spec: SessionSpec,
    reservation: String,
}
pub(super) enum AgentResumeCommand {
    /// Native callers attest loss themselves; a viewer has no such authority.
    ConfirmedLoss {
        uuid: String,
        reply: oneshot::Sender<Result<AgentResumeLaunch, Arc<AgentResumeError>>>,
    },
    WorkerFailed {
        source_uuid: String,
        error: AgentResumeError,
        reply: Option<oneshot::Sender<Result<AgentResumeLaunch, Arc<AgentResumeError>>>>,
    },
    Completed {
        store: SessionStore,
        launch: Launch,
        id: Option<u64>,
        attachment: Result<Attached, AgentResumeError>,
        reply: Option<oneshot::Sender<Result<AgentResumeLaunch, Arc<AgentResumeError>>>>,
    },
}

impl Host {
    /// Subscribe to bounded recovery outcomes without installing UI/OS alerts.
    pub fn observe_agent_resume(&self) -> AgentResumeObserver {
        AgentResumeObserver {
            receiver: self.resume_events.subscribe(),
        }
    }

    /// The native orchestrator attests that the old process cannot still run
    /// (for example, it observed a machine reboot). A failed SSH dial alone is
    /// not that evidence. Automatic recovery needs no call for a positively
    /// observed abnormal child exit. Both paths use one durable reservation per
    /// source UUID and preserve the original terminal as history.
    pub async fn resume_agent_after_confirmed_loss(
        &self,
        uuid: &str,
    ) -> Result<AgentResumeLaunch, Arc<AgentResumeError>> {
        let (reply, answer) = oneshot::channel();
        self.tx
            .send(Command::AgentResume(AgentResumeCommand::ConfirmedLoss {
                uuid: uuid.to_owned(),
                reply,
            }))
            .await
            .map_err(|_| Arc::new(AgentResumeError::Closed))?;
        answer
            .await
            .map_err(|_| Arc::new(AgentResumeError::Closed))?
    }
}

impl Actor {
    /// Save stop intent before the destructive write. Otherwise an abnormal
    /// stop code could trigger recovery after the native host restarts.
    pub(super) fn record_agent_stop(&mut self, id: u64) -> Result<(), Error> {
        let enabled = self
            .processes
            .get(&id)
            .and_then(|p| p.settings.profile.as_ref())
            .and_then(|name| self.agent_resumes.config.profiles.get(name))
            .is_some_and(|policy| !matches!(policy, ResumePolicy::Disabled));
        if !enabled {
            return Ok(());
        }
        let mut settings = self.settings.clone();
        let setting = settings.get_mut(&id).ok_or(Error::Invalid { reason: "stop recovery metadata is unavailable; reconnect before stopping this terminal" })?;
        setting.resume_blocked = true;
        self.recovery.save_settings(&settings)?;
        self.settings = settings;
        if let Some(p) = self.processes.get_mut(&id) {
            p.settings.resume_blocked = true;
        }
        Ok(())
    }

    pub(super) async fn settle_agent_resume(&mut self) -> Result<(), Error> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        while self.agent_resumes.in_flight {
            let command = tokio::time::timeout_at(deadline, self.rx.recv())
                .await
                .map_err(|source| Error::AgentResumePending { source })?
                .ok_or(Error::Closed)?;
            if let Command::Shutdown { reply } = command {
                let _ = reply.send(Err(Error::Closed));
                continue;
            }
            self.command(command).await;
            self.ready_snapshots();
        }
        Ok(())
    }

    pub(super) fn agent_resume_tick(&mut self) {
        if self.agent_resumes.closing
            || self.store.is_none()
            || self.agent_resumes.config.profiles.is_empty()
        {
            return;
        }
        // Only a holder's positive nonzero exit report permits automatic
        // recovery. Offline, missing socket and SSH errors never select this.
        let candidate = self.processes.iter().find_map(|(id, p)| {
            if p.stopping
                || p.settings.resume_blocked
                || !p.exited.is_some_and(|code| code != 0)
                || self.agent_resumes.attempted.contains(id)
            {
                return None;
            }
            let enabled = p
                .settings
                .profile
                .as_ref()
                .and_then(|name| self.agent_resumes.config.profiles.get(name))
                .is_some_and(|policy| !matches!(policy, ResumePolicy::Disabled));
            let child = self
                .store
                .as_ref()
                .and_then(|store| store.session(*id).ok())
                .and_then(|saved| saved.startup_profile.as_deref())
                .is_some_and(|key| key.starts_with(RESERVATION_PREFIX));
            (enabled && !child).then_some(*id)
        });
        if let Some(id) = candidate {
            self.begin_agent_resume(id, None);
        }
    }

    fn prepare_agent_resume(&self, id: u64) -> Result<Launch, AgentResumeError> {
        let p = self
            .processes
            .get(&id)
            .ok_or_else(|| AgentResumeError::Missing {
                uuid: id.to_string(),
            })?;
        if p.stopping || matches!(p.info.state, SessionState::Running) {
            return Err(AgentResumeError::Live {
                uuid: p.info.uuid.clone(),
            });
        }
        let store = self.store.as_ref().ok_or_else(|| {
            if self.agent_resumes.store_failed {
                AgentResumeError::StoreUnavailable
            } else {
                AgentResumeError::Busy
            }
        })?;
        let saved = store
            .session(id)
            .map_err(|source| AgentResumeError::Session {
                id: Some(id),
                stage: "read source",
                source,
            })?;
        let hint =
            p.settings
                .recovery
                .clone()
                .ok_or_else(|| AgentResumeError::MissingIdentity {
                    uuid: p.info.uuid.clone(),
                })?;
        let name = p
            .settings
            .profile
            .as_ref()
            .ok_or_else(|| AgentResumeError::Disabled {
                uuid: p.info.uuid.clone(),
            })?;
        let policy = self
            .agent_resumes
            .config
            .profiles
            .get(name)
            .ok_or_else(|| AgentResumeError::Disabled {
                uuid: p.info.uuid.clone(),
            })?;
        let mut profile =
            self.profiles
                .get(name)
                .cloned()
                .ok_or_else(|| AgentResumeError::Disabled {
                    uuid: p.info.uuid.clone(),
                })?;
        if profile.endpoint != saved.spec.endpoint {
            return Err(AgentResumeError::EndpointChanged {
                profile: name.clone(),
            });
        }
        let plan = self
            .agent_resumes
            .config
            .registry
            .plan(
                policy,
                &ResumeIdentity {
                    provider: hint.provider.clone(),
                    session_id: hint.session_id.clone(),
                    working_directory: hint.working_directory.clone(),
                },
            )
            .map_err(|source| AgentResumeError::Plan { source })?;
        let cwd = PathBuf::from(&plan.working_directory);
        if !cwd.is_absolute() {
            return Err(AgentResumeError::Directory {
                path: plan.working_directory,
            });
        }
        profile.args = plan.args;
        profile.cwd = cwd;
        profile.title = clean(&format!("{} · resumed", p.info.title), 120);
        profile.recovery = Some(hint.clone());
        profile.history = p.settings.history.clone();
        let mut spec = profile.spec();
        spec.cols = p.vt.size().cols;
        spec.rows = p.vt.size().rows;
        Ok(Launch {
            source_uuid: p.info.uuid.clone(),
            profile,
            hint,
            spec,
            reservation: format!("{RESERVATION_PREFIX}{}", p.info.uuid),
        })
    }

    fn begin_agent_resume(
        &mut self,
        id: u64,
        reply: Option<oneshot::Sender<Result<AgentResumeLaunch, Arc<AgentResumeError>>>>,
    ) {
        let launch = match self.prepare_agent_resume(id) {
            Ok(launch) => launch,
            Err(error) => {
                self.agent_resumes.attempted.insert(id);
                let uuid = self
                    .processes
                    .get(&id)
                    .map(|p| p.info.uuid.clone())
                    .unwrap_or_default();
                self.agent_resume_failed(uuid, error, reply);
                return;
            }
        };
        // Reservation is written by SessionStore BEFORE spawning. A retry or
        // host restart finds it even if the first worker never reported back.
        if let Some(saved) = self.store.as_ref().and_then(|store| {
            store
                .sessions()
                .iter()
                .find(|saved| saved.startup_profile.as_deref() == Some(&launch.reservation))
        }) {
            let same_identity = saved.recovery.as_ref().is_some_and(|hint| {
                hint.provider == launch.hint.provider && hint.session_id == launch.hint.session_id
            });
            let live = self
                .processes
                .get(&saved.id)
                .is_some_and(|p| matches!(p.info.state, SessionState::Running) && !p.stopping);
            if !same_identity || !live {
                let error = AgentResumeError::Reservation {
                    uuid: launch.source_uuid.clone(),
                    id: saved.id,
                };
                self.agent_resumes.attempted.insert(id);
                self.agent_resume_failed(launch.source_uuid, error, reply);
                return;
            }
            let outcome = AgentResumeLaunch {
                source_uuid: launch.source_uuid,
                terminal_uuid: saved.uuid.clone(),
                terminal_id: saved.id,
                provider: launch.hint.provider,
                session_id: launch.hint.session_id,
                existing: true,
            };
            self.agent_resumes.attempted.insert(id);
            let _ = self
                .agent_resumes
                .events
                .send(AgentResumeObservation::Launched(outcome.clone()));
            if let Some(reply) = reply {
                let _ = reply.send(Ok(outcome));
            }
            return;
        }
        let Some(mut store) = self.store.take() else {
            if let Some(reply) = reply {
                let _ = reply.send(Err(Arc::new(AgentResumeError::Busy)));
            }
            return;
        };
        self.agent_resumes.attempted.insert(id);
        self.reconnects.halt(id);
        self.agent_resumes.in_flight = true;
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let spec = launch.spec.clone();
            let reservation = launch.reservation.clone();
            let hint = launch.hint.clone();
            let source_uuid = launch.source_uuid.clone();
            let launch_profile = launch.profile.name.clone();
            let worker = tokio::task::spawn_blocking(move || {
                let attrs = BTreeMap::from([
                    (SOURCE_ATTRIBUTE.to_owned(), source_uuid),
                    (PROFILE_ATTRIBUTE.to_owned(), launch_profile),
                ]);
                let started =
                    store.create_startup_with_metadata(spec, &reservation, Some(hint), attrs);
                let id = started.as_ref().ok().copied().or_else(|| {
                    store
                        .sessions()
                        .iter()
                        .find(|saved| saved.startup_profile.as_deref() == Some(&reservation))
                        .map(|s| s.id)
                });
                let started = started.map_err(|source| AgentResumeError::Session {
                    id,
                    stage: "reserve and launch",
                    source,
                });
                (store, id, started)
            })
            .await;
            match worker {
                Ok((store, id, started)) => {
                    let attachment = match started {
                        Ok(id) => store.attach_live_from(id, 0).await.map_err(|source| {
                            AgentResumeError::Session {
                                id: Some(id),
                                stage: "attach resumed process",
                                source,
                            }
                        }),
                        Err(error) => Err(error),
                    };
                    let _ = tx
                        .send(Command::AgentResume(AgentResumeCommand::Completed {
                            store,
                            launch,
                            id,
                            attachment,
                            reply,
                        }))
                        .await;
                }
                Err(source) => {
                    // Store ownership is lost with the failed worker. Existing
                    // processes stay alive; no second launch is attempted.
                    let _ = tx
                        .send(Command::AgentResume(AgentResumeCommand::WorkerFailed {
                            source_uuid: launch.source_uuid,
                            error: AgentResumeError::Worker { source },
                            reply,
                        }))
                        .await;
                }
            }
        });
    }

    pub(super) fn agent_resume_command(&mut self, command: AgentResumeCommand) {
        match command {
            AgentResumeCommand::WorkerFailed {
                source_uuid,
                error,
                reply,
            } => {
                self.agent_resumes.in_flight = false;
                self.agent_resumes.store_failed = true;
                self.failure(None, "agent_resume_worker", "The native host lost its workspace owner during agent recovery. Inspect its durable reservation and restart the native host; reconnecting only a viewer cannot repair it.");
                self.agent_resume_failed(source_uuid, error, reply);
            }
            AgentResumeCommand::ConfirmedLoss { uuid, reply } => {
                if self.agent_resumes.closing {
                    let _ = reply.send(Err(Arc::new(AgentResumeError::Closed)));
                    return;
                }
                let id = self
                    .processes
                    .iter()
                    .find_map(|(id, p)| (p.info.uuid == uuid).then_some(*id));
                match id {
                    Some(id) => self.begin_agent_resume(id, Some(reply)),
                    None => {
                        let _ = reply.send(Err(Arc::new(AgentResumeError::Missing { uuid })));
                    }
                }
            }
            AgentResumeCommand::Completed {
                store,
                launch,
                id,
                attachment,
                reply,
            } => {
                self.agent_resumes.in_flight = false;
                self.store = Some(store);
                self.persist_cwds();
                let (attached, mut error) = match attachment {
                    Ok(a) => (Some(a), None),
                    Err(e) => (None, Some(e)),
                };
                if let Some(id) = id {
                    let settings = Settings {
                        resume_blocked: false,
                        foreground_session: Some(launch.hint.session_id.clone()),
                        profile: Some(launch.profile.name),
                        recovery: Some(launch.hint.clone()),
                        history: launch.profile.history,
                    };
                    let result = new_terminal(&launch.spec, history_limits(&settings.history).0)
                        .map_err(|source| AgentResumeError::Host { id, source });
                    match result {
                        Ok(vt) => {
                            self.add(
                                id,
                                &launch.spec,
                                settings,
                                vt,
                                0,
                                None,
                                attached,
                                ReplyCursor::default(),
                            );
                            if let Err(source) = self.recovery.save_settings(&self.settings) {
                                error = Some(AgentResumeError::Host { id, source });
                            }
                        }
                        Err(e) => error = Some(e),
                    }
                    self.sessions();
                    if let Some(error) = error {
                        self.agent_resume_failed(launch.source_uuid, error, reply);
                    } else if let Some(p) = self.processes.get(&id) {
                        let outcome = AgentResumeLaunch {
                            source_uuid: launch.source_uuid,
                            terminal_uuid: p.info.uuid.clone(),
                            terminal_id: id,
                            provider: launch.hint.provider,
                            session_id: launch.hint.session_id,
                            existing: false,
                        };
                        let _ = self
                            .agent_resumes
                            .events
                            .send(AgentResumeObservation::Launched(outcome.clone()));
                        if let Some(reply) = reply {
                            let _ = reply.send(Ok(outcome));
                        }
                    } else if let Some(reply) = reply {
                        let _ = reply.send(Err(Arc::new(AgentResumeError::Closed)));
                    }
                } else {
                    self.agent_resume_failed(
                        launch.source_uuid,
                        error.unwrap_or(AgentResumeError::Closed),
                        reply,
                    );
                }
            }
        }
    }

    fn agent_resume_failed(
        &self,
        source_uuid: String,
        error: AgentResumeError,
        reply: Option<oneshot::Sender<Result<AgentResumeLaunch, Arc<AgentResumeError>>>>,
    ) {
        // Consumers choose notification policy. No provider text, terminal
        // input, pending input or original prompt is included in this event.
        let error = Arc::new(error);
        let _ = self
            .agent_resumes
            .events
            .send(AgentResumeObservation::Failed {
                source_uuid,
                error: error.clone(),
            });
        if let Some(reply) = reply {
            let _ = reply.send(Err(error));
        }
    }
}

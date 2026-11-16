//! Fresh sibling PTYs from trusted launch intent; no process-state cloning.
use super::*;
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Clone, Default)]
pub enum SpawnProfile {
    #[default]
    Same,
    Named(String),
}
#[derive(Clone, Default)]
pub enum SpawnCwd {
    /// Require a valid OSC7 report from the source's canonical terminal state.
    #[default]
    RequireObserved,
    /// Explicitly accept the last-known saved/launch directory when no OSC7 exists.
    AllowLastKnown,
    Explicit(PathBuf),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CwdSource {
    Observed,
    LastKnown,
    Explicit,
}
#[derive(Default)]
pub struct SpawnFromOptions {
    pub profile: SpawnProfile,
    pub cwd: SpawnCwd,
    pub title: Option<String>,
    /// None inherits the source policy, including Off. A host-wide Off remains sticky.
    pub history: Option<HistoryPolicy>,
}
#[derive(Debug)]
pub struct Spawned {
    pub id: u64,
    pub uuid: String,
    pub cwd: PathBuf,
    pub cwd_source: CwdSource,
}
#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    #[error(
        "Source terminal {id} does not exist. Select an existing terminal before opening a sibling."
    )]
    Missing { id: u64 },
    #[error(
        "Sibling {id} was reserved but its durable identity is unavailable. Preserve the workspace and inspect this record before launching another process."
    )]
    Identity { id: u64 },
    #[error(
        "Source terminal {id} is offline, stopping or still hydrating. Reattach and wait for its live state before opening a sibling."
    )]
    Unavailable { id: u64 },
    #[error(
        "The source launch profile is no longer configured. Configure the trusted profile or choose an explicit configured profile."
    )]
    ProfileUnavailable {
        source_id: u64,
        profile: Option<String>,
    },
    #[error(
        "Source terminal {id} has no observed current directory. Supply an explicit directory, enable shell OSC7 integration or deliberately allow its last-known directory."
    )]
    CwdUnavailable { id: u64 },
    #[error("Sibling directory {} must be an absolute path without control characters. Supply a directory valid on the source machine.",path.display())]
    InvalidCwd { path: PathBuf },
    #[error("Another terminal is starting. Wait for that launch to finish before retrying.")]
    Busy,
    #[error(
        "The host is unavailable. Reconnect before opening a sibling; do not retry an ambiguous launch without checking session records."
    )]
    HostUnavailable,
    #[error(
        "The host closed after sibling {id} was reserved. Its process may be running; inspect that session when reconnecting before retrying the launch."
    )]
    HostAfterReservation { id: u64 },
    #[error("Sibling options were rejected before launch.")]
    Options {
        #[source]
        source: Error,
    },
    #[error(
        "Sibling {id:?} failed during {stage}. Inspect the reserved session record before retrying; a process may already exist."
    )]
    Launch {
        id: Option<u64>,
        stage: &'static str,
        #[source]
        source: terminal_session::Error,
    },
    #[error(
        "Sibling {id} was reserved, but its terminal screen could not be allocated. Inspect that session before trying another launch."
    )]
    Engine {
        id: u64,
        #[source]
        source: Error,
    },
    #[error(
        "Sibling {id} exists, but its recovery metadata could not be saved. Fix state-folder permissions and inspect this session before another launch."
    )]
    Persist {
        id: u64,
        #[source]
        source: Error,
    },
    #[error(
        "The sibling launch worker failed. Restart the host and inspect its existing sessions before retrying."
    )]
    Worker {
        #[source]
        source: tokio::task::JoinError,
    },
}
pub(super) enum SpawnCommand {
    Request {
        source: u64,
        options: SpawnFromOptions,
        reserved: Arc<AtomicU64>,
        reply: oneshot::Sender<Result<Spawned, SpawnError>>,
    },
    Completed {
        store: SessionStore,
        profile: Profile,
        id: Option<u64>,
        attached: Result<Attached, SpawnError>,
        cwd_source: CwdSource,
        reply: oneshot::Sender<Result<Spawned, SpawnError>>,
    },
}
impl Host {
    /// Open a fresh PTY on the same machine using configured launch intent.
    /// No history, pending input, process memory or provider ID is inherited.
    /// Explicit arguments in a configured profile are used as configured.
    pub async fn spawn_from(
        &self,
        source: u64,
        options: SpawnFromOptions,
    ) -> Result<Spawned, SpawnError> {
        let (reply, answer) = oneshot::channel();
        let reserved = Arc::new(AtomicU64::new(0));
        self.tx
            .send(Command::Spawn(SpawnCommand::Request {
                source,
                options,
                reserved: reserved.clone(),
                reply,
            }))
            .await
            .map_err(|_| SpawnError::HostUnavailable)?;
        answer
            .await
            .map_err(|_| match reserved.load(Ordering::Acquire) {
                0 => SpawnError::HostUnavailable,
                id => SpawnError::HostAfterReservation { id },
            })?
    }
}
fn usable_cwd(path: &Path) -> bool {
    path.is_absolute()
        && path
            .to_str()
            .is_some_and(|s| !s.is_empty() && s.len() <= 4096 && !s.chars().any(char::is_control))
}
impl Actor {
    pub(super) fn spawn_command(&mut self, command: SpawnCommand) {
        match command {
            SpawnCommand::Request {
                source,
                options,
                reserved,
                reply,
            } => {
                let result = self.prepare_spawn(source, options);
                let (profile, cwd_source) = match result {
                    Ok(v) => v,
                    Err(error) => {
                        let _ = reply.send(Err(error));
                        return;
                    }
                };
                let Some(mut store) = self.store.take() else {
                    let _ = reply.send(Err(SpawnError::Busy));
                    return;
                };
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let launch = profile.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        let before: BTreeSet<_> = store.sessions().iter().map(|s| s.id).collect();
                        let launched = store.create(launch.spec());
                        let id = launched.as_ref().ok().copied().or_else(|| {
                            store
                                .sessions()
                                .iter()
                                .find(|s| !before.contains(&s.id))
                                .map(|s| s.id)
                        });
                        (store, launched, id)
                    })
                    .await;
                    match result {
                        Ok((store, launched, id)) => {
                            if let Some(id) = id {
                                reserved.store(id, Ordering::Release);
                            }
                            let attached = match launched {
                                Ok(id) => store.attach_live_from(id, 0).await.map_err(|source| {
                                    SpawnError::Launch {
                                        id: Some(id),
                                        stage: "attach",
                                        source,
                                    }
                                }),
                                Err(source) => Err(SpawnError::Launch {
                                    id,
                                    stage: "launch",
                                    source,
                                }),
                            };
                            let _ = tx
                                .send(Command::Spawn(SpawnCommand::Completed {
                                    store,
                                    profile,
                                    id,
                                    attached,
                                    cwd_source,
                                    reply,
                                }))
                                .await;
                        }
                        Err(source) => {
                            let _ = reply.send(Err(SpawnError::Worker { source }));
                        }
                    }
                });
            }
            SpawnCommand::Completed {
                store,
                profile,
                id,
                attached,
                cwd_source,
                reply,
            } => {
                self.store = Some(store);
                self.persist_cwds();
                let (attachment, error) = match attached {
                    Ok(a) => (Some(a), None),
                    Err(e) => (None, Some(e)),
                };
                if let Some(id) = id {
                    let uuid = self
                        .store
                        .as_ref()
                        .and_then(|store| store.sessions().iter().find(|s| s.id == id))
                        .map(|saved| saved.uuid.clone());
                    let Some(uuid) = uuid else {
                        let _ = reply.send(Err(SpawnError::Identity { id }));
                        return;
                    };
                    let spec = profile.spec();
                    let settings = Settings {
                        resume_blocked: false,
                        foreground_session: None,
                        profile: Some(profile.name),
                        recovery: None,
                        history: profile.history,
                    };
                    match new_terminal(&spec, history_limits(&settings.history).0) {
                        Ok(vt) => {
                            self.add(
                                id,
                                &spec,
                                settings,
                                vt,
                                0,
                                None,
                                attachment,
                                ReplyCursor::default(),
                            );
                            let saved = self.recovery.save_settings(&self.settings);
                            self.sessions();
                            let outcome = if let Some(error) = error {
                                Err(error)
                            } else {
                                saved
                                    .map_err(|source| SpawnError::Persist { id, source })
                                    .map(|_| Spawned {
                                        id,
                                        uuid,
                                        cwd: spec.cwd,
                                        cwd_source,
                                    })
                            };
                            let _ = reply.send(outcome);
                        }
                        Err(source) => {
                            let _ = reply.send(Err(SpawnError::Engine { id, source }));
                        }
                    }
                } else {
                    let _ = reply.send(Err(error.unwrap_or(SpawnError::HostUnavailable)));
                }
            }
        }
    }
    fn prepare_spawn(
        &self,
        source: u64,
        options: SpawnFromOptions,
    ) -> Result<(Profile, CwdSource), SpawnError> {
        let p = self
            .processes
            .get(&source)
            .ok_or(SpawnError::Missing { id: source })?;
        if p.writer.is_none() || p.stopping || p.exited.is_some() || p.offset < p.replay_end {
            return Err(SpawnError::Unavailable { id: source });
        }
        let store = self.store.as_ref().ok_or(SpawnError::Busy)?;
        let saved = store
            .sessions()
            .iter()
            .find(|s| s.id == source)
            .ok_or(SpawnError::Missing { id: source })?;
        let name = match options.profile {
            SpawnProfile::Same => {
                p.settings
                    .profile
                    .clone()
                    .ok_or(SpawnError::ProfileUnavailable {
                        source_id: source,
                        profile: None,
                    })?
            }
            SpawnProfile::Named(name) => name,
        };
        let mut profile =
            self.profiles
                .get(&name)
                .cloned()
                .ok_or_else(|| SpawnError::ProfileUnavailable {
                    source_id: source,
                    profile: Some(name.clone()),
                })?;
        let observed = p.vt.pwd().as_deref().and_then(reported_cwd);
        let (cwd, cwd_source) = match options.cwd {
            SpawnCwd::Explicit(path) => (path, CwdSource::Explicit),
            SpawnCwd::RequireObserved => (
                observed.ok_or(SpawnError::CwdUnavailable { id: source })?,
                CwdSource::Observed,
            ),
            SpawnCwd::AllowLastKnown => match observed {
                Some(path) => (path, CwdSource::Observed),
                None => (PathBuf::from(&p.info.cwd), CwdSource::LastKnown),
            },
        };
        if !usable_cwd(&cwd) {
            return Err(SpawnError::InvalidCwd { path: cwd });
        }
        profile.cwd = cwd;
        profile.endpoint = saved.spec.endpoint.clone();
        profile.recovery = None;
        profile.history = if self.history_disabled {
            HistoryPolicy::Off
        } else {
            options
                .history
                .unwrap_or_else(|| p.settings.history.clone())
        };
        validate_history(&profile.history).map_err(|source| SpawnError::Options { source })?;
        if let Some(title) = options.title {
            if title.is_empty() || title.len() > 120 || title.chars().any(char::is_control) {
                return Err(SpawnError::Options {
                    source: Error::Invalid {
                        reason: "sibling title must contain 1–120 bytes without control characters",
                    },
                });
            }
            profile.title = title;
        }
        if self.processes.len() >= 128 {
            return Err(SpawnError::Options {
                source: Error::Invalid {
                    reason: "this host retains 128 session records; use another host workspace",
                },
            });
        }
        Ok((profile, cwd_source))
    }
}

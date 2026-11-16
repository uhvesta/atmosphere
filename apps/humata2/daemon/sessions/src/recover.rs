//! Finishing or undoing what a crash cut short (PER-27): channels whose
//! main clones were still being made, and sessions whose start hadn't
//! reached its end. Run after revive's reconcile at startup, so an agent
//! whose launch was recorded is already running again.

use std::collections::HashSet;
use std::fs;

use humata2_revive::Env;
use humata2_store::{MainClone, SessionRecord, StartStep, Store, StoreError};

use crate::error::StartError;
use crate::layout::partial;
use crate::setup::clone_mains;
use crate::start::{Starter, blocking};

/// What recovery did for one channel or session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recovery {
    /// The channel's main clones were finished.
    ChannelFinished { channel: String },
    /// The channel's setup couldn't be finished; it stays as it was.
    ChannelStuck { channel: String, reason: String },
    /// The session's start was finished and its agent runs.
    SessionFinished { session: String },
    /// The session's start couldn't be finished, so the session, its
    /// folder and its clones were removed; `reason` says why.
    SessionUndone { session: String, reason: String },
    /// The session couldn't even be undone; `reason` says what is left.
    SessionStuck { session: String, reason: String },
}

/// The channels and sessions the last run left, read before this one
/// serves anything. Recovery touches only these: a channel or session made
/// since is being set up or started by its own call, and finishing it here
/// too would clone or launch it twice.
#[derive(Clone, Debug, Default)]
pub struct Leftovers {
    channels: HashSet<String>,
    sessions: HashSet<String>,
}

impl Leftovers {
    /// Reads what the store holds now. Call it before the daemon listens.
    pub fn read(store: &Store) -> Result<Self, StoreError> {
        Ok(Self {
            channels: store
                .channels()?
                .into_iter()
                .map(|channel| channel.id)
                .collect(),
            sessions: store
                .sessions()?
                .into_iter()
                .map(|session| session.id)
                .collect(),
        })
    }
}

/// Finishes or undoes every unfinished channel setup and session start
/// among `left`, after clearing what a crash left of own copies (FK-9).
pub async fn recover(starter: &Starter, env: Env<'_>, left: &Leftovers) -> Vec<Recovery> {
    let mut done = Vec::new();
    remove_own_copy_partials(starter).await;
    recover_channels(starter, left, &mut done).await;
    recover_sessions(starter, env, left, &mut done).await;
    done
}

/// Removes the partial copies that own copies of the code leave beside a
/// channel's session folders, as `sessions/<folder>.partial` (FK-9). A
/// session start's partials are one folder deeper and are left alone. Best
/// effort: nothing is reported, and a channel with no sessions folder has
/// nothing to remove.
async fn remove_own_copy_partials(starter: &Starter) {
    let this = starter.clone();
    let Ok(Ok(channels)) = blocking(move || this.store.channels()).await else {
        return;
    };
    let this = starter.clone();
    let Ok(Ok(named)) = blocking(move || this.store.sessions()).await else {
        return;
    };
    let named: std::collections::HashSet<std::path::PathBuf> =
        named.into_iter().map(|session| session.folder).collect();
    for channel in channels {
        let sessions = starter.layout.sessions(&channel.folder);
        let named = named.clone();
        let _ = blocking(move || {
            let Ok(entries) = fs::read_dir(&sessions) else {
                return;
            };
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().ends_with(".partial") {
                    // The conversation copy it made, if it got that far.
                    // A copy that can't be removed keeps its note, so the next
                    // start tries again.
                    let note = crate::own_copy::own_copy_note(&entry.path());
                    if crate::fork::remove_noted_copy(&note).is_ok() {
                        let _ = fs::remove_dir_all(entry.path());
                    }
                } else if !named.contains(&entry.path()) {
                    // A folder renamed into place that no record names: an own
                    // copy cut before its records were saved. Only its note
                    // says so; any other folder has none.
                    let note = crate::own_copy::own_copy_note(&entry.path());
                    if note.exists() && crate::fork::remove_noted_copy(&note).is_ok() {
                        let _ = fs::remove_dir_all(entry.path());
                    }
                }
            }
        })
        .await;
    }
}

async fn recover_channels(starter: &Starter, left: &Leftovers, done: &mut Vec<Recovery>) {
    let this = starter.clone();
    let listed = blocking(move || this.store.channels()).await;
    let channels = match listed {
        Ok(Ok(channels)) => channels,
        Ok(Err(error)) => {
            done.push(Recovery::ChannelStuck {
                channel: String::new(),
                reason: error.to_string(),
            });
            return;
        }
        Err(error) => {
            done.push(Recovery::ChannelStuck {
                channel: String::new(),
                reason: error.to_string(),
            });
            return;
        }
    };
    for channel in channels
        .into_iter()
        .filter(|channel| left.channels.contains(&channel.id))
    {
        if !channel.repos.iter().any(|r| r.main == MainClone::Cloning) {
            continue;
        }
        let this = starter.clone();
        let (id, folder) = (channel.id.clone(), channel.folder.clone());
        let result = blocking(move || {
            this.layout
                .create_channel(&folder)
                .map_err(|e| e.to_string())?;
            clone_mains(
                &this.store,
                &this.layout,
                &this.remote_template,
                &id,
                |_, _| {},
            )
            .map_err(|e| e.to_string())
        })
        .await;
        done.push(match result {
            Ok(Ok(())) => Recovery::ChannelFinished {
                channel: channel.id,
            },
            Ok(Err(reason)) => Recovery::ChannelStuck {
                channel: channel.id,
                reason,
            },
            Err(error) => Recovery::ChannelStuck {
                channel: channel.id,
                reason: error.to_string(),
            },
        });
    }
}

async fn recover_sessions(
    starter: &Starter,
    env: Env<'_>,
    left: &Leftovers,
    done: &mut Vec<Recovery>,
) {
    let this = starter.clone();
    let listed = blocking(move || this.store.session_details()).await;
    let details = match listed {
        Ok(Ok(details)) => details,
        Ok(Err(error)) => {
            done.push(Recovery::SessionStuck {
                session: String::new(),
                reason: error.to_string(),
            });
            return;
        }
        Err(error) => {
            done.push(Recovery::SessionStuck {
                session: String::new(),
                reason: error.to_string(),
            });
            return;
        }
    };
    remove_unrecorded(starter, &details, left, done).await;
    for detail in details
        .into_iter()
        .filter(|d| d.start != StartStep::Done && left.sessions.contains(&d.session))
    {
        let this = starter.clone();
        let id = detail.session.clone();
        let loaded = blocking(move || (this.store.session(&id), this.store.agent(&id))).await;
        let (session, agent) = match loaded {
            Ok((Ok(Some(session)), Ok(agent))) => (session, agent),
            _ => continue,
        };
        let id = session.id.clone();
        let this = starter.clone();
        let forked = blocking(move || this.store.fork_origin(&id)).await;
        if agent.is_none() && matches!(forked, Ok(Ok(Some(_)))) {
            // A fork's code is a copy of its parent's, so cloning the rest
            // from main would make something else: take it back.
            let reason = "its fork was cut short before its agent started".to_owned();
            done.push(match starter.undo(&session).await {
                Ok(()) => Recovery::SessionUndone {
                    session: session.id,
                    reason,
                },
                Err(undo) => Recovery::SessionStuck {
                    session: session.id,
                    reason: format!("{reason}; and removing it failed: {undo}"),
                },
            });
            continue;
        }
        let outcome = if agent.is_some() {
            // The launch was recorded, and revive has brought it back.
            let mut detail = detail;
            starter.set_step(&mut detail, StartStep::Done).await
        } else {
            finish(starter, &session, detail, env).await
        };
        done.push(match outcome {
            Ok(()) => Recovery::SessionFinished {
                session: session.id,
            },
            Err(error) => {
                let reason = format!("{error}");
                match starter.undo(&session).await {
                    Ok(()) => Recovery::SessionUndone {
                        session: session.id,
                        reason,
                    },
                    Err(undo) => Recovery::SessionStuck {
                        session: session.id,
                        reason: format!("{reason}; and removing it failed: {undo}"),
                    },
                }
            }
        });
    }
}

/// Re-clones what is missing, then launches.
async fn finish(
    starter: &Starter,
    session: &SessionRecord,
    mut detail: humata2_store::SessionDetail,
    env: Env<'_>,
) -> Result<(), StartError> {
    let this = starter.clone();
    let channel_id = session.channel.clone();
    let channel = blocking(move || this.store.channel(&channel_id))
        .await?
        .map_err(|source| StartError::Store {
            doing: "read the channel",
            source,
        })?
        .ok_or(StartError::NoChannel {
            id: session.channel.clone(),
        })?;
    if detail.start == StartStep::Recorded {
        // A clone cut short left a partial next to where it was going.
        for repo in &detail.repos {
            let stale = partial(&repo.path);
            let _ = blocking(move || std::fs::remove_dir_all(stale)).await;
        }
        starter.clone_repos(&channel, &detail, &mut |_| {}).await?;
        starter.set_step(&mut detail, StartStep::Cloned).await?;
    }
    starter
        .launch_session(session, detail, 80, 24, env, &mut |_| {})
        .await
        .map(|_| ())
}

/// A session saved but cut off before its start was recorded has no
/// detail, agent or tab, and its folder is missing or empty: nothing
/// else could have made it, so it is removed.
async fn remove_unrecorded(
    starter: &Starter,
    details: &[humata2_store::SessionDetail],
    left: &Leftovers,
    done: &mut Vec<Recovery>,
) {
    let this = starter.clone();
    let listed = blocking(move || {
        (
            this.store.sessions(),
            this.store.agents(),
            this.store.tabs(),
        )
    })
    .await;
    let Ok((Ok(sessions), Ok(agents), Ok(tabs))) = listed else {
        return;
    };
    for session in sessions
        .into_iter()
        .filter(|session| left.sessions.contains(&session.id))
    {
        let has = details.iter().any(|d| d.session == session.id)
            || agents.iter().any(|a| a.session == session.id)
            || tabs
                .iter()
                .any(|t| t.session.as_deref() == Some(session.id.as_str()));
        let empty = std::fs::read_dir(&session.folder).map_or(true, |mut d| d.next().is_none());
        if has || !empty {
            continue;
        }
        done.push(match starter.undo(&session).await {
            Ok(()) => Recovery::SessionUndone {
                session: session.id,
                reason: "its start was cut short before it was recorded".to_owned(),
            },
            Err(error) => Recovery::SessionStuck {
                session: session.id,
                reason: error.to_string(),
            },
        });
    }
}

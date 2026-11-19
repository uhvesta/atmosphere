//! Finishing or undoing what a crash cut short (PER-27): channels whose
//! main clones were still being made, and sessions whose start hadn't
//! reached its end. Run after revive's reconcile at startup, so an agent
//! whose launch was recorded is already running again.

use std::collections::HashSet;
use std::fs;

use humata2_revive::Env;
use humata2_store::{
    AgentRecord, MainClone, SessionDetail, SessionRecord, StartStep, Store, StoreError, TabRecord,
};

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

    /// The sessions among these whose start `recover` will finish or undo:
    /// those whose start hadn't reached its end, and those saved but cut
    /// off before their start was recorded. Read with [`Leftovers::read`],
    /// before the daemon listens, so the restart report can name each one
    /// as owed before it is written (D241).
    pub fn cut_starts(&self, store: &Store) -> Result<Vec<String>, StoreError> {
        let details = store.session_details()?;
        let (sessions, agents, tabs) = (store.sessions()?, store.agents()?, store.tabs()?);
        let mut cut: Vec<String> = details
            .iter()
            .filter(|detail| detail.start != StartStep::Done && self.sessions.contains(&detail.session))
            .map(|detail| detail.session.clone())
            .collect();
        cut.extend(
            sessions
                .iter()
                .filter(|session| self.sessions.contains(&session.id) && unrecorded(session, &details, &agents, &tabs))
                .map(|session| session.id.clone()),
        );
        Ok(cut)
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
/// The copy notes in `folder` when they are all it holds, each named as
/// the note it is (a note cut while written is its `.tmp`); `None` when
/// the folder holds anything else, or nothing.
fn copy_notes_only(folder: &std::path::Path) -> Option<Vec<std::path::PathBuf>> {
    let mut entries = fs::read_dir(folder).ok()?;
    let only = entries.next()?.ok()?;
    if entries.next().is_some() || only.file_name() != ".humata" {
        return None;
    }
    let mut notes = Vec::new();
    for entry in fs::read_dir(only.path()).ok()? {
        let name = entry.ok()?.file_name();
        let name = name.to_str()?;
        if !name.starts_with("fork-copy-") {
            return None;
        }
        notes.push(only.path().join(name.strip_suffix(".tmp").unwrap_or(name)));
    }
    notes.sort();
    notes.dedup();
    (!notes.is_empty()).then_some(notes)
}

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
                    if note.exists() {
                        if crate::fork::remove_noted_copy(&note).is_ok() {
                            let _ = fs::remove_dir_all(entry.path());
                        }
                    } else if let Some(notes) = copy_notes_only(&entry.path()) {
                        // A continue cut after its copy note and before its
                        // records: the folder holds the note and nothing else.
                        if notes.iter().all(|note| crate::fork::remove_noted_copy(note).is_ok()) {
                            let _ = fs::remove_dir_all(entry.path());
                        }
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
        let is_fork = matches!(forked, Ok(Ok(Some(_))));
        // A session continued from another one (CV-8) keeps its conversation
        // copy noted until its agent starts.
        let note = crate::fork::copy_note(&session.folder, &session.id);
        let continued = blocking(move || note.exists()).await.unwrap_or(false);
        if agent.is_none() && (is_fork || continued) {
            // A fork's code is a copy of its parent's, so cloning the rest
            // from main would make something else: take it back. A
            // continued session without its conversation would be another
            // session than the one asked for: take it back too.
            let reason = if is_fork {
                "its fork was cut short before its agent started".to_owned()
            } else {
                "it was cut short while copying another session's conversation, before its agent started"
                    .to_owned()
            };
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
            // The launch was recorded, and revive has brought it back. The
            // agent record names any conversation copy now, so its note goes
            // as it does after a launch that wasn't cut.
            let (folder, id) = (session.folder.clone(), session.id.clone());
            let _ = blocking(move || crate::fork::forget_note(&crate::fork::copy_note(&folder, &id))).await;
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
    let (cols, rows) = (humata2_revive::START_COLS, humata2_revive::START_ROWS);
    starter
        .launch_session(session, detail, cols, rows, env, &mut |_| {})
        .await
        .map(|_| ())
}

/// Whether `session` was saved and then cut off before its start was
/// recorded: no detail, agent or tab, and a folder missing or empty.
fn unrecorded(session: &SessionRecord, details: &[SessionDetail], agents: &[AgentRecord], tabs: &[TabRecord]) -> bool {
    let has = details.iter().any(|d| d.session == session.id)
        || agents.iter().any(|a| a.session == session.id)
        || tabs
            .iter()
            .any(|t| t.session.as_deref() == Some(session.id.as_str()));
    let empty = std::fs::read_dir(&session.folder).map_or(true, |mut d| d.next().is_none());
    !has && empty
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
        if !unrecorded(&session, details, &agents, &tabs) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str, folder: &std::path::Path) -> SessionRecord {
        SessionRecord {
            id: id.to_owned(),
            channel: "desk".to_owned(),
            machine: String::new(),
            title: id.to_owned(),
            agent: "claude".to_owned(),
            folder: folder.to_owned(),
            created_unix_ms: 1,
        }
    }

    fn detail(id: &str, start: StartStep) -> SessionDetail {
        SessionDetail {
            session: id.to_owned(),
            repos: Vec::new(),
            model: None,
            first_message: None,
            start,
            operation: None,
            revision: 1,
        }
    }

    /// The sessions recovery will finish or undo are named, and no others:
    /// not a finished start, not an unrecorded session whose folder holds
    /// someone's work, not a session made after the leftovers were read.
    #[test]
    fn the_starts_recovery_will_finish_or_undo_are_named_and_no_others() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("state")).unwrap();
        let folders = dir.path().join("folders");
        for (id, step) in [("se_cut", Some(StartStep::Launching)), ("se_done", Some(StartStep::Done)), ("se_bare", None), ("se_used", None)] {
            let folder = folders.join(id);
            fs::create_dir_all(&folder).unwrap();
            if id == "se_used" {
                fs::write(folder.join("notes.md"), "mine\n").unwrap();
            }
            store.put_session(&session(id, &folder)).unwrap();
            if let Some(step) = step {
                store.put_session_detail(&detail(id, step)).unwrap();
            }
        }
        let left = Leftovers::read(&store).unwrap();
        let later = folders.join("se_later");
        fs::create_dir_all(&later).unwrap();
        store.put_session(&session("se_later", &later)).unwrap();
        store.put_session_detail(&detail("se_later", StartStep::Recorded)).unwrap();

        let mut cut = left.cut_starts(&store).unwrap();
        cut.sort();

        assert_eq!(cut, vec!["se_bare".to_owned(), "se_cut".to_owned()]);
    }
}

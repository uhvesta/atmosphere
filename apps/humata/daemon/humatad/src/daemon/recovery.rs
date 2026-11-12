//! Recovery copies of sessions on other machines: after each turn the Mac
//! keeps the session's record, history and backend state, and the commits
//! of its snapshots in a mirror per repository, so a host that loses a
//! session, or is lost itself, can have it set up again, or another host
//! can carry it on.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use humata_agent::bundle;
use humata_wire::{ClientError, pb};

use super::Daemon;
use crate::convert;
use crate::error::{DaemonError, StoreError};

impl Daemon {
    /// The folder of `session`'s mirrors: its project's, which its other
    /// sessions share, or its own outside a project.
    fn mirrors(&self, session: &str, project: Option<&str>) -> PathBuf {
        match project {
            Some(project) => self.recovery.join("projects").join(project),
            None => self.recovery.join("sessions").join(session),
        }
    }

    /// Backs up `session` on `host` in the background, unless its copy
    /// already has `turns` turns. A backup asked for while one runs runs
    /// again after it, so the copy ends up with the last.
    pub(super) fn back_up_soon(&self, host: &str, session: &str, turns: Option<u32>) {
        if host.is_empty() {
            return;
        }
        if let Some(turns) = turns {
            let kept = self.store.recovery_turns(session).ok().flatten();
            if kept.is_some_and(|kept| kept >= turns) {
                return;
            }
        }
        {
            let mut backing_up = self.lock_backing_up();
            if let Some(again) = backing_up.get_mut(session) {
                *again = true;
                return;
            }
            backing_up.insert(session.to_owned(), false);
        }
        let Some(daemon) = self.me.upgrade() else {
            return;
        };
        let (host, session) = (host.to_owned(), session.to_owned());
        tokio::spawn(async move {
            loop {
                // A copy that couldn't be made stays as it was; the next
                // turn tries again.
                if let Err(error) = daemon.back_up(&host, &session).await {
                    eprintln!(
                        "humatad: couldn't back up session {session} on {host}: {}",
                        humata_wire::describe(&error)
                    );
                }
                let mut backing_up = daemon.lock_backing_up();
                if backing_up.get(&session) == Some(&true) {
                    backing_up.insert(session.clone(), false);
                } else {
                    backing_up.remove(&session);
                    return;
                }
            }
        });
    }

    fn lock_backing_up(&self) -> std::sync::MutexGuard<'_, HashMap<String, bool>> {
        self.backing_up
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Deletes an archived session: on its host, then its recovery copy
    /// and everything the daemon kept of it.
    pub(super) async fn delete_session(&self, id: &str) -> Result<(), DaemonError> {
        let sessions = self.list("").await?.sessions;
        let session = sessions
            .iter()
            .find(|session| session.id == id)
            .ok_or_else(|| StoreError::UnknownSession { id: id.to_owned() })?;
        let name = session.name.clone();
        if !session.archived {
            return Err(DaemonError::DeleteUnarchived { name });
        }
        if let Some(fork) = sessions.iter().find(|fork| fork.parent_id == id) {
            let fork = fork.name.clone();
            return Err(DaemonError::DeleteForked { name, fork });
        }
        if session.host_offline {
            let host = session.host.clone();
            return Err(DaemonError::DeleteOffline { name, host });
        }
        let project = convert::optional(session.project_id.clone());
        let request = pb::request::Kind::DeleteSession(pb::DeleteSession {
            session_id: id.to_owned(),
        });
        match self.ask(&session.host, request).await? {
            pb::response::Kind::SessionDeleted(_) => {}
            _ => return Err(unexpected(&session.host, "the session deleted")),
        }
        let folder = self.mirrors(id, project.as_deref());
        if project.is_some() {
            for (_, mirror) in mirrors_in(&folder) {
                humata_agent::forget(&mirror, id)
                    .await
                    .map_err(|source| DaemonError::Mirror {
                        session: id.to_owned(),
                        path: mirror.clone(),
                        source,
                    })?;
            }
        } else if folder.exists() {
            std::fs::remove_dir_all(&folder).map_err(|source| DaemonError::DeleteRecovery {
                session: id.to_owned(),
                path: folder.clone(),
                source,
            })?;
        }
        self.store.forget_session(id)?;
        Ok(())
    }

    /// Takes `session`'s backup from `host`, its new commits into the
    /// mirrors and the rest into the store.
    async fn back_up(&self, host: &str, session: &str) -> Result<(), DaemonError> {
        let project = self
            .store
            .placement(session)?
            .and_then(|placement| placement.project);
        let folder = self.mirrors(session, project.as_deref());
        let mut haves = Vec::new();
        for (name, mirror) in mirrors_in(&folder) {
            // A mirror that can't be read sends everything again.
            let commits = bundle::tips(&mirror).await.unwrap_or_default();
            haves.push(pb::RepositoryCommits { name, commits });
        }
        let request = pb::request::Kind::BackupSession(pb::BackupSession {
            session_id: session.to_owned(),
            haves,
        });
        let mut backup = match self.ask(host, request).await? {
            pb::response::Kind::SessionBackup(backup) => backup,
            _ => return Err(unexpected(host, "the session's backup")),
        };
        for repository in std::mem::take(&mut backup.bundles) {
            if !is_folder_name(&repository.name) {
                return Err(DaemonError::BadRepositoryName {
                    host: host.to_owned(),
                    name: repository.name,
                });
            }
            let mirror = folder.join(format!("{}.git", repository.name));
            let kept = async {
                bundle::mirror(&mirror).await?;
                bundle::apply(&mirror, &repository.bundle).await
            };
            kept.await.map_err(|source| DaemonError::Mirror {
                session: session.to_owned(),
                path: mirror.clone(),
                source,
            })?;
        }
        let name = record_name(&backup.record);
        self.store.keep_recovery(session, host, &name, &backup)?;
        Ok(())
    }

    /// Every recovery copy, and whether its host still has the session.
    pub(super) async fn recoveries(&self) -> Result<pb::Recoveries, DaemonError> {
        let sessions = self.list("").await?.sessions;
        let listed: HashMap<(&str, &str), &pb::SessionInfo> = sessions
            .iter()
            .map(|session| ((session.id.as_str(), session.host.as_str()), session))
            .collect();
        let recoveries = self
            .store
            .recoveries()?
            .into_iter()
            .map(|recovery| {
                let found = listed.get(&(recovery.session.as_str(), recovery.host.as_str()));
                pb::Recovery {
                    turns: found.map_or(recovery.turns, |session| session.turns),
                    on_host: found.is_some(),
                    session_id: recovery.session,
                    name: recovery.name,
                    host: recovery.host,
                    backed_up_turns: recovery.turns,
                    backed_up_unix_ms: recovery.backed_up_unix_ms,
                }
            })
            .collect();
        Ok(pb::Recoveries { recoveries })
    }

    /// Sets `session` up again on `target` ("" for this Mac) from its
    /// recovery copy: on the host it was on, if that host lost it, or on
    /// another, which it then runs on.
    pub(super) async fn restore_session(
        &self,
        session: &str,
        target: &str,
    ) -> Result<pb::SessionInfo, DaemonError> {
        // Moved while where it is answers: it must be between turns, and
        // goes as it is now, not as its copy last was.
        let source = self.live_source(session, target).await?;
        if let Some(source) = &source {
            self.back_up(source, session).await?;
        }
        // A copy left on the target by an earlier move makes way.
        if self.store.moved_away(session, target)? {
            self.remove_copy(target, session).await;
        }
        let Some((from, mut backup)) = self.store.recovery(session)? else {
            return Err(DaemonError::NoRecovery {
                session: session.to_owned(),
            });
        };
        let (project, item) = self
            .store
            .placement(session)?
            .map(|placement| (placement.project, placement.item))
            .unwrap_or_default();
        let mut repositories = Vec::new();
        let mut tips: HashMap<String, Vec<String>> = HashMap::new();
        if let Some(project) = &project {
            let project = self.store.project(project)?;
            if target.is_empty() {
                for repository in &project.repositories {
                    // A repository that can't be read is sent everything.
                    let commits = bundle::tips(&repository.path).await.unwrap_or_default();
                    tips.insert(repository.name.clone(), commits);
                    repositories.push(convert::wire_repository(repository));
                }
            } else {
                let cloned = self.clone_on(target, &project).await?;
                repositories = cloned.repositories;
                tips.extend(
                    cloned
                        .tips
                        .into_iter()
                        .map(|tips| (tips.name, tips.commits)),
                );
            }
        }
        let folder = self.mirrors(session, project.as_deref());
        for repository in &repositories {
            let mirror = folder.join(format!("{}.git", repository.name));
            if !mirror.join("HEAD").is_file() {
                continue;
            }
            let known = tips.get(&repository.name).map_or(&[][..], Vec::as_slice);
            let made = bundle::create(&mirror, session, known)
                .await
                .map_err(|source| DaemonError::Mirror {
                    session: session.to_owned(),
                    path: mirror.clone(),
                    source,
                })?;
            if let Some(made) = made {
                backup.bundles.push(pb::RepositoryBundle {
                    name: repository.name.clone(),
                    bundle: made,
                });
            }
        }
        let request = pb::request::Kind::RestoreBackup(pb::RestoreBackup {
            backup: Some(backup),
            repositories,
        });
        let mut restored = match self.ask(target, request).await? {
            pb::response::Kind::SessionRestored(pb::SessionRestored {
                session: Some(restored),
            }) => restored,
            _ => return Err(unexpected(target, "the session restored")),
        };
        self.store.move_host(session, &from, target)?;
        self.own(&mut restored, target, project.as_deref(), item.as_deref())?;
        if let Some(source) = &source {
            self.remove_copy(source, session).await;
        }
        Ok(restored)
    }

    /// The host `session` is on, if it answers with it there, for a move to
    /// `target`. Refused while it's in a turn, already on `target`, or forked
    /// there, since its copy is deleted once moved.
    async fn live_source(
        &self,
        session: &str,
        target: &str,
    ) -> Result<Option<String>, DaemonError> {
        let Some(host) = self
            .store
            .placement(session)?
            .map(|placement| placement.host)
        else {
            return Ok(None);
        };
        let listed = match self
            .ask(
                &host,
                pb::request::Kind::ListSessions(pb::ListSessions::default()),
            )
            .await
        {
            Ok(pb::response::Kind::Sessions(listed)) => listed.sessions,
            // Gone, or not answering: moved from its copy, as it last was.
            _ => return Ok(None),
        };
        let Some(info) = listed.iter().find(|info| info.id == session) else {
            return Ok(None);
        };
        let name = info.name.clone();
        if host == target {
            let host = if host.is_empty() {
                "this Mac".to_owned()
            } else {
                host
            };
            return Err(DaemonError::AlreadyThere { name, host });
        }
        if matches!(
            info.state(),
            pb::SessionState::Working | pb::SessionState::WaitingForApproval
        ) {
            return Err(DaemonError::MoveWhileWorking { name });
        }
        if let Some(fork) = listed.iter().find(|fork| fork.parent_id == session) {
            let fork = fork.name.clone();
            return Err(DaemonError::MoveForked { name, fork });
        }
        Ok(Some(host))
    }

    /// Deletes the copy of `session` that `host` holds, which moved away.
    /// One that can't be deleted stays left out of the list.
    async fn remove_copy(&self, host: &str, session: &str) {
        let archive = pb::request::Kind::UpdateSession(pb::UpdateSession {
            session_id: session.to_owned(),
            archived: Some(true),
            ..Default::default()
        });
        let delete = pb::request::Kind::DeleteSession(pb::DeleteSession {
            session_id: session.to_owned(),
        });
        let removed = match self.ask(host, archive).await {
            Ok(_) => self.ask(host, delete).await.map(drop),
            Err(error) => Err(error),
        };
        if let Err(error) = removed {
            eprintln!(
                "humatad: couldn't delete the copy of session {session} left on {}: {}",
                if host.is_empty() { "this Mac" } else { host },
                humata_wire::describe(&error)
            );
        }
    }

    /// Restores every session `host` lost onto it again.
    pub(super) async fn restore_host(&self, host: &str) -> Result<pb::HostRestored, DaemonError> {
        let mut restored = Vec::new();
        let mut failed = Vec::new();
        let lost = self
            .recoveries()
            .await?
            .recoveries
            .into_iter()
            .filter(|recovery| recovery.host == host && !recovery.on_host);
        for recovery in lost {
            match self.restore_session(&recovery.session_id, host).await {
                Ok(session) => restored.push(session),
                Err(error) => failed.push(pb::RestoreFailure {
                    session_id: recovery.session_id,
                    message: humata_wire::describe(&error),
                }),
            }
        }
        Ok(pb::HostRestored { restored, failed })
    }
}

/// The mirrors in `folder`, by repository name.
fn mirrors_in(folder: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut mirrors: Vec<(String, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file_name = entry.file_name();
            let name = file_name.to_str()?.strip_suffix(".git")?;
            Some((name.to_owned(), entry.path()))
        })
        .collect();
    mirrors.sort();
    mirrors
}

/// Whether a host's name for a repository can name a folder of mirrors.
fn is_folder_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\'])
}

/// The session's name in its record's JSON; empty if it has none.
fn record_name(record: &[u8]) -> String {
    serde_json::from_slice::<serde_json::Value>(record)
        .ok()
        .and_then(|record| record.get("name")?.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn unexpected(host: &str, expected: &'static str) -> DaemonError {
    DaemonError::Host {
        host: host.to_owned(),
        source: ClientError::Unexpected {
            expected,
            got: "another answer".to_owned(),
        },
    }
}

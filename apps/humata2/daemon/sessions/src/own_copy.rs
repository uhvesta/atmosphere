//! Giving a context-only fork its own copy of the code (FK-9, D185, D187).
//!
//! A context-only fork works in its parent's folder, so its clones are the
//! parent's. This copies them, as they are now (working tree included), into
//! the fork's own folder on a new branch; copies the conversation to the path
//! the agent keys on for that folder and removes the old one once the fork's
//! records are saved; and starts a running agent again there with its
//! conversation. A fork has its own copy once its folder isn't its parent's,
//! so nothing else is recorded. The copy is built beside the folder
//! (`<folder>.partial`) and renamed into place only when the records are
//! saved, so a crash at any step leaves no folder behind (see `recover`).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use humata2_store::fault::point;
use humata2_revive::{Env, ReviveError, Revived, restart_agent};
use humata2_store::{
    AgentRecord, AgentState, ChannelRecord, SessionDetail, SessionRecord, StoreError,
};

use crate::agents::{AgentKind, LaunchPlan};
use crate::error::{AgentError, GitError, StartError};
use crate::fork::{copy_clone, forget_note, note_copy_at};
use crate::layout::partial;
use crate::names::Branch;
use crate::setup::unix_ms;
use crate::start::{RepoName, Starter, blocking};

/// Why a fork couldn't be given its own copy. The errors before the copy
/// change nothing; see [`Starter::give_own_copy`] for the rest.
#[derive(Debug, thiserror::Error)]
pub enum OwnCopyError {
    #[error("no session {session} is on this machine")]
    NoSession { session: String },
    #[error("session {session} isn't a fork, so it has no parent's code to copy")]
    NotAFork { session: String },
    #[error(
        "session {session}'s parent {parent} is no longer on this machine, so there is no code to copy from it"
    )]
    NoParent { session: String, parent: String },
    #[error("session {session} already works in its own copy of the code")]
    HasOwnCode { session: String },
    #[error("session {session} has no agent record, so its agent can't be moved")]
    NoAgent { session: String },
    #[error("session {session}'s agent is still starting or resuming")]
    Busy { session: String },
    #[error("couldn't find agent {agent} for the session's own copy")]
    Agent {
        agent: String,
        #[source]
        source: AgentError,
    },
    #[error("couldn't read the session's channel or choose its new branch")]
    Start {
        #[source]
        source: StartError,
    },
    #[error("couldn't {doing} for session {session}")]
    Store {
        session: String,
        doing: &'static str,
        #[source]
        source: StoreError,
    },
    #[error("couldn't copy {owner}/{name} into the session's own folder")]
    Git {
        owner: String,
        name: String,
        #[source]
        source: GitError,
    },
    #[error("couldn't write {path}")]
    Copy {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the fork's conversation file {path} is missing, so the conversation can't move")]
    NoConversation {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("session {session}'s terminal is still running after it was asked to stop")]
    HolderAnswers { session: String },
    #[error("couldn't start session {session}'s agent again in its own copy")]
    Resume {
        session: String,
        #[source]
        source: ReviveError,
    },
    #[error("session {session}'s agent didn't start again in its own copy: {what}")]
    NotResumed { session: String, what: &'static str },
}

impl From<StartError> for OwnCopyError {
    fn from(source: StartError) -> Self {
        Self::Start { source }
    }
}

/// What a copy works from, read once from the store.
struct Plan {
    kind: AgentKind,
    channel: ChannelRecord,
    session: SessionRecord,
    detail: SessionDetail,
    agent: AgentRecord,
}

impl Starter {
    /// Whether `fork` could be given its own copy now. Blocking: it reads the
    /// store.
    pub fn check_own_copy(&self, fork: &str) -> Result<(), OwnCopyError> {
        self.own_copy_plan(fork).map(drop)
    }

    /// Gives the context-only fork `fork` its own copy of the code (FK-9,
    /// D187): the parent's clones are copied as they are now into the fork's
    /// folder on a new branch, the conversation is copied there, and a running
    /// agent starts again there with its conversation. Returns whether the
    /// agent was started again.
    ///
    /// `progress` hears `copy_code:<repo>` for each copied repository,
    /// `move_conversation`, and `launch` and `verify` when the agent starts
    /// again. A failure removes the new folder and leaves the conversation
    /// where it was. An agent stopped to make the copy is started again in
    /// the parent's code, with the record it had, so it never says it runs
    /// while nothing does.
    pub async fn give_own_copy(
        &self,
        fork: &str,
        env: Env<'_>,
        progress: &mut (dyn FnMut(&str) + Send),
    ) -> Result<bool, OwnCopyError> {
        let (this, id) = (self.clone(), fork.to_owned());
        let plan = blocking(move || this.own_copy_plan(&id)).await??;

        let repos: Vec<RepoName> = plan
            .detail
            .repos
            .iter()
            .map(|repo| (repo.owner.clone(), repo.name.clone()))
            .collect();
        let (this, channel, title) = (
            self.clone(),
            plan.channel.clone(),
            plan.session.title.clone(),
        );
        let branch = blocking(move || this.pick(&channel, &repos, &title)).await??;
        let folder = self.layout.session(&plan.channel.folder, &branch.folder);

        let clones: Vec<(PathBuf, RepoName)> = plan
            .detail
            .repos
            .iter()
            .map(|repo| (repo.path.clone(), (repo.owner.clone(), repo.name.clone())))
            .collect();
        let (copy_folder, copy_branch) = (folder.clone(), branch.name.clone());
        blocking(move || copy_clones(&copy_folder, &clones, &copy_branch)).await??;
        for repo in &plan.detail.repos {
            progress(&format!("copy_code:{}", repo.name));
        }

        let running = matches!(plan.agent.state, AgentState::Running { .. });
        let agent = match self
            .relocate(&plan, &folder, &branch, running, progress)
            .await
        {
            Ok(agent) => agent,
            Err(error) => {
                discard(&folder).await;
                // An agent whose holder still answers never stopped, so it
                // is left alone.
                if running && !matches!(error, OwnCopyError::HolderAnswers { .. }) {
                    self.restart_in_parent(&plan, env).await;
                }
                return Err(error);
            }
        };
        if !running {
            return Ok(false);
        }
        progress("launch");
        let line = format!(
            "This session has its own copy of the code now, in {}. Resuming its conversation.",
            folder.display()
        );
        let revived = restart_agent(&self.store, &self.revive, agent, env, &line)
            .await
            .map_err(|source| OwnCopyError::Resume {
                session: fork.to_owned(),
                source,
            })?;
        match revived {
            Revived::Resumed { .. } | Revived::Reattached | Revived::Relaunched => {
                progress("verify");
                Ok(true)
            }
            Revived::ResumeFailed { .. } => Err(OwnCopyError::NotResumed {
                session: fork.to_owned(),
                what: "its conversation couldn't be resumed",
            }),
            Revived::Exited { .. } => Err(OwnCopyError::NotResumed {
                session: fork.to_owned(),
                what: "the agent exited as soon as it started",
            }),
            Revived::Left => Err(OwnCopyError::NotResumed {
                session: fork.to_owned(),
                what: "the agent wasn't due to run",
            }),
        }
    }

    /// Starts a stopped agent again in the parent's code, with the record it
    /// had before the copy was tried. Revive saves a failed start itself, so
    /// the record never says it runs while nothing does. The copy's error is
    /// the one reported, so an error here is ignored.
    async fn restart_in_parent(&self, plan: &Plan, env: Env<'_>) {
        let line =
            "Couldn't give this session its own copy of the code; it goes on in its parent's code.";
        let _ = restart_agent(&self.store, &self.revive, plan.agent.clone(), env, line).await;
    }

    /// The fork's records, checked: a context-only fork whose folder is still
    /// its parent's, with its agent not starting.
    fn own_copy_plan(&self, fork: &str) -> Result<Plan, OwnCopyError> {
        let origin = self
            .store
            .fork_origin(fork)
            .map_err(store_error(fork, "read where the fork came from"))?
            .ok_or_else(|| OwnCopyError::NotAFork {
                session: fork.to_owned(),
            })?;
        let session = self
            .store
            .session(fork)
            .map_err(store_error(fork, "read the session"))?
            .ok_or_else(|| OwnCopyError::NoSession {
                session: fork.to_owned(),
            })?;
        let parent = self
            .store
            .session(&origin.parent)
            .map_err(store_error(fork, "read the fork's parent"))?
            .ok_or_else(|| OwnCopyError::NoParent {
                session: fork.to_owned(),
                parent: origin.parent.clone(),
            })?;
        if session.folder != parent.folder {
            return Err(OwnCopyError::HasOwnCode {
                session: fork.to_owned(),
            });
        }
        let detail = self
            .store
            .session_detail(fork)
            .map_err(store_error(fork, "read the session's start"))?
            .ok_or_else(|| OwnCopyError::NoSession {
                session: fork.to_owned(),
            })?;
        let agent = self
            .store
            .agent(fork)
            .map_err(store_error(fork, "read the session's agent"))?
            .ok_or_else(|| OwnCopyError::NoAgent {
                session: fork.to_owned(),
            })?;
        if matches!(agent.state, AgentState::Launching | AgentState::Resuming) {
            return Err(OwnCopyError::Busy {
                session: fork.to_owned(),
            });
        }
        let kind = self
            .agents
            .get(&session.agent)
            .map_err(|source| OwnCopyError::Agent {
                agent: session.agent.clone(),
                source,
            })?
            .clone();
        let channel = self.channel_record(&session.channel)?;
        Ok(Plan {
            kind,
            channel,
            session,
            detail,
            agent,
        })
    }

    /// Stops the agent if it runs, copies its conversation to the path it
    /// keys on for the fork's folder, renames the copied folder into place and
    /// saves the fork's records there. Returns the agent's record as saved,
    /// for starting it again. Once the records are saved the conversation's
    /// old file is removed; before that, the old file is what the records
    /// point at, so a failure leaves it where it is.
    async fn relocate(
        &self,
        plan: &Plan,
        folder: &Path,
        branch: &Branch,
        running: bool,
        progress: &mut (dyn FnMut(&str) + Send),
    ) -> Result<AgentRecord, OwnCopyError> {
        let fork = plan.session.id.clone();
        if running {
            // Saved as being resumed before it is stopped: the stop is not
            // the agent ending, so its exit isn't saved as one, and a cut
            // from here on has the restart resume it (in the parent's code
            // until the copy's records are saved).
            self.put_agent_state(plan, AgentState::Resuming).await?;
            let socket = plan.agent.launch.holder_socket.clone();
            // A holder that is already gone is as stopped as one asked to stop.
            let _ = humata2_holder::shutdown(&socket).await;
            // Shutdown is only asked for: give the holder a moment to go.
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
            while humata2_holder::attach(&socket, u64::MAX).await.is_ok() {
                if tokio::time::Instant::now() >= deadline {
                    // It still runs: its record says so again.
                    self.put_agent_state(plan, plan.agent.state).await?;
                    return Err(OwnCopyError::HolderAnswers { session: fork });
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
        let launch = plan
            .kind
            .launch(&self.home, folder, &plan.agent.launch.conversation_id);
        let from = plan.agent.launch.conversation_file.clone();
        let to = launch.conversation_file.clone();

        let (copying_from, copying_to) = (from.clone(), to.clone());
        let note = own_copy_note(&partial(folder));
        if let Err(error) = blocking(move || {
            // Named before the copy exists: a cut leaves the partial folder,
            // which recovery clears, and the note says what else to remove.
            if copying_from != copying_to {
                note_copy_at(&note, &copying_to).map_err(|source| OwnCopyError::Copy {
                    path: note.clone(),
                    source,
                })?;
            }
            copy_conversation(&copying_from, &copying_to)
        })
        .await?
        {
            remove_copy(&from, &to).await;
            return Err(error);
        }
        progress("move_conversation");

        let (built, dest) = (partial(folder), folder.to_owned());
        if let Err(source) = blocking(move || fs::rename(built, dest)).await? {
            remove_copy(&from, &to).await;
            return Err(OwnCopyError::Copy {
                path: folder.to_owned(),
                source,
            });
        }

        // The folder is in place and no record names it yet: its note is what
        // lets `recover` take it back.
        point("own_copy.renamed");
        match self.save_own_copy(plan, folder, &branch.name, launch).await {
            Ok(agent) => {
                remove_old_conversation(&from, &to).await;
                // The records point at the copy now; its note, renamed into
                // the folder with the clones, is done with.
                let note = own_copy_note(folder);
                let _ = blocking(move || forget_note(&note)).await;
                Ok(agent)
            }
            Err(error) => {
                remove_copy(&from, &to).await;
                Err(error)
            }
        }
    }

    /// Saves the fork's agent record as it was planned, in `state`.
    async fn put_agent_state(&self, plan: &Plan, state: AgentState) -> Result<(), OwnCopyError> {
        let mut agent = plan.agent.clone();
        agent.state = state;
        agent.updated_unix_ms = unix_ms();
        let store = self.store.clone();
        blocking(move || store.put_agent(&agent))
            .await?
            .map_err(store_error(
                &plan.session.id,
                "save that the fork's agent is stopping for its own copy",
            ))
    }

    /// Saves the fork's session, its repositories on the new branch in its
    /// folder, and its agent launched there.
    async fn save_own_copy(
        &self,
        plan: &Plan,
        folder: &Path,
        branch: &str,
        launch: LaunchPlan,
    ) -> Result<AgentRecord, OwnCopyError> {
        let mut detail = plan.detail.clone();
        detail.revision += 1;
        for repo in &mut detail.repos {
            repo.branch = branch.to_owned();
            repo.path = folder.join(&repo.name);
        }
        let session = SessionRecord {
            folder: folder.to_owned(),
            ..plan.session.clone()
        };
        let mut agent = plan.agent.clone();
        agent.launch.folder = folder.to_owned();
        agent.launch.argv = launch.argv;
        agent.launch.resume_argv = launch.resume_argv;
        agent.launch.conversation_file = launch.conversation_file;
        agent.updated_unix_ms = unix_ms();
        let (store, saved_agent) = (self.store.clone(), agent.clone());
        blocking(move || -> Result<(), StoreError> {
            store.put_session(&session)?;
            store.put_session_detail(&detail)?;
            store.put_agent(&saved_agent)
        })
        .await?
        .map_err(store_error(&plan.session.id, "save the fork's own copy"))?;
        Ok(agent)
    }
}

fn store_error<'a>(
    session: &'a str,
    doing: &'static str,
) -> impl Fn(StoreError) -> OwnCopyError + 'a {
    move |source| OwnCopyError::Store {
        session: session.to_owned(),
        doing,
        source,
    }
}

/// Copies each clone in `repos` into `partial(folder)/<name>` on `branch`,
/// through partials of their own (see `copy_clone`). The partial folder is a
/// sibling of the session folders, so start-up clears one a crash left
/// (see `recover`). A failure removes it, so nothing is left half-copied.
fn copy_clones(
    folder: &Path,
    repos: &[(PathBuf, RepoName)],
    branch: &str,
) -> Result<(), OwnCopyError> {
    let built = partial(folder);
    let _ = fs::remove_dir_all(&built);
    for (src, (owner, name)) in repos {
        if let Err(source) = copy_clone(src, &built.join(name), branch) {
            let _ = fs::remove_dir_all(&built);
            return Err(OwnCopyError::Git {
                owner: owner.clone(),
                name: name.clone(),
                source,
            });
        }
    }
    Ok(())
}

/// Where, in an own copy being built in `dir`, the path of the conversation
/// copy made for it is noted until its records are saved.
pub(crate) fn own_copy_note(dir: &Path) -> PathBuf {
    dir.join(".humata").join("own-copy-conversation")
}

/// Copies a conversation file to where the agent keys it for the new folder.
/// When it's keyed the same way, it stays where it is: copying a file onto
/// itself would empty it.
fn copy_conversation(from: &Path, to: &Path) -> Result<(), OwnCopyError> {
    let missing = |source: io::Error| match source.kind() {
        io::ErrorKind::NotFound => OwnCopyError::NoConversation {
            path: from.to_owned(),
            source,
        },
        _ => OwnCopyError::Copy {
            path: from.to_owned(),
            source,
        },
    };
    if from == to {
        return fs::metadata(from).map(drop).map_err(missing);
    }
    if let Some(dir) = to.parent() {
        fs::create_dir_all(dir).map_err(|source| OwnCopyError::Copy {
            path: dir.to_owned(),
            source,
        })?;
    }
    fs::copy(from, to).map(drop).map_err(|source| match source.kind() {
        io::ErrorKind::NotFound => missing(source),
        _ => OwnCopyError::Copy {
            path: to.to_owned(),
            source,
        },
    })
}

/// Removes the copy of a conversation made for the new folder, when there is
/// one. Best effort: the error that stopped the copy is the one reported.
async fn remove_copy(from: &Path, to: &Path) {
    if from != to {
        let to = to.to_owned();
        let _ = blocking(move || fs::remove_file(to)).await;
    }
}

/// Removes the conversation file the records no longer point at, once they
/// point at the copy. Best effort: a file left behind is only disk space.
async fn remove_old_conversation(from: &Path, to: &Path) {
    if from != to {
        let from = from.to_owned();
        let _ = blocking(move || fs::remove_file(from)).await;
    }
}

/// Removes the new folder and its partial copy, whichever a failed copy
/// left. Best effort: the error that stopped the copy is the one worth
/// reporting.
async fn discard(folder: &Path) {
    let (folder, built) = (folder.to_owned(), partial(folder));
    let _ = blocking(move || {
        let _ = fs::remove_dir_all(&built);
        let _ = fs::remove_dir_all(&folder);
    })
    .await;
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;
    use crate::git::git;
    use crate::work::tests::origin_and_clone;

    fn branch_of(dir: &Path) -> String {
        git(Some(dir), &["symbolic-ref", "--short", "HEAD"])
            .unwrap()
            .trim()
            .to_owned()
    }

    fn repo(path: PathBuf) -> Vec<(PathBuf, RepoName)> {
        vec![(path, ("acme".to_owned(), "api".to_owned()))]
    }

    #[test]
    fn the_copy_keeps_the_working_change_and_starts_a_new_branch() {
        let t = tempfile::tempdir().unwrap();
        let (_origin, clone) = origin_and_clone(t.path(), "me/x");
        fs::write(clone.join("work.txt"), "unstaged\n").unwrap();
        let folder = t.path().join("sessions/fork");

        copy_clones(&folder, &repo(clone.clone()), "me/y").unwrap();

        let own = partial(&folder).join("api");
        assert_eq!(fs::read_to_string(own.join("work.txt")).unwrap(), "unstaged\n");
        assert_eq!(branch_of(&own), "me/y");
        // The parent keeps its branch and its change, and the copy is its own.
        assert_eq!(branch_of(&clone), "me/x");
        assert_eq!(fs::read_to_string(clone.join("work.txt")).unwrap(), "unstaged\n");
        fs::write(own.join("mine.txt"), "fork only\n").unwrap();
        assert!(!clone.join("mine.txt").exists());
    }

    #[test]
    fn a_leftover_partial_copy_is_cleared_before_copying() {
        let t = tempfile::tempdir().unwrap();
        let (_origin, clone) = origin_and_clone(t.path(), "me/x");
        let folder = t.path().join("sessions/fork");
        let stale = partial(&folder);
        fs::create_dir_all(&stale).unwrap();
        fs::write(stale.join("junk"), "old\n").unwrap();

        copy_clones(&folder, &repo(clone), "me/y").unwrap();

        assert!(!stale.join("junk").exists());
        assert_eq!(branch_of(&stale.join("api")), "me/y");
    }

    #[test]
    fn a_failed_copy_leaves_no_folder() {
        let t = tempfile::tempdir().unwrap();
        let folder = t.path().join("sessions/fork");
        let missing = repo(t.path().join("missing"));
        assert!(matches!(
            copy_clones(&folder, &missing, "me/y"),
            Err(OwnCopyError::Git { .. })
        ));
        assert!(!folder.exists());
        assert!(!partial(&folder).exists());
    }

    #[test]
    fn the_conversation_is_copied_and_the_old_file_is_kept_until_the_records_say_so() {
        let t = tempfile::tempdir().unwrap();
        let from = t.path().join("old/abc.jsonl");
        let to = t.path().join("new/abc.jsonl");
        fs::create_dir_all(from.parent().unwrap()).unwrap();
        fs::write(&from, "turns").unwrap();

        copy_conversation(&from, &to).unwrap();

        assert_eq!(fs::read_to_string(&to).unwrap(), "turns");
        assert_eq!(fs::read_to_string(&from).unwrap(), "turns");
    }

    #[test]
    fn a_conversation_keyed_the_same_way_is_left_as_it_is() {
        let t = tempfile::tempdir().unwrap();
        let file = t.path().join("old/abc.jsonl");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "turns").unwrap();

        copy_conversation(&file, &file).unwrap();

        assert_eq!(fs::read_to_string(&file).unwrap(), "turns");
    }

    #[test]
    fn a_missing_conversation_is_reported_as_missing() {
        let t = tempfile::tempdir().unwrap();
        let from = t.path().join("old/abc.jsonl");
        assert!(matches!(
            copy_conversation(&from, &t.path().join("new/abc.jsonl")),
            Err(OwnCopyError::NoConversation { .. })
        ));
        assert!(matches!(
            copy_conversation(&from, &from),
            Err(OwnCopyError::NoConversation { .. })
        ));
    }
}

//! Forking a session on this machine, from now (FK-1..FK-5): a new session
//! in the same channel with the same agent, clones that are exact copies of
//! the parent's (changes, untracked and ignored files included) on a new
//! branch, and, when asked, a copy of the parent's conversation.

use std::fs;
use std::io::{self, Read};
use std::path::Path;

use humata2_mirror::MirrorError;
use humata2_revive::Env;
use humata2_store::fault::point;
use humata2_store::{ChannelRecord, ForkOrigin, SessionDetail, SessionRecord, SessionRepo, StartStep};

use crate::error::{GitError, StartError};
use crate::git::{git, in_partial, io as git_io};
use crate::names::Branch;
use crate::setup::unix_ms;
use crate::start::{RepoName, Started, Starter, blocking, store_err};

/// What a fork carries over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForkCopy {
    /// The code, and the parent's conversation: the agent resumes it.
    CodeAndContext,
    /// The code only; the agent starts fresh.
    CodeOnly,
    /// The conversation only: the agent reads the parent's clones and
    /// never writes them, until it gets its own copy (FK-7, FK-9, D181).
    ContextOnly,
}

/// A fork to make.
#[derive(Clone, Debug)]
pub struct ForkRequest {
    /// The fork's id when the host chose it; a fresh one when None.
    pub id: Option<String>,
    pub parent: String,
    pub title: String,
    pub copy: ForkCopy,
    /// The API's ForkKind, kept with the fork's origin.
    pub kind: i32,
    /// 0: the conversation as it is now; n: as it was before turn n
    /// (FK-3, D181). The code is always copied as it is now.
    pub before_turn: u32,
    /// A branch typed in the dialog (FC-15), used as typed; None: picked from the title.
    pub branch: Option<String>,
    pub operation: Option<String>,
    pub cols: u16,
    pub rows: u16,
}

/// What a fork would do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForkPlan {
    pub branch: Branch,
    pub repos: Vec<RepoName>,
    /// What the clones' copies take on disk.
    pub copy_bytes: u64,
}

impl Starter {
    /// The branch a fork titled `title` would get and the bytes its clones
    /// would copy. Blocking: it runs git.
    pub fn preview_fork(
        &self,
        parent: &str,
        title: &str,
        typed: Option<&str>,
    ) -> Result<ForkPlan, StartError> {
        let (record, detail) = self.parent(parent)?;
        let channel = self.channel_record(&record.channel)?;
        let repos: Vec<RepoName> = detail
            .repos
            .iter()
            .map(|r| (r.owner.clone(), r.name.clone()))
            .collect();
        let branch = self.pick_fork(&channel, &repos, title, typed)?;
        let mut copy_bytes = 0u64;
        for repo in &detail.repos {
            copy_bytes += tree_bytes(&repo.path).map_err(|source| StartError::Copy {
                path: repo.path.clone(),
                source,
            })?;
        }
        Ok(ForkPlan {
            branch,
            repos,
            copy_bytes,
        })
    }

    /// What each of `parent`'s clones would copy, in bytes, for the card a
    /// fork waits on (IN-10). Blocking: it walks the clones.
    pub fn fork_sizes(&self, parent: &str) -> Result<Vec<(RepoName, u64)>, StartError> {
        let (_, detail) = self.parent(parent)?;
        detail
            .repos
            .iter()
            .map(|repo| {
                let bytes = tree_bytes(&repo.path).map_err(|source| StartError::Copy {
                    path: repo.path.clone(),
                    source,
                })?;
                Ok(((repo.owner.clone(), repo.name.clone()), bytes))
            })
            .collect()
    }

    /// Whether a fork of `parent` titled `title` may take the branch `typed`: the
    /// refusal a bad or taken name gets, before anything is copied.
    pub fn check_fork_branch(&self, parent: &str, title: &str, typed: &str) -> Result<(), StartError> {
        let (record, detail) = self.parent(parent)?;
        let channel = self.channel_record(&record.channel)?;
        let repos: Vec<RepoName> = detail.repos.iter().map(|r| (r.owner.clone(), r.name.clone())).collect();
        self.pick_fork(&channel, &repos, title, Some(typed)).map(|_| ())
    }

    pub(crate) fn pick_fork(
        &self,
        channel: &ChannelRecord,
        repos: &[RepoName],
        title: &str,
        typed: Option<&str>,
    ) -> Result<Branch, StartError> {
        match typed.filter(|branch| !branch.is_empty()) {
            Some(typed) => self.pick_typed(channel, repos, title, typed),
            None => self.pick(channel, repos, title),
        }
    }

    pub(crate) fn parent(&self, id: &str) -> Result<(SessionRecord, SessionDetail), StartError> {
        let record = self
            .store
            .session(id)
            .map_err(store_err("read the session"))?
            .ok_or_else(|| StartError::NoSession { id: id.to_owned() })?;
        let detail = self
            .store
            .session_detail(id)
            .map_err(store_err("read the session"))?
            .ok_or_else(|| StartError::NoSession { id: id.to_owned() })?;
        Ok((record, detail))
    }

    /// Forks a session: records the fork, copies the parent's clones onto a
    /// new branch, copies the conversation (CodeAndContext), launches the
    /// agent. `progress` hears `write_record`, `copy_code:<repo>` for each
    /// repository, `copy_conversation`, `launch` and `verify`. A failure
    /// before the launch leaves nothing behind; from the launch on, the
    /// records stay for revive and [`crate::recover`].
    pub async fn fork(
        &self,
        request: ForkRequest,
        env: Env<'_>,
        progress: &mut (dyn FnMut(&str) + Send),
    ) -> Result<Started, StartError> {
        let this = self.clone();
        let parent_id = request.parent.clone();
        let with_context = request.copy != ForkCopy::CodeOnly;
        let context_only = request.copy == ForkCopy::ContextOnly;
        let (parent, parent_detail, parent_agent) = blocking(move || {
            let (record, detail) = this.parent(&parent_id)?;
            let agent = this
                .store
                .agent(&parent_id)
                .map_err(store_err("read the session's agent"))?;
            Ok::<_, StartError>((record, detail, agent))
        })
        .await??;
        let source_conversation = if with_context {
            let agent = parent_agent.ok_or_else(|| StartError::NoAgent {
                session: parent.id.clone(),
            })?;
            Some(agent.launch.conversation_file)
        } else {
            None
        };
        let this = self.clone();
        let (channel_id, title, typed) = (parent.channel.clone(), request.title.clone(), request.branch.clone());
        let repos: Vec<RepoName> = parent_detail
            .repos
            .iter()
            .map(|r| (r.owner.clone(), r.name.clone()))
            .collect();
        let (channel, branch) = blocking(move || {
            let channel = this.channel_record(&channel_id)?;
            // A context-only fork works in its parent's clones, so it takes
            // no branch of its own.
            let branch = if context_only {
                None
            } else {
                Some(this.pick_fork(&channel, &repos, &title, typed.as_deref())?)
            };
            Ok::<_, StartError>((channel, branch))
        })
        .await??;

        let kind = self.agents.get(&parent.agent)?;
        // A context-only fork launches where its parent's clones are (D187).
        let (folder, repos) = match &branch {
            None => (parent.folder.clone(), parent_detail.repos.clone()),
            Some(branch) => {
                let folder = self.layout.session(&channel.folder, &branch.folder);
                let repos = parent_detail
                    .repos
                    .iter()
                    .map(|r| SessionRepo {
                        owner: r.owner.clone(),
                        name: r.name.clone(),
                        branch: branch.name.clone(),
                        path: folder.join(&r.name),
                    })
                    .collect();
                (folder, repos)
            }
        };
        let conversation = uuid::Uuid::new_v4().hyphenated().to_string();
        let new_conversation = kind
            .launch(&self.home, &folder, &conversation)
            .conversation_file;
        let session = SessionRecord {
            id: request
                .id
                .clone()
                .unwrap_or_else(|| format!("se_{}", uuid::Uuid::new_v4().simple())),
            channel: channel.id.clone(),
            machine: parent.machine.clone(),
            title: request.title.clone(),
            agent: parent.agent.clone(),
            folder: folder.clone(),
            created_unix_ms: unix_ms(),
        };
        let mut detail = SessionDetail {
            session: session.id.clone(),
            repos,
            model: parent_detail.model.clone(),
            first_message: None,
            start: StartStep::Recorded,
            operation: request.operation.clone(),
            revision: 1,
        };
        let origin = ForkOrigin {
            session_id: session.id.clone(),
            parent: parent.id.clone(),
            kind: request.kind,
            before_turn: request.before_turn,
            created_unix_ms: session.created_unix_ms,
        };

        let early = self
            .record_and_copy(
                &session,
                &mut detail,
                &parent_detail,
                &origin,
                source_conversation.as_deref().map(|s| (s, &*new_conversation)),
                branch.is_some(),
                progress,
            )
            .await;
        if let Err(error) = early {
            // Best effort: the fork's own error is the one worth reporting.
            let _ = self.take_back(&session, context_only).await;
            let _ = fs::remove_file(&new_conversation);
            return Err(error);
        }
        let (detail, agent) = self
            .launch_with(
                &session,
                detail,
                request.cols,
                request.rows,
                env,
                progress,
                conversation,
                with_context,
            )
            .await?;
        Ok(Started {
            session,
            detail,
            agent,
        })
    }

    /// Takes back what `record_and_copy` saved. A context-only fork's folder
    /// is its parent's, so only its records go.
    async fn take_back(
        &self,
        session: &SessionRecord,
        context_only: bool,
    ) -> Result<(), StartError> {
        if !context_only {
            return self.undo(session).await;
        }
        let (store, id) = (self.store.clone(), session.id.clone());
        blocking(move || store.remove_session(&id).map_err(store_err("remove the session")))
            .await?
    }

    /// Saves the fork's records and copies what it carries: the clones when
    /// `clones` (a context-only fork has none of its own), then the
    /// conversation when there is one.
    #[allow(clippy::too_many_arguments)]
    async fn record_and_copy(
        &self,
        session: &SessionRecord,
        detail: &mut SessionDetail,
        parent_detail: &SessionDetail,
        origin: &ForkOrigin,
        conversation: Option<(&Path, &Path)>,
        clones: bool,
        progress: &mut (dyn FnMut(&str) + Send),
    ) -> Result<(), StartError> {
        let (store, saved, saved_detail, saved_origin) = (
            self.store.clone(),
            session.clone(),
            detail.clone(),
            origin.clone(),
        );
        blocking(move || -> Result<(), StartError> {
            store
                .put_session(&saved)
                .map_err(store_err("save the session"))?;
            store
                .put_session_detail(&saved_detail)
                .map_err(store_err("save the session's start"))?;
            store
                .put_fork_origin(&saved_origin)
                .map_err(store_err("save where the fork came from"))
        })
        .await??;
        point("sessions.fork.recorded");
        progress("write_record");
        if clones {
            for (repo, from) in detail.repos.iter().zip(&parent_detail.repos) {
                let (src, dest, branch) =
                    (from.path.clone(), repo.path.clone(), repo.branch.clone());
                blocking(move || copy_clone(&src, &dest, &branch))
                    .await?
                    .map_err(|source| StartError::Git {
                        owner: repo.owner.clone(),
                        name: repo.name.clone(),
                        source,
                    })?;
                progress(&format!("copy_code:{}", repo.name));
            }
        }
        point("sessions.fork.copied");
        self.set_step(detail, StartStep::Cloned).await?;
        if let Some((from, to)) = conversation {
            let (from, to, before_turn) = (from.to_owned(), to.to_owned(), origin.before_turn);
            blocking(move || copy_conversation(&from, &to, before_turn)).await??;
            progress("copy_conversation");
        }
        Ok(())
    }
}

/// Copies the conversation at `from` to `to`: all of it when `before_turn`
/// is 0, else only the bytes before turn `before_turn`'s line (FK-3, D181).
pub(crate) fn copy_conversation(from: &Path, to: &Path, before_turn: u32) -> Result<(), StartError> {
    let limit = match before_turn {
        0 => u64::MAX,
        turn => turn_offset(from, turn)?,
    };
    if let Some(dir) = to.parent() {
        fs::create_dir_all(dir).map_err(|source| StartError::Copy {
            path: dir.to_owned(),
            source,
        })?;
    }
    let source = fs::File::open(from).map_err(|source| StartError::NoConversation {
        path: from.to_owned(),
        source,
    })?;
    let mut copy = fs::File::create(to).map_err(|source| StartError::Copy {
        path: to.to_owned(),
        source,
    })?;
    io::copy(&mut source.take(limit), &mut copy)
        .map(drop)
        .map_err(|source| StartError::Copy {
            path: to.to_owned(),
            source,
        })
}

/// Where turn `turn` of the conversation at `from` starts.
fn turn_offset(from: &Path, turn: u32) -> Result<u64, StartError> {
    let turns = humata2_mirror::turns_of(from).map_err(|error| StartError::NoConversation {
        path: from.to_owned(),
        source: match error {
            MirrorError::Io { source, .. } => source,
            other => io::Error::other(other),
        },
    })?;
    turns
        .iter()
        .find(|mark| mark.index == turn)
        .map(|mark| mark.offset)
        .ok_or_else(|| StartError::NoConversation {
            path: from.to_owned(),
            source: io::Error::new(io::ErrorKind::NotFound, format!("it has no turn {turn}")),
        })
}

/// Copies the clone at `src` exactly to `dest` through `<dest>.partial`,
/// then starts `branch` there; the working changes stay.
pub(crate) fn copy_clone(src: &Path, dest: &Path, branch: &str) -> Result<(), GitError> {
    in_partial(dest, |tmp| {
        copy_tree(src, tmp).map_err(git_io(src))?;
        git(Some(tmp), &["checkout", "-b", branch])?;
        Ok(())
    })
}

/// Copies a folder with its files' modes and symlinks as they are.
fn copy_tree(src: &Path, dest: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(src)?;
    if meta.is_dir() {
        match fs::create_dir(dest) {
            Err(e) if e.kind() != io::ErrorKind::AlreadyExists => return Err(e),
            _ => {}
        }
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            copy_tree(&entry.path(), &dest.join(entry.file_name()))?;
        }
        fs::set_permissions(dest, meta.permissions())
    } else if meta.file_type().is_symlink() {
        std::os::unix::fs::symlink(fs::read_link(src)?, dest)
    } else if meta.is_file() {
        fs::copy(src, dest).map(drop)
    } else {
        Ok(())
    }
}

/// The bytes of the files under `path`.
fn tree_bytes(path: &Path) -> io::Result<u64> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() {
        return Ok(if meta.is_file() { meta.len() } else { 0 });
    }
    let mut total = 0;
    for entry in fs::read_dir(path)? {
        total += tree_bytes(&entry?.path())?;
    }
    Ok(total)
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::work::tests::{origin_and_clone, run};

    #[test]
    fn copy_keeps_every_kind_of_change_and_ends_on_the_new_branch() {
        let t = tempfile::tempdir().unwrap();
        let (_origin, clone) = origin_and_clone(t.path(), "me/x");
        fs::write(clone.join(".gitignore"), "ignored\n").unwrap();
        run(&clone, &["add", ".gitignore"]);
        run(&clone, &["commit", "-m", "ignore"]);
        fs::write(clone.join("f"), "unstaged\n").unwrap();
        fs::write(clone.join("staged"), "s\n").unwrap();
        run(&clone, &["add", "staged"]);
        fs::write(clone.join("untracked"), "u\n").unwrap();
        fs::write(clone.join("ignored"), "i\n").unwrap();
        std::os::unix::fs::symlink("f", clone.join("link")).unwrap();
        let dest = t.path().join("sessions/fork/api");
        copy_clone(&clone, &dest, "me/y").unwrap();
        for (name, text) in [
            ("f", "unstaged\n"),
            ("staged", "s\n"),
            ("untracked", "u\n"),
            ("ignored", "i\n"),
        ] {
            assert_eq!(fs::read_to_string(dest.join(name)).unwrap(), text);
        }
        assert_eq!(fs::read_link(dest.join("link")).unwrap(), Path::new("f"));
        let head = git(Some(&dest), &["symbolic-ref", "--short", "HEAD"]).unwrap();
        assert_eq!(head.trim(), "me/y");
        let status = git(Some(&dest), &["status", "--porcelain"]).unwrap();
        assert!(status.contains("staged") && status.contains("untracked"));
        assert!(!crate::layout::partial(&dest).exists());
        // The parent is untouched.
        let parent_head = git(Some(&clone), &["symbolic-ref", "--short", "HEAD"]).unwrap();
        assert_eq!(parent_head.trim(), "me/x");
    }

    #[test]
    fn copy_keeps_file_modes() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let (_origin, clone) = origin_and_clone(t.path(), "me/x");
        fs::write(clone.join("run.sh"), "#!/bin/sh\n").unwrap();
        fs::set_permissions(clone.join("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
        let dest = t.path().join("copy");
        copy_clone(&clone, &dest, "me/y").unwrap();
        let mode = fs::metadata(dest.join("run.sh")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }

    #[test]
    fn failed_copy_leaves_nothing() {
        let t = tempfile::tempdir().unwrap();
        let dest = t.path().join("copy");
        assert!(copy_clone(&t.path().join("missing"), &dest, "me/y").is_err());
        assert!(!dest.exists());
        assert!(!crate::layout::partial(&dest).exists());
    }

    #[test]
    fn tree_bytes_sums_files() {
        let t = tempfile::tempdir().unwrap();
        fs::create_dir(t.path().join("d")).unwrap();
        fs::write(t.path().join("a"), "123").unwrap();
        fs::write(t.path().join("d/b"), "45").unwrap();
        assert_eq!(tree_bytes(t.path()).unwrap(), 5);
    }

    #[test]
    fn conversation_copy_reports_a_missing_file() {
        let t = tempfile::tempdir().unwrap();
        let from = t.path().join("c.jsonl");
        let to = t.path().join("deep/n.jsonl");
        assert!(matches!(
            copy_conversation(&from, &to, 0),
            Err(StartError::NoConversation { .. })
        ));
        fs::write(&from, "turns").unwrap();
        copy_conversation(&from, &to, 0).unwrap();
        assert_eq!(fs::read_to_string(&to).unwrap(), "turns");
    }

    #[test]
    fn conversation_copy_before_a_turn_keeps_the_lines_before_its_prompt() {
        let t = tempfile::tempdir().unwrap();
        let from = t.path().join("c.jsonl");
        let prompt_one = r#"{"type":"user","message":{"role":"user","content":"one"}}"#;
        let answer = r#"{"type":"assistant","message":{"role":"assistant","content":[]}}"#;
        let prompt_two = r#"{"type":"user","message":{"role":"user","content":"two"}}"#;
        fs::write(&from, format!("{prompt_one}\n{answer}\n{prompt_two}\n{answer}\n")).unwrap();

        let to = t.path().join("n.jsonl");
        copy_conversation(&from, &to, 2).unwrap();
        assert_eq!(
            fs::read_to_string(&to).unwrap(),
            format!("{prompt_one}\n{answer}\n")
        );
        // The conversation has two turns, so there is no turn 3 to cut at.
        assert!(matches!(
            copy_conversation(&from, &t.path().join("m.jsonl"), 3),
            Err(StartError::NoConversation { .. })
        ));
    }
}

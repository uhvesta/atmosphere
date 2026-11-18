//! Forking a session on this machine, from now (FK-1..FK-6): a new session
//! in the same channel with the parent's agent or another one, clones that are exact copies of
//! the parent's (changes, untracked and ignored files included) on a new
//! branch, and, when asked, a copy of the parent's conversation.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use humata2_mirror::MirrorError;
use humata2_revive::Env;
use humata2_store::fault::point;
use humata2_store::{ChannelRecord, ForkOrigin, SessionDetail, SessionRecord, SessionRepo, StartStep};

use crate::error::{GitError, StartError};
use crate::git::{git, in_partial, io as git_io};
use crate::names::Branch;
use crate::setup::unix_ms;
use crate::start::{RepoName, Started, Starter, blocking, store_err};
use crate::tree_copy::{Dirs, copy_tree};

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
    /// The agent the fork runs; None: its parent's. Another agent, with
    /// context, is handed the conversation as a file (FK-6, D212).
    pub agent: Option<String>,
    /// The fork's conversation id when the host chose it, so it can keep the
    /// fork's first message for it before the launch; a fresh one when None.
    pub conversation: Option<String>,
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

/// Where, in a fork's folder, a fork whose agent can't resume its parent's
/// conversation finds it (FK-6, D212): outside the clones, named for the
/// fork so context-only forks sharing their parent's folder don't collide.
pub fn handover_file(fork: &str) -> PathBuf {
    Path::new(".humata").join(format!("handover-{fork}.jsonl"))
}

impl Starter {
    /// Whether a fork of a session running `parent_agent`, carrying `copy`
    /// and running `agent`, is handed the conversation as a file and a
    /// summary instead of resuming it (FK-6, D212): it carries context, and
    /// it runs another agent or its parent's agent keeps no conversation file.
    pub fn hands_over(&self, parent_agent: &str, agent: &str, copy: ForkCopy) -> Result<bool, StartError> {
        if copy == ForkCopy::CodeOnly {
            return Ok(false);
        }
        Ok(agent != parent_agent || self.agents.get(parent_agent)?.conversation_file.is_empty())
    }

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
    /// agent. A fork the parent's conversation can't be resumed in (another
    /// agent, or one that keeps no file) gets it at [`handover_file`] and
    /// starts a fresh conversation; the host types its first message. `progress` hears `write_record`, `copy_code:<repo>` for each
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
        let agent_name = request
            .agent
            .clone()
            .filter(|agent| !agent.is_empty())
            .unwrap_or_else(|| parent.agent.clone());
        let handover = self.hands_over(&parent.agent, &agent_name, request.copy)?;
        let parent_conversation = parent_agent.as_ref().map(|agent| agent.launch.conversation_id.clone());
        let source_conversation = if handover {
            // A parent with no conversation file hands over none: its first
            // message falls back to its terminal output.
            parent_agent
                .map(|agent| agent.launch.conversation_file)
                .filter(|file| !file.as_os_str().is_empty())
        } else if with_context {
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

        let kind = self.agents.get(&agent_name)?;
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
        let conversation = request
            .conversation
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().hyphenated().to_string());
        let id = request
            .id
            .clone()
            .unwrap_or_else(|| format!("se_{}", uuid::Uuid::new_v4().simple()));
        // A handed-over conversation goes beside the clones; one the agent
        // resumes, where the agent keeps it.
        let new_conversation = if handover {
            folder.join(handover_file(&id))
        } else {
            kind.launch(&self.home, &folder, &conversation).conversation_file
        };
        // A handover before a turn hands over the parent's bytes before that
        // turn's start, found as the parent's own format says (a Codex or
        // Copilot rollout through its adapter).
        let handed_start = match (&source_conversation, &parent_conversation) {
            (Some(file), Some(from_id)) if handover && request.before_turn != 0 => {
                let kind = self.agents.get(&parent.agent).ok().cloned();
                let (home, folder) = (self.home.clone(), parent.folder.clone());
                let (file, from_id) = (file.clone(), from_id.clone());
                let turn = request.before_turn;
                blocking(move || kind?.turn_start(&home, &folder, &from_id, &file, turn)).await?
            }
            _ => None,
        };
        // A conversation in a format an adapter knows is forked through it,
        // so the copy is named for its new id and folder (MV-16).
        let through = match parent_conversation {
            Some(from_id) if !handover && with_context => {
                let parent_kind = self.agents.get(&parent.agent)?;
                match (parent_kind.placement(&self.home, &parent.folder), kind.placement(&self.home, &folder)) {
                    (Some(from), Some(to)) => Some(Through {
                        format: kind.conversation_format,
                        from,
                        from_id,
                        to,
                        to_id: conversation.clone(),
                    }),
                    _ => None,
                }
            }
            _ => None,
        };
        let session = SessionRecord {
            id,
            channel: channel.id.clone(),
            machine: parent.machine.clone(),
            title: request.title.clone(),
            agent: agent_name.clone(),
            folder: folder.clone(),
            created_unix_ms: unix_ms(),
        };
        let mut detail = SessionDetail {
            session: session.id.clone(),
            repos,
            // Another agent starts on its own default model.
            model: (agent_name == parent.agent)
                .then(|| parent_detail.model.clone())
                .flatten(),
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
                through,
                handed_start,
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
                with_context && !handover,
            )
            .await?;
        // The agent record names the copy now.
        let (folder, id) = (session.folder.clone(), session.id.clone());
        let _ = blocking(move || forget_note(&copy_note(&folder, &id))).await;
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
    /// conversation when there is one: a handover before a turn up to
    /// `handed_start`, that turn's start, when its parent's format found it.
    #[allow(clippy::too_many_arguments)]
    async fn record_and_copy(
        &self,
        session: &SessionRecord,
        detail: &mut SessionDetail,
        parent_detail: &SessionDetail,
        origin: &ForkOrigin,
        conversation: Option<(&Path, &Path)>,
        through: Option<Through>,
        handed_start: Option<u64>,
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
            let (folder, id) = (session.folder.clone(), session.id.clone());
            blocking(move || match handed_start {
                Some(start) => hand_over_conversation(&folder, &id, &from, &to, start),
                None => carry_conversation(&folder, &id, &from, &to, through, before_turn),
            })
            .await??;
            progress("copy_conversation");
        }
        Ok(())
    }
}

/// Where, in a session's folder, the path of the conversation copy its fork
/// made is kept until the agent is launched, so [`crate::start::Starter::undo`]
/// can remove a copy outside the folder (an agent keeps its conversations
/// elsewhere) however the fork was cut short. A context-only fork shares
/// its parent's folder, so the name carries the fork's id.
pub(crate) fn copy_note(folder: &Path, fork: &str) -> PathBuf {
    folder.join(".humata").join(format!("fork-copy-{fork}"))
}

/// Saves, durably, in the file `note`, that the conversation copy at `to` is
/// about to be made, so a cut at any point after leaves it findable by
/// [`remove_noted_copy`]. The note is renamed into place whole, so it never
/// names half a path.
pub(crate) fn note_copy_at(note: &Path, to: &Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let dir = note.parent().unwrap_or(Path::new("."));
    create_dir_all_durably(dir)?;
    let temp = temp_note(note);
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    let mut file = humata2_durable::File::open(&options, &temp)?;
    file.append(to.as_os_str().as_bytes())?;
    file.sync_all()?;
    drop(file);
    humata2_durable::rename(&temp, note)?;
    humata2_durable::sync_dir(dir)
}

/// Makes `dir` and every missing folder above it, and puts each new
/// folder's name in its parent on disk, so a power cut afterwards keeps
/// them and what is synced inside them.
fn create_dir_all_durably(dir: &Path) -> io::Result<()> {
    let missing: Vec<PathBuf> = dir
        .ancestors()
        .take_while(|up| !up.as_os_str().is_empty() && fs::symlink_metadata(up).is_err())
        .map(Path::to_owned)
        .collect();
    humata2_durable::create_dir_all(dir, 0o755)?;
    for made in &missing {
        if let Some(parent) = made.parent().filter(|up| !up.as_os_str().is_empty()) {
            humata2_durable::sync_dir(parent)?;
        }
    }
    Ok(())
}

fn temp_note(note: &Path) -> PathBuf {
    let mut name = note.as_os_str().to_os_string();
    name.push(".tmp");
    PathBuf::from(name)
}

/// Removes the conversation copy named in the file `note`, then its folder
/// when that left it empty (an agent that keys conversations by folder made
/// one for the copy), then the note. No note: nothing to do. Safe to repeat.
pub(crate) fn remove_noted_copy(note: &Path) -> Result<(), StartError> {
    use std::os::unix::ffi::OsStrExt;
    let bytes = match fs::read(note) {
        Ok(bytes) => bytes,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return forget_note(note),
        Err(source) => {
            return Err(StartError::Remove {
                path: note.to_owned(),
                source,
            });
        }
    };
    let copy = PathBuf::from(std::ffi::OsStr::from_bytes(&bytes));
    match fs::remove_file(&copy) {
        Err(source) if source.kind() != io::ErrorKind::NotFound => {
            return Err(StartError::Remove { path: copy, source });
        }
        _ => {}
    }
    if let Some(dir) = copy.parent() {
        // Only an empty folder goes; one with other conversations stays.
        let _ = fs::remove_dir(dir);
    }
    forget_note(note)
}

/// Removes the file `note` and any half-written one.
pub(crate) fn forget_note(note: &Path) -> Result<(), StartError> {
    for path in [temp_note(note), note.to_owned()] {
        match fs::remove_file(&path) {
            Err(source) if source.kind() != io::ErrorKind::NotFound => {
                return Err(StartError::Remove { path, source });
            }
            _ => {}
        }
    }
    Ok(())
}

/// Takes back the conversation copy of the fork `fork` in `folder`.
pub(crate) fn remove_fork_copy(folder: &Path, fork: &str) -> Result<(), StartError> {
    remove_noted_copy(&copy_note(folder, fork))
}

/// Copies the conversation at `from` to `to`, up to turn `before_turn` when
/// it is above 0: through its format's adapter when `through` finds it there
/// (MV-16), else byte for byte. The copy is noted in `folder` for the session
/// `id` before it exists, so a cut can't leave it unfound. A fork and a
/// session continued in another channel (CV-8) both copy through here.
pub(crate) fn carry_conversation(
    folder: &Path,
    id: &str,
    from: &Path,
    to: &Path,
    through: Option<Through>,
    before_turn: u32,
) -> Result<(), StartError> {
    note_copy_at(&copy_note(folder, id), to).map_err(|source| StartError::Copy {
        path: copy_note(folder, id),
        source,
    })?;
    match through.filter(|through| through.keeps(from)) {
        Some(through) => fork_conversation(
            through.format,
            &through.from,
            &through.from_id,
            &through.to,
            &through.to_id,
            before_turn,
        )
        .map(drop),
        None => copy_conversation(from, to, before_turn),
    }
}

/// Copies the first `start` bytes of the conversation at `from` to `to`,
/// the handover of a fork before the turn that starts there, noted in
/// `folder` for the session `id` before it exists as
/// [`carry_conversation`] notes its copy.
fn hand_over_conversation(
    folder: &Path,
    id: &str,
    from: &Path,
    to: &Path,
    start: u64,
) -> Result<(), StartError> {
    note_copy_at(&copy_note(folder, id), to).map_err(|source| StartError::Copy {
        path: copy_note(folder, id),
        source,
    })?;
    copy_bytes(from, to, start)
}

/// Copies the conversation at `from` to `to`: all of it when `before_turn`
/// is 0, else only the bytes before turn `before_turn`'s line (FK-3, D181).
pub(crate) fn copy_conversation(from: &Path, to: &Path, before_turn: u32) -> Result<(), StartError> {
    let limit = match before_turn {
        0 => u64::MAX,
        turn => turn_offset(from, turn)?,
    };
    copy_bytes(from, to, limit)
}

/// Copies the first `limit` bytes of the file at `from` to a new file at
/// `to`, durably.
pub(crate) fn copy_bytes(from: &Path, to: &Path, limit: u64) -> Result<(), StartError> {
    if let Some(dir) = to.parent() {
        humata2_durable::create_dir_all(dir, 0o755).map_err(|source| StartError::Copy {
            path: dir.to_owned(),
            source,
        })?;
    }
    let source = fs::File::open(from).map_err(|source| StartError::NoConversation {
        path: from.to_owned(),
        source,
    })?;
    copy_durably(source.take(limit), to).map_err(|source| StartError::Copy {
        path: to.to_owned(),
        source,
    })
}

/// Writes everything `from` yields to a new file at `to`, and puts its bytes
/// and its name in its folder on disk before returning, so a power cut
/// afterwards leaves the whole copy.
pub(crate) fn copy_durably(mut from: impl Read, to: &Path) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true).mode(0o600);
    let file = humata2_durable::File::open(&options, to)?;
    let mut buffer = vec![0u8; 64 * 1024];
    let mut at = 0u64;
    loop {
        let read = match from.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        file.write_all_at(&buffer[..read], at)?;
        at += read as u64;
    }
    file.sync_all()?;
    match to.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => humata2_durable::sync_dir(dir),
        _ => Ok(()),
    }
}

/// Forks conversation `from_id` at `from` into `to_id` at `to` through the
/// adapter for `format` (`humata2_conversation::adapter(..).fork`), keeping
/// the turns before `before_turn` (all of them when 0), so the copy is named
/// for its new id and folder and reads strictly (FK-3, MV-16); returns the
/// new transcript's path. `copy_conversation` stays for kinds with no
/// format.
pub(crate) fn fork_conversation(
    format: crate::agents::ConversationFormat,
    from: &humata2_conversation::Placement,
    from_id: &str,
    to: &humata2_conversation::Placement,
    to_id: &str,
    before_turn: u32,
) -> Result<PathBuf, StartError> {
    let format = format.adapter_format().ok_or(StartError::NoConversationFormat)?;
    let adapter = humata2_conversation::adapter(format);
    let refused = |source| StartError::Conversation { id: from_id.to_owned(), source };
    let parent = adapter.locate(from, from_id).map_err(refused)?;
    let before = (before_turn != 0).then_some(before_turn);
    let forked = adapter.fork(&parent, before, to_id, to).map_err(refused)?;
    forked
        .files
        .into_iter()
        .find(|file| file.role == humata2_conversation::FileRole::Transcript)
        .map(|file| file.path)
        .ok_or_else(|| {
            refused(humata2_conversation::ConversationError::NotFound { path: to.folder.clone() })
        })
}

/// A conversation copied through its format's adapter (MV-16) rather than
/// byte for byte: where the parent's and the fork's are, and their ids.
pub(crate) struct Through {
    pub(crate) format: crate::agents::ConversationFormat,
    pub(crate) from: humata2_conversation::Placement,
    pub(crate) from_id: String,
    pub(crate) to: humata2_conversation::Placement,
    pub(crate) to_id: String,
}

impl Through {
    /// Whether the adapter finds the parent's conversation at `file`, the
    /// path its agent record names. An agent set up to keep its conversations
    /// elsewhere than its CLI does is copied as a file, as before.
    pub(crate) fn keeps(&self, file: &Path) -> bool {
        self.format.adapter_format().is_some_and(|format| {
            humata2_conversation::adapter(format)
                .locate(&self.from, &self.from_id)
                .is_ok_and(|found| found.files.first().is_some_and(|transcript| transcript.path == file))
        })
    }
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
        // `tmp` is renamed to `dest`: a link into the clone points into `dest`.
        // The error names the file that failed, not just the clone.
        copy_tree(src, tmp, dest, Dirs::KeepModes)
            .map_err(|error| git_io(&error.path)(error.source))?;
        git(Some(tmp), &["checkout", "-b", branch])?;
        Ok(())
    })
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

    /// The worker of the power-cut test below: this test binary run again
    /// with this test selected; it does nothing otherwise.
    #[cfg(faults_build)]
    #[test]
    fn power_cut_child() {
        let (Ok(from), Ok(to)) = (
            std::env::var("HUMATA2_COPY_FROM"),
            std::env::var("HUMATA2_COPY_TO"),
        ) else {
            return;
        };
        copy_conversation(Path::new(&from), Path::new(&to), 0).unwrap();
        humata2_durable::cut("after the conversation copy");
    }

    #[cfg(faults_build)]
    #[test]
    fn a_fork_conversation_copy_is_whole_after_a_power_cut() {
        use std::os::unix::process::ExitStatusExt;
        let dir = tempfile::Builder::new().prefix("h2pc").tempdir_in("/tmp").unwrap();
        let from = dir.path().join("parent.jsonl");
        fs::write(&from, "{\"turn\":1}\n{\"turn\":2}\n").unwrap();
        let home = dir.path().join("home");
        fs::create_dir(&home).unwrap();
        let to = home.join("conversations/fork.jsonl");

        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["fork::tests::power_cut_child", "--exact", "--nocapture", "--test-threads=1"])
            .env("HUMATA2_COPY_FROM", &from)
            .env("HUMATA2_COPY_TO", &to)
            .env("HUMATA2_POWER_CUT_ROOTS", &home)
            .env("HUMATA2_POWER_CUT_SPARE", dir.path().join("spare"))
            .env_remove("TESTBRIDGE_TEST_ONLY")
            .status()
            .unwrap();

        assert_eq!(status.signal(), Some(9), "the worker didn't stop at the cut: {status}");
        assert_eq!(fs::read(&to).unwrap(), fs::read(&from).unwrap());
    }

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
    fn copy_keeps_links_as_links_and_points_those_into_the_folder_into_the_fork() {
        use std::os::unix::fs::symlink;
        let t = tempfile::tempdir().unwrap();
        let (_origin, clone) = origin_and_clone(t.path(), "me/x");
        let elsewhere = t.path().join("elsewhere.md");
        fs::write(&elsewhere, "x\n").unwrap();
        fs::create_dir_all(clone.join("docs")).unwrap();
        fs::write(clone.join("docs/plan.md"), "plan\n").unwrap();
        symlink("docs/plan.md", clone.join("relative")).unwrap();
        symlink(clone.join("docs/plan.md"), clone.join("inside")).unwrap();
        symlink(&elsewhere, clone.join("outside")).unwrap();
        symlink("missing.md", clone.join("dangling")).unwrap();
        let dest = t.path().join("sessions/fork/api");

        copy_clone(&clone, &dest, "me/y").unwrap();

        let link = |name: &str| fs::read_link(dest.join(name)).expect("a link, neither followed nor left out");
        assert_eq!(link("relative"), Path::new("docs/plan.md"));
        assert_eq!(link("inside"), dest.join("docs/plan.md"), "a link into the folder points into the fork");
        assert_eq!(link("outside"), elsewhere, "a link out of the folder points where it did");
        assert_eq!(link("dangling"), Path::new("missing.md"));
        assert_eq!(fs::read_to_string(dest.join("inside")).unwrap(), "plan\n");
        // The parent keeps its own.
        assert_eq!(fs::read_link(clone.join("inside")).unwrap(), clone.join("docs/plan.md"));
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

    /// Two turns of a Claude conversation as `{id}` in `{folder}`.
    const CLAUDE_TWO_TURNS: &str = concat!(
        r#"{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"{folder}","sessionId":"{id}","version":"2.1.325","gitBranch":"humata/fix","type":"user","message":{"role":"user","content":"Fix the login redirect"},"uuid":"7c0e5a10-0001-4a8e-9b1d-3f2a6c8d0001","timestamp":"2026-11-16T09:01:00.000Z"}"#,
        "\n",
        r#"{"parentUuid":"7c0e5a10-0001-4a8e-9b1d-3f2a6c8d0001","isSidechain":false,"userType":"external","cwd":"{folder}","sessionId":"{id}","version":"2.1.325","gitBranch":"humata/fix","message":{"id":"msg_01A","type":"message","role":"assistant","model":"claude-sonnet-4-5","content":[{"type":"text","text":"Fixed {folder}/api/src/login.rs."}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":812,"output_tokens":64}},"requestId":"req_01A","type":"assistant","uuid":"7c0e5a10-0011-4a8e-9b1d-3f2a6c8d0011","timestamp":"2026-11-16T09:02:00.000Z"}"#,
        "\n",
        r#"{"parentUuid":"7c0e5a10-0011-4a8e-9b1d-3f2a6c8d0011","isSidechain":false,"userType":"external","cwd":"{folder}","sessionId":"{id}","version":"2.1.325","gitBranch":"humata/fix","type":"user","message":{"role":"user","content":"Now add a test for it"},"uuid":"7c0e5a10-0002-4a8e-9b1d-3f2a6c8d0002","timestamp":"2026-11-16T09:05:00.000Z"}"#,
        "\n",
        r#"{"parentUuid":"7c0e5a10-0002-4a8e-9b1d-3f2a6c8d0002","isSidechain":false,"userType":"external","cwd":"{folder}","sessionId":"{id}","version":"2.1.325","gitBranch":"humata/fix","message":{"id":"msg_01B","type":"message","role":"assistant","model":"claude-sonnet-4-5","content":[{"type":"text","text":"Added a test."}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":812,"output_tokens":64}},"requestId":"req_01B","type":"assistant","uuid":"7c0e5a10-0012-4a8e-9b1d-3f2a6c8d0012","timestamp":"2026-11-16T09:06:00.000Z"}"#,
        "\n",
    );

    #[test]
    fn a_forked_conversation_holds_the_turns_before_and_is_named_for_its_new_id_and_folder() {
        const ID: &str = "4f6d2c1e-8a3b-4c5d-9e7f-1a2b3c4d5e6f";
        const NEW_ID: &str = "5a6b7c8d-9e0f-4a1b-8c2d-3e4f5a6b7c8d";
        let t = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(t.path()).unwrap();
        let from = humata2_conversation::Placement {
            home: root.join("home"),
            cli_home: None,
            folder: root.join("payments/sessions/fix"),
        };
        let to = humata2_conversation::Placement { folder: root.join("payments/sessions/fix-2"), ..from.clone() };
        let transcript = |at: &humata2_conversation::Placement, id: &str| {
            let project = humata2_conversation::claude_project_name(&at.folder);
            at.home.join(".claude/projects").join(project).join(format!("{id}.jsonl"))
        };
        let parent = transcript(&from, ID);
        let text = CLAUDE_TWO_TURNS.replace("{folder}", &from.folder.to_string_lossy()).replace("{id}", ID);
        fs::create_dir_all(parent.parent().unwrap()).unwrap();
        fs::write(&parent, &text).unwrap();

        let path =
            fork_conversation(crate::agents::ConversationFormat::ClaudeJsonl, &from, ID, &to, NEW_ID, 2).unwrap();

        assert_eq!(path, transcript(&to, NEW_ID));
        let claude = humata2_conversation::adapter(humata2_conversation::Format::ClaudeJsonl);
        let forked = claude.locate(&to, NEW_ID).unwrap();
        assert_eq!(claude.turns(&forked).unwrap().len(), 1);
        let new_folder = serde_json::Value::from(to.folder.to_string_lossy().into_owned());
        for line in fs::read_to_string(&path).unwrap().lines() {
            let record: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(record["sessionId"], serde_json::Value::from(NEW_ID));
            assert_eq!(record["cwd"], new_folder);
        }
        assert_eq!(fs::read_to_string(&parent).unwrap(), text, "the parent is as it was");
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

    #[test]
    fn a_noted_copy_is_removed_with_its_folder_when_that_is_left_empty() {
        let t = tempfile::tempdir().unwrap();
        let (note, keyed) = (t.path().join("s/.humata/note"), t.path().join("conv/key"));
        let (copy, other) = (keyed.join("a.jsonl"), t.path().join("conv/kept/b.jsonl"));
        fs::create_dir_all(&keyed).unwrap();
        fs::create_dir_all(other.parent().unwrap()).unwrap();
        fs::write(&other, "b").unwrap();
        // Noted before the copy exists: removing it then is not an error.
        note_copy_at(&note, &copy).unwrap();
        remove_noted_copy(&note).unwrap();
        note_copy_at(&note, &copy).unwrap();
        fs::create_dir_all(&keyed).unwrap();
        fs::write(&copy, "a").unwrap();

        remove_noted_copy(&note).unwrap();

        assert!(!copy.exists() && !keyed.exists() && !note.exists());
        assert!(other.exists(), "another conversation's file is not touched");
        // Repeating it, with no note left, does nothing.
        remove_noted_copy(&note).unwrap();
    }
}

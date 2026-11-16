//! Starting a session (SE-4, FC-3, FC-4, FC-14, FC-15, PER-11, PER-12):
//! record it, clone its repositories on a new branch, then launch its
//! agent. Each step is saved first, so [`crate::recover`] can finish or
//! undo a start a crash cut short.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use humata2_revive::{Env, ReviveConfig};
use humata2_store::fault::point;
use humata2_store::{
    AgentLaunch, AgentRecord, ChannelRecord, MainClone, SessionDetail, SessionRecord, SessionRepo,
    StartStep, Store, StoreError, TabKind, TabRecord,
};

use crate::agents::Agents;
use crate::error::StartError;
use crate::git::{clone_session, local_branches, remote_branches, remote_url};
use crate::layout::Layout;
use crate::names::{Branch, branch_slug, pick_branch, valid_branch_name};
use crate::prefix::effective_prefix;
use crate::setup::unix_ms;

/// `(owner, name)` of a repository.
pub type RepoName = (String, String);

/// Between typing the first message and pressing return, so an agent
/// that reads a paste whole doesn't take the return as part of it.
const TYPE_PAUSE: Duration = Duration::from_millis(100);

/// What a start needs from this machine.
#[derive(Clone)]
pub struct Starter {
    pub store: Arc<Store>,
    pub layout: Layout,
    pub remote_template: String,
    pub agents: Agents,
    pub home: PathBuf,
    pub revive: ReviveConfig,
    /// Where each terminal's holder keeps its socket, as `<tab>.sock`.
    pub holders: PathBuf,
}

/// A session to start.
#[derive(Clone, Debug)]
pub struct StartRequest {
    /// The session's id when the host chose it (D150); a fresh one when None.
    pub id: Option<String>,
    pub channel: String,
    pub machine: String,
    pub title: String,
    /// The channel's default agent when None.
    pub agent: Option<String>,
    pub model: Option<String>,
    /// A subset of the channel's repositories (FC-4); all those with a
    /// ready main clone when None.
    pub repos: Option<Vec<RepoName>>,
    pub first_message: Option<String>,
    /// A branch typed in the dialog (FC-15), used as typed for every
    /// repository; Humata picks one from the title when None.
    pub branch: Option<String>,
    pub operation: Option<String>,
    pub cols: u16,
    pub rows: u16,
}

/// A session that started.
#[derive(Clone, Debug)]
pub struct Started {
    pub session: SessionRecord,
    pub detail: SessionDetail,
    pub agent: AgentRecord,
}

pub(crate) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, StartError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|source| StartError::Background { source })
}

pub(crate) fn store_err(doing: &'static str) -> impl Fn(StoreError) -> StartError {
    move |source| StartError::Store { doing, source }
}

impl Starter {
    /// The branch each repository of the channel would get for `title`
    /// (FC-15). Blocking: it runs git. A remote that can't be reached
    /// counts as having no branches.
    pub fn preview_branches(
        &self,
        channel: &str,
        title: &str,
    ) -> Result<Vec<(RepoName, Branch)>, StartError> {
        let channel = self.channel_record(channel)?;
        let repos: Vec<RepoName> = channel
            .repos
            .iter()
            .map(|r| (r.owner.clone(), r.name.clone()))
            .collect();
        let branch = self.pick(&channel, &repos, title)?;
        Ok(repos.into_iter().map(|r| (r, branch.clone())).collect())
    }

    pub(crate) fn channel_record(&self, id: &str) -> Result<ChannelRecord, StartError> {
        self.store
            .channel(id)
            .map_err(store_err("read the channel"))?
            .ok_or_else(|| StartError::NoChannel { id: id.to_owned() })
    }

    fn pick_for(
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

    /// One branch for all `repos`: the first free name, given what the
    /// main clones, the remotes and the channel's other sessions use
    /// (FC-14), and no session folder already there.
    pub(crate) fn pick(
        &self,
        channel: &ChannelRecord,
        repos: &[RepoName],
        title: &str,
    ) -> Result<Branch, StartError> {
        self.pick_excluding(channel, repos, title, None)
    }

    /// As [`Starter::pick`], where the branch and folder of the session
    /// `own` don't count as taken (a rename keeps its folder).
    pub(crate) fn pick_excluding(
        &self,
        channel: &ChannelRecord,
        repos: &[RepoName],
        title: &str,
        own: Option<&str>,
    ) -> Result<Branch, StartError> {
        let taken = self.taken_names(channel, repos, own)?;
        let prefix = self.prefix(channel)?;
        let prefix = prefix.as_str();
        Ok(pick_branch(prefix, &branch_slug(title), |name| {
            let folder = name.strip_prefix(prefix).unwrap_or(name);
            taken.contains(name)
                || (own.is_none() && self.layout.session(&channel.folder, folder).exists())
        }))
    }

    pub(crate) fn prefix(&self, channel: &ChannelRecord) -> Result<String, StartError> {
        effective_prefix(&self.store, channel).map_err(store_err("read the branch prefix"))
    }

    /// The names already used for branches in `repos`: the main clones, the
    /// remotes and the channel's sessions, except the session `own`.
    fn taken_names(
        &self,
        channel: &ChannelRecord,
        repos: &[RepoName],
        own: Option<&str>,
    ) -> Result<HashSet<String>, StartError> {
        let mut taken: HashSet<String> = HashSet::new();
        for (owner, name) in repos {
            let main = self.layout.main_clone(&channel.folder, name);
            taken.extend(local_branches(&main).map_err(|source| StartError::Git {
                owner: owner.clone(),
                name: name.clone(),
                source,
            })?);
            if let Ok(url) = remote_url(&self.remote_template, owner, name) {
                if let Ok(remote) = remote_branches(&url) {
                    taken.extend(remote);
                }
            }
        }
        let in_channel: HashSet<String> = self
            .store
            .sessions()
            .map_err(store_err("list the sessions"))?
            .into_iter()
            .filter(|s| s.channel == channel.id)
            .map(|s| s.id)
            .collect();
        for detail in self.store.session_details().map_err(store_err("list the sessions"))? {
            if in_channel.contains(&detail.session) && own != Some(detail.session.as_str()) {
                taken.extend(detail.repos.into_iter().map(|r| r.branch));
            }
        }
        Ok(taken)
    }

    /// A branch typed in the dialog (FC-15): used as typed, with no prefix.
    /// The folder stays the title's slug.
    pub(crate) fn pick_typed(
        &self,
        channel: &ChannelRecord,
        repos: &[RepoName],
        title: &str,
        typed: &str,
    ) -> Result<Branch, StartError> {
        if !valid_branch_name(typed) {
            return Err(StartError::BadBranch { branch: typed.to_owned() });
        }
        let taken = self.taken_names(channel, repos, None)?;
        if taken.contains(typed) {
            let suggestion = pick_branch("", typed, |name| taken.contains(name)).name;
            return Err(StartError::BranchTaken { branch: typed.to_owned(), suggestion });
        }
        let slug = branch_slug(title);
        let folder = pick_branch("", &slug, |name| self.layout.session(&channel.folder, name).exists()).name;
        Ok(Branch { name: typed.to_owned(), folder })
    }

    fn choose(
        &self,
        channel: &ChannelRecord,
        wanted: Option<&[RepoName]>,
    ) -> Result<Vec<RepoName>, StartError> {
        let not_ready = |owner: &str, name: &str, main: &MainClone| StartError::NotReady {
            owner: owner.to_owned(),
            name: name.to_owned(),
            state: match main {
                MainClone::Cloning => "still cloning".to_owned(),
                MainClone::Ready => "ready".to_owned(),
                MainClone::Failed { reason } => format!("the clone failed: {reason}"),
            },
        };
        let chosen: Vec<RepoName> = match wanted {
            Some(list) => {
                for (owner, name) in list {
                    let repo = channel
                        .repos
                        .iter()
                        .find(|r| &r.owner == owner && &r.name == name)
                        .ok_or_else(|| StartError::UnknownRepo {
                            channel: channel.id.clone(),
                            owner: owner.clone(),
                            name: name.clone(),
                        })?;
                    if repo.main != MainClone::Ready {
                        return Err(not_ready(owner, name, &repo.main));
                    }
                }
                list.to_vec()
            }
            None => channel
                .repos
                .iter()
                .filter(|r| r.main == MainClone::Ready)
                .map(|r| (r.owner.clone(), r.name.clone()))
                .collect(),
        };
        if chosen.is_empty() {
            return Err(match channel.repos.first() {
                Some(r) => not_ready(&r.owner, &r.name, &r.main),
                None => StartError::NoRepos {
                    channel: channel.id.clone(),
                },
            });
        }
        Ok(chosen)
    }

    /// Starts a session: records it, clones its repositories, launches its
    /// agent. `progress` hears `write_record`, `clone:<name>` for each
    /// repository, `launch` and `verify`. A failure before the launch
    /// leaves nothing behind; from the launch on, the records stay and
    /// revive and [`crate::recover`] own them.
    pub async fn start(
        &self,
        request: StartRequest,
        env: Env<'_>,
        progress: &mut (dyn FnMut(&str) + Send),
    ) -> Result<Started, StartError> {
        let this = self.clone();
        let channel_id = request.channel.clone();
        let channel = blocking(move || this.channel_record(&channel_id)).await??;
        let agent_name = request
            .agent
            .clone()
            .unwrap_or_else(|| channel.default_agent.clone());
        self.agents.get(&agent_name)?;
        let chosen = self.choose(&channel, request.repos.as_deref())?;

        let this = self.clone();
        let (picked, title, typed) = (channel.clone(), request.title.clone(), request.branch.clone());
        let branch = {
            let chosen = chosen.clone();
            blocking(move || this.pick_for(&picked, &chosen, &title, typed.as_deref())).await??
        };
        let folder = self.layout.session(&channel.folder, &branch.folder);
        let session = SessionRecord {
            id: request
                .id
                .clone()
                .unwrap_or_else(|| format!("se_{}", uuid::Uuid::new_v4().simple())),
            channel: channel.id.clone(),
            machine: request.machine.clone(),
            title: request.title.clone(),
            agent: agent_name,
            folder: folder.clone(),
            created_unix_ms: unix_ms(),
        };
        let mut detail = SessionDetail {
            session: session.id.clone(),
            repos: chosen
                .iter()
                .map(|(owner, name)| SessionRepo {
                    owner: owner.clone(),
                    name: name.clone(),
                    branch: branch.name.clone(),
                    path: folder.join(name),
                })
                .collect(),
            model: request.model.clone(),
            first_message: request.first_message.clone(),
            start: StartStep::Recorded,
            operation: request.operation.clone(),
            revision: 1,
        };

        let early = self
            .record_and_clone(&channel, &session, &mut detail, progress)
            .await;
        if let Err(error) = early {
            // Best effort: the start's own error is the one worth reporting.
            let _ = self.undo(&session).await;
            return Err(error);
        }
        let (detail, agent) = self
            .launch_session(&session, detail, request.cols, request.rows, env, progress)
            .await?;
        Ok(Started {
            session,
            detail,
            agent,
        })
    }

    /// Writes the row and start of a session that lives on another machine
    /// (SL-1, PER-11, D139): the host records it under the id it chose, then asks
    /// that machine to start it. Nothing is cloned or launched here. The start is
    /// saved as done, so [`crate::recover`] after a restart never launches it on
    /// this machine; the other machine's own start does the work. The repositories
    /// are the ones named (or all the channel's), whether or not this machine has
    /// cloned them: the other machine checks its own clones.
    pub async fn place_session(&self, request: &StartRequest) -> Result<SessionRecord, StartError> {
        let this = self.clone();
        let channel_id = request.channel.clone();
        let channel = blocking(move || this.channel_record(&channel_id)).await??;
        let agent_name = request
            .agent
            .clone()
            .unwrap_or_else(|| channel.default_agent.clone());
        let chosen: Vec<RepoName> = match request.repos.as_deref() {
            Some(list) => {
                for (owner, name) in list {
                    if !channel
                        .repos
                        .iter()
                        .any(|r| &r.owner == owner && &r.name == name)
                    {
                        return Err(StartError::UnknownRepo {
                            channel: channel.id.clone(),
                            owner: owner.clone(),
                            name: name.clone(),
                        });
                    }
                }
                list.to_vec()
            }
            None => channel
                .repos
                .iter()
                .map(|r| (r.owner.clone(), r.name.clone()))
                .collect(),
        };
        if chosen.is_empty() {
            return Err(StartError::NoRepos {
                channel: channel.id.clone(),
            });
        }
        let this = self.clone();
        let (picked, repos, title, typed) =
            (channel.clone(), chosen.clone(), request.title.clone(), request.branch.clone());
        let branch = blocking(move || match this.pick_for(&picked, &repos, &title, typed.as_deref()) {
            // This machine may not have the clones (or not reach the remote): the names
            // this channel's sessions use are all it can know.
            Err(StartError::Git { .. }) => this.pick_unseen(&picked, &title),
            other => other,
        })
        .await??;
        let folder = self.layout.session(&channel.folder, &branch.folder);
        let session = SessionRecord {
            id: request
                .id
                .clone()
                .unwrap_or_else(|| format!("se_{}", uuid::Uuid::new_v4().simple())),
            channel: channel.id.clone(),
            machine: request.machine.clone(),
            title: request.title.clone(),
            agent: agent_name,
            folder: folder.clone(),
            created_unix_ms: unix_ms(),
        };
        let detail = SessionDetail {
            session: session.id.clone(),
            repos: chosen
                .iter()
                .map(|(owner, name)| SessionRepo {
                    owner: owner.clone(),
                    name: name.clone(),
                    branch: branch.name.clone(),
                    path: folder.join(name),
                })
                .collect(),
            model: request.model.clone(),
            first_message: request.first_message.clone(),
            start: StartStep::Done,
            operation: request.operation.clone(),
            revision: 1,
        };
        let (store, saved, saved_detail) = (self.store.clone(), session.clone(), detail);
        blocking(move || -> Result<(), StartError> {
            store
                .put_session(&saved)
                .map_err(store_err("save the session"))?;
            store
                .put_session_detail(&saved_detail)
                .map_err(store_err("save the session's start"))
        })
        .await??;
        point("sessions.start.recorded");
        Ok(session)
    }

    /// The branch for `title` given only the names this channel's sessions use.
    fn pick_unseen(&self, channel: &ChannelRecord, title: &str) -> Result<Branch, StartError> {
        let in_channel: HashSet<String> = self
            .store
            .sessions()
            .map_err(store_err("list the sessions"))?
            .into_iter()
            .filter(|s| s.channel == channel.id)
            .map(|s| s.id)
            .collect();
        let mut taken: HashSet<String> = HashSet::new();
        for detail in self
            .store
            .session_details()
            .map_err(store_err("list the sessions"))?
        {
            if in_channel.contains(&detail.session) {
                taken.extend(detail.repos.into_iter().map(|r| r.branch));
            }
        }
        Ok(pick_branch(
            &self.prefix(channel)?,
            &branch_slug(title),
            |name| taken.contains(name),
        ))
    }

    async fn record_and_clone(
        &self,
        channel: &ChannelRecord,
        session: &SessionRecord,
        detail: &mut SessionDetail,
        progress: &mut (dyn FnMut(&str) + Send),
    ) -> Result<(), StartError> {
        let (store, saved, saved_detail) = (self.store.clone(), session.clone(), detail.clone());
        blocking(move || -> Result<(), StartError> {
            store
                .put_session(&saved)
                .map_err(store_err("save the session"))?;
            store
                .put_session_detail(&saved_detail)
                .map_err(store_err("save the session's start"))
        })
        .await??;
        point("sessions.start.recorded");
        progress("write_record");
        self.clone_repos(channel, detail, progress).await?;
        self.set_step(detail, StartStep::Cloned).await
    }

    /// Clones each repository of `detail` whose folder isn't there yet.
    pub(crate) async fn clone_repos(
        &self,
        channel: &ChannelRecord,
        detail: &SessionDetail,
        progress: &mut (dyn FnMut(&str) + Send),
    ) -> Result<(), StartError> {
        for repo in &detail.repos {
            if repo.path.join(".git").exists() {
                continue;
            }
            let main = self.layout.main_clone(&channel.folder, &repo.name);
            let url =
                remote_url(&self.remote_template, &repo.owner, &repo.name).map_err(|source| {
                    StartError::Git {
                        owner: repo.owner.clone(),
                        name: repo.name.clone(),
                        source,
                    }
                })?;
            let (path, branch) = (repo.path.clone(), repo.branch.clone());
            blocking(move || clone_session(&main, &url, &path, &branch))
                .await?
                .map_err(|source| StartError::Git {
                    owner: repo.owner.clone(),
                    name: repo.name.clone(),
                    source,
                })?;
            point("sessions.start.cloned");
            progress(&format!("clone:{}", repo.name));
        }
        Ok(())
    }

    pub(crate) async fn set_step(
        &self,
        detail: &mut SessionDetail,
        step: StartStep,
    ) -> Result<(), StartError> {
        detail.start = step;
        detail.revision += 1;
        let (store, saved) = (self.store.clone(), detail.clone());
        blocking(move || store.put_session_detail(&saved))
            .await?
            .map_err(store_err("save how far the session's start got"))
    }

    /// Launches the session's agent, types the first message, and marks
    /// the start done. From `Launching` on a failure leaves the records
    /// for revive.
    pub(crate) async fn launch_session(
        &self,
        session: &SessionRecord,
        detail: SessionDetail,
        cols: u16,
        rows: u16,
        env: Env<'_>,
        progress: &mut (dyn FnMut(&str) + Send),
    ) -> Result<(SessionDetail, AgentRecord), StartError> {
        let conversation = uuid::Uuid::new_v4().hyphenated().to_string();
        self.launch_with(session, detail, cols, rows, env, progress, conversation, false)
            .await
    }

    /// [`Self::launch_session`] on the given `conversation`; with `resume`
    /// the agent starts by resuming it (a fork's copied conversation).
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn launch_with(
        &self,
        session: &SessionRecord,
        mut detail: SessionDetail,
        cols: u16,
        rows: u16,
        env: Env<'_>,
        progress: &mut (dyn FnMut(&str) + Send),
        conversation: String,
        resume: bool,
    ) -> Result<(SessionDetail, AgentRecord), StartError> {
        self.set_step(&mut detail, StartStep::Launching).await?;
        point("sessions.start.launching");
        progress("launch");
        let kind = self.agents.get(&session.agent)?;
        let plan = kind.launch(&self.home, &session.folder, &conversation);
        let tab = format!("tb_{}", uuid::Uuid::new_v4().simple());
        let socket = self.holders.join(format!("{tab}.sock"));
        let probe = TabRecord {
            id: tab.clone(),
            session: Some(session.id.clone()),
            kind: TabKind::Agent,
            title: session.agent.clone(),
            holder_socket: socket.clone(),
            holder_pid: 0,
            created_unix_ms: unix_ms(),
        };
        let variable_names = env(&probe)
            .iter()
            .map(|(name, _)| name.to_string_lossy().into_owned())
            .collect();
        let launch = AgentLaunch {
            agent: session.agent.clone(),
            folder: session.folder.clone(),
            argv: if resume {
                plan.resume_argv.clone()
            } else {
                plan.argv
            },
            resume_argv: plan.resume_argv,
            variable_names,
            conversation_id: conversation,
            conversation_file: plan.conversation_file,
            holder_socket: socket.clone(),
            cols,
            rows,
        };
        let agent =
            humata2_revive::launch_agent(&self.store, &self.revive, session, &tab, launch, None, env)
                .await
                .map_err(|source| StartError::Revive { source })?;
        if let Some(message) = detail.first_message.clone() {
            type_message(&socket, &message).await?;
        }
        self.set_step(&mut detail, StartStep::Done).await?;
        progress("verify");
        Ok((detail, agent))
    }

    /// Takes a start back: removes the session folder (partial clones
    /// included) and the session with its records. Safe to repeat. A folder
    /// another session also uses (a context-only fork works in its parent's,
    /// D185) is left alone: only this session's records go.
    pub(crate) async fn undo(&self, session: &SessionRecord) -> Result<(), StartError> {
        let (store, id, folder) = (
            self.store.clone(),
            session.id.clone(),
            session.folder.clone(),
        );
        blocking(move || -> Result<(), StartError> {
            let shared = store
                .sessions()
                .map_err(store_err("list the sessions"))?
                .iter()
                .any(|other| other.id != id && other.folder == folder);
            if !shared {
                remove_folder(&folder)?;
            }
            store
                .remove_session(&id)
                .map_err(store_err("remove the session"))
        })
        .await?
    }
}

fn remove_folder(path: &Path) -> Result<(), StartError> {
    match std::fs::remove_dir_all(path) {
        Err(source) if source.kind() != std::io::ErrorKind::NotFound => Err(StartError::Remove {
            path: path.to_owned(),
            source,
        }),
        _ => Ok(()),
    }
}

/// Types `message` into the terminal on `socket` and presses return.
async fn type_message(socket: &Path, message: &str) -> Result<(), StartError> {
    let first = |source| StartError::FirstMessage { source };
    let mut attached = humata2_holder::attach(socket, u64::MAX)
        .await
        .map_err(first)?;
    attached
        .control
        .input(1, message.as_bytes())
        .await
        .map_err(first)?;
    tokio::time::sleep(TYPE_PAUSE).await;
    attached.control.input(2, b"\r").await.map_err(first)
}

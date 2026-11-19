//! Staging a fork for another machine (FK-8, flow 26.5; D195): the parent
//! keeps running where it is. This machine makes the fork's own copy of the
//! parent's clones in the carry stage, on a new branch, the parent's
//! conversation under a new id when the fork carries it, and the fork's
//! records in a side file, then names all of it in the carry manifest. From
//! there the fork travels as a moved session does: the folder and the staged
//! files are copied, the other machine checks every hash and takes the fork
//! in ([`Starter::unpack`]), and its agent starts.
//!
//! This is a child of `carry` so it builds on the carry core's own parts.

use std::fs;
use std::path::PathBuf;

use humata2_store::{
    AgentLaunch, AgentRecord, AgentState, ChannelRecord, SessionDetail, SessionRecord, SessionRepo, StartStep, Store, StoreError,
};

use super::{
    AGENT, Area, AgentFiles, CARRY_DIR, CarryError, Item, Manifest, ManifestProblem, Packed, RECORDS, STAGE, MANIFEST,
    digest_of, folder_name, io, items_under, project_keyed, remove, sha_of_file,
};
use humata2_revive::Env;
use humata2_sessions_core::error::{GitError, StartError};
use humata2_sessions_core::fork::{ForkCopy, copy_clone, copy_conversation, copy_durably};
use humata2_sessions_core::setup::unix_ms;
use humata2_sessions_core::start::{RepoName, Started, Starter, blocking};

/// The folder of a carry operation that holds the fork's staged session folder.
const FORK: &str = "fork";

/// The folder of a carry operation the fork's conversation is written in
/// before it is staged.
const REID: &str = "reid";

/// Why a fork couldn't be staged. Nothing of the parent was changed.
#[derive(Debug, thiserror::Error)]
pub enum ForkPackError {
    #[error("couldn't stage the fork's files for another machine")]
    Carry {
        #[source]
        source: CarryError,
    },

    #[error("couldn't choose or read what the fork starts from; the parent is unchanged, so fork it again once that is fixed")]
    Start {
        #[source]
        source: StartError,
    },

    #[error("couldn't copy {owner}/{name}'s code for the fork; the parent is unchanged, so check the disk has room and fork again")]
    Git {
        owner: String,
        name: String,
        #[source]
        source: GitError,
    },

    #[error("a fork on another machine has its own clones and branch, so a context-only fork can't be staged")]
    ContextOnly,

    #[error("the parent has no agent conversation to carry; fork with the code only")]
    NoConversation,

    #[error("couldn't find where turn {turn} starts in the parent's conversation; the parent is unchanged, so fork from a turn its list of turns shows, or from now")]
    NoTurn { turn: u32 },
}

impl From<CarryError> for ForkPackError {
    fn from(source: CarryError) -> Self {
        Self::Carry { source }
    }
}

fn start(source: StartError) -> ForkPackError {
    ForkPackError::Start { source }
}

/// A fork to stage.
#[derive(Clone, Debug)]
pub struct ForkPack {
    /// The carry operation, which names the stage.
    pub operation: String,
    pub parent: String,
    /// The fork's own id, chosen by the host.
    pub session: String,
    pub title: String,
    pub copy: ForkCopy,
    pub before_turn: u32,
    /// A branch typed in the dialog; None: picked from the title.
    pub branch: Option<String>,
}

/// A fork staged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForkPacked {
    /// `folder` is the staged session folder, under this machine's channel
    /// folder; the copy takes it into the other machine's `sessions`.
    pub packed: Packed,
    /// The branch every clone of the fork is on.
    pub branch: String,
    /// The fork's conversation id; empty for a code-only fork.
    pub conversation_id: String,
}

impl crate::CarryOps<'_> {
    /// Stages `request`'s fork (see the module). Blocking: it runs git and
    /// copies files. A failure leaves nothing staged.
    pub fn pack_fork(&self, request: &ForkPack) -> Result<ForkPacked, ForkPackError> {
        if request.copy == ForkCopy::ContextOnly {
            return Err(ForkPackError::ContextOnly);
        }
        let record = self
            .store
            .session(&request.parent)
            .map_err(|source| CarryError::Store { doing: "read the parent session", source })?
            .ok_or_else(|| CarryError::NoSession { session: request.parent.clone() })?;
        let channel = self.carry_channel(&record.channel)?;
        let dir = self.carry_dir(&channel, &request.operation);
        let staged = self.stage_fork(request, &channel, &dir);
        if staged.is_err() {
            // The error worth reporting is the first one.
            let _ = remove(&dir);
        }
        staged
    }

    fn stage_fork(
        &self,
        request: &ForkPack,
        channel: &ChannelRecord,
        dir: &PathBuf,
    ) -> Result<ForkPacked, ForkPackError> {
        let (parent, parent_detail) = self.parent(&request.parent).map_err(start)?;
        let repos: Vec<RepoName> = parent_detail.repos.iter().map(|r| (r.owner.clone(), r.name.clone())).collect();
        let branch = self
            .pick_fork(channel, &repos, &request.title, request.branch.as_deref())
            .map_err(start)?;
        let folder = self.layout.session(&channel.folder, &branch.folder);

        remove(dir)?;
        let stage = dir.join(STAGE);
        let staged_folder = dir.join(FORK).join(&branch.folder);
        fs::create_dir_all(&staged_folder).map_err(io("create", &staged_folder))?;
        fs::create_dir_all(&stage).map_err(io("create", &stage))?;
        for repo in &parent_detail.repos {
            copy_clone(&repo.path, &staged_folder.join(&repo.name), &branch.name).map_err(|source| ForkPackError::Git {
                owner: repo.owner.clone(),
                name: repo.name.clone(),
                source,
            })?;
        }

        let with_context = request.copy == ForkCopy::CodeAndContext;
        let mut agent_files = AgentFiles::None;
        let mut conversation_id = String::new();
        let mut agent = None;
        if with_context {
            let parent_agent = self
                .store
                .agent(&parent.id)
                .map_err(|source| CarryError::Store {
                    doing: "read the parent's agent",
                    source,
                })?
                .ok_or(ForkPackError::NoConversation)?;
            let kind = self
                .agents
                .get(&parent.agent)
                .map_err(|source| CarryError::Agent { agent: parent.agent.clone(), source })?;
            conversation_id = uuid::Uuid::new_v4().hyphenated().to_string();
            // The fork runs the parent's agent, on the parent's model.
            let plan = kind.launch_with_model(&self.home, &folder, &conversation_id, parent_detail.model.as_deref());
            let name = plan
                .conversation_file
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let target = stage.join(AGENT);
            fs::create_dir_all(&target).map_err(io("create", &target))?;
            let (from, to) = (&parent_agent.launch.conversation_file, target.join(&name));
            // The parent's conversation through its adapter, as a fork on this
            // machine copies it, so the copy names the fork's own id. A kind
            // with no adapter has its file copied as it is.
            let reid = super::placed::forked_for_stage(
                kind,
                &self.home,
                &parent.folder,
                &parent_agent.launch,
                &conversation_id,
                request.before_turn,
                &dir.join(REID),
            )
            .map_err(start)?;
            if let Some(bytes) = reid {
                copy_durably(bytes.as_slice(), &to)
                    .map_err(|source| start(StartError::Copy { path: to.clone(), source }))?;
            } else {
                copy_conversation(from, &to, request.before_turn).map_err(start)?;
            }
            agent_files = if project_keyed(kind) { AgentFiles::Project } else { AgentFiles::File { name } };
            let tab = format!("tb_{}", uuid::Uuid::new_v4().simple());
            agent = Some(AgentRecord {
                session: request.session.clone(),
                tab: tab.clone(),
                launch: AgentLaunch {
                    agent: parent.agent.clone(),
                    folder: folder.clone(),
                    argv: plan.resume_argv.clone(),
                    resume_argv: plan.resume_argv,
                    variable_names: parent_agent.launch.variable_names.clone(),
                    conversation_id: conversation_id.clone(),
                    conversation_file: plan.conversation_file,
                    holder_socket: self.holders.join(format!("{tab}.sock")),
                    cols: parent_agent.launch.cols,
                    rows: parent_agent.launch.rows,
                    cli_version: parent_agent.launch.cli_version.clone(),
                },
                state: AgentState::Stopped,
                generation: 1,
                interruptions: Vec::new(),
                updated_unix_ms: unix_ms(),
            });
        }

        // The fork's records, in a store of their own so nothing of the
        // fork shows on this machine while it is staged.
        let scratch = dir.join("scratch");
        let session = SessionRecord {
            id: request.session.clone(),
            channel: channel.id.clone(),
            machine: String::new(),
            title: request.title.clone(),
            agent: parent.agent.clone(),
            folder: folder.clone(),
            created_unix_ms: unix_ms(),
        };
        let detail = SessionDetail {
            session: session.id.clone(),
            repos: parent_detail
                .repos
                .iter()
                .map(|r| SessionRepo {
                    owner: r.owner.clone(),
                    name: r.name.clone(),
                    branch: branch.name.clone(),
                    path: folder.join(&r.name),
                })
                .collect(),
            model: parent_detail.model.clone(),
            first_message: None,
            // A code-only fork has no agent yet: starting it is the first
            // launch of its session.
            start: if with_context { StartStep::Done } else { StartStep::Cloned },
            operation: None,
            revision: 1,
        };
        let side = stage.join(RECORDS);
        let kept = |source: StoreError| CarryError::Store { doing: "keep the fork's records", source };
        {
            let records = Store::open(&scratch).map_err(kept)?;
            records.put_channel(channel).map_err(kept)?;
            records.put_session(&session).map_err(kept)?;
            records.put_session_detail(&detail).map_err(kept)?;
            if let Some(agent) = &agent {
                records.put_agent(agent).map_err(kept)?;
            }
            records.export_session(&session.id, &side).map_err(kept)?;
        }
        remove(&scratch)?;

        let mut items = items_under(Area::Folder, &staged_folder)?;
        let agent_dir = stage.join(AGENT);
        if agent_dir.is_dir() {
            items.extend(items_under(Area::Agent, &agent_dir)?);
        }
        let (size, sha256) = sha_of_file(&side).map_err(io("read", &side))?;
        items.push(Item { area: Area::Records, rel: RECORDS.to_owned(), size, sha256, link: None });

        let manifest = Manifest {
            operation: request.operation.clone(),
            session: session.id.clone(),
            channel: channel.id.clone(),
            folder: format!("sessions/{}", branch.folder),
            // The parent's paths are what the clones and the copied
            // conversation name; they are rewritten to the fork's own.
            old_folder: parent.folder.to_string_lossy().into_owned(),
            agent: parent.agent.clone(),
            conversation_id: conversation_id.clone(),
            agent_files,
            items: items.clone(),
            // A fork starts with no shell tabs of its parent's.
            tabs: Vec::new(),
        };
        folder_name(&manifest.folder)?;
        let path = stage.join(MANIFEST);
        let bytes = serde_json::to_vec_pretty(&manifest).map_err(|source| CarryError::Manifest {
            path: path.clone(),
            source: ManifestProblem::Parse(source),
        })?;
        fs::write(&path, bytes).map_err(io("write", &path))?;
        Ok(ForkPacked {
            packed: Packed {
                folder: format!("{CARRY_DIR}/{}/{FORK}/{}", request.operation, branch.folder),
                stage: format!("{CARRY_DIR}/{}/{STAGE}", request.operation),
                digest: digest_of(&items),
                files: u32::try_from(items.len()).unwrap_or(u32::MAX),
                bytes: items.iter().map(|item| item.size).sum(),
                agent_running: false,
            },
            branch: branch.name,
            conversation_id,
        })
    }
}

impl crate::CarryOps<'_> {
    /// Starts the agent of a carried session that never had one: a code-only
    /// fork on the machine it was carried to. It gets a conversation of its
    /// own; `progress` hears `launch` and `verify`, as a start's does.
    pub async fn launch_unlaunched(
        &self,
        session: &str,
        cols: u16,
        rows: u16,
        env: Env<'_>,
        progress: &mut (dyn FnMut(&str) + Send),
    ) -> Result<Started, StartError> {
        let (this, id) = (self.0.clone(), session.to_owned());
        let (record, detail) = blocking(move || this.parent(&id)).await??;
        let conversation = uuid::Uuid::new_v4().hyphenated().to_string();
        let (detail, agent) = self
            .launch_with(&record, detail, cols, rows, env, progress, conversation, false)
            .await?;
        Ok(Started { session: record, detail, agent })
    }
}

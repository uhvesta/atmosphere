//! Thin re-export facade over `humata2_sessions_core`, `humata2_sessions_desk`,
//! and `humata2_sessions_carry` during the Phase 4A migration.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use humata2_revive::{Env, ReviveConfig};
use humata2_store::{SessionDetail, SessionRecord, Store, Take};

pub use humata2_sessions_carry::{
    Brought, CARRY_DIR, CHUNKED_FROM, CHUNKS_DIR, CHUNKS_FILE, CLONES_FILE, CarryError,
    CarryStarter, Discard, ForkPack, ForkPackError, ForkPacked, Landing, Packed,
    REUSE_FILE, Unpacked, check_brought, chunk_store_dir, frame_chunk, held_chunks,
};
pub use humata2_sessions_core::{
    AddedRepo, AgentError, AgentKind, Agents, BRANCH_PREFIX_KEY, Boxed, Branch, BranchRename,
    CHANNELS_ROOT, CloneUnits, ContinueRequest, ConversationFormat, DEFAULT_REMOTE_TEMPLATE,
    DESK_CHANNEL, FileChange, FirstMessage, FollowError, ForkCopy, ForkPlan, ForkRequest,
    GateRefusal, GitError, HeldMessages, HumataTools, LaunchPlan, Layout, LayoutError, Leftovers,
    NameError, NewChannel, OwnCopyError, PushError, Recovery, RepoName, RepoWork, Resend,
    RestoredFrom, ReviewStart, SetupError, StartError, StartRequest, StartSignal, Started,
    Starter as CoreStarter, Totals, TurnEnd, TurnsError, USE_WORKTREES_KEY, WorktreeDirs,
    WriteGate, branch_slug, channel_folder, clone_main, clone_main_with, clone_mains,
    clone_mains_with, clone_session, clone_session_with, collect_worktrees_in_folder, commit_all,
    copy_clone_standalone, copy_worktree, create_channel, default_branch, default_prefix,
    effective_prefix, ensure_bare_cache, git_path, handover_file, is_branch_checked_out,
    is_detached_head, is_linked_worktree, local_branches, lock_path_for_bare, partial,
    pick_branch, prune_worktrees, push_branch, remote_branches, remote_url,
    remove_worktrees_in_folder, repair_or_reattach_worktree, repair_worktrees_in_folder,
    repo_work, resolve_worktree_dirs, restore_clone, set_default_prefix, totals,
    use_worktrees_enabled, valid_branch_name, valid_prefix, with_clone_units,
    with_repo_cache_lock, worktree_branches, worktree_main_with, worktree_pull_head_with,
    worktree_restore_with, worktree_session_with,
};
pub use humata2_sessions_desk::{
    ChannelDeskStart, DeskError, DeskFolder, DeskMoveError, DeskMoveRepo, DeskStart, DeskStarter,
    check_repo_name, claim_scratch, clone_repo, clone_repos, clone_work, copy_document, desk_move_repos,
    desk_root, doc_repo, document_lines_under, documents_under, folder_bytes, held_work, new_row,
    repo_clone, repoint_clone, repos_under, scratch_child,
};

/// Facade `Starter` with the same 8 public fields and all core, desk, and carry
/// inherent methods so existing callers in `daemon/humatad` compile unchanged
/// without needing to import `DeskStarter` or `CarryStarter` at every call site.
#[derive(Clone)]
pub struct Starter {
    pub store: Arc<Store>,
    pub layout: Layout,
    pub remote_template: String,
    pub agents: Agents,
    pub home: PathBuf,
    pub revive: ReviveConfig,
    pub holders: PathBuf,
    pub first_message: FirstMessage,
}

impl From<CoreStarter> for Starter {
    fn from(core: CoreStarter) -> Self {
        Self {
            store: core.store,
            layout: core.layout,
            remote_template: core.remote_template,
            agents: core.agents,
            home: core.home,
            revive: core.revive,
            holders: core.holders,
            first_message: core.first_message,
        }
    }
}

impl From<Starter> for CoreStarter {
    fn from(starter: Starter) -> Self {
        Self {
            store: starter.store,
            layout: starter.layout,
            remote_template: starter.remote_template,
            agents: starter.agents,
            home: starter.home,
            revive: starter.revive,
            holders: starter.holders,
            first_message: starter.first_message,
        }
    }
}

impl Starter {
    #[must_use]
    pub fn to_core(&self) -> CoreStarter {
        CoreStarter {
            store: self.store.clone(),
            layout: self.layout.clone(),
            remote_template: self.remote_template.clone(),
            agents: self.agents.clone(),
            home: self.home.clone(),
            revive: self.revive.clone(),
            holders: self.holders.clone(),
            first_message: self.first_message.clone(),
        }
    }

    pub fn preview_branches(
        &self,
        channel: &str,
        title: &str,
    ) -> Result<Vec<(RepoName, Branch)>, StartError> {
        self.to_core().preview_branches(channel, title)
    }

    pub async fn start(
        &self,
        request: StartRequest,
        env: Env<'_>,
        progress: &mut (dyn FnMut(&str) + Send),
    ) -> Result<Started, StartError> {
        self.to_core().start(request, env, progress).await
    }

    pub async fn start_review(
        &self,
        request: StartRequest,
        review: ReviewStart,
        env: Env<'_>,
        progress: &mut (dyn FnMut(&str) + Send),
    ) -> Result<Started, StartError> {
        self.to_core().start_review(request, review, env, progress).await
    }

    pub async fn place_session(&self, request: &StartRequest) -> Result<SessionRecord, StartError> {
        self.to_core().place_session(request).await
    }

    pub async fn add_repo_to_session(
        &self,
        session: &str,
        repo: &RepoName,
    ) -> Result<AddedRepo, StartError> {
        self.to_core().add_repo_to_session(session, repo).await
    }

    pub async fn continue_in(
        &self,
        request: ContinueRequest,
        env: Env<'_>,
        progress: &mut (dyn FnMut(&str) + Send),
    ) -> Result<Started, StartError> {
        self.to_core().continue_in(request, env, progress).await
    }

    pub fn hands_over(
        &self,
        parent_agent: &str,
        agent: &str,
        copy: ForkCopy,
    ) -> Result<bool, StartError> {
        self.to_core().hands_over(parent_agent, agent, copy)
    }

    pub fn preview_fork(
        &self,
        parent: &str,
        title: &str,
        typed: Option<&str>,
    ) -> Result<ForkPlan, StartError> {
        self.to_core().preview_fork(parent, title, typed)
    }

    pub fn fork_sizes(&self, parent: &str) -> Result<Vec<(RepoName, u64)>, StartError> {
        self.to_core().fork_sizes(parent)
    }

    pub fn check_fork_branch(
        &self,
        parent: &str,
        title: &str,
        typed: &str,
    ) -> Result<(), StartError> {
        self.to_core().check_fork_branch(parent, title, typed)
    }

    pub async fn fork(
        &self,
        request: ForkRequest,
        env: Env<'_>,
        progress: &mut (dyn FnMut(&str) + Send),
    ) -> Result<Started, StartError> {
        self.to_core().fork(request, env, progress).await
    }

    pub fn check_own_copy(&self, fork: &str) -> Result<(), OwnCopyError> {
        self.to_core().check_own_copy(fork)
    }

    pub async fn give_own_copy(
        &self,
        fork: &str,
        env: Env<'_>,
        progress: &mut (dyn FnMut(&str) + Send),
    ) -> Result<bool, OwnCopyError> {
        self.to_core().give_own_copy(fork, env, progress).await
    }

    pub fn plan_branch_rename(
        &self,
        session: &str,
        new_title: &str,
    ) -> Result<BranchRename, StartError> {
        self.to_core().plan_branch_rename(session, new_title)
    }

    pub fn restore_repo_blocking(
        &self,
        channel_folder: &str,
        owner: &str,
        name: &str,
        url: &str,
        path: &Path,
        branch: &str,
    ) -> Result<RestoredFrom, GitError> {
        self.to_core()
            .restore_repo_blocking(channel_folder, owner, name, url, path, branch)
    }

    pub async fn reclone(
        &self,
        session: &SessionRecord,
        detail: &SessionDetail,
    ) -> Result<Vec<(String, RestoredFrom)>, StartError> {
        self.to_core().reclone(session, detail).await
    }

    pub async fn start_desk(
        &self,
        request: DeskStart,
        env: Env<'_>,
        progress: &mut (dyn FnMut(&str) + Send),
    ) -> Result<Started, DeskError> {
        DeskStarter::start_desk(&self.to_core(), request, env, progress).await
    }

    pub async fn start_channel_desk(
        &self,
        request: ChannelDeskStart,
        env: Env<'_>,
        progress: &mut (dyn FnMut(&str) + Send),
    ) -> Result<Started, DeskError> {
        DeskStarter::start_channel_desk(&self.to_core(), request, env, progress).await
    }

    pub fn desk_move_branch(
        &self,
        channel: &str,
        repos: &[RepoName],
        title: &str,
        typed: Option<&str>,
    ) -> Result<Branch, StartError> {
        DeskStarter::desk_move_branch(&self.to_core(), channel, repos, title, typed)
    }

    pub fn desk_move_remote(
        &self,
        channel_folder: &str,
        owner: &str,
        name: &str,
    ) -> Result<String, GitError> {
        DeskStarter::desk_move_remote(&self.to_core(), channel_folder, owner, name)
    }

    pub fn pack(&self, operation: &str, session: &str) -> Result<Packed, CarryError> {
        CarryStarter::pack(&self.to_core(), operation, session)
    }

    pub fn pack_for(
        &self,
        operation: &str,
        session: &str,
        target: Option<&Path>,
    ) -> Result<Packed, CarryError> {
        CarryStarter::pack_for(&self.to_core(), operation, session, target)
    }

    pub fn shell_holders(&self, session: &str) -> Result<Vec<(PathBuf, u32)>, CarryError> {
        CarryStarter::shell_holders(&self.to_core(), session)
    }

    pub fn unpack(
        &self,
        operation: &str,
        session: &str,
        channel: &str,
        take: Take,
    ) -> Result<Unpacked, CarryError> {
        CarryStarter::unpack(&self.to_core(), operation, session, channel, take)
    }

    pub fn abort_incoming(
        &self,
        operation: &str,
        session: &str,
        channel: &str,
        folder: &str,
        take: Take,
    ) -> Result<(), CarryError> {
        CarryStarter::abort_incoming(&self.to_core(), operation, session, channel, folder, take)
    }

    pub fn discards(&self, session: &str) -> Result<Vec<Discard>, CarryError> {
        CarryStarter::discards(&self.to_core(), session)
    }

    pub fn forget_moved(
        &self,
        operation: &str,
        session: &str,
        channel: &str,
        take: Take,
    ) -> Result<(), CarryError> {
        CarryStarter::forget_moved(&self.to_core(), operation, session, channel, take)
    }

    pub fn unstage(&self, operation: &str, channel: &str) -> Result<(), CarryError> {
        CarryStarter::unstage(&self.to_core(), operation, channel)
    }

    pub fn pack_fork(&self, request: &ForkPack) -> Result<ForkPacked, ForkPackError> {
        CarryStarter::pack_fork(&self.to_core(), request)
    }

    pub async fn launch_unlaunched(
        &self,
        session: &str,
        cols: u16,
        rows: u16,
        env: Env<'_>,
        progress: &mut (dyn FnMut(&str) + Send),
    ) -> Result<Started, StartError> {
        CarryStarter::launch_unlaunched(&self.to_core(), session, cols, rows, env, progress).await
    }

    pub fn pack_landing(
        &self,
        operation: &str,
        session: &str,
        landing: &Landing,
    ) -> Result<Packed, CarryError> {
        CarryStarter::pack_landing(&self.to_core(), operation, session, landing)
    }

    pub fn check_landing(&self, session: &str) -> Result<(), CarryError> {
        CarryStarter::check_landing(&self.to_core(), session)
    }

    pub fn left_behind(&self, operation: &str, channel: &str) -> Result<Vec<Discard>, CarryError> {
        CarryStarter::left_behind(&self.to_core(), operation, channel)
    }

    pub fn land_copy(
        &self,
        operation: &str,
        session: &str,
        channel: &str,
        packed: &Packed,
    ) -> Result<PathBuf, CarryError> {
        CarryStarter::land_copy(&self.to_core(), operation, session, channel, packed)
    }

    pub fn resync_landing(
        &self,
        operation: &str,
        session: &str,
        landing: &Landing,
    ) -> Result<(), CarryError> {
        CarryStarter::resync_landing(&self.to_core(), operation, session, landing)
    }

    pub fn unpack_landed(
        &self,
        operation: &str,
        session: &str,
        channel: &str,
    ) -> Result<Unpacked, CarryError> {
        CarryStarter::unpack_landed(&self.to_core(), operation, session, channel)
    }

    pub fn take_landing_back(
        &self,
        operation: &str,
        session: &str,
        channel: &str,
        unpacked: bool,
    ) -> Result<(), CarryError> {
        CarryStarter::take_landing_back(&self.to_core(), operation, session, channel, unpacked)
    }

    pub fn carry_target(&self, channel: &str, rel: &str) -> Result<PathBuf, CarryError> {
        CarryStarter::carry_target(&self.to_core(), channel, rel)
    }
}

pub async fn recover(starter: &Starter, env: Env<'_>, left: &Leftovers) -> Vec<Recovery> {
    let core = starter.to_core();
    humata2_sessions_core::recover(&core, env, left).await
}

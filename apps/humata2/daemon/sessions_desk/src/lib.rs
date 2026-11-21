mod channel_desk;
mod desk;
mod desk_move;
pub mod doc_repo;
pub mod tab_limits;

pub use channel_desk::ChannelDeskStart;
pub use desk::{
    DESK_CHANNEL, DeskError, DeskFolder, DeskStart, check_repo_name, claim_scratch, clone_repo,
    clone_repos, clone_work, copy_document, desk_root, document_lines_under, documents_under,
    folder_bytes, held_work, new_row, repo_clone, repos_under, scratch_child,
};
pub use desk_move::{DeskMoveError, DeskMoveRepo, desk_move_repos, repoint_clone};
pub use tab_limits::{MAX_TABS_PER_KIND, TabCapError, check_tab_cap, flat_tab_cap};

pub(crate) struct DeskOps<'a>(pub(crate) &'a humata2_sessions_core::Starter);

impl std::ops::Deref for DeskOps<'_> {
    type Target = humata2_sessions_core::Starter;
    fn deref(&self) -> &Self::Target {
        self.0
    }
}

pub trait DeskStarter {
    fn start_desk<'a>(
        &'a self,
        request: DeskStart,
        env: humata2_revive::Env<'a>,
        progress: &'a mut (dyn FnMut(&str) + Send),
    ) -> impl std::future::Future<Output = Result<humata2_sessions_core::Started, DeskError>> + Send + 'a;

    fn start_channel_desk<'a>(
        &'a self,
        request: ChannelDeskStart,
        env: humata2_revive::Env<'a>,
        progress: &'a mut (dyn FnMut(&str) + Send),
    ) -> impl std::future::Future<Output = Result<humata2_sessions_core::Started, DeskError>> + Send + 'a;

    fn desk_move_branch(
        &self,
        channel: &str,
        repos: &[humata2_sessions_core::RepoName],
        title: &str,
        typed: Option<&str>,
    ) -> Result<humata2_sessions_core::Branch, humata2_sessions_core::StartError>;

    fn desk_move_remote(
        &self,
        channel_folder: &str,
        owner: &str,
        name: &str,
    ) -> Result<String, humata2_sessions_core::GitError>;
}

impl DeskStarter for humata2_sessions_core::Starter {
    fn start_desk<'a>(
        &'a self,
        request: DeskStart,
        env: humata2_revive::Env<'a>,
        progress: &'a mut (dyn FnMut(&str) + Send),
    ) -> impl std::future::Future<Output = Result<humata2_sessions_core::Started, DeskError>> + Send + 'a {
        async move { DeskOps(self).start_desk(request, env, progress).await }
    }

    fn start_channel_desk<'a>(
        &'a self,
        request: ChannelDeskStart,
        env: humata2_revive::Env<'a>,
        progress: &'a mut (dyn FnMut(&str) + Send),
    ) -> impl std::future::Future<Output = Result<humata2_sessions_core::Started, DeskError>> + Send + 'a {
        async move { DeskOps(self).start_channel_desk(request, env, progress).await }
    }

    fn desk_move_branch(
        &self,
        channel: &str,
        repos: &[humata2_sessions_core::RepoName],
        title: &str,
        typed: Option<&str>,
    ) -> Result<humata2_sessions_core::Branch, humata2_sessions_core::StartError> {
        DeskOps(self).desk_move_branch(channel, repos, title, typed)
    }

    fn desk_move_remote(
        &self,
        channel_folder: &str,
        owner: &str,
        name: &str,
    ) -> Result<String, humata2_sessions_core::GitError> {
        DeskOps(self).desk_move_remote(channel_folder, owner, name)
    }
}

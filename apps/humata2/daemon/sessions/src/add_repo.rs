//! Adding a repository to a session that is already running (CH-21, FC-3,
//! FC-11): the session gets its own clone of the repo on a branch named from
//! its title, beside the clones it has. The channel's main clone is the
//! source, so the repo must be in the channel with its main clone ready.

use humata2_store::{MainClone, SessionRepo};

use crate::error::{GitError, StartError};
use crate::git::{clone_session, git, remote_url};
use crate::start::{RepoName, Starter, blocking, store_err};

/// What adding a repo to a session did.
#[derive(Clone, Debug)]
pub struct AddedRepo {
    /// The session's clone of the repo.
    pub repo: SessionRepo,
    /// False when the session already had the clone, so nothing was made.
    pub cloned: bool,
}

impl Starter {
    /// Clones `repo` for session `session` on a branch named as FC-11 and
    /// records it in the session's repos. A repo the session already has is
    /// left as it is; a clone a cut run left in the folder without a record is
    /// recorded as it stands. Blocking git runs on a blocking thread.
    pub async fn add_repo_to_session(
        &self,
        session: &str,
        repo: &RepoName,
    ) -> Result<AddedRepo, StartError> {
        let this = self.clone();
        let (key, wanted) = (session.to_owned(), repo.clone());
        blocking(move || this.add_repo_blocking(&key, &wanted)).await?
    }

    fn add_repo_blocking(&self, session: &str, repo: &RepoName) -> Result<AddedRepo, StartError> {
        let (owner, name) = repo;
        let record = self
            .store
            .session(session)
            .map_err(store_err("read the session"))?
            .ok_or_else(|| StartError::NoSession { id: session.to_owned() })?;
        let mut detail = self
            .store
            .session_detail(session)
            .map_err(store_err("read the session's repos"))?
            .ok_or_else(|| StartError::NoSession { id: session.to_owned() })?;
        if let Some(have) = detail.repos.iter().find(|have| &have.owner == owner && &have.name == name)
            && have.path.join(".git").exists()
        {
            return Ok(AddedRepo { repo: have.clone(), cloned: false });
        }
        let channel = self.channel_record(&record.channel)?;
        let in_channel = channel
            .repos
            .iter()
            .find(|have| &have.owner == owner && &have.name == name)
            .ok_or_else(|| StartError::UnknownRepo {
                channel: channel.id.clone(),
                owner: owner.clone(),
                name: name.clone(),
            })?;
        if in_channel.main != MainClone::Ready {
            return Err(StartError::NotReady {
                owner: owner.clone(),
                name: name.clone(),
                state: match &in_channel.main {
                    MainClone::Failed { reason } => format!("the clone failed: {reason}"),
                    _ => "still cloning".to_owned(),
                },
            });
        }
        let git_error = |source: GitError| StartError::Git {
            owner: owner.clone(),
            name: name.clone(),
            source,
        };
        let path = record.folder.join(name);
        let branch = if path.join(".git").exists() {
            // A clone a cut run left without its record: it is kept as it is.
            git(Some(&path), &["rev-parse", "--abbrev-ref", "HEAD"])
                .map_err(git_error)?
                .trim()
                .to_owned()
        } else {
            // This session's own branch and folder don't count as taken: the
            // title's name is used when the repo has no branch of it yet.
            let branch = self.pick_excluding(&channel, std::slice::from_ref(repo), &record.title, Some(session))?;
            let main = self.layout.main_clone(&channel.folder, name);
            let url = remote_url(&self.remote_template, owner, name).map_err(git_error)?;
            clone_session(&main, &url, &path, &branch.name).map_err(git_error)?;
            branch.name
        };
        let added = SessionRepo {
            owner: owner.clone(),
            name: name.clone(),
            branch,
            path,
        };
        detail.repos.retain(|have| !(&have.owner == owner && &have.name == name));
        detail.repos.push(added.clone());
        detail.revision += 1;
        self.store
            .put_session_detail(&detail)
            .map_err(store_err("save the session's repos"))?;
        Ok(AddedRepo { repo: added, cloned: true })
    }
}

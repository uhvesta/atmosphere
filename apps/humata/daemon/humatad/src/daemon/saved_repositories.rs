//! Saved repositories, to pick from when making or changing a project, and
//! the GitHub owners whose repositories are listed among them: once when
//! one is added, every few hours after, and when the person asks.

use std::time::Duration;

use humata_wire::{describe, pb};

use super::Daemon;
use crate::error::{DaemonError, OwnerError};
use crate::github::GitHubError;
use crate::store::{OwnedRepository, Owner, SavedRepository, Source, now_unix_ms};

/// How long after the daemon starts owners are first looked at.
const FIRST_LOOK: Duration = Duration::from_secs(60);
/// How often owners are looked at; one that failed is tried again then.
const LOOK_EVERY: Duration = Duration::from_secs(30 * 60);
/// How old an owner's listing may get before it's listed again.
const LISTING_LASTS: Duration = Duration::from_secs(6 * 60 * 60);

/// The longest GitHub login.
const MOST_LOGIN: usize = 39;

impl Daemon {
    pub(super) fn saved_repositories(&self) -> Result<pb::SavedRepositories, DaemonError> {
        Ok(pb::SavedRepositories {
            repositories: self
                .store
                .saved_repositories()?
                .into_iter()
                .map(saved_to_wire)
                .collect(),
        })
    }

    pub(super) fn rename_saved_repository(
        &self,
        rename: &pb::RenameSavedRepository,
    ) -> Result<(), DaemonError> {
        let name = rename.name.trim();
        if name.is_empty() {
            return Err(DaemonError::NoRepositoryName);
        }
        Ok(self.store.rename_saved_repository(rename.id, name)?)
    }

    /// Takes a repository out of the list; projects that have it keep it.
    pub(super) fn remove_saved_repository(&self, id: i64) -> Result<(), DaemonError> {
        Ok(self.store.remove_saved_repository(id)?)
    }

    pub(super) fn repository_owners(&self) -> Result<pb::RepositoryOwners, DaemonError> {
        Ok(pb::RepositoryOwners {
            owners: self
                .store
                .repository_owners()?
                .into_iter()
                .map(owner_to_wire)
                .collect(),
        })
    }

    /// Saves the owner `given` names, and lists its repositories in the
    /// background.
    pub(super) fn add_repository_owner(&self, given: &str) -> Result<(), DaemonError> {
        let login = owner_login(given).ok_or_else(|| DaemonError::BadOwnerName {
            login: given.trim().to_owned(),
        })?;
        self.store.add_repository_owner(&login)?;
        let me = self.me.clone();
        tokio::spawn(async move {
            if let Some(daemon) = me.upgrade() {
                daemon.refresh_repository_owners(Some(&login)).await;
            }
        });
        Ok(())
    }

    /// Removes an owner and the repositories listed for it.
    pub(super) fn remove_repository_owner(&self, given: &str) -> Result<(), DaemonError> {
        let login = owner_login(given).unwrap_or_else(|| given.trim().to_owned());
        Ok(self.store.remove_repository_owner(&login)?)
    }

    /// Lists the repositories of `only`, or of every owner, on GitHub now,
    /// and returns once done. Each failure is kept on its owner, for the
    /// person, rather than returned.
    pub(super) async fn refresh_repository_owners_now(
        &self,
        only: &str,
    ) -> Result<(), DaemonError> {
        let only = if only.trim().is_empty() {
            None
        } else {
            let login = owner_login(only).unwrap_or_else(|| only.trim().to_owned());
            Some(self.store.repository_owner(&login)?)
        };
        self.refresh_repository_owners(only.as_deref()).await;
        Ok(())
    }

    /// Lists the repositories of `only`, or of every owner, one listing at
    /// a time.
    async fn refresh_repository_owners(&self, only: Option<&str>) {
        let _listing = self.owners_listing.lock().await;
        let logins = match only {
            Some(login) => vec![login.to_owned()],
            None => match self.store.repository_owners() {
                Ok(owners) => owners.into_iter().map(|owner| owner.login).collect(),
                Err(error) => {
                    eprintln!(
                        "humatad: couldn't read the GitHub owners to list: {}",
                        describe(&error)
                    );
                    return;
                }
            },
        };
        for login in logins {
            self.refresh_owner(&login).await;
        }
    }

    /// Lists the owners whose listing is old, or never was.
    async fn refresh_due_owners(&self) {
        let lasts = i64::try_from(LISTING_LASTS.as_millis()).unwrap_or(i64::MAX);
        let due = match self
            .store
            .repository_owners_due(now_unix_ms().saturating_sub(lasts))
        {
            Ok(due) => due,
            Err(error) => {
                eprintln!(
                    "humatad: couldn't read the GitHub owners to list: {}",
                    describe(&error)
                );
                return;
            }
        };
        if due.is_empty() {
            return;
        }
        let _listing = self.owners_listing.lock().await;
        for login in due {
            self.refresh_owner(&login).await;
        }
    }

    async fn refresh_owner(&self, login: &str) {
        let kept = match self.owned_repositories(login).await {
            Ok(listed) => self.store.repository_owner_listed(login, &listed),
            Err(error) => self.store.repository_owner_failed(login, &describe(&error)),
        };
        if let Err(error) = kept {
            eprintln!(
                "humatad: couldn't save the repositories of {login}: {}",
                describe(&error)
            );
        }
    }

    /// `login`'s repositories on GitHub, by This Mac's token.
    async fn owned_repositories(&self, login: &str) -> Result<Vec<OwnedRepository>, OwnerError> {
        let failed = |source| OwnerError::List {
            login: login.to_owned(),
            source,
        };
        let (github, _) = self.github().await.map_err(failed)?;
        match github.owned_repos(login).await {
            Ok(listed) => Ok(listed
                .into_iter()
                .map(|repo| OwnedRepository {
                    name: repo.name,
                    location: repo.html_url,
                })
                .collect()),
            Err(GitHubError::NotFound { .. }) => Err(OwnerError::NotFound {
                login: login.to_owned(),
            }),
            Err(source) => Err(failed(source)),
        }
    }
}

/// Lists owners whose listing got old while the daemon runs.
pub(super) async fn follow(daemon: std::sync::Weak<Daemon>) {
    // The daemon has only just started, so this waits a while.
    tokio::time::sleep(FIRST_LOOK).await;
    loop {
        let Some(running) = daemon.upgrade() else {
            return;
        };
        running.refresh_due_owners().await;
        drop(running);
        tokio::time::sleep(LOOK_EVERY).await;
    }
}

/// The GitHub login `given` names, as `octo-org`, `@octo-org` or
/// `https://github.com/octo-org`; none if it can't be one.
fn owner_login(given: &str) -> Option<String> {
    let given = given.trim().trim_end_matches('/');
    let login = ["https://github.com/", "http://github.com/", "github.com/"]
        .iter()
        .find_map(|prefix| given.strip_prefix(prefix))
        .or_else(|| given.strip_prefix('@'))
        .unwrap_or(given);
    let fits = !login.is_empty()
        && login.len() <= MOST_LOGIN
        && login
            .chars()
            .all(|letter| letter.is_ascii_alphanumeric() || letter == '-')
        && !login.starts_with('-')
        && !login.ends_with('-')
        && !login.contains("--");
    fits.then(|| login.to_owned())
}

fn saved_to_wire(saved: SavedRepository) -> pb::SavedRepository {
    pb::SavedRepository {
        id: saved.id,
        name: saved.name,
        location: saved.location,
        source: match saved.source {
            Source::Used => pb::RepositorySource::Used,
            Source::GitHub => pb::RepositorySource::Github,
        }
        .into(),
        owner: saved.owner.unwrap_or_default(),
        added_unix_ms: saved.added_unix_ms,
        last_used_unix_ms: saved.last_used_unix_ms.unwrap_or_default(),
    }
}

fn owner_to_wire(owner: Owner) -> pb::RepositoryOwner {
    pb::RepositoryOwner {
        login: owner.login,
        added_unix_ms: owner.added_unix_ms,
        refreshed_unix_ms: owner.refreshed_unix_ms.unwrap_or_default(),
        error: owner.error.unwrap_or_default(),
        repositories: owner.repositories,
    }
}

#[cfg(test)]
mod tests {
    use super::owner_login;

    #[test]
    fn an_owner_is_taken_by_its_login_its_handle_or_its_page() {
        for given in [
            "octo-org",
            " @octo-org ",
            "https://github.com/octo-org/",
            "github.com/octo-org",
        ] {
            assert_eq!(owner_login(given).as_deref(), Some("octo-org"), "{given}");
        }
    }

    #[test]
    fn what_cant_be_a_github_login_isnt_taken() {
        let long = "a".repeat(40);
        for given in [
            "",
            "@",
            "-octo",
            "octo-",
            "oc--to",
            "octo/app",
            "octo org",
            "https://github.com/octo/app",
            long.as_str(),
        ] {
            assert_eq!(owner_login(given), None, "{given}");
        }
    }
}

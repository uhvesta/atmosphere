//! A project's repositories as folders on this Mac: a folder as it is, and
//! one added by its URL as Humata's clone of it in the cache, made once and
//! fetched before each session here. Hosts clone from the URL themselves.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;

use humata_agent::Repository;
use humata_wire::pb;
use sha2::{Digest as _, Sha256};

use super::{Daemon, folder_name};

/// Where the Mac's clones of repositories added by their URLs are kept.
pub struct Clones {
    /// Where new ones go: not a cache, since they hold the sessions'
    /// branches and snapshots.
    pub dir: PathBuf,
    /// Where an older daemon put them, in the cache folder; one found there
    /// is used where it is, as projects and sessions name its path.
    pub old: Option<PathBuf>,
}
use crate::error::DaemonError;
use crate::fresh;
use crate::store::Project;

impl Daemon {
    /// `wanted` as a project keeps them, named uniquely, with the URL each
    /// one added by its URL was cloned from.
    pub(super) async fn resolve_repositories(
        &self,
        wanted: Vec<pb::Repository>,
    ) -> Result<(Vec<Repository>, BTreeMap<String, String>), DaemonError> {
        if wanted.is_empty() {
            return Err(DaemonError::NoRepositories);
        }
        let mut repositories: Vec<Repository> = Vec::new();
        let mut urls = BTreeMap::new();
        for repository in wanted {
            let url = repository.url.trim().to_owned();
            let (path, from) = if url.is_empty() {
                (folder(PathBuf::from(repository.path))?, None)
            } else {
                (self.clone_of(&url).await?, Some(url))
            };
            let wanted = Some(repository.name)
                .filter(|name| !name.is_empty())
                .or_else(|| from.as_deref().and_then(url_name))
                .or_else(|| {
                    path.file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                });
            let taken: Vec<&str> = repositories
                .iter()
                .map(|known| known.name.as_str())
                .collect();
            let name = folder_name(wanted.as_deref(), &taken);
            if let Some(url) = from {
                urls.insert(name.clone(), url);
            }
            repositories.push(Repository { name, path });
        }
        Ok((repositories, urls))
    }

    /// The Mac's clone of `url`, cloned now if there's none yet.
    async fn clone_of(&self, url: &str) -> Result<PathBuf, DaemonError> {
        let name = clone_name(url);
        let path = self.clones.dir.join(&name);
        if path.join(".git").exists() {
            return Ok(path);
        }
        if let Some(old) = &self.clones.old {
            let kept = old.join(&name);
            if kept.join(".git").exists() {
                return Ok(kept);
            }
        }
        std::fs::create_dir_all(&self.clones.dir).map_err(|source| DaemonError::CreateClones {
            path: self.clones.dir.clone(),
            source,
        })?;
        // Cloned beside it then moved, so a clone cut short is never taken
        // for one.
        let partial = path.with_extension(format!("partial-{}", uuid::Uuid::new_v4()));
        let output = tokio::process::Command::new("git")
            .args(["clone", "--quiet", "--", url])
            .arg(&partial)
            .stdin(Stdio::null())
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .await
            .map_err(|source| DaemonError::RunGit { source })?;
        if !output.status.success() {
            let _ = std::fs::remove_dir_all(&partial);
            return Err(DaemonError::CloneUrl {
                url: url.to_owned(),
                output: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        if std::fs::rename(&partial, &path).is_err() {
            // Another project cloned it meanwhile; that clone is as good.
            let _ = std::fs::remove_dir_all(&partial);
        }
        if path.join(".git").exists() {
            Ok(path)
        } else {
            Err(DaemonError::CreateClones {
                path,
                source: std::io::Error::from(std::io::ErrorKind::NotFound),
            })
        }
    }

    /// Fetches `project`'s repositories on this Mac: its clones of those
    /// added by URL move to their remote's newest code, and folders keep
    /// their checkouts. Returns what a session started from them should be
    /// told, as that one couldn't be fetched and is used as last fetched.
    pub(super) async fn refresh(&self, project: &Project) -> Vec<String> {
        // All at once, each given only so long, so a session in several
        // repositories waits for the slowest, not for their sum.
        let fetching: Vec<_> = project
            .repositories
            .iter()
            .map(|repository| {
                let kind = if project.urls.contains_key(&repository.name) {
                    fresh::Kind::Clone
                } else {
                    fresh::Kind::Folder
                };
                let (path, name) = (repository.path.clone(), repository.name.clone());
                tokio::spawn(async move {
                    let git = || tokio::process::Command::new("git");
                    fresh::update(&git, &path, &name, kind, "on this Mac").await
                })
            })
            .collect();
        let mut notices = Vec::new();
        for fetched in fetching {
            // One that panicked says nothing, as one that fetched does.
            notices.extend(fetched.await.ok().flatten());
        }
        notices
    }
}

/// `path` if it's an absolute path to a folder.
fn folder(path: PathBuf) -> Result<PathBuf, DaemonError> {
    let not_a_folder = |source| DaemonError::NotAFolder {
        repository: path.clone(),
        source,
    };
    if !path.is_absolute() {
        return Err(not_a_folder(None));
    }
    let metadata = std::fs::metadata(&path).map_err(|source| not_a_folder(Some(source)))?;
    if !metadata.is_dir() {
        return Err(not_a_folder(None));
    }
    Ok(path)
}

/// The clone's folder name: the repository's name and a hash of the URL,
/// so two URLs with the same name don't share one.
fn clone_name(url: &str) -> String {
    let hash = Sha256::digest(url.as_bytes());
    let hex: String = hash
        .iter()
        .take(6)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    match url_name(url) {
        Some(name) => format!("{}-{hex}", folder_name(Some(&name), &[])),
        None => hex,
    }
}

/// The repository's name in its URL: `name` in `…/owner/name.git`.
fn url_name(url: &str) -> Option<String> {
    let last = url
        .trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()?
        .trim_end_matches(".git");
    (!last.is_empty()).then(|| last.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_is_named_after_its_repository() {
        for url in [
            "https://github.com/owner/app.git",
            "https://github.com/owner/app/",
            "git@github.com:owner/app.git",
            "file:///srv/git/app",
        ] {
            assert_eq!(url_name(url).as_deref(), Some("app"), "{url}");
        }
    }

    #[test]
    fn two_urls_of_one_name_get_their_own_clones() {
        let one = clone_name("https://github.com/one/app.git");
        let two = clone_name("https://github.com/two/app.git");
        assert_ne!(one, two);
        assert!(one.starts_with("app-"));
        assert_eq!(one, clone_name("https://github.com/one/app.git"));
    }
}

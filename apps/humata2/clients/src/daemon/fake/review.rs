use super::FakeDaemon;
use crate::daemon::{DaemonError, Operation, Reply};
use crate::model::{
    DiffBase, PushChoice, PushPlan, PushRepo, RepoBase, RepoBases, RepoDiff, RepoDiffSpec,
    SessionId,
};

pub(super) fn read_file(
    daemon: &FakeDaemon,
    session: &SessionId,
    repo: &str,
    path: &str,
) -> Reply<String> {
    let key = (session.clone(), repo.to_owned(), path.to_owned());
    daemon.reply(
        None,
        format!("read {session} {repo}/{path}"),
        move |inner| {
            inner.session_mut(&key.0)?;
            inner
                .workspace
                .file_contents
                .get(&key)
                .cloned()
                .ok_or_else(|| DaemonError::NotFound {
                    what: format!("File {}/{} in {}", key.1, key.2, key.0),
                })
        },
    )
}

/// The session's current changes: what a test set, else the workspace's.
fn current(inner: &super::Inner, session: &SessionId) -> Vec<RepoDiff> {
    inner.diffs.get(session).cloned().unwrap_or_else(|| {
        inner
            .workspace
            .diffs
            .iter()
            .filter(|diff| &diff.session == session)
            .cloned()
            .collect()
    })
}

pub(super) fn diffs(
    daemon: &FakeDaemon,
    session: &SessionId,
    base: &DiffBase,
) -> Reply<Vec<RepoDiff>> {
    let session = session.clone();
    let base = base.clone();
    daemon.reply(
        Some(Operation::Diffs),
        format!("diffs {session} {base:?}"),
        move |inner| {
            inner.session_mut(&session)?;
            // The daemon reads every repository at once, so one failing
            // repository fails the whole read (DS-35).
            let mut failing: Vec<(&String, &String)> = inner
                .failing_repos
                .iter()
                .filter(|((held, _), _)| *held == session)
                .map(|((_, repo), message)| (repo, message))
                .collect();
            failing.sort();
            if let Some((_, message)) = failing.first() {
                return Err(scripted(message));
            }
            read_diffs(inner, &session, &base)
        },
    )
}

/// One repository's changes against `base` (DS-35): what [`diffs`] reads,
/// for that repository alone. Empty when it has no changes.
pub(super) fn repo_diff(
    daemon: &FakeDaemon,
    session: &SessionId,
    base: &DiffBase,
    repo: &str,
) -> Reply<Vec<RepoDiff>> {
    let session = session.clone();
    let base = base.clone();
    let repo = repo.to_owned();
    daemon.reply(
        Some(Operation::Diffs),
        format!("repo diff {session} {repo} {base:?}"),
        move |inner| {
            inner.session_mut(&session)?;
            if let Some(message) = inner.failing_repos.get(&(session.clone(), repo.clone())) {
                return Err(scripted(message));
            }
            let mut diffs = read_diffs(inner, &session, &base)?;
            diffs.retain(|diff| diff.repo == repo);
            Ok(diffs)
        },
    )
}

fn scripted(message: &str) -> DaemonError {
    DaemonError::Scripted {
        operation: Operation::Diffs,
        message: message.to_owned(),
    }
}

/// Every repository's changes in the session against `base`.
fn read_diffs(
    inner: &mut super::Inner,
    session: &SessionId,
    base: &DiffBase,
) -> Result<Vec<RepoDiff>, DaemonError> {
    let home = inner.session_mut(session)?.repo.clone();
    if let Some(repo) = missing_default(inner, session, base) {
        return Err(DaemonError::NoDefaultBranch { repo });
    }
    if let DiffBase::Spec(specs) = base {
        return spec_diffs(inner, session, &home, specs);
    }
    let mut diffs = current(inner, session);
    if *base == DiffBase::SinceLastReviewed
        && let Some(seen) = inner.reviewed.get(session)
    {
        for diff in &mut diffs {
            diff.base = "last reviewed".to_owned();
            let before = seen.iter().find(|old| old.repo == diff.repo);
            diff.files.retain(|file| {
                before.is_none_or(|old| !old.files.iter().any(|same| same == file))
            });
        }
    }
    Ok(diffs)
}

/// A spec gives each repository of the session its own base (VIEWS C1). A
/// repository the spec leaves out is compared with its default branch, as the
/// daemon does. A repository the session doesn't have, or one listed twice, is
/// refused.
fn spec_diffs(
    inner: &super::Inner,
    session: &SessionId,
    home: &str,
    specs: &[RepoDiffSpec],
) -> Result<Vec<RepoDiff>, DaemonError> {
    let mine = current(inner, session);
    let mut repos: Vec<String> = Vec::new();
    for repo in std::iter::once(home).chain(mine.iter().map(|diff| diff.repo.as_str())) {
        if !repo.is_empty() && !repos.iter().any(|have| have.as_str() == repo) {
            repos.push(repo.to_owned());
        }
    }
    for (index, spec) in specs.iter().enumerate() {
        if !repos.contains(&spec.repo) {
            return Err(DaemonError::Refused {
                what: "The session's changes".to_owned(),
                reason: format!(
                    "{} isn't a repository of this session, so it has no changes to compare. Leave it out of the spec, or pick one of the session's repositories",
                    spec.repo
                ),
            });
        }
        if specs[..index].iter().any(|earlier| earlier.repo == spec.repo) {
            return Err(DaemonError::Refused {
                what: "The session's changes".to_owned(),
                reason: format!(
                    "{} is listed twice in the spec. List each repository once, with the base it should be compared with",
                    spec.repo
                ),
            });
        }
    }
    // D800: a stack layer or the Whole stack shows exactly its repositories.
    let stack = specs
        .iter()
        .any(|spec| matches!(spec.base, RepoBase::Range { .. } | RepoBase::Overlay { .. }));
    if stack {
        repos.retain(|repo| specs.iter().any(|spec| spec.repo == *repo));
    }
    Ok(repos
        .iter()
        .map(|repo| {
            let base = specs
                .iter()
                .find(|spec| spec.repo == *repo)
                .map_or(RepoBase::DefaultBranch, |spec| spec.base.clone());
            one_repo_diff(inner, session, repo, &mine, &base)
        })
        .collect())
}

/// One repository's changes against its own base. The default branch and
/// `main` are the dummy changes as they are. Other branches have the changes
/// the fixture models (acme/api against release/2.4) and none otherwise.
/// Comments stay with the changes they were made on.
fn one_repo_diff(
    inner: &super::Inner,
    session: &SessionId,
    repo: &str,
    mine: &[RepoDiff],
    base: &RepoBase,
) -> RepoDiff {
    let own = mine.iter().find(|diff| diff.repo == repo);
    let own_files = own.map_or_else(Vec::new, |diff| diff.files.clone());
    let own_comments = own.map_or_else(Vec::new, |diff| diff.comments.clone());
    let (label, files, comments) = match base {
        RepoBase::DefaultBranch => (
            own.map_or_else(|| "main".to_owned(), |diff| diff.base.clone()),
            own_files,
            own_comments,
        ),
        RepoBase::Branch(branch) => {
            let name = branch.strip_prefix("origin/").unwrap_or(branch.as_str());
            if name == "main" {
                ("main".to_owned(), own_files, own_comments)
            } else {
                (
                    name.to_owned(),
                    crate::fixture::diffs::against(repo, name).unwrap_or_default(),
                    Vec::new(),
                )
            }
        }
        RepoBase::LastReviewed => {
            let mut files = own_files;
            if let Some(seen) = inner.reviewed.get(session) {
                let before = seen.iter().find(|old| old.repo == repo);
                files.retain(|file| {
                    before.is_none_or(|old| !old.files.iter().any(|same| same == file))
                });
            }
            ("last reviewed".to_owned(), files, own_comments)
        }
        RepoBase::Rev(rev) => (rev.clone(), Vec::new(), Vec::new()),
        RepoBase::ForkPoint => ("since fork".to_owned(), Vec::new(), Vec::new()),
        RepoBase::Uncommitted => ("uncommitted changes".to_owned(), Vec::new(), Vec::new()),
        // A range has the files a test set for it, none otherwise; an
        // overlay is the changes on disk.
        RepoBase::Range { base, head } => {
            let range = format!("{base}..{head}");
            let files = inner
                .ranges
                .get(&(session.clone(), repo.to_owned(), range.clone()))
                .cloned()
                .unwrap_or_default();
            (range, files, Vec::new())
        }
        RepoBase::Overlay { base, .. } => (
            format!("{base} and uncommitted changes"),
            own_files,
            own_comments,
        ),
        // The fixture's pull requests merge into main, so their base is the
        // changes as they are.
        RepoBase::PullRequestBase => (
            "the pull request's base".to_owned(),
            own_files,
            own_comments,
        ),
    };
    RepoDiff {
        session: session.clone(),
        repo: repo.to_owned(),
        base: label,
        files,
        comments,
    }
}

/// The branches one of the session's repositories can be compared with, the
/// default branch first (VIEWS C3). A repository the session doesn't have is
/// refused, as a spec's repositories are.
pub(super) fn bases(daemon: &FakeDaemon, session: &SessionId, repo: &str) -> Reply<RepoBases> {
    let session = session.clone();
    let repo = repo.to_owned();
    daemon.reply(
        Some(Operation::Bases),
        format!("bases {session} {repo}"),
        move |inner| {
            let home = inner.session_mut(&session)?.repo.clone();
            let own = (!home.is_empty() && home == repo)
                || current(inner, &session).iter().any(|diff| diff.repo == repo);
            if !own {
                return Err(DaemonError::Refused {
                    what: format!("The branches of {repo}"),
                    reason: format!(
                        "{repo} isn't a repository of this session, so it has no branches to compare against. Pick one of the session's repositories"
                    ),
                });
            }
            Ok(crate::fixture::diffs::bases(&repo))
        },
    )
}

/// The first repository of `session` the test said has no default branch
/// that `base` would compare with its default branch.
fn missing_default(inner: &super::Inner, session: &SessionId, base: &DiffBase) -> Option<String> {
    inner
        .no_default
        .iter()
        .filter(|(held, _)| held == session)
        .map(|(_, repo)| repo)
        .find(|repo| match base {
            DiffBase::UncommittedOnly | DiffBase::OtherSession(_) | DiffBase::PullRequestBase => false,
            DiffBase::Spec(specs) => specs
                .iter()
                .find(|spec| spec.repo == **repo)
                .is_none_or(|spec| spec.base == RepoBase::DefaultBranch),
            _ => true,
        })
        .cloned()
}

/// Finds `repo`'s default branch: the fake's remotes all call it main.
pub(super) fn find_default_branch(daemon: &FakeDaemon, session: &SessionId, repo: &str) -> Reply<String> {
    let session = session.clone();
    let repo = repo.to_owned();
    daemon.reply(
        Some(Operation::FindDefaultBranch),
        format!("find default branch {session} {repo}"),
        move |inner| {
            inner.session_mut(&session)?;
            inner.no_default.retain(|(held, named)| !(*held == session && *named == repo));
            Ok("main".to_owned())
        },
    )
}

pub(super) fn mark_reviewed(daemon: &FakeDaemon, session: &SessionId) -> Reply<()> {
    let session = session.clone();
    daemon.reply(
        Some(Operation::MarkReviewed),
        format!("mark reviewed {session}"),
        move |inner| {
            inner.session_mut(&session)?;
            let now = current(inner, &session);
            inner.reviewed.insert(session, now);
            Ok(())
        },
    )
}

pub(super) fn looks(daemon: &FakeDaemon, session: &SessionId) -> Reply<Vec<crate::model::LookedFile>> {
    let session = session.clone();
    daemon.reply(
        Some(Operation::Looks),
        format!("looks {session}"),
        move |inner| {
            inner.session_mut(&session)?;
            Ok(inner.looks.get(&session).cloned().unwrap_or_default())
        },
    )
}

pub(super) fn save_looks(
    daemon: &FakeDaemon,
    session: &SessionId,
    looks: Vec<crate::model::LookedFile>,
) -> Reply<()> {
    let session = session.clone();
    daemon.reply(
        Some(Operation::SaveLooks),
        format!("save looks {session}"),
        move |inner| {
            inner.session_mut(&session)?;
            if looks.is_empty() {
                inner.looks.remove(&session);
            } else {
                inner.looks.insert(session, looks);
            }
            Ok(())
        },
    )
}

/// What pushing each repository of the session's changes would do: one
/// dirty file, on `humata/<session>`, with the file totals of its changes.
pub(super) fn plan_push(daemon: &FakeDaemon, session: &SessionId) -> Reply<PushPlan> {
    let session = session.clone();
    daemon.reply(
        Some(Operation::PlanPush),
        format!("plan push {session}"),
        move |inner| {
            inner.session_mut(&session)?;
            let repos = current(inner, &session)
                .into_iter()
                .map(|diff| PushRepo {
                    branch: format!("humata/{session}"),
                    dirty_files: 1,
                    unpushed_commits: 0,
                    added: diff.files.iter().map(|file| file.added).sum(),
                    removed: diff.files.iter().map(|file| file.removed).sum(),
                    open_pull_request: inner
                        .pull_requests
                        .get(&(session.clone(), diff.repo.clone()))
                        .copied(),
                    repo: diff.repo,
                })
                .collect();
            Ok(PushPlan {
                token: inner.id("push-plan"),
                repos,
            })
        },
    )
}

/// Records each choice. A choice that opens a pull request gets the next
/// number; one that doesn't returns the repository's existing one, if any.
/// The fake's existing pull requests target main, so one asked for another
/// base is left as it is with a note saying so, as the daemon does (VIEWS P5).
pub(super) fn push(
    daemon: &FakeDaemon,
    session: &SessionId,
    choices: Vec<PushChoice>,
) -> Reply<Vec<crate::model::PushedPullRequest>> {
    let session = session.clone();
    daemon.reply(
        Some(Operation::Push),
        format!("push {session}"),
        move |inner| {
            inner.session_mut(&session)?;
            let mut pull_requests = Vec::new();
            for choice in choices {
                let key = (session.clone(), choice.repo.clone());
                let number = if choice.open_pull_request {
                    let number = inner.next_pull_request;
                    inner.next_pull_request += 1;
                    inner.pull_requests.insert(key, number);
                    Some(number)
                } else {
                    inner.pull_requests.get(&key).copied()
                };
                if let Some(number) = number {
                    let wanted = choice.base_branch.as_str();
                    let note = if !choice.open_pull_request && !wanted.is_empty() && wanted != "main" {
                        format!(
                            "PR #{number} targets main, so it was not retargeted to {wanted}; change its base on GitHub if {wanted} is the right one"
                        )
                    } else {
                        String::new()
                    };
                    pull_requests.push(crate::model::PushedPullRequest {
                        repo: choice.repo.clone(),
                        number,
                        note,
                    });
                }
                inner.pushes.push((session.clone(), choice));
            }
            Ok(pull_requests)
        },
    )
}

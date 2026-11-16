use super::FakeDaemon;
use crate::daemon::{DaemonError, Operation, Reply};
use crate::model::{DiffBase, PushChoice, PushPlan, PushRepo, RepoDiff, SessionId};

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
            let mut diffs = current(inner, &session);
            if base == DiffBase::SinceLastReviewed
                && let Some(seen) = inner.reviewed.get(&session)
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
pub(super) fn push(
    daemon: &FakeDaemon,
    session: &SessionId,
    choices: Vec<PushChoice>,
) -> Reply<Option<u32>> {
    let session = session.clone();
    daemon.reply(
        Some(Operation::Push),
        format!("push {session}"),
        move |inner| {
            inner.session_mut(&session)?;
            let mut pull_request = None;
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
                pull_request = number.or(pull_request);
                inner.pushes.push((session.clone(), choice));
            }
            Ok(pull_request)
        },
    )
}

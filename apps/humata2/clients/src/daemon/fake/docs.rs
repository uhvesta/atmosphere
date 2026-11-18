//! Document versions on the fake daemon (DV-2 to DV-8): versions of each
//! Desk file kept in memory the way the daemon keeps commits. The file's
//! current text is the workspace's `file_contents` for the session with no
//! repository.

use std::collections::HashMap;

use super::{FakeDaemon, Inner};
use crate::daemon::{DaemonError, Event, Operation as Call, Reply};
use crate::desk;
use crate::docs::{DocVersion, DocVersions, Restored, VersionBy, VersionDiff, VersionText};
use crate::model::{DiffLine, LineKind, SessionId};

const FIRST_AT_MS: i64 = 1_700_000_000_000;
const MINUTE_MS: i64 = 60_000;

struct Stored {
    id: String,
    by: VersionBy,
    who: String,
    message: String,
    text: String,
    added: u32,
    removed: u32,
    at_ms: i64,
}

/// Every version the fake holds, oldest first per session and file.
#[derive(Default)]
pub(super) struct Store {
    versions: HashMap<(SessionId, String), Vec<Stored>>,
    commits: i64,
}

fn call(operation: desk::Operation) -> Option<Call> {
    Some(Call::Desk(operation))
}

fn refused(what: &str, reason: &str) -> DaemonError {
    DaemonError::Refused {
        what: what.to_owned(),
        reason: reason.to_owned(),
    }
}

/// The session must be a Desk session whose folder keeps versions.
fn require(inner: &Inner, session: &SessionId) -> Result<(), DaemonError> {
    match inner.workspace.desk.sessions.get(session) {
        None => Err(DaemonError::NotFound {
            what: format!("Desk session {session}"),
        }),
        Some(record) if !record.has_versions => Err(refused(
            "Reading versions",
            "this folder was made before versions existed. Run Migrate Desk folders from the Inbox, then try again",
        )),
        Some(_) => Ok(()),
    }
}

fn current<'a>(inner: &'a Inner, session: &SessionId, path: &str) -> Option<&'a str> {
    inner
        .workspace
        .file_contents
        .get(&(session.clone(), String::new(), path.to_owned()))
        .map(String::as_str)
}

fn latest<'a>(inner: &'a Inner, session: &SessionId, path: &str) -> Option<&'a str> {
    inner
        .docs
        .versions
        .get(&(session.clone(), path.to_owned()))
        .and_then(|versions| versions.last())
        .map(|stored| stored.text.as_str())
}

/// The files whose current text differs from their newest version, or that
/// have no version yet (DV-5).
fn unsaved_paths(inner: &Inner, session: &SessionId) -> Vec<String> {
    let mut paths = Vec::new();
    for ((owner, repo, path), text) in &inner.workspace.file_contents {
        if owner == session && repo.is_empty() && latest(inner, session, path) != Some(text.as_str())
        {
            paths.push(path.clone());
        }
    }
    paths
}

/// Lines added and removed going from `old` to `new`.
fn line_delta(old: &str, new: &str) -> (u32, u32) {
    let mut balance: HashMap<&str, i64> = HashMap::new();
    for line in old.lines() {
        *balance.entry(line).or_default() -= 1;
    }
    for line in new.lines() {
        *balance.entry(line).or_default() += 1;
    }
    let (mut added, mut removed) = (0_u32, 0_u32);
    for change in balance.values() {
        if *change > 0 {
            added = added.saturating_add(u32::try_from(*change).unwrap_or(u32::MAX));
        } else {
            removed = removed.saturating_add(u32::try_from(-*change).unwrap_or(u32::MAX));
        }
    }
    (added, removed)
}

fn shown(stored: &Stored, number: usize) -> DocVersion {
    DocVersion {
        id: stored.id.clone(),
        number: u32::try_from(number).unwrap_or(u32::MAX),
        at_ms: stored.at_ms,
        by: stored.by,
        who: stored.who.clone(),
        message: stored.message.clone(),
        added: stored.added,
        removed: stored.removed,
        has_comments: false,
    }
}

/// Adds a version holding `text` and moves the session's `doc_head`.
fn commit(
    inner: &mut Inner,
    session: &SessionId,
    path: &str,
    text: &str,
    by: VersionBy,
    message: &str,
) -> DocVersion {
    let who = match by {
        VersionBy::You => "You".to_owned(),
        VersionBy::Agent => inner
            .workspace
            .session(session)
            .map_or_else(|| "agent".to_owned(), |found| found.agent.name().to_owned()),
    };
    let (added, removed) = line_delta(latest(inner, session, path).unwrap_or(""), text);
    inner.docs.commits += 1;
    let n = inner.docs.commits;
    let stored = Stored {
        id: format!("c{n:04}"),
        by,
        who,
        message: message.to_owned(),
        text: text.to_owned(),
        added,
        removed,
        at_ms: FIRST_AT_MS + n * MINUTE_MS,
    };
    let list = inner
        .docs
        .versions
        .entry((session.clone(), path.to_owned()))
        .or_default();
    let version = shown(&stored, list.len() + 1);
    list.push(stored);
    if let Some(record) = inner.workspace.desk.sessions.get_mut(session) {
        record.doc_head = version.id.clone();
        record.version_count += 1;
        record.holds_files = true;
    }
    version
}

/// Saves every unsaved file as one version each; the last one made.
fn save_changes(inner: &mut Inner, session: &SessionId, message: &str) -> Option<DocVersion> {
    let mut last = None;
    for path in unsaved_paths(inner, session) {
        let Some(text) = current(inner, session, &path).map(str::to_owned) else {
            continue;
        };
        last = Some(commit(inner, session, &path, &text, VersionBy::You, message));
    }
    last
}

pub(super) fn list_versions(
    daemon: &FakeDaemon,
    session: &SessionId,
    path: &str,
) -> Reply<DocVersions> {
    let (session, path) = (session.clone(), path.to_owned());
    daemon.reply(
        call(desk::Operation::ListVersions),
        format!("desk versions {session} {path}"),
        move |inner| {
            require(inner, &session)?;
            let versions = inner
                .docs
                .versions
                .get(&(session.clone(), path.clone()))
                .map(|stored| {
                    stored
                        .iter()
                        .enumerate()
                        .rev()
                        .map(|(index, found)| shown(found, index + 1))
                        .collect()
                })
                .unwrap_or_default();
            let unsaved = !unsaved_paths(inner, &session).is_empty();
            Ok(DocVersions { versions, unsaved })
        },
    )
}

pub(super) fn read_version(
    daemon: &FakeDaemon,
    session: &SessionId,
    path: &str,
    id: &str,
) -> Reply<VersionText> {
    let (session, path, id) = (session.clone(), path.to_owned(), id.to_owned());
    daemon.reply(
        call(desk::Operation::ReadVersion),
        format!("desk read version {id} of {path} in {session}"),
        move |inner| {
            require(inner, &session)?;
            inner
                .docs
                .versions
                .get(&(session.clone(), path.clone()))
                .and_then(|stored| {
                    stored
                        .iter()
                        .enumerate()
                        .find(|(_, found)| found.id == id)
                        .map(|(index, found)| VersionText {
                            version: shown(found, index + 1),
                            text: found.text.clone(),
                            comments: None,
                        })
                })
                .ok_or_else(|| DaemonError::NotFound {
                    what: format!("Version {id} of {path}"),
                })
        },
    )
}

pub(super) fn diff_version(
    daemon: &FakeDaemon,
    session: &SessionId,
    path: &str,
    id: &str,
) -> Reply<VersionDiff> {
    let (session, path, id) = (session.clone(), path.to_owned(), id.to_owned());
    daemon.reply(
        call(desk::Operation::DiffVersion),
        format!("desk diff version {id} of {path} in {session}"),
        move |inner| {
            require(inner, &session)?;
            let old = inner
                .docs
                .versions
                .get(&(session.clone(), path.clone()))
                .and_then(|stored| stored.iter().find(|found| found.id == id))
                .map(|found| found.text.clone())
                .ok_or_else(|| DaemonError::NotFound {
                    what: format!("Version {id} of {path}"),
                })?;
            let lines = hunks(line_diff(&old, current(inner, &session, &path).unwrap_or("")));
            let count = |kind: LineKind| {
                u32::try_from(lines.iter().filter(|line| line.kind == kind).count()).unwrap_or(u32::MAX)
            };
            Ok(VersionDiff {
                added: count(LineKind::Added),
                removed: count(LineKind::Removed),
                lines,
            })
        },
    )
}

pub(super) fn save_version(
    daemon: &FakeDaemon,
    session: &SessionId,
    message: &str,
) -> Reply<DocVersion> {
    let (session, message) = (session.clone(), message.trim().to_owned());
    daemon.reply(
        call(desk::Operation::SaveVersion),
        format!("desk save version in {session}"),
        move |inner| {
            require(inner, &session)?;
            let message = if message.is_empty() {
                "Saved by you"
            } else {
                message.as_str()
            };
            let saved = save_changes(inner, &session, message).ok_or_else(|| {
                refused(
                    "Saving a version",
                    "nothing has changed since the newest version. Change the document, then save",
                )
            })?;
            inner.emit(Event::Resync);
            Ok(saved)
        },
    )
}

pub(super) fn restore_version(
    daemon: &FakeDaemon,
    session: &SessionId,
    path: &str,
    id: &str,
) -> Reply<Restored> {
    let (session, path, id) = (session.clone(), path.to_owned(), id.to_owned());
    daemon.reply(
        call(desk::Operation::RestoreVersion),
        format!("desk restore version {id} of {path} in {session}"),
        move |inner| {
            require(inner, &session)?;
            let found = inner
                .docs
                .versions
                .get(&(session.clone(), path.clone()))
                .and_then(|stored| {
                    stored
                        .iter()
                        .enumerate()
                        .find(|(_, found)| found.id == id)
                        .map(|(index, found)| (index + 1, found.text.clone()))
                });
            let Some((number, text)) = found else {
                return Err(DaemonError::NotFound {
                    what: format!("Version {id} of {path}"),
                });
            };
            if current(inner, &session, &path) == Some(text.as_str()) {
                return Err(refused(
                    "Restoring a version",
                    "the document already has that text. Pick a different version",
                ));
            }
            let saved_first = save_changes(
                inner,
                &session,
                &format!("Saved before restoring v{number}"),
            );
            inner
                .workspace
                .file_contents
                .insert((session.clone(), String::new(), path.clone()), text.clone());
            let restored = commit(
                inner,
                &session,
                &path,
                &text,
                VersionBy::You,
                &format!("Restored v{number}"),
            );
            inner.emit(Event::Resync);
            Ok(Restored {
                saved_first,
                restored,
            })
        },
    )
}

impl FakeDaemon {
    /// Stands in for a change to a Desk file that no version holds yet: the
    /// person or the agent edited it without saving. `path` is relative to
    /// the session's folder.
    pub fn edit_document(&self, session: &SessionId, path: &str, text: impl Into<String>) {
        let mut inner = self.inner();
        if let Some(record) = inner.workspace.desk.sessions.get_mut(session) {
            record.holds_files = true;
        }
        inner.workspace.file_contents.insert(
            (session.clone(), String::new(), path.to_owned()),
            text.into(),
        );
    }

    /// Stands in for the session's agent writing `text` to a Desk file and
    /// running `git commit`: a version by the agent is added and the
    /// session's `doc_head` moves, as the daemon's HEAD watcher reports it
    /// (DV-5, DV-12). The folder starts keeping versions if it didn't.
    pub fn commit_as_agent(
        &self,
        session: &SessionId,
        path: &str,
        text: impl Into<String>,
        message: impl Into<String>,
    ) {
        let (text, message) = (text.into(), message.into());
        let mut inner = self.inner();
        if let Some(record) = inner.workspace.desk.sessions.get_mut(session) {
            record.has_versions = true;
        }
        inner.workspace.file_contents.insert(
            (session.clone(), String::new(), path.to_owned()),
            text.clone(),
        );
        commit(&mut inner, session, path, &text, VersionBy::Agent, &message);
        inner.emit(Event::Resync);
    }
}

/// Three lines of context around each change, each run led by a hunk line.
fn hunks(lines: Vec<DiffLine>) -> Vec<DiffLine> {
    const CONTEXT: usize = 3;
    let mut keep = vec![false; lines.len()];
    for (at, line) in lines.iter().enumerate() {
        if line.kind != LineKind::Context {
            for slot in keep
                .iter_mut()
                .take((at + CONTEXT + 1).min(lines.len()))
                .skip(at.saturating_sub(CONTEXT))
            {
                *slot = true;
            }
        }
    }
    let mut out = Vec::new();
    let mut open = false;
    for (line, keep) in lines.into_iter().zip(keep) {
        if !keep {
            open = false;
            continue;
        }
        if !open {
            out.push(DiffLine {
                kind: LineKind::Hunk,
                old: None,
                new: None,
                text: format!("@@ -{} +{} @@", line.old.unwrap_or(0), line.new.unwrap_or(0)),
            });
            open = true;
        }
        out.push(line);
    }
    out
}

/// The lines of `new` against `old`: unchanged, removed and added, in order,
/// numbered on each side. Compares whole lines.
fn line_diff(old: &str, new: &str) -> Vec<DiffLine> {
    // Beyond this many cells the middle is shown as all removed, all added.
    const LIMIT: usize = 4_000_000;
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (mid_a, mid_b) = (&a[prefix..a.len() - suffix], &b[prefix..b.len() - suffix]);

    let mut kinds: Vec<(LineKind, &str)> = a[..prefix].iter().map(|line| (LineKind::Context, *line)).collect();
    let (n, m) = (mid_a.len(), mid_b.len());
    if n * m > LIMIT {
        kinds.extend(mid_a.iter().map(|line| (LineKind::Removed, *line)));
        kinds.extend(mid_b.iter().map(|line| (LineKind::Added, *line)));
    } else {
        // table[i][j]: the longest common run of mid_a[i..] and mid_b[j..].
        let width = m + 1;
        let mut table = vec![0_u32; (n + 1) * width];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                table[i * width + j] = if mid_a[i] == mid_b[j] {
                    table[(i + 1) * width + j + 1] + 1
                } else {
                    table[(i + 1) * width + j].max(table[i * width + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n && j < m {
            if mid_a[i] == mid_b[j] {
                kinds.push((LineKind::Context, mid_a[i]));
                i += 1;
                j += 1;
            } else if table[(i + 1) * width + j] >= table[i * width + j + 1] {
                kinds.push((LineKind::Removed, mid_a[i]));
                i += 1;
            } else {
                kinds.push((LineKind::Added, mid_b[j]));
                j += 1;
            }
        }
        kinds.extend(mid_a[i..].iter().map(|line| (LineKind::Removed, *line)));
        kinds.extend(mid_b[j..].iter().map(|line| (LineKind::Added, *line)));
    }
    kinds.extend(a[a.len() - suffix..].iter().map(|line| (LineKind::Context, *line)));

    let (mut old_at, mut new_at) = (0_u32, 0_u32);
    kinds
        .into_iter()
        .map(|(kind, line)| {
            let old_line = (kind != LineKind::Added).then(|| {
                old_at += 1;
                old_at
            });
            let new_line = (kind != LineKind::Removed).then(|| {
                new_at += 1;
                new_at
            });
            DiffLine {
                kind,
                old: old_line,
                new: new_line,
                text: line.to_owned(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(lines: &[DiffLine]) -> Vec<(LineKind, &str)> {
        lines.iter().map(|line| (line.kind, line.text.as_str())).collect()
    }

    #[test]
    fn a_line_removed_and_a_line_added_show_with_their_numbers() {
        let lines = line_diff("a\nb\nc\n", "a\nc\nd\n");
        assert_eq!(
            kinds(&lines),
            vec![
                (LineKind::Context, "a"),
                (LineKind::Removed, "b"),
                (LineKind::Context, "c"),
                (LineKind::Added, "d"),
            ]
        );
        assert_eq!((lines[1].old, lines[1].new), (Some(2), None));
        assert_eq!((lines[2].old, lines[2].new), (Some(3), Some(2)));
        assert_eq!((lines[3].old, lines[3].new), (None, Some(3)));
    }

    #[test]
    fn the_same_text_has_no_changed_line_and_so_no_hunk() {
        let lines = line_diff("a\nb\n", "a\nb\n");
        assert!(lines.iter().all(|line| line.kind == LineKind::Context));
        assert!(hunks(lines).is_empty());
    }

    #[test]
    fn unchanged_lines_far_from_a_change_are_left_out() {
        let old: String = (1..=20).map(|n| format!("{n}\n")).collect();
        let new = old.replace("10\n", "ten\n");
        let shown = hunks(line_diff(&old, &new));
        // A header, the two changed lines and three of context each side.
        assert_eq!(shown.len(), 1 + 2 + 6);
        assert_eq!(shown[0].kind, LineKind::Hunk);
    }
}

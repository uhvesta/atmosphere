//! Before a machine's copy of a session restored here from the mirror is
//! released, the host compares it with what it restored (D224): the copy
//! goes only when the machine has nothing more. Anything the host can't
//! learn counts as more, so a copy is never released on a guess.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

use humata2_grpc::humata::v1 as pb;
use humata2_mirror::{TurnMark, turns_in};
use humata2_store::SessionRepo;

/// A turn by what identifies it: its place, the first line of its prompt and
/// the time it was written at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Turn {
    pub(super) index: u32,
    pub(super) first_line: String,
    pub(super) at: Option<String>,
}

impl Turn {
    fn of(mark: &TurnMark) -> Self {
        Self {
            index: mark.index,
            first_line: mark.first_line.clone(),
            at: mark.at.clone(),
        }
    }
}

/// One repository as a restore here cloned it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RestoredRepo {
    pub(super) owner: String,
    pub(super) name: String,
    /// The clone here.
    pub(super) path: PathBuf,
    /// Where the clone stood once cloned; empty when it couldn't be read.
    pub(super) sha: String,
}

/// What a restore here from the mirror brought: how many turns the
/// conversation had and its last one, and each repository's clone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Restored {
    pub(super) turns: u32,
    pub(super) last_turn: Option<Turn>,
    pub(super) repos: Vec<RestoredRepo>,
}

impl Restored {
    /// What a restore wrote: `lines`, the conversation as the mirror kept
    /// it, and the clones of `repos` here. Blocking: it runs git.
    pub(super) fn of(lines: &[u8], repos: &[SessionRepo]) -> Self {
        let marks = turns_in(lines);
        Self {
            turns: u32::try_from(marks.len()).unwrap_or(u32::MAX),
            last_turn: marks.last().map(Turn::of),
            repos: repos
                .iter()
                .map(|repo| RestoredRepo {
                    owner: repo.owner.clone(),
                    name: repo.name.clone(),
                    path: repo.path.clone(),
                    sha: head_of(&repo.path).unwrap_or_default(),
                })
                .collect(),
        }
    }
}

/// What the machine's copy holds, as far as the host could learn it: the
/// turns of its conversation, and its own archive's summary.
#[derive(Debug, Default)]
pub(super) struct Theirs {
    pub(super) turns: Option<Vec<TurnMark>>,
    pub(super) summary: Option<pb::ArchiveSummary>,
}

/// What a machine's copy has beyond the restore here, or couldn't be told.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Extra {
    /// The restore here wasn't recorded (a journal from before D224).
    Unrecorded,
    /// The machine's conversation couldn't be read.
    TurnsUnknown,
    /// The machine's conversation differs from the restored one up to its
    /// last turn.
    TurnsDiffer,
    /// Turns after the last one restored here.
    LaterTurns(usize),
    /// The machine's archive summary couldn't be read.
    NoSummary,
    /// Commits of `repo` never pushed.
    Unpushed { repo: String, commits: usize },
    /// Files `repo`'s clone changed.
    Changed { repo: String, files: usize },
    /// The machine pushed `repo` past what was restored here.
    PushedPast { repo: String },
    /// Nothing is known of `repo` on one side.
    RepoUnknown { repo: String },
}

impl fmt::Display for Extra {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unrecorded => write!(f, "what was restored here wasn't recorded"),
            Self::TurnsUnknown => write!(f, "its conversation couldn't be read"),
            Self::TurnsDiffer => write!(f, "its conversation differs from the one restored here"),
            Self::LaterTurns(n) => write!(f, "{n} later turn(s)"),
            Self::NoSummary => write!(f, "its archive summary couldn't be read"),
            Self::Unpushed { repo, commits } => write!(f, "{repo} has {commits} unpushed commit(s)"),
            Self::Changed { repo, files } => write!(f, "{repo} has {files} changed file(s)"),
            Self::PushedPast { repo } => write!(f, "{repo} was pushed past what was restored here"),
            Self::RepoUnknown { repo } => write!(f, "nothing is known of {repo} on one side"),
        }
    }
}

/// Everything a machine's copy has more of; never empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct More(pub(super) Vec<Extra>);

impl fmt::Display for More {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (n, extra) in self.0.iter().enumerate() {
            if n > 0 {
                write!(f, "; ")?;
            }
            write!(f, "{extra}")?;
        }
        Ok(())
    }
}

/// What the machine's copy has beyond `restored`; `None` when nothing.
/// `holds(repo, sha)` says whether `sha` is the restored clone's commit or
/// an ancestor of it.
pub(super) fn compare(
    restored: Option<&Restored>,
    theirs: &Theirs,
    holds: impl Fn(&RestoredRepo, &str) -> bool,
) -> Option<More> {
    let Some(restored) = restored else {
        return Some(More(vec![Extra::Unrecorded]));
    };
    let mut extra = Vec::new();
    match &theirs.turns {
        None => extra.push(Extra::TurnsUnknown),
        Some(turns) => {
            let count = usize::try_from(restored.turns).unwrap_or(usize::MAX);
            let last = count.checked_sub(1).and_then(|at| turns.get(at)).map(Turn::of);
            if turns.len() < count || last != restored.last_turn {
                extra.push(Extra::TurnsDiffer);
            } else if turns.len() > count {
                extra.push(Extra::LaterTurns(turns.len() - count));
            }
        }
    }
    match &theirs.summary {
        None => extra.push(Extra::NoSummary),
        Some(summary) => {
            for repo in &restored.repos {
                extra.extend(repo_extra(repo, summary, &holds));
            }
            for kept in &summary.repos {
                let known = kept.repo.as_ref().is_some_and(|named| {
                    restored
                        .repos
                        .iter()
                        .any(|repo| repo.owner == named.owner && repo.name == named.name)
                });
                if !known {
                    let repo = kept
                        .repo
                        .as_ref()
                        .map(|named| format!("{}/{}", named.owner, named.name))
                        .unwrap_or_default();
                    extra.push(Extra::RepoUnknown { repo });
                }
            }
        }
    }
    (!extra.is_empty()).then_some(More(extra))
}

/// What the machine's summary says `repo` has beyond its clone here.
fn repo_extra(
    repo: &RestoredRepo,
    summary: &pb::ArchiveSummary,
    holds: &impl Fn(&RestoredRepo, &str) -> bool,
) -> Vec<Extra> {
    let named = format!("{}/{}", repo.owner, repo.name);
    let Some(kept) = summary.repos.iter().find(|kept| {
        kept.repo
            .as_ref()
            .is_some_and(|other| other.owner == repo.owner && other.name == repo.name)
    }) else {
        return vec![Extra::RepoUnknown { repo: named }];
    };
    let mut extra = Vec::new();
    if !kept.unpushed_subjects.is_empty() {
        extra.push(Extra::Unpushed {
            repo: named.clone(),
            commits: kept.unpushed_subjects.len(),
        });
    }
    if !kept.files.is_empty() {
        extra.push(Extra::Changed {
            repo: named.clone(),
            files: kept.files.len(),
        });
    }
    if kept.last_pushed_sha.is_empty() || repo.sha.is_empty() {
        extra.push(Extra::RepoUnknown { repo: named });
    } else if !holds(repo, &kept.last_pushed_sha) {
        extra.push(Extra::PushedPast { repo: named });
    }
    extra
}

/// Whether `sha` is `repo`'s restored commit or an ancestor of it, in the
/// clone here. A sha the clone doesn't know isn't. Blocking: it runs git.
pub(super) fn holds(repo: &RestoredRepo, sha: &str) -> bool {
    if !is_sha(sha) || !is_sha(&repo.sha) {
        return false;
    }
    git(&repo.path)
        .args(["merge-base", "--is-ancestor", sha, &repo.sha])
        .output()
        .is_ok_and(|out| out.status.success())
}

/// The commit `clone` stands at. Blocking: it runs git.
fn head_of(clone: &Path) -> Option<String> {
    let out = git(clone).args(["rev-parse", "HEAD"]).output().ok()?;
    let head = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (out.status.success() && is_sha(&head)).then_some(head)
}

fn is_sha(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn git(clone: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(clone)
        .env("GIT_TERMINAL_PROMPT", "0");
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINES: &str = concat!(
        r#"{"type":"user","message":{"content":"Warm the cache"},"timestamp":"2026-11-16T10:00:00Z"}"#,
        "\n",
        r#"{"type":"assistant","message":{"content":"Done"}}"#,
        "\n",
        r#"{"type":"user","message":{"content":"Now the tests"},"timestamp":"2026-11-16T10:05:00Z"}"#,
        "\n",
    );
    const LATER: &str = concat!(
        r#"{"type":"user","message":{"content":"And the docs"},"timestamp":"2026-11-16T10:09:00Z"}"#,
        "\n",
    );
    const SHA: &str = "1111111111111111111111111111111111111111";

    fn restored(lines: &str) -> Restored {
        let marks = turns_in(lines.as_bytes());
        Restored {
            turns: u32::try_from(marks.len()).unwrap(),
            last_turn: marks.last().map(Turn::of),
            repos: vec![RestoredRepo {
                owner: "acme".to_owned(),
                name: "api".to_owned(),
                path: PathBuf::from("/work/se_1/api"),
                sha: SHA.to_owned(),
            }],
        }
    }

    fn kept(sha: &str) -> pb::archive_summary::Repo {
        pb::archive_summary::Repo {
            repo: Some(pb::RepoRef {
                owner: "acme".to_owned(),
                name: "api".to_owned(),
            }),
            branch: "cache-warmup".to_owned(),
            last_pushed_sha: sha.to_owned(),
            ..Default::default()
        }
    }

    fn theirs(lines: &str, repo: pb::archive_summary::Repo) -> Theirs {
        Theirs {
            turns: Some(turns_in(lines.as_bytes())),
            summary: Some(pb::ArchiveSummary {
                repos: vec![repo],
                ..Default::default()
            }),
        }
    }

    /// The restored clone holds exactly `SHA` and nothing else.
    fn at_sha(_: &RestoredRepo, sha: &str) -> bool {
        sha == SHA
    }

    fn extras(restored: Option<&Restored>, theirs: &Theirs) -> Vec<Extra> {
        compare(restored, theirs, at_sha).map(|more| more.0).unwrap_or_default()
    }

    #[test]
    fn a_copy_with_the_same_turns_and_pushed_work_has_nothing_more() {
        assert_eq!(compare(Some(&restored(LINES)), &theirs(LINES, kept(SHA)), at_sha), None);
    }

    #[test]
    fn a_copy_with_turns_after_the_last_restored_one_has_more() {
        let later = format!("{LINES}{LATER}");
        assert_eq!(extras(Some(&restored(LINES)), &theirs(&later, kept(SHA))), [Extra::LaterTurns(1)]);
    }

    #[test]
    fn a_copy_whose_turns_differ_at_the_last_restored_one_has_more_even_with_as_many() {
        let other = LINES.replace("Now the tests", "Now the build");
        assert_eq!(extras(Some(&restored(LINES)), &theirs(&other, kept(SHA))), [Extra::TurnsDiffer]);
    }

    #[test]
    fn a_copy_with_fewer_turns_than_were_restored_has_more() {
        let fewer: String = LINES.lines().take(1).map(|line| format!("{line}\n")).collect();
        assert_eq!(extras(Some(&restored(LINES)), &theirs(&fewer, kept(SHA))), [Extra::TurnsDiffer]);
    }

    #[test]
    fn a_conversation_with_no_turns_on_either_side_matches() {
        assert_eq!(compare(Some(&restored("")), &theirs("", kept(SHA)), at_sha), None);
    }

    #[test]
    fn changed_files_or_unpushed_commits_in_the_summary_are_more() {
        let mut repo = kept(SHA);
        repo.files.push(pb::FileChange {
            path: "wip.txt".to_owned(),
            added: 1,
            removed: 0,
        });
        repo.unpushed_subjects.push("Warm on start".to_owned());
        let found = extras(Some(&restored(LINES)), &theirs(LINES, repo));
        assert!(found.contains(&Extra::Changed { repo: "acme/api".to_owned(), files: 1 }), "{found:?}");
        assert!(found.contains(&Extra::Unpushed { repo: "acme/api".to_owned(), commits: 1 }), "{found:?}");
    }

    #[test]
    fn a_pushed_sha_the_restored_clone_doesnt_hold_is_more() {
        let other = "2222222222222222222222222222222222222222";
        assert_eq!(
            extras(Some(&restored(LINES)), &theirs(LINES, kept(other))),
            [Extra::PushedPast { repo: "acme/api".to_owned() }]
        );
    }

    #[test]
    fn whatever_cant_be_learned_counts_as_more() {
        let restored = restored(LINES);
        assert_eq!(extras(None, &theirs(LINES, kept(SHA))), [Extra::Unrecorded]);
        let unread = Theirs {
            turns: None,
            summary: None,
        };
        assert_eq!(extras(Some(&restored), &unread), [Extra::TurnsUnknown, Extra::NoSummary]);
        assert_eq!(
            extras(Some(&restored), &theirs(LINES, kept(""))),
            [Extra::RepoUnknown { repo: "acme/api".to_owned() }]
        );
        let mut elsewhere = kept(SHA);
        elsewhere.repo = Some(pb::RepoRef {
            owner: "acme".to_owned(),
            name: "web".to_owned(),
        });
        let found = extras(Some(&restored), &theirs(LINES, elsewhere));
        assert!(found.contains(&Extra::RepoUnknown { repo: "acme/api".to_owned() }), "{found:?}");
        assert!(found.contains(&Extra::RepoUnknown { repo: "acme/web".to_owned() }), "{found:?}");
    }

    #[test]
    fn holds_is_true_for_the_restored_commit_and_its_ancestors_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_owned();
        let run = |args: &[&str]| {
            let out = git(&path).args(args).output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        run(&["init", "-q"]);
        let commit = |message: &str| {
            run(&[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                message,
            ]);
            head_of(&path).unwrap()
        };
        let first = commit("first");
        let second = commit("second");
        let repo = RestoredRepo {
            owner: "acme".to_owned(),
            name: "api".to_owned(),
            path: path.clone(),
            sha: first.clone(),
        };
        assert!(holds(&repo, &first));
        assert!(!holds(&repo, &second), "a later commit isn't in what was restored");
        assert!(!holds(&repo, "3333333333333333333333333333333333333333"), "an unknown sha isn't held");
        let later = RestoredRepo { sha: second, ..repo };
        assert!(holds(&later, &first));
    }
}

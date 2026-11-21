//! A9, D299: which of a pull request's commits the user reviewed. The user
//! marks commits one by one, or every commit up to the head with RV-6 Mark
//! reviewed; the agent only reads the marks. Marks belong to the pull
//! request, so every review of it shares them, and they outlive the
//! session. A force push carries a mark to the commit with the same
//! `git patch-id --stable`; a changed patch is unreviewed again. Every
//! mark, unmark, carry and whole-PR review adds to a history that is never
//! rewritten.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use humata2_access::{Call, Capability, Target};
use humata2_daemon_ports::Caller;
use crate::pb;
use humata2_rpc_kit::{blocking, chain, failed, invalid_field, log::say, missing, timestamp_ms};
use humata2_store::{
    CommitMark, KeptAnswer, MarkedBy, PrCommitMarks, ReviewEvent, ReviewPr, Store, StoreError,
};
use prost::Message;
use tonic::{Code, Status};

use super::PullRequests;
use super::github;
use super::heads::{self, HeadsError};
use crate::ports::{actor, session_ref, store_error};

/// The lock held while pull request `pr`'s marks are read, worked out and
/// written, so two lists at once don't both add the same carry and a
/// repeated mark sees the first one's key. One per pull request; only the
/// store work holds it, git and `gh` run before.
fn marking(pr: &ReviewPr) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    let mut locks = LOCKS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    // A lock nobody holds or waits on is made again when next needed.
    locks.retain(|_, lock| Arc::strong_count(lock) > 1);
    Arc::clone(locks.entry(key_of(pr)).or_default())
}

/// What went wrong reading or marking a pull request's commits.
#[derive(Debug, thiserror::Error)]
pub(super) enum CommitsError {
    #[error("{pr} is not one of this review's pull requests")]
    NoSuchPr { pr: String },
    #[error("commit {sha} is not on {pr}'s head in this review's clone")]
    NotOnHead { pr: String, sha: String },
    #[error("this review doesn't know which branch {pr} goes into")]
    NoBase { pr: String },
    #[error("couldn't list {pr}'s commits in the review's clone")]
    Git {
        pr: String,
        #[source]
        source: HeadsError,
    },
    #[error("couldn't run `git patch-id` for the clone at {clone}")]
    PatchIdSpawn {
        clone: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("`git patch-id --stable` failed in the clone at {clone} ({status}): {stderr}")]
    PatchIdFailed {
        clone: PathBuf,
        status: String,
        stderr: String,
    },
    #[error("couldn't reach the review's clone on its machine")]
    Remote {
        #[source]
        source: super::clone_git::RemoteGitError,
    },
    #[error("the store holds a reviewer of {pr} this version of Humata can't read")]
    Reviewer {
        pr: String,
        #[source]
        source: prost::DecodeError,
    },
    #[error("the store holds an answer for {pr} this version of Humata can't read")]
    UnreadableAnswer {
        pr: String,
        #[source]
        source: prost::DecodeError,
    },
    #[error("couldn't keep or read {pr}'s reviewed commits")]
    Store {
        pr: String,
        #[source]
        source: StoreError,
    },
}

impl CommitsError {
    fn status(self, machine: &str) -> Status {
        match self {
            Self::NoSuchPr { .. } => invalid_field(
                "pr",
                pb::invalid_field::Problem::NotAllowedHere,
                chain(&self),
                "name one of the review's pull requests, as the review session lists them",
            ),
            Self::NotOnHead { .. } => invalid_field(
                "commits",
                pb::invalid_field::Problem::NotAllowedHere,
                chain(&self),
                "list the pull request's commits again and mark the shas it shows; the head may have moved since",
            ),
            Self::NoBase { .. } => failed(
                Code::FailedPrecondition,
                &self,
                "check the review's pull requests for news (Refresh) so the review learns each one's base, then try again",
                None,
            ),
            Self::Git { ref source, .. } => {
                let fix = match source {
                    HeadsError::GitSpawn { .. } => {
                        "install git on this Mac (`xcode-select --install`) and try again"
                    }
                    _ => {
                        "check that the review's clone is still there and this Mac can reach GitHub, then try again; nothing was marked"
                    }
                };
                failed(Code::FailedPrecondition, &self, fix, None)
            }
            Self::Remote { ref source } => source.status(),
            Self::PatchIdSpawn { .. } | Self::PatchIdFailed { .. } => failed(
                Code::FailedPrecondition,
                &self,
                "check that git on this Mac runs (`git patch-id -h`), then try again; nothing was marked",
                None,
            ),
            Self::UnreadableAnswer { .. } => failed(
                Code::FailedPrecondition,
                &self,
                "send the mark again with a new idempotency key; the first call's marks stand, so list the commits first to see them",
                None,
            ),
            Self::Reviewer { .. } => failed(
                Code::FailedPrecondition,
                &self,
                "update Humata on this Mac to the version that marked these commits; the marks are kept as they were",
                None,
            ),
            Self::Store { source, pr } => store_error(
                source,
                machine,
                &format!("keep or read {pr}'s reviewed commits"),
            ),
        }
    }
}

/// A commit on the clone's head.
struct OnHead {
    sha: String,
    subject: String,
    /// Empty when git gives it none, as for a merge or an empty commit,
    /// which never carry.
    patch_id: String,
}

/// A pull request's commits as the clone has them now.
struct Head {
    pr: ReviewPr,
    head: String,
    commits: Vec<OnHead>,
}

fn key_of(pr: &ReviewPr) -> String {
    format!("{}/{}#{}", pr.owner, pr.name, pr.number)
}

fn now_ms() -> i64 {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
}

/// Runs `git patch-id --stable` in the clone here, feeding it `patches`.
fn local_patch_ids(clone: &Path, patches: Vec<u8>) -> Result<std::process::Output, CommitsError> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(clone)
        .args(["patch-id", "--stable"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| CommitsError::PatchIdSpawn {
            clone: clone.to_path_buf(),
            source,
        })?;
    let writer = child.stdin.take().map(|mut stdin| {
        // A write that fails shows as git's own failure.
        std::thread::spawn(move || stdin.write_all(&patches))
    });
    let output = child
        .wait_with_output()
        .map_err(|source| CommitsError::PatchIdSpawn {
            clone: clone.to_path_buf(),
            source,
        })?;
    if let Some(writer) = writer {
        drop(writer.join());
    }
    Ok(output)
}

/// `git patch-id --stable` of every commit `log` shows, by sha.
fn patch_ids(clone: &Path, log: &str) -> Result<BTreeMap<String, String>, CommitsError> {
    let mut patches = log.as_bytes().to_vec();
    patches.push(b'\n');
    // A review on another machine has its clones there (PR-4, PR-21).
    let output = match super::clone_git::remote(clone, &["patch-id", "--stable"], Some(&patches)) {
        Some(done) => done.map_err(|source| CommitsError::Remote { source })?,
        None => local_patch_ids(clone, patches)?,
    };
    if !output.status.success() {
        return Err(CommitsError::PatchIdFailed {
            clone: clone.to_path_buf(),
            status: output.status.to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split_once(' '))
        .map(|(patch_id, sha)| (sha.trim().to_owned(), patch_id.to_owned()))
        .collect())
}

/// The commits of `pr` in its clone in `folder`, base..head, oldest first.
/// When `refs/humata/pr-<n>` exists in the clone, resolves `base..refs/humata/pr-<n>`
/// (using the parent PR's `refs/humata/pr-<parent>` when `pr` is stacked in `stack`,
/// or `origin/<base_branch>` otherwise), falling back to `HEAD` when `refs/humata/pr-<n>`
/// does not exist. Blocking.
fn head_of_in_stack(folder: &Path, pr: &ReviewPr, stack: &[ReviewPr]) -> Result<Head, CommitsError> {
    if pr.base_branch.is_empty() {
        return Err(CommitsError::NoBase { pr: key_of(pr) });
    }
    let clone = folder.join(&pr.name);
    let git_failed = |source| CommitsError::Git {
        pr: key_of(pr),
        source,
    };
    let layer_ref = heads::pr_ref_name(pr.number);
    let layer_head = heads::git(
        &clone,
        &["rev-parse", "--verify", &format!("{layer_ref}^{{commit}}")],
    )
    .ok();
    let (base, head_ref, head) = if let Some(head_sha) = layer_head {
        let parent_ref = stack
            .iter()
            .find(|other| {
                other.owner == pr.owner
                    && other.name == pr.name
                    && other.number != pr.number
                    && other.head_branch == pr.base_branch
            })
            .map(|parent| heads::pr_ref_name(parent.number))
            .filter(|parent_ref| {
                heads::git(
                    &clone,
                    &["rev-parse", "--verify", &format!("{parent_ref}^{{commit}}")],
                )
                .is_ok()
            });
        let base = match parent_ref {
            Some(parent_ref) => parent_ref,
            None => heads::base_ref_blocking(&clone, &pr.base_branch).map_err(git_failed)?,
        };
        (base, layer_ref, head_sha)
    } else {
        let base = heads::base_ref_blocking(&clone, &pr.base_branch).map_err(git_failed)?;
        let head =
            heads::git(&clone, &["rev-parse", "--verify", "HEAD^{commit}"]).map_err(git_failed)?;
        (base, "HEAD".to_owned(), head)
    };
    let range = format!("{base}..{head_ref}");
    let listed = heads::git(&clone, &["log", "--reverse", "--format=%H%x00%s", &range])
        .map_err(git_failed)?;
    let log = heads::git(
        &clone,
        &[
            "log",
            "-p",
            "--no-color",
            "--no-ext-diff",
            "--format=commit %H",
            &range,
        ],
    )
    .map_err(git_failed)?;
    let ids = patch_ids(&clone, &log)?;
    let commits = listed
        .lines()
        .filter_map(|line| line.split_once('\0'))
        .map(|(sha, subject)| OnHead {
            sha: sha.to_owned(),
            subject: subject.to_owned(),
            patch_id: ids.get(sha).cloned().unwrap_or_default(),
        })
        .collect();
    Ok(Head {
        pr: pr.clone(),
        head,
        commits,
    })
}

/// The commits of `pr` in its clone in `folder`, base..HEAD (or `refs/humata/pr-<n>`
/// when present), oldest first. Blocking.
fn head_of(folder: &Path, pr: &ReviewPr) -> Result<Head, CommitsError> {
    head_of_in_stack(folder, pr, &[])
}

/// New shas on the head whose patch was marked under another sha: the
/// marks to carry to them, oldest first. A sha with a mark of its own, even
/// an unmark, keeps it.
fn carries(head: &Head, saved: &PrCommitMarks) -> Vec<CommitMark> {
    let marked: BTreeSet<&str> = saved.marks.iter().map(|mark| mark.sha.as_str()).collect();
    head.commits
        .iter()
        .filter(|commit| !commit.patch_id.is_empty() && !marked.contains(commit.sha.as_str()))
        .filter_map(|commit| {
            let from = saved
                .marks
                .iter()
                .filter(|mark| mark.reviewed && mark.patch_id == commit.patch_id)
                .min_by(|a, b| (a.reviewed_unix_ms, &a.sha).cmp(&(b.reviewed_unix_ms, &b.sha)))?;
            let reviewed_as_sha = if from.reviewed_as_sha.is_empty() {
                from.sha.clone()
            } else {
                from.reviewed_as_sha.clone()
            };
            Some(CommitMark {
                sha: commit.sha.clone(),
                patch_id: commit.patch_id.clone(),
                reviewed_as_sha,
                ..from.clone()
            })
        })
        .collect()
}

/// The pull request's marks, with any carry the head calls for written
/// first. The caller holds `marking(&head.pr)`.
fn carried(store: &Store, head: &Head) -> Result<PrCommitMarks, StoreError> {
    let pr = &head.pr;
    let saved = store.pr_commit_marks(&pr.owner, &pr.name, pr.number)?;
    let carry = carries(head, &saved);
    if carry.is_empty() {
        return Ok(saved);
    }
    let event = ReviewEvent {
        at_unix_ms: now_ms(),
        kind: pb::commit_review_entry::Kind::Carried as i32,
        commits: carry.iter().map(|mark| mark.sha.clone()).collect(),
        head_sha: head.head.clone(),
        snapshot_n: None,
        by: None,
    };
    store.add_pr_commit_marks(&pr.owner, &pr.name, pr.number, &carry, &event)
}

/// What one mark does.
struct Marking<'a> {
    /// The head's commits to mark; all of them when `None`.
    shas: Option<&'a BTreeSet<String>>,
    reviewed: bool,
    kind: pb::commit_review_entry::Kind,
    snapshot_n: Option<u32>,
    /// The idempotency key of the call, kept with its entry.
    key: Option<&'a str>,
}

/// Marks the head's commits as `marking` says, carrying first. The caller
/// holds `marking(&head.pr)`. Blocking.
fn marked(
    store: &Store,
    head: &Head,
    marking: &Marking<'_>,
    by: &MarkedBy,
) -> Result<PrCommitMarks, CommitsError> {
    let pr = &head.pr;
    let Marking {
        shas,
        reviewed,
        kind,
        snapshot_n,
        key,
    } = *marking;
    let at_unix_ms = now_ms();
    let marks: Vec<CommitMark> = head
        .commits
        .iter()
        .filter(|commit| shas.is_none_or(|shas| shas.contains(&commit.sha)))
        .map(|commit| CommitMark {
            sha: commit.sha.clone(),
            patch_id: commit.patch_id.clone(),
            reviewed,
            reviewed_as_sha: String::new(),
            reviewed_unix_ms: at_unix_ms,
            reviewed_head_sha: head.head.clone(),
            by: Some(by.clone()),
        })
        .collect();
    let event = ReviewEvent {
        at_unix_ms,
        kind: kind as i32,
        commits: marks.iter().map(|mark| mark.sha.clone()).collect(),
        head_sha: head.head.clone(),
        snapshot_n,
        by: Some(by.clone()),
    };
    let store_failed = |source| CommitsError::Store {
        pr: key_of(pr),
        source,
    };
    carried(store, head).map_err(store_failed)?;
    match key {
        Some(key) => {
            store.add_keyed_pr_commit_marks(&pr.owner, &pr.name, pr.number, &marks, &event, key)
        }
        None => store.add_pr_commit_marks(&pr.owner, &pr.name, pr.number, &marks, &event),
    }
    .map_err(store_failed)
}

fn reviewed_by(
    pr: &ReviewPr,
    by: Option<&MarkedBy>,
) -> Result<Option<pb::ReviewedBy>, CommitsError> {
    by.map(|by| {
        let actor =
            pb::Actor::decode(by.actor.as_slice()).map_err(|source| CommitsError::Reviewer {
                pr: key_of(pr),
                source,
            })?;
        Ok(pb::ReviewedBy {
            actor: Some(actor),
            gh_login: by.gh_login.clone(),
        })
    })
    .transpose()
}

/// What the rpcs answer: the head's commits with their marks, and the
/// history. Built from the store's rows alone, so a restart answers the
/// same.
fn answer(head: &Head, saved: &PrCommitMarks) -> Result<pb::PullRequestCommits, CommitsError> {
    let pr = &head.pr;
    let marks: BTreeMap<&str, &CommitMark> = saved
        .marks
        .iter()
        .map(|mark| (mark.sha.as_str(), mark))
        .collect();
    let mut commits = Vec::with_capacity(head.commits.len());
    for commit in &head.commits {
        let mut listed = pb::PrCommit {
            sha: commit.sha.clone(),
            subject: commit.subject.clone(),
            patch_id: commit.patch_id.clone(),
            state: pb::pr_commit::State::Unreviewed as i32,
            ..pb::PrCommit::default()
        };
        if let Some(mark) = marks.get(commit.sha.as_str()).filter(|mark| mark.reviewed) {
            listed.state = if mark.reviewed_as_sha.is_empty() {
                pb::pr_commit::State::Reviewed as i32
            } else {
                pb::pr_commit::State::Carried as i32
            };
            listed.reviewed_as_sha = mark.reviewed_as_sha.clone();
            listed.reviewed_at = Some(timestamp_ms(mark.reviewed_unix_ms));
            listed.reviewed_head_sha = mark.reviewed_head_sha.clone();
            listed.by = reviewed_by(pr, mark.by.as_ref())?;
        }
        commits.push(listed);
    }
    let history = saved
        .history
        .iter()
        .map(|entry| {
            Ok::<_, CommitsError>(pb::CommitReviewEntry {
                seq: entry.seq,
                at: Some(timestamp_ms(entry.event.at_unix_ms)),
                kind: entry.event.kind,
                commits: entry.event.commits.clone(),
                head_sha: entry.event.head_sha.clone(),
                snapshot_n: entry.event.snapshot_n,
                by: reviewed_by(pr, entry.event.by.as_ref())?,
            })
        })
        .collect::<Result<_, CommitsError>>()?;
    Ok(pb::PullRequestCommits {
        pr: Some(heads::pr_ref(pr)),
        head_sha: head.head.clone(),
        commits,
        history,
    })
}

/// Who is marking: the caller, and the `gh` login now. A login GitHub won't
/// give is left empty, saying so in the log; the mark still stands.
fn marked_by(actor: &pb::Actor) -> MarkedBy {
    let gh_login = github::signed_in_login().unwrap_or_else(|error| {
        say!(
            "humatad2: couldn't read the gh login to record with a reviewed commit: {}",
            chain(&error)
        );
        String::new()
    });
    MarkedBy {
        actor: actor.encode_to_vec(),
        gh_login,
    }
}

/// As `super::mark_whole_pr_blocking`.
pub(super) fn mark_whole_blocking(
    store: &Store,
    machine: &str,
    session: &str,
    folder: &Path,
    snapshots: &[(String, u32)],
    actor: &pb::Actor,
) -> Result<Vec<(String, u32)>, Status> {
    let Some(review) = store
        .review_session(session)
        .map_err(|error| store_error(error, machine, "read the review's pull requests"))?
    else {
        return Ok(Vec::new());
    };
    let mut by = None;
    let mut done = Vec::new();
    for (key, n) in snapshots {
        let (owner, name) = key
            .rsplit_once('/')
            .map_or((None, key.as_str()), |(owner, name)| (Some(owner), name));
        let Some(pr) = review
            .prs
            .iter()
            .find(|pr| pr.name == name && owner.is_none_or(|owner| pr.owner == owner))
        else {
            continue;
        };
        let head =
            head_of_in_stack(folder, pr, &review.prs).map_err(|error| error.status(machine))?;
        let who = by.get_or_insert_with(|| marked_by(actor));
        let whole = Marking {
            shas: None,
            reviewed: true,
            kind: pb::commit_review_entry::Kind::WholePr,
            snapshot_n: Some(*n),
            key: None,
        };
        let lock = marking(pr);
        let _marking = lock.lock().unwrap_or_else(PoisonError::into_inner);
        marked(store, &head, &whole, who).map_err(|error| error.status(machine))?;
        done.push((key.clone(), *n));
    }
    Ok(done)
}

/// UpdateClone moved `pr`'s clone in `folder` to its new head: carries the
/// marks the new head calls for now, so the carry is dated when it
/// happened. A pull request without a known base has nothing to carry.
/// Blocking.
pub(super) fn carry_moved_blocking(
    store: &Store,
    folder: &Path,
    pr: &ReviewPr,
) -> Result<(), CommitsError> {
    if pr.base_branch.is_empty() {
        return Ok(());
    }
    let head = head_of(folder, pr)?;
    let lock = marking(pr);
    let _marking = lock.lock().unwrap_or_else(PoisonError::into_inner);
    carried(store, &head)
        .map(drop)
        .map_err(|source| CommitsError::Store {
            pr: key_of(pr),
            source,
        })
}

/// The answer a call with idempotency key `key` got, kept in the store.
fn kept_answer(
    store: &Store,
    pr: &ReviewPr,
    key: &str,
) -> Result<Option<KeptAnswer>, CommitsError> {
    store
        .pr_marks_answer(&pr.owner, &pr.name, pr.number, key)
        .map_err(|source| CommitsError::Store {
            pr: key_of(pr),
            source,
        })
}

fn decoded(pr: &ReviewPr, answer: &[u8]) -> Result<pb::PullRequestCommits, CommitsError> {
    pb::PullRequestCommits::decode(answer).map_err(|source| CommitsError::UnreadableAnswer {
        pr: key_of(pr),
        source,
    })
}

/// What MarkCommitsReviewed asks, once per idempotency key.
struct MarkCall {
    key: Option<String>,
    commits: Vec<String>,
    reviewed: bool,
}

/// Marks as `call` asks, or answers what the call with its key got.
/// Blocking.
fn mark_once(
    store: &Store,
    folder: &Path,
    pr: &ReviewPr,
    stack: &[ReviewPr],
    call: MarkCall,
    actor: &pb::Actor,
) -> Result<pb::PullRequestCommits, CommitsError> {
    let key = call.key.as_deref();
    // A repeat whose answer is kept needs no git.
    if let Some(key) = key
        && let Some(KeptAnswer::Kept(answer)) = kept_answer(store, pr, key)?
    {
        return decoded(pr, &answer);
    }
    let head = head_of_in_stack(folder, pr, stack)?;
    let by = marked_by(actor);
    let lock = marking(pr);
    let _marking = lock.lock().unwrap_or_else(PoisonError::into_inner);
    let kept = key
        .map(|key| kept_answer(store, pr, key))
        .transpose()?
        .flatten();
    let saved = match kept {
        Some(KeptAnswer::Kept(answer)) => return decoded(pr, &answer),
        // The call marked but its answer is gone: answer how things stand.
        Some(KeptAnswer::Lost) => carried(store, &head).map_err(|source| CommitsError::Store {
            pr: key_of(pr),
            source,
        })?,
        None => {
            let on_head: BTreeSet<&str> = head
                .commits
                .iter()
                .map(|commit| commit.sha.as_str())
                .collect();
            if let Some(sha) = call
                .commits
                .iter()
                .find(|sha| !on_head.contains(sha.as_str()))
            {
                return Err(CommitsError::NotOnHead {
                    pr: key_of(pr),
                    sha: sha.clone(),
                });
            }
            let shas: BTreeSet<String> = call.commits.iter().cloned().collect();
            let kind = if call.reviewed {
                pb::commit_review_entry::Kind::Marked
            } else {
                pb::commit_review_entry::Kind::Unmarked
            };
            let asked = Marking {
                shas: Some(&shas),
                reviewed: call.reviewed,
                kind,
                snapshot_n: None,
                key,
            };
            marked(store, &head, &asked, &by)?
        }
    };
    let answer = answer(&head, &saved)?;
    if let Some(key) = key {
        store
            .keep_pr_marks_answer(&pr.owner, &pr.name, pr.number, key, &answer.encode_to_vec())
            .map_err(|source| CommitsError::Store {
                pr: key_of(pr),
                source,
            })?;
    }
    Ok(answer)
}

impl PullRequests {
    /// The session's folder, the review's pull request `reference` names, and
    /// all PRs in the review, once the caller may make `capability`'s call on
    /// the session (read it, when `None`).
    async fn pr_named(
        &self,
        caller: &Caller,
        session: Option<&pb::SessionId>,
        reference: Option<&pb::PullRequestRef>,
        capability: Option<Capability>,
    ) -> Result<(PathBuf, ReviewPr, Vec<ReviewPr>), Status> {
        let (record, review) = self.review_records(session).await?;
        let target = session_ref(&record, &self.inner.machine);
        let call = match capability {
            Some(capability) => Call::Use {
                capability,
                target: Target::Session(target),
            },
            None => Call::ReadSession(target),
        };
        self.inner.guard.require(caller, &call)?;
        let reference = reference.ok_or_else(|| missing("pr"))?;
        let repo = reference.repo.clone().unwrap_or_default();
        let pr = review
            .prs
            .iter()
            .find(|pr| {
                pr.owner == repo.owner && pr.name == repo.name && pr.number == reference.number
            })
            .cloned()
            .ok_or_else(|| {
                let pr = format!("{}/{}#{}", repo.owner, repo.name, reference.number);
                CommitsError::NoSuchPr { pr }.status(&self.inner.machine)
            })?;
        Ok((record.folder, pr, review.prs))
    }

    /// ListPullRequestCommits: anyone who may read the review, its agent
    /// included.
    pub(super) async fn list_commits_of(
        &self,
        caller: &Caller,
        request: pb::ListPullRequestCommitsRequest,
    ) -> Result<pb::PullRequestCommits, Status> {
        let (folder, pr, stack) = self
            .pr_named(caller, request.session.as_ref(), request.pr.as_ref(), None)
            .await?;
        let (store, machine) = (self.inner.store.clone(), self.inner.machine.clone());
        blocking(move || {
            let head = head_of_in_stack(&folder, &pr, &stack)?;
            let lock = marking(&pr);
            let saved = {
                let _marking = lock.lock().unwrap_or_else(PoisonError::into_inner);
                carried(&store, &head).map_err(|source| CommitsError::Store {
                    pr: key_of(&pr),
                    source,
                })
            };
            saved.and_then(|saved| answer(&head, &saved))
        })
        .await?
        .map_err(|error| error.status(&machine))
    }

    /// MarkCommitsReviewed: the user only, through the guard as every user
    /// review state call; the agent is refused and nothing is kept. A
    /// repeat with the same idempotency key answers what the first call
    /// did and marks nothing, across restarts too.
    pub(super) async fn mark_commits_of(
        &self,
        caller: &Caller,
        request: pb::MarkCommitsReviewedRequest,
    ) -> Result<pb::PullRequestCommits, Status> {
        let (folder, pr, stack) = self
            .pr_named(
                caller,
                request.session.as_ref(),
                request.pr.as_ref(),
                Some(Capability::UserReviewState),
            )
            .await?;
        if request.commits.is_empty() {
            return Err(missing("commits"));
        }
        let call = MarkCall {
            key: request
                .idempotency_key
                .map(|key| key.value)
                .filter(|key| !key.is_empty()),
            commits: request.commits,
            reviewed: request.reviewed,
        };
        let who = actor(caller);
        let (store, machine) = (self.inner.store.clone(), self.inner.machine.clone());
        blocking(move || mark_once(&store, &folder, &pr, &stack, call, &who))
            .await?
            .map_err(|error| error.status(&machine))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(number: u32) -> ReviewPr {
        ReviewPr {
            owner: "acme".to_owned(),
            name: "api".to_owned(),
            number,
            base_branch: "main".to_owned(),
            head_branch: "patch-1".to_owned(),
            head_sha: String::new(),
            checked_sha: String::new(),
            state: 0,
            submit_started_unix_ms: None,
        }
    }

    #[test]
    fn marking_one_pull_request_does_not_wait_on_another() {
        let first = marking(&pr(482));
        let _held = first.lock().unwrap();
        let other = marking(&pr(483));
        assert!(
            other.try_lock().is_ok(),
            "another pull request marks meanwhile"
        );
        let same = marking(&pr(482));
        assert!(same.try_lock().is_err(), "the same pull request waits");
    }

    fn git_ok(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .output()
            .expect("spawn git");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    #[test]
    fn head_of_in_stack_resolves_per_layer_commits_and_falls_back_to_head() {
        let tmp = tempfile::tempdir().unwrap();
        let origin = tmp.path().join("origin.git");
        let work = tmp.path().join("work");
        let session_folder = tmp.path().join("session");
        let clone = session_folder.join("api");
        std::fs::create_dir_all(&origin).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        std::fs::create_dir_all(&session_folder).unwrap();

        git_ok(&origin, &["init", "--bare"]);
        git_ok(&work, &["init", "-b", "main"]);
        git_ok(&work, &["remote", "add", "origin", origin.to_str().unwrap()]);

        std::fs::write(work.join("base.txt"), "base\n").unwrap();
        git_ok(&work, &["add", "."]);
        git_ok(&work, &["commit", "-m", "base commit"]);
        git_ok(&work, &["push", "origin", "main"]);

        // Layer 1 (PR #101): 1 commit
        git_ok(&work, &["checkout", "-b", "feat-1"]);
        std::fs::write(work.join("a.txt"), "a\n").unwrap();
        git_ok(&work, &["add", "."]);
        git_ok(&work, &["commit", "-m", "layer 1 commit"]);
        git_ok(&work, &["push", "origin", "HEAD:refs/pull/101/head"]);

        // Layer 2 (PR #102): 2 commits
        git_ok(&work, &["checkout", "-b", "feat-2"]);
        std::fs::write(work.join("b1.txt"), "b1\n").unwrap();
        git_ok(&work, &["add", "."]);
        git_ok(&work, &["commit", "-m", "layer 2 commit 1"]);
        std::fs::write(work.join("b2.txt"), "b2\n").unwrap();
        git_ok(&work, &["add", "."]);
        git_ok(&work, &["commit", "-m", "layer 2 commit 2"]);
        git_ok(&work, &["push", "origin", "HEAD:refs/pull/102/head"]);

        // Single clone under session_folder/api
        git_ok(
            &session_folder,
            &["clone", origin.to_str().unwrap(), clone.to_str().unwrap()],
        );

        let mut pr1 = pr(101);
        pr1.base_branch = "main".to_owned();
        pr1.head_branch = "feat-1".to_owned();
        let mut pr2 = pr(102);
        pr2.base_branch = "feat-1".to_owned();
        pr2.head_branch = "feat-2".to_owned();
        let stack = vec![pr1.clone(), pr2.clone()];

        heads::fetch_stack_layer_refs_blocking(&clone, &stack).unwrap();

        let h1 = head_of_in_stack(&session_folder, &pr1, &stack).unwrap();
        let subjects1: Vec<&str> = h1.commits.iter().map(|c| c.subject.as_str()).collect();
        assert_eq!(subjects1, vec!["layer 1 commit"]);

        let h2 = head_of_in_stack(&session_folder, &pr2, &stack).unwrap();
        let subjects2: Vec<&str> = h2.commits.iter().map(|c| c.subject.as_str()).collect();
        assert_eq!(subjects2, vec!["layer 2 commit 1", "layer 2 commit 2"]);

        // Fallback to HEAD when refs/humata/pr-<n> does not exist.
        git_ok(&clone, &["checkout", "-b", "solo-branch"]);
        std::fs::write(clone.join("solo.txt"), "solo\n").unwrap();
        git_ok(&clone, &["add", "."]);
        git_ok(&clone, &["commit", "-m", "solo commit on HEAD"]);
        let mut solo = pr(999);
        solo.base_branch = "main".to_owned();
        solo.head_branch = "solo-branch".to_owned();
        let h_solo = head_of(&session_folder, &solo).unwrap();
        let solo_subjects: Vec<&str> = h_solo.commits.iter().map(|c| c.subject.as_str()).collect();
        assert_eq!(solo_subjects, vec!["solo commit on HEAD"]);
    }
}

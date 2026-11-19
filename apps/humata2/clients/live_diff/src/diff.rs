//! A session's changes as the Diff panel draws them, read through the
//! daemon's DiffService.

use humata2_clients::DaemonError;
use humata2_clients::model::{
    DiffBase, DiffLine, FileChange, FileDiff, FileTier, LineKind, PushChoice, PushPlan, PushRepo,
    LookedFile, RepoBase, RepoBases, RepoDiff, RepoDiffSpec, SessionFile, SessionId,
};
use humata2_clients_live_core::{
    LiveConnection, MESSAGE_MAX_BYTES, Wire, fresh_key, map,
};
use crate::humata2_grpc::humata::v1 as pb;
use crate::humata2_grpc::humata::v1::{
    diff_service_client::DiffServiceClient, file_service_client::FileServiceClient,
    session_service_client::SessionServiceClient,
};

fn file_service(conn: &LiveConnection) -> FileServiceClient<Wire> {
    FileServiceClient::new(conn.wire())
}

fn diffs(conn: &LiveConnection) -> DiffServiceClient<Wire> {
    DiffServiceClient::new(conn.wire())
        .max_decoding_message_size(MESSAGE_MAX_BYTES)
        .max_encoding_message_size(MESSAGE_MAX_BYTES)
}

fn sessions(conn: &LiveConnection) -> SessionServiceClient<Wire> {
    SessionServiceClient::new(conn.wire())
        .max_decoding_message_size(MESSAGE_MAX_BYTES)
        .max_encoding_message_size(MESSAGE_MAX_BYTES)
}

/// A tier as the wire carries it (D802); unset reads as normal.
#[must_use]
pub fn file_tier(wire: i32) -> FileTier {
    use pb::review_importance::Tier;
    match Tier::try_from(wire) {
        Ok(Tier::Key) => FileTier::Key,
        Ok(Tier::Skim) => FileTier::Skim,
        _ => FileTier::Normal,
    }
}

const WHAT: &str = "The session's changes";

fn wire_base(base: &DiffBase) -> Option<pb::DiffBase> {
    use pb::diff_base::Base;
    let base = match base {
        DiffBase::OriginMain => Base::OriginMain(pb::OriginMain {}),
        DiffBase::UncommittedOnly => Base::UncommittedOnly(pb::UncommittedOnly {}),
        DiffBase::SinceLastReviewed => Base::SinceLastReviewed(pb::SinceLastReviewed {}),
        DiffBase::SinceFork => Base::SinceFork(pb::SinceFork {}),
        DiffBase::OtherSession(other) => Base::OtherSession(pb::SessionId {
            value: other.as_str().to_owned(),
        }),
        DiffBase::PullRequestBase => Base::PullRequestBase(pb::PullRequestBase {}),
        DiffBase::Spec(repos) => Base::Spec(spec_to_wire(repos)?),
    };
    Some(pb::DiffBase { base: Some(base) })
}

/// Each repository's base as the daemon's presets make it, with everything
/// on disk on the new side (unstaged and untracked files). `None` when a
/// repository isn't `owner/name`.
#[must_use]
pub fn spec_to_wire(repos: &[RepoDiffSpec]) -> Option<pb::DiffSpec> {
    use pb::diff_spec_compare::{Untracked, Upto};
    let repos = repos
        .iter()
        .map(|each| {
            Some(pb::RepoDiffSpec {
                repo: Some(map::parse_repo(&each.repo)?),
                base: Some(pb::DiffSpecBase {
                    base: Some(repo_base_to_wire(&each.base)),
                }),
                compare: Some(pb::DiffSpecCompare {
                    upto: Upto::Unstaged as i32,
                    untracked: Untracked::All as i32,
                    untracked_paths: Vec::new(),
                    head: match &each.base {
                        RepoBase::Range { head, .. } => head.clone(),
                        _ => String::new(),
                    },
                    worktree_paths: match &each.base {
                        RepoBase::Overlay { paths, .. } => paths.clone(),
                        _ => Vec::new(),
                    },
                }),
                ..pb::RepoDiffSpec::default()
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(pb::DiffSpec { repos })
}

/// The old side of one repository's spec. `DefaultBranch` is `origin/HEAD`,
/// which git resolves to the remote's default branch; `Uncommitted` is HEAD,
/// as the daemon's `UncommittedOnly` base is.
fn repo_base_to_wire(base: &RepoBase) -> pb::diff_spec_base::Base {
    use pb::diff_spec_base::Base;
    match base {
        RepoBase::DefaultBranch => Base::MergeBaseWith("origin/HEAD".to_owned()),
        RepoBase::Branch(branch) if branch.starts_with("origin/") => {
            Base::MergeBaseWith(branch.clone())
        }
        RepoBase::Branch(branch) => Base::MergeBaseWith(format!("origin/{branch}")),
        RepoBase::Rev(rev) => Base::Rev(rev.clone()),
        RepoBase::LastReviewed => Base::ReviewedSnapshot(0),
        RepoBase::ForkPoint => Base::ForkPoint(pb::ForkPointBase {}),
        RepoBase::Uncommitted => Base::Rev("HEAD".to_owned()),
        RepoBase::PullRequestBase => Base::PullRequestBase(pb::PullRequestBaseRef {}),
        RepoBase::Range { base, .. } | RepoBase::Overlay { base, .. } => Base::Rev(base.clone()),
    }
}

/// Whether a base is a stack layer or the Whole stack (D800): exactly its
/// repositories show.
fn is_stack(base: &DiffBase) -> bool {
    match base {
        DiffBase::Spec(repos) => repos
            .iter()
            .any(|each| matches!(each.base, RepoBase::Range { .. } | RepoBase::Overlay { .. })),
        _ => false,
    }
}

/// The repositories a `GetDiff` is limited to: a stack's own, none (every
/// repository of the session) for anything else.
fn repo_filter(base: &DiffBase) -> Vec<pb::RepoRef> {
    match base {
        DiffBase::Spec(repos) if is_stack(base) => repos
            .iter()
            .filter_map(|each| map::parse_repo(&each.repo))
            .collect(),
        _ => Vec::new(),
    }
}

fn line(kind: LineKind, old: u32, new: u32, text: &str) -> DiffLine {
    let (old, new) = match kind {
        LineKind::Context => (Some(old), Some(new)),
        LineKind::Added => (None, Some(new)),
        LineKind::Removed => (Some(old), None),
        LineKind::Hunk | LineKind::Note => (None, None),
    };
    DiffLine {
        kind,
        old,
        new,
        text: text.to_owned(),
    }
}

/// The base's name as the Diff panel shows it. A spec has one base per
/// repository, so `repo` picks it out; a repository the spec leaves out is
/// compared with its default branch, as the daemon compares it.
fn readable_base(base: &DiffBase, repo: &str) -> String {
    match base {
        DiffBase::OriginMain => "origin/main".to_owned(),
        DiffBase::UncommittedOnly => "uncommitted changes".to_owned(),
        DiffBase::SinceLastReviewed => "since last reviewed".to_owned(),
        DiffBase::SinceFork => "since fork".to_owned(),
        DiffBase::OtherSession(other) => format!("session {}", other.as_str()),
        DiffBase::PullRequestBase => "the pull request's base".to_owned(),
        DiffBase::Spec(repos) => match repos.iter().find(|each| each.repo == repo) {
            Some(each) => readable_repo_base(&each.base),
            None => readable_repo_base(&RepoBase::DefaultBranch),
        },
    }
}

fn readable_repo_base(base: &RepoBase) -> String {
    match base {
        RepoBase::DefaultBranch => "default branch".to_owned(),
        RepoBase::Branch(branch) => branch
            .strip_prefix("origin/")
            .unwrap_or(branch.as_str())
            .to_owned(),
        RepoBase::Rev(rev) => rev.clone(),
        RepoBase::LastReviewed => "last reviewed".to_owned(),
        RepoBase::ForkPoint => "since fork".to_owned(),
        RepoBase::Uncommitted => "uncommitted changes".to_owned(),
        RepoBase::PullRequestBase => "the pull request's base".to_owned(),
        RepoBase::Range { base, head } => format!("{base}..{head}"),
        RepoBase::Overlay { base, .. } => format!("{base} and uncommitted changes"),
    }
}

fn push_repo(repo: &pb::push_plan::Repo) -> PushRepo {
    PushRepo {
        repo: repo.repo.as_ref().map(map::repo_text).unwrap_or_default(),
        branch: repo.branch.clone(),
        dirty_files: repo.dirty_files,
        unpushed_commits: repo.unpushed_commits,
        added: repo.added,
        removed: repo.removed,
        open_pull_request: repo.existing.as_ref().map(|existing| existing.number),
    }
}

/// A file in the session's clone of `repo` (`owner/name`), whole, as
/// the editor and document tabs show it. Refuses a file that isn't text.
pub async fn read_session_file(
    conn: &LiveConnection,
    session: &SessionId,
    repo: &str,
    path: &str,
) -> Result<String, DaemonError> {
    const WHAT: &str = "Reading the file";
    let (owner, name) = repo.split_once('/').unwrap_or(("", repo));
    let session_id = Some(pb::SessionId {
        value: session.as_str().to_owned(),
    });
    // D382: a Desk session has no repository; its document sits in the
    // session's own folder.
    let root = match repo.is_empty() {
        true => pb::file_location::Root::SessionFolder(pb::SessionFolder {
            session: session_id,
        }),
        false => pb::file_location::Root::SessionClone(pb::SessionClone {
            session: session_id,
            repo: Some(pb::RepoRef {
                owner: owner.to_owned(),
                name: name.to_owned(),
            }),
        }),
    };
    let location = pb::FileLocation {
        machine: None,
        root: Some(root),
        path: path.to_owned(),
    };
    let mut chunks = file_service(conn)
        .read_file(pb::ReadFileRequest {
            location: Some(location),
            ..pb::ReadFileRequest::default()
        })
        .await
        .map_err(|status| conn.error(&status, WHAT))?
        .into_inner();
    let mut bytes = Vec::new();
    while let Some(chunk) = chunks
        .message()
        .await
        .map_err(|status| conn.error(&status, WHAT))?
    {
        bytes.extend_from_slice(&chunk.data);
        if chunk.last {
            break;
        }
    }
    String::from_utf8(bytes).map_err(|_| DaemonError::Refused {
        what: WHAT.to_owned(),
        reason: format!("{repo}/{path} isn't UTF-8 text, so it can't be shown as text; open it with Open big or in another app"),
    })
}

/// The files in each of the session's clones, as many as
/// [`SESSION_FILES_CAP`] shared fairly among the repos, for the palette's
/// Files group and the Files panel. The daemon pages repos in the
/// session's order, so pages are followed (up to [`FIND_PAGES_MAX`]) until
/// every repo is reached, and one big repo can't hide the rest.
pub async fn find_session_files(
    conn: &LiveConnection,
    session: &SessionId,
) -> Result<Vec<SessionFile>, DaemonError> {
    let mut files = Vec::new();
    let mut token = String::new();
    for _ in 0..FIND_PAGES_MAX {
        let reply = file_service(conn)
            .find_files(pb::FindFilesRequest {
                session: Some(pb::SessionId {
                    value: session.as_str().to_owned(),
                }),
                query: String::new(),
                page: Some(pb::PageRequest {
                    page_size: FIND_PAGE_SIZE,
                    page_token: std::mem::take(&mut token),
                }),
            })
            .await
            .map_err(|status| conn.error(&status, "The session's files"))?
            .into_inner();
        files.extend(reply.entries.iter().filter_map(session_file));
        token = reply.page.map(|page| page.next_page_token).unwrap_or_default();
        if token.is_empty() {
            break;
        }
    }
    Ok(humata2_clients::fair_share(files, humata2_clients::SESSION_FILES_CAP))
}

/// What pushing each of the session's repositories would do, with the
/// token that spends the plan.
pub async fn plan_push(
    conn: &LiveConnection,
    session: &SessionId,
) -> Result<PushPlan, DaemonError> {
    let plan = sessions(conn)
        .plan_push(pb::PlanPushRequest {
            id: Some(pb::SessionId {
                value: session.as_str().to_owned(),
            }),
            spec: None,
        })
        .await
        .map_err(|status| conn.error(&status, "Reading what pushing would do"))?
        .into_inner();
    let token = plan
        .token
        .map(|token| token.value)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| DaemonError::Refused {
            what: "Reading what pushing would do".to_owned(),
            reason: "the daemon gave no plan to push with. Read the changes again; if it keeps happening, update the daemon".to_owned(),
        })?;
    Ok(PushPlan {
        token,
        repos: plan.repos.iter().map(push_repo).collect(),
    })
}

/// Commits, pushes and opens pull requests for the chosen repositories
/// with the plan's token. Returns the pull request the daemon named.
pub async fn push(
    conn: &LiveConnection,
    session: &SessionId,
    token: &str,
    choices: Vec<PushChoice>,
) -> Result<Vec<humata2_clients::model::PushedPullRequest>, DaemonError> {
    let mut repos = Vec::with_capacity(choices.len());
    for choice in choices {
        let repo = map::parse_repo(&choice.repo).ok_or_else(|| DaemonError::Refused {
            what: format!("Pushing {}", choice.repo),
            reason: format!(
                "{} isn't a repository name in owner/name form. Read the changes again and push from the group the repository is in",
                choice.repo
            ),
        })?;
        repos.push(pb::PushChoice {
            repo: Some(repo),
            commit_message: choice.commit_message,
            open_pull_request: choice.open_pull_request,
            pr_title: choice.title,
            pr_body: choice.body,
            base_branch: choice.base_branch,
        });
    }
    let names = repos
        .iter()
        .filter_map(|repo| repo.repo.as_ref().map(map::repo_text))
        .collect::<Vec<_>>()
        .join(", ");
    let what = format!("Push {names}");
    let operation = sessions(conn)
        .push_and_open_pull_requests(pb::PushAndOpenPullRequestsRequest {
            idempotency_key: fresh_key(),
            id: Some(pb::SessionId {
                value: session.as_str().to_owned(),
            }),
            repos,
            plan: Some(pb::PlanToken {
                value: token.to_owned(),
            }),
            from_view: None,
        })
        .await
        .map_err(|status| conn.error(&status, &what))?
        .into_inner();
    let done = conn.finish_operation(operation, &what).await?;
    Ok(done
        .pull_requests
        .into_iter()
        .map(|pull_request| humata2_clients::model::PushedPullRequest {
            repo: pull_request
                .repo
                .as_ref()
                .map(map::repo_text)
                .unwrap_or_default(),
            number: pull_request.number,
            note: pull_request.note,
        })
        .collect())
}

/// Remembers what the session's changes are now as reviewed.
pub async fn mark_reviewed(
    conn: &LiveConnection,
    session: &SessionId,
) -> Result<(), DaemonError> {
    diffs(conn)
        .mark_reviewed(pb::MarkReviewedRequest {
            idempotency_key: fresh_key(),
            session: Some(pb::SessionId {
                value: session.as_str().to_owned(),
            }),
            // The app is the person: no presence proof is sent.
            presence: None,
            shown_tree: Vec::new(),
        })
        .await
        .map_err(|status| conn.error(&status, "Marking the changes reviewed"))?;
    Ok(())
}

/// The files the person marked Looks good in the session, as the daemon
/// kept them (D852).
pub async fn looks(
    conn: &LiveConnection,
    session: &SessionId,
) -> Result<Vec<LookedFile>, DaemonError> {
    let kept = diffs(conn)
        .get_looks(pb::GetLooksRequest {
            session: Some(pb::SessionId {
                value: session.as_str().to_owned(),
            }),
        })
        .await
        .map_err(|status| conn.error(&status, "Reading what you looked at"))?
        .into_inner();
    Ok(kept
        .looks
        .into_iter()
        .map(|look| LookedFile {
            repo: look
                .repo
                .map(|repo| {
                    if repo.owner.is_empty() {
                        repo.name
                    } else {
                        format!("{}/{}", repo.owner, repo.name)
                    }
                })
                .unwrap_or_default(),
            path: look.path,
            fingerprint: look.fingerprint,
        })
        .collect())
}

/// Replaces the session's looks on the daemon with `looks` (D852).
pub async fn save_looks(
    conn: &LiveConnection,
    session: &SessionId,
    looks: Vec<LookedFile>,
) -> Result<(), DaemonError> {
    let looks = looks
        .into_iter()
        .map(|look| pb::LookedFile {
            repo: Some(map::parse_repo(&look.repo).unwrap_or_else(|| pb::RepoRef {
                owner: String::new(),
                name: look.repo.clone(),
            })),
            path: look.path,
            fingerprint: look.fingerprint,
        })
        .collect();
    diffs(conn)
        .save_looks(pb::SaveLooksRequest {
            idempotency_key: fresh_key(),
            session: Some(pb::SessionId {
                value: session.as_str().to_owned(),
            }),
            looks,
        })
        .await
        .map_err(|status| conn.error(&status, "Saving what you looked at"))?;
    Ok(())
}

/// The branches one repository of the session can be compared with, the
/// default branch first, as the session's clone knows them at its last
/// fetch. Nothing is fetched.
pub async fn repo_bases(
    conn: &LiveConnection,
    session: &SessionId,
    repo: &str,
) -> Result<RepoBases, DaemonError> {
    let what = format!("The branches of {repo}");
    let repo_ref = map::parse_repo(repo).ok_or_else(|| DaemonError::Refused {
        what: what.clone(),
        reason: format!(
            "{repo} isn't a repository name in owner/name form. Pick one of the session's repositories from its list"
        ),
    })?;
    let listed = diffs(conn)
        .list_bases(pb::ListBasesRequest {
            session: Some(pb::SessionId {
                value: session.as_str().to_owned(),
            }),
            repo: Some(repo_ref),
        })
        .await
        .map_err(|status| conn.error(&status, &what))?
        .into_inner();
    Ok(RepoBases {
        branches: listed.branches,
        default_branch: listed.default_branch,
    })
}

/// Has the daemon ask `repo`'s remote for its default branch in the
/// session's clone (frame 98); the branch origin/HEAD names now.
pub async fn find_default_branch(
    conn: &LiveConnection,
    session: &SessionId,
    repo: &str,
) -> Result<String, DaemonError> {
    let what = format!("Finding {repo}'s default branch");
    // The daemon checks the name; one without an owner is a session
    // repository the daemon knows no owner for.
    let repo_ref = map::parse_repo(repo).unwrap_or_else(|| pb::RepoRef {
        owner: String::new(),
        name: repo.to_owned(),
    });
    let found = diffs(conn)
        .find_default_branch(pb::FindDefaultBranchRequest {
            idempotency_key: fresh_key(),
            session: Some(pb::SessionId {
                value: session.as_str().to_owned(),
            }),
            repo: Some(repo_ref),
        })
        .await
        .map_err(|status| conn.error(&status, &what))?
        .into_inner();
    Ok(found.default_branch)
}

/// Every repository's changes in the session's clones, against where
/// its branch started. Each file is fetched whole, page after page.
pub async fn session_diffs(
    conn: &LiveConnection,
    session: &SessionId,
    base: &DiffBase,
) -> Result<Vec<RepoDiff>, DaemonError> {
    read_diffs(conn, session, base, repo_filter(base)).await
}

/// One repository's changes in the session (DS-35), read on its own so a
/// repository that fails doesn't hide the others.
pub async fn repo_diff(
    conn: &LiveConnection,
    session: &SessionId,
    base: &DiffBase,
    repo: &str,
) -> Result<Vec<RepoDiff>, DaemonError> {
    let repo_ref = map::parse_repo(repo).ok_or_else(|| DaemonError::Refused {
        what: format!("The changes of {repo}"),
        reason: format!(
            "{repo} isn't a repository name in owner/name form. Pick one of the session's repositories from its list"
        ),
    })?;
    read_diffs(conn, session, base, vec![repo_ref]).await
}

/// The changes of the repositories in `repos` (all of the session's when
/// empty), each file fetched whole, page after page.
async fn read_diffs(
    conn: &LiveConnection,
    session: &SessionId,
    base: &DiffBase,
    repos: Vec<pb::RepoRef>,
) -> Result<Vec<RepoDiff>, DaemonError> {
    let id = Some(pb::SessionId {
        value: session.as_str().to_owned(),
    });
    let listing = diffs(conn)
        .get_diff(pb::GetDiffRequest {
            session: id.clone(),
            base: wire_base(base),
            repos,
        })
        .await
        .map_err(|status| conn.error(&status, WHAT))?
        .into_inner();
    let mut result = Vec::new();
    for listed in &listing.repos {
        let repo = listed.repo.clone().unwrap_or_default();
        let name = map::repo_text(&repo);
        let mut files = Vec::new();
        for summary in &listed.files {
            let mut lines = Vec::new();
            if !summary.binary {
                let mut from = None;
                loop {
                    let page = diffs(conn)
                        .get_file_diff(pb::GetFileDiffRequest {
                            session: id.clone(),
                            base: wire_base(base),
                            file: Some(pb::RepoPath {
                                repo: Some(repo.clone()),
                                path: summary.path.clone(),
                            }),
                            trees: listed.trees.clone(),
                            from,
                            ..pb::GetFileDiffRequest::default()
                        })
                        .await
                        .map_err(|status| conn.error(&status, WHAT))?
                        .into_inner();
                    for hunk in &page.hunks {
                        if !hunk.continued {
                            lines.push(line(LineKind::Hunk, 0, 0, &hunk.header));
                        }
                        for each in &hunk.lines {
                            let kind = match pb::diff_line::Kind::try_from(each.kind) {
                                Ok(pb::diff_line::Kind::Added) => LineKind::Added,
                                Ok(pb::diff_line::Kind::Removed) => LineKind::Removed,
                                _ => LineKind::Context,
                            };
                            lines.push(line(kind, each.old_line, each.new_line, &each.text));
                            if each.no_newline {
                                lines.push(DiffLine {
                                    kind: LineKind::Note,
                                    old: None,
                                    new: None,
                                    text: "\\ No newline at end of file".to_owned(),
                                });
                            }
                        }
                    }
                    match page.next {
                        Some(next) => from = Some(next),
                        None => break,
                    }
                }
            }
            let change = match pb::ChangeKind::try_from(summary.change) {
                Ok(pb::ChangeKind::Added) => FileChange::Added,
                Ok(pb::ChangeKind::Deleted) => FileChange::Deleted,
                Ok(pb::ChangeKind::Renamed) => FileChange::Renamed,
                _ => FileChange::Modified,
            };
            let old_path = (change == FileChange::Renamed && !summary.old_path.is_empty())
                .then(|| summary.old_path.clone());
            files.push(FileDiff {
                path: summary.path.clone(),
                added: summary.added,
                removed: summary.removed,
                lines,
                change,
                old_path,
                similarity: summary.similarity,
                binary: summary.binary,
                error: None,
                old_mode: summary.old_mode,
                new_mode: summary.new_mode,
                tier: file_tier(summary.tier),
            });
        }
        result.push(RepoDiff {
            session: session.clone(),
            repo: name.clone(),
            base: if listed.label.is_empty() {
                readable_base(base, &name)
            } else {
                listed.label.clone()
            },
            files,
            comments: Vec::new(),
        });
    }
    Ok(result)
}

/// Files asked for per page when listing a session's files: the daemon's
/// own page limit.
const FIND_PAGE_SIZE: u32 = 1000;

/// Pages followed at most when listing a session's files, so a huge clone
/// costs a bounded number of calls.
const FIND_PAGES_MAX: usize = 20;

/// A session clone's file as the Files panel lists it, or None for anything
/// else.
fn session_file(entry: &pb::FileEntry) -> Option<SessionFile> {
    let location = entry.location.as_ref()?;
    // D382: a Desk session's own folder is named by the empty repo, as its
    // documents and comments are.
    let repo = match &location.root {
        Some(pb::file_location::Root::SessionClone(clone)) => map::repo_text(clone.repo.as_ref()?),
        Some(pb::file_location::Root::SessionFolder(_)) => String::new(),
        _ => return None,
    };
    Some(SessionFile {
        repo,
        path: location.path.clone(),
        folder: entry.kind == pb::file_entry::Kind::Folder as i32,
    })
}

#[cfg(test)]
mod tests {
    use humata2_clients::model::RepoDiffSpec;

    use super::*;
    use pb::diff_spec_base::Base as SpecBase;

    fn spec(repo: &str, base: RepoBase) -> RepoDiffSpec {
        RepoDiffSpec {
            repo: repo.to_owned(),
            base,
        }
    }

    fn repo_specs(base: &DiffBase) -> Option<Vec<pb::RepoDiffSpec>> {
        match wire_base(base)?.base? {
            pb::diff_base::Base::Spec(each) => Some(each.repos),
            _ => None,
        }
    }

    fn old_side(repo: &pb::RepoDiffSpec) -> Option<SpecBase> {
        repo.base.clone()?.base
    }

    fn repo_ref(owner: &str, name: &str) -> Option<pb::RepoRef> {
        Some(pb::RepoRef {
            owner: owner.to_owned(),
            name: name.to_owned(),
        })
    }

    #[test]
    fn each_repository_keeps_its_own_base() {
        let base = DiffBase::Spec(vec![
            spec("acme/api", RepoBase::Branch("release/2.4".to_owned())),
            spec("acme/web", RepoBase::LastReviewed),
        ]);
        let Some(repos) = repo_specs(&base) else {
            panic!("a spec should map to a wire spec");
        };
        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0].repo, repo_ref("acme", "api"));
        assert_eq!(
            old_side(&repos[0]),
            Some(SpecBase::MergeBaseWith("origin/release/2.4".to_owned()))
        );
        assert_eq!(repos[1].repo, repo_ref("acme", "web"));
        assert_eq!(old_side(&repos[1]), Some(SpecBase::ReviewedSnapshot(0)));
    }

    #[test]
    fn a_branch_already_on_origin_is_not_prefixed_twice() {
        let base = DiffBase::Spec(vec![spec(
            "acme/api",
            RepoBase::Branch("origin/x".to_owned()),
        )]);
        let Some(repos) = repo_specs(&base) else {
            panic!("a spec should map to a wire spec");
        };
        assert_eq!(
            old_side(&repos[0]),
            Some(SpecBase::MergeBaseWith("origin/x".to_owned()))
        );
    }

    #[test]
    fn uncommitted_is_head_and_everything_on_disk() {
        let base = DiffBase::Spec(vec![spec("acme/api", RepoBase::Uncommitted)]);
        let Some(repos) = repo_specs(&base) else {
            panic!("a spec should map to a wire spec");
        };
        assert_eq!(old_side(&repos[0]), Some(SpecBase::Rev("HEAD".to_owned())));
        let Some(compare) = repos[0].compare.as_ref() else {
            panic!("every repository carries its compare side");
        };
        assert_eq!(
            compare.upto,
            pb::diff_spec_compare::Upto::Unstaged as i32
        );
        assert_eq!(
            compare.untracked,
            pb::diff_spec_compare::Untracked::All as i32
        );
    }

    #[test]
    fn a_repository_without_its_own_label_shows_its_own_base() {
        let base = DiffBase::Spec(vec![
            spec("acme/api", RepoBase::Branch("release/2.4".to_owned())),
            spec("acme/web", RepoBase::LastReviewed),
        ]);
        assert_eq!(readable_base(&base, "acme/api"), "release/2.4");
        assert_eq!(readable_base(&base, "acme/web"), "last reviewed");
    }

    #[test]
    fn a_branch_named_main_is_compared_with_origin_main() {
        let base = DiffBase::Spec(vec![spec("acme/web", RepoBase::Branch("main".to_owned()))]);
        let Some(repos) = repo_specs(&base) else {
            panic!("a spec should map to a wire spec");
        };
        assert_eq!(
            old_side(&repos[0]),
            Some(SpecBase::MergeBaseWith("origin/main".to_owned()))
        );
    }

    #[test]
    fn a_range_compares_its_head_and_an_overlay_its_paths() {
        let base = DiffBase::Spec(vec![
            spec(
                "acme/api",
                RepoBase::Range {
                    base: "aaa".to_owned(),
                    head: "bbb".to_owned(),
                },
            ),
            spec(
                "acme/web",
                RepoBase::Overlay {
                    base: "ccc".to_owned(),
                    paths: vec!["src".to_owned()],
                },
            ),
        ]);
        let Some(repos) = repo_specs(&base) else {
            panic!("a spec should map to a wire spec");
        };
        assert_eq!(old_side(&repos[0]), Some(SpecBase::Rev("aaa".to_owned())));
        assert_eq!(
            repos[0].compare.as_ref().map(|c| c.head.clone()),
            Some("bbb".to_owned())
        );
        assert_eq!(old_side(&repos[1]), Some(SpecBase::Rev("ccc".to_owned())));
        assert_eq!(
            repos[1].compare.as_ref().map(|c| c.worktree_paths.clone()),
            Some(vec!["src".to_owned()])
        );
        assert_eq!(
            repos[1].compare.as_ref().map(|c| c.head.clone()),
            Some(String::new())
        );
    }

    #[test]
    fn a_stack_spec_limits_the_diff_to_its_repositories_and_a_plain_one_does_not() {
        let stack = DiffBase::Spec(vec![spec(
            "acme/api",
            RepoBase::Range {
                base: "a".to_owned(),
                head: "b".to_owned(),
            },
        )]);
        assert_eq!(repo_filter(&stack), vec![pb::RepoRef {
            owner: "acme".to_owned(),
            name: "api".to_owned(),
        }]);
        let plain = DiffBase::Spec(vec![spec("acme/api", RepoBase::DefaultBranch)]);
        assert!(repo_filter(&plain).is_empty());
        assert!(repo_filter(&DiffBase::OriginMain).is_empty());
    }

    #[test]
    fn a_repository_name_that_is_not_owner_slash_name_sends_no_base() {
        let base = DiffBase::Spec(vec![spec("api", RepoBase::LastReviewed)]);
        assert!(wire_base(&base).is_none());
    }

    #[test]
    fn each_pull_requests_base_is_asked_of_the_daemon() {
        assert_eq!(
            wire_base(&DiffBase::PullRequestBase),
            Some(pb::DiffBase {
                base: Some(pb::diff_base::Base::PullRequestBase(pb::PullRequestBase {})),
            })
        );
    }

    #[test]
    fn a_repository_on_its_pull_requests_base_asks_for_it_in_a_spec() {
        let base = DiffBase::Spec(vec![spec("acme/api", RepoBase::PullRequestBase)]);
        let Some(pb::DiffBase {
            base: Some(pb::diff_base::Base::Spec(wire)),
        }) = wire_base(&base)
        else {
            panic!("a spec should map to a wire spec");
        };
        assert_eq!(
            old_side(&wire.repos[0]),
            Some(SpecBase::PullRequestBase(pb::PullRequestBaseRef {}))
        );
    }
}

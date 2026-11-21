//! A session's changes as the Diff panel draws them, read through the
//! daemon's DiffService.

use humata2_clients::DaemonError;
use humata2_clients::model::{
    DiffBase, DiffLine, DiffUntracked, DiffUpto, DiffWhitespace, FileChange, FileDiff, FileTier,
    LineKind, LookedFile, PushChoice, PushPlan, PushRepo, RepoBase, RepoBases, RepoDiff,
    RepoDiffSpec, SessionFile, SessionId,
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

fn wire_base_for_repo(base: &DiffBase, repo: &str) -> Option<pb::DiffBase> {
    if let DiffBase::Spec(repos) = base {
        let matched: Vec<RepoDiffSpec> = repos
            .iter()
            .filter(|each| each.repo == repo)
            .cloned()
            .collect();
        if !matched.is_empty() {
            return Some(pb::DiffBase {
                base: Some(pb::diff_base::Base::Spec(spec_to_wire(&matched)?)),
            });
        }
    }
    wire_base(base)
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
            let pinned = matches!(
                each.base,
                RepoBase::Range { .. } | RepoBase::Overlay { .. }
            );
            let upto = match each.options.upto {
                Some(DiffUpto::Commits) => Upto::Commits as i32,
                Some(DiffUpto::Staged) => Upto::Staged as i32,
                Some(DiffUpto::Unstaged) => Upto::Unstaged as i32,
                None if pinned => Upto::Commits as i32,
                None => Upto::Unstaged as i32,
            };
            let untracked = match each.options.untracked {
                Some(DiffUntracked::None) => Untracked::None as i32,
                Some(DiffUntracked::All) => Untracked::All as i32,
                Some(DiffUntracked::Paths) => Untracked::Paths as i32,
                None if pinned => Untracked::None as i32,
                None => Untracked::All as i32,
            };
            let head = match &each.base {
                RepoBase::Range { head, .. } => head.clone(),
                _ => each.options.head.clone().unwrap_or_default(),
            };
            let worktree_paths = match &each.base {
                RepoBase::Overlay { paths, .. } => paths.clone(),
                _ => each.options.worktree_paths.clone(),
            };
            let whitespace = match each.options.whitespace {
                DiffWhitespace::Default => pb::DiffWhitespace::Unspecified as i32,
                DiffWhitespace::IgnoreChanges => pb::DiffWhitespace::IgnoreChanges as i32,
                DiffWhitespace::IgnoreAll => pb::DiffWhitespace::IgnoreAll as i32,
            };
            Some(pb::RepoDiffSpec {
                repo: Some(map::parse_repo(&each.repo)?),
                base: Some(pb::DiffSpecBase {
                    base: Some(repo_base_to_wire(&each.base)),
                }),
                compare: Some(pb::DiffSpecCompare {
                    upto,
                    untracked,
                    untracked_paths: each.options.untracked_paths.clone(),
                    head,
                    worktree_paths,
                }),
                include_paths: each.options.include_paths.clone(),
                exclude_paths: each.options.exclude_paths.clone(),
                whitespace,
                no_renames: each.options.no_renames,
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
        RepoBase::MergeBase(branch) => Base::MergeBaseWith(branch.clone()),
        RepoBase::Rev(rev) => Base::Rev(rev.clone()),
        RepoBase::LastReviewed => Base::ReviewedSnapshot(0),
        RepoBase::ForkPoint => Base::ForkPoint(pb::ForkPointBase {}),
        RepoBase::Uncommitted => Base::Rev("HEAD".to_owned()),
        RepoBase::Index => Base::Index(pb::IndexBase {}),
        RepoBase::PullRequestBase => Base::PullRequestBase(pb::PullRequestBaseRef {}),
        RepoBase::Range { base, .. } | RepoBase::Overlay { base, .. } => Base::Rev(base.clone()),
    }
}

/// The repositories a `GetDiff` is limited to: a spec's explicit repositories
/// (for both stacked and non-stacked review plans), or none (every repository
/// of the session) for a session-wide preset.
fn repo_filter(base: &DiffBase) -> Vec<pb::RepoRef> {
    match base {
        DiffBase::Spec(repos) => repos
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
        RepoBase::MergeBase(branch) => branch.clone(),
        RepoBase::Rev(rev) => rev.clone(),
        RepoBase::LastReviewed => "last reviewed".to_owned(),
        RepoBase::ForkPoint => "since fork".to_owned(),
        RepoBase::Uncommitted => "uncommitted changes".to_owned(),
        RepoBase::Index => "staged changes".to_owned(),
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

const AGENT_STATE_REPO: &str = "@agent-state";
const AGENT_STATE_PREV_PREFIX: &str = "@agent-state:prev:";

#[derive(Debug, PartialEq)]
enum SessionFileRootTarget {
    Direct(pb::file_location::Root),
    ChannelShared,
    MainClone(String),
}

fn classify_session_file_repo(session: &SessionId, repo: &str) -> SessionFileRootTarget {
    let session_id = Some(pb::SessionId {
        value: session.as_str().to_owned(),
    });
    if repo.is_empty() {
        // D382: a Desk session has no repository; its document sits in the
        // session's own folder.
        return SessionFileRootTarget::Direct(pb::file_location::Root::SessionFolder(
            pb::SessionFolder {
                session: session_id,
            },
        ));
    }
    if repo == AGENT_STATE_REPO {
        return SessionFileRootTarget::Direct(pb::file_location::Root::SessionAgentState(
            pb::SessionAgentState {
                session: session_id,
                conversation_id: String::new(),
            },
        ));
    }
    if let Some(conversation_id) = repo.strip_prefix(AGENT_STATE_PREV_PREFIX) {
        return SessionFileRootTarget::Direct(pb::file_location::Root::SessionAgentState(
            pb::SessionAgentState {
                session: session_id,
                conversation_id: conversation_id.to_owned(),
            },
        ));
    }
    if repo == "shared" {
        return SessionFileRootTarget::ChannelShared;
    }
    if let Some(main_repo) = repo.strip_prefix("main/").filter(|rest| !rest.is_empty()) {
        return SessionFileRootTarget::MainClone(main_repo.to_owned());
    }
    let (owner, name) = repo.split_once('/').unwrap_or(("", repo));
    SessionFileRootTarget::Direct(pb::file_location::Root::SessionClone(pb::SessionClone {
        session: session_id,
        repo: Some(pb::RepoRef {
            owner: owner.to_owned(),
            name: name.to_owned(),
        }),
    }))
}

fn resolve_main_clone_repo(spec: &str, session_repos: &[pb::SessionRepo]) -> pb::RepoRef {
    if let Some(repo_ref) = map::parse_repo(spec) {
        return repo_ref;
    }
    if let Some(found) = session_repos
        .iter()
        .filter_map(|sr| sr.repo.as_ref())
        .find(|repo_ref| repo_ref.name == spec)
    {
        return found.clone();
    }
    pb::RepoRef {
        owner: String::new(),
        name: spec.to_owned(),
    }
}

fn resolve_channel_root(
    target: SessionFileRootTarget,
    channel: Option<pb::ChannelId>,
    session_repos: &[pb::SessionRepo],
    what: &str,
) -> Result<pb::file_location::Root, DaemonError> {
    match target {
        SessionFileRootTarget::Direct(root) => Ok(root),
        SessionFileRootTarget::ChannelShared => {
            let channel = channel
                .filter(|ch| !ch.value.is_empty())
                .ok_or_else(|| DaemonError::Refused {
                    what: what.to_owned(),
                    reason: "this session has no channel, so there is no shared folder to read from".to_owned(),
                })?;
            Ok(pb::file_location::Root::ChannelShared(pb::ChannelShared {
                channel: Some(channel),
            }))
        }
        SessionFileRootTarget::MainClone(spec) => {
            let channel = channel
                .filter(|ch| !ch.value.is_empty())
                .ok_or_else(|| DaemonError::Refused {
                    what: what.to_owned(),
                    reason: "this session has no channel, so there is no main clone to read from".to_owned(),
                })?;
            let repo = resolve_main_clone_repo(&spec, session_repos);
            Ok(pb::file_location::Root::MainClone(pb::MainClone {
                channel: Some(channel),
                repo: Some(repo),
            }))
        }
    }
}

/// A file in the session's clone of `repo` (`owner/name`), session folder
/// (`""`), agent state (`@agent-state` / `@agent-state:prev:<id>`), or
/// channel-level read-only root (`shared`, `main/<repo>`), whole, as the
/// editor and document tabs show it. Refuses a file that isn't text.
pub async fn read_session_file(
    conn: &LiveConnection,
    session: &SessionId,
    repo: &str,
    path: &str,
) -> Result<String, DaemonError> {
    const WHAT: &str = "Reading the file";
    let root = match classify_session_file_repo(session, repo) {
        SessionFileRootTarget::Direct(root) => root,
        target => {
            let fetched = sessions(conn)
                .get_session(pb::GetSessionRequest {
                    id: Some(pb::SessionId {
                        value: session.as_str().to_owned(),
                    }),
                })
                .await
                .map_err(|status| conn.error(&status, WHAT))?
                .into_inner();
            resolve_channel_root(target, fetched.channel, &fetched.repos, WHAT)?
        }
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
    let display = if repo.is_empty() {
        path.to_owned()
    } else {
        format!("{repo}/{path}")
    };
    String::from_utf8(bytes).map_err(|_| DaemonError::Refused {
        what: WHAT.to_owned(),
        reason: format!("{display} isn't UTF-8 text, so it can't be shown as text; open it with Open big or in another app"),
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

/// Whether a `GetFileDiff` error belongs to this one file (DS-36) rather than
/// the connection, daemon version, or repository configuration.
fn file_read_error(error: &DaemonError) -> Option<String> {
    match error {
        DaemonError::Refused { .. }
        | DaemonError::NotFound { .. }
        | DaemonError::Conflict { .. } => Some(error.to_string()),
        DaemonError::Offline { .. }
        | DaemonError::MigrationNeeded { .. }
        | DaemonError::NoDefaultBranch { .. }
        | DaemonError::NotServedYet { .. }
        | DaemonError::Scripted { .. } => None,
    }
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
            let mut error = None;
            if !summary.binary {
                let file_what = format!("{name}/{}", summary.path);
                let mut from = None;
                loop {
                    let page = match diffs(conn)
                        .get_file_diff(pb::GetFileDiffRequest {
                            session: id.clone(),
                            base: wire_base_for_repo(base, &name),
                            file: Some(pb::RepoPath {
                                repo: Some(repo.clone()),
                                path: summary.path.clone(),
                            }),
                            trees: listed.trees.clone(),
                            from,
                            ..pb::GetFileDiffRequest::default()
                        })
                        .await
                    {
                        Ok(reply) => reply.into_inner(),
                        Err(status) => {
                            let failed = conn.error(&status, &file_what);
                            if let Some(reason) = file_read_error(&failed) {
                                lines.clear();
                                error = Some(reason);
                                break;
                            }
                            return Err(failed);
                        }
                    };
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
                error,
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
        Some(pb::file_location::Root::SessionAgentState(state)) => {
            if state.conversation_id.is_empty() {
                AGENT_STATE_REPO.to_owned()
            } else {
                format!("{AGENT_STATE_PREV_PREFIX}{}", state.conversation_id)
            }
        }
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
    use humata2_clients::model::{RepoDiffOptions, RepoDiffSpec};

    use super::*;
    use pb::diff_spec_base::Base as SpecBase;

    fn spec(repo: &str, base: RepoBase) -> RepoDiffSpec {
        RepoDiffSpec {
            repo: repo.to_owned(),
            base,
            options: RepoDiffOptions::default(),
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
        use pb::diff_spec_compare::{Untracked, Upto};
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
            spec("acme/docs", RepoBase::DefaultBranch),
        ]);
        let Some(repos) = repo_specs(&base) else {
            panic!("a spec should map to a wire spec");
        };
        assert_eq!(old_side(&repos[0]), Some(SpecBase::Rev("aaa".to_owned())));
        let range_cmp = repos[0].compare.as_ref().expect("range compare");
        assert_eq!(range_cmp.head, "bbb");
        assert_eq!(range_cmp.upto, Upto::Commits as i32);
        assert_eq!(range_cmp.untracked, Untracked::None as i32);

        assert_eq!(old_side(&repos[1]), Some(SpecBase::Rev("ccc".to_owned())));
        let overlay_cmp = repos[1].compare.as_ref().expect("overlay compare");
        assert_eq!(overlay_cmp.worktree_paths, vec!["src".to_owned()]);
        assert_eq!(overlay_cmp.head, "");
        assert_eq!(overlay_cmp.upto, Upto::Commits as i32);
        assert_eq!(overlay_cmp.untracked, Untracked::None as i32);

        let default_cmp = repos[2].compare.as_ref().expect("default compare");
        assert_eq!(default_cmp.upto, Upto::Unstaged as i32);
        assert_eq!(default_cmp.untracked, Untracked::All as i32);
    }

    #[test]
    fn explicit_spec_limits_the_diff_to_its_repositories_and_presets_do_not() {
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
        assert_eq!(repo_filter(&plain), vec![pb::RepoRef {
            owner: "acme".to_owned(),
            name: "api".to_owned(),
        }]);
        assert!(repo_filter(&DiffBase::OriginMain).is_empty());
    }

    #[test]
    fn repo_diff_options_index_and_local_merge_base_survive_spec_to_wire() {
        use pb::diff_spec_compare::{Untracked, Upto};
        let base = DiffBase::Spec(vec![
            RepoDiffSpec {
                repo: "acme/api".to_owned(),
                base: RepoBase::Index,
                options: RepoDiffOptions {
                    include_paths: vec!["src/".to_owned()],
                    exclude_paths: vec!["src/gen/".to_owned()],
                    upto: Some(DiffUpto::Staged),
                    untracked: Some(DiffUntracked::Paths),
                    untracked_paths: vec!["src/new.rs".to_owned()],
                    head: None,
                    worktree_paths: Vec::new(),
                    whitespace: DiffWhitespace::IgnoreAll,
                    no_renames: true,
                },
            },
            RepoDiffSpec {
                repo: "acme/web".to_owned(),
                base: RepoBase::MergeBase("local-feature".to_owned()),
                options: RepoDiffOptions::default(),
            },
        ]);
        let repos = repo_specs(&base).expect("maps to wire spec");
        assert_eq!(old_side(&repos[0]), Some(SpecBase::Index(pb::IndexBase {})));
        let compare = repos[0].compare.as_ref().expect("compare");
        assert_eq!(compare.upto, Upto::Staged as i32);
        assert_eq!(compare.untracked, Untracked::Paths as i32);
        assert_eq!(compare.untracked_paths, vec!["src/new.rs".to_owned()]);
        assert_eq!(repos[0].include_paths, vec!["src/".to_owned()]);
        assert_eq!(repos[0].exclude_paths, vec!["src/gen/".to_owned()]);
        assert_eq!(repos[0].whitespace, pb::DiffWhitespace::IgnoreAll as i32);
        assert!(repos[0].no_renames);
        assert_eq!(
            old_side(&repos[1]),
            Some(SpecBase::MergeBaseWith("local-feature".to_owned()))
        );
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

    #[test]
    fn per_file_errors_are_captured_while_connection_errors_abort() {
        use humata2_clients::Operation;
        use humata2_clients::model::{MachineId, MigrateThing};

        let file_errors = [
            DaemonError::Refused {
                what: "acme/api/src/a.rs".into(),
                reason: "blob unreadable".into(),
            },
            DaemonError::NotFound {
                what: "acme/api/src/a.rs".into(),
            },
            DaemonError::Conflict {
                what: "acme/api/src/a.rs".into(),
                reason: "changed".into(),
            },
        ];
        for error in &file_errors {
            assert!(
                file_read_error(error).is_some(),
                "expected per-file error to be captured: {error:?}"
            );
        }

        let fatal_errors = [
            DaemonError::Offline {
                machine: MachineId::new("ma_1"),
            },
            DaemonError::NoDefaultBranch {
                repo: "acme/api".into(),
            },
            DaemonError::MigrationNeeded {
                thing: MigrateThing::Daemon,
                machine: MachineId::new("ma_1"),
                request: None,
            },
            DaemonError::NotServedYet {
                what: "diffs".into(),
            },
            DaemonError::Scripted {
                operation: Operation::Diffs,
                message: "boom".into(),
            },
        ];
        for error in &fatal_errors {
            assert_eq!(
                file_read_error(error),
                None,
                "expected fatal error to abort read_diffs: {error:?}"
            );
        }
    }

    #[test]
    fn session_file_decodes_agent_state_current_and_previous_conversations() {
        let entry = |root: pb::file_location::Root, path: &str, folder: bool| pb::FileEntry {
            location: Some(pb::FileLocation {
                machine: None,
                root: Some(root),
                path: path.to_owned(),
            }),
            kind: if folder {
                pb::file_entry::Kind::Folder as i32
            } else {
                pb::file_entry::Kind::File as i32
            },
            ..pb::FileEntry::default()
        };

        let current = session_file(&entry(
            pb::file_location::Root::SessionAgentState(pb::SessionAgentState {
                session: Some(pb::SessionId { value: "s1".to_owned() }),
                conversation_id: String::new(),
            }),
            "plans/plan.md",
            false,
        ));
        assert_eq!(
            current,
            Some(SessionFile {
                repo: "@agent-state".to_owned(),
                path: "plans/plan.md".to_owned(),
                folder: false,
            })
        );

        let previous = session_file(&entry(
            pb::file_location::Root::SessionAgentState(pb::SessionAgentState {
                session: Some(pb::SessionId { value: "s1".to_owned() }),
                conversation_id: "conv-42".to_owned(),
            }),
            "subagents",
            true,
        ));
        assert_eq!(
            previous,
            Some(SessionFile {
                repo: "@agent-state:prev:conv-42".to_owned(),
                path: "subagents".to_owned(),
                folder: true,
            })
        );

        let folder = session_file(&entry(
            pb::file_location::Root::SessionFolder(pb::SessionFolder {
                session: Some(pb::SessionId { value: "s1".to_owned() }),
            }),
            "notes.md",
            false,
        ));
        assert_eq!(
            folder,
            Some(SessionFile {
                repo: String::new(),
                path: "notes.md".to_owned(),
                folder: false,
            })
        );

        let clone = session_file(&entry(
            pb::file_location::Root::SessionClone(pb::SessionClone {
                session: Some(pb::SessionId { value: "s1".to_owned() }),
                repo: repo_ref("acme", "api"),
            }),
            "src/lib.rs",
            false,
        ));
        assert_eq!(
            clone,
            Some(SessionFile {
                repo: "acme/api".to_owned(),
                path: "src/lib.rs".to_owned(),
                folder: false,
            })
        );
    }

    #[test]
    fn classify_and_resolve_session_file_roots_cover_all_conventions() {
        let session = SessionId::new("s1");
        let pb_session = Some(pb::SessionId {
            value: "s1".to_owned(),
        });
        let channel = Some(pb::ChannelId {
            value: "ch1".to_owned(),
        });
        let session_repos = vec![pb::SessionRepo {
            repo: repo_ref("acme", "api"),
            ..pb::SessionRepo::default()
        }];

        // Empty repo -> SessionFolder
        assert_eq!(
            classify_session_file_repo(&session, ""),
            SessionFileRootTarget::Direct(pb::file_location::Root::SessionFolder(
                pb::SessionFolder {
                    session: pb_session.clone(),
                }
            ))
        );

        // @agent-state -> SessionAgentState (empty conversation_id)
        assert_eq!(
            classify_session_file_repo(&session, "@agent-state"),
            SessionFileRootTarget::Direct(pb::file_location::Root::SessionAgentState(
                pb::SessionAgentState {
                    session: pb_session.clone(),
                    conversation_id: String::new(),
                }
            ))
        );

        // @agent-state:prev:<id> -> SessionAgentState (with conversation_id)
        assert_eq!(
            classify_session_file_repo(&session, "@agent-state:prev:conv-99"),
            SessionFileRootTarget::Direct(pb::file_location::Root::SessionAgentState(
                pb::SessionAgentState {
                    session: pb_session.clone(),
                    conversation_id: "conv-99".to_owned(),
                }
            ))
        );

        // owner/name -> SessionClone
        assert_eq!(
            classify_session_file_repo(&session, "acme/api"),
            SessionFileRootTarget::Direct(pb::file_location::Root::SessionClone(
                pb::SessionClone {
                    session: pb_session,
                    repo: repo_ref("acme", "api"),
                }
            ))
        );

        // shared -> ChannelShared
        let shared_target = classify_session_file_repo(&session, "shared");
        assert_eq!(shared_target, SessionFileRootTarget::ChannelShared);
        assert_eq!(
            resolve_channel_root(shared_target, channel.clone(), &session_repos, "read"),
            Ok(pb::file_location::Root::ChannelShared(pb::ChannelShared {
                channel: channel.clone(),
            }))
        );

        // main/<repo_name> -> MainClone with owner resolved from session_repos
        let main_short = classify_session_file_repo(&session, "main/api");
        assert_eq!(main_short, SessionFileRootTarget::MainClone("api".to_owned()));
        assert_eq!(
            resolve_channel_root(main_short, channel.clone(), &session_repos, "read"),
            Ok(pb::file_location::Root::MainClone(pb::MainClone {
                channel: channel.clone(),
                repo: repo_ref("acme", "api"),
            }))
        );

        // main/<owner>/<name> -> MainClone with explicit owner/name
        let main_full = classify_session_file_repo(&session, "main/other/web");
        assert_eq!(
            main_full,
            SessionFileRootTarget::MainClone("other/web".to_owned())
        );
        assert_eq!(
            resolve_channel_root(main_full, channel, &session_repos, "read"),
            Ok(pb::file_location::Root::MainClone(pb::MainClone {
                channel: Some(pb::ChannelId {
                    value: "ch1".to_owned(),
                }),
                repo: repo_ref("other", "web"),
            }))
        );

        // Missing channel returns Refused
        assert!(matches!(
            resolve_channel_root(SessionFileRootTarget::ChannelShared, None, &session_repos, "read"),
            Err(DaemonError::Refused { .. })
        ));
        assert!(matches!(
            resolve_channel_root(
                SessionFileRootTarget::MainClone("api".to_owned()),
                None,
                &session_repos,
                "read"
            ),
            Err(DaemonError::Refused { .. })
        ));
    }
}

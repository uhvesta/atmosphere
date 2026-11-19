//! A2, PR-2, PR-4..PR-6: starting a review of one or more pull requests as
//! one session of the PR reviews channel.
//!
//! Each pull request is looked up (`resolve`), the channel is made on first
//! use and gains the repositories as pull requests come, and the session
//! service starts one session whose clones are made on each pull request's
//! own head branch name at the head GitHub keeps for it
//! (`refs/pull/<n>/head`, so a fork's repository is never asked). The
//! binding (pull requests, the head each clone was made at, push allowed)
//! is recorded by that start, before the agent launches. The agent is told
//! each pull request and the user's context, and then waits: it never
//! reviews on its own (D59).

use humata2_daemon_ports::Caller;
use humata2_grpc::humata::v1 as pb;
use humata2_rpc_kit::{blocking, chain, internal, invalid_field, missing};
use humata2_store::{ChannelRecord, ChannelRepo, MainClone, ReviewPr, Store, StoreError};
use tonic::Status;

use super::PullRequests;
use crate::ports::{ReviewStarter, store_error};

/// The folder of the PR reviews channel under the channels folder, which is
/// also how a channel is known as it (`Channel.is_pr_reviews`).
pub const PR_REVIEWS_FOLDER: &str = "pr-reviews";

/// What kept a review from starting before the session service took over.
#[derive(Debug, thiserror::Error)]
enum StartReviewError {
    #[error("the request names no pull request to review")]
    NoPullRequests,
    #[error("the review names {repo} twice, and a review keeps one clone of each repository")]
    RepoTwice { repo: String },
    #[error("{asked} and {held} share the name {name:?}, and the PR reviews channel keeps one clone folder per repository name")]
    SameName {
        asked: String,
        held: String,
        name: String,
    },
    #[error("couldn't {doing}")]
    Store {
        doing: &'static str,
        #[source]
        source: StoreError,
    },
    #[error("couldn't make the PR reviews channel")]
    Channel {
        #[source]
        source: Status,
    },
}

impl StartReviewError {
    fn into_status(self, machine: &str) -> Status {
        match self {
            Self::NoPullRequests => missing("prs"),
            Self::RepoTwice { .. } => invalid_field(
                "prs",
                pb::invalid_field::Problem::NotAllowedHere,
                chain(&self),
                "start one review per pull request of that repository, or review them one after the other",
            ),
            Self::SameName { .. } => invalid_field(
                "prs",
                pb::invalid_field::Problem::NotAllowedHere,
                chain(&self),
                "review the pull requests of the two repositories in separate reviews after one of them is out of the channel; Humata can't keep both yet",
            ),
            Self::Channel { source } => source,
            Self::Store { doing, source } => store_error(source, machine, doing),
        }
    }
}

/// `owner/name`.
fn slug(owner: &str, name: &str) -> String {
    format!("{owner}/{name}")
}

/// The PR reviews channel with `wanted` among its repositories. Made on first
/// use; a repository is added the first time a pull request of it is reviewed.
/// Blocking.
fn prepare_channel(
    store: &Store,
    starter: &dyn ReviewStarter,
    machine: &str,
    default_agent: &str,
    wanted: &[(String, String)],
) -> Result<ChannelRecord, StartReviewError> {
    let store_failed =
        |doing: &'static str| move |source: StoreError| StartReviewError::Store { doing, source };
    let existing = store
        .channels()
        .map_err(store_failed("list the channels"))?
        .into_iter()
        .find(|channel| channel.folder == PR_REVIEWS_FOLDER);
    let mut channel = match existing {
        Some(channel) => channel,
        None => starter
            .create_pr_reviews_channel(machine, default_agent)
            .map_err(|source| StartReviewError::Channel { source })?,
    };
    let mut changed = false;
    for (owner, name) in wanted {
        if let Some(other) = channel
            .repos
            .iter()
            .find(|repo| repo.name == *name && repo.owner != *owner)
        {
            return Err(StartReviewError::SameName {
                asked: slug(owner, name),
                held: slug(&other.owner, &other.name),
                name: name.clone(),
            });
        }
        match channel
            .repos
            .iter_mut()
            .find(|repo| repo.owner == *owner && repo.name == *name)
        {
            Some(repo) => {
                // A clone that failed is tried again by this start.
                if matches!(repo.main, MainClone::Failed { .. }) {
                    repo.main = MainClone::Cloning;
                    changed = true;
                }
            }
            None => {
                channel.repos.push(ChannelRepo {
                    owner: owner.clone(),
                    name: name.clone(),
                    main: MainClone::Cloning,
                });
                changed = true;
            }
        }
    }
    if changed {
        channel.revision += 1;
        store
            .put_channel(&channel)
            .map_err(store_failed("save the PR reviews channel"))?;
        starter
            .clone_pr_reviews_mains(&channel.id)
            .map_err(|source| StartReviewError::Channel { source })?;
    }
    let channel = store
        .channel(&channel.id)
        .map_err(store_failed("read the PR reviews channel"))?
        .unwrap_or(channel);
    Ok(channel)
}

impl PullRequests {
    /// A2: starts the review `request` describes and answers with the
    /// operation that starts its session. This does not check who asks:
    /// the RPC requires the user, and an agent comes here only through the
    /// MCP tool, after its Ask was answered yes.
    pub(super) async fn start_review_of(
        &self,
        caller: &Caller,
        request: pb::StartReviewRequest,
    ) -> Result<pb::Operation, Status> {
        if request.prs.is_empty() {
            return Err(StartReviewError::NoPullRequests.into_status(&self.inner.machine));
        }
        let mut infos = Vec::new();
        for pr in &request.prs {
            let info = self
                .resolve(pb::ResolvePullRequestRequest {
                    r#ref: Some(pb::resolve_pull_request_request::Ref::Pr(pr.clone())),
                })
                .await?;
            infos.push(info);
        }
        let mut prs = Vec::new();
        for (asked, info) in request.prs.iter().zip(&infos) {
            let repo = asked.repo.clone().unwrap_or_default();
            if prs
                .iter()
                .any(|held: &ReviewPr| held.owner == repo.owner && held.name == repo.name)
            {
                return Err(StartReviewError::RepoTwice {
                    repo: slug(&repo.owner, &repo.name),
                }
                .into_status(&self.inner.machine));
            }
            prs.push(ReviewPr {
                owner: repo.owner,
                name: repo.name,
                number: asked.number,
                base_branch: info.base_branch.clone(),
                head_branch: info.head_branch.clone(),
                // The start records the head each clone really got.
                head_sha: info.head_sha.clone(),
                checked_sha: info.head_sha.clone(),
                state: info.state,
                submit_started_unix_ms: None,
            });
        }
        let wanted: Vec<(String, String)> = prs
            .iter()
            .map(|pr| (pr.owner.clone(), pr.name.clone()))
            .collect();

        let _one_at_a_time = self.inner.channel.lock().await;
        let (store, starter, machine, agent) = (
            self.inner.store.clone(),
            self.inner.starter.clone(),
            self.inner.machine.clone(),
            self.inner.default_agent.clone(),
        );
        let channel = blocking(move || {
            prepare_channel(&store, starter.as_ref(), &machine, &agent, &wanted)
        })
        .await?
        .map_err(|error| error.into_status(&self.inner.machine))?;

        let sessions = self.inner.sessions.get().ok_or_else(|| {
            internal(
                "pull request reviews were asked for before the session service was ready",
                "",
            )
        })?;
        let started = sessions
            .begin_review(caller, channel, &request, prs, request.push_allowed)
            .await?;
        Ok(started.operation.unwrap_or_default())
    }
}

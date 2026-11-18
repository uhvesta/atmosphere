//! Pull request reviews on other machines (PR-4, PR-21; flow 26.17 step 2).
//!
//! The host asks GitHub through its own `gh`, keeps the review's records and
//! the PR reviews channel, and starts the review's session on the machine
//! like any session there (`remote.rs`): the host's record first, then the
//! same start on the machine, which this time carries each pull request's
//! branch and head (`StartSessionRequest.review`). The machine makes the
//! channel's clones itself from its own remote, fetching `refs/pull/<n>/head`
//! with no token, and records the review as the host did. What the host
//! needs to know of those clones afterwards it asks the machine
//! (`review/pull_requests/clone_git.rs`).
//!
//! Every branch here runs after the review service's own vetting of the
//! caller, because the machine honours a relayed call as the user's.

use std::sync::Arc;

use humata2_grpc::humata::v1 as pb;
use humata2_sessions::{ReviewStart, StartRequest};
use humata2_store::{ChannelRecord, ReviewPr, ReviewSessionRecord, StoreError};
use tonic::{Code, Status};

use super::remote_start::Asked;
use super::{COLS, Gated, ROWS, SessionSvc, clean_title, start_error};
use crate::channel_service::{blocking, failed, tell_channel};
use crate::identity::{self, Caller};
use crate::operations::Started;
use crate::remote_work::{HostOp, MachineOutcome, RemoteWork, channel_steps};
use crate::status;

#[derive(Debug, thiserror::Error)]
enum RemoteReviewError {
    #[error("{machine} was removed from Humata while the PR reviews channel was being set up on it")]
    MachineRemoved { machine: String },
    #[error("only the host starts a pull request review on a machine")]
    NotTheHost,
    #[error("the review's start has no pull request to clone")]
    NoPullRequests,
    #[error("the review's start was ended before it could be journaled")]
    Ended,
}

impl RemoteReviewError {
    fn status(&self) -> Status {
        match self {
            Self::MachineRemoved { .. } => failed(
                Code::FailedPrecondition,
                self,
                "add the machine again from the Machines page, or review on this Mac or another machine",
                None,
            ),
            Self::NotTheHost => failed(
                Code::PermissionDenied,
                self,
                "start the review from Humata on the host; it asks this machine itself",
                None,
            ),
            Self::NoPullRequests => failed(
                Code::InvalidArgument,
                self,
                "start the review from Humata on the host, which names its pull requests",
                None,
            ),
            Self::Ended => failed(
                Code::Internal,
                self,
                "start the review again",
                None,
            ),
        }
    }
}

/// The failure an operation records for `status`.
fn failure_of(status: &Status) -> pb::Error {
    status::error_of(status).unwrap_or_else(|| pb::Error {
        message: status.message().to_owned(),
        fix: String::new(),
        retry: None,
        kind: None,
    })
}

/// Ends the host's operation as failed with `status`, if it is still open.
fn fail(host: &HostOp, status: &Status) {
    if let Some(progress) = host.take() {
        progress.fail(failure_of(status));
    }
}

impl SessionSvc {
    /// The host's way to its machines, none on a machine's own daemon.
    pub(crate) fn remote_work(&self) -> Option<RemoteWork> {
        self.remote.clone()
    }

    /// The work the host does for the host-side start of a review on
    /// `machine` (the one the review service handed over after it asked
    /// GitHub and made the channel here). An offline machine answers at once
    /// and leaves nothing behind (CH-4); a machine that refuses the start takes
    /// the host's record back.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn begin_review_remote(
        &self,
        caller: &Caller,
        channel: ChannelRecord,
        request: &pb::StartReviewRequest,
        review: ReviewStart,
        title: String,
        first_message: String,
        machine: String,
        remote: RemoteWork,
    ) -> Result<pb::Operation, Status> {
        let agent = self.agent_for(request.agent.as_ref(), &channel)?;
        // SEAM: AGENTSET: refused before it is relayed when the kind is off.
        crate::agent_gate::enabled(&self.starter.agents, &agent)?;
        if let Some(operation) = self
            .operations
            .existing(caller, request.idempotency_key.as_ref())?
        {
            return Ok(operation);
        }
        remote.require_link(&machine).map_err(Status::from)?;

        let repo_refs: Vec<pb::RepoRef> = channel
            .repos
            .iter()
            .map(|repo| pb::RepoRef {
                owner: repo.owner.clone(),
                name: repo.name.clone(),
            })
            .collect();
        let mut steps = channel_steps(&machine, &repo_refs);
        steps.push(("write_record".to_owned(), "Saving the session".to_owned()));
        for pr in &review.prs {
            steps.push((format!("clone:{}", pr.name), format!("Cloning {}", pr.name)));
        }
        steps.push(("launch".to_owned(), "Starting the agent".to_owned()));
        steps.push(("verify".to_owned(), "Checking it started".to_owned()));
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Channel(pb::ChannelId {
                value: channel.id.clone(),
            })),
        };
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::StartSession,
            target,
            &steps,
            false,
            request.idempotency_key.clone(),
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(operation),
        };
        let host = Arc::new(HostOp::new(progress));

        // The machine gets the PR reviews channel, with the same id, and the
        // main clones its sessions' clones are made from.
        let create = pb::CreateChannelRequest {
            idempotency_key: None,
            name: channel.name.clone(),
            repos: repo_refs,
            machines: Vec::new(),
        };
        match remote.create_channel_on(&machine, &channel.id, create, host.clone()).await {
            MachineOutcome::Ready => {}
            MachineOutcome::Failed(error) => {
                let status = status::failure(Code::FailedPrecondition, error);
                fail(&host, &status);
                return Err(status);
            }
            MachineOutcome::Gone => {
                let status = RemoteReviewError::MachineRemoved { machine }.status();
                fail(&host, &status);
                return Err(status);
            }
        }
        if let Err(status) = self.add_machine_to(&channel, &machine, caller).await {
            fail(&host, &status);
            return Err(status);
        }

        let id = format!("se_{}", uuid::Uuid::new_v4().simple());
        let asked = pb::StartSessionRequest {
            channel: Some(pb::ChannelId {
                value: channel.id.clone(),
            }),
            title: title.clone(),
            agent: Some(pb::AgentId { value: agent.clone() }),
            model: request.model.clone(),
            repos: review
                .prs
                .iter()
                .map(|pr| pb::RepoChoice {
                    repo: Some(pb::RepoRef {
                        owner: pr.owner.clone(),
                        name: pr.name.clone(),
                    }),
                    include: true,
                })
                .collect(),
            first_message: first_message.clone(),
            review: Some(pb::ReviewFrom {
                prs: review
                    .prs
                    .iter()
                    .map(|pr| pb::review_from::Pr {
                        repo: Some(pb::RepoRef {
                            owner: pr.owner.clone(),
                            name: pr.name.clone(),
                        }),
                        number: pr.number,
                        base_branch: pr.base_branch.clone(),
                        head_branch: pr.head_branch.clone(),
                        head_sha: pr.head_sha.clone(),
                        state: pr.state,
                    })
                    .collect(),
                push_allowed: review.push_allowed,
            }),
            ..pb::StartSessionRequest::default()
        };
        // D432: the start's journal goes first, so a power cut anywhere from
        // the host's record to the machine's answer leaves a start to ask
        // again, never a record on the host that nothing runs.
        let Some(journal) = host.with_progress(|progress| Asked::new(progress, &id, &machine, &asked)) else {
            let status = RemoteReviewError::Ended.status();
            return Err(status);
        };
        if let Err(status) = self.keep_asked(&journal).await {
            host.set_step("write_record", pb::StepState::Failed, Some(status.message().to_owned()));
            fail(&host, &status);
            return Err(status);
        }
        let start = StartRequest {
            id: Some(id.clone()),
            channel: channel.id.clone(),
            machine: machine.clone(),
            title,
            agent: Some(agent),
            model: Some(request.model.clone()).filter(|model| !model.is_empty()),
            repos: Some(
                review
                    .prs
                    .iter()
                    .map(|pr| (pr.owner.clone(), pr.name.clone()))
                    .collect(),
            ),
            first_message: Some(first_message).filter(|message| !message.is_empty()),
            branch: None,
            operation: host
                .operation()
                .and_then(|operation| operation.id)
                .map(|operation| operation.value),
            cols: COLS,
            rows: ROWS,
        };
        if let Err(error) = self.starter.place_session(&start).await {
            self.drop_asked(&id).await;
            let (code, failure) = start_error(&error);
            host.set_step("write_record", pb::StepState::Failed, Some(status::chain(&error)));
            let status = status::failure(code, failure);
            fail(&host, &status);
            return Err(status);
        }
        // The review's records on the host, with each clone on the pull
        // request's own branch as the machine will make it (PR-6).
        if let Err(status) = self.record_review(&id, &review).await {
            self.remove_placed_then_asked(&id).await;
            fail(&host, &status);
            return Err(status);
        }
        host.set_step("write_record", pb::StepState::Done, None);

        match remote
            .start_session_answered(&machine, &id, asked, host.clone(), Some(self.answered_hook(&id)))
            .await
        {
            Ok(response) => {
                self.tell
                    .session(&id, pb::Change::Created, Some(identity::actor(caller)))
                    .await;
                Ok(response
                    .operation
                    .or_else(|| host.operation())
                    .unwrap_or_default())
            }
            Err(error) => {
                let status = Status::from(error);
                fail(&host, &status);
                self.remove_placed_then_asked(&id).await;
                Err(status)
            }
        }
    }

    /// Adds `machine` to the PR reviews channel's machines, once it has the
    /// channel there.
    pub(super) async fn add_machine_to(&self, channel: &ChannelRecord, machine: &str, caller: &Caller) -> Result<(), Status> {
        let (store, feed, by) = (
            self.starter.store.clone(),
            self.tell.feed.clone(),
            identity::actor(caller),
        );
        let (id, machine, held) = (channel.id.clone(), machine.to_owned(), self.machine.clone());
        let name = channel.id.clone();
        blocking(move || -> Result<(), StoreError> {
            let Some(mut current) = store.channel(&id)? else {
                return Ok(());
            };
            if !current.machines.contains(&machine) {
                current.machines.push(machine);
                current.revision += 1;
                store.put_channel(&current)?;
                tell_channel(&feed, &store, &id, pb::Change::Updated, Some(by));
            }
            Ok(())
        })
        .await?
        .map_err(|error| status::store(error, &held, &format!("add the machine to channel {name}")))
    }

    /// Saves the host's record of the review: the pull requests as GitHub
    /// answered them, and each clone on its pull request's head branch.
    async fn record_review(&self, session: &str, review: &ReviewStart) -> Result<(), Status> {
        let record = ReviewSessionRecord {
            session: session.to_owned(),
            push_allowed: review.push_allowed,
            prs: review.prs.clone(),
        };
        let (store, id) = (self.starter.store.clone(), session.to_owned());
        let held = self.machine.clone();
        blocking(move || -> Result<(), StoreError> {
            store.put_review_session(&record)?;
            if let Some(mut detail) = store.session_detail(&id)? {
                for repo in &mut detail.repos {
                    if let Some(pr) = record
                        .prs
                        .iter()
                        .find(|pr| pr.owner == repo.owner && pr.name == repo.name)
                    {
                        repo.branch = pr.head_branch.clone();
                    }
                }
                store.put_session_detail(&detail)?;
            }
            Ok(())
        })
        .await?
        .map_err(|error| status::store(error, &held, "save the review's pull requests"))
    }

    /// The machine's side of the host's relayed review start: the same start
    /// a review on the host makes, under the id the host chose, from the
    /// pull requests the host carried.
    pub(super) async fn begin_review_here(
        &self,
        caller: &Caller,
        request: pb::StartSessionRequest,
        review: pb::ReviewFrom,
        assigned: Option<String>,
        channel: ChannelRecord,
    ) -> Result<Gated<pb::StartSessionResponse>, Status> {
        let Some(id) = assigned else {
            return Err(RemoteReviewError::NotTheHost.status());
        };
        if let Some(known) = self.known_start(&id).await? {
            return Ok(Gated::Done(known));
        }
        let mut prs = Vec::new();
        for (n, pr) in review.prs.iter().enumerate() {
            let repo = pr
                .repo
                .clone()
                .ok_or_else(|| status::missing(&format!("review.prs[{n}].repo")))?;
            prs.push(ReviewPr {
                owner: repo.owner,
                name: repo.name,
                number: pr.number,
                base_branch: pr.base_branch.clone(),
                head_branch: pr.head_branch.clone(),
                // The start records the head each clone really got.
                head_sha: pr.head_sha.clone(),
                checked_sha: pr.head_sha.clone(),
                state: pr.state,
                submit_started_unix_ms: None,
            });
        }
        if prs.is_empty() {
            return Err(RemoteReviewError::NoPullRequests.status());
        }
        let cleaned = clean_title(&request.title)?;
        let title = if cleaned.is_empty() { "Session".to_owned() } else { cleaned };
        let asked = pb::StartReviewRequest {
            agent: request.agent.clone(),
            model: request.model.clone(),
            push_allowed: review.push_allowed,
            ..pb::StartReviewRequest::default()
        };
        let operation = self
            .begin_review(
                caller,
                channel,
                &asked,
                ReviewStart {
                    prs,
                    push_allowed: review.push_allowed,
                },
                title,
                request.first_message,
                Some(id),
            )
            .await?;
        Ok(Gated::Done(self.started_by(operation).await?))
    }
}

//! Pull request reviews on other machines (PR-4, PR-21; flow 26.17 step 2).

use humata2_daemon_ports::Caller;
use humata2_sessions::ReviewStart;
use humata2_store::{ChannelRecord, ReviewPr, ReviewSessionRecord, StoreError};
use humata2_svc_sessions_common::pb;
use tonic::{Code, Status};

use crate::SessionTransferSvc;
use crate::ports::{blocking, chain, clean_title, failed, internal, missing};

#[derive(Debug, thiserror::Error)]
pub enum RemoteReviewError {
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
    pub fn status(&self) -> Status {
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
            Self::Ended => failed(Code::Internal, self, "start the review again", None),
        }
    }
}

/// The failure an operation records for `status`.
pub fn failure_of(status: &Status) -> pb::Error {
    pb::Error {
        message: status.message().to_owned(),
        fix: String::new(),
        retry: None,
        kind: None,
    }
}

impl SessionTransferSvc {
    /// The work the host does for the host-side start of a review on `machine`.
    #[allow(clippy::too_many_arguments)]
    pub async fn begin_review_remote(
        &self,
        _caller: &Caller,
        _channel: ChannelRecord,
        _request: &humata2_grpc_review::humata::v1::StartReviewRequest,
        _review: ReviewStart,
        _title: String,
        _first_message: String,
        machine: String,
    ) -> Result<pb::Operation, Status> {
        Err(self
            .env
            .not_wired(&format!("begin_review_remote on {machine}")))
    }

    /// Adds `machine` to the PR reviews channel's machines, once it has the
    /// channel there.
    pub async fn add_machine_to(
        &self,
        channel: &ChannelRecord,
        machine: &str,
        _caller: &Caller,
    ) -> Result<(), Status> {
        let store = self.core.store.clone();
        let (id, machine) = (channel.id.clone(), machine.to_owned());
        let name = channel.id.clone();
        blocking(move || -> Result<(), StoreError> {
            let Some(mut current) = store.channel(&id)? else {
                return Ok(());
            };
            if !current.machines.contains(&machine) {
                current.machines.push(machine);
                current.revision += 1;
                store.put_channel(&current)?;
            }
            Ok(())
        })
        .await?
        .map_err(|error| {
            internal(
                format!("couldn't add the machine to channel {name}: {}", chain(&error)),
                "",
            )
        })
    }

    /// Saves the host's record of the review: the pull requests as GitHub
    /// answered them, and each clone on its pull request's head branch.
    pub async fn record_review(&self, session: &str, review: &ReviewStart) -> Result<(), Status> {
        let record = ReviewSessionRecord {
            session: session.to_owned(),
            push_allowed: review.push_allowed,
            prs: review.prs.clone(),
        };
        let (store, id) = (self.core.store.clone(), session.to_owned());
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
        .map_err(|error| {
            internal(
                format!("couldn't save the review's pull requests: {}", chain(&error)),
                "",
            )
        })
    }

    /// The machine's side of the host's relayed review start.
    pub async fn begin_review_here(
        &self,
        _caller: &Caller,
        request: pb::StartSessionRequest,
        review: pb::ReviewFrom,
        assigned: Option<String>,
        _channel: ChannelRecord,
    ) -> Result<pb::StartSessionResponse, Status> {
        let Some(_id) = assigned else {
            return Err(RemoteReviewError::NotTheHost.status());
        };
        let mut prs = Vec::new();
        for (n, pr) in review.prs.iter().enumerate() {
            let repo = pr
                .repo
                .clone()
                .ok_or_else(|| missing(&format!("review.prs[{n}].repo")))?;
            prs.push(ReviewPr {
                owner: repo.owner,
                name: repo.name,
                number: pr.number,
                base_branch: pr.base_branch.clone(),
                head_branch: pr.head_branch.clone(),
                head_sha: pr.head_sha.clone(),
                checked_sha: pr.head_sha.clone(),
                state: pr.state,
                submit_started_unix_ms: None,
            });
        }
        if prs.is_empty() {
            return Err(RemoteReviewError::NoPullRequests.status());
        }
        let _cleaned = clean_title(&request.title)?;
        Err(self.env.not_wired("begin_review_here"))
    }
}

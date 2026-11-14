//! The daemon, as `humata` and `humata mcp` reach it: an admin action, such
//! as adding or deleting a task, waits for the person to approve it in the
//! Humata app, unless a grant they gave covers it.

use std::time::Duration;

use humata_wire::{ClientError, pb};

/// How long one wait asks the daemon to wait; the daemon waits at most
/// five minutes.
const ONE_WAIT: Duration = Duration::from_secs(300);

/// How long to wait for the person to approve an action.
#[derive(Clone, Copy)]
pub enum Wait {
    /// Until they decide, as a person at the command line would.
    UntilDecided,
    /// This long, then say it still waits.
    For(Duration),
}

/// The daemon's client, waiting for approvals.
pub struct Client {
    inner: humata_wire::Client,
    wait: Wait,
    /// Whether to say on stderr that an action waits, for a person
    /// watching the command line.
    say_waiting: bool,
}

impl Client {
    pub fn new(inner: humata_wire::Client, wait: Wait, say_waiting: bool) -> Self {
        Self {
            inner,
            wait,
            say_waiting,
        }
    }

    /// Whether the connection to the daemon is still open.
    pub fn is_connected(&self) -> bool {
        self.inner.is_connected()
    }

    /// Sends a request answered with one response, as
    /// [`humata_wire::Client::call`], waiting for the person if it needs
    /// their approval.
    pub async fn call(
        &self,
        request: pb::request::Kind,
    ) -> Result<pb::response::Kind, ClientError> {
        match self.inner.call(request).await? {
            pb::response::Kind::NeedsApproval(needs) => {
                let approval = needs.approval.ok_or_else(|| ClientError::Unexpected {
                    expected: "the approval asked for",
                    got: "none".to_owned(),
                })?;
                if self.say_waiting {
                    eprintln!(
                        "humata: {} waits for your approval in Humata's inbox (approval {})",
                        approval.summary, approval.id
                    );
                }
                let decided = self.wait_for(&approval.id, self.wait).await?;
                outcome(*decided)
            }
            answer => Ok(answer),
        }
    }

    /// Sends a request answered with a response and a stream. No admin
    /// action streams, so none waits for approval.
    pub async fn stream(
        &self,
        request: pb::request::Kind,
    ) -> Result<(pb::response::Kind, humata_wire::Stream), ClientError> {
        self.inner.stream(request).await
    }

    /// Approval `id`, once the person decides it or `wait` runs out.
    pub async fn wait_for(&self, id: &str, wait: Wait) -> Result<Box<pb::Approval>, ClientError> {
        let until = match wait {
            Wait::UntilDecided => None,
            Wait::For(wait) => Some(tokio::time::Instant::now() + wait),
        };
        loop {
            let left = until.map_or(ONE_WAIT, |until| {
                until.saturating_duration_since(tokio::time::Instant::now())
            });
            let request = pb::request::Kind::WaitForApproval(pb::WaitForApproval {
                id: id.to_owned(),
                wait_seconds: u32::try_from(left.min(ONE_WAIT).as_secs()).unwrap_or(u32::MAX),
            });
            let approval = match self.inner.call(request).await? {
                pb::response::Kind::ApprovalStatus(status) => status.approval,
                other => {
                    return Err(ClientError::Unexpected {
                        expected: "the approval's status",
                        got: format!("{other:?}"),
                    });
                }
            }
            .ok_or_else(|| ClientError::Unexpected {
                expected: "the approval",
                got: "none".to_owned(),
            })?;
            let out_of_time = until.is_some_and(|until| tokio::time::Instant::now() >= until);
            // Approved but not yet answered is still running.
            let settled = match approval.state() {
                pb::ApprovalState::Pending => false,
                pb::ApprovalState::Approved => approval.result.is_some(),
                _ => true,
            };
            if settled || out_of_time {
                return Ok(approval);
            }
        }
    }
}

/// What a decided approval answered: what the action answered if it was
/// approved, else why it didn't run.
pub fn outcome(approval: pb::Approval) -> Result<pb::response::Kind, ClientError> {
    match approval.state() {
        pb::ApprovalState::Approved => match approval.result.and_then(|result| result.kind) {
            Some(pb::response::Kind::Error(refused)) => Err(ClientError::Refused {
                code: refused.code(),
                message: refused.message,
            }),
            Some(answer) => Ok(answer),
            None => Err(ClientError::StillRunning {
                id: approval.id,
                summary: approval.summary,
            }),
        },
        pb::ApprovalState::Denied => Err(ClientError::Denied {
            id: approval.id,
            summary: approval.summary,
        }),
        pb::ApprovalState::Pending | pb::ApprovalState::Unspecified => {
            Err(ClientError::AwaitingApproval {
                id: approval.id,
                summary: approval.summary,
            })
        }
    }
}

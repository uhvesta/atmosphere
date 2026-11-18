//! Add a repo to a running session on the real daemon, over
//! `SessionService.AddRepoToSession` (CH-21). The daemon clones the repo on
//! the session's machine, adds it to the channel when the channel lacks it
//! and records the use in the repo library, telling the agent nothing (D743);
//! this client sends `owner/name` and waits for the operation.

use humata2_clients::DaemonError;
use humata2_clients::model::SessionId;
use humata2_grpc::humata::v1 as pb;

use crate::connection::{Shared, fresh_key};
use crate::daemon::bad_repo;
use crate::map;

impl Shared {
    /// Adds `repo` (`owner/name`) to `session` and waits until its clone is
    /// made or has failed, which is refused with the daemon's words.
    pub(crate) async fn add_repo_to_session(
        &self,
        session: SessionId,
        repo: String,
    ) -> Result<(), DaemonError> {
        const WHAT: &str = "Adding the repo to the session";
        let repo = map::parse_repo(&repo).ok_or_else(|| bad_repo(&repo))?;
        let operation = self
            .sessions()
            .add_repo_to_session(pb::AddRepoToSessionRequest {
                idempotency_key: fresh_key(),
                id: Some(pb::SessionId {
                    value: session.as_str().to_owned(),
                }),
                repo: Some(repo),
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        self.finish(operation, WHAT).await.map(|_| ())
    }
}

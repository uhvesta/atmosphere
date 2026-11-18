//! Add a GitHub repo to a channel on the real daemon, over
//! `ChannelService.AddRepo` (CH-6, CH-18, CH-19). The daemon makes the main
//! clone under the channel by its root rules and records the add in the repo
//! library; this client sends `owner/name` and waits for the operation.

use humata2_clients::DaemonError;
use humata2_clients::model::ChannelId;
use humata2_grpc::humata::v1 as pb;

use crate::connection::{Shared, fresh_key};
use crate::daemon::bad_repo;
use crate::map;

impl Shared {
    /// Adds `repo` (`owner/name`) to `channel` and waits until its clone is
    /// made or has failed, which is refused with the daemon's words.
    pub(crate) async fn add_repo_to_channel(
        &self,
        channel: ChannelId,
        repo: String,
    ) -> Result<(), DaemonError> {
        const WHAT: &str = "Adding the repo to the channel";
        let repo = map::parse_repo(&repo).ok_or_else(|| bad_repo(&repo))?;
        let operation = self
            .channels()
            .add_repo(pb::AddRepoRequest {
                idempotency_key: fresh_key(),
                channel: Some(pb::ChannelId {
                    value: channel.as_str().to_owned(),
                }),
                repo: Some(repo),
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        self.finish(operation, WHAT).await.map(|_| ())
    }
}

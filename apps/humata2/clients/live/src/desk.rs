//! [`DeskClient`] for the live daemon. The daemon serves no Desk sessions
//! yet, so every call answers [`DaemonError::NotServedYet`] without touching
//! the network. The daemon lane adds the Desk service later; each method
//! then calls it instead.

use humata2_clients::desk::{
    Access, CleanUp, CleanUpRow, DeskClient, Document, NewDeskSession,
};
use humata2_clients::model::{ChannelId, MachineId, SessionId};
use humata2_clients::{DaemonError, Reply};

use crate::connection::{LiveDaemon, not_in_this_release};

/// An answer that is not in this release.
fn later<T: Send + 'static>(what: &'static str) -> Reply<T> {
    Box::pin(async move { Err::<T, DaemonError>(not_in_this_release(what)) })
}

impl DeskClient for LiveDaemon {
    fn start(&self, _new: NewDeskSession) -> Reply<SessionId> {
        later("Starting a desk session")
    }

    fn close(&self, _session: &SessionId) -> Reply<()> {
        later("Closing a desk session")
    }

    fn reopen(&self, _session: &SessionId) -> Reply<()> {
        later("Reopening a desk session")
    }

    fn keep(&self, _session: &SessionId) -> Reply<()> {
        later("Keeping a desk session's folder")
    }

    fn clean_up_list(&self) -> Reply<Vec<CleanUpRow>> {
        later("Listing closed desk sessions")
    }

    fn clean_up(&self, _shown: Vec<CleanUpRow>, _delete: Vec<SessionId>) -> Reply<CleanUp> {
        later("Cleaning up closed desk sessions")
    }

    fn copy_document(
        &self,
        _document: &Document,
        _channel: &ChannelId,
        _machine: Option<&MachineId>,
    ) -> Reply<()> {
        later("Copying a document to a channel")
    }

    fn add_repo(&self, _session: &SessionId, _repo: &str) -> Reply<()> {
        later("Adding a repository to a desk session")
    }

    fn remove_repo(&self, _session: &SessionId, _repo: &str) -> Reply<()> {
        later("Removing a repository from a desk session")
    }

    fn move_to(&self, _session: &SessionId, _machine: &MachineId) -> Reply<()> {
        later("Moving a desk session to another machine")
    }

    fn set_access(&self, _session: &SessionId, _access: Access) -> Reply<()> {
        later("Changing a desk session's access")
    }
}

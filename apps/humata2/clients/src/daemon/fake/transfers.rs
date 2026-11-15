use super::FakeDaemon;
use crate::daemon::{DaemonError, Event, Reply};
use crate::model::{TransferId, TransferState};

pub(super) fn retry_transfer(daemon: &FakeDaemon, transfer: &TransferId) -> Reply<()> {
    let id = transfer.clone();
    daemon.reply(None, format!("retry {transfer}"), move |inner| {
        let found = inner
            .workspace
            .transfers
            .iter_mut()
            .find(|found| found.id == id)
            .ok_or_else(|| DaemonError::NotFound {
                what: format!("Transfer {id}"),
            })?;
        found.state = TransferState::Copying(0);
        let transfer = found.clone();
        inner.emit(Event::Transfer(transfer));
        Ok(())
    })
}

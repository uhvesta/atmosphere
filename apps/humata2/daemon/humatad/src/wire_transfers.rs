//! Adapters wiring `humata2_svc_transfers` port traits to `humatad`'s concrete
//! `Guard`, `Inbox`, `Bin`, and `Links`.

use std::path::PathBuf;
use std::sync::Arc;

use humata2_access::{AccessRequest, Call};
use humata2_daemon_ports::{BoardAsk, Caller, Run};
use humata2_rpc_kit::compression::Compression;
use humata2_sessions::Starter;
use humata2_svc_files::Roots;
use humata2_svc_transfers::ports::{
    Checked, LinkProvider, LinkState, RelayError, TransferGuardPort, TransferInboxPort,
    TrashBinPort,
};
use humata2_svc_transfers::{SessionCarrySvc, TransferEndSvc, TransferSvc};
use tokio::sync::broadcast;
use tonic::Status;
use tonic::transport::Channel;

use crate::guard::Guard;
use crate::host_relay::Links;
use crate::inbox::Inbox;
use crate::trash_service::Bin;

pub(crate) struct TransferGuardAdapter(pub(crate) Arc<Guard>);

impl TransferGuardPort for TransferGuardAdapter {
    fn check(&self, caller: &Caller, call: &Call) -> Result<Checked, Status> {
        match self.0.check(caller, call)? {
            crate::guard::Checked::Allowed => Ok(Checked::Allowed),
            crate::guard::Checked::Ask(req) => Ok(Checked::Ask(req)),
        }
    }

    fn require(&self, caller: &Caller, call: &Call) -> Result<(), Status> {
        self.0.require(caller, call)
    }
}

pub(crate) struct TransferInboxAdapter(pub(crate) Arc<Inbox>);

#[tonic::async_trait]
impl TransferInboxPort for TransferInboxAdapter {
    async fn ask(
        &self,
        caller: &Caller,
        asked: AccessRequest,
        operation: &str,
        run: Option<Run>,
        _board: Option<BoardAsk>,
    ) -> Result<(), Status> {
        self.0.ask_read(caller, asked, operation, run).await
    }
}

pub(crate) struct TrashBinAdapter(pub(crate) Arc<Bin>);

impl TrashBinPort for TrashBinAdapter {
    fn get(&self) -> Result<Arc<humata2_transfers::Trash>, Status> {
        self.0.get()
    }
}

pub(crate) struct LinksAdapter(pub(crate) Links);

impl LinkProvider for LinksAdapter {
    fn channel(&self, machine: &str) -> Result<Channel, RelayError> {
        self.0.channel(machine).map_err(|_| RelayError::Offline {
            machine: machine.to_owned(),
        })
    }

    fn watch_all(&self) -> broadcast::Receiver<(String, LinkState)> {
        let (_tx, rx) = broadcast::channel(16);
        rx
    }

    fn up_machines(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Construct `humata2_svc_transfers::SessionCarrySvc` wired to `humatad`.
pub(crate) fn carry_svc(
    starter: Starter,
    bin: Arc<Bin>,
    is_host: bool,
) -> Arc<SessionCarrySvc> {
    SessionCarrySvc::new(starter, Arc::new(TrashBinAdapter(bin)), is_host)
}

/// Construct `humata2_svc_transfers::TransferEndSvc` wired to `humatad`.
pub(crate) fn transfer_end_svc(
    roots: Arc<Roots>,
    guard: Arc<Guard>,
    bin: Arc<Bin>,
    machine: String,
) -> TransferEndSvc {
    TransferEndSvc::new(
        roots,
        Arc::new(TransferGuardAdapter(guard)),
        Arc::new(TrashBinAdapter(bin)),
        machine,
    )
}

/// Construct `humata2_svc_transfers::TransferSvc` wired to `humatad`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn transfer_svc(
    roots: Arc<Roots>,
    guard: Arc<Guard>,
    bin: Arc<Bin>,
    state: PathBuf,
    machine: String,
    compression: Arc<Compression>,
    inbox: Arc<Inbox>,
    links: Option<Links>,
) -> Arc<TransferSvc> {
    let svc = TransferSvc::new(
        roots,
        Arc::new(TransferGuardAdapter(guard)),
        Arc::new(TrashBinAdapter(bin)),
        state,
        machine,
        compression,
        Arc::new(TransferInboxAdapter(inbox)),
    );
    if let Some(links) = links {
        svc.use_links(Arc::new(LinksAdapter(links)));
    }
    svc
}

//! Concrete `humatad` adapters for `humata2_svc_upgrade::ports` (Wiring Wave 2).
//!
//! Bridges `humata2_svc_upgrade`'s narrow port traits onto `humatad`'s
//! concrete `Work`, `Inbox`, `Tokens`, `Feed`, `Started`, `turns`, and `Tell`
//! types, and exports constructors for `Holders` and `Watch`.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use humata2_daemon_ports::{PortError, Release};
use humata2_grpc::humata::v1 as pb;
use humata2_machines::{Builds as MachineBuilds, Machines};
use humata2_sessions::Starter;
use humata2_store::TabRecord;
use humata2_svc_upgrade::holders::{Builds, Deps, Holders, Reports};
use humata2_svc_upgrade::items::{Approver, Migrations, Watch};
use humata2_svc_upgrade::ports::{
    HolderEventsPort, HolderSessionPort, HolderStartedPort, HolderTokensPort, HolderTurnsPort,
    MigrateEdit, MigrateInboxPort, RemoteHostPort,
};
use tokio::sync::{broadcast, watch};
use tonic::Status;

use crate::events::Feed;
use crate::identity::Identities;
use crate::inbox::Inbox;
use crate::machine_service::Work;
use crate::restart::{Started, Tokens};
use crate::session_service::{Tell, turns, watch as agent_watch};

/// Adapts `crate::machine_service::Work` to [`RemoteHostPort`].
pub(crate) struct RemoteHostAdapter(pub(crate) Arc<Work>);

impl RemoteHostPort for RemoteHostAdapter {
    fn channel(&self, machine: &str) -> Result<tonic::transport::Channel, PortError> {
        self.0
            .links()
            .channel(machine)
            .map_err(|err| PortError::new(crate::status::chain(&err)))
    }

    fn machines(&self) -> &Machines {
        &self.0.machines
    }

    fn builds(&self) -> &(dyn MachineBuilds + Send + Sync) {
        self.0.builds.as_ref()
    }
}

/// Adapts `crate::inbox::Inbox` to [`MigrateInboxPort`].
pub(crate) struct MigrateInboxAdapter(pub(crate) Arc<Inbox>);

#[tonic::async_trait]
impl MigrateInboxPort for MigrateInboxAdapter {
    fn kept_migrates(&self) -> Vec<pb::InboxRequest> {
        self.0.kept_migrates()
    }

    fn migrating(&self, approver: Approver) {
        self.0.migrating(crate::wire_inbox::wrap_migrator(approver));
    }

    async fn post_migrate(&self, request: pb::InboxRequest) -> Result<(), Status> {
        self.0.post_migrate(request).await
    }

    async fn update_migrate(
        &self,
        id: &str,
        change: MigrateEdit,
    ) -> Result<Option<pb::InboxRequest>, Status> {
        let mut updated = None;
        let ok = self
            .0
            .update_migrate(id, |request| {
                change(request);
                updated = Some(request.clone());
            })
            .await?;
        Ok(if ok { updated } else { None })
    }

    async fn settle_migrate(&self, id: &str, done: String) -> Result<bool, Status> {
        self.0.settle_migrate(id, done).await
    }

    async fn withdraw_migrate(
        &self,
        id: &str,
        reason: pb::WithdrawReason,
    ) -> Result<bool, Status> {
        self.0.withdraw_migrate(id, reason).await
    }
}

/// Adapts `crate::restart::Tokens` to [`HolderTokensPort`].
pub(crate) struct HolderTokensAdapter(pub(crate) Tokens);

impl HolderTokensPort for HolderTokensAdapter {
    fn env(&self, tab: &TabRecord) -> Vec<(OsString, OsString)> {
        self.0.env(tab)
    }
}

/// Adapts `crate::events::Feed` to [`HolderEventsPort`].
pub(crate) struct HolderEventsAdapter(pub(crate) Arc<Feed>);

impl HolderEventsPort for HolderEventsAdapter {
    fn subscribe(&self) -> broadcast::Receiver<pb::Event> {
        let (_, mut src) = self.0.subscribe(None);
        let (tx, rx) = broadcast::channel(256);
        tokio::spawn(async move {
            loop {
                match src.recv().await {
                    Ok(published) => {
                        if tx.send((**published).clone()).is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        rx
    }
}

/// Adapts `crate::restart::Started` to [`HolderStartedPort`].
pub(crate) struct HolderStartedAdapter(pub(crate) Arc<Started>);

#[tonic::async_trait]
impl HolderStartedPort for HolderStartedAdapter {
    async fn wait(&self) {
        self.0.wait().await;
    }
}

/// Adapts `crate::session_service::turns` to [`HolderTurnsPort`].
pub(crate) struct HolderTurnsAdapter;

impl HolderTurnsPort for HolderTurnsAdapter {
    fn changes(&self) -> watch::Receiver<u64> {
        turns::changes()
    }

    fn running(&self, session: &str, holder_pid: u32) -> pb::SessionStatus {
        turns::running(session, holder_pid)
    }
}

/// Adapts `crate::session_service::Tell` + `agent_watch` to [`HolderSessionPort`].
pub(crate) struct HolderSessionAdapter(pub(crate) Tell);

#[tonic::async_trait]
impl HolderSessionPort for HolderSessionAdapter {
    async fn publish_session(&self, session: &str, change: pb::Change, by: Option<pb::Actor>) {
        self.0.session(session, change, by).await;
    }

    fn watch_agent(&self, session: String) {
        agent_watch::agent(self.0.clone(), session);
    }
}

/// Wraps `Work` as a [`RemoteHostPort`] for `humata2_svc_upgrade::remote::Remote::of_host`.
pub(crate) fn remote_host(work: Arc<Work>) -> Arc<dyn RemoteHostPort> {
    Arc::new(RemoteHostAdapter(work))
}

/// Wraps `Inbox` as a [`MigrateInboxPort`] for `humata2_svc_upgrade::items::Migrations::new`.
pub(crate) fn migrate_inbox(inbox: Arc<Inbox>) -> Arc<dyn MigrateInboxPort> {
    Arc::new(MigrateInboxAdapter(inbox))
}

/// Wraps `Started` as a [`HolderStartedPort`] for `Holders::start`.
pub(crate) fn started(started: Arc<Started>) -> Arc<dyn HolderStartedPort> {
    Arc::new(HolderStartedAdapter(started))
}

/// Constructs [`Holders`] wired to `humatad`'s concrete services.
#[allow(clippy::too_many_arguments)]
pub(crate) fn holders(
    machine: String,
    release: Release,
    builds: Arc<Builds>,
    migrations: Migrations,
    starter: Starter,
    identities: Arc<Identities>,
    socket: PathBuf,
    tell: Tell,
    feed: Arc<Feed>,
    report: Option<Arc<Reports>>,
) -> Holders {
    let tokens = Tokens::new(
        starter.store.clone(),
        Arc::new(crate::wire_stop_restart::TabEnvAdapter(identities)),
        socket,
    );
    Holders::new(Deps {
        machine,
        release,
        builds,
        migrations,
        starter,
        tokens: Arc::new(HolderTokensAdapter(tokens)),
        sessions: Arc::new(HolderSessionAdapter(tell)),
        events: Arc::new(HolderEventsAdapter(feed)),
        turns: Arc::new(HolderTurnsAdapter),
        report,
    })
}

/// Constructs [`Watch`] for link-up release comparisons and holder reports.
pub(crate) fn watch(migrations: Migrations, release: Release) -> Watch {
    Watch::new(migrations, release)
}

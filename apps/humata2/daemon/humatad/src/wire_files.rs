//! Adapters wiring `humata2_svc_files` port traits to `humatad`'s concrete
//! `Guard`, `Inbox`, `Feed`, `Bin`, `Links`, and `Migrations`.

use std::path::Path;
use std::sync::Arc;

use humata2_access::{AccessRequest, Call};
use humata2_chunk::Link;
use humata2_daemon_ports::{Caller, Run};
use humata2_grpc::humata::v1 as pb;
use humata2_rpc_kit::compression::Compression;
use humata2_svc_files::ports::{
    Checked, FileFeedPort, FileGuardPort, FileInboxPort, FileLayout, FileRelayPort, FileTrashPort,
    ReadFileStream, WriteFileStream,
};
use humata2_svc_files::{Files, Roots};
use tonic::Status;

use crate::events::Feed;
use crate::guard::Guard;
use crate::host_relay::Links;
use crate::inbox::Inbox;
use crate::trash_service::Bin;
use crate::upgrade::items::Migrations;

pub(crate) struct GuardAdapter(pub(crate) Arc<Guard>);

impl FileGuardPort for GuardAdapter {
    fn check(&self, caller: &Caller, call: &Call) -> Result<Checked, Status> {
        match self.0.check(caller, call)? {
            crate::guard::Checked::Allowed => Ok(Checked::Allowed),
            crate::guard::Checked::Ask(req) => Ok(Checked::Ask(req)),
        }
    }

    fn require(&self, caller: &Caller, call: &Call) -> Result<(), Status> {
        self.0.require(caller, call)
    }

    fn is_present_user(&self, caller: &Caller) -> bool {
        matches!(
            caller,
            Caller::User(
                humata2_daemon_ports::UserVia::App
                    | humata2_daemon_ports::UserVia::CliLogin { .. }
                    | humata2_daemon_ports::UserVia::Host
            )
        )
    }
}

pub(crate) struct InboxAdapter(pub(crate) Arc<Inbox>);

#[tonic::async_trait]
impl FileInboxPort for InboxAdapter {
    async fn ask_read(
        &self,
        caller: &Caller,
        asked: AccessRequest,
        operation: &str,
        run: Option<Run>,
    ) -> Result<(), Status> {
        self.0.ask_read(caller, asked, operation, run).await
    }
}

pub(crate) struct FeedAdapter(pub(crate) Arc<Feed>);

impl FileFeedPort for FeedAdapter {
    fn send_file_changed(&self, session: &str, event: pb::FileChanged) {
        self.0.publish_in(
            pb::Change::Updated,
            pb::event::Payload::FileChanged(event),
            crate::events::Scope {
                channel: String::new(),
                session: session.to_owned(),
            },
            None,
        );
    }
}

pub(crate) struct TrashAdapter(pub(crate) Arc<Bin>);

impl FileTrashPort for TrashAdapter {
    fn trash_path(
        &self,
        path: &Path,
        description: String,
        caller_data: String,
    ) -> Result<String, Status> {
        let trash = self.0.get()?;
        let item = humata2_transfers::NewItem {
            description,
            caller_data,
            now_unix_ms: humata2_rpc_kit::now_ms(),
        };
        let trashed = trash
            .put(path, item)
            .map_err(|e| crate::status::internal("move to trash", &e.to_string()))?;
        Ok(trashed.id)
    }
}

pub(crate) struct RelayAdapter {
    pub(crate) links: Links,
    pub(crate) migrations: Option<Migrations>,
}

#[tonic::async_trait]
impl FileRelayPort for RelayAdapter {
    fn is_online(&self, machine: &str) -> bool {
        self.links.channel(machine).is_ok()
    }

    fn machine_label(&self, machine: &str) -> String {
        machine.to_owned()
    }

    fn check_machine_version(&self, machine: &str) -> Result<(), Status> {
        let Some(migrations) = &self.migrations else {
            return Ok(());
        };
        crate::upgrade::gate::machine_ready(
            migrations,
            &pb::MachineId {
                value: machine.to_owned(),
            },
        )
    }

    async fn read_file(
        &self,
        machine: &str,
        request: pb::ReadFileRequest,
    ) -> Result<ReadFileStream, Status> {
        let channel = self.links.channel(machine).map_err(Status::from)?;
        let mut client = pb::file_service_client::FileServiceClient::new(channel);
        let stream = client.read_file(request).await?.into_inner();
        Ok(Box::pin(stream))
    }

    async fn write_file(
        &self,
        machine: &str,
        mut stream: WriteFileStream,
    ) -> Result<pb::FileEntry, Status> {
        let channel = self.links.channel(machine).map_err(Status::from)?;
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        tokio::spawn(async move {
            use tokio_stream::StreamExt as _;
            while let Some(msg) = stream.next().await {
                if tx.send(msg).await.is_err() {
                    break;
                }
            }
        });
        let mut client = pb::file_service_client::FileServiceClient::new(channel);
        Ok(client
            .write_file(tokio_stream::wrappers::ReceiverStream::new(rx))
            .await?
            .into_inner())
    }

    async fn list_directory(
        &self,
        machine: &str,
        request: pb::ListDirectoryRequest,
    ) -> Result<pb::ListDirectoryResponse, Status> {
        let channel = self.links.channel(machine).map_err(Status::from)?;
        let mut client = pb::file_service_client::FileServiceClient::new(channel);
        Ok(client.list_directory(request).await?.into_inner())
    }

    async fn stat_file(
        &self,
        machine: &str,
        request: pb::StatRequest,
    ) -> Result<pb::FileEntry, Status> {
        let channel = self.links.channel(machine).map_err(Status::from)?;
        let mut client = pb::file_service_client::FileServiceClient::new(channel);
        Ok(client.stat(request).await?.into_inner())
    }

    async fn create_directory(
        &self,
        machine: &str,
        request: pb::CreateFolderRequest,
    ) -> Result<pb::FileEntry, Status> {
        let channel = self.links.channel(machine).map_err(Status::from)?;
        let mut client = pb::file_service_client::FileServiceClient::new(channel);
        Ok(client.create_folder(request).await?.into_inner())
    }

    async fn rename_file(
        &self,
        machine: &str,
        request: pb::RenameRequest,
    ) -> Result<pb::FileEntry, Status> {
        let channel = self.links.channel(machine).map_err(Status::from)?;
        let mut client = pb::file_service_client::FileServiceClient::new(channel);
        Ok(client.rename(request).await?.into_inner())
    }

    async fn move_to_trash(
        &self,
        machine: &str,
        request: pb::MoveToTrashRequest,
    ) -> Result<pb::MoveToTrashResponse, Status> {
        let channel = self.links.channel(machine).map_err(Status::from)?;
        let mut client = pb::file_service_client::FileServiceClient::new(channel);
        Ok(client.move_to_trash(request).await?.into_inner())
    }

    async fn find_files(
        &self,
        machine: &str,
        request: pb::FindFilesRequest,
    ) -> Result<pb::FindFilesResponse, Status> {
        let channel = self.links.channel(machine).map_err(Status::from)?;
        let mut client = pb::file_service_client::FileServiceClient::new(channel);
        Ok(client.find_files(request).await?.into_inner())
    }

    async fn list_trash(
        &self,
        machine: &str,
        request: pb::ListTrashRequest,
    ) -> Result<pb::ListTrashResponse, Status> {
        let channel = self.links.channel(machine).map_err(Status::from)?;
        let mut client = pb::trash_service_client::TrashServiceClient::new(channel);
        Ok(client.list_trash(request).await?.into_inner())
    }

    async fn restore_from_trash(
        &self,
        machine: &str,
        request: pb::RestoreFromTrashRequest,
    ) -> Result<pb::RestoreFromTrashResponse, Status> {
        let channel = self.links.channel(machine).map_err(Status::from)?;
        let mut client = pb::trash_service_client::TrashServiceClient::new(channel);
        Ok(client.restore_from_trash(request).await?.into_inner())
    }

    async fn plan_empty_trash(
        &self,
        machine: &str,
        request: pb::PlanEmptyTrashRequest,
    ) -> Result<pb::EmptyTrashPlan, Status> {
        let channel = self.links.channel(machine).map_err(Status::from)?;
        let mut client = pb::trash_service_client::TrashServiceClient::new(channel);
        Ok(client.plan_empty_trash(request).await?.into_inner())
    }

    async fn empty_trash(
        &self,
        machine: &str,
        request: pb::EmptyTrashRequest,
    ) -> Result<pb::EmptyTrashResponse, Status> {
        let channel = self.links.channel(machine).map_err(Status::from)?;
        let mut client = pb::trash_service_client::TrashServiceClient::new(channel);
        Ok(client.empty_trash(request).await?.into_inner())
    }
}

/// Construct `humata2_svc_files::Files` wired to `humatad`'s concrete services.
#[allow(clippy::too_many_arguments)]
pub(crate) fn files_svc(
    store: Arc<humata2_store::Store>,
    machine: String,
    channels_root: std::path::PathBuf,
    home: std::path::PathBuf,
    guard: Arc<Guard>,
    bin: Arc<Bin>,
    compression: Arc<Compression>,
    link: Link,
    inbox: Arc<Inbox>,
    _feed: Arc<Feed>,
) -> Arc<Files> {
    let roots = Arc::new(Roots::new(store, machine, FileLayout::new(channels_root), home));
    let svc = Files::new(
        roots,
        Arc::new(GuardAdapter(guard)),
        Arc::new(TrashAdapter(bin)),
        compression,
        link,
        Arc::new(InboxAdapter(inbox)),
    );
    Arc::new(svc)
}

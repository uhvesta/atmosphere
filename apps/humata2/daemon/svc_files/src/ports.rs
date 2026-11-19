//! Port traits and value types decoupling `humata2_svc_files` from
//! `humatad`-internal services (`Guard`, `Inbox`, `Feed`, `Bin`, `Links`,
//! `Migrations`).

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::{SystemTime, UNIX_EPOCH};

use humata2_access::{AccessRequest, Call, ChannelId, MachineId, Refusal, SessionId, SessionRef};
use humata2_daemon_ports::{Caller, Run};
use crate::pb;
use humata2_store::{SessionRecord, StoreError};
use tokio_stream::Stream;
use tonic::{Code, Request, Status};

pub use humata2_daemon_ports::actor;
pub use humata2_rpc_kit::{
    Answers, answer_key_for_actor, blocking, caller_data, chain, error, failure, internal,
    invalid_field, missing, not_found, now_ms, status_error_of,
};

/// The Desk channel id constant (matches `humata2_sessions_core::DESK_CHANNEL`).
pub const DESK_CHANNEL: &str = "desk";

/// The channels root sentinel (matches `humata2_sessions_core::CHANNELS_ROOT`).
pub const CHANNELS_ROOT: &str = "@root";

/// Result of a guard check: allowed or must ask the user.
#[derive(Debug)]
pub enum Checked {
    Allowed,
    Ask(AccessRequest),
}

/// Access-control port implemented by `humatad::guard::Guard`.
pub trait FileGuardPort: Send + Sync {
    fn check(&self, caller: &Caller, call: &Call) -> Result<Checked, Status>;
    fn require(&self, caller: &Caller, call: &Call) -> Result<(), Status>;
    fn is_present_user(&self, caller: &Caller) -> bool;
}

/// Inbox port implemented by `humatad::inbox::Inbox`.
#[tonic::async_trait]
pub trait FileInboxPort: Send + Sync {
    async fn ask_read(
        &self,
        caller: &Caller,
        asked: AccessRequest,
        operation: &str,
        run: Option<Run>,
    ) -> Result<(), Status>;
}

/// Event-feed port implemented by `humatad::events::Feed`.
pub trait FileFeedPort: Send + Sync {
    fn send_file_changed(&self, session: &str, event: pb::FileChanged);
}

/// Trash-bin port implemented by `humatad::trash_service::Bin`.
pub trait FileTrashPort: Send + Sync {
    fn trash_path(
        &self,
        path: &Path,
        description: String,
        caller_data: String,
    ) -> Result<String, Status>;
}

/// Stream of `FileChunk` responses from a local or relayed `ReadFile`.
pub type ReadFileStream = Pin<Box<dyn Stream<Item = Result<pb::FileChunk, Status>> + Send>>;

/// Stream of `WriteFileMessage` messages forwarded to a remote machine.
pub type WriteFileStream = Pin<Box<dyn Stream<Item = pb::WriteFileMessage> + Send>>;

/// Cross-machine file & trash relay port implemented by `humatad::host_relay::Links`
/// and `humatad::upgrade::items::Migrations`.
#[tonic::async_trait]
pub trait FileRelayPort: Send + Sync {
    fn is_online(&self, machine: &str) -> bool;
    fn machine_label(&self, machine: &str) -> String;
    fn check_machine_version(&self, machine: &str) -> Result<(), Status>;
    async fn read_file(
        &self,
        machine: &str,
        request: pb::ReadFileRequest,
    ) -> Result<ReadFileStream, Status>;
    async fn write_file(
        &self,
        machine: &str,
        stream: WriteFileStream,
    ) -> Result<pb::FileEntry, Status>;
    async fn list_directory(
        &self,
        machine: &str,
        request: pb::ListDirectoryRequest,
    ) -> Result<pb::ListDirectoryResponse, Status>;
    async fn stat_file(
        &self,
        machine: &str,
        request: pb::StatRequest,
    ) -> Result<pb::FileEntry, Status>;
    async fn create_directory(
        &self,
        machine: &str,
        request: pb::CreateFolderRequest,
    ) -> Result<pb::FileEntry, Status>;
    async fn rename_file(
        &self,
        machine: &str,
        request: pb::RenameRequest,
    ) -> Result<pb::FileEntry, Status>;
    async fn move_to_trash(
        &self,
        machine: &str,
        request: pb::MoveToTrashRequest,
    ) -> Result<pb::MoveToTrashResponse, Status>;
    async fn find_files(
        &self,
        machine: &str,
        request: pb::FindFilesRequest,
    ) -> Result<pb::FindFilesResponse, Status>;
    async fn list_trash(
        &self,
        machine: &str,
        request: pb::ListTrashRequest,
    ) -> Result<pb::ListTrashResponse, Status>;
    async fn restore_from_trash(
        &self,
        machine: &str,
        request: pb::RestoreFromTrashRequest,
    ) -> Result<pb::RestoreFromTrashResponse, Status>;
    async fn plan_empty_trash(
        &self,
        machine: &str,
        request: pb::PlanEmptyTrashRequest,
    ) -> Result<pb::EmptyTrashPlan, Status>;
    async fn empty_trash(
        &self,
        machine: &str,
        request: pb::EmptyTrashRequest,
    ) -> Result<pb::EmptyTrashResponse, Status>;
}

/// Directory layout for channels on this machine (mirrors `humata2_sessions::Layout`).
#[derive(Clone, Debug)]
pub struct FileLayout {
    root: PathBuf,
}

impl FileLayout {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn channel(&self, folder: &str) -> PathBuf {
        self.root.join(folder)
    }

    pub fn shared(&self, folder: &str) -> PathBuf {
        self.root.join(folder).join("shared")
    }

    pub fn main_clone(&self, folder: &str, repo: &str) -> PathBuf {
        self.root.join(folder).join("main").join(repo)
    }
}

impl From<PathBuf> for FileLayout {
    fn from(root: PathBuf) -> Self {
        Self::new(root)
    }
}

/// Extracts the authenticated [`Caller`] from a tonic request's extensions.
pub fn caller<T>(request: &Request<T>) -> Result<Caller, Status> {
    request.extensions().get::<Caller>().cloned().ok_or_else(|| {
        internal(
            "this call reached its handler without being authenticated",
            "",
        )
    })
}

/// Converts a [`SessionRecord`] to a policy [`SessionRef`].
pub fn session_ref(record: &SessionRecord, default_machine: &str) -> SessionRef {
    let machine = match record.machine.is_empty() {
        true => default_machine,
        false => record.machine.as_str(),
    };
    SessionRef {
        id: SessionId::new(record.id.as_str()),
        channel: ChannelId::new(record.channel.as_str()),
        machine: MachineId::new(machine),
    }
}

/// Formats a [`StoreError`] as a gRPC `Status`.
pub fn store_error(err: StoreError, _machine: &str, doing: &str) -> Status {
    internal(format!("couldn't {doing}: {}", chain(&err)), "")
}

/// Formats a [`Refusal`] as a gRPC `PermissionDenied` status.
pub fn refused(refusal: &Refusal) -> Status {
    failure(
        Code::PermissionDenied,
        error(
            refusal.to_string(),
            "check your permissions and try again",
            pb::error::Kind::Denied(pb::Denied {
                source: pb::denied::Source::Unspecified as i32,
                capability: 0,
                request: None,
                user_note: String::new(),
            }),
        ),
    )
}

/// Converts a `SystemTime` to a protobuf `Timestamp`.
pub fn timestamp(at: SystemTime) -> humata2_grpc_core::Timestamp {
    let since = at.duration_since(UNIX_EPOCH).unwrap_or_default();
    humata2_grpc_core::Timestamp {
        seconds: i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
        nanos: i32::try_from(since.subsec_nanos()).unwrap_or(0),
    }
}

/// Computes an idempotency answer key for `caller`.
pub fn answer_key(
    call: &str,
    caller: &Caller,
    key: Option<&pb::IdempotencyKey>,
    what: &str,
) -> Option<String> {
    answer_key_for_actor(call, &actor(caller), key, what)
}

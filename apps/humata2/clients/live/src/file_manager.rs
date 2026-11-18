//! [`FileManager`] over the daemon's gRPC services: files (`FileService`),
//! transfers (`TransferService`), trash (`TrashService`) and Storage
//! (`MachineService`). Every call goes to the host daemon this client is
//! connected to, with `location.machine` set; the host relays to other
//! machines.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error as _;
use std::fmt::Write as _;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use humata2_clients::model::{MachineId, SessionId, TransferId};
use humata2_clients::{
    EVENT_QUEUE, EmptyPlan, Entry, EntryKind, FileManager, FileManagerError, FileReply, FileRoot, Folder,
    ItemLanding, Landing, Listing, RetentionSkip, StorageAction, StorageKind, StorageReport, StorageRow, TransferMode, TransferPhase, TransferRow,
    TrashEntry, TrashListing,
};
use humata2_grpc::humata::v1 as pb;
use humata2_grpc::humata::v1::{
    file_service_client::FileServiceClient, machine_service_client::MachineServiceClient,
    transfer_service_client::TransferServiceClient, trash_service_client::TrashServiceClient,
};
use prost::Message;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Code, Status, Streaming};

use crate::connection::{LiveDaemon, Shared, Wire, fresh_key};

/// Entries, transfers or trash items per page asked for.
const PAGE_SIZE: u32 = 200;
/// Upload chunks are at most 1 MiB (TR-11).
const CHUNK_BYTES: u64 = 1 << 20;
/// How long a new transfer is watched for a first state that says it can't
/// run (a full disk), before the caller is given its id anyway.
const FIRST_STATE: Duration = Duration::from_millis(300);
/// The wait before a broken transfer stream is opened again, and its cap.
const REOPEN_FIRST: Duration = Duration::from_millis(500);
const REOPEN_MAX: Duration = Duration::from_secs(10);
/// How long the local UTC offset is kept before it is read again.
const OFFSET_KEPT: Duration = Duration::from_secs(600);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    pub(crate) fn file_service(&self) -> FileServiceClient<Wire> {
        FileServiceClient::new(self.wire.clone())
    }

    fn transfer_service(&self) -> TransferServiceClient<Wire> {
        TransferServiceClient::new(self.wire.clone())
    }

    fn trash_service(&self) -> TrashServiceClient<Wire> {
        TrashServiceClient::new(self.wire.clone())
    }

    fn machine_service(&self) -> MachineServiceClient<Wire> {
        MachineServiceClient::new(self.wire.clone())
    }
}

struct Inner {
    shared: Arc<Shared>,
    subscribers: Mutex<Vec<mpsc::Sender<TransferRow>>>,
    /// Transfers with a running `WatchTransfer` task.
    watching: Mutex<BTreeSet<String>>,
    /// What each upload this client made is called (the daemon's transfer
    /// has no sources to name it by).
    upload_names: Mutex<BTreeMap<String, String>>,
    /// Whether the transfers that were already running have been picked up.
    scanned: AtomicBool,
    /// The local UTC offset and when it was read.
    offset: Mutex<Option<(Instant, i32)>>,
}

/// The app's file manager over one daemon connection. Cheap to clone.
#[derive(Clone)]
pub struct LiveFileManager {
    inner: Arc<Inner>,
}

impl LiveFileManager {
    /// A file manager over `daemon`'s connection.
    #[must_use]
    pub fn new(daemon: &LiveDaemon) -> Self {
        Self {
            inner: Arc::new(Inner {
                shared: daemon.shared.clone(),
                subscribers: Mutex::new(Vec::new()),
                watching: Mutex::new(BTreeSet::new()),
                upload_names: Mutex::new(BTreeMap::new()),
                scanned: AtomicBool::new(false),
                offset: Mutex::new(None),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Errors.
// ---------------------------------------------------------------------------

/// `google.rpc.Status`, the wrapper a Humata error travels in.
#[derive(Clone, PartialEq, prost::Message)]
struct RpcStatus {
    #[prost(int32, tag = "1")]
    code: i32,
    #[prost(string, tag = "2")]
    message: String,
    #[prost(message, repeated, tag = "3")]
    details: Vec<Any>,
}

/// `google.protobuf.Any`.
#[derive(Clone, PartialEq, prost::Message)]
struct Any {
    #[prost(string, tag = "1")]
    type_url: String,
    #[prost(bytes = "vec", tag = "2")]
    value: Vec<u8>,
}

/// The `humata.v1.Error` in a status's details.
fn detail_of(status: &Status) -> Option<pb::Error> {
    RpcStatus::decode(status.details())
        .ok()?
        .details
        .into_iter()
        .find(|any| any.type_url.ends_with("/humata.v1.Error"))
        .and_then(|any| pb::Error::decode(any.value.as_slice()).ok())
}

/// The daemon's words for a failure: what failed, then what to do.
fn words(error: &pb::Error) -> String {
    if error.fix.is_empty() {
        error.message.clone()
    } else {
        format!("{} {}", error.message, error.fix)
    }
}

fn machine_of(id: Option<&pb::MachineId>, fallback: &MachineId) -> MachineId {
    id.filter(|id| !id.value.is_empty())
        .map_or_else(|| fallback.clone(), |id| MachineId::new(id.value.clone()))
}

/// The errors that name their kind, by kind: never by message text.
fn from_error(error: &pb::Error, machine: &MachineId) -> Option<FileManagerError> {
    use pb::error::Kind as K;
    Some(match error.kind.as_ref()? {
        K::MachineOffline(offline) => FileManagerError::Offline {
            machine: machine_of(offline.machine.as_ref(), machine),
        },
        K::HostUnreachable(_) => FileManagerError::Offline {
            machine: machine.clone(),
        },
        K::NotFound(missing) => FileManagerError::NotFound {
            path: missing.id.clone(),
        },
        K::NameTaken(taken) => FileManagerError::NameTaken {
            name: taken.name.clone(),
        },
        K::DiskFull(full) => FileManagerError::DiskFull {
            machine: machine_of(full.machine.as_ref(), machine),
            needs_bytes: full.needed_bytes,
            free_bytes: full.free_bytes,
        },
        K::Denied(_)
        | K::UserOnly(_)
        | K::PresenceRequired(_)
        | K::OutsideHumataFolders(_)
        | K::MainCloneReadOnly(_)
        | K::ParentReadOnly(_) => FileManagerError::NotAllowed { what: words(error) },
        _ => return None,
    })
}

/// A gRPC failure of a call about `what` on `machine` as the app's error.
fn status_error(status: &Status, what: &str, machine: &MachineId) -> FileManagerError {
    let detail = detail_of(status);
    if let Some(mapped) = detail.as_ref().and_then(|error| from_error(error, machine)) {
        return mapped;
    }
    let transport = status
        .source()
        .is_some_and(|source| source.is::<tonic::transport::Error>());
    if transport || status.code() == Code::Unavailable {
        return FileManagerError::Offline {
            machine: machine.clone(),
        };
    }
    let message = detail
        .as_ref()
        .map_or_else(|| status.message().to_owned(), words);
    match status.code() {
        Code::NotFound => FileManagerError::NotFound {
            path: what.to_owned(),
        },
        Code::PermissionDenied => FileManagerError::NotAllowed { what: message },
        // UNIMPLEMENTED included: the daemon's words say what isn't served.
        _ => FileManagerError::Failed {
            what: what.to_owned(),
            message,
        },
    }
}

/// A name the caller gave stands in the daemon's own spelling of it.
fn named(error: FileManagerError, name: &str) -> FileManagerError {
    match error {
        FileManagerError::NameTaken { .. } => FileManagerError::NameTaken { name: name.to_owned() },
        other => other,
    }
}

fn failed(what: &str, message: impl Into<String>) -> FileManagerError {
    FileManagerError::Failed {
        what: what.to_owned(),
        message: message.into(),
    }
}

fn valid_name(name: &str) -> Result<(), FileManagerError> {
    if name.is_empty() || name.contains('/') || name == "." || name == ".." {
        return Err(FileManagerError::NotAllowed {
            what: format!("{name:?} can't be a name. Use a name without / that isn't empty"),
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Mapping.
// ---------------------------------------------------------------------------

fn location(folder: &Folder, name: Option<&str>, what: &str) -> Result<pb::FileLocation, FileManagerError> {
    use pb::file_location::Root;
    let root = match &folder.root {
        FileRoot::Home => Root::Home(pb::MachineHome {}),
        FileRoot::ChannelFolder(channel) => Root::ChannelFolder(pb::ChannelFolder {
            channel: Some(pb::ChannelId {
                value: channel.0.clone(),
            }),
        }),
        FileRoot::ChannelShared(channel) => Root::ChannelShared(pb::ChannelShared {
            channel: Some(pb::ChannelId {
                value: channel.0.clone(),
            }),
        }),
        FileRoot::SessionClone { session, repo } => {
            let Some((owner, name)) = repo.split_once('/').filter(|(owner, name)| !owner.is_empty() && !name.is_empty())
            else {
                return Err(failed(
                    what,
                    format!("the repository {repo:?} isn't written owner/name. Pick the repository again from the session"),
                ));
            };
            Root::SessionClone(pb::SessionClone {
                session: Some(pb::SessionId {
                    value: session.0.clone(),
                }),
                repo: Some(pb::RepoRef {
                    owner: owner.to_owned(),
                    name: name.to_owned(),
                }),
            })
        }
        FileRoot::SessionPastes(session) => Root::SessionPastes(pb::SessionPastes {
            session: Some(pb::SessionId {
                value: session.0.clone(),
            }),
        }),
        FileRoot::Received { from } => Root::Received(pb::MachineReceived {
            from: Some(pb::MachineId { value: from.0.clone() }),
        }),
        FileRoot::SessionFolder { session } => Root::SessionFolder(pb::SessionFolder {
            session: Some(pb::SessionId {
                value: session.0.clone(),
            }),
        }),
    };
    let path = match name {
        Some(name) => folder.child(name).path,
        None => folder.path.clone(),
    };
    Ok(pb::FileLocation {
        machine: Some(pb::MachineId {
            value: folder.machine.0.clone(),
        }),
        root: Some(root),
        path,
    })
}

fn machine_id(machine: &MachineId) -> Option<pb::MachineId> {
    Some(pb::MachineId {
        value: machine.0.clone(),
    })
}

fn now_unix_s() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_secs()).unwrap_or(i64::MAX))
}

fn entry(entry: &pb::FileEntry) -> Entry {
    use pb::file_entry::Kind;
    let kind = match Kind::try_from(entry.kind).unwrap_or(Kind::Unspecified) {
        Kind::Folder => EntryKind::Folder,
        Kind::Symlink => EntryKind::Link,
        Kind::File | Kind::Unspecified => EntryKind::File,
    };
    let path = entry.location.as_ref().map(|location| location.path.as_str()).unwrap_or("");
    Entry {
        name: path.rsplit('/').find(|part| !part.is_empty()).unwrap_or("").to_owned(),
        kind,
        size: (kind != EntryKind::Folder).then_some(entry.size),
        modified_unix_s: entry.modified_at.as_ref().map_or(0, |at| at.seconds),
    }
}

/// Folders first, then files, each by name.
fn sort_entries(entries: &mut [Entry]) {
    entries.sort_by(|a, b| {
        (a.kind != EntryKind::Folder)
            .cmp(&(b.kind != EntryKind::Folder))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

fn state_of(transfer: &pb::Transfer) -> pb::TransferState {
    pb::TransferState::try_from(transfer.state).unwrap_or(pb::TransferState::Unspecified)
}

fn is_over(transfer: &pb::Transfer) -> bool {
    use pb::TransferState as S;
    matches!(state_of(transfer), S::Done | S::Failed | S::Cancelled)
}

fn transfer_id(transfer: &pb::Transfer) -> String {
    transfer.id.as_ref().map(|id| id.value.clone()).unwrap_or_default()
}

fn phase_of(transfer: &pb::Transfer, host: &MachineId) -> TransferPhase {
    use pb::TransferState as S;
    let problem = transfer.problem.as_ref();
    match state_of(transfer) {
        S::Running | S::Unspecified => TransferPhase::Copying,
        S::Verifying => TransferPhase::Verifying,
        S::Paused => TransferPhase::Paused {
            waiting_for: None,
            reason: problem.map_or_else(|| "It was paused".to_owned(), words),
        },
        S::WaitingForMachine => {
            let waiting = match problem.and_then(|problem| problem.kind.as_ref()) {
                Some(pb::error::Kind::MachineOffline(offline)) => {
                    Some(machine_of(offline.machine.as_ref(), host)).filter(|_| offline.machine.is_some())
                }
                _ => None,
            };
            TransferPhase::Paused {
                waiting_for: waiting,
                reason: problem.map_or_else(|| "A machine is offline; it continues when it is back".to_owned(), words),
            }
        }
        S::Done => TransferPhase::Done,
        S::Failed => TransferPhase::Failed {
            reason: problem.map_or_else(|| "It failed. Try it again".to_owned(), words),
        },
        S::Cancelled => TransferPhase::Cancelled,
    }
}

/// The last name in a location's path.
fn last_part(location: &pb::FileLocation) -> String {
    location
        .path
        .rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or("")
        .to_owned()
}

fn what_of(transfer: &pb::Transfer, upload_name: Option<&str>) -> String {
    let last = last_part;
    match transfer.sources.as_slice() {
        [] => upload_name.map_or_else(|| "Upload".to_owned(), str::to_owned),
        [one] => {
            let name = last(one);
            // One source with several files in it is a folder.
            if transfer.total_files > 1 { format!("{name}/") } else { name }
        }
        [first, rest @ ..] => format!("{} and {} more", last(first), rest.len()),
    }
}

/// The tray row of a daemon transfer. `host` is the daemon this client talks
/// to: the end an upload comes from, and the end that isn't "via host".
fn transfer_row(transfer: &pb::Transfer, host: &MachineId, upload_name: Option<&str>) -> TransferRow {
    let from = match transfer.sources.first() {
        Some(source) => machine_of(source.machine.as_ref(), host),
        None => host.clone(),
    };
    let to = machine_of(transfer.destination.as_ref().and_then(|d| d.machine.as_ref()), host);
    let via_host = from != to && &from != host && &to != host;
    TransferRow {
        id: TransferId::new(transfer_id(transfer)),
        what: what_of(transfer, upload_name),
        from,
        to,
        via_host,
        done_bytes: transfer.done_bytes,
        total_bytes: transfer.total_bytes,
        phase: phase_of(transfer, host),
        outcomes: outcomes_of(transfer),
    }
}

/// What became of each source of a finished transfer (PL-13): the daemon
/// sends one outcome per source, in order.
fn outcomes_of(transfer: &pb::Transfer) -> Vec<ItemLanding> {
    transfer
        .sources
        .iter()
        .zip(&transfer.outcomes)
        .map(|(source, outcome)| ItemLanding {
            name: last_part(source),
            landing: match pb::TransferLanding::try_from(outcome.landing) {
                Ok(pb::TransferLanding::SavedAs) => Landing::SavedAs(outcome.saved_as.clone()),
                Ok(pb::TransferLanding::AlreadyThere) => Landing::AlreadyThere,
                Ok(pb::TransferLanding::Unspecified) | Err(_) => Landing::AsAsked,
            },
        })
        .collect()
}

/// A transfer that failed at once for a reason the caller can act on (a full
/// disk, a name in use), as the error to return instead of an id (LD-19).
fn failed_at_start(transfer: &pb::Transfer, machine: &MachineId) -> Option<FileManagerError> {
    if state_of(transfer) != pb::TransferState::Failed {
        return None;
    }
    from_error(transfer.problem.as_ref()?, machine).filter(|error| {
        matches!(
            error,
            FileManagerError::DiskFull { .. } | FileManagerError::NameTaken { .. } | FileManagerError::NotFound { .. }
        )
    })
}

fn storage_kind(kind: i32) -> Option<StorageKind> {
    Some(match pb::StorageKind::try_from(kind).ok()? {
        pb::StorageKind::SessionClones => StorageKind::SessionClones,
        pb::StorageKind::MainClones => StorageKind::MainClones,
        pb::StorageKind::BuildCache => StorageKind::BuildCache,
        pb::StorageKind::Trash => StorageKind::Trash,
        pb::StorageKind::Logs => StorageKind::Logs,
        pb::StorageKind::Unspecified => return None,
    })
}

fn row_session(row: &pb::StorageRow) -> Option<SessionId> {
    match row.entity.as_ref()?.entity.as_ref()? {
        pb::entity_ref::Entity::Session(session) if !session.value.is_empty() => {
            Some(SessionId::new(session.value.clone()))
        }
        _ => None,
    }
}

fn storage_row(row: &pb::StorageRow) -> Option<StorageRow> {
    let kind = storage_kind(row.kind)?;
    let session = row_session(row);
    let action = match pb::StorageAction::try_from(row.action).unwrap_or(pb::StorageAction::Unspecified) {
        pb::StorageAction::Archive => session.clone().map_or(StorageAction::None, StorageAction::Archive),
        pb::StorageAction::Clear => session.clone().map_or(StorageAction::None, StorageAction::ClearBuildCache),
        pb::StorageAction::Empty => StorageAction::EmptyTrash,
        pb::StorageAction::Trim => StorageAction::TrimLogs,
        pb::StorageAction::None | pb::StorageAction::Unspecified => StorageAction::None,
    };
    Some(StorageRow {
        kind,
        what: row.what.clone(),
        // The daemon's row has no second line.
        note: None,
        bytes: row.bytes,
        last_used_unix_s: row.last_used_at.as_ref().map(|at| at.seconds),
        // The row doesn't say whether the session runs, so a build cache
        // always asks first: the daemon refuses a running session's clear
        // without confirmation anyway (LD-15).
        asks_first: matches!(action, StorageAction::ClearBuildCache(_)),
        action,
    })
}

fn storage_report(machine: &MachineId, report: &pb::StorageReport) -> StorageReport {
    StorageReport {
        machine: machine.clone(),
        free_bytes: report.free_bytes,
        total_bytes: report.total_bytes,
        measured_unix_s: report.measured_at.as_ref().map_or(0, |at| at.seconds),
        rows: report.rows.iter().filter_map(storage_row).collect(),
        retention_skips: report
            .retention_skips
            .iter()
            .filter_map(|skip| {
                let session = skip.session.as_ref().map(|session| session.value.clone())?;
                Some(RetentionSkip {
                    // The daemon sends no title.
                    title: session.clone(),
                    session: SessionId::new(session),
                    reason: skip.reason.clone(),
                })
            })
            .collect(),
    }
}

/// `+0530` as seconds east of UTC.
pub(crate) fn parse_offset(text: &str) -> Option<i32> {
    let text = text.trim();
    let (sign, digits) = match text.split_at_checked(1)? {
        ("+", digits) => (1, digits),
        ("-", digits) => (-1, digits),
        _ => return None,
    };
    if digits.len() != 4 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let hours: i32 = digits.get(..2)?.parse().ok()?;
    let minutes: i32 = digits.get(2..)?.parse().ok()?;
    Some(sign * (hours * 3600 + minutes * 60))
}

// ---------------------------------------------------------------------------
// Calls.
// ---------------------------------------------------------------------------

impl Inner {
    fn host(&self) -> &MachineId {
        &self.shared.machine
    }

    fn publish(&self, row: &TransferRow) {
        lock(&self.subscribers).retain(|subscriber| match subscriber.try_send(row.clone()) {
            Err(mpsc::error::TrySendError::Closed(_)) => false,
            Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => true,
        });
    }

    fn row(&self, transfer: &pb::Transfer) -> TransferRow {
        let names = lock(&self.upload_names);
        transfer_row(transfer, self.host(), names.get(&transfer_id(transfer)).map(String::as_str))
    }

    /// The local offset from UTC, read from `date` at most every ten minutes.
    /// 0 when it can't be read.
    async fn utc_offset(&self) -> i32 {
        if let Some((at, offset)) = *lock(&self.offset) {
            if at.elapsed() < OFFSET_KEPT {
                return offset;
            }
        }
        let read = self
            .shared
            .spawn(async {
                let output = tokio::process::Command::new("date").arg("+%z").output().await.ok()?;
                parse_offset(&String::from_utf8_lossy(&output.stdout))
            })
            .await
            .ok()
            .flatten()
            .unwrap_or(0);
        *lock(&self.offset) = Some((Instant::now(), read));
        read
    }

    async fn list(&self, folder: Folder) -> Result<Listing, FileManagerError> {
        let what = format!("Listing {}", folder.display_path());
        let place = location(&folder, None, &what)?;
        let mut entries = Vec::new();
        let mut token = String::new();
        loop {
            let reply = self
                .shared
                .file_service()
                .list_directory(pb::ListDirectoryRequest {
                    location: Some(place.clone()),
                    page: Some(pb::PageRequest {
                        page_size: PAGE_SIZE,
                        page_token: token.clone(),
                    }),
                })
                .await
                .map_err(|status| status_error(&status, &what, &folder.machine))?
                .into_inner();
            entries.extend(reply.entries.iter().map(entry));
            token = reply.page.map(|page| page.next_page_token).unwrap_or_default();
            if token.is_empty() {
                break;
            }
        }
        sort_entries(&mut entries);
        Ok(Listing {
            folder,
            entries,
            read_unix_s: now_unix_s(),
            utc_offset_s: self.utc_offset().await,
        })
    }

    async fn create_folder(&self, folder: &Folder, name: &str) -> Result<(), FileManagerError> {
        let what = format!("Making the folder {name}");
        valid_name(name)?;
        self.shared
            .file_service()
            .create_folder(pb::CreateFolderRequest {
                idempotency_key: fresh_key(),
                location: Some(location(folder, Some(name), &what)?),
            })
            .await
            .map_err(|status| named(status_error(&status, &what, &folder.machine), name))?;
        Ok(())
    }

    async fn rename(&self, folder: &Folder, old_name: &str, new_name: &str) -> Result<(), FileManagerError> {
        let what = format!("Renaming {old_name}");
        valid_name(new_name)?;
        self.shared
            .file_service()
            .rename(pb::RenameRequest {
                idempotency_key: fresh_key(),
                location: Some(location(folder, Some(old_name), &what)?),
                new_name: new_name.to_owned(),
            })
            .await
            .map_err(|status| named(status_error(&status, &what, &folder.machine), new_name))?;
        Ok(())
    }

    async fn delete(&self, folder: &Folder, names: Vec<String>) -> Result<(), FileManagerError> {
        let what = "Moving to the trash";
        let locations = names
            .iter()
            .map(|name| location(folder, Some(name), what))
            .collect::<Result<Vec<_>, _>>()?;
        self.shared
            .file_service()
            .move_to_trash(pb::MoveToTrashRequest {
                idempotency_key: fresh_key(),
                locations,
            })
            .await
            .map_err(|status| status_error(&status, what, &folder.machine))?;
        Ok(())
    }

    // -- Transfers --------------------------------------------------------

    async fn start_transfer(
        self: &Arc<Self>,
        from: Folder,
        names: Vec<String>,
        into: Folder,
        mode: TransferMode,
        keep_both: bool,
    ) -> Result<TransferId, FileManagerError> {
        let on_existing = if keep_both {
            pb::OnExisting::KeepBoth
        } else {
            pb::OnExisting::Fail
        };
        let what = match mode {
            TransferMode::Copy => "Copying",
            TransferMode::Move => "Moving",
        };
        let sources = names
            .iter()
            .map(|name| location(&from, Some(name), what))
            .collect::<Result<Vec<_>, _>>()?;
        let started = self
            .shared
            .transfer_service()
            .start_transfer(pb::StartTransferRequest {
                idempotency_key: fresh_key(),
                sources,
                destination: Some(location(&into, None, what)?),
                mode: match mode {
                    TransferMode::Copy => pb::TransferMode::Copy,
                    TransferMode::Move => pb::TransferMode::Move,
                } as i32,
                on_existing: on_existing as i32,
                reason: String::new(),
                name: String::new(),
            })
            .await
            .map_err(|status| status_error(&status, what, &into.machine))?
            .into_inner();
        self.follow_new(started, what, &into.machine).await
    }

    /// Takes a transfer this client just started: the error to return when
    /// it can't run, else its id, with its changes pushed from here on.
    async fn follow_new(
        self: &Arc<Self>,
        started: pb::Transfer,
        what: &str,
        machine: &MachineId,
    ) -> Result<TransferId, FileManagerError> {
        let id = transfer_id(&started);
        if id.is_empty() {
            return Err(failed(what, "the daemon answered without a transfer id. Update Humata and the daemon to the same version"));
        }
        if let Some(error) = failed_at_start(&started, machine) {
            return Err(error);
        }
        if is_over(&started) {
            self.publish(&self.row(&started));
            return Ok(TransferId::new(id));
        }
        if !self.claim(&id) {
            return Ok(TransferId::new(id));
        }
        let mut stream = None;
        let mut from = started.version;
        if let Ok(mut opened) = self.open_watch(&id, 0).await {
            if let Ok(Ok(Some(first))) = tokio::time::timeout(FIRST_STATE, opened.message()).await {
                if let Some(error) = failed_at_start(&first, machine) {
                    self.release(&id);
                    return Err(error);
                }
                self.publish(&self.row(&first));
                from = from.max(first.version);
                if is_over(&first) {
                    self.release(&id);
                    return Ok(TransferId::new(id));
                }
            }
            stream = Some(opened);
        }
        self.shared.spawn(self.clone().follow(id.clone(), stream, from));
        Ok(TransferId::new(id))
    }

    /// Whether this call is the one to watch `id` (nobody watches it yet).
    fn claim(&self, id: &str) -> bool {
        lock(&self.watching).insert(id.to_owned())
    }

    fn release(&self, id: &str) {
        lock(&self.watching).remove(id);
    }

    async fn open_watch(&self, id: &str, from_version: u64) -> Result<Streaming<pb::Transfer>, Status> {
        self.shared
            .transfer_service()
            .watch_transfer(pb::WatchTransferRequest {
                id: Some(pb::TransferId { value: id.to_owned() }),
                from_version,
            })
            .await
            .map(tonic::Response::into_inner)
    }

    /// Pushes every state of one transfer until it is over. A stream that
    /// breaks before that is opened again from the last version, with a
    /// growing wait, for as long as someone listens.
    async fn follow(self: Arc<Self>, id: String, mut stream: Option<Streaming<pb::Transfer>>, mut from: u64) {
        let mut wait = REOPEN_FIRST;
        loop {
            let mut open = match stream.take() {
                Some(open) => open,
                None => match self.open_watch(&id, from).await {
                    Ok(open) => open,
                    Err(status) => {
                        let nobody = lock(&self.subscribers).iter().all(mpsc::Sender::is_closed);
                        if status.code() == Code::NotFound || nobody {
                            break;
                        }
                        tokio::time::sleep(wait).await;
                        wait = (wait * 2).min(REOPEN_MAX);
                        continue;
                    }
                },
            };
            while let Ok(Some(transfer)) = open.message().await {
                wait = REOPEN_FIRST;
                from = from.max(transfer.version);
                self.publish(&self.row(&transfer));
                if is_over(&transfer) {
                    self.release(&id);
                    return;
                }
            }
            tokio::time::sleep(wait).await;
            wait = (wait * 2).min(REOPEN_MAX);
        }
        self.release(&id);
    }

    /// Every transfer the daemon knows, newest first.
    async fn all_transfers(&self, what: &str) -> Result<Vec<pb::Transfer>, FileManagerError> {
        let mut all = Vec::new();
        let mut token = String::new();
        loop {
            let reply = self
                .shared
                .transfer_service()
                .list_transfers(pb::ListTransfersRequest {
                    active_only: false,
                    page: Some(pb::PageRequest {
                        page_size: PAGE_SIZE,
                        page_token: token.clone(),
                    }),
                })
                .await
                .map_err(|status| status_error(&status, what, self.host()))?
                .into_inner();
            all.extend(reply.transfers);
            token = reply.page.map(|page| page.next_page_token).unwrap_or_default();
            if token.is_empty() {
                return Ok(all);
            }
        }
    }

    /// Starts watching the transfers that were already running, once.
    fn pick_up_running(self: &Arc<Self>) {
        if self.scanned.swap(true, Ordering::SeqCst) {
            return;
        }
        let this = self.clone();
        self.shared.spawn(async move {
            match this.all_transfers("Listing transfers").await {
                Ok(all) => {
                    for transfer in all.iter().filter(|transfer| !is_over(transfer)) {
                        let id = transfer_id(transfer);
                        if !id.is_empty() && this.claim(&id) {
                            this.shared.spawn(this.clone().follow(id, None, 0));
                        }
                    }
                }
                Err(_) => this.scanned.store(false, Ordering::SeqCst),
            }
        });
    }

    async fn cancel_transfer(&self, transfer: &TransferId) -> Result<(), FileManagerError> {
        let what = "Cancelling the transfer";
        self.shared
            .transfer_service()
            .cancel_transfer(pb::TransferControlRequest {
                idempotency_key: fresh_key(),
                id: Some(pb::TransferId {
                    value: transfer.0.clone(),
                }),
            })
            .await
            .map_err(|status| match status.code() {
                Code::NotFound => FileManagerError::NotFound {
                    path: transfer.to_string(),
                },
                _ => status_error(&status, what, self.host()),
            })?;
        Ok(())
    }

    // -- Upload (FM-4) ----------------------------------------------------

    async fn upload(self: &Arc<Self>, local: Vec<PathBuf>, into: Folder) -> Result<TransferId, FileManagerError> {
        let this = self.clone();
        // Files are read on the runtime this client connected on.
        self.shared
            .spawn(async move { this.upload_tree(local, into).await })
            .await
            .unwrap_or_else(|error| Err(failed("Uploading", format!("the upload stopped: {error}"))))
    }

    /// Every file under `local`, folders made first, links skipped. The
    /// first file's transfer is the one returned.
    async fn upload_tree(self: &Arc<Self>, local: Vec<PathBuf>, into: Folder) -> Result<TransferId, FileManagerError> {
        const WHAT: &str = "Uploading";
        // All or nothing: a dropped name the folder already has refuses the
        // whole drop before the first folder is made or byte is sent.
        let present = self.list(into.clone()).await?;
        let taken: Vec<String> = local
            .iter()
            .filter_map(|path| path.file_name().map(|name| name.to_string_lossy().into_owned()))
            .filter(|name| present.entries.iter().any(|entry| &entry.name == name))
            .collect();
        if !taken.is_empty() {
            return Err(FileManagerError::NotAllowed {
                what: format!(
                    "Nothing was uploaded: {} already {} in {}. Move the old {} away or leave {} out of the drop, then drop again",
                    taken.join(", "),
                    if taken.len() == 1 { "exists" } else { "exist" },
                    into.display_path(),
                    if taken.len() == 1 { "one" } else { "ones" },
                    if taken.len() == 1 { "it" } else { "them" },
                ),
            });
        }
        let mut work: Vec<(PathBuf, Folder)> = local.into_iter().rev().map(|path| (path, into.clone())).collect();
        let mut first = None;
        while let Some((path, dest)) = work.pop() {
            let meta = tokio::fs::symlink_metadata(&path)
                .await
                .map_err(|error| local_error(WHAT, &path, &error))?;
            let Some(name) = path.file_name().map(|name| name.to_string_lossy().into_owned()) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                self.create_folder(&dest, &name).await?;
                let inside = dest.child(&name);
                let mut children = Vec::new();
                let mut read = tokio::fs::read_dir(&path)
                    .await
                    .map_err(|error| local_error(WHAT, &path, &error))?;
                while let Some(child) = read
                    .next_entry()
                    .await
                    .map_err(|error| local_error(WHAT, &path, &error))?
                {
                    children.push(child.path());
                }
                children.sort();
                work.extend(children.into_iter().rev().map(|child| (child, inside.clone())));
            } else if meta.is_file() {
                let id = self.upload_file(&path, meta.len(), &dest, &name).await?;
                first.get_or_insert(id);
            }
        }
        first.ok_or_else(|| FileManagerError::NotAllowed {
            what: "There was no file to upload (folders were made, links and empty folders carry nothing). Drop files, or a folder with files in it".to_owned(),
        })
    }

    async fn upload_file(
        self: &Arc<Self>,
        path: &Path,
        size: u64,
        dest: &Folder,
        name: &str,
    ) -> Result<TransferId, FileManagerError> {
        let what = format!("Uploading {name}");
        let transfer = self.send_file(path, size, dest, name).await?;
        lock(&self.upload_names).insert(transfer_id(&transfer), name.to_owned());
        self.follow_new(transfer, &what, &dest.machine).await
    }

    /// Uploads `local` into a new folder of the session's pastes on
    /// `machine` and returns where each file is on that machine's disk, in
    /// order (D335). Anything that isn't a file is refused before anything
    /// is sent.
    async fn paste_files(
        &self,
        local: Vec<PathBuf>,
        machine: &MachineId,
        session: &SessionId,
    ) -> Result<Vec<String>, FileManagerError> {
        const WHAT: &str = "Pasting files";
        let mut files = Vec::with_capacity(local.len());
        for path in local {
            let meta = tokio::fs::metadata(&path)
                .await
                .map_err(|error| local_error(WHAT, &path, &error))?;
            match path.file_name().map(|name| name.to_string_lossy().into_owned()) {
                Some(name) if meta.is_file() => files.push((path, meta.len(), name)),
                _ => {
                    return Err(FileManagerError::NotAllowed {
                        what: format!(
                            "{} isn't a file, so it can't be pasted into a terminal on another machine. Paste files only, or copy the folder over in Files",
                            path.display()
                        ),
                    });
                }
            }
        }
        let pastes = Folder::root(machine.clone(), FileRoot::SessionPastes(session.clone()));
        let folder = format!("paste-{}", uuid::Uuid::new_v4().simple());
        self.create_folder(&pastes, &folder).await?;
        let into = pastes.child(&folder);
        let mut typed = Vec::with_capacity(files.len());
        for (path, size, name) in files {
            let transfer = self.send_file(&path, size, &into, &name).await?;
            if let Some(error) = failed_at_start(&transfer, machine) {
                return Err(error);
            }
            if transfer.disk_path.is_empty() {
                return Err(failed(
                    &format!("Pasting {name}"),
                    "the daemon didn't say where the file landed. Update Humata and the daemon to the same version",
                ));
            }
            typed.push(transfer.disk_path);
        }
        Ok(typed)
    }

    /// Sends one file into `dest` as `name`, continuing once from its bytes
    /// when the stream broke, and returns the transfer the daemon answered.
    async fn send_file(
        &self,
        path: &Path,
        size: u64,
        dest: &Folder,
        name: &str,
    ) -> Result<pb::Transfer, FileManagerError> {
        let what = format!("Uploading {name}");
        let sha256 = hash_file(path).await.map_err(|error| local_error(&what, path, &error))?;
        let destination = location(dest, None, &what)?;
        let key = fresh_key();
        let header = |resume: Option<String>| pb::UploadHeader {
            idempotency_key: key.clone(),
            destination: Some(destination.clone()),
            name: name.to_owned(),
            size,
            sha256: sha256.clone(),
            on_existing: pb::OnExisting::Fail as i32,
            resume: resume.map(|value| pb::TransferId { value }),
        };
        let fail = |status: &Status| named(status_error(status, &what, &dest.machine), name);
        let transfer = match self.send_upload(path, header(None), 0).await {
            Ok(transfer) => transfer,
            Err(broken) if stream_broke(&broken) => {
                // Ask for the transfer the first attempt made (the same key
                // answers with it), then continue it once from its bytes.
                let found = self.lookup_upload(header(None)).await.map_err(|_| fail(&broken))?;
                match state_of(&found) {
                    pb::TransferState::Done => found,
                    pb::TransferState::Paused => {
                        let from = found.done_bytes;
                        self.send_upload(path, header(Some(transfer_id(&found))), from)
                            .await
                            .map_err(|status| fail(&status))?
                    }
                    _ => return Err(fail(&broken)),
                }
            }
            Err(status) => return Err(fail(&status)),
        };
        Ok(transfer)
    }

    /// One upload stream: the header, then the file's bytes from `offset`.
    async fn send_upload(&self, path: &Path, header: pb::UploadHeader, offset: u64) -> Result<pb::Transfer, Status> {
        let (sender, receiver) = mpsc::channel(4);
        let feeder = tokio::spawn(feed(path.to_owned(), header, offset, sender));
        let reply = self
            .shared
            .transfer_service()
            .upload(ReceiverStream::new(receiver))
            .await
            .map(tonic::Response::into_inner);
        feeder.abort();
        reply
    }

    /// The transfer an earlier upload with this header's key made: a header
    /// with no chunks is answered with it.
    async fn lookup_upload(&self, header: pb::UploadHeader) -> Result<pb::Transfer, Status> {
        let only = pb::UploadMessage {
            message: Some(pb::upload_message::Message::Header(header)),
        };
        self.shared
            .transfer_service()
            .upload(tokio_stream::iter(vec![only]))
            .await
            .map(tonic::Response::into_inner)
    }

    // -- Trash and Storage ------------------------------------------------

    async fn trash(&self, machine: &MachineId) -> Result<TrashListing, FileManagerError> {
        let what = "Reading the trash";
        let mut items = Vec::new();
        // Set by the first page, which always arrives before the loop ends.
        let mut total_bytes;
        let mut token = String::new();
        loop {
            let reply = self
                .shared
                .trash_service()
                .list_trash(pb::ListTrashRequest {
                    machine: machine_id(machine),
                    page: Some(pb::PageRequest {
                        page_size: PAGE_SIZE,
                        page_token: token.clone(),
                    }),
                })
                .await
                .map_err(|status| status_error(&status, what, machine))?
                .into_inner();
            total_bytes = reply.total_bytes;
            items.extend(reply.items.iter().map(|item| TrashEntry {
                id: item.id.as_ref().map(|id| id.value.clone()).unwrap_or_default(),
                description: item.description.clone(),
                bytes: item.bytes,
                deleted_unix_s: item.deleted_at.as_ref().map_or(0, |at| at.seconds),
                empties_unix_s: item.empties_at.as_ref().map_or(0, |at| at.seconds),
            }));
            token = reply.page.map(|page| page.next_page_token).unwrap_or_default();
            if token.is_empty() {
                break;
            }
        }
        // Newest first.
        items.sort_by(|a, b| b.deleted_unix_s.cmp(&a.deleted_unix_s));
        Ok(TrashListing {
            machine: machine.clone(),
            items,
            total_bytes,
            read_unix_s: now_unix_s(),
        })
    }

    async fn restore(&self, machine: &MachineId, id: &str) -> Result<(), FileManagerError> {
        let what = "Putting it back";
        self.shared
            .trash_service()
            .restore_from_trash(pb::RestoreFromTrashRequest {
                idempotency_key: fresh_key(),
                id: Some(pb::TrashItemId { value: id.to_owned() }),
            })
            .await
            .map_err(|status| status_error(&status, what, machine))?;
        Ok(())
    }

    async fn plan_empty_trash(&self, machine: &MachineId) -> Result<EmptyPlan, FileManagerError> {
        let plan = self
            .shared
            .trash_service()
            .plan_empty_trash(pb::PlanEmptyTrashRequest {
                machine: machine_id(machine),
            })
            .await
            .map_err(|status| status_error(&status, "Planning to empty the trash", machine))?
            .into_inner();
        Ok(EmptyPlan {
            machine: machine.clone(),
            items: plan.items,
            frees_bytes: plan.frees_bytes,
            token: plan.token.map(|token| token.value).unwrap_or_default(),
        })
    }

    async fn empty_trash(&self, plan: EmptyPlan) -> Result<u64, FileManagerError> {
        // The app is the person: no presence proof is sent (the daemon takes
        // a call on its own socket from the app as the user's).
        let emptied = self
            .shared
            .trash_service()
            .empty_trash(pb::EmptyTrashRequest {
                idempotency_key: fresh_key(),
                machine: machine_id(&plan.machine),
                plan: Some(pb::PlanToken { value: plan.token }),
                presence: None,
            })
            .await
            .map_err(|status| status_error(&status, "Emptying the trash", &plan.machine))?
            .into_inner();
        Ok(emptied.freed_bytes)
    }

    async fn storage_report(&self, machine: &MachineId) -> Result<pb::StorageReport, FileManagerError> {
        self.shared
            .machine_service()
            .get_storage(pb::GetStorageRequest {
                id: machine_id(machine),
            })
            .await
            .map(tonic::Response::into_inner)
            .map_err(|status| status_error(&status, "Measuring storage", machine))
    }

    async fn clear_build_cache(&self, machine: &MachineId, session: &SessionId) -> Result<(), FileManagerError> {
        let what = "Clearing the build cache";
        // The trait names a session; the daemon names each cache's folder,
        // which only its Storage rows know.
        let report = self.storage_report(machine).await?;
        let paths: Vec<String> = report
            .rows
            .iter()
            .filter(|row| {
                pb::StorageAction::try_from(row.action) == Ok(pb::StorageAction::Clear)
                    && row_session(row).as_ref() == Some(session)
            })
            .map(|row| row.path.clone())
            .collect();
        if paths.is_empty() {
            return Err(FileManagerError::NotFound {
                path: format!("the build cache of {session}"),
            });
        }
        for path in paths {
            self.shared
                .machine_service()
                .clear_build_cache(pb::ClearBuildCacheRequest {
                    idempotency_key: fresh_key(),
                    session: Some(pb::SessionId {
                        value: session.0.clone(),
                    }),
                    path,
                    // The app asked the person first (the row's asks_first).
                    confirm_running: true,
                })
                .await
                .map_err(|status| status_error(&status, what, machine))?;
        }
        Ok(())
    }

    async fn trim_logs(&self, machine: &MachineId) -> Result<(), FileManagerError> {
        self.shared
            .machine_service()
            .trim_logs(pb::TrimLogsRequest {
                idempotency_key: fresh_key(),
                id: machine_id(machine),
            })
            .await
            .map_err(|status| status_error(&status, "Trimming logs", machine))?;
        Ok(())
    }
}

/// Whether an upload stream died rather than being refused.
fn stream_broke(status: &Status) -> bool {
    let transport = status
        .source()
        .is_some_and(|source| source.is::<tonic::transport::Error>());
    transport || matches!(status.code(), Code::Aborted | Code::Unavailable | Code::Unknown | Code::Cancelled)
}

fn local_error(what: &str, path: &Path, error: &std::io::Error) -> FileManagerError {
    failed(
        what,
        format!(
            "couldn't read {}: {error}. Check that the file still exists and that Humata may read it",
            path.display()
        ),
    )
}

/// The file's SHA-256 as lowercase hex, read in 1 MiB pieces.
async fn hash_file(path: &Path) -> Result<String, std::io::Error> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let mut hex = String::new();
    for byte in hasher.finalize().iter() {
        let _ = write!(hex, "{byte:02x}");
    }
    Ok(hex)
}

/// Sends the header, then the file from `offset` in raw 1 MiB chunks. Ends
/// quietly when the receiver is gone or the file can't be read; the stream
/// closing early is what the daemon turns into a paused transfer.
async fn feed(path: PathBuf, header: pb::UploadHeader, offset: u64, sender: mpsc::Sender<pb::UploadMessage>) {
    use pb::upload_message::Message;
    let (size, sha256) = (header.size, header.sha256.clone());
    let head = pb::UploadMessage {
        message: Some(Message::Header(header)),
    };
    if sender.send(head).await.is_err() {
        return;
    }
    let Ok(mut file) = tokio::fs::File::open(&path).await else {
        return;
    };
    if offset > 0 && file.seek(SeekFrom::Start(offset)).await.is_err() {
        return;
    }
    let mut at = offset;
    loop {
        let want = size.saturating_sub(at).min(CHUNK_BYTES);
        let Ok(length) = usize::try_from(want) else {
            return;
        };
        let mut data = vec![0u8; length];
        let mut filled = 0;
        while filled < length {
            match file.read(&mut data[filled..]).await {
                // The file shrank since it was measured.
                Ok(0) | Err(_) => return,
                Ok(read) => filled += read,
            }
        }
        let last = at + want >= size;
        let chunk = pb::FileChunk {
            offset: at,
            data,
            sha256: if last { sha256.clone() } else { String::new() },
            last,
            codec: pb::ChunkCodec::None as i32,
            raw_size: u32::try_from(want).unwrap_or(0),
            crc32: None,
        };
        let message = pb::UploadMessage {
            message: Some(Message::Chunk(chunk)),
        };
        if sender.send(message).await.is_err() || last {
            return;
        }
        at += want;
    }
}

// ---------------------------------------------------------------------------
// The trait.
// ---------------------------------------------------------------------------

impl FileManager for LiveFileManager {
    fn list(&self, folder: &Folder) -> FileReply<Listing> {
        let (inner, folder) = (self.inner.clone(), folder.clone());
        Box::pin(async move { inner.list(folder).await })
    }

    fn create_folder(&self, folder: &Folder, name: &str) -> FileReply<()> {
        let (inner, folder, name) = (self.inner.clone(), folder.clone(), name.to_owned());
        Box::pin(async move { inner.create_folder(&folder, &name).await })
    }

    fn rename(&self, folder: &Folder, old_name: &str, new_name: &str) -> FileReply<()> {
        let (inner, folder) = (self.inner.clone(), folder.clone());
        let (old_name, new_name) = (old_name.to_owned(), new_name.to_owned());
        Box::pin(async move { inner.rename(&folder, &old_name, &new_name).await })
    }

    fn delete(&self, folder: &Folder, names: Vec<String>) -> FileReply<()> {
        let (inner, folder) = (self.inner.clone(), folder.clone());
        Box::pin(async move { inner.delete(&folder, names).await })
    }

    fn start_transfer(
        &self,
        from: &Folder,
        names: Vec<String>,
        into: &Folder,
        mode: TransferMode,
    ) -> FileReply<TransferId> {
        let (inner, from, into) = (self.inner.clone(), from.clone(), into.clone());
        Box::pin(async move { inner.start_transfer(from, names, into, mode, false).await })
    }

    fn start_transfer_keep_both(
        &self,
        from: &Folder,
        names: Vec<String>,
        into: &Folder,
        mode: TransferMode,
    ) -> FileReply<TransferId> {
        let (inner, from, into) = (self.inner.clone(), from.clone(), into.clone());
        Box::pin(async move { inner.start_transfer(from, names, into, mode, true).await })
    }

    fn upload(&self, local: Vec<PathBuf>, into: &Folder) -> FileReply<TransferId> {
        let (inner, into) = (self.inner.clone(), into.clone());
        Box::pin(async move { inner.upload(local, into).await })
    }

    fn paste_files(
        &self,
        local: Vec<PathBuf>,
        machine: &MachineId,
        session: &SessionId,
    ) -> FileReply<Vec<String>> {
        let (inner, machine, session) = (self.inner.clone(), machine.clone(), session.clone());
        Box::pin(async move { inner.paste_files(local, &machine, &session).await })
    }

    fn cancel_transfer(&self, transfer: &TransferId) -> FileReply<()> {
        let (inner, transfer) = (self.inner.clone(), transfer.clone());
        Box::pin(async move { inner.cancel_transfer(&transfer).await })
    }

    fn transfers(&self) -> FileReply<Vec<TransferRow>> {
        let inner = self.inner.clone();
        Box::pin(async move {
            let all = inner.all_transfers("Listing transfers").await?;
            Ok(all.iter().map(|transfer| inner.row(transfer)).collect())
        })
    }

    fn transfer_events(&self) -> mpsc::Receiver<TransferRow> {
        let (sender, receiver) = mpsc::channel(EVENT_QUEUE);
        lock(&self.inner.subscribers).push(sender);
        self.inner.pick_up_running();
        receiver
    }

    fn trash(&self, machine: &MachineId) -> FileReply<TrashListing> {
        let (inner, machine) = (self.inner.clone(), machine.clone());
        Box::pin(async move { inner.trash(&machine).await })
    }

    fn restore(&self, machine: &MachineId, id: &str) -> FileReply<()> {
        let (inner, machine, id) = (self.inner.clone(), machine.clone(), id.to_owned());
        Box::pin(async move { inner.restore(&machine, &id).await })
    }

    fn plan_empty_trash(&self, machine: &MachineId) -> FileReply<EmptyPlan> {
        let (inner, machine) = (self.inner.clone(), machine.clone());
        Box::pin(async move { inner.plan_empty_trash(&machine).await })
    }

    fn empty_trash(&self, plan: EmptyPlan) -> FileReply<u64> {
        let inner = self.inner.clone();
        Box::pin(async move { inner.empty_trash(plan).await })
    }

    fn storage(&self, machine: &MachineId) -> FileReply<StorageReport> {
        let (inner, machine) = (self.inner.clone(), machine.clone());
        Box::pin(async move {
            let report = inner.storage_report(&machine).await?;
            Ok(storage_report(&machine, &report))
        })
    }

    fn clear_build_cache(&self, machine: &MachineId, session: &SessionId) -> FileReply<()> {
        let (inner, machine, session) = (self.inner.clone(), machine.clone(), session.clone());
        Box::pin(async move { inner.clear_build_cache(&machine, &session).await })
    }

    fn trim_logs(&self, machine: &MachineId) -> FileReply<()> {
        let (inner, machine) = (self.inner.clone(), machine.clone());
        Box::pin(async move { inner.trim_logs(&machine).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error(kind: pb::error::Kind) -> pb::Error {
        pb::Error {
            message: "it failed".to_owned(),
            fix: "do this".to_owned(),
            retry: None,
            kind: Some(kind),
        }
    }

    fn transfer(state: pb::TransferState) -> pb::Transfer {
        pb::Transfer {
            state: state as i32,
            ..pb::Transfer::default()
        }
    }

    fn mac() -> MachineId {
        MachineId::new("ma_mac")
    }

    #[test]
    fn errors_map_by_kind() {
        let machine = mac();
        let full = error(pb::error::Kind::DiskFull(pb::DiskFull {
            machine: Some(pb::MachineId {
                value: "ma_box".to_owned(),
            }),
            path: String::new(),
            needed_bytes: 900,
            free_bytes: 100,
        }));
        assert_eq!(
            from_error(&full, &machine),
            Some(FileManagerError::DiskFull {
                machine: MachineId::new("ma_box"),
                needs_bytes: 900,
                free_bytes: 100
            })
        );
        let taken = error(pb::error::Kind::NameTaken(pb::NameTaken {
            kind: 0,
            name: "out".to_owned(),
            suggestion: String::new(),
        }));
        assert_eq!(
            from_error(&taken, &machine),
            Some(FileManagerError::NameTaken { name: "out".to_owned() })
        );
        let outside = error(pb::error::Kind::OutsideHumataFolders(pb::OutsideHumataFolders {
            path: "../x".to_owned(),
        }));
        assert!(matches!(
            from_error(&outside, &machine),
            Some(FileManagerError::NotAllowed { .. })
        ));
        // A kind with no meaning here is left to the caller's default.
        let git = error(pb::error::Kind::GitFailed(pb::GitFailed::default()));
        assert_eq!(from_error(&git, &machine), None);
    }

    #[test]
    fn a_full_disk_at_start_is_an_error_not_an_id() {
        let mut started = transfer(pb::TransferState::Failed);
        started.problem = Some(error(pb::error::Kind::DiskFull(pb::DiskFull {
            machine: None,
            path: String::new(),
            needed_bytes: 5,
            free_bytes: 1,
        })));
        assert_eq!(
            failed_at_start(&started, &mac()),
            Some(FileManagerError::DiskFull {
                machine: mac(),
                needs_bytes: 5,
                free_bytes: 1
            })
        );
        started.problem = Some(error(pb::error::Kind::Internal(pb::Internal::default())));
        assert_eq!(failed_at_start(&started, &mac()), None);
        assert_eq!(failed_at_start(&transfer(pb::TransferState::Running), &mac()), None);
    }

    #[test]
    fn a_transfer_waiting_for_a_machine_names_it() {
        let mut waiting = transfer(pb::TransferState::WaitingForMachine);
        waiting.problem = Some(error(pb::error::Kind::MachineOffline(pb::MachineOffline {
            machine: Some(pb::MachineId {
                value: "ma_box".to_owned(),
            }),
            machine_name: String::new(),
            since: None,
        })));
        match phase_of(&waiting, &mac()) {
            TransferPhase::Paused { waiting_for, .. } => assert_eq!(waiting_for, Some(MachineId::new("ma_box"))),
            other => panic!("expected Paused, got {other:?}"),
        }
        assert_eq!(phase_of(&transfer(pb::TransferState::Done), &mac()), TransferPhase::Done);
        assert_eq!(
            phase_of(&transfer(pb::TransferState::Verifying), &mac()),
            TransferPhase::Verifying
        );
    }

    #[test]
    fn a_transfer_between_two_other_machines_runs_via_the_host() {
        let place = |machine: &str, path: &str| pb::FileLocation {
            machine: Some(pb::MachineId {
                value: machine.to_owned(),
            }),
            root: None,
            path: path.to_owned(),
        };
        let mut moving = transfer(pb::TransferState::Running);
        moving.sources = vec![place("ma_a", "data/set.csv")];
        moving.destination = Some(place("ma_b", ""));
        let row = transfer_row(&moving, &mac(), None);
        assert!(row.via_host);
        assert_eq!(row.what, "set.csv");
        assert_eq!((row.from, row.to), (MachineId::new("ma_a"), MachineId::new("ma_b")));
        moving.sources = vec![place("ma_mac", "data/set.csv")];
        assert!(!transfer_row(&moving, &mac(), None).via_host);
    }

    #[test]
    fn a_finished_row_says_where_each_source_landed() {
        let place = |path: &str| pb::FileLocation {
            machine: None,
            root: None,
            path: path.to_owned(),
        };
        let mut done = transfer(pb::TransferState::Done);
        done.sources = vec![place("a/notes.md"), place("a/same.md"), place("a/plain.md")];
        done.outcomes = vec![
            pb::TransferOutcome {
                landing: pb::TransferLanding::SavedAs as i32,
                saved_as: "notes-2.md".to_owned(),
            },
            pb::TransferOutcome {
                landing: pb::TransferLanding::AlreadyThere as i32,
                saved_as: String::new(),
            },
            pb::TransferOutcome::default(),
        ];
        let row = transfer_row(&done, &mac(), None);
        let landings: Vec<_> = row.outcomes.iter().map(|item| (item.name.as_str(), item.landing.clone())).collect();
        assert_eq!(
            landings,
            vec![
                ("notes.md", Landing::SavedAs("notes-2.md".to_owned())),
                ("same.md", Landing::AlreadyThere),
                ("plain.md", Landing::AsAsked),
            ]
        );
    }

    #[test]
    fn an_upload_row_is_named_by_the_file_and_comes_from_the_host() {
        let mut up = transfer(pb::TransferState::Running);
        up.destination = Some(pb::FileLocation {
            machine: Some(pb::MachineId {
                value: "ma_box".to_owned(),
            }),
            root: None,
            path: String::new(),
        });
        let row = transfer_row(&up, &mac(), Some("big.bin"));
        assert_eq!(row.what, "big.bin");
        assert_eq!(row.from, mac());
        assert!(!row.via_host);
    }

    #[test]
    fn storage_rows_carry_the_action_and_its_session() {
        let session = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Session(pb::SessionId {
                value: "se_1".to_owned(),
            })),
        };
        let row = pb::StorageRow {
            kind: pb::StorageKind::BuildCache as i32,
            what: "api/target".to_owned(),
            entity: Some(session),
            path: "api/target".to_owned(),
            bytes: 10,
            last_used_at: None,
            action: pb::StorageAction::Clear as i32,
        };
        let mapped = storage_row(&row).expect("a build cache row maps");
        assert_eq!(mapped.action, StorageAction::ClearBuildCache(SessionId::new("se_1")));
        assert_eq!(mapped.kind, StorageKind::BuildCache);
        // A row with no kind isn't shown.
        let unknown = pb::StorageRow::default();
        assert!(storage_row(&unknown).is_none());
    }

    #[test]
    fn local_offsets_parse() {
        assert_eq!(parse_offset("+0530\n"), Some(19_800));
        assert_eq!(parse_offset("-0800"), Some(-28_800));
        assert_eq!(parse_offset("+0000"), Some(0));
        assert_eq!(parse_offset("PST"), None);
        assert_eq!(parse_offset(""), None);
    }

    #[test]
    fn entries_sort_folders_first_then_by_name() {
        let item = |name: &str, kind| Entry {
            name: name.to_owned(),
            kind,
            size: None,
            modified_unix_s: 0,
        };
        let mut entries = vec![
            item("b.txt", EntryKind::File),
            item("Zed", EntryKind::Folder),
            item("a.txt", EntryKind::File),
            item("apps", EntryKind::Folder),
        ];
        sort_entries(&mut entries);
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["apps", "Zed", "a.txt", "b.txt"]);
    }
}

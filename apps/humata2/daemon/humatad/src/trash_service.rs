//! This machine's Humata trash (LD-6, LD-7, FM-3) as a service: list what
//! was deleted, restore it, empty it after a plan. Files the file manager
//! deletes (`FileService.MoveToTrash`) go into the same trash through
//! [`Bin`].
//!
//! The trash folder is opened the first time it is needed, so one that
//! can't be used (a full disk, wrong permissions) fails the calls that need
//! it and never stops the daemon. Items 30 days old are removed whenever
//! the trash is used and once at start ([`Bin::sweep`]), with no timer.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use humata2_access::{Call, Capability, ChannelId, Refusal, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_transfers::{EmptyPlan, KEEP_MS, Trash, TrashError, TrashItem};
use pb::invalid_field::Problem;
use prost::Message;
use tonic::{Code, Request, Response, Status};

use crate::desk_service::channel_desk_close::DeskHandle;
use crate::files::TrashForward;
use crate::grant_target;
use crate::guard::{self, Guard};
use crate::host_relay::Links;
use crate::identity::{self, Caller};
use crate::status;

/// How long a plan to empty the trash can be used.
const PLAN_LIFETIME: Duration = Duration::from_secs(10 * 60);

/// Between two sweeps of items past their 30 days.
const SWEEP_EVERY: Duration = Duration::from_secs(60);

/// Finished answers kept for retries with the same idempotency key.
pub(crate) const REMEMBERED_MAX: usize = 1024;

/// Items per page when the caller names no size.
const PAGE_DEFAULT: usize = 100;

/// The most items in one page.
const PAGE_MAX: usize = 1000;

/// This machine's trash, opened on first use.
pub(crate) struct Bin {
    open: Mutex<Open>,
}

struct Open {
    /// `<root>/.trash`; changes when the machine's root does (FC-10).
    folder: PathBuf,
    trash: Option<Arc<Trash>>,
    swept: Option<Instant>,
}

impl Bin {
    /// `folder` is `<root>/.trash`. Never fails and touches no disk.
    pub(crate) fn new(folder: PathBuf) -> Self {
        Self {
            open: Mutex::new(Open {
                folder,
                trash: None,
                swept: None,
            }),
        }
    }

    /// Keeps the trash in `folder` from now on; it is opened there on the
    /// next use. The items are moved by the caller.
    pub(crate) fn move_to(&self, folder: PathBuf) {
        let mut open = lock(&self.open);
        open.folder = folder;
        open.trash = None;
        open.swept = None;
    }

    /// The trash, opened if it wasn't yet, with items past their 30 days
    /// removed (at most once a minute). Does file work: call it on a
    /// blocking thread.
    pub(crate) fn get(&self) -> Result<Arc<Trash>, Status> {
        let mut open = lock(&self.open);
        let trash = match &open.trash {
            Some(trash) => trash.clone(),
            None => {
                let trash = Trash::open(open.folder.clone()).map_err(|error| {
                    status::internal(
                        format!(
                            "the trash folder {} can't be used: {}. Check that the folder belongs to you and the disk has free space; the next call tries again",
                            open.folder.display(),
                            status::chain(&error)
                        ),
                        "",
                    )
                })?;
                let trash = Arc::new(trash);
                open.trash = Some(trash.clone());
                trash
            }
        };
        if open.swept.is_none_or(|at| at.elapsed() >= SWEEP_EVERY) {
            open.swept = Some(Instant::now());
            if let Err(error) = trash.expire(now_ms()) {
                crate::log::say!(
                    "humatad2: couldn't remove old items from the trash: {}",
                    status::chain(&error)
                );
            }
        }
        Ok(trash)
    }

    /// Opens the trash and removes items past their 30 days. For the
    /// daemon's start; a trash that can't be used is logged. Blocking.
    pub(crate) fn sweep(&self) {
        if let Err(status) = self.get() {
            crate::log::say!("humatad2: {}", status.message());
        }
    }
}

/// What a deleter and the deleted thing were, kept with the item.
#[derive(Clone, PartialEq, Message)]
struct Data {
    #[prost(message, optional, tag = "1")]
    deleted_by: Option<pb::Actor>,
    #[prost(message, optional, tag = "2")]
    was: Option<pb::EntityRef>,
}

/// `deleted_by` and `was` as the string the trash keeps with an item.
pub(crate) fn caller_data(deleted_by: &pb::Actor, was: &pb::EntityRef) -> String {
    let data = Data {
        deleted_by: Some(deleted_by.clone()),
        was: Some(was.clone()),
    };
    data.encode_to_vec()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// What [`caller_data`] kept; nothing for data it can't read.
pub(crate) fn decode_caller_data(text: &str) -> (Option<pb::Actor>, Option<pb::EntityRef>) {
    let bytes: Option<Vec<u8>> = (text.len() % 2 == 0)
        .then(|| {
            (0..text.len())
                .step_by(2)
                .map(|at| text.get(at..at + 2).and_then(|pair| u8::from_str_radix(pair, 16).ok()))
                .collect()
        })
        .flatten();
    match bytes.and_then(|bytes| Data::decode(bytes.as_slice()).ok()) {
        Some(data) => (data.deleted_by, data.was),
        None => (None, None),
    }
}

/// Runs `work` on a blocking thread.
pub(crate) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, Status> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| status::internal(format!("a trash worker stopped: {error}"), ""))
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Answers kept by idempotency key, oldest forgotten first.
pub(crate) struct Answers<T> {
    inner: Mutex<(HashMap<String, T>, VecDeque<String>)>,
}

impl<T: Clone> Answers<T> {
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new((HashMap::new(), VecDeque::new())),
        }
    }

    pub(crate) fn get(&self, key: &str) -> Option<T> {
        lock(&self.inner).0.get(key).cloned()
    }

    pub(crate) fn put(&self, key: String, answer: T) {
        let mut inner = lock(&self.inner);
        if inner.0.insert(key.clone(), answer).is_none() {
            inner.1.push_back(key);
        }
        while inner.1.len() > REMEMBERED_MAX {
            if let Some(oldest) = inner.1.pop_front() {
                inner.0.remove(&oldest);
            }
        }
    }
}

/// The key a retry is recognised by: the call, who made it and its key.
/// `None` when the request carries no key.
pub(crate) fn answer_key(
    call: &str,
    caller: &Caller,
    key: Option<&pb::IdempotencyKey>,
    what: &str,
) -> Option<String> {
    let key = key.map(|key| key.value.as_str()).filter(|key| !key.is_empty())?;
    Some(format!("{call}\0{:?}\0{what}\0{key}", identity::actor(caller)))
}

pub(crate) struct TrashSvc {
    bin: Arc<Bin>,
    guard: Arc<Guard>,
    machine: String,
    /// Calls that name another machine's trash (FM-9, FM-12).
    forward: TrashForward,
    plans: Mutex<HashMap<String, (EmptyPlan, Instant)>>,
    restored: Answers<pb::EntityRef>,
    emptied: Answers<u64>,
    /// The state a restored channel's records go back to (D194).
    store: std::sync::OnceLock<Arc<humata2_store::Store>>,
    /// The Desk service, which checks and reopens a restored channel Desk
    /// session (CD-4, CD-5).
    desk: std::sync::OnceLock<DeskHandle>,
}

impl TrashSvc {
    pub(crate) fn new(bin: Arc<Bin>, guard: Arc<Guard>, machine: String) -> Self {
        Self {
            forward: TrashForward::new(guard.clone(), machine.clone()),
            bin,
            guard,
            machine,
            plans: Mutex::new(HashMap::new()),
            restored: Answers::new(),
            emptied: Answers::new(),
            store: std::sync::OnceLock::new(),
            desk: std::sync::OnceLock::new(),
        }
    }

    /// Lets a restored channel's records come back (D194).
    pub(crate) fn use_store(&self, store: Arc<humata2_store::Store>) {
        let _ = self.store.set(store);
    }

    /// Lets a restored channel Desk session reopen (CD-4).
    pub(crate) fn use_desk(&self, desk: DeskHandle) {
        let _ = self.desk.set(desk);
    }

    /// The machines' links (the host only; without them every other
    /// machine's trash is offline).
    pub(crate) fn use_links(&self, links: Links) {
        self.forward.use_links(links);
    }

    /// The trash of this machine; another machine's is forwarded before
    /// a handler gets here.
    fn own(&self, machine: Option<&pb::MachineId>, field: &str) -> Result<(), Status> {
        match machine.filter(|machine| !machine.value.is_empty()) {
            Some(machine) if machine.value != self.machine => Err(status::invalid_field(
                field,
                Problem::NotAllowedHere,
                format!(
                    "this is machine {}, and the trash of machine {} isn't relayed through it yet",
                    self.machine, machine.value
                ),
                "ask the daemon on that machine, or leave the machine out for this one",
            )),
            _ => Ok(()),
        }
    }

    fn require_user_only(&self, caller: &Caller) -> Result<(), Status> {
        if matches!(caller, Caller::Peer) {
            return Err(status::refused(&Refusal::PeerCannotCall));
        }
        self.guard.require(
            caller,
            &Call::Use {
                capability: Capability::TrashEmpty,
                target: Target::Global,
            },
        )
    }

    fn machine_id(&self) -> pb::MachineId {
        pb::MachineId {
            value: self.machine.clone(),
        }
    }
}

fn trash_item(machine: &pb::MachineId, item: &TrashItem) -> pb::TrashItem {
    let (deleted_by, was) = decode_caller_data(&item.caller_data);
    let at = |ms: i64| {
        grant_target::timestamp(UNIX_EPOCH + Duration::from_millis(u64::try_from(ms).unwrap_or(0)))
    };
    pb::TrashItem {
        id: Some(pb::TrashItemId {
            value: item.id.clone(),
        }),
        machine: Some(machine.clone()),
        description: item.description.clone(),
        bytes: item.bytes,
        deleted_at: Some(at(item.deleted_unix_ms)),
        empties_at: Some(at(item.deleted_unix_ms.saturating_add(KEEP_MS))),
        deleted_by,
        was,
    }
}

/// Where in the oldest-first order an item stands, as a page token.
fn position(item: &TrashItem) -> String {
    format!("{}:{}", item.deleted_unix_ms, item.id)
}

fn after(token: &str, item: &TrashItem) -> bool {
    let Some((ms, id)) = token.split_once(':') else {
        return true;
    };
    let Ok(ms) = ms.parse::<i64>() else {
        return true;
    };
    (item.deleted_unix_ms, item.id.as_str()) > (ms, id)
}

/// Puts the item back where it was; a channel's folder goes to the first
/// free `-2`, `-3`, ... of its old place when something took it (D194). A
/// channel Desk session's goes to `place`, its channel's Desk folder as the
/// channel stands now (MV-16), which is its old place unless the channel
/// moved while it sat in the trash.
fn restore_at_free_place(
    trash: &Trash,
    id: &str,
    channels: Option<&humata2_store::Store>,
    place: Option<&Path>,
) -> Result<PathBuf, TrashError> {
    if let Some(place) = place {
        return trash.restore_to(id, place);
    }
    // A file trashed inside a channel names the channel too; only a
    // deleted channel's own folder carries its records and may need a new
    // place.
    let Some(store) = channels.filter(|_| {
        trash
            .holds(id, Path::new(humata2_store::SIDE_FILE))
            .unwrap_or(false)
    }) else {
        return trash.restore(id);
    };
    let original = trash
        .list()?
        .into_iter()
        .find(|item| item.id == id)
        .map(|item| item.original)
        .ok_or_else(|| TrashError::NoSuchItem { id: id.to_owned() })?;
    let place = crate::channel_service::pick_place(store, &original).unwrap_or(original);
    trash.restore_to(id, &place)
}

/// A trash library failure the caller can't fix from the request.
fn trash_failure(error: &TrashError, doing: &str) -> Status {
    status::internal(
        format!("couldn't {doing}: {}", status::chain(error)),
        "",
    )
}

fn no_item(id: &str) -> Status {
    status::not_found(
        pb::EntityKind::TrashItem,
        id,
        format!("there is no item {id} in the trash"),
        "it may have been restored or emptied already; list the trash to see what is there",
    )
}

fn item_id(id: Option<pb::TrashItemId>) -> Result<String, Status> {
    id.map(|id| id.value)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| status::missing("id"))
}

#[tonic::async_trait]
impl pb::trash_service_server::TrashService for TrashSvc {
    /// Everything in the trash is the user's to see: agents are refused.
    async fn list_trash(
        &self,
        request: Request<pb::ListTrashRequest>,
    ) -> Result<Response<pb::ListTrashResponse>, Status> {
        if self.forward.elsewhere(request.get_ref().machine.as_ref()).is_some() {
            return self.forward.list_trash(request).await;
        }
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        self.own(request.machine.as_ref(), "machine")?;
        self.require_user_only(&caller)?;
        let (size, token) = match request.page {
            Some(page) => (page.page_size as usize, page.page_token),
            None => (0, String::new()),
        };
        let size = match size {
            0 => PAGE_DEFAULT,
            size => size.min(PAGE_MAX),
        };
        let bin = self.bin.clone();
        let items = blocking(move || {
            let trash = bin.get()?;
            trash
                .list()
                .map_err(|error| trash_failure(&error, "list the trash"))
        })
        .await??;
        let total_bytes = items.iter().map(|item| item.bytes).sum();
        let machine = self.machine_id();
        let mut rest = items.iter().filter(|item| token.is_empty() || after(&token, item));
        let page: Vec<&TrashItem> = rest.by_ref().take(size).collect();
        let next_page_token = match (rest.next(), page.last()) {
            (Some(_), Some(last)) => position(last),
            _ => String::new(),
        };
        Ok(Response::new(pb::ListTrashResponse {
            items: page.iter().map(|item| trash_item(&machine, item)).collect(),
            total_bytes,
            page: Some(pb::PageResponse { next_page_token }),
        }))
    }

    /// Puts an item back where it was deleted from. A path something else
    /// took since is never overwritten.
    async fn restore_from_trash(
        &self,
        request: Request<pb::RestoreFromTrashRequest>,
    ) -> Result<Response<pb::RestoreFromTrashResponse>, Status> {
        if self.forward.elsewhere_item(request.get_ref().id.as_ref()).is_some() {
            let item = request.get_ref().id.as_ref().map(|id| id.value.clone()).unwrap_or_default();
            let caller = identity::caller(&request)?;
            if matches!(caller, Caller::Peer) {
                return Err(status::refused(&Refusal::PeerCannotCall));
            }
            // CD-4, CD-5: a channel Desk session comes back only with its
            // channel and its target, which the host holds (SL-1); the
            // machine doesn't judge them. Its channel's rule is asked
            // first, so a caller who may not restore there learns nothing
            // of the channel or the target; any other item the machine
            // judges.
            if let Some(desk) = self.desk.get().and_then(DeskHandle::get) {
                if let Some(channel) = desk.closed_channel_of(&item).await? {
                    self.guard.require(
                        &caller,
                        &Call::Use {
                            capability: Capability::TrashRestore,
                            target: Target::Channel(ChannelId::new(channel.as_str())),
                        },
                    )?;
                }
                desk.before_restore(&item).await?;
            }
            let restored = self.forward.restore_from_trash(request).await?;
            // CD-4: the machine reopened the session; the host's row follows.
            if let Some(desk) = self.desk.get().and_then(DeskHandle::get)
                && let Err(error) = desk.restored_there(&caller, &item).await
            {
                crate::log::say!(
                    "humatad2: {item} was restored on its machine, but the host's row of the Desk session didn't reopen: {}; reload the Desk, and restart the host if it still shows the session closed",
                    error.message()
                );
            }
            return Ok(restored);
        }
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let id = item_id(request.id)?;
        let key = answer_key("restore", &caller, request.idempotency_key.as_ref(), &id);
        if let Some(restored) = key.as_deref().and_then(|key| self.restored.get(key)) {
            return Ok(Response::new(pb::RestoreFromTrashResponse {
                restored: Some(restored),
            }));
        }
        if matches!(caller, Caller::Peer) {
            return Err(status::refused(&Refusal::PeerCannotCall));
        }
        // What the item was decides whose rule judges the restore.
        let bin = self.bin.clone();
        let wanted = id.clone();
        let item = blocking(move || {
            let trash = bin.get()?;
            let items = trash
                .list()
                .map_err(|error| trash_failure(&error, "list the trash"))?;
            Ok::<_, Status>(items.into_iter().find(|item| item.id == wanted))
        })
        .await??
        .ok_or_else(|| no_item(&id))?;
        let (_, was) = decode_caller_data(&item.caller_data);
        let target = match (&was, &caller) {
            (
                Some(pb::EntityRef {
                    entity: Some(pb::entity_ref::Entity::Channel(channel)),
                }),
                _,
            ) => Target::Channel(ChannelId::new(channel.value.as_str())),
            // The item names no channel: an agent is judged in its own.
            (_, Caller::Agent(agent)) => Target::Channel(agent.session.channel.clone()),
            _ => Target::Global,
        };
        self.guard.require(
            &caller,
            &Call::Use {
                capability: Capability::TrashRestore,
                target,
            },
        )?;
        // CD-4, CD-5: a channel Desk session comes back only with its
        // channel and its target.
        let mut place = None;
        if let Some(desk) = self.desk.get().and_then(DeskHandle::get) {
            desk.before_restore(&id).await?;
            place = desk.restore_place(&id).await?;
        }
        let bin = self.bin.clone();
        let wanted = id.clone();
        let channel_store = match &was {
            Some(pb::EntityRef {
                entity: Some(pb::entity_ref::Entity::Channel(_)),
            }) => self.store.get().cloned(),
            _ => None,
        };
        let restoring = channel_store.clone();
        let placed = blocking(move || {
            let trash = bin.get()?;
            restore_at_free_place(&trash, &wanted, channel_store.as_deref(), place.as_deref()).map_err(|error| match error {
                TrashError::NoSuchItem { id } => no_item(&id),
                TrashError::Occupied { path } => status::failure(
                    Code::AlreadyExists,
                    status::error(
                        format!(
                            "{} already exists, so {wanted} can't be put back there; nothing was restored",
                            path.display()
                        ),
                        "rename or move what is at that path, then restore again",
                        pb::error::Kind::NameTaken(pb::NameTaken {
                            kind: pb::EntityKind::File as i32,
                            name: path.display().to_string(),
                            suggestion: String::new(),
                        }),
                    ),
                ),
                other => trash_failure(&other, "restore from the trash"),
            })
        })
        .await??;
        // CD-4: a restored channel Desk session reopens.
        if let Some(desk) = self.desk.get().and_then(DeskHandle::get) {
            desk.after_restore(&caller, &id, &placed).await?;
        }
        // A file or folder trashed inside a channel names that channel too;
        // only a deleted channel's own folder carries its records' side file.
        let restoring = restoring.filter(|_| placed.join(humata2_store::SIDE_FILE).is_file());
        if let Some(store) = restoring {
            blocking(move || crate::channel_service::finish_restore(&store, &placed))
                .await?
                .map_err(|error| {
                    status::internal(
                        format!(
                            "the channel's folder is back, but bringing the channel fully back failed: {}. The folder is safe where it is: fix the cause the message names, then restart Humata on this machine so it finishes what it can",
                            status::chain(&error)
                        ),
                        "",
                    )
                })?;
        }
        let restored = was.unwrap_or_default();
        if let Some(key) = key {
            self.restored.put(key, restored.clone());
        }
        Ok(Response::new(pb::RestoreFromTrashResponse {
            restored: Some(restored),
        }))
    }

    /// What emptying would free, and a token valid for ten minutes.
    async fn plan_empty_trash(
        &self,
        request: Request<pb::PlanEmptyTrashRequest>,
    ) -> Result<Response<pb::EmptyTrashPlan>, Status> {
        if self.forward.elsewhere(request.get_ref().machine.as_ref()).is_some() {
            return self.forward.plan_empty_trash(request).await;
        }
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        self.own(request.machine.as_ref(), "machine")?;
        self.require_user_only(&caller)?;
        let bin = self.bin.clone();
        let plan = blocking(move || {
            let trash = bin.get()?;
            trash
                .plan_empty()
                .map_err(|error| trash_failure(&error, "plan emptying the trash"))
        })
        .await??;
        let token = format!("pl_{}", uuid::Uuid::new_v4().simple());
        let reply = pb::EmptyTrashPlan {
            machine: Some(self.machine_id()),
            frees_bytes: plan.bytes,
            items: u32::try_from(plan.ids.len()).unwrap_or(u32::MAX),
            token: Some(pb::PlanToken {
                value: token.clone(),
            }),
        };
        let mut plans = lock(&self.plans);
        plans.retain(|_, (_, made)| made.elapsed() < PLAN_LIFETIME);
        plans.insert(token, (plan, Instant::now()));
        Ok(Response::new(reply))
    }

    /// Removes what the plan listed, once: the user only, present.
    async fn empty_trash(
        &self,
        request: Request<pb::EmptyTrashRequest>,
    ) -> Result<Response<pb::EmptyTrashResponse>, Status> {
        if self.forward.elsewhere(request.get_ref().machine.as_ref()).is_some() {
            return self.forward.empty_trash(request).await;
        }
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        self.own(request.machine.as_ref(), "machine")?;
        self.require_user_only(&caller)?;
        let principal = guard::principal(&caller);
        if self
            .guard
            .with_policy(|_, context| principal.present_user(context))
            .is_none()
        {
            return Err(status::refused(&Refusal::UserOnly {
                capability: Some(Capability::TrashEmpty),
            }));
        }
        let key = answer_key("empty", &caller, request.idempotency_key.as_ref(), "");
        if let Some(freed) = key.as_deref().and_then(|key| self.emptied.get(key)) {
            return Ok(Response::new(pb::EmptyTrashResponse { freed_bytes: freed }));
        }
        let token = request
            .plan
            .map(|plan| plan.value)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| status::missing("plan"))?;
        let plan = lock(&self.plans)
            .remove(&token)
            .filter(|(_, made)| made.elapsed() < PLAN_LIFETIME)
            .map(|(plan, _)| plan)
            .ok_or_else(|| {
                status::failure(
                    Code::FailedPrecondition,
                    status::error(
                        "the plan to empty the trash is unknown, used or older than ten minutes",
                        "ask for a new plan (PlanEmptyTrash), check what it frees, then empty with its token",
                        pb::error::Kind::PlanStale(pb::PlanStale { changes: Vec::new() }),
                    ),
                )
            })?;
        let bin = self.bin.clone();
        let freed = blocking(move || {
            let trash = bin.get()?;
            trash
                .empty(&plan)
                .map_err(|error| trash_failure(&error, "empty the trash"))
        })
        .await??;
        if let Some(key) = key {
            self.emptied.put(key, freed);
        }
        Ok(Response::new(pb::EmptyTrashResponse { freed_bytes: freed }))
    }
}

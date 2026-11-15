//! DiffService and ReviewChatService (D66, SPEC RV-28–RV-34, PR-22–PR-26):
//! a session's diffs, computed with `humata2_review_diff` in its clones on
//! this machine, the reviewed snapshots Mark reviewed takes, and the review
//! chat's view of the review.
//!
//! What a review showed is kept per session in memory: the last GetDiff's
//! files per repo, which MarkReviewed snapshots and the review chat's tools
//! check files against, and the chat's view, notes and changes. None of it
//! persists yet (OVERVIEW › review_diff wiring).

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use humata2_access::{Call, Capability, Entry, Place, RepoPath, Root, SessionRef, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_review_diff::{
    BaseContext, Checkout, ChatError, Clones, DiffError, FileRef, FileSummary, Matching, RepoDiff,
    ReviewChat, Side, Viewed, wire,
};
use humata2_store::{SessionRecord, Store};
use tokio::sync::watch;
use tokio_stream::Stream;
use tonic::{Code, Request, Response, Status};

use crate::guard::Guard;
use crate::identity::{self, Caller};
use crate::status;

/// At most this many lines of a selection go back with review_context.
const SELECTED_LINES_MAX: u32 = 200;

/// The services' shared state: per session, what its review showed.
#[derive(Clone)]
pub(crate) struct Review {
    inner: Arc<Inner>,
}

struct Inner {
    store: Arc<Store>,
    guard: Arc<Guard>,
    machine: String,
    sessions: Mutex<HashMap<String, Arc<SessionReview>>>,
    /// Answers to calls that carried an idempotency key, by key.
    marked: Mutex<HashMap<String, pb::MarkReviewedResponse>>,
    focused: Mutex<HashMap<String, pb::ReviewChange>>,
}

/// One session's review.
#[derive(Default)]
struct SessionReview {
    /// The last GetDiff's answer per repo folder.
    shown: Mutex<BTreeMap<String, RepoDiff>>,
    chat: Mutex<Chat>,
    /// The last change's seq, for WatchReviewChanges.
    changed: watch::Sender<u64>,
}

#[derive(Default)]
struct Chat {
    chat: ReviewChat,
    /// When each change was made, by seq, in Unix seconds.
    at: BTreeMap<u64, i64>,
}

impl Review {
    pub(crate) fn new(store: Arc<Store>, guard: Arc<Guard>, machine: String) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
                guard,
                machine,
                sessions: Mutex::default(),
                marked: Mutex::default(),
                focused: Mutex::default(),
            }),
        }
    }

    /// The session `id` names, from the store.
    fn session(&self, id: Option<&pb::SessionId>) -> Result<(SessionRecord, SessionRef), Status> {
        let id = id
            .map(|id| id.value.clone())
            .filter(|id| !id.is_empty())
            .ok_or_else(|| status::missing("session"))?;
        match self.inner.store.session(&id) {
            Ok(Some(record)) => {
                let session = identity::session_ref(&record, &self.inner.machine);
                Ok((record, session))
            }
            Ok(None) => Err(status::not_found(
                pb::EntityKind::Session,
                &id,
                format!("no session {id} on this machine"),
                "check the session id; diffs are computed on the session's own machine, so ask its daemon",
            )),
            Err(error) => Err(status::store(error, &self.inner.machine, "look up the session")),
        }
    }

    fn review(&self, session: &str) -> Arc<SessionReview> {
        lock(&self.inner.sessions)
            .entry(session.to_owned())
            .or_default()
            .clone()
    }

    fn require(&self, caller: &Caller, call: &Call) -> Result<(), Status> {
        self.inner.guard.require(caller, call)
    }

    /// Checks `caller` may read each repo's clone.
    fn require_read(&self, caller: &Caller, session: &SessionRef, repos: &[String]) -> Result<(), Status> {
        for repo in repos {
            let path = RepoPath::new(repo.as_str()).map_err(|_| bad_repo(repo))?;
            self.require(
                caller,
                &Call::ReadFiles(Place {
                    machine: session.machine.clone(),
                    root: Root::Clone(session.clone()),
                    path,
                    entry: Entry::Folder,
                }),
            )?;
        }
        Ok(())
    }

    fn require_use(&self, caller: &Caller, capability: Capability, session: &SessionRef) -> Result<(), Status> {
        self.require(
            caller,
            &Call::Use {
                capability,
                target: Target::Session(session.clone()),
            },
        )
    }
}

/// The repos `asked` names by key, or every clone in the session's folder.
async fn repo_keys(record: &SessionRecord, asked: &[pb::RepoRef]) -> Result<Vec<String>, Status> {
    if !asked.is_empty() {
        let mut keys = Vec::with_capacity(asked.len());
        for repo in asked {
            let key = wire::repo_key(repo);
            if repo.name.is_empty() || repo.name.contains('/') || repo.name == "." || repo.name == ".." {
                return Err(bad_repo(&key));
            }
            keys.push(key);
        }
        return Ok(keys);
    }
    let folder = record.folder.clone();
    blocking(move || {
        let entries = std::fs::read_dir(&folder).map_err(|error| {
            status::internal(format!("couldn't list the session's folder {}: {error}", folder.display()), "")
        })?;
        let mut keys: Vec<String> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().join(".git").exists())
            .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
            .collect();
        keys.sort();
        Ok(keys)
    })
    .await?
}

/// The folder of a repo's clone: its name, without the owner.
fn folder_name(key: &str) -> &str {
    key.rsplit_once('/').map_or(key, |(_, name)| name)
}

/// The session's clones of `keys`, opened.
async fn clones(record: &SessionRecord, keys: &[String]) -> Result<Clones, Status> {
    let folder = record.folder.clone();
    let keys = keys.to_vec();
    blocking(move || {
        let mut by_repo = BTreeMap::new();
        for key in keys {
            let checkout = Checkout::open(folder.join(folder_name(&key))).map_err(|error| diff_failed(&error))?;
            by_repo.insert(key, checkout);
        }
        Ok(Clones::new(by_repo))
    })
    .await?
}

fn context_of(record: &SessionRecord) -> BaseContext {
    BaseContext {
        session: record.id.clone(),
        fork_point: None,
    }
}

#[tonic::async_trait]
impl pb::diff_service_server::DiffService for Review {
    async fn get_diff(&self, request: Request<pb::GetDiffRequest>) -> Result<Response<pb::GetDiffResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let (record, session) = self.session(request.session.as_ref())?;
        let keys = repo_keys(&record, &request.repos).await?;
        self.require_read(&caller, &session, &keys)?;
        let base = request.base.unwrap_or_default();
        let spec = wire::spec_for_base(&base, &keys, |_| None).map_err(|error| wire_failed("base", &error))?;
        let clones = clones(&record, &keys).await?;
        let diffs = clones.diff(&spec, &context_of(&record)).await.map_err(|error| diff_failed(&error))?;
        let review = self.review(&record.id);
        let mut shown = lock(&review.shown);
        for diff in &diffs {
            shown.insert(folder_name(&diff.repo).to_owned(), diff.clone());
        }
        Ok(Response::new(pb::GetDiffResponse {
            any_changes: diffs.iter().any(|diff| !diff.files.is_empty()),
            repos: diffs.iter().map(wire::repo_diff_to_pb).collect(),
        }))
    }

    async fn get_file_diff(&self, request: Request<pb::GetFileDiffRequest>) -> Result<Response<pb::FileDiff>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let (record, session) = self.session(request.session.as_ref())?;
        let file = request.file.clone().ok_or_else(|| status::missing("file"))?;
        let repo = file.repo.clone().ok_or_else(|| status::missing("file.repo"))?;
        if file.path.is_empty() {
            return Err(status::missing("file.path"));
        }
        let keys = repo_keys(&record, std::slice::from_ref(&repo)).await?;
        self.require_read(&caller, &session, &keys)?;
        let base = request.base.clone().unwrap_or_default();
        let spec = wire::spec_for_base(&base, &keys, |_| None).map_err(|error| wire_failed("base", &error))?;
        let clones = clones(&record, &keys).await?;
        let key = wire::repo_key(&repo);
        let checkout = clones.get(&key).map_err(|error| diff_failed(&error))?.clone();
        let repo_spec = spec.repos.first().cloned().ok_or_else(|| status::missing("file.repo"))?;
        let matching = Matching::from(&repo_spec);
        // The file list the hunks belong to: the trees the client was
        // shown, or the spec as it is now.
        let diff = checkout
            .diff(repo_spec, context_of(&record))
            .await
            .map_err(|error| diff_failed(&error))?;
        let trees = wire::trees_from_pb(request.trees.as_ref()).unwrap_or_else(|| diff.trees.clone());
        let summary: FileSummary = diff.file(&file.path).cloned().ok_or_else(|| {
            status::not_found(
                pb::EntityKind::File,
                &file.path,
                format!("{key}/{} has no changes in this diff", file.path),
                "ask GetDiff for the files that changed, and pick one of those",
            )
        })?;
        let (from, budget) = wire::chunk_request(&request);
        if !request.viewed_blob_sha.is_empty() {
            let viewed = checkout
                .since_viewed(request.viewed_blob_sha.clone(), summary.clone(), matching, from, budget)
                .await
                .map_err(|error| diff_failed(&error))?;
            let whole = match viewed {
                Viewed::Forgotten => Some(
                    checkout
                        .file_chunk(trees, summary.clone(), matching, from, budget)
                        .await
                        .map_err(|error| diff_failed(&error))?,
                ),
                _ => None,
            };
            return Ok(Response::new(wire::viewed_file_diff_to_pb(&key, &summary, &viewed, whole.as_ref())));
        }
        let chunk = checkout
            .file_chunk(trees, summary.clone(), matching, from, budget)
            .await
            .map_err(|error| diff_failed(&error))?;
        Ok(Response::new(wire::file_diff_to_pb(&key, &summary, &chunk)))
    }

    async fn mark_reviewed(
        &self,
        request: Request<pb::MarkReviewedRequest>,
    ) -> Result<Response<pb::MarkReviewedResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let (record, session) = self.session(request.session.as_ref())?;
        self.require_use(&caller, Capability::UserReviewState, &session)?;
        let idempotency = request
            .idempotency_key
            .as_ref()
            .map(|key| format!("{}/{}", record.id, key.value))
            .filter(|key| !key.ends_with('/'));
        if let Some(answer) = idempotency.as_ref().and_then(|key| lock(&self.inner.marked).get(key).cloned()) {
            return Ok(Response::new(answer));
        }
        let keys = repo_keys(&record, &[]).await?;
        let clones = clones(&record, &keys).await?;
        // RV-28: per repo, the tree the client says it showed, else the
        // last GetDiff's, else the working tree as it is now.
        let review = self.review(&record.id);
        let mut trees: BTreeMap<String, String> = lock(&review.shown)
            .iter()
            .map(|(folder, diff)| (folder.clone(), diff.trees.new.clone()))
            .collect();
        for shown in &request.shown_tree {
            let repo = shown.repo.as_ref().ok_or_else(|| status::missing("shown_tree.repo"))?;
            if !shown.tree.is_empty() {
                trees.insert(folder_name(&wire::repo_key(repo)).to_owned(), shown.tree.clone());
            }
        }
        let mut snapshots = Vec::with_capacity(keys.len());
        for key in &keys {
            let checkout = clones.get(key).map_err(|error| diff_failed(&error))?;
            let tree = trees.get(folder_name(key)).cloned();
            let snapshot = checkout
                .mark_reviewed(record.id.clone(), tree)
                .await
                .map_err(|error| diff_failed(&error))?;
            snapshots.push(wire::snapshot_to_pb(key, &snapshot));
        }
        let answer = pb::MarkReviewedResponse { snapshots };
        if let Some(key) = idempotency {
            lock(&self.inner.marked).insert(key, answer.clone());
        }
        Ok(Response::new(answer))
    }

    async fn list_reviewed_snapshots(
        &self,
        request: Request<pb::ListReviewedSnapshotsRequest>,
    ) -> Result<Response<pb::ListReviewedSnapshotsResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let (record, session) = self.session(request.session.as_ref())?;
        let keys = repo_keys(&record, &request.repos).await?;
        self.require_read(&caller, &session, &keys)?;
        let clones = clones(&record, &keys).await?;
        let mut snapshots = Vec::new();
        for key in &keys {
            let checkout = clones.get(key).map_err(|error| diff_failed(&error))?;
            let kept = checkout.snapshots(record.id.clone()).await.map_err(|error| diff_failed(&error))?;
            snapshots.extend(kept.iter().map(|snapshot| wire::snapshot_to_pb(key, snapshot)));
        }
        Ok(Response::new(pb::ListReviewedSnapshotsResponse { snapshots }))
    }
}

type Changes = Pin<Box<dyn Stream<Item = Result<pb::ReviewChange, Status>> + Send>>;

#[tonic::async_trait]
impl pb::review_chat_service_server::ReviewChatService for Review {
    async fn get_review_context(
        &self,
        request: Request<pb::GetReviewContextRequest>,
    ) -> Result<Response<pb::ReviewContext>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let (record, session) = self.session(request.session.as_ref())?;
        // Never gated by the switches (PR-25): reading is what the chat is
        // for.
        self.require(&caller, &Call::ReadSession(session))?;
        let review = self.review(&record.id);
        let context = lock(&review.chat).chat.context();
        let selected = match context.view.open.as_ref() {
            Some(place) => selected_text(&record.folder, place).await,
            None => Vec::new(),
        };
        Ok(Response::new(wire::context_to_pb(&context, selected)))
    }

    async fn report_review_view(
        &self,
        request: Request<pb::ReportReviewViewRequest>,
    ) -> Result<Response<pb::ReportReviewViewResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let (record, session) = self.session(request.session.as_ref())?;
        self.require_use(&caller, Capability::UserReviewState, &session)?;
        let view = request.view.ok_or_else(|| status::missing("view"))?;
        let view = wire::view_from_pb(&view).map_err(|error| wire_failed("view", &error))?;
        lock(&self.review(&record.id).chat).chat.report(view);
        Ok(Response::new(pb::ReportReviewViewResponse {}))
    }

    async fn focus_review(&self, request: Request<pb::FocusReviewRequest>) -> Result<Response<pb::ReviewChange>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let (record, session) = self.session(request.session.as_ref())?;
        self.require_use(&caller, Capability::ReviewFocus, &session)?;
        let idempotency = request
            .idempotency_key
            .as_ref()
            .map(|key| format!("{}/{}", record.id, key.value))
            .filter(|key| !key.ends_with('/'));
        if let Some(answer) = idempotency.as_ref().and_then(|key| lock(&self.inner.focused).get(key).cloned()) {
            return Ok(Response::new(answer));
        }
        let (place, highlights) = wire::focus_from_pb(&request).map_err(|error| wire_failed("file", &error))?;
        let review = self.review(&record.id);
        let shown = lock(&review.shown).clone();
        let change = {
            let mut chat = lock(&review.chat);
            let change = chat
                .chat
                .focus(place, highlights, |file| in_diff(&shown, file))
                .map_err(|error| chat_failed(&error))?;
            let at = now();
            chat.at.insert(change.seq, at);
            wire::change_to_pb(&change, at)
        };
        review.changed.send_replace(change.seq);
        if let Some(key) = idempotency {
            lock(&self.inner.focused).insert(key, change.clone());
        }
        Ok(Response::new(change))
    }

    async fn undo_review_change(
        &self,
        request: Request<pb::UndoReviewChangeRequest>,
    ) -> Result<Response<pb::ReviewChange>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let (record, session) = self.session(request.session.as_ref())?;
        self.require_use(&caller, Capability::UserReviewState, &session)?;
        if request.undo_id.is_empty() {
            return Err(status::missing("undo_id"));
        }
        let review = self.review(&record.id);
        let change = {
            let mut chat = lock(&review.chat);
            let change = chat.chat.undo(&request.undo_id).map_err(|error| chat_failed(&error))?;
            let at = now();
            chat.at.insert(change.seq, at);
            wire::change_to_pb(&change, at)
        };
        review.changed.send_replace(change.seq);
        Ok(Response::new(change))
    }

    type WatchReviewChangesStream = Changes;

    async fn watch_review_changes(
        &self,
        request: Request<pb::WatchReviewChangesRequest>,
    ) -> Result<Response<Self::WatchReviewChangesStream>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let (record, session) = self.session(request.session.as_ref())?;
        self.require(&caller, &Call::ReadSession(session))?;
        let review = self.review(&record.id);
        let mut changed = review.changed.subscribe();
        let mut after = request.after_seq;
        let stream = async_stream(move |sender| async move {
            loop {
                let next: Vec<pb::ReviewChange> = {
                    let chat = lock(&review.chat);
                    chat.chat
                        .changes_after(after)
                        .map(|change| wire::change_to_pb(change, chat.at.get(&change.seq).copied().unwrap_or(0)))
                        .collect()
                };
                for change in next {
                    after = change.seq;
                    if sender.send(Ok(change)).await.is_err() {
                        return;
                    }
                }
                tokio::select! {
                    () = sender.closed() => return,
                    moved = changed.changed() => if moved.is_err() { return },
                }
            }
        });
        Ok(Response::new(stream))
    }

    async fn set_agent_switches(
        &self,
        request: Request<pb::SetAgentSwitchesRequest>,
    ) -> Result<Response<pb::AgentSwitches>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let (record, session) = self.session(request.session.as_ref())?;
        self.require_use(&caller, Capability::UserReviewState, &session)?;
        let switches = wire::switches_from_pb(request.switches.as_ref());
        let review = self.review(&record.id);
        let mut chat = lock(&review.chat);
        chat.chat.set_switches(switches);
        Ok(Response::new(wire::switches_to_pb(chat.chat.switches())))
    }
}

/// A stream fed by `produce` through a channel, which ends when the client
/// goes.
fn async_stream<F, Fut>(produce: F) -> Changes
where
    F: FnOnce(tokio::sync::mpsc::Sender<Result<pb::ReviewChange, Status>>) -> Fut,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let (sender, receiver) = tokio::sync::mpsc::channel(16);
    tokio::spawn(produce(sender));
    Box::pin(tokio_stream::wrappers::ReceiverStream::new(receiver))
}

/// Whether the last GetDiff showed `file`.
fn in_diff(shown: &BTreeMap<String, RepoDiff>, file: &FileRef) -> bool {
    shown
        .get(folder_name(&file.repo))
        .is_some_and(|diff| diff.file(&file.path).is_some())
}

/// The selection's lines on the new side, read from the clone, at most
/// [`SELECTED_LINES_MAX`] of them. The old side and anything unreadable
/// give none: the agent can still read the file itself.
async fn selected_text(folder: &Path, place: &humata2_review_diff::Place) -> Vec<String> {
    let Some(selection) = place.selection else {
        return Vec::new();
    };
    if place.side != Side::New {
        return Vec::new();
    }
    let clone = folder.join(folder_name(&place.file.repo));
    let path = place.file.path.clone();
    blocking(move || {
        let (Ok(root), Ok(file)) = (
            std::fs::canonicalize(&clone),
            std::fs::canonicalize(clone.join(&path)),
        ) else {
            return Vec::new();
        };
        if !file.starts_with(&root) {
            return Vec::new();
        }
        let Ok(text) = std::fs::read_to_string(file) else {
            return Vec::new();
        };
        let first = selection.first.max(1);
        let last = selection.last.min(first.saturating_add(SELECTED_LINES_MAX - 1));
        text.lines()
            .skip(usize::try_from(first - 1).unwrap_or(usize::MAX))
            .take(usize::try_from(last + 1 - first).unwrap_or(0))
            .map(str::to_owned)
            .collect()
    })
    .await
    .unwrap_or_default()
}

fn bad_repo(repo: &str) -> Status {
    status::invalid_field(
        "repos",
        pb::invalid_field::Problem::BadFormat,
        format!("{repo:?} isn't a repository of the session"),
        "name the repository as owner/name, like acme/api, or leave repos empty for every one",
    )
}

fn wire_failed(field: &str, error: &wire::WireError) -> Status {
    status::invalid_field(field, pb::invalid_field::Problem::BadFormat, error.to_string(), "")
}

/// A diff that couldn't be computed, in the API's terms. Each message
/// already says what to do.
fn diff_failed(error: &DiffError) -> Status {
    let (code, kind) = match error {
        DiffError::NotARepository { path, .. } => (Code::NotFound, not_found_kind(&path.display().to_string())),
        DiffError::UnknownRepo { repo, .. } => (Code::NotFound, not_found_kind(repo)),
        DiffError::UnknownRevision { .. }
        | DiffError::NoMergeBase { .. }
        | DiffError::NoCommits { .. }
        | DiffError::NoSnapshot { .. }
        | DiffError::SnapshotMissing { .. }
        | DiffError::NotAFork => (
            Code::FailedPrecondition,
            pb::error::Kind::InvalidField(pb::InvalidField {
                field: "base".to_owned(),
                problem: pb::invalid_field::Problem::NotAllowedHere as i32,
            }),
        ),
        DiffError::BadSession { .. } => (
            Code::InvalidArgument,
            pb::error::Kind::InvalidField(pb::InvalidField {
                field: "session".to_owned(),
                problem: pb::invalid_field::Problem::BadFormat as i32,
            }),
        ),
        DiffError::Git { stderr, .. } => (
            Code::FailedPrecondition,
            pb::error::Kind::GitFailed(pb::GitFailed {
                stderr_tail: stderr.clone(),
                ..pb::GitFailed::default()
            }),
        ),
        _ => return status::internal(status::chain(error), ""),
    };
    status::failure(code, status::error(status::chain(error), "", kind))
}

fn not_found_kind(id: &str) -> pb::error::Kind {
    pb::error::Kind::NotFound(pb::NotFound {
        kind: pb::EntityKind::File as i32,
        id: id.to_owned(),
        in_trash: false,
    })
}

/// A review chat call the agent can't make, as the tool result it reads:
/// a switched-off tool is `Denied` (PR-25).
fn chat_failed(error: &ChatError) -> Status {
    if let Some(denied) = wire::denied(error) {
        return status::failure(
            Code::PermissionDenied,
            status::error(
                error.to_string(),
                "the user turns it back on in the session's Info panel, under \"Agent may\"",
                pb::error::Kind::Denied(denied),
            ),
        );
    }
    let (code, field) = match error {
        ChatError::NotInReview { .. } => (Code::FailedPrecondition, "file"),
        ChatError::NoSuchNote { .. } => (Code::NotFound, "note_id"),
        _ => (Code::FailedPrecondition, "undo_id"),
    };
    status::failure(
        code,
        status::error(
            error.to_string(),
            "",
            pb::error::Kind::InvalidField(pb::InvalidField {
                field: field.to_owned(),
                problem: pb::invalid_field::Problem::NotAllowedHere as i32,
            }),
        ),
    )
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T, Status> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| status::internal(format!("a review worker stopped: {error}"), ""))
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_secs()).unwrap_or(i64::MAX))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

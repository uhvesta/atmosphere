//! DiffService, ReviewChatService, and the review chat's side of
//! CommentService and ViewService (D66, SPEC RV-28–RV-34, PR-22–PR-26): a
//! session's diffs, computed with `humata2_review_diff` in its clones on
//! this machine, the reviewed snapshots Mark reviewed takes, the review
//! chat's view of the review, and the notes and plans its agent leaves.
//!
//! What a review showed is kept per session: the last GetDiff's files per
//! repo, which MarkReviewed snapshots and the review chat's tools check
//! files against, and the chat's view, notes and switches. Each change is
//! saved to the store before it is answered, so a restart keeps them
//! (PR-26), with what Undo can still take back and the answers to calls
//! that carried an idempotency key (D86). The feed of changes for
//! WatchReviewChanges lives only as long as the daemon.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use humata2_access::{Call, Capability, Entry, Place, RepoPath, Root, SessionRef, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_review_diff::{
    AnchorError, Base, BaseContext, ChangeKind, ChatError, ChatTool, Checkout, Clones, Compare,
    DiffError, DiffSpec, FileRef, FileSummary, Matching, PlanError, Replaced, RepoDiff, RepoSpec,
    ReviewChat, Side, Undoable, UserNote, ViewState, Viewed, Whitespace, wire,
};
use humata2_store::{ReviewRecord, SessionRecord, Store};
use humata2_store_proto::humata::store::v1 as stored;
use prost::Message;
use tokio::sync::watch;
use tokio_stream::Stream;
use tonic::{Code, Request, Response, Status};

use crate::guard::Guard;
use crate::identity::{self, AgentCaller, Caller};
use crate::status;

/// At most this many lines of a selection go back with review_context.
const SELECTED_LINES_MAX: u32 = 200;

/// A review keeps the answers to this many calls with an idempotency key;
/// a retry comes soon after its call, so older ones are forgotten.
const ANSWERS_MAX: usize = 256;

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
}

/// What a call with an idempotency key answered, so a repeat answers the
/// same without acting again.
#[derive(Clone)]
enum Answer {
    Comment(pb::Comment),
    Dropped(pb::DropCommentResponse),
    Prepared(pb::PreparedView),
    Focused(pb::ReviewChange),
    Marked(pb::MarkReviewedResponse),
}

/// A review's answers by idempotency key, oldest first, at most
/// [`ANSWERS_MAX`].
#[derive(Default)]
struct Answers(Vec<(String, Answer)>);

impl Answers {
    fn get(&self, key: &str) -> Option<&Answer> {
        self.0
            .iter()
            .find(|(held, _)| held == key)
            .map(|(_, answer)| answer)
    }

    fn put(&mut self, key: String, answer: Answer) {
        self.0.retain(|(held, _)| *held != key);
        self.0.push((key, answer));
        let over = self.0.len().saturating_sub(ANSWERS_MAX);
        self.0.drain(..over);
    }
}

/// One session's review.
#[derive(Default)]
struct SessionReview {
    /// The last GetDiff's answer per repo folder.
    shown: Mutex<BTreeMap<String, RepoDiff>>,
    chat: Mutex<Chat>,
    answers: Mutex<Answers>,
    /// The last change's seq, for WatchReviewChanges.
    changed: watch::Sender<u64>,
    /// Held while the review is saved, so saves land in order.
    saving: tokio::sync::Mutex<()>,
}

#[derive(Default)]
struct Chat {
    chat: ReviewChat,
    /// When each change was made, by seq, in Unix seconds.
    at: BTreeMap<u64, i64>,
    /// Whether the app reported the user's view: the review is open in a
    /// review chat, so a plan applies at once (PR-24).
    reported: bool,
}

impl Review {
    pub(crate) fn new(store: Arc<Store>, guard: Arc<Guard>, machine: String) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
                guard,
                machine,
                sessions: Mutex::default(),
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

    /// The session's review: in memory, else as the store saved it.
    fn review(&self, session: &str) -> Result<Arc<SessionReview>, Status> {
        let mut sessions = lock(&self.inner.sessions);
        if let Some(review) = sessions.get(session) {
            return Ok(review.clone());
        }
        let saved = self
            .inner
            .store
            .review(session)
            .map_err(|error| status::store(error, &self.inner.machine, "read the session's review"))?;
        let review = Arc::new(match saved {
            Some(record) => restored(&record),
            None => SessionReview::default(),
        });
        sessions.insert(session.to_owned(), review.clone());
        Ok(review)
    }

    /// Saves the session's review as it is now. Saves of one review land
    /// in the order they're made.
    async fn save(&self, session: &str, review: &SessionReview) -> Result<(), Status> {
        let _saving = review.saving.lock().await;
        let record = saved(session, review);
        let store = self.inner.store.clone();
        blocking(move || store.put_review(&record))
            .await?
            .map_err(|error| status::store(error, &self.inner.machine, "save the session's review"))
    }

    /// Saves the review, then tells the app's feed about change `seq`.
    async fn publish(&self, session: &str, review: &SessionReview, seq: u64) -> Result<(), Status> {
        let saved = self.save(session, review).await;
        review.changed.send_replace(seq);
        saved
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
        let clones = clones(&record, &keys).await?;
        let spec = match &base.base {
            Some(pb::diff_base::Base::OtherSession(other)) => {
                self.other_session_spec(&caller, &clones, &keys, other).await?
            }
            _ => wire::spec_for_base(&base, &keys, |_| None).map_err(|error| wire_failed("base", &error))?,
        };
        let diffs = clones.diff(&spec, &context_of(&record)).await.map_err(|error| diff_failed(&error))?;
        let review = self.review(&record.id)?;
        let moved = {
            let mut shown = lock(&review.shown);
            let mut moved = false;
            for diff in &diffs {
                let before = shown.insert(folder_name(&diff.repo).to_owned(), diff.clone());
                moved |= before.as_ref() != Some(diff);
            }
            moved
        };
        let refreshed = self.refresh_notes(&clones, &review, &diffs).await?;
        if moved || refreshed {
            self.save(&record.id, &review).await?;
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
        let review = self.review(&record.id)?;
        let idempotency = idempotency(request.idempotency_key.as_ref());
        if let Some(Answer::Marked(answer)) = answered(&review, idempotency.as_ref()) {
            return Ok(Response::new(answer));
        }
        let keys = repo_keys(&record, &[]).await?;
        let clones = clones(&record, &keys).await?;
        // RV-28: per repo, the tree the client says it showed, else the
        // last GetDiff's, else the working tree as it is now.
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
        if remember(&review, idempotency, Answer::Marked(answer.clone())) {
            self.save(&record.id, &review).await?;
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
        let review = self.review(&record.id)?;
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
        let review = self.review(&record.id)?;
        {
            let mut chat = lock(&review.chat);
            chat.chat.report(view);
            chat.reported = true;
        }
        self.save(&record.id, &review).await?;
        Ok(Response::new(pb::ReportReviewViewResponse {}))
    }

    async fn focus_review(&self, request: Request<pb::FocusReviewRequest>) -> Result<Response<pb::ReviewChange>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let (record, session) = self.session(request.session.as_ref())?;
        self.require_use(&caller, Capability::ReviewFocus, &session)?;
        let review = self.review(&record.id)?;
        let idempotency = idempotency(request.idempotency_key.as_ref());
        if let Some(Answer::Focused(answer)) = answered(&review, idempotency.as_ref()) {
            return Ok(Response::new(answer));
        }
        let (place, highlights) =
            wire::focus_from_pb(&request).map_err(|error| wire_failed("file", &error))?;
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
        remember(&review, idempotency, Answer::Focused(change.clone()));
        self.publish(&record.id, &review, change.seq).await?;
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
        let review = self.review(&record.id)?;
        let change = {
            let mut chat = lock(&review.chat);
            let change = chat.chat.undo(&request.undo_id).map_err(|error| chat_failed(&error))?;
            let at = now();
            chat.at.insert(change.seq, at);
            wire::change_to_pb(&change, at)
        };
        self.publish(&record.id, &review, change.seq).await?;
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
        let review = self.review(&record.id)?;
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
        let review = self.review(&record.id)?;
        let switches = {
            let mut chat = lock(&review.chat);
            chat.chat.set_switches(switches);
            chat.chat.switches()
        };
        self.save(&record.id, &review).await?;
        Ok(Response::new(wire::switches_to_pb(switches)))
    }
}

/// The review chat's notes for the user (PR-23): CreateComment with the
/// `note` destination, and EditComment and DropComment on those notes.
/// Comments to send to the agent, PR drafts and the rest aren't built yet.
#[tonic::async_trait]
impl pb::comment_service_server::CommentService for Review {
    /// The review chat's notes, the only comments built yet. They have no
    /// status (they are never sent), so a status filter lists none.
    async fn list_comments(
        &self,
        request: Request<pb::ListCommentsRequest>,
    ) -> Result<Response<pb::ListCommentsResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let asked = request.session.clone().or_else(|| own_session(&caller));
        let (record, session) = self.session(asked.as_ref())?;
        self.require(&caller, &Call::ReadSession(session))?;
        let review = self.review(&record.id)?;
        let comments = if request.statuses.is_empty() {
            lock(&review.chat)
                .chat
                .notes()
                .filter(|note| request.file.as_ref().is_none_or(|file| on_file(&note.note.file, file)))
                .map(|note| note_comment(note, &record.id))
                .collect()
        } else {
            Vec::new()
        };
        Ok(Response::new(pb::ListCommentsResponse {
            comments,
            draft_count: 0,
            page: None,
        }))
    }

    async fn create_comment(&self, request: Request<pb::CreateCommentRequest>) -> Result<Response<pb::Comment>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let is_note = matches!(
            request.destination.as_ref().and_then(|destination| destination.destination.as_ref()),
            Some(pb::comment_destination::Destination::Note(_))
        );
        if !is_note {
            return Err(status::not_built(
                "humata.v1.CommentService.CreateComment for comments to the agent or a pull request",
            ));
        }
        let asked = request.session.clone().or_else(|| own_session(&caller));
        let (record, session) = self.session(asked.as_ref())?;
        self.require_use(&caller, Capability::ReviewNote, &session)?;
        let author = note_author(&caller)?;
        let review = self.review(&record.id)?;
        let idempotency = idempotency(request.idempotency_key.as_ref());
        if let Some(Answer::Comment(answer)) = answered(&review, idempotency.as_ref()) {
            return Ok(Response::new(answer));
        }
        let mut note = wire::note_from_anchor(request.anchor.as_ref(), &request.text)
            .map_err(|error| wire_failed("anchor", &error))?;
        lock(&review.chat)
            .chat
            .switches()
            .allow(ChatTool::Note)
            .map_err(|error| chat_failed(&error))?;
        let shown_diff = lock(&review.shown).get(folder_name(&note.file.repo)).cloned();
        let Some(diff) = shown_diff.filter(|diff| diff.file(&note.file.path).is_some()) else {
            return Err(chat_failed(&ChatError::NotInReview {
                repo: note.file.repo.clone(),
                path: note.file.path.clone(),
            }));
        };
        // The diff names the repo as GetDiff was asked; the note follows it.
        note.file.repo.clone_from(&diff.repo);
        self.require_read(&caller, &session, std::slice::from_ref(&diff.repo))?;
        let clones = clones(&record, std::slice::from_ref(&diff.repo)).await?;
        let checkout = clones.get(&diff.repo).map_err(|error| diff_failed(&error))?.clone();
        let matching = Matching {
            whitespace: Whitespace::Counts,
            renames: true,
        };
        let note = checkout
            .anchor_notes(diff, matching, vec![note])
            .await
            .map_err(|error| anchor_failed(&error))?
            .into_iter()
            .next()
            .ok_or_else(|| status::internal("anchoring a note lost it".to_owned(), ""))?;
        let shown = lock(&review.shown).clone();
        let (seq, comment) = {
            let mut chat = lock(&review.chat);
            let change = chat
                .chat
                .note(&author.agent, note, |file| in_diff(&shown, file))
                .map_err(|error| chat_failed(&error))?;
            chat.at.insert(change.seq, now());
            (change.seq, changed_note(&change, &record.id))
        };
        if let Some(comment) = &comment {
            remember(&review, idempotency, Answer::Comment(comment.clone()));
        }
        self.publish(&record.id, &review, seq).await?;
        let comment =
            comment.ok_or_else(|| status::internal("a new note made no note".to_owned(), ""))?;
        Ok(Response::new(comment))
    }

    async fn edit_comment(&self, request: Request<pb::EditCommentRequest>) -> Result<Response<pb::Comment>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let (record, session, id) = self.note_session(&caller, request.id.as_ref())?;
        self.require_use(&caller, Capability::ReviewNote, &session)?;
        let review = self.review(&record.id)?;
        let idempotency = idempotency(request.idempotency_key.as_ref());
        if let Some(Answer::Comment(answer)) = answered(&review, idempotency.as_ref()) {
            return Ok(Response::new(answer));
        }
        let (seq, comment) = {
            let mut chat = lock(&review.chat);
            let change = chat
                .chat
                .edit_note(&id, request.text)
                .map_err(|error| chat_failed(&error))?;
            chat.at.insert(change.seq, now());
            (change.seq, changed_note(&change, &record.id))
        };
        if let Some(comment) = &comment {
            remember(&review, idempotency, Answer::Comment(comment.clone()));
        }
        self.publish(&record.id, &review, seq).await?;
        let comment = comment
            .ok_or_else(|| status::internal("an edited note made no note".to_owned(), ""))?;
        Ok(Response::new(comment))
    }

    async fn drop_comment(
        &self,
        request: Request<pb::DropCommentRequest>,
    ) -> Result<Response<pb::DropCommentResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let (record, session, id) = self.note_session(&caller, request.id.as_ref())?;
        self.require_use(&caller, Capability::ReviewNote, &session)?;
        let review = self.review(&record.id)?;
        let idempotency = idempotency(request.idempotency_key.as_ref());
        if let Some(Answer::Dropped(answer)) = answered(&review, idempotency.as_ref()) {
            return Ok(Response::new(answer));
        }
        let change = {
            let mut chat = lock(&review.chat);
            let change = chat.chat.drop_note(&id).map_err(|error| chat_failed(&error))?;
            chat.at.insert(change.seq, now());
            change
        };
        let answer = pb::DropCommentResponse {
            undo_id: change.undo_id.clone(),
        };
        remember(&review, idempotency, Answer::Dropped(answer.clone()));
        self.publish(&record.id, &review, change.seq).await?;
        Ok(Response::new(answer))
    }

    async fn reattach_comment(
        &self,
        _request: Request<pb::ReattachCommentRequest>,
    ) -> Result<Response<pb::Comment>, Status> {
        Err(status::not_built("humata.v1.CommentService.ReattachComment"))
    }

    async fn send_comments(
        &self,
        _request: Request<pb::SendCommentsRequest>,
    ) -> Result<Response<pb::SendCommentsResponse>, Status> {
        Err(status::not_built("humata.v1.CommentService.SendComments"))
    }

    async fn resolve_comment(&self, _request: Request<pb::ResolveCommentRequest>) -> Result<Response<pb::Comment>, Status> {
        Err(status::not_built("humata.v1.CommentService.ResolveComment"))
    }

    async fn reopen_comment(&self, _request: Request<pb::ReopenCommentRequest>) -> Result<Response<pb::Comment>, Status> {
        Err(status::not_built("humata.v1.CommentService.ReopenComment"))
    }
}

/// A review plan the review chat's agent prepares (PR-24): in an open
/// review chat it applies at once. Draft views aren't built yet.
#[tonic::async_trait]
impl pb::view_service_server::ViewService for Review {
    async fn prepare_view(&self, request: Request<pb::PrepareViewRequest>) -> Result<Response<pb::PreparedView>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let Some(pb::prepare_view_request::Content::ReviewPlan(plan)) = request.content else {
            return Err(status::not_built(
                "humata.v1.ViewService.PrepareView for walkthroughs, board offers and pull request drafts",
            ));
        };
        let asked = own_session(&caller).ok_or_else(|| {
            status::failure(
                Code::FailedPrecondition,
                status::error(
                    "a view is prepared by a session's agent: PrepareView takes the session from the agent's token".to_owned(),
                    "call it from the agent's MCP server (review_prepare), not as the user",
                    pb::error::Kind::InvalidField(pb::InvalidField {
                        field: "review_plan".to_owned(),
                        problem: pb::invalid_field::Problem::NotAllowedHere as i32,
                    }),
                ),
            )
        })?;
        let (record, session) = self.session(Some(&asked))?;
        let author = note_author(&caller)?;
        let review = self.review(&record.id)?;
        if !lock(&review.chat).reported {
            // A draft view is offered in the session's channel.
            self.require(
                &caller,
                &Call::Use {
                    capability: Capability::ViewsPrepare,
                    target: Target::Channel(session.channel.clone()),
                },
            )?;
            return Err(status::not_built(
                "humata.v1.ViewService.PrepareView outside an open review chat (draft views)",
            ));
        }
        self.require_use(&caller, Capability::ReviewOrder, &session)?;
        let idempotency = idempotency(request.idempotency_key.as_ref());
        if let Some(Answer::Prepared(answer)) = answered(&review, idempotency.as_ref()) {
            return Ok(Response::new(answer));
        }
        lock(&review.chat)
            .chat
            .switches()
            .allow(ChatTool::Prepare)
            .map_err(|error| chat_failed(&error))?;
        let mut plan = wire::plan_from_pb(&plan).map_err(|error| wire_failed("review_plan", &error))?;
        let asked_repos: Vec<pb::RepoRef> = plan.spec.repos.iter().map(|repo| wire::repo_ref(&repo.repo)).collect();
        let keys = repo_keys(&record, &asked_repos).await?;
        self.require_read(&caller, &session, &keys)?;
        let clones = clones(&record, &keys).await?;
        let diffs = clones
            .diff(&plan.spec, &context_of(&record))
            .await
            .map_err(|error| diff_failed(&error))?;
        plan.check(&diffs).map_err(|error| plan_failed(&error))?;
        for (diff, spec) in diffs.iter().zip(&plan.spec.repos) {
            let checkout = clones.get(&diff.repo).map_err(|error| diff_failed(&error))?.clone();
            let notes = std::mem::take(&mut plan.notes);
            plan.notes = checkout
                .anchor_notes(diff.clone(), Matching::from(spec), notes)
                .await
                .map_err(|error| anchor_failed(&error))?;
        }
        // What the plan compares is what the review shows from now on.
        {
            let mut shown = lock(&review.shown);
            for diff in &diffs {
                shown.insert(folder_name(&diff.repo).to_owned(), diff.clone());
            }
        }
        let change = {
            let mut chat = lock(&review.chat);
            let change = chat
                .chat
                .prepare(&author.agent, plan)
                .map_err(|error| chat_failed(&error))?;
            chat.at.insert(change.seq, now());
            change
        };
        let answer = pb::PreparedView {
            title: request.title,
            undo_id: change.undo_id.clone(),
            ..pb::PreparedView::default()
        };
        remember(&review, idempotency, Answer::Prepared(answer.clone()));
        self.publish(&record.id, &review, change.seq).await?;
        Ok(Response::new(answer))
    }

    async fn revise_view(&self, _request: Request<pb::ReviseViewRequest>) -> Result<Response<pb::PreparedView>, Status> {
        Err(status::not_built("humata.v1.ViewService.ReviseView"))
    }

    async fn mark_ready(&self, _request: Request<pb::MarkReadyRequest>) -> Result<Response<pb::PreparedView>, Status> {
        Err(status::not_built("humata.v1.ViewService.MarkReady"))
    }

    async fn list_views(
        &self,
        _request: Request<pb::ListViewsRequest>,
    ) -> Result<Response<pb::ListViewsResponse>, Status> {
        Err(status::not_built("humata.v1.ViewService.ListViews"))
    }

    async fn set_view_state(
        &self,
        _request: Request<pb::SetViewStateRequest>,
    ) -> Result<Response<pb::PreparedView>, Status> {
        Err(status::not_built("humata.v1.ViewService.SetViewState"))
    }
}

impl Review {
    /// RV-1's other session: per repo, this clone against the commit the
    /// other session's own clone of it is on, fetched into this one. The
    /// caller must be able to read that session's clones too.
    async fn other_session_spec(
        &self,
        caller: &Caller,
        clones: &Clones,
        keys: &[String],
        other: &pb::SessionId,
    ) -> Result<DiffSpec, Status> {
        let (other_record, other_session) = self.session(Some(other))?;
        self.require_read(caller, &other_session, keys)?;
        let mut repos = Vec::with_capacity(keys.len());
        for key in keys {
            let checkout = clones.get(key).map_err(|error| diff_failed(&error))?;
            let from = other_record.folder.join(folder_name(key));
            let commit = checkout
                .fetch_session_head(from, other_record.id.clone())
                .await
                .map_err(|error| diff_failed(&error))?;
            repos.push(RepoSpec::new(key.clone(), Base::Rev { rev: commit }, Compare::everything()));
        }
        Ok(DiffSpec { repos })
    }

    /// Marks the session's notes outdated where `diffs` changed the lines
    /// they quote, and moves the ones whose lines only moved (RV-34,
    /// PR-26). Returns whether any note changed.
    async fn refresh_notes(&self, clones: &Clones, review: &SessionReview, diffs: &[RepoDiff]) -> Result<bool, Status> {
        let notes: Vec<_> = lock(&review.chat).chat.notes().map(|note| note.note.clone()).collect();
        if notes.is_empty() {
            return Ok(false);
        }
        let mut states = vec![None; notes.len()];
        for diff in diffs {
            let checkout = clones.get(&diff.repo).map_err(|error| diff_failed(&error))?.clone();
            let found = checkout
                .note_states(diff.clone(), notes.clone())
                .await
                .map_err(|error| diff_failed(&error))?;
            for (state, found) in states.iter_mut().zip(found) {
                if state.is_none() {
                    *state = found;
                }
            }
        }
        let mut chat = lock(&review.chat);
        let before: Vec<UserNote> = chat.chat.notes().cloned().collect();
        // A note made or dropped meanwhile: the next GetDiff refreshes them.
        if !before.iter().map(|note| &note.note).eq(notes.iter()) {
            return Ok(false);
        }
        chat.chat.refresh_notes(&states);
        Ok(!chat.chat.notes().eq(before.iter()))
    }

    /// The session and note a note's comment id names: `<session>/note-N`,
    /// or from the session's own agent, `note-N` as review_context lists
    /// it.
    fn note_session(
        &self,
        caller: &Caller,
        id: Option<&pb::CommentId>,
    ) -> Result<(SessionRecord, SessionRef, String), Status> {
        let id = id
            .map(|id| id.value.clone())
            .filter(|id| !id.is_empty())
            .ok_or_else(|| status::missing("id"))?;
        let (asked, note) = match id.split_once('/') {
            Some((session, note)) => (
                pb::SessionId {
                    value: session.to_owned(),
                },
                note.to_owned(),
            ),
            None => (
                own_session(caller).ok_or_else(|| {
                    status::invalid_field(
                        "id",
                        pb::invalid_field::Problem::BadFormat,
                        format!("{id:?} doesn't say whose review it's in"),
                        "name a review chat note as <session>/<note id>, like se_1/note-2",
                    )
                })?,
                id,
            ),
        };
        let (record, session) = self.session(Some(&asked))?;
        Ok((record, session, note))
    }
}

/// The session an agent's token belongs to.
fn own_session(caller: &Caller) -> Option<pb::SessionId> {
    match caller {
        Caller::Agent(agent) => Some(pb::SessionId {
            value: agent.session.id.to_string(),
        }),
        _ => None,
    }
}

/// Notes and plans for the user come from the session's agent.
fn note_author(caller: &Caller) -> Result<&AgentCaller, Status> {
    match caller {
        Caller::Agent(agent) => Ok(agent),
        _ => Err(status::failure(
            Code::FailedPrecondition,
            status::error(
                "notes for the user come from the session's agent in the review chat".to_owned(),
                "ask the agent in the review chat to leave the note or arrange the review",
                pb::error::Kind::InvalidField(pb::InvalidField {
                    field: "destination".to_owned(),
                    problem: pb::invalid_field::Problem::NotAllowedHere as i32,
                }),
            ),
        )),
    }
}

/// Whether a note on `note` is on the file ListComments asked for; a file
/// without a repo matches that path in any repo.
fn on_file(note: &FileRef, file: &pb::RepoPath) -> bool {
    let repo = file
        .repo
        .as_ref()
        .map(|repo| repo.name.as_str())
        .filter(|name| !name.is_empty());
    note.path == file.path && repo.is_none_or(|name| folder_name(&note.repo) == name)
}

/// The call's idempotency key; a review holds its own session's keys.
fn idempotency(key: Option<&pb::IdempotencyKey>) -> Option<String> {
    key.filter(|key| !key.value.is_empty())
        .map(|key| key.value.clone())
}

/// The answer an earlier call with the same idempotency key got.
fn answered(review: &SessionReview, key: Option<&String>) -> Option<Answer> {
    key.and_then(|key| lock(&review.answers).get(key).cloned())
}

/// Keeps `answer` for a repeat of the call; it is saved with the review.
/// Returns whether there was a key.
fn remember(review: &SessionReview, key: Option<String>, answer: Answer) -> bool {
    let Some(key) = key else {
        return false;
    };
    lock(&review.answers).put(key, answer);
    true
}

/// The note a change shows, as the Comment CreateComment and EditComment
/// answer: with its session and author, and that change's id for Undo.
fn changed_note(change: &humata2_review_diff::Change, session: &str) -> Option<pb::Comment> {
    let ChangeKind::Note(note) = &change.what else {
        return None;
    };
    let mut comment = note_comment(note, session);
    comment.undo_id.clone_from(&change.undo_id);
    Some(comment)
}

/// A note as a Comment, with its session and the agent that wrote it.
fn note_comment(note: &UserNote, session: &str) -> pb::Comment {
    let mut comment = wire::user_note_to_pb(note);
    comment.session = Some(pb::SessionId {
        value: session.to_owned(),
    });
    comment.author = Some(pb::Actor {
        actor: Some(pb::actor::Actor::Agent(pb::AgentActor {
            session: comment.session.clone(),
            agent: Some(pb::AgentId {
                value: note.author.clone(),
            }),
            ..pb::AgentActor::default()
        })),
    });
    comment
}

/// The review as the store keeps it: the chat as a ReviewContext, the
/// shown diffs as RepoDiffs.
fn saved(session: &str, review: &SessionReview) -> ReviewRecord {
    let (context, last_seq, last_note, reported, undoable) = {
        let chat = lock(&review.chat);
        (
            chat.chat.context(),
            chat.chat.last_seq(),
            chat.chat.last_note(),
            chat.reported,
            chat.chat.undoable(),
        )
    };
    let history = stored::ReviewHistory {
        undo: undoable
            .iter()
            .map(|entry| undo_to_stored(entry, session))
            .collect(),
        answers: lock(&review.answers)
            .0
            .iter()
            .map(|(key, answer)| answer_to_stored(key, answer))
            .collect(),
    };
    let mut chat = wire::context_to_pb(&context, Vec::new());
    chat.notes = context.notes.iter().map(|note| note_comment(note, session)).collect();
    let shown = lock(&review.shown)
        .values()
        .map(|diff| wire::repo_diff_to_pb(diff).encode_to_vec())
        .collect();
    ReviewRecord {
        session: session.to_owned(),
        chat: chat.encode_to_vec(),
        shown,
        last_seq,
        last_note,
        view_reported: reported,
        history: history.encode_to_vec(),
        updated_unix_ms: now().saturating_mul(1000),
    }
}

fn undo_to_stored(entry: &Undoable, session: &str) -> stored::ReviewUndo {
    use stored::review_undo::Before;
    let view = |view: ViewState| wire::view_to_pb(&view).encode_to_vec();
    let mut added_notes = Vec::new();
    let before = match &entry.before {
        Replaced::Focus { place, highlights } => Before::Focus(view(ViewState {
            open: place.clone(),
            highlights: highlights.clone(),
            ..ViewState::default()
        })),
        Replaced::NoNote(id) => Before::AddedNote(id.clone()),
        Replaced::Note(note) => Before::Note(note_comment(note, session).encode_to_vec()),
        Replaced::Order {
            order,
            groups,
            added,
        } => {
            added_notes.clone_from(added);
            Before::Order(view(ViewState {
                order: order.clone(),
                groups: groups.clone(),
                ..ViewState::default()
            }))
        }
    };
    stored::ReviewUndo {
        undo_id: entry.undo_id.clone(),
        seq: entry.seq,
        undone: entry.undone,
        before: Some(before),
        added_notes,
    }
}

/// A saved Undo entry back, or `None` when this build can't read it.
fn undo_from_stored(entry: stored::ReviewUndo) -> Option<Undoable> {
    use stored::review_undo::Before;
    let view = |bytes: &[u8]| {
        pb::ReviewViewState::decode(bytes)
            .ok()
            .and_then(|view| wire::view_from_pb(&view).ok())
    };
    let before = match entry.before? {
        Before::Focus(bytes) => {
            let view = view(&bytes)?;
            Replaced::Focus {
                place: view.open,
                highlights: view.highlights,
            }
        }
        Before::AddedNote(id) => Replaced::NoNote(id),
        Before::Note(bytes) => Replaced::Note(note_from_comment(
            &pb::Comment::decode(bytes.as_slice()).ok()?,
        )?),
        Before::Order(bytes) => {
            let view = view(&bytes)?;
            Replaced::Order {
                order: view.order,
                groups: view.groups,
                added: entry.added_notes,
            }
        }
    };
    Some(Undoable {
        undo_id: entry.undo_id,
        seq: entry.seq,
        undone: entry.undone,
        before,
    })
}

fn answer_to_stored(key: &str, answer: &Answer) -> stored::ReviewAnswer {
    use stored::review_answer::Answer as Kept;
    let kept = match answer {
        Answer::Comment(comment) => Kept::Comment(comment.encode_to_vec()),
        Answer::Dropped(dropped) => Kept::Dropped(dropped.encode_to_vec()),
        Answer::Prepared(prepared) => Kept::Prepared(prepared.encode_to_vec()),
        Answer::Focused(change) => Kept::Focused(change.encode_to_vec()),
        Answer::Marked(marked) => Kept::Marked(marked.encode_to_vec()),
    };
    stored::ReviewAnswer {
        key: key.to_owned(),
        answer: Some(kept),
    }
}

/// A saved answer back, or `None` when this build can't read it.
fn answer_from_stored(answer: stored::ReviewAnswer) -> Option<(String, Answer)> {
    use stored::review_answer::Answer as Kept;
    let back = match answer.answer? {
        Kept::Comment(bytes) => Answer::Comment(pb::Comment::decode(bytes.as_slice()).ok()?),
        Kept::Dropped(bytes) => {
            Answer::Dropped(pb::DropCommentResponse::decode(bytes.as_slice()).ok()?)
        }
        Kept::Prepared(bytes) => Answer::Prepared(pb::PreparedView::decode(bytes.as_slice()).ok()?),
        Kept::Focused(bytes) => Answer::Focused(pb::ReviewChange::decode(bytes.as_slice()).ok()?),
        Kept::Marked(bytes) => {
            Answer::Marked(pb::MarkReviewedResponse::decode(bytes.as_slice()).ok()?)
        }
    };
    Some((answer.key, back))
}

/// A saved note back, with the agent that wrote it.
fn note_from_comment(comment: &pb::Comment) -> Option<UserNote> {
    let author = match comment
        .author
        .as_ref()
        .and_then(|author| author.actor.as_ref())
    {
        Some(pb::actor::Actor::Agent(agent)) => agent.agent.as_ref().map(|id| id.value.clone()),
        _ => None,
    };
    wire::user_note_from_pb(comment, author.as_deref().unwrap_or_default()).ok()
}

/// A review back from the store. What can't be read back starts empty,
/// and the daemon's log says so: the review still opens.
fn restored(record: &ReviewRecord) -> SessionReview {
    let lost = |what: &str| {
        crate::log::say!(
            "humatad2: session {}'s saved review had {what} this build can't read, so the review chat starts without it",
            record.session
        );
    };
    let context = pb::ReviewContext::decode(record.chat.as_slice()).unwrap_or_else(|_| {
        lost("a chat");
        pb::ReviewContext::default()
    });
    let view = context
        .view
        .as_ref()
        .map(wire::view_from_pb)
        .transpose()
        .unwrap_or_else(|_| {
            lost("a view");
            None
        })
        .unwrap_or_default();
    let notes: Vec<UserNote> = context
        .notes
        .iter()
        .filter_map(|comment| {
            let note = note_from_comment(comment);
            if note.is_none() {
                lost("a note");
            }
            note
        })
        .collect();
    let shown = record
        .shown
        .iter()
        .filter_map(|bytes| {
            let diff = pb::RepoDiff::decode(bytes.as_slice()).ok().and_then(|diff| wire::repo_diff_from_pb(&diff));
            if diff.is_none() {
                lost("a diff");
            }
            diff
        })
        .map(|diff| (folder_name(&diff.repo).to_owned(), diff))
        .collect();
    let switches = wire::switches_from_pb(context.switches.as_ref());
    let history = stored::ReviewHistory::decode(record.history.as_slice()).unwrap_or_else(|_| {
        lost("an Undo history");
        stored::ReviewHistory::default()
    });
    let undoable: Vec<Undoable> = history
        .undo
        .into_iter()
        .filter_map(|entry| {
            let entry = undo_from_stored(entry);
            if entry.is_none() {
                lost("a change to undo");
            }
            entry
        })
        .collect();
    let mut answers = Answers::default();
    for answer in history.answers {
        match answer_from_stored(answer) {
            Some((key, answer)) => answers.put(key, answer),
            None => lost("an answer to a repeated call"),
        }
    }
    SessionReview {
        shown: Mutex::new(shown),
        chat: Mutex::new(Chat {
            chat: ReviewChat::restore(view, switches, notes, record.last_seq, record.last_note)
                .with_undoable(undoable),
            at: BTreeMap::new(),
            reported: record.view_reported,
        }),
        answers: Mutex::new(answers),
        ..SessionReview::default()
    }
}

/// A plan that doesn't fit its diff: the agent fixes the plan.
fn plan_failed(error: &PlanError) -> Status {
    status::failure(
        Code::FailedPrecondition,
        status::error(
            error.to_string(),
            "",
            pb::error::Kind::InvalidField(pb::InvalidField {
                field: "review_plan".to_owned(),
                problem: pb::invalid_field::Problem::NotAllowedHere as i32,
            }),
        ),
    )
}

fn anchor_failed(error: &AnchorError) -> Status {
    match error {
        AnchorError::Plan(error) => plan_failed(error),
        AnchorError::Diff(error) => diff_failed(error),
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
        | DiffError::NotAFork
        | DiffError::OtherSession { .. } => (
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

//! DiffService, ReviewChatService, and CommentService and ViewService (D66,
//! SPEC RV-28–RV-34, PR-22–PR-26, SM-7–SM-21): a session's diffs, computed
//! with `humata2_review_diff` in its clones on this machine, the reviewed
//! snapshots Mark reviewed takes, the review chat's view of the review, the
//! notes and plans its agent leaves, and the user's comments for the
//! agent (`comments`), typed into its terminal through a `CommentSink`.
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
use humata2_daemon_ports::Caller;
use humata2_grpc::humata::v1 as pb;
use humata2_review_diff::{
    AnchorError, Base, BaseContext, ChangeKind, ChatError, ChatTool, Checkout, Clones, CommentLedger, Compare,
    DiffError, DiffSpec, FileRef, FileSummary, Matching, PlanError, Replaced, RepoDiff, RepoSpec,
    ReviewChat, Side, StackBinary, StackStatus, Undoable, UserNote, ViewState, Viewed, Whitespace, wire,
};
use humata2_rpc_kit::log::say;
use humata2_store::{ReviewRecord, SessionRecord, Store};
use humata2_store_proto::humata::store::v1 as stored;
use prost::Message;
use tokio::sync::{broadcast, watch};
use tokio_stream::Stream;
use tonic::{Code, Request, Response, Status};

pub mod ports;
mod bases;
mod comments;
mod default_branch;
mod pull_requests;
mod replay;
mod sink;
mod stack;
mod views;

pub use comments::{Doubtful, RecoveryState, UserWriteGuard};
pub use pull_requests::{PR_REVIEWS_FOLDER, PullRequests, ReviewRemote, binding};
pub use replay::Replayed;
pub use sink::{CommentSink, TerminalSink};

use crate::ports::{
    AgentCaller, ReviewFeed, ReviewGuard, Scope, actor, blocking, call_failed, caller_of, session_ref,
};

/// At most this many lines of a selection go back with review_context.
const SELECTED_LINES_MAX: u32 = 200;

/// Comments per ListComments page when the caller names no size.
const COMMENTS_PAGE_DEFAULT: usize = 50;

/// The most comments in one ListComments page.
const COMMENTS_PAGE_MAX: usize = 500;

/// A review keeps the answers to this many calls with an idempotency key;
/// a retry comes soon after its call, so older ones are forgotten.
const ANSWERS_MAX: usize = 256;

/// The services' shared state: per session, what its review showed.
#[derive(Clone)]
pub struct Review {
    inner: Arc<Inner>,
}

struct Inner {
    store: Arc<Store>,
    guard: Arc<dyn ReviewGuard>,
    machine: String,
    /// Where offered views are told (D87).
    feed: Arc<dyn ReviewFeed>,
    sessions: Mutex<HashMap<String, Arc<SessionReview>>>,
    /// Held while a view is read, changed and saved, so two calls on one
    /// view, or two with one idempotency key, don't both act.
    views: tokio::sync::Mutex<()>,
    /// Where batches of the user's comments are typed (SM-9).
    sink: Arc<dyn CommentSink>,
    /// Reads a repo's stack metadata for `from_stack` reviews (D803).
    stack_status: Arc<dyn StackStatus>,
    /// Where an offered view is journaled for the host (VW-12); unset on
    /// the host and in tests, which journal nothing.
    view_journal: std::sync::OnceLock<Arc<dyn ports::ViewJournal>>,
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
    /// The stack as it resolved at prepare, and where it was read from.
    stacked: Mutex<stack::Stacked>,
    /// Held while the review is saved, so saves land in order.
    saving: tokio::sync::Mutex<()>,
    /// The notes as the event feed was last told of them, by id.
    told: Mutex<BTreeMap<String, pb::Comment>>,
    /// The user's comments for the agent and the batches being typed.
    comments: Mutex<CommentLedger>,
    /// What of the comments was last saved, so a save writes only what
    /// changed (their own table, not the review's blob).
    comments_kept: Mutex<comments::Kept>,
    /// Held while the comments are saved, so saves land in order.
    comments_saving: tokio::sync::Mutex<()>,
    /// The user's writes of files under way, which a look at the comments waits out.
    user_writes: comments::Writes,
    /// Each change the comments' saves make, for WatchComments (D161).
    comment_feed: CommentFeed,
}

/// The most changes a WatchComments stream can fall behind by before it is
/// told to list again.
const COMMENT_FEED_CAPACITY: usize = 256;

/// Where saved changes to a session's comments are told, in order.
struct CommentFeed(broadcast::Sender<pb::CommentFeedEvent>);

impl Default for CommentFeed {
    fn default() -> Self {
        Self(broadcast::channel(COMMENT_FEED_CAPACITY).0)
    }
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
    pub fn new(
        store: Arc<Store>,
        guard: Arc<dyn ReviewGuard>,
        machine: String,
        feed: Arc<dyn ReviewFeed>,
        sink: Arc<dyn CommentSink>,
    ) -> Self {
        Self::with_stack_status(store, guard, machine, feed, sink, Arc::new(StackBinary))
    }

    /// As [`Self::new`], reading stacks through `stack_status` instead of
    /// the `stack` tool on PATH.
    pub fn with_stack_status(
        store: Arc<Store>,
        guard: Arc<dyn ReviewGuard>,
        machine: String,
        feed: Arc<dyn ReviewFeed>,
        sink: Arc<dyn CommentSink>,
        stack_status: Arc<dyn StackStatus>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
                guard,
                machine,
                feed,
                sessions: Mutex::default(),
                views: tokio::sync::Mutex::default(),
                sink,
                stack_status,
                view_journal: std::sync::OnceLock::new(),
            }),
        }
    }

    /// VW-12: journal each view this machine's agents offer for the host.
    /// A second call is ignored.
    pub fn use_view_journal(&self, journal: Arc<dyn ports::ViewJournal>) {
        let _ = self.inner.view_journal.set(journal);
    }

    /// PR-10: the ref each of `keys` is compared with when `base` is each
    /// pull request's base, or a spec puts a repo on it; empty for every other
    /// base.
    async fn pull_request_bases(
        &self,
        record: &SessionRecord,
        base: &pb::DiffBase,
        keys: &[String],
    ) -> Result<BTreeMap<String, String>, Status> {
        let asks = match &base.base {
            Some(pb::diff_base::Base::PullRequestBase(_)) => true,
            Some(pb::diff_base::Base::Spec(spec)) => spec.repos.iter().any(|own| {
                matches!(
                    own.base.as_ref().and_then(|base| base.base.as_ref()),
                    Some(pb::diff_spec_base::Base::PullRequestBase(_))
                )
            }),
            _ => false,
        };
        if !asks {
            return Ok(BTreeMap::new());
        }
        let store = self.inner.store.clone();
        let machine = self.inner.machine.clone();
        let (session, folder, keys) = (record.id.clone(), record.folder.clone(), keys.to_vec());
        blocking(move || {
            pull_requests::pull_request_bases_blocking(&store, &machine, &session, &folder, &keys)
        })
        .await?
    }

    /// `base` for `keys` of a session on another machine, with each pull
    /// request base written out as the ref the host resolved: the host keeps
    /// the review's pull requests, the machine computing the diff doesn't.
    async fn written_out(
        &self,
        record: &SessionRecord,
        base: Option<&pb::DiffBase>,
        keys: &[String],
    ) -> Result<pb::DiffBase, Status> {
        let base = base.cloned().unwrap_or_default();
        let pr_bases = self.pull_request_bases(record, &base, keys).await?;
        wire::with_pull_request_bases(&base, keys, |repo| pr_bases.get(repo).cloned())
            .map_err(|error| wire_failed("base", &error))
    }

    /// The session `id` names, from the store.
    fn session(&self, id: Option<&pb::SessionId>) -> Result<(SessionRecord, SessionRef), Status> {
        let id = id
            .map(|id| id.value.clone())
            .filter(|id| !id.is_empty())
            .ok_or_else(|| ports::missing("session"))?;
        match self.inner.store.session(&id) {
            Ok(Some(record)) => {
                let session = session_ref(&record, &self.inner.machine);
                Ok((record, session))
            }
            Ok(None) => Err(ports::not_found(
                pb::EntityKind::Session,
                &id,
                format!("no session {id} on this machine"),
                "check the session id; diffs are computed on the session's own machine, so ask its daemon",
            )),
            Err(error) => Err(ports::store_error(error, &self.inner.machine, "look up the session")),
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
            .map_err(|error| ports::store_error(error, &self.inner.machine, "read the session's review"))?;
        let review = Arc::new(match saved {
            Some(record) => restored(&record),
            None => SessionReview::default(),
        });
        let (ledger, kept) = comments::load(&self.inner.store, session)
            .map_err(|error| ports::store_error(error, &self.inner.machine, "read the session's comments"))?;
        *lock(&review.comments) = ledger;
        *lock(&review.comments_kept) = kept;
        // The feed hears of changes from here on, not of notes made before.
        *lock(&review.told) = notes_now(&review, session);
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
            .map_err(|error| ports::store_error(error, &self.inner.machine, "save the session's review"))
    }

    /// Saves the review, then tells WatchReviewChanges about change `seq`
    /// and the event feed about each note it made, changed or took away.
    /// `caller` made the change.
    async fn publish(
        &self,
        caller: &Caller,
        record: &SessionRecord,
        review: &SessionReview,
        seq: u64,
    ) -> Result<(), Status> {
        let saved = self.save(&record.id, review).await;
        review.changed.send_replace(seq);
        self.tell_notes(record, review, &actor(caller));
        saved
    }

    /// Tells the feed how the notes differ from when it was last told:
    /// Created, Updated, or Deleted with the note as it last stood.
    fn tell_notes(&self, record: &SessionRecord, review: &SessionReview, by: &pb::Actor) {
        let now = notes_now(review, &record.id);
        // Held while publishing, so two calls tell in the order they saw.
        let mut told = lock(&review.told);
        let feed = &self.inner.feed;
        for (id, comment) in &now {
            let change = match told.get(id) {
                None => pb::Change::Created,
                Some(before) if before != comment => pb::Change::Updated,
                Some(_) => continue,
            };
            feed.send(
                Scope::Session(record.id.clone()),
                change,
                pb::event::Payload::Comment(comment.clone()),
                Some(by.clone()),
            );
        }
        for (id, comment) in told.iter() {
            if !now.contains_key(id) {
                feed.send(
                    Scope::Session(record.id.clone()),
                    pb::Change::Deleted,
                    pb::event::Payload::Comment(comment.clone()),
                    Some(by.clone()),
                );
            }
        }
        *told = now;
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

    /// `plan` checked against the diffs its spec makes now, its notes
    /// anchored, with those diffs.
    async fn checked_plan(
        &self,
        caller: &Caller,
        record: &SessionRecord,
        session: &SessionRef,
        plan: &pb::ReviewPlan,
    ) -> Result<(humata2_review_diff::ReviewPlan, Vec<RepoDiff>), Status> {
        let (plan, diffs, _) = self.checked_plan_with_stack(caller, record, session, plan).await?;
        Ok((plan, diffs))
    }

    /// As [`Self::checked_plan`], with the checked stack when the plan sent
    /// `stack` or `from_stack` (D800, D803).
    async fn checked_plan_with_stack(
        &self,
        caller: &Caller,
        record: &SessionRecord,
        session: &SessionRef,
        plan: &pb::ReviewPlan,
    ) -> Result<(humata2_review_diff::ReviewPlan, Vec<RepoDiff>, Option<humata2_review_diff::CheckedStack>), Status> {
        let stacked = plan.stack.is_some() || plan.from_stack.is_some();
        let mut sent = plan.clone();
        if stacked {
            if sent.spec.as_ref().is_some_and(|spec| !spec.repos.is_empty()) {
                return Err(ports::invalid_field(
                    "spec",
                    pb::invalid_field::Problem::NotAllowedHere,
                    "a review plan sends a stack or a spec, not both",
                    "drop `spec`; the daemon shows the whole stack",
                ));
            }
            sent.spec = Some(pb::DiffSpec::default());
        }
        let mut plan = wire::plan_from_pb(&sent).map_err(|error| wire_failed("review_plan", &error))?;
        plan.check_importance()
            .map_err(|error| importance_failed("review_plan.importance", &error))?;
        if stacked {
            let (plan, diffs, checked) = self.checked_stack_plan(caller, record, session, plan).await?;
            return Ok((plan, diffs, Some(checked)));
        }
        let asked_repos: Vec<pb::RepoRef> = plan.spec.repos.iter().map(|repo| wire::repo_ref(&repo.repo)).collect();
        let keys = repo_keys(&self.inner, record, &asked_repos).await?;
        self.require_read(caller, session, &keys)?;
        let clones = clones(record, &keys).await?;
        let diffs = clones
            .diff(&plan.spec, &context_of(record))
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
        Ok((plan, diffs, None))
    }
}

/// The repositories asked for, else every clone in the session's folder,
/// each as `owner/name` when the session's start recorded its owner (the
/// key comments, reviews and the app's repo names use) and as the folder
/// name otherwise.
async fn repo_keys(inner: &Inner, record: &SessionRecord, asked: &[pb::RepoRef]) -> Result<Vec<String>, Status> {
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
    let owners: BTreeMap<String, String> = inner
        .store
        .session_detail(&record.id)
        .map_err(|error| ports::store_error(error, &inner.machine, "read the session's repositories"))?
        .map(|detail| {
            detail
                .repos
                .into_iter()
                .filter(|repo| !repo.owner.is_empty())
                .map(|repo| (repo.name, repo.owner))
                .collect()
        })
        .unwrap_or_default();
    if let Some(machine) = pull_requests::machine_of(&record.id) {
        return Err(not_here(&machine));
    }
    let folder = record.folder.clone();
    blocking(move || {
        let entries = std::fs::read_dir(&folder).map_err(|error| {
            ports::internal(format!("couldn't list the session's folder {}: {error}", folder.display()), "")
        })?;
        let mut keys: Vec<String> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().join(".git").exists())
            .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
            .map(|name| match owners.get(&name) {
                Some(owner) => format!("{owner}/{name}"),
                None => name,
            })
            .collect();
        keys.sort();
        Ok(keys)
    })
    .await?
}

/// Every repo the session's start recorded, as [`repo_keys`] names them,
/// for a session whose clones are on another machine and so can't be
/// listed here.
async fn recorded_keys(inner: &Inner, record: &SessionRecord) -> Result<Vec<String>, Status> {
    let mut keys: Vec<String> = inner
        .store
        .session_detail(&record.id)
        .map_err(|error| ports::store_error(error, &inner.machine, "read the session's repositories"))?
        .map(|detail| {
            detail
                .repos
                .into_iter()
                .map(|repo| {
                    if repo.owner.is_empty() {
                        repo.name
                    } else {
                        format!("{}/{}", repo.owner, repo.name)
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    keys.sort();
    keys.dedup();
    Ok(keys)
}

/// D736: of a session on another machine, the host forwards only GetDiff
/// and GetFileDiff; everything else that reads its clones is refused, so
/// none of it reads the host's empty placement of the folder.
fn not_here(machine: &str) -> Status {
    ports::failure(
        Code::Unimplemented,
        pb::Error {
            message: format!(
                "this session runs on {machine}, and Mark reviewed, review plans and line notes don't work yet for a session on another machine"
            ),
            fix: "use the Diff panel and comments, which do work here; Mark reviewed and review plans work for sessions on this machine".to_owned(),
            retry: None,
            kind: None,
        },
    )
}

/// The folder of a repo's clone: its name, without the owner.
fn folder_name(key: &str) -> &str {
    key.rsplit_once('/').map_or(key, |(_, name)| name)
}

/// The session's clones of `keys`, opened.
async fn clones(record: &SessionRecord, keys: &[String]) -> Result<Clones, Status> {
    if let Some(machine) = pull_requests::machine_of(&record.id) {
        return Err(not_here(&machine));
    }
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
        let caller = caller_of(&request)?;
        let request = request.into_inner();
        let (record, session) = self.session(request.session.as_ref())?;
        if let Some((machine, relayed)) = pull_requests::relayed_for(&record.id) {
            let keys = if request.repos.is_empty() {
                recorded_keys(&self.inner, &record).await?
            } else {
                repo_keys(&self.inner, &record, &request.repos).await?
            };
            self.require_read(&caller, &session, &keys)?;
            let base = self.written_out(&record, request.base.as_ref(), &keys).await?;
            let sent = pb::GetDiffRequest {
                base: Some(base),
                ..request
            };
            return relayed
                .get_diff(&machine, sent)
                .await
                .map(Response::new)
                .map_err(|status| call_failed(&machine, status));
        }
        // A spec is checked against every repo of the session, then cut down to
        // the repos asked for (B2).
        let session_keys = repo_keys(&self.inner, &record, &[]).await?;
        let keys = if request.repos.is_empty() {
            session_keys.clone()
        } else {
            repo_keys(&self.inner, &record, &request.repos).await?
        };
        self.require_read(&caller, &session, &keys)?;
        let base = request.base.unwrap_or_default();
        let clones = clones(&record, &keys).await?;
        let spec = match &base.base {
            Some(pb::diff_base::Base::OtherSession(other)) => {
                self.other_session_spec(&caller, &clones, &keys, other).await?
            }
            _ => {
                let defaults = clones
                    .default_bases(&wire::repos_needing_default(&base, &keys))
                    .await
                    .map_err(|error| default_branch::default_failed(&error, &keys))?;
                let pr_bases = self.pull_request_bases(&record, &base, &keys).await?;
                wire::spec_for_base(&base, &keys, &session_keys, &defaults, |repo| {
                    pr_bases.get(repo).cloned()
                })
                .map_err(|error| wire_failed("base", &error))?
            }
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
        match self.refresh_notes(&clones, &review, &diffs).await? {
            Some(seq) => self.publish(&caller, &record, &review, seq).await?,
            None if moved => self.save(&record.id, &review).await?,
            None => {}
        }
        self.refreshed_comments(&record, &review).await?;
        let agent = lock(&review.chat).chat.view().agent_importance();
        let (diffs, tiers) = served_tiers(&clones, diffs, agent).await?;
        Ok(Response::new(pb::GetDiffResponse {
            any_changes: diffs.iter().any(|diff| !diff.files.is_empty()),
            repos: diffs
                .iter()
                .zip(&tiers)
                .map(|(diff, tiers)| wire::repo_diff_to_pb_with_tiers(diff, tiers))
                .collect(),
        }))
    }

    async fn get_file_diff(&self, request: Request<pb::GetFileDiffRequest>) -> Result<Response<pb::FileDiff>, Status> {
        let caller = caller_of(&request)?;
        let request = request.into_inner();
        let (record, session) = self.session(request.session.as_ref())?;
        let file = request.file.clone().ok_or_else(|| ports::missing("file"))?;
        let repo = file.repo.clone().ok_or_else(|| ports::missing("file.repo"))?;
        if file.path.is_empty() {
            return Err(ports::missing("file.path"));
        }
        let keys = repo_keys(&self.inner, &record, std::slice::from_ref(&repo)).await?;
        self.require_read(&caller, &session, &keys)?;
        if let Some((machine, relayed)) = pull_requests::relayed_for(&record.id) {
            let base = self.written_out(&record, request.base.as_ref(), &keys).await?;
            let sent = pb::GetFileDiffRequest {
                base: Some(base),
                ..request
            };
            return relayed
                .get_file_diff(&machine, sent)
                .await
                .map(Response::new)
                .map_err(|status| call_failed(&machine, status));
        }
        let base = request.base.clone().unwrap_or_default();
        let session_keys = repo_keys(&self.inner, &record, &[]).await?;
        let clones = clones(&record, &keys).await?;
        let defaults = clones
            .default_bases(&wire::repos_needing_default(&base, &keys))
            .await
            .map_err(|error| default_branch::default_failed(&error, &keys))?;
        let pr_bases = self.pull_request_bases(&record, &base, &keys).await?;
        let spec = wire::spec_for_base(&base, &keys, &session_keys, &defaults, |repo| {
            pr_bases.get(repo).cloned()
        })
        .map_err(|error| wire_failed("base", &error))?;
        let key = wire::repo_key(&repo);
        let checkout = clones.get(&key).map_err(|error| diff_failed(&error))?.clone();
        // The file's own repo's spec (RV-31), never another repo's.
        let repo_spec = spec
            .repos
            .iter()
            .find(|own| own.repo == key)
            .cloned()
            .ok_or_else(|| ports::missing("file.repo"))?;
        let matching = Matching::from(&repo_spec);
        // The file list the hunks belong to: the trees the client was
        // shown, listed again without a new snapshot (DV-4), or the spec
        // as it is now.
        let (trees, listed) = match wire::trees_from_pb(request.trees.as_ref()) {
            Some(trees) => {
                let files = checkout
                    .files_between(trees.clone(), repo_spec.clone())
                    .await
                    .map_err(|error| diff_failed(&error))?;
                (trees, files.into_iter().find(|listed| listed.path == file.path))
            }
            None => {
                let diff = checkout
                    .diff(repo_spec, context_of(&record))
                    .await
                    .map_err(|error| diff_failed(&error))?;
                (diff.trees.clone(), diff.file(&file.path).cloned())
            }
        };
        let summary: FileSummary = listed.ok_or_else(|| {
            ports::not_found(
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
        let caller = caller_of(&request)?;
        let request = request.into_inner();
        let (record, session) = self.session(request.session.as_ref())?;
        self.require_use(&caller, Capability::UserReviewState, &session)?;
        let review = self.review(&record.id)?;
        let idempotency = idempotency(request.idempotency_key.as_ref());
        if let Some(Answer::Marked(answer)) = answered(&review, idempotency.as_ref()) {
            return Ok(Response::new(answer));
        }
        let keys = repo_keys(&self.inner, &record, &[]).await?;
        let clones = clones(&record, &keys).await?;
        // RV-28: per repo, the tree the client says it showed, else the
        // last GetDiff's, else the working tree as it is now.
        let mut trees: BTreeMap<String, String> = lock(&review.shown)
            .iter()
            .map(|(folder, diff)| (folder.clone(), diff.trees.new.clone()))
            .collect();
        for shown in &request.shown_tree {
            let repo = shown.repo.as_ref().ok_or_else(|| ports::missing("shown_tree.repo"))?;
            if !shown.tree.is_empty() {
                trees.insert(folder_name(&wire::repo_key(repo)).to_owned(), shown.tree.clone());
            }
        }
        let mut snapshots = Vec::with_capacity(keys.len());
        let mut taken = Vec::with_capacity(keys.len());
        for key in &keys {
            let checkout = clones.get(key).map_err(|error| diff_failed(&error))?;
            let tree = trees.get(folder_name(key)).cloned();
            let snapshot = checkout
                .mark_reviewed(record.id.clone(), tree)
                .await
                .map_err(|error| diff_failed(&error))?;
            snapshots.push(wire::snapshot_to_pb(key, &snapshot));
            taken.push((key.clone(), snapshot.n));
        }
        // A9, D299: in a pull request review this marks every commit up to
        // the head too, and keeps the snapshot it was marked at past RV-30's
        // last 20.
        let (store, machine) = (self.inner.store.clone(), self.inner.machine.clone());
        let (id, folder) = (record.id.clone(), record.folder.clone());
        let by = actor(&caller);
        let whole = blocking(move || {
            pull_requests::mark_whole_pr_blocking(&store, &machine, &id, &folder, &taken, &by)
        })
        .await??;
        for (key, snapshot) in keys.iter().zip(&mut snapshots) {
            if whole.iter().any(|(marked, n)| marked == key && *n == snapshot.n) {
                let checkout = clones.get(key).map_err(|error| diff_failed(&error))?;
                checkout
                    .pin(record.id.clone(), snapshot.n, true)
                    .await
                    .map_err(|error| diff_failed(&error))?;
                snapshot.pinned = true;
            }
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
        let caller = caller_of(&request)?;
        let request = request.into_inner();
        let (record, session) = self.session(request.session.as_ref())?;
        let keys = repo_keys(&self.inner, &record, &request.repos).await?;
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

    async fn get_looks(
        &self,
        request: Request<pb::GetLooksRequest>,
    ) -> Result<Response<pb::GetLooksResponse>, Status> {
        let caller = caller_of(&request)?;
        let request = request.into_inner();
        let (record, session) = self.session(request.session.as_ref())?;
        let keys = repo_keys(&self.inner, &record, &[]).await?;
        self.require_read(&caller, &session, &keys)?;
        let clones = clones(&record, &keys).await?;
        let mut looks = Vec::new();
        for key in &keys {
            let checkout = clones.get(key).map_err(|error| diff_failed(&error))?;
            let kept = checkout.looks(record.id.clone()).await.map_err(|error| diff_failed(&error))?;
            looks.extend(kept.into_iter().map(|look| pb::LookedFile {
                repo: Some(wire::repo_ref(key)),
                path: look.path,
                fingerprint: look.fingerprint,
            }));
        }
        Ok(Response::new(pb::GetLooksResponse { looks }))
    }

    async fn save_looks(
        &self,
        request: Request<pb::SaveLooksRequest>,
    ) -> Result<Response<pb::SaveLooksResponse>, Status> {
        let caller = caller_of(&request)?;
        let request = request.into_inner();
        let (record, session) = self.session(request.session.as_ref())?;
        self.require_use(&caller, Capability::UserReviewState, &session)?;
        let keys = repo_keys(&self.inner, &record, &[]).await?;
        let mut by_repo: BTreeMap<String, Vec<humata2_review_diff::Look>> = BTreeMap::new();
        for look in request.looks {
            let repo = look.repo.as_ref().ok_or_else(|| ports::missing("looks.repo"))?;
            let asked = wire::repo_key(repo);
            let Some(key) = keys.iter().find(|known| folder_name(known) == folder_name(&asked)).cloned() else {
                return Err(ports::invalid_field(
                    "looks.repo",
                    pb::invalid_field::Problem::BadFormat,
                    format!("{asked} isn't a repository of this session"),
                    "send looks only for the session's own repositories, as owner/name",
                ));
            };
            by_repo.entry(key).or_default().push(humata2_review_diff::Look {
                path: look.path,
                fingerprint: look.fingerprint,
            });
        }
        let clones = clones(&record, &keys).await?;
        // A repo with no looks in the request has its saved ones forgotten.
        for key in &keys {
            let checkout = clones.get(key).map_err(|error| diff_failed(&error))?;
            let looks = by_repo.remove(key).unwrap_or_default();
            checkout
                .save_looks(record.id.clone(), looks)
                .await
                .map_err(|error| diff_failed(&error))?;
        }
        Ok(Response::new(pb::SaveLooksResponse {}))
    }

    async fn list_bases(&self, request: Request<pb::ListBasesRequest>) -> Result<Response<pb::ListBasesResponse>, Status> {
        let caller = caller_of(&request)?;
        let request = request.into_inner();
        let (record, session) = self.session(request.session.as_ref())?;
        let repo = request.repo.ok_or_else(|| ports::missing("repo"))?;
        bases::list(self, &caller, &record, &session, &repo).await.map(Response::new)
    }

    async fn find_default_branch(
        &self,
        request: Request<pb::FindDefaultBranchRequest>,
    ) -> Result<Response<pb::FindDefaultBranchResponse>, Status> {
        let caller = caller_of(&request)?;
        let request = request.into_inner();
        let (record, session) = self.session(request.session.as_ref())?;
        let repo = request.repo.ok_or_else(|| ports::missing("repo"))?;
        default_branch::find(self, &caller, &record, &session, &repo).await.map(Response::new)
    }
}

type Changes = Pin<Box<dyn Stream<Item = Result<pb::ReviewChange, Status>> + Send>>;

type CommentEvents = Pin<Box<dyn Stream<Item = Result<pb::CommentFeedEvent, Status>> + Send>>;

#[tonic::async_trait]
impl pb::review_chat_service_server::ReviewChatService for Review {
    async fn get_review_context(
        &self,
        request: Request<pb::GetReviewContextRequest>,
    ) -> Result<Response<pb::ReviewContext>, Status> {
        let caller = caller_of(&request)?;
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
        let mut answer = wire::context_to_pb(&context, selected);
        if let Some(stack) = context.view.stack.as_ref()
            && let Some(told) = self.context_stack(&record, &review, stack).await
            && let Some(view) = answer.view.as_mut()
        {
            view.stack = Some(told);
        }
        Ok(Response::new(answer))
    }

    async fn report_review_view(
        &self,
        request: Request<pb::ReportReviewViewRequest>,
    ) -> Result<Response<pb::ReportReviewViewResponse>, Status> {
        let caller = caller_of(&request)?;
        let request = request.into_inner();
        let (record, session) = self.session(request.session.as_ref())?;
        self.require_use(&caller, Capability::UserReviewState, &session)?;
        let view = request.view.ok_or_else(|| ports::missing("view"))?;
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
        let caller = caller_of(&request)?;
        let request = request.into_inner();
        let (record, session) = self.session(request.session.as_ref())?;
        self.require_use(&caller, Capability::ReviewFocus, &session)?;
        let review = self.review(&record.id)?;
        let idempotency = idempotency(request.idempotency_key.as_ref());
        if let Some(Answer::Focused(answer)) = answered(&review, idempotency.as_ref()) {
            return Ok(Response::new(answer));
        }
        let layer = stack::layer_asked(&request)?;
        // A layer with no file only moves the layer; with a file, the layer
        // moves first and the file is opened in it.
        let file_asked = request.file.is_some() || !request.highlights.is_empty();
        let moved = match layer {
            Some(layer) => {
                let mut chat = lock(&review.chat);
                let change = chat.chat.focus_layer(layer).map_err(|error| chat_failed(&error))?;
                chat.at.insert(change.seq, now());
                Some(change)
            }
            None => None,
        };
        let change = if file_asked || moved.is_none() {
            let (place, highlights) =
                wire::focus_from_pb(&request).map_err(|error| wire_failed("file", &error))?;
            let shown = lock(&review.shown).clone();
            let mut chat = lock(&review.chat);
            let change = chat
                .chat
                .focus(place, highlights, |file| in_diff(&shown, file))
                .map_err(|error| chat_failed(&error))?;
            let at = now();
            chat.at.insert(change.seq, at);
            wire::change_to_pb(&change, at)
        } else if let Some(change) = moved {
            let chat = lock(&review.chat);
            let at = chat.at.get(&change.seq).copied().unwrap_or_else(now);
            wire::change_to_pb(&change, at)
        } else {
            return Err(ports::missing("file"));
        };
        remember(&review, idempotency, Answer::Focused(change.clone()));
        self.publish(&caller, &record, &review, change.seq).await?;
        Ok(Response::new(change))
    }

    async fn undo_review_change(
        &self,
        request: Request<pb::UndoReviewChangeRequest>,
    ) -> Result<Response<pb::ReviewChange>, Status> {
        let caller = caller_of(&request)?;
        let request = request.into_inner();
        let (record, session) = self.session(request.session.as_ref())?;
        self.require_use(&caller, Capability::UserReviewState, &session)?;
        if request.undo_id.is_empty() {
            return Err(ports::missing("undo_id"));
        }
        let review = self.review(&record.id)?;
        let change = {
            let mut chat = lock(&review.chat);
            let change = chat.chat.undo(&request.undo_id).map_err(|error| chat_failed(&error))?;
            let at = now();
            chat.at.insert(change.seq, at);
            wire::change_to_pb(&change, at)
        };
        self.publish(&caller, &record, &review, change.seq).await?;
        Ok(Response::new(change))
    }

    type WatchReviewChangesStream = Changes;

    async fn watch_review_changes(
        &self,
        request: Request<pb::WatchReviewChangesRequest>,
    ) -> Result<Response<Self::WatchReviewChangesStream>, Status> {
        let caller = caller_of(&request)?;
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
        let caller = caller_of(&request)?;
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
/// `note` destination, and EditComment and DropComment on those notes; the
/// user's own comments for the agent (`comments`), whose ids are
/// `<session>/c-<n>`; and drafts for a review's pull requests, made with the
/// `pull_request` destination or from a note by MakeNoteADraft (D284).
#[tonic::async_trait]
impl pb::comment_service_server::CommentService for Review {
    /// D284: the note becomes a draft in place, same id, author and anchor.
    async fn make_note_a_draft(
        &self,
        request: Request<pb::MakeNoteADraftRequest>,
    ) -> Result<Response<pb::Comment>, Status> {
        let caller = caller_of(&request)?;
        self.make_note_a_draft_of(&caller, request.into_inner()).await.map(Response::new)
    }

    /// The review chat's notes, which have no status (they are never sent),
    /// so a status filter lists none of them, and the user's comments.
    async fn list_comments(
        &self,
        request: Request<pb::ListCommentsRequest>,
    ) -> Result<Response<pb::ListCommentsResponse>, Status> {
        let caller = caller_of(&request)?;
        let request = request.into_inner();
        let asked = request.session.clone().or_else(|| own_session(&caller));
        let (record, session) = self.session(asked.as_ref())?;
        self.require(&caller, &Call::ReadSession(session))?;
        let review = self.review(&record.id)?;
        let (user_comments, draft_count) = self.list_user_comments(&record, &review, &request).await?;
        let mut comments: Vec<pb::Comment> = if request.statuses.is_empty() && !request.archived {
            lock(&review.chat)
                .chat
                .notes()
                .filter(|note| request.file.as_ref().is_none_or(|file| on_file(&note.note.file, file)))
                // A note made a draft (D284) is listed once, as the draft.
                .filter(|note| lock(&review.comments).comment(&note.id).is_none())
                .map(|note| note_comment(note, &record.id))
                .collect()
        } else {
            Vec::new()
        };
        comments.extend(user_comments);
        let (size, token) = match request.page {
            Some(page) => (page.page_size as usize, page.page_token),
            None => (0, String::new()),
        };
        let size = match size {
            0 => COMMENTS_PAGE_DEFAULT,
            size => size.min(COMMENTS_PAGE_MAX),
        };
        let (comments, next_page_token) = page_of_comments(comments, size, &token)?;
        Ok(Response::new(pb::ListCommentsResponse {
            comments,
            draft_count,
            page: Some(pb::PageResponse { next_page_token }),
        }))
    }

    async fn create_comment(&self, request: Request<pb::CreateCommentRequest>) -> Result<Response<pb::Comment>, Status> {
        let caller = caller_of(&request)?;
        let request = request.into_inner();
        let destination = request.destination.as_ref().and_then(|destination| destination.destination.as_ref());
        if matches!(
            destination,
            Some(
                pb::comment_destination::Destination::Terminal(_)
                    | pb::comment_destination::Destination::PullRequest(_)
            )
        ) {
            return self.create_user_comment(&caller, request).await.map(Response::new);
        }
        if destination.is_none() {
            return Err(ports::missing("destination"));
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
            .ok_or_else(|| ports::internal("anchoring a note lost it".to_owned(), ""))?;
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
        self.publish(&caller, &record, &review, seq).await?;
        let comment =
            comment.ok_or_else(|| ports::internal("a new note made no note".to_owned(), ""))?;
        Ok(Response::new(comment))
    }

    async fn edit_comment(&self, request: Request<pb::EditCommentRequest>) -> Result<Response<pb::Comment>, Status> {
        let caller = caller_of(&request)?;
        let request = request.into_inner();
        if self.is_ledger_id(&caller, request.id.as_ref()) {
            return self.edit_user_comment(&caller, request).await.map(Response::new);
        }
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
        self.publish(&caller, &record, &review, seq).await?;
        let comment = comment
            .ok_or_else(|| ports::internal("an edited note made no note".to_owned(), ""))?;
        Ok(Response::new(comment))
    }

    async fn drop_comment(
        &self,
        request: Request<pb::DropCommentRequest>,
    ) -> Result<Response<pb::DropCommentResponse>, Status> {
        let caller = caller_of(&request)?;
        let request = request.into_inner();
        if self.is_ledger_id(&caller, request.id.as_ref()) {
            return self.drop_user_comment(&caller, request).await.map(Response::new);
        }
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
        self.publish(&caller, &record, &review, change.seq).await?;
        Ok(Response::new(answer))
    }

    async fn reattach_comment(
        &self,
        request: Request<pb::ReattachCommentRequest>,
    ) -> Result<Response<pb::Comment>, Status> {
        let caller = caller_of(&request)?;
        let request = request.into_inner();
        if !comments::is_user_id(request.id.as_ref()) {
            return Err(ports::not_built("humata.v1.CommentService.ReattachComment for a note"));
        }
        self.reattach_user_comment(&caller, request).await.map(Response::new)
    }

    async fn send_comments(
        &self,
        request: Request<pb::SendCommentsRequest>,
    ) -> Result<Response<pb::SendCommentsResponse>, Status> {
        let caller = caller_of(&request)?;
        self.send_user_comments(&caller, request.into_inner()).await.map(Response::new)
    }

    async fn resolve_comment(&self, request: Request<pb::ResolveCommentRequest>) -> Result<Response<pb::Comment>, Status> {
        let caller = caller_of(&request)?;
        let request = request.into_inner();
        if !comments::is_user_id(request.id.as_ref()) {
            return Err(ports::not_built("humata.v1.CommentService.ResolveComment for a note"));
        }
        self.resolve_user_comment(&caller, request).await.map(Response::new)
    }

    async fn reopen_comment(&self, request: Request<pb::ReopenCommentRequest>) -> Result<Response<pb::Comment>, Status> {
        let caller = caller_of(&request)?;
        let request = request.into_inner();
        if !comments::is_user_id(request.id.as_ref()) {
            return Err(ports::not_built("humata.v1.CommentService.ReopenComment for a note"));
        }
        self.reopen_user_comment(&caller, request).await.map(Response::new)
    }

    async fn take_back_comments(
        &self,
        request: Request<pb::TakeBackCommentsRequest>,
    ) -> Result<Response<pb::TakeBackCommentsResponse>, Status> {
        let caller = caller_of(&request)?;
        self.take_back_user_comments(&caller, request.into_inner()).await.map(Response::new)
    }

    async fn resolve_addressed_comments(
        &self,
        request: Request<pb::ResolveAddressedCommentsRequest>,
    ) -> Result<Response<pb::ResolveAddressedCommentsResponse>, Status> {
        let caller = caller_of(&request)?;
        self.resolve_addressed_user_comments(&caller, request.into_inner()).await.map(Response::new)
    }

    async fn reply_to_comment(&self, request: Request<pb::ReplyToCommentRequest>) -> Result<Response<pb::Comment>, Status> {
        let caller = caller_of(&request)?;
        self.reply_to_user_comment(&caller, request.into_inner()).await.map(Response::new)
    }

    async fn archive_comment(&self, request: Request<pb::ArchiveCommentRequest>) -> Result<Response<pb::Comment>, Status> {
        let caller = caller_of(&request)?;
        self.archive_user_comment(&caller, request.into_inner()).await.map(Response::new)
    }

    async fn unarchive_comment(
        &self,
        request: Request<pb::UnarchiveCommentRequest>,
    ) -> Result<Response<pb::Comment>, Status> {
        let caller = caller_of(&request)?;
        self.unarchive_user_comment(&caller, request.into_inner()).await.map(Response::new)
    }

    type WatchCommentsStream = CommentEvents;

    async fn watch_comments(
        &self,
        request: Request<pb::WatchCommentsRequest>,
    ) -> Result<Response<Self::WatchCommentsStream>, Status> {
        let caller = caller_of(&request)?;
        let request = request.into_inner();
        let (record, session) = self.session(request.session.as_ref())?;
        self.require_use(&caller, Capability::UserReviewState, &session)?;
        let review = self.review(&record.id)?;
        // Subscribed before the first event goes out, so the listing the app
        // makes after it misses no change.
        let mut changes = review.comment_feed.0.subscribe();
        let (sender, receiver) = tokio::sync::mpsc::channel(16);
        tokio::spawn(async move {
            let resync = pb::CommentFeedEvent {
                event: Some(pb::comment_feed_event::Event::Resync(true)),
            };
            if sender.send(Ok(resync.clone())).await.is_err() {
                return;
            }
            loop {
                let event = tokio::select! {
                    () = sender.closed() => return,
                    received = changes.recv() => match received {
                        Ok(event) => event,
                        Err(broadcast::error::RecvError::Lagged(_)) => resync.clone(),
                        Err(broadcast::error::RecvError::Closed) => return,
                    },
                };
                if sender.send(Ok(event)).await.is_err() {
                    return;
                }
            }
        });
        Ok(Response::new(Box::pin(tokio_stream::wrappers::ReceiverStream::new(receiver))))
    }
}

/// A view a session's agent prepares. A review plan in the session's open
/// review chat applies at once (PR-24); anything else is a draft view
/// until the agent marks it ready (VW-14, `views`).
#[tonic::async_trait]
impl pb::view_service_server::ViewService for Review {
    async fn prepare_view(&self, request: Request<pb::PrepareViewRequest>) -> Result<Response<pb::PreparedView>, Status> {
        let caller = caller_of(&request)?;
        let request = request.into_inner();
        let asked = own_session(&caller).ok_or_else(|| views::agents_only("PrepareView"))?;
        let (record, session) = self.session(Some(&asked))?;
        let in_chat = matches!(request.content, Some(pb::prepare_view_request::Content::ReviewPlan(_)))
            && lock(&self.review(&record.id)?.chat).reported;
        if !in_chat {
            return self.prepare_draft(&caller, &record, &session, request).await.map(Response::new);
        }
        let Some(pb::prepare_view_request::Content::ReviewPlan(plan)) = request.content else {
            return Err(ports::missing("review_plan"));
        };
        let author = note_author(&caller)?;
        let review = self.review(&record.id)?;
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
        let (plan, diffs, checked) = self.checked_plan_with_stack(&caller, &record, &session, &plan).await?;
        if let Some(checked) = checked {
            Self::keep_stack(&review, checked, plan.from_stack.clone());
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
        self.publish(&caller, &record, &review, change.seq).await?;
        Ok(Response::new(answer))
    }

    async fn revise_view(&self, request: Request<pb::ReviseViewRequest>) -> Result<Response<pb::PreparedView>, Status> {
        let caller = caller_of(&request)?;
        self.revise_draft(&caller, request.into_inner()).await.map(Response::new)
    }

    async fn mark_ready(&self, request: Request<pb::MarkReadyRequest>) -> Result<Response<pb::PreparedView>, Status> {
        let caller = caller_of(&request)?;
        self.mark_draft_ready(&caller, request.into_inner()).await.map(Response::new)
    }

    async fn list_views(
        &self,
        request: Request<pb::ListViewsRequest>,
    ) -> Result<Response<pb::ListViewsResponse>, Status> {
        let caller = caller_of(&request)?;
        self.list_prepared(&caller, request.into_inner()).await.map(Response::new)
    }

    async fn set_view_state(
        &self,
        request: Request<pb::SetViewStateRequest>,
    ) -> Result<Response<pb::PreparedView>, Status> {
        let caller = caller_of(&request)?;
        self.set_state(&caller, request.into_inner()).await.map(Response::new)
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
    /// PR-26). Returns the last change's seq when any note changed, for
    /// the app's feed.
    async fn refresh_notes(
        &self,
        clones: &Clones,
        review: &SessionReview,
        diffs: &[RepoDiff],
    ) -> Result<Option<u64>, Status> {
        let notes: Vec<_> = lock(&review.chat).chat.notes().map(|note| note.note.clone()).collect();
        if notes.is_empty() {
            return Ok(None);
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
            return Ok(None);
        }
        let changed = chat.chat.refresh_notes(&states);
        let at = now();
        for change in &changed {
            chat.at.insert(change.seq, at);
        }
        Ok(changed.last().map(|change| change.seq))
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
            .ok_or_else(|| ports::missing("id"))?;
        let (asked, note) = match id.split_once('/') {
            Some((session, note)) => (
                pb::SessionId {
                    value: session.to_owned(),
                },
                note.to_owned(),
            ),
            None => (
                own_session(caller).ok_or_else(|| {
                    ports::invalid_field(
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
        _ => Err(ports::failure(
            Code::FailedPrecondition,
            ports::error(
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

/// Where a comment stands in ListComments order, from its wire id
/// (`<session>/note-<n>` or `<session>/c-<n>`): notes first, then the
/// user's comments, each by number. `None` when the id is neither.
fn comment_key(id: &str) -> Option<(u8, u64)> {
    let last = id.rsplit('/').next()?;
    if let Some(n) = last.strip_prefix("note-") {
        return n.parse().ok().map(|n| (0, n));
    }
    let n = last.strip_prefix("c-")?;
    n.parse().ok().map(|n| (1, n))
}

/// Puts `comments` in ListComments order (an id that doesn't parse last),
/// and cuts one page of at most `size` after the comment `token` names.
/// Answers the page and the token for the next one, empty on the last.
/// A token is a comment id; it still works when that comment is gone.
fn page_of_comments(mut comments: Vec<pb::Comment>, size: usize, token: &str) -> Result<(Vec<pb::Comment>, String), Status> {
    let key_of = |comment: &pb::Comment| {
        comment
            .id
            .as_ref()
            .and_then(|id| comment_key(&id.value))
            .unwrap_or((u8::MAX, u64::MAX))
    };
    comments.sort_by_key(key_of);
    let after = if token.is_empty() {
        None
    } else {
        Some(comment_key(token).ok_or_else(|| {
            ports::invalid_field(
                "page.page_token",
                pb::invalid_field::Problem::BadFormat,
                format!("{token:?} isn't a comment page token"),
                "pass the next_page_token of the previous answer, or leave it empty for the first page",
            )
        })?)
    };
    let mut rest = comments.into_iter().filter(|comment| after.is_none_or(|after| key_of(comment) > after));
    let page: Vec<pb::Comment> = rest.by_ref().take(size).collect();
    let next_page_token = match (rest.next(), page.last()) {
        (Some(_), Some(last)) => last.id.as_ref().map(|id| id.value.clone()).unwrap_or_default(),
        _ => String::new(),
    };
    Ok((page, next_page_token))
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

/// The review's notes and the user's comments as Comments, by id.
fn notes_now(review: &SessionReview, session: &str) -> BTreeMap<String, pb::Comment> {
    let mut now: BTreeMap<String, pb::Comment> = lock(&review.chat)
        .chat
        .notes()
        .map(|note| {
            let comment = note_comment(note, session);
            let id = comment.id.as_ref().map(|id| id.value.clone()).unwrap_or_default();
            (id, comment)
        })
        .collect();
    now.extend(comments::all_now(&lock(&review.comments), session));
    now
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
            stack,
            layer,
            importance,
        } => {
            added_notes.clone_from(added);
            Before::Order(view(ViewState {
                order: order.clone(),
                groups: groups.clone(),
                stack: stack.clone(),
                layer: layer.clone(),
                importance: importance.clone(),
                ..ViewState::default()
            }))
        }
        // The store has no arm for a layer move (D800 seam), so it rides in
        // a Focus view with only `layer` set; `undo_from_stored` reads that
        // back. Undoing a move *to* Whole stack after a restart (no layer to
        // go back to) restores as a Focus with no place instead.
        Replaced::Layer { layer } => Before::Focus(view(ViewState {
            layer: layer.clone(),
            ..ViewState::default()
        })),
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
            if view.open.is_none() && view.highlights.is_empty() && view.layer.is_some() {
                Replaced::Layer { layer: view.layer }
            } else {
                Replaced::Focus {
                    place: view.open,
                    highlights: view.highlights,
                }
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
                stack: view.stack,
                layer: view.layer,
                importance: view.importance,
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
        say!(
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
    ports::failure(
        Code::FailedPrecondition,
        ports::error(
            error.to_string(),
            "",
            pb::error::Kind::InvalidField(pb::InvalidField {
                field: "review_plan".to_owned(),
                problem: pb::invalid_field::Problem::NotAllowedHere as i32,
            }),
        ),
    )
}

/// A plan or layer that named a file twice in its importance list (D802).
fn importance_failed(field: &str, error: &humata2_review_diff::ImportanceError) -> Status {
    ports::invalid_field(
        field,
        pb::invalid_field::Problem::NotAllowedHere,
        error.to_string(),
        "send each file once in `importance`, with the one tier it should have",
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
    ports::invalid_field(
        "repos",
        pb::invalid_field::Problem::BadFormat,
        format!("{repo:?} isn't a repository of the session"),
        "name the repository as owner/name, like acme/api, or leave repos empty for every one",
    )
}

fn wire_failed(field: &str, error: &wire::WireError) -> Status {
    ports::invalid_field(field, pb::invalid_field::Problem::BadFormat, error.to_string(), "")
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
        | DiffError::NoDefaultBranch { .. }
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
        _ => return ports::internal(ports::chain(error), ""),
    };
    ports::failure(code, ports::error(ports::chain(error), "", kind))
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
        return ports::failure(
            Code::PermissionDenied,
            ports::error(
                error.to_string(),
                "the user turns it back on in the session's Info panel, under \"Agent may\"",
                pb::error::Kind::Denied(denied),
            ),
        );
    }
    let (code, field) = match error {
        ChatError::NotInReview { .. } => (Code::FailedPrecondition, "file"),
        ChatError::NoSuchNote { .. } => (Code::NotFound, "note_id"),
        ChatError::NoSuchLayer { .. } => (Code::FailedPrecondition, "layer"),
        _ => (Code::FailedPrecondition, "undo_id"),
    };
    ports::failure(
        code,
        ports::error(
            error.to_string(),
            "",
            pb::error::Kind::InvalidField(pb::InvalidField {
                field: field.to_owned(),
                problem: pb::invalid_field::Problem::NotAllowedHere as i32,
            }),
        ),
    )
}

/// Each diff's file tiers (D802): the agent's `agent` list, else the
/// daemon's own skims (lockfiles, generated paths, pure renames). Asks git
/// once per repo off the async threads; hands the diffs back with them.
async fn served_tiers(
    clones: &Clones,
    diffs: Vec<RepoDiff>,
    agent: Vec<humata2_review_diff::Importance>,
) -> Result<(Vec<RepoDiff>, Vec<HashMap<String, humata2_review_diff::Tier>>), Status> {
    let checkouts = diffs
        .iter()
        .map(|diff| clones.get(&diff.repo).cloned())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| diff_failed(&error))?;
    let (diffs, tiers) = blocking(move || {
        let tiers = diffs
            .iter()
            .zip(&checkouts)
            .map(|(diff, checkout)| humata2_review_diff::tiers_blocking(checkout, diff, &agent))
            .collect::<Result<Vec<_>, _>>();
        (diffs, tiers)
    })
    .await?;
    Ok((diffs, tiers.map_err(|error| diff_failed(&error))?))
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_secs()).unwrap_or(i64::MAX))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

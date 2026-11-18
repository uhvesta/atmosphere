//! The user's comments for the session's agent (SPEC SM-7, SM-9-SM-21,
//! DK-9, SE-23, RV-27, D138): CommentService's `terminal` destination.
//!
//! Per session a [`CommentLedger`] holds the comments and the batches being
//! typed into the agent's terminal. A comment's id on the wire is
//! `<session>/c-<n>`, the ledger's is `c-<n>`; the review chat's notes keep
//! `note-<n>` and their own code path in the parent module. A batch is saved
//! as Sending before anything is typed, so a restart knows it may or may not
//! have arrived (D138).

use std::collections::BTreeMap;
use std::path::Path;

use humata2_access::{Call, Capability, Refusal, Target};
use humata2_grpc::Timestamp;
use humata2_grpc::humata::v1 as pb;
use humata2_review_diff::{
    Addressed, CommentAnchor, CommentAuthor, CommentError, CommentFilter, CommentLedger, CommentReply, CommentStamp,
    CommentStatus, Delivery, DeliveryState, DiffContext, FileRef, LedgerParts, MessagePart, NoteTarget, Placement,
    Side, Typed, UserComment, bracketed_paste, compose, place, wire,
};
use humata2_store::{
    CommentChange, CommentDeliveryRecord, CommentDeliveryState, CommentNumbers, CommentRecord, SessionRecord, Store,
    StoreError,
};
use prost::Message;
use tonic::{Code, Status};

use super::sink::{self, Arrived, Pasted};
use super::{
    Answer, Review, SELECTED_LINES_MAX, SessionReview, answered, blocking, folder_name, idempotency, lock, now, on_file,
    own_session, remember, repo_keys,
};
use crate::identity::{self, Caller};
use crate::status;

/// The batches of comments the last run left in doubt, per session, as
/// `Review::note_comments` found them.
pub(crate) struct Doubtful(
    Vec<(SessionRecord, std::sync::Arc<SessionReview>, Vec<(String, String, sink::Target)>)>,
);

/// Whether a comment id names one of the user's comments (`c-<n>`, with or
/// without its session) rather than a review chat note.
pub(super) fn is_user_id(id: Option<&pb::CommentId>) -> bool {
    id.is_some_and(|id| id.value.rsplit('/').next().is_some_and(|last| last.starts_with("c-")))
}

impl Review {
    /// The user's comments as ListComments asks for them, and the draft
    /// count (SM-15). Anchors are looked at again first (SM-7, SM-16).
    pub(super) async fn list_user_comments(
        &self,
        record: &SessionRecord,
        review: &SessionReview,
        request: &pb::ListCommentsRequest,
    ) -> Result<(Vec<pb::Comment>, u32), Status> {
        self.refreshed_comments(record, review).await?;
        let asked: Vec<CommentStatus> = request.statuses.iter().filter_map(|s| status_from_pb(*s)).collect();
        let ledger = lock(&review.comments);
        let draft_count = ledger.draft_count();
        if !request.statuses.is_empty() && asked.is_empty() {
            // Only statuses a comment to the agent never has.
            return Ok((Vec::new(), draft_count));
        }
        let comments = ledger
            .list(&CommentFilter {
                statuses: &asked,
                file: None,
                archived: request.archived,
            })
            .into_iter()
            .filter(|comment| request.file.as_ref().is_none_or(|file| on_file(&comment.anchor.file, file)))
            .map(|comment| to_pb(comment, &record.id))
            .collect();
        Ok((comments, draft_count))
    }

    /// Looks at each comment's passage again, and saves and tells the feed
    /// when one moved, changed or was addressed.
    pub(super) async fn refreshed_comments(&self, record: &SessionRecord, review: &SessionReview) -> Result<(), Status> {
        if self.refresh_user_comments(record, review).await {
            self.keep_comments(&system_actor(), record, review).await?;
        }
        Ok(())
    }

    /// Saves the comments and tells the feed how they changed.
    async fn keep_comments(&self, by: &pb::Actor, record: &SessionRecord, review: &SessionReview) -> Result<(), Status> {
        let saved = self.save_comments(&record.id, review).await;
        self.tell_notes(record, review, by);
        saved
    }

    /// Saves what differs between the comments now and what was last saved,
    /// as one transaction, and nothing when none does. Saves of one
    /// session's comments land in the order they're made. The review's own
    /// blob is not written.
    async fn save_comments(&self, session: &str, review: &SessionReview) -> Result<(), Status> {
        let _saving = review.comments_saving.lock().await;
        let planned = difference(session, &lock(&review.comments), &lock(&review.comments_kept));
        let Some((change, kept, told)) = planned else {
            return Ok(());
        };
        let store = self.inner.store.clone();
        blocking(move || store.apply_comments(&change))
            .await?
            .map_err(|error| status::store(error, &self.inner.machine, "save the session's comments"))?;
        *lock(&review.comments_kept) = kept;
        // Told while the saves are held in order, so the feed agrees with
        // the store. Nobody watching is no one to tell.
        for event in told {
            let _ = review.comment_feed.0.send(event);
        }
        Ok(())
    }

    /// A new draft from CreateComment with the `terminal` destination.
    pub(super) async fn create_user_comment(
        &self,
        caller: &Caller,
        request: pb::CreateCommentRequest,
    ) -> Result<pb::Comment, Status> {
        let for_pr = match request.destination.as_ref().and_then(|destination| destination.destination.as_ref()) {
            Some(pb::comment_destination::Destination::PullRequest(pr)) => Some(pr.clone()),
            _ => None,
        };
        let asked = request.session.clone().or_else(|| own_session(caller));
        let (record, session) = self.session(asked.as_ref())?;
        let author = match &for_pr {
            Some(_) => Some(self.draft_author(caller, &session)?),
            None => {
                self.require_use(caller, Capability::UserReviewState, &session)?;
                None
            }
        };
        let review = self.review(&record.id)?;
        let idempotency = idempotency(request.idempotency_key.as_ref());
        if let Some(Answer::Comment(answer)) = answered(&review, idempotency.as_ref()) {
            return Ok(answer);
        }
        let anchor = request.anchor.as_ref().ok_or_else(|| status::missing("anchor"))?;
        let comment = match (for_pr, author) {
            (Some(pr), Some(author)) => {
                let (repo, number) = self.bound_pull_request(&record.id, &pr)?;
                let named = anchor
                    .file
                    .as_ref()
                    .and_then(|file| file.repo.as_ref())
                    .filter(|named| !named.name.is_empty())
                    .map(wire::repo_key);
                if named.as_ref().is_some_and(|named| !repo_holds_pull(named, &repo)) {
                    return Err(status::invalid_field(
                        "anchor.file.repo",
                        pb::invalid_field::Problem::NotAllowedHere,
                        format!("the draft is for a pull request of {repo}, but its lines are in another repository"),
                        format!("anchor the draft on a file of {repo}"),
                    ));
                }
                let anchor = self.anchor_from_pb(&record, anchor, Some(repo.clone())).await?;
                lock(&review.comments)
                    .add_for_pull_request(anchor, &request.text, author, &repo, number, now())
                    .clone()
            }
            _ => {
                let anchor = self.anchor_from_pb(&record, anchor, None).await?;
                lock(&review.comments).add(anchor, &request.text, now()).clone()
            }
        };
        let answer = to_pb(&comment, &record.id);
        remember(&review, idempotency, Answer::Comment(answer.clone()));
        self.keep_comments(&identity::actor(caller), &record, &review).await?;
        Ok(answer)
    }

    /// Changes a draft's text.
    pub(super) async fn edit_user_comment(
        &self,
        caller: &Caller,
        request: pb::EditCommentRequest,
    ) -> Result<pb::Comment, Status> {
        let expected = expected(request.expected_revision.as_ref());
        let text = request.text;
        if let Caller::Agent(agent) = caller {
            let author = CommentAuthor::Agent(agent.agent.clone());
            return self
                .agent_comment_change(
                    caller,
                    agent,
                    Capability::ReviewDraft,
                    request.id.as_ref(),
                    request.idempotency_key.as_ref(),
                    move |ledger, id| {
                        own_draft(ledger, id, &author)?;
                        ledger.edit(id, &text, expected).cloned().map_err(|error| comment_failed(&error))
                    },
                )
                .await;
        }
        self.change_user_comment(
            caller,
            request.id.as_ref(),
            request.idempotency_key.as_ref(),
            move |ledger, id| ledger.edit(id, &text, expected).cloned(),
        )
        .await
    }

    /// Deletes a comment for good (SM-21: Drop deletes, Archive keeps).
    pub(super) async fn drop_user_comment(
        &self,
        caller: &Caller,
        request: pb::DropCommentRequest,
    ) -> Result<pb::DropCommentResponse, Status> {
        let agent = match caller {
            Caller::Agent(agent) => Some(agent),
            _ => None,
        };
        let (record, id, review) = match agent {
            Some(agent) => self.agent_session(caller, agent, Capability::ReviewDraft, request.id.as_ref())?,
            None => self.user_comment_target(caller, request.id.as_ref())?,
        };
        let idempotency = idempotency(request.idempotency_key.as_ref());
        if let Some(Answer::Dropped(answer)) = answered(&review, idempotency.as_ref()) {
            return Ok(answer);
        }
        {
            let mut ledger = lock(&review.comments);
            if let Some(agent) = agent {
                own_draft(&ledger, &id, &CommentAuthor::Agent(agent.agent.clone()))?;
            }
            ledger
                .remove(&id, expected(request.expected_revision.as_ref()))
                .map_err(|error| comment_failed(&error))?;
        }
        let answer = pb::DropCommentResponse { undo_id: String::new() };
        remember(&review, idempotency, Answer::Dropped(answer.clone()));
        self.keep_comments(&identity::actor(caller), &record, &review).await?;
        Ok(answer)
    }

    /// Points a comment at a new passage (SM-7).
    pub(super) async fn reattach_user_comment(
        &self,
        caller: &Caller,
        request: pb::ReattachCommentRequest,
    ) -> Result<pb::Comment, Status> {
        let (record, id, review) = self.user_comment_target(caller, request.id.as_ref())?;
        let idempotency = idempotency(request.idempotency_key.as_ref());
        if let Some(Answer::Comment(answer)) = answered(&review, idempotency.as_ref()) {
            return Ok(answer);
        }
        let anchor = request.anchor.as_ref().ok_or_else(|| status::missing("anchor"))?;
        let held = lock(&review.comments).comment(&id).map(|comment| comment.anchor.file.repo.clone());
        let anchor = self.anchor_from_pb(&record, anchor, held).await?;
        let expected = expected(request.expected_revision.as_ref());
        let comment = lock(&review.comments)
            .reattach(&id, anchor, expected)
            .map_err(|error| comment_failed(&error))?
            .clone();
        let answer = to_pb(&comment, &record.id);
        remember(&review, idempotency, Answer::Comment(answer.clone()));
        self.keep_comments(&identity::actor(caller), &record, &review).await?;
        Ok(answer)
    }

    /// Resolves a comment. From the user it is Resolve (SM-11, SM-19).
    pub(super) async fn resolve_user_comment(
        &self,
        caller: &Caller,
        request: pb::ResolveCommentRequest,
    ) -> Result<pb::Comment, Status> {
        if let Caller::Agent(agent) = caller {
            let expected = expected(request.expected_revision.as_ref());
            let author = CommentAuthor::Agent(agent.agent.clone());
            let reply = request.reply;
            return self
                .agent_comment_change(
                    caller,
                    agent,
                    Capability::CommentResolve,
                    request.id.as_ref(),
                    request.idempotency_key.as_ref(),
                    move |ledger, id| {
                        ledger
                            .resolve(id, author, Some(reply.as_str()), now(), expected)
                            .cloned()
                            .map_err(|error| comment_failed(&error))
                    },
                )
                .await;
        }
        let expected = expected(request.expected_revision.as_ref());
        let reply = request.reply;
        self.change_user_comment(
            caller,
            request.id.as_ref(),
            request.idempotency_key.as_ref(),
            move |ledger, id| {
                let reply = (!reply.trim().is_empty()).then_some(reply.as_str());
                ledger.resolve(id, CommentAuthor::User, reply, now(), expected).cloned()
            },
        )
        .await
    }

    /// Resolves every addressed comment of one file, or of the session, as
    /// the user (SM-18). The idempotency key is unused: a repeat finds
    /// nothing left to resolve.
    pub(super) async fn resolve_addressed_user_comments(
        &self,
        caller: &Caller,
        request: pb::ResolveAddressedCommentsRequest,
    ) -> Result<pb::ResolveAddressedCommentsResponse, Status> {
        let asked = request.session.clone().or_else(|| own_session(caller));
        let (record, session) = self.session(asked.as_ref())?;
        self.require_use(caller, Capability::UserReviewState, &session)?;
        let review = self.review(&record.id)?;
        self.refreshed_comments(&record, &review).await?;
        let comments: Vec<pb::Comment> = {
            let mut ledger = lock(&review.comments);
            let ids: Vec<String> = ledger
                .list(&CommentFilter {
                    statuses: &[CommentStatus::Addressed],
                    file: None,
                    archived: false,
                })
                .into_iter()
                .filter(|comment| request.file.as_ref().is_none_or(|file| on_file(&comment.anchor.file, file)))
                .map(|comment| comment.id.clone())
                .collect();
            let mut resolved = Vec::with_capacity(ids.len());
            for id in &ids {
                let comment = ledger
                    .resolve(id, CommentAuthor::User, None, now(), None)
                    .map_err(|error| comment_failed(&error))?;
                resolved.push(to_pb(comment, &record.id));
            }
            resolved
        };
        if !comments.is_empty() {
            self.keep_comments(&identity::actor(caller), &record, &review).await?;
        }
        Ok(pb::ResolveAddressedCommentsResponse { comments })
    }

    /// The agent answers in a comment's thread (DK-9).
    pub(super) async fn reply_to_user_comment(
        &self,
        caller: &Caller,
        request: pb::ReplyToCommentRequest,
    ) -> Result<pb::Comment, Status> {
        let Caller::Agent(agent) = caller else {
            return Err(status::not_built("humata.v1.CommentService.ReplyToComment from the user"));
        };
        let author = CommentAuthor::Agent(agent.agent.clone());
        let text = request.text;
        self.agent_comment_change(
            caller,
            agent,
            Capability::CommentReply,
            request.id.as_ref(),
            request.idempotency_key.as_ref(),
            move |ledger, id| {
                ledger
                    .reply(id, author, &text, now())
                    .map_err(|error| comment_failed(&error))?;
                ledger
                    .comment(id)
                    .cloned()
                    .ok_or_else(|| comment_failed(&CommentError::Unknown { id: id.to_owned() }))
            },
        )
        .await
    }

    /// Puts a resolved comment back to a draft (SM-11).
    pub(super) async fn reopen_user_comment(
        &self,
        caller: &Caller,
        request: pb::ReopenCommentRequest,
    ) -> Result<pb::Comment, Status> {
        let expected = expected(request.expected_revision.as_ref());
        self.change_user_comment(
            caller,
            request.id.as_ref(),
            request.idempotency_key.as_ref(),
            move |ledger, id| ledger.reopen(id, expected).cloned(),
        )
        .await
    }

    /// Sets a comment aside without deleting it (SM-20).
    pub(super) async fn archive_user_comment(
        &self,
        caller: &Caller,
        request: pb::ArchiveCommentRequest,
    ) -> Result<pb::Comment, Status> {
        self.change_user_comment(
            caller,
            request.id.as_ref(),
            request.idempotency_key.as_ref(),
            |ledger, id| ledger.archive(id, now()).cloned(),
        )
        .await
    }

    /// Brings an archived comment back (SM-21).
    pub(super) async fn unarchive_user_comment(
        &self,
        caller: &Caller,
        request: pb::UnarchiveCommentRequest,
    ) -> Result<pb::Comment, Status> {
        self.change_user_comment(
            caller,
            request.id.as_ref(),
            request.idempotency_key.as_ref(),
            |ledger, id| ledger.unarchive(id).cloned(),
        )
        .await
    }

    /// Comments Humata is holding go back to drafts (SE-23 Remove).
    pub(super) async fn take_back_user_comments(
        &self,
        caller: &Caller,
        request: pb::TakeBackCommentsRequest,
    ) -> Result<pb::TakeBackCommentsResponse, Status> {
        let asked = request.session.clone().or_else(|| own_session(caller));
        let (record, session) = self.session(asked.as_ref())?;
        self.require_use(caller, Capability::UserReviewState, &session)?;
        let review = self.review(&record.id)?;
        let ids = local_ids(&record.id, &request.ids)?;
        let comments: Vec<pb::Comment> = {
            let mut ledger = lock(&review.comments);
            let back = ledger.take_back(&ids).map_err(|error| comment_failed(&error))?;
            back.iter()
                .filter_map(|id| ledger.comment(id))
                .map(|comment| to_pb(comment, &record.id))
                .collect()
        };
        self.keep_comments(&identity::actor(caller), &record, &review).await?;
        Ok(pb::TakeBackCommentsResponse { comments })
    }

    /// Types the named comments, or every draft and held one, into the
    /// session's agent terminal as one message (SM-9, SM-10, D126, D138).
    pub(super) async fn send_user_comments(
        &self,
        caller: &Caller,
        request: pb::SendCommentsRequest,
    ) -> Result<pb::SendCommentsResponse, Status> {
        let asked = request.session.clone().or_else(|| own_session(caller));
        let (record, session) = self.session(asked.as_ref())?;
        self.require_use(caller, Capability::UserReviewState, &session)?;
        let review = self.review(&record.id)?;
        let several_clones = if record.machine.is_empty() || record.machine == self.inner.machine {
            repo_keys(&self.inner, &record, &[]).await?.len() > 1
        } else {
            // The folder is on the session's own machine, after a move too
            // (MV-4): its start says which repositories it has.
            self.inner
                .store
                .session_detail(&record.id)
                .map_err(|error| status::store(error, &self.inner.machine, "read the session's repositories"))?
                .is_some_and(|detail| detail.repos.len() > 1)
        };
        let named = local_ids(&record.id, &request.ids)?;

        // Begin the batches in memory: the ledger checks everything before
        // it changes anything.
        let mut batches: Vec<Batch> = Vec::new();
        {
            let mut ledger = lock(&review.comments);
            let ids = if named.is_empty() { sendable_ids(&ledger) } else { named };
            let mut retry: Vec<String> = Vec::new();
            let mut fresh: Vec<String> = Vec::new();
            for id in &ids {
                let unconfirmed = ledger.deliveries().iter().find(|delivery| {
                    delivery.state == DeliveryState::Unconfirmed && delivery.comments.contains(id)
                });
                match unconfirmed {
                    Some(delivery) => {
                        if !retry.contains(&delivery.key) {
                            retry.push(delivery.key.clone());
                        }
                    }
                    None => fresh.push(id.clone()),
                }
            }
            let message = if fresh.is_empty() && !retry.is_empty() {
                None
            } else {
                let picked: Vec<UserComment> = ledger
                    .sendable(&fresh)
                    .map_err(|error| comment_failed(&error))?
                    .into_iter()
                    .cloned()
                    .collect();
                let folders: Vec<String> = picked
                    .iter()
                    .map(|comment| folder_name(&comment.anchor.file.repo).to_owned())
                    .collect();
                let parts: Vec<MessagePart<'_>> = picked
                    .iter()
                    .zip(&folders)
                    .map(|(comment, folder)| MessagePart {
                        // A folder file (repo "") has no repo to lead with.
                        repo: (several_clones && !folder.is_empty()).then_some(folder.as_str()),
                        path: &comment.anchor.file.path,
                        first: comment.anchor.first,
                        last: comment.anchor.last,
                        diff: (!comment.anchor.base.is_empty()).then(|| DiffContext {
                            side: comment.anchor.side,
                            base: &comment.anchor.base,
                        }),
                        quoted: &comment.anchor.quoted,
                        text: &comment.text,
                    })
                    .collect();
                Some(compose(&parts))
            };
            for key in &retry {
                let delivery = ledger.retry(key).map_err(|error| comment_failed(&error))?;
                batches.push(Batch {
                    key: delivery.key.clone(),
                    text: delivery.text.clone(),
                    comments: delivery.comments.clone(),
                    retried: true,
                });
            }
            if let Some(text) = message {
                let delivery = ledger.begin(&fresh, text, now()).map_err(|error| comment_failed(&error))?;
                batches.push(Batch {
                    key: delivery.key.clone(),
                    text: delivery.text.clone(),
                    comments: delivery.comments.clone(),
                    retried: false,
                });
            }
        }

        // Where the batches go, written down before the write-ahead save so
        // a restart knows which holder to ask. With nothing to type into,
        // nothing is typed: a fresh batch waits, a retried one stays
        // unconfirmed.
        let target = match self.inner.sink.target(&record.id).await {
            Ok(target) => {
                let mut ledger = lock(&review.comments);
                for batch in &batches {
                    if let Err(error) = ledger.set_target(&batch.key, &target.tab, target.holder) {
                        crate::log::say!("humatad2: session {}: {error}", record.id);
                    }
                }
                Some(target)
            }
            Err(status) => {
                let mut ledger = lock(&review.comments);
                for batch in &batches {
                    let pasted = if batch.retried {
                        Pasted::Unknown(status.clone())
                    } else {
                        Pasted::NotTyped(status.clone())
                    };
                    settled(&mut ledger, &record.id, &batch.key, pasted, batch.retried);
                }
                None
            }
        };

        // The write-ahead: a restart now knows the batches exist.
        if let Err(status) = self.save_comments(&record.id, &review).await {
            let mut ledger = lock(&review.comments);
            for batch in &batches {
                let outcome = if batch.retried { Typed::Unknown } else { Typed::No };
                if let Err(error) = ledger.settle(&batch.key, outcome, now()) {
                    crate::log::say!("humatad2: session {}: undoing batch {}: {error}", record.id, batch.key);
                }
            }
            return Err(status);
        }

        if let Some(target) = &target {
            crate::fault::point("comments.begun");
            for batch in &batches {
                let key = format!("{}/{}", record.id, batch.key);
                let pasted = self.inner.sink.paste(target, &key, &bracketed_paste(&batch.text)).await;
                crate::fault::point("comments.typed");
                settled(&mut lock(&review.comments), &record.id, &batch.key, pasted, batch.retried);
            }
        }

        let (comments, queued) = {
            let ledger = lock(&review.comments);
            let comments: Vec<&UserComment> = batches
                .iter()
                .flat_map(|batch| batch.comments.iter())
                .filter_map(|id| ledger.comment(id))
                .collect();
            let queued = comments.iter().any(|comment| comment.status == CommentStatus::Waiting);
            (comments.into_iter().map(|comment| to_pb(comment, &record.id)).collect(), queued)
        };
        self.keep_comments(&identity::actor(caller), &record, &review).await?;
        Ok(pb::SendCommentsResponse { comments, queued })
    }

    /// Looks at the passage of each comment that still follows its lines
    /// (SM-7, SM-16, SM-17, SM-18): moved, changed or addressed. Returns
    /// whether anything changed; the caller saves and tells the feed.
    pub(super) async fn refresh_user_comments(&self, record: &SessionRecord, review: &SessionReview) -> bool {
        struct Watched {
            id: String,
            file: (String, String),
            quoted: Vec<String>,
            first: u32,
        }
        let watched: Vec<Watched> = lock(&review.comments)
            .comments()
            .iter()
            .filter(|comment| {
                comment.archived.is_none()
                    && comment.pull_request.is_none()
                    && comment.anchor.side == Side::New
                    && !comment.anchor.quoted.is_empty()
                    && matches!(
                        comment.status,
                        CommentStatus::Draft | CommentStatus::Waiting | CommentStatus::Sent
                    )
            })
            .map(|comment| Watched {
                id: comment.id.clone(),
                file: (comment.anchor.file.repo.clone(), comment.anchor.file.path.clone()),
                quoted: comment.anchor.quoted.clone(),
                first: comment.anchor.first,
            })
            .collect();
        if watched.is_empty() {
            return false;
        }
        // A user's write of a file is under way, or finishes while this looks: its own rebaseline decides (BUG 42).
        let epoch = {
            let writes = lock(&review.user_writes.0);
            if writes.pending > 0 {
                return false;
            }
            writes.finished
        };
        let mut wanted: Vec<(String, String)> = watched.iter().map(|w| w.file.clone()).collect();
        wanted.sort();
        wanted.dedup();
        let folder = record.folder.clone();
        let read: Vec<((String, String), Option<Vec<String>>)> = match blocking(move || {
            wanted
                .into_iter()
                .map(|(repo, path)| {
                    let lines = read_lines(&folder, &repo, &path);
                    ((repo, path), lines)
                })
                .collect()
        })
        .await
        {
            Ok(read) => read,
            Err(status) => {
                crate::log::say!(
                    "humatad2: session {}: couldn't look at the comments' files: {}",
                    record.id,
                    status.message()
                );
                return false;
            }
        };
        let files: BTreeMap<(String, String), Vec<String>> = read
            .into_iter()
            .filter_map(|(file, lines)| lines.map(|lines| (file, lines)))
            .collect();
        let mut changed = false;
        // Held through the apply, so no write begins between this check and the ledger's change.
        let writes = lock(&review.user_writes.0);
        if writes.pending > 0 || writes.finished != epoch {
            return false;
        }
        let mut ledger = lock(&review.comments);
        for w in watched {
            let Some(lines) = files.get(&w.file) else {
                continue;
            };
            let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
            match place(&w.quoted, w.first, &lines) {
                Placement::Same => {}
                Placement::Moved { first, last } => changed |= ledger.moved(&w.id, first, last),
                Placement::Changed { first, last, now: after } => {
                    changed |= ledger.changed(&w.id, first, last, after, now());
                }
                Placement::Gone => changed |= ledger.gone(&w.id),
            }
        }
        changed
    }

    /// At start (D138, D241): finds each batch the last run was still
    /// typing and names it pending in the restart report. Call it before
    /// anything is served, then give what it found to `recover_comments`.
    pub(crate) fn note_comments(&self, operations: &crate::restart::Operations) -> Doubtful {
        let mut found = Vec::new();
        let sessions = match self.inner.store.sessions_with_sending_comments() {
            Ok(sessions) => sessions,
            Err(error) => {
                crate::log::say!("humatad2: couldn't list the sessions with comments to settle: {error}");
                return Doubtful(found);
            }
        };
        for session in sessions {
            let record = match self.inner.store.session(&session) {
                Ok(Some(record)) => record,
                Ok(None) => continue,
                Err(error) => {
                    crate::log::say!("humatad2: couldn't read session {session} to settle its comments: {error}");
                    continue;
                }
            };
            let review = match self.review(&record.id) {
                Ok(review) => review,
                Err(status) => {
                    crate::log::say!("humatad2: session {}'s comments: {}", record.id, status.message());
                    continue;
                }
            };
            let doubtful: Vec<(String, String, sink::Target)> = lock(&review.comments)
                .in_doubt()
                .into_iter()
                .map(|delivery| {
                    let target = sink::Target {
                        tab: delivery.tab.clone(),
                        holder: delivery.holder,
                    };
                    (delivery.key.clone(), delivery.text.clone(), target)
                })
                .collect();
            if doubtful.is_empty() {
                continue;
            }
            for (key, _, _) in &doubtful {
                operations.record(
                    &format!("comment_send:{}/{key}", record.id),
                    "comment_send",
                    "typing",
                    crate::restart::OperationState::Pending { waits_for: "the terminal to say whether it arrived".to_owned() },
                );
            }
            found.push((record, review, doubtful));
        }
        Doubtful(found)
    }

    /// At start (D138): settles each batch `note_comments` found, by asking
    /// the sink whether it arrived. Each is named in the restart report
    /// (D241). Failures are logged; the daemon starts regardless.
    pub(crate) async fn recover_comments(&self, operations: &crate::restart::Operations, doubtful: Doubtful) {
        for (record, review, doubtful) in doubtful.0 {
            for (key, text, target) in doubtful {
                let wire_key = format!("{}/{key}", record.id);
                let operation = format!("comment_send:{wire_key}");
                // Only the holder that held the terminal when the batch was
                // begun can say it never came; without one on record the
                // user decides.
                let unknown = || Pasted::Unknown(Status::unknown("the terminal can't say whether it arrived"));
                let pasted = if target.tab.is_empty() || target.holder == 0 {
                    unknown()
                } else {
                    match self.inner.sink.arrived(&target, &wire_key).await {
                        Arrived::Yes => Pasted::Typed,
                        Arrived::No => {
                            self.inner.sink.paste(&target, &wire_key, &bracketed_paste(&text)).await
                        }
                        Arrived::Unknown => unknown(),
                    }
                };
                operations.record(&operation, "comment_send", "typing", operation_state(&pasted));
                settled(&mut lock(&review.comments), &record.id, &key, pasted, false);
            }
            if let Err(status) = self.keep_comments(&system_actor(), &record, &review).await {
                crate::log::say!(
                    "humatad2: couldn't save session {}'s settled comments: {}",
                    record.id,
                    status.message()
                );
            }
        }
    }

    /// The session, ledger id and review a user-only call on one comment is
    /// about.
    fn user_comment_target(
        &self,
        caller: &Caller,
        id: Option<&pb::CommentId>,
    ) -> Result<(SessionRecord, String, std::sync::Arc<SessionReview>), Status> {
        let (record, session, id) = self.locate(caller, id, holds_comment)?;
        self.require_use(caller, Capability::UserReviewState, &session)?;
        let review = self.review(&record.id)?;
        Ok((record, id, review))
    }

    /// The session and local id a comment's wire id names. `<session>/<id>`
    /// and an agent's own bare ids name it outright; a user's bare id
    /// (`note-N`, as comments list it) names the one loaded review where
    /// `holds` says it is.
    fn locate(
        &self,
        caller: &Caller,
        id: Option<&pb::CommentId>,
        holds: fn(&SessionReview, &str) -> bool,
    ) -> Result<(SessionRecord, humata2_access::SessionRef, String), Status> {
        let bare = id
            .map(|id| id.value.as_str())
            .filter(|id| !id.is_empty() && !id.contains('/'));
        if let (Some(bare), None) = (bare, own_session(caller)) {
            let found: Vec<String> = lock(&self.inner.sessions)
                .iter()
                .filter(|(_, review)| holds(review, bare))
                .map(|(session, _)| session.clone())
                .collect();
            if let [only] = found.as_slice() {
                let (record, session) = self.session(Some(&pb::SessionId { value: only.clone() }))?;
                return Ok((record, session, bare.to_owned()));
            }
        }
        self.note_session(caller, id)
    }

    /// The session a pull request draft is made or changed in, checked for
    /// the agent that asks: its own, and drafts allowed to it.
    fn agent_session(
        &self,
        caller: &Caller,
        agent: &crate::identity::AgentCaller,
        capability: Capability,
        id: Option<&pb::CommentId>,
    ) -> Result<(SessionRecord, String, std::sync::Arc<SessionReview>), Status> {
        let (record, session, id) = self.note_session(caller, id)?;
        if agent.session.id.as_str() != record.id {
            return Err(status::invalid_field(
                "id",
                pb::invalid_field::Problem::NotAllowedHere,
                format!("comment {id} is in session {}, not your own", record.id),
                "answer only comments sent to your own session; list them with comments_list",
            ));
        }
        if matches!(capability, Capability::ReviewDraft) {
            self.require(
                caller,
                &Call::Use {
                    capability,
                    target: Target::Channel(session.channel.clone()),
                },
            )?;
        } else {
            self.require_use(caller, capability, &session)?;
        }
        let review = self.review(&record.id)?;
        Ok((record, id, review))
    }

    /// One change to one comment, answered with the comment as it is now.
    async fn change_user_comment(
        &self,
        caller: &Caller,
        id: Option<&pb::CommentId>,
        key: Option<&pb::IdempotencyKey>,
        change: impl FnOnce(&mut CommentLedger, &str) -> Result<UserComment, CommentError> + Send,
    ) -> Result<pb::Comment, Status> {
        let (record, id, review) = self.user_comment_target(caller, id)?;
        let idempotency = idempotency(key);
        if let Some(Answer::Comment(answer)) = answered(&review, idempotency.as_ref()) {
            return Ok(answer);
        }
        let comment = change(&mut lock(&review.comments), &id).map_err(|error| comment_failed(&error))?;
        let answer = to_pb(&comment, &record.id);
        remember(&review, idempotency, Answer::Comment(answer.clone()));
        self.keep_comments(&identity::actor(caller), &record, &review).await?;
        Ok(answer)
    }

    /// One change an agent makes to a comment typed into its own session
    /// (DK-9, SM-19), under `capability`, answered with the comment as it
    /// is now. A comment of another session is refused before the access
    /// check, so the agent is told how to find its own.
    async fn agent_comment_change(
        &self,
        caller: &Caller,
        agent: &crate::identity::AgentCaller,
        capability: Capability,
        id: Option<&pb::CommentId>,
        key: Option<&pb::IdempotencyKey>,
        change: impl FnOnce(&mut CommentLedger, &str) -> Result<UserComment, Status> + Send,
    ) -> Result<pb::Comment, Status> {
        let (record, id, review) = self.agent_session(caller, agent, capability, id)?;
        let idempotency = idempotency(key);
        if let Some(Answer::Comment(answer)) = answered(&review, idempotency.as_ref()) {
            return Ok(answer);
        }
        let comment = change(&mut lock(&review.comments), &id)?;
        let answer = to_pb(&comment, &record.id);
        remember(&review, idempotency, Answer::Comment(answer.clone()));
        self.keep_comments(&identity::actor(caller), &record, &review).await?;
        Ok(answer)
    }

    /// Who a draft for a pull request is by, once `caller` may make one in
    /// `session`: the user, or the session's own agent (PR-9).
    fn draft_author(&self, caller: &Caller, session: &humata2_access::SessionRef) -> Result<CommentAuthor, Status> {
        match caller {
            Caller::Agent(agent) => {
                if agent.session.id != session.id {
                    return Err(status::invalid_field(
                        "session",
                        pb::invalid_field::Problem::NotAllowedHere,
                        "an agent drafts only in its own review".to_owned(),
                        "leave the session out, or name your own",
                    ));
                }
                self.require(
                    caller,
                    &Call::Use {
                        capability: Capability::ReviewDraft,
                        target: Target::Channel(session.channel.clone()),
                    },
                )?;
                Ok(CommentAuthor::Agent(agent.agent.clone()))
            }
            _ => {
                self.require_use(caller, Capability::UserReviewState, session)?;
                Ok(CommentAuthor::User)
            }
        }
    }

    /// The repository key and number of `pr` when it is one of the
    /// review's pull requests.
    fn bound_pull_request(&self, session: &str, pr: &pb::PullRequestRef) -> Result<(String, u32), Status> {
        let repo = pr.repo.as_ref().map(wire::repo_key).unwrap_or_default();
        let record = self
            .inner
            .store
            .review_session(session)
            .map_err(|error| status::store(error, &self.inner.machine, "read the review's pull requests"))?;
        let held = record
            .as_ref()
            .is_some_and(|record| record.prs.iter().any(|held| held.number == pr.number && format!("{}/{}", held.owner, held.name) == repo));
        if held {
            Ok((repo, pr.number))
        } else {
            Err(status::invalid_field(
                "destination.pull_request",
                pb::invalid_field::Problem::NotAllowedHere,
                format!("{repo}#{} is not one of the pull requests this session reviews", pr.number),
                "draft only for the pull requests of this review; start another review to cover this one",
            ))
        }
    }

    /// A review chat note becomes a draft for one of the review's pull
    /// requests, in place: the same id, author and anchor (D284).
    pub(super) async fn make_note_a_draft_of(
        &self,
        caller: &Caller,
        request: pb::MakeNoteADraftRequest,
    ) -> Result<pb::Comment, Status> {
        let (record, session, id) = self.locate(caller, request.id.as_ref(), holds_note)?;
        self.require_use(caller, Capability::UserReviewState, &session)?;
        let review = self.review(&record.id)?;
        let idempotency = idempotency(request.idempotency_key.as_ref());
        if let Some(Answer::Comment(answer)) = answered(&review, idempotency.as_ref()) {
            return Ok(answer);
        }
        if let Some(held) = lock(&review.comments).comment(&id) {
            if held.pull_request.is_some() {
                return Ok(to_pb(held, &record.id));
            }
            return Err(comment_failed(&CommentError::NotDraft {
                id: id.clone(),
                status: held.status,
            }));
        }
        let note = lock(&review.chat)
            .chat
            .notes()
            .find(|note| note.id == id)
            .cloned()
            .ok_or_else(|| {
                status::not_found(
                    pb::EntityKind::Comment,
                    &id,
                    format!("there is no review chat note {id}; it may have been dropped"),
                    "list the session's comments and make a draft of one of its notes",
                )
            })?;
        let (repo, number) = match request.pr.as_ref() {
            Some(pr) => self.bound_pull_request(&record.id, pr)?,
            None => {
                let binding = self
                    .inner
                    .store
                    .review_session(&record.id)
                    .map_err(|error| status::store(error, &self.inner.machine, "read the review's pull requests"))?;
                let folder = folder_name(&note.note.file.repo).to_owned();
                let mut hits: Vec<(String, u32)> = binding
                    .iter()
                    .flat_map(|binding| binding.prs.iter())
                    .filter(|held| {
                        format!("{}/{}", held.owner, held.name) == note.note.file.repo || held.name == folder
                    })
                    .map(|held| (format!("{}/{}", held.owner, held.name), held.number))
                    .collect();
                match (hits.pop(), hits.is_empty()) {
                    (Some(only), true) => only,
                    (None, _) => {
                        return Err(status::invalid_field(
                            "id",
                            pb::invalid_field::Problem::NotAllowedHere,
                            format!(
                                "note {id} is on {}, which is none of this review's pull requests' repositories",
                                note.note.file.repo
                            ),
                            "make a draft only of a note on a repository the review covers",
                        ));
                    }
                    _ => {
                        return Err(status::invalid_field(
                            "pr",
                            pb::invalid_field::Problem::Missing,
                            format!("this review has several pull requests in {}", note.note.file.repo),
                            "pass `pr` with the pull request the draft is for",
                        ));
                    }
                }
            }
        };
        let (first, last, side) = match note.note.target {
            NoteTarget::File => (0, 0, Side::New),
            NoteTarget::Hunk { line, side } => (line, line, side),
            NoteTarget::Lines { first, last, side } => (first, last, side),
        };
        let (quoted, blob) = note
            .note
            .anchor
            .as_ref()
            .map(|anchor| (anchor.quoted.clone(), anchor.blob.clone()))
            .unwrap_or_default();
        let anchor = CommentAnchor {
            file: note.note.file.clone(),
            side,
            first,
            last,
            quoted,
            base: String::new(),
            blob,
            detached: false,
        };
        let comment = {
            let mut ledger = lock(&review.comments);
            ledger.adopt_note(
                &id,
                anchor,
                &note.note.text,
                CommentAuthor::Agent(note.author.clone()),
                (&repo, number),
                now(),
            );
            if note.outdated {
                ledger.set_outdated(&id, true);
            }
            ledger.comment(&id).cloned()
        }
        .ok_or_else(|| status::internal("a note made a draft vanished".to_owned(), ""))?;
        let answer = to_pb(&comment, &record.id);
        remember(&review, idempotency, Answer::Comment(answer.clone()));
        self.keep_comments(&identity::actor(caller), &record, &review).await?;
        Ok(answer)
    }

    /// Whether a comment id names a ledger comment (a user's comment, or
    /// a note made a draft) rather than a review chat note.
    pub(super) fn is_ledger_id(&self, caller: &Caller, id: Option<&pb::CommentId>) -> bool {
        is_user_id(id)
            || self
                .locate(caller, id, holds_comment)
                .ok()
                .is_some_and(|(record, _, local)| {
                    self.review(&record.id)
                        .is_ok_and(|review| lock(&review.comments).comment(&local).is_some())
                })
    }

    /// PR-17: how many of the session's pull request drafts were never
    /// submitted (not posted, not dropped).
    pub(crate) fn unsubmitted_pr_drafts(&self, session: &str) -> Result<u32, Status> {
        let review = self.review(session)?;
        let ledger = lock(&review.comments);
        let count = ledger
            .comments()
            .iter()
            .filter(|c| c.pull_request.is_some() && c.status == CommentStatus::Draft && c.archived.is_none())
            .count();
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }

    /// Every comment of the session for a pull request, any status, as the
    /// API shows it.
    pub(super) async fn pr_comments(&self, session: &str) -> Result<Vec<pb::Comment>, Status> {
        let review = self.review(session)?;
        let ledger = lock(&review.comments);
        Ok(ledger
            .comments()
            .iter()
            .filter(|comment| comment.pull_request.is_some())
            .map(|comment| to_pb(comment, session))
            .collect())
    }

    /// GitHub took these drafts; each still a draft becomes posted at `url`.
    pub(super) async fn mark_pr_posted(&self, session: &str, ids: &[String], url: &str) -> Result<(), Status> {
        self.change_pr(session, ids, |ledger, id| {
            let draft = ledger.comment(id).is_some_and(|c| c.status == CommentStatus::Draft);
            draft && ledger.mark_posted(id, url).is_ok()
        })
        .await
    }

    /// A submit was cut short and GitHub lists no review holding these.
    pub(super) async fn mark_pr_unconfirmed(&self, session: &str, ids: &[String]) -> Result<(), Status> {
        self.change_pr(session, ids, |ledger, id| ledger.set_post_unconfirmed(id, true)).await
    }

    /// Marks outdated every open draft for a pull request, on the new side,
    /// whose lines `changed(repo, path, first, last)` says changed (PR-16).
    /// Returns how many it marked.
    pub(super) async fn outdate_drafts(
        &self,
        session: &str,
        changed: &(dyn Fn(&str, &str, u32, u32) -> bool + Sync),
    ) -> Result<usize, Status> {
        let (record, _) = self.session(Some(&pb::SessionId {
            value: session.to_owned(),
        }))?;
        let review = self.review(session)?;
        let marked = {
            let mut ledger = lock(&review.comments);
            let hits: Vec<String> = ledger
                .comments()
                .iter()
                .filter(|c| {
                    c.pull_request.is_some()
                        && c.status == CommentStatus::Draft
                        && c.archived.is_none()
                        && !c.outdated
                        && c.anchor.side == Side::New
                        && changed(&c.anchor.file.repo, &c.anchor.file.path, c.anchor.first, c.anchor.last)
                })
                .map(|c| c.id.clone())
                .collect();
            hits.iter().filter(|id| ledger.set_outdated(id, true)).count()
        };
        if marked > 0 {
            self.keep_comments(&system_actor(), &record, &review).await?;
        }
        Ok(marked)
    }

    /// Applies `change` to each of the drafts `ids` name, then saves and
    /// tells the feed when any changed.
    async fn change_pr(
        &self,
        session: &str,
        ids: &[String],
        mut change: impl FnMut(&mut CommentLedger, &str) -> bool + Send,
    ) -> Result<(), Status> {
        let (record, _) = self.session(Some(&pb::SessionId {
            value: session.to_owned(),
        }))?;
        let review = self.review(session)?;
        let changed = {
            let mut ledger = lock(&review.comments);
            let mut any = false;
            for id in ids {
                let local = id.rsplit('/').next().unwrap_or(id);
                any |= change(&mut ledger, local);
            }
            any
        };
        if changed {
            self.keep_comments(&system_actor(), &record, &review).await?;
        }
        Ok(())
    }

    /// An anchor from the wire. The repo is the one named, else `fallback`,
    /// else "" (the session folder itself) when the path is a file of the
    /// folder outside every clone (a desk session's own document), else the
    /// session's only clone. A new-side passage with lines but no
    /// quoted text is read from the clone, so it can be found again.
    async fn anchor_from_pb(
        &self,
        record: &SessionRecord,
        anchor: &pb::CommentAnchor,
        fallback: Option<String>,
    ) -> Result<CommentAnchor, Status> {
        let path = anchor.file.as_ref().map(|file| file.path.clone()).unwrap_or_default();
        if path.is_empty() {
            return Err(status::missing("anchor.file"));
        }
        let named = anchor
            .file
            .as_ref()
            .and_then(|file| file.repo.as_ref())
            .filter(|repo| !repo.name.is_empty())
            .map(wire::repo_key);
        let repo = match (named, fallback) {
            (Some(repo), _) | (None, Some(repo)) => repo,
            (None, None) => {
                let folder = record.folder.clone();
                let asked_path = path.clone();
                if blocking(move || is_folder_file(&folder, &asked_path)).await? {
                    String::new()
                } else {
                    let mut clones = repo_keys(&self.inner, record, &[]).await?;
                    match (clones.pop(), clones.is_empty()) {
                        // With no clone, the session comments on its own
                        // folder, even on a file it hasn't written yet.
                        (None, _) => String::new(),
                        (Some(only), true) => only,
                        _ => {
                            return Err(status::invalid_field(
                                "anchor.file.repo",
                                pb::invalid_field::Problem::Missing,
                                "the session has more than one repository, and the comment doesn't say which one it is in",
                                "name the repository in the anchor's file, like acme/api",
                            ));
                        }
                    }
                }
            }
        };
        let (first, last) = anchor
            .lines
            .as_ref()
            .map_or((0, 0), |lines| (lines.first.min(lines.last), lines.first.max(lines.last)));
        let side = if anchor.side == pb::DiffSide::Old as i32 { Side::Old } else { Side::New };
        let mut quoted: Vec<String> = if anchor.quoted.is_empty() {
            Vec::new()
        } else {
            anchor.quoted.lines().map(str::to_owned).collect()
        };
        if quoted.is_empty() && side == Side::New && first > 0 {
            let folder = record.folder.clone();
            let (repo, path) = (repo.clone(), path.clone());
            quoted = blocking(move || read_lines(&folder, &repo, &path))
                .await?
                .map(|lines| passage(&lines, first, last))
                .unwrap_or_default();
        }
        Ok(CommentAnchor {
            file: FileRef { repo, path },
            side,
            first,
            last,
            quoted,
            base: anchor.base.clone(),
            blob: anchor.blob_sha.clone(),
            detached: false,
        })
    }
}

/// Every comment of the ledger as the feed tells of it, by wire id.
pub(super) fn all_now(ledger: &CommentLedger, session: &str) -> BTreeMap<String, pb::Comment> {
    ledger
        .comments()
        .iter()
        .map(|comment| (wire_id(session, &comment.id), to_pb(comment, session)))
        .collect()
}

/// A comment's id on the wire: the user's comments carry their session
/// (`<session>/c-<n>`); a note made a draft keeps its bare `note-<n>`.
/// Whether lines in repository `anchor` can carry a draft for a pull
/// request of `pull` (D732): the same key, or a bare name (a clone with
/// no owner, D624) equal to the pull request's name part. The app's
/// `humata2_clients::comments::repo_holds_pull` is the same rule.
fn repo_holds_pull(anchor: &str, pull: &str) -> bool {
    anchor == pull || (!anchor.contains('/') && pull.rsplit_once('/').is_some_and(|(_, name)| name == anchor))
}

fn wire_id(session: &str, id: &str) -> String {
    if id.starts_with("c-") {
        format!("{session}/{id}")
    } else {
        id.to_owned()
    }
}

/// Whether the ledger of `review` holds comment `id`.
fn holds_comment(review: &SessionReview, id: &str) -> bool {
    lock(&review.comments).comment(id).is_some()
}

/// Whether `review` holds `id` as a chat note, or as a draft made of one.
fn holds_note(review: &SessionReview, id: &str) -> bool {
    holds_comment(review, id) || lock(&review.chat).chat.notes().any(|note| note.id == id)
}

/// `id` is a draft for a pull request written by `author`, the agent. A
/// comment the user wrote, for the agent or for GitHub, is the user's to
/// change (SM-13, PR-11); another agent's draft is that agent's.
fn own_draft(ledger: &CommentLedger, id: &str, author: &CommentAuthor) -> Result<(), Status> {
    match ledger.comment(id) {
        None => Err(comment_failed(&CommentError::Unknown { id: id.to_owned() })),
        Some(comment) if comment.pull_request.is_some() && comment.author == *author => Ok(()),
        Some(comment) if comment.author == CommentAuthor::User => Err(status::refused(&Refusal::UserOnly {
            capability: Some(Capability::UserReviewState),
        })),
        Some(comment) => Err(comment_failed(&CommentError::NotYours { id: comment.id.clone() })),
    }
}

/// One batch begun or retried in a Send.
struct Batch {
    key: String,
    text: String,
    comments: Vec<String>,
    /// Typed before, so its state was Unconfirmed.
    retried: bool,
}

/// What a Send with no ids sends: every draft and held comment with text
/// that isn't archived and isn't in a batch.
fn sendable_ids(ledger: &CommentLedger) -> Vec<String> {
    ledger
        .comments()
        .iter()
        .filter(|comment| {
            comment.archived.is_none()
                && comment.pull_request.is_none()
                && matches!(comment.status, CommentStatus::Draft | CommentStatus::Waiting)
                && !comment.text.trim().is_empty()
                && !ledger
                    .deliveries()
                    .iter()
                    .any(|delivery| delivery.comments.contains(&comment.id))
        })
        .map(|comment| comment.id.clone())
        .collect()
}

/// How a batch the last run was typing ended, as the restart report says it.
fn operation_state(pasted: &Pasted) -> crate::restart::OperationState {
    use crate::restart::OperationState as State;
    match pasted {
        Pasted::Typed => State::Finished,
        Pasted::NotTyped(status) => State::Stuck {
            reason: format!(
                "it was not typed into the agent's terminal ({}); the comments are held in the review, so send them again or take them back",
                status.message()
            ),
        },
        Pasted::Unknown(status) => State::Stuck {
            reason: format!(
                "{}; check the agent's terminal for the comments, then send them again or take them back",
                status.message()
            ),
        },
    }
}

/// Records what became of typing batch `key`.
fn settled(ledger: &mut CommentLedger, session: &str, key: &str, pasted: Pasted, in_doubt: bool) {
    let outcome = match pasted {
        Pasted::Typed => Typed::Yes,
        Pasted::NotTyped(status) => {
            // Humata holds the comments; the user sends or takes them back.
            crate::log::say!(
                "humatad2: session {session}: comments {key} weren't typed, so they wait: {}",
                status.message()
            );
            // An earlier try of this batch may have arrived, so "not typed
            // this time" still isn't "never typed": it stays unconfirmed and
            // keeps its key.
            if in_doubt { Typed::Unknown } else { Typed::No }
        }
        Pasted::Unknown(status) => {
            crate::log::say!(
                "humatad2: session {session}: can't tell whether comments {key} were typed: {}",
                status.message()
            );
            Typed::Unknown
        }
    };
    if let Err(error) = ledger.settle(key, outcome, now()) {
        crate::log::say!("humatad2: session {session}: {error}");
    }
}

/// The comment ids a request names, without their session. An id of
/// another session is refused.
fn local_ids(session: &str, ids: &[pb::CommentId]) -> Result<Vec<String>, Status> {
    ids.iter()
        .map(|id| match id.value.split_once('/') {
            Some((of, local)) if of == session => Ok(local.to_owned()),
            Some((of, _)) => Err(status::invalid_field(
                "ids",
                pb::invalid_field::Problem::NotAllowedHere,
                format!("comment {} is in session {of}, not {session}", id.value),
                "send comments of one session at a time",
            )),
            None => Ok(id.value.clone()),
        })
        .collect()
}

/// A revision the caller expects; unset or 0 means no check.
fn expected(revision: Option<&pb::Revision>) -> Option<u64> {
    revision.map(|revision| revision.value).filter(|value| *value != 0)
}

/// `lines` from 1-based `first` to `last`, at most [`SELECTED_LINES_MAX`].
fn passage(lines: &[String], first: u32, last: u32) -> Vec<String> {
    let last = last.min(first.saturating_add(SELECTED_LINES_MAX - 1));
    lines
        .iter()
        .skip(usize::try_from(first.saturating_sub(1)).unwrap_or(usize::MAX))
        .take(usize::try_from(last.saturating_add(1).saturating_sub(first)).unwrap_or(0))
        .cloned()
        .collect()
}

/// Whether `path` is a regular file inside the session folder itself and
/// not inside any clone (a folder with a `.git`). A path that climbs out of
/// the folder, or goes through a link out of it, is not.
fn is_folder_file(folder: &Path, path: &str) -> bool {
    let Ok(root) = std::fs::canonicalize(folder) else {
        return false;
    };
    let Ok(file) = std::fs::canonicalize(folder.join(path)) else {
        return false;
    };
    let Ok(inside) = file.strip_prefix(&root) else {
        return false;
    };
    let in_clone = inside
        .components()
        .next()
        .is_some_and(|first| root.join(first).join(".git").exists());
    !in_clone && file.is_file()
}

/// The lines of `path` in the session's clone of `repo`. A file that isn't
/// there is an empty file; a clone or file that can't be read safely gives
/// `None`, so nothing is concluded from it. The repo "" is the session
/// folder itself, for the session's own documents.
fn read_lines(folder: &Path, repo: &str, path: &str) -> Option<Vec<String>> {
    let clone = folder.join(folder_name(repo));
    let root = std::fs::canonicalize(&clone).ok()?;
    let file = match std::fs::canonicalize(clone.join(path)) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Some(Vec::new()),
        Err(_) => return None,
    };
    if !file.starts_with(&root) {
        return None;
    }
    let text = std::fs::read_to_string(file).ok()?;
    Some(text.lines().map(str::to_owned).collect())
}

/// The user's writes of files in one session, as the review service counts them.
#[derive(Default)]
pub(super) struct Writes(std::sync::Mutex<WritesState>);

#[derive(Default)]
struct WritesState {
    /// Writes between taking their hold and finishing.
    pending: usize,
    /// Writes finished, so a look that started before one can tell.
    finished: u64,
}

/// A user write under way in one session (BUG 42). Dropping it, however the
/// write ended, counts it finished.
struct Held {
    review: Review,
    session: String,
    state: std::sync::Arc<SessionReview>,
}

impl Drop for Held {
    fn drop(&mut self) {
        let mut writes = lock(&self.state.user_writes.0);
        writes.pending = writes.pending.saturating_sub(1);
        writes.finished += 1;
    }
}

impl crate::files::UserWrites for Review {
    fn writing(&self, session: &str) -> Option<Box<dyn crate::files::UserWrite>> {
        let state = self.review(session).ok()?;
        lock(&state.user_writes.0).pending += 1;
        Some(Box::new(Held {
            review: self.clone(),
            session: session.to_owned(),
            state,
        }))
    }
}

#[tonic::async_trait]
impl crate::files::UserWrite for Held {
    async fn wrote(self: Box<Self>, repo: &pb::RepoRef, path: &str) {
        let record = match self.review.inner.store.session(&self.session) {
            Ok(Some(record)) => record,
            _ => return,
        };
        self.review
            .rebaseline_user_comments(&record, &self.state, &wire::repo_key(repo), path)
            .await;
    }
}

impl Review {
    /// The user wrote `path` of the session's clone of `repo`: the comments
    /// on it follow the new text and stay as they were, a sent one still
    /// sent. Only the agent's change addresses a comment.
    async fn rebaseline_user_comments(&self, record: &SessionRecord, review: &SessionReview, repo: &str, path: &str) {
        let watched: Vec<(String, Vec<String>, u32)> = lock(&review.comments)
            .comments()
            .iter()
            .filter(|comment| {
                comment.archived.is_none()
                    && comment.pull_request.is_none()
                    && comment.anchor.side == Side::New
                    && !comment.anchor.quoted.is_empty()
                    && comment.anchor.file.repo == repo
                    && comment.anchor.file.path == path
                    && matches!(
                        comment.status,
                        CommentStatus::Draft | CommentStatus::Waiting | CommentStatus::Sent
                    )
            })
            .map(|comment| (comment.id.clone(), comment.anchor.quoted.clone(), comment.anchor.first))
            .collect();
        if watched.is_empty() {
            return;
        }
        let folder = record.folder.clone();
        let (repo_key, file) = (repo.to_owned(), path.to_owned());
        let lines = match blocking(move || read_lines(&folder, &repo_key, &file)).await {
            Ok(Some(lines)) => lines,
            _ => return,
        };
        let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
        let mut changed = false;
        {
            let mut ledger = lock(&review.comments);
            for (id, quoted, first) in watched {
                match place(&quoted, first, &lines) {
                    Placement::Same => {}
                    Placement::Moved { first, last } => changed |= ledger.moved(&id, first, last),
                    Placement::Changed { first, last, now: after } => {
                        changed |= ledger.rebaseline(&id, first, last, after) || ledger.changed(&id, first, last, Vec::new(), now());
                    }
                    Placement::Gone => changed |= ledger.gone(&id),
                }
            }
        }
        if changed && let Err(status) = self.keep_comments(&system_actor(), record, review).await {
            crate::log::say!(
                "humatad2: session {}: couldn't save the comments after a write: {}",
                record.id,
                status.message()
            );
        }
    }
}

fn status_from_pb(status: i32) -> Option<CommentStatus> {
    match pb::CommentStatus::try_from(status).ok()? {
        pb::CommentStatus::Draft => Some(CommentStatus::Draft),
        pb::CommentStatus::Queued => Some(CommentStatus::Waiting),
        pb::CommentStatus::Sent => Some(CommentStatus::Sent),
        pb::CommentStatus::Addressed => Some(CommentStatus::Addressed),
        pb::CommentStatus::Resolved => Some(CommentStatus::Resolved),
        pb::CommentStatus::Posted => Some(CommentStatus::Posted),
        pb::CommentStatus::Unspecified => None,
    }
}

fn status_to_pb(status: CommentStatus) -> pb::CommentStatus {
    match status {
        CommentStatus::Draft => pb::CommentStatus::Draft,
        CommentStatus::Waiting => pb::CommentStatus::Queued,
        CommentStatus::Sent => pb::CommentStatus::Sent,
        CommentStatus::Addressed => pb::CommentStatus::Addressed,
        CommentStatus::Resolved => pb::CommentStatus::Resolved,
        CommentStatus::Posted => pb::CommentStatus::Posted,
    }
}

fn timestamp(seconds: i64) -> Timestamp {
    Timestamp { seconds, nanos: 0 }
}

fn user_actor() -> pb::Actor {
    pb::Actor {
        actor: Some(pb::actor::Actor::User(pb::UserActor {
            via: pb::user_actor::Via::App as i32,
            machine: None,
        })),
    }
}

/// The actor for changes nobody called for: a refresh or a recovery.
fn system_actor() -> pb::Actor {
    pb::Actor {
        actor: Some(pb::actor::Actor::System(pb::SystemActor { reason: 0 })),
    }
}

fn actor_of(author: &CommentAuthor, session: &str) -> pb::Actor {
    match author {
        CommentAuthor::User => user_actor(),
        CommentAuthor::Agent(name) => pb::Actor {
            actor: Some(pb::actor::Actor::Agent(pb::AgentActor {
                session: Some(pb::SessionId {
                    value: session.to_owned(),
                }),
                agent: Some(pb::AgentId { value: name.clone() }),
                ..pb::AgentActor::default()
            })),
        },
    }
}

fn author_of(actor: Option<&pb::Actor>) -> CommentAuthor {
    match actor.and_then(|actor| actor.actor.as_ref()) {
        Some(pb::actor::Actor::Agent(agent)) => CommentAuthor::Agent(
            agent
                .agent
                .as_ref()
                .map(|id| id.value.clone())
                .unwrap_or_default(),
        ),
        _ => CommentAuthor::User,
    }
}

/// A comment as the API shows it.
fn to_pb(comment: &UserComment, session: &str) -> pb::Comment {
    let anchor = &comment.anchor;
    let whole_file = anchor.first == 0 && anchor.last == 0;
    pb::Comment {
        id: Some(pb::CommentId {
            value: wire_id(session, &comment.id),
        }),
        session: Some(pb::SessionId {
            value: session.to_owned(),
        }),
        anchor: Some(pb::CommentAnchor {
            file: Some(pb::RepoPath {
                // A file of the session folder (repo "") goes out with no
                // repo at all, not an empty one.
                repo: (!anchor.file.repo.is_empty()).then(|| wire::repo_ref(&anchor.file.repo)),
                path: anchor.file.path.clone(),
            }),
            lines: (!whole_file).then_some(pb::LineRange {
                first: anchor.first,
                last: anchor.last,
            }),
            side: match anchor.side {
                Side::Old => pb::DiffSide::Old as i32,
                Side::New => pb::DiffSide::New as i32,
            },
            quoted: anchor.quoted.join("\n"),
            blob_sha: anchor.blob.clone(),
            detached: anchor.detached,
            outdated: comment.outdated,
            base: anchor.base.clone(),
        }),
        text: comment.text.clone(),
        author: Some(actor_of(&comment.author, session)),
        destination: Some(pb::CommentDestination {
            destination: Some(match &comment.pull_request {
                Some((repo, number)) => pb::comment_destination::Destination::PullRequest(pb::PullRequestRef {
                    repo: Some(wire::repo_ref(repo)),
                    number: *number,
                    ..pb::PullRequestRef::default()
                }),
                None => pb::comment_destination::Destination::Terminal(pb::SessionTerminal {}),
            }),
        }),
        posted_url: comment.posted_url.clone().unwrap_or_default(),
        post_unconfirmed: comment.post_unconfirmed,
        status: status_to_pb(comment.status) as i32,
        created_at: Some(timestamp(comment.created_at)),
        revision: Some(pb::Revision { value: comment.revision }),
        addressed: comment.addressed.as_ref().map(|addressed| pb::Timestamped {
            at: Some(timestamp(addressed.at)),
            by: None,
        }),
        addressed_text: comment
            .addressed
            .as_ref()
            .map(|addressed| addressed.now.join("\n"))
            .unwrap_or_default(),
        resolved: comment.resolved.as_ref().map(|stamp| pb::Timestamped {
            at: Some(timestamp(stamp.at)),
            by: Some(actor_of(&stamp.by, session)),
        }),
        archived_at: comment.archived.map(timestamp),
        replies: comment
            .replies
            .iter()
            .map(|reply| pb::CommentReply {
                id: reply.id.clone(),
                author: Some(actor_of(&reply.by, session)),
                text: reply.text.clone(),
                at: Some(timestamp(reply.at)),
            })
            .collect(),
        delivery_unconfirmed: comment.unconfirmed,
        sent_at: comment.sent_at.map(timestamp),
        ..pb::Comment::default()
    }
}

/// A saved comment back: the inverse of [`to_pb`]; `None` when this build
/// can't read it.
fn from_pb(comment: &pb::Comment) -> Option<UserComment> {
    let id = comment.id.as_ref()?.value.rsplit('/').next()?.to_owned();
    if !id.starts_with("c-") && !id.starts_with("note-") {
        return None;
    }
    let anchor = comment.anchor.as_ref()?;
    let file = anchor.file.as_ref()?;
    let (first, last) = anchor.lines.as_ref().map_or((0, 0), |lines| (lines.first, lines.last));
    let seconds = |at: &Option<Timestamp>| at.as_ref().map_or(0, |at| at.seconds);
    let status = status_from_pb(comment.status)?;
    Some(UserComment {
        id,
        anchor: CommentAnchor {
            file: FileRef {
                repo: file.repo.as_ref().map(wire::repo_key).unwrap_or_default(),
                path: file.path.clone(),
            },
            side: if anchor.side == pb::DiffSide::Old as i32 { Side::Old } else { Side::New },
            first,
            last,
            quoted: if anchor.quoted.is_empty() {
                Vec::new()
            } else {
                anchor.quoted.lines().map(str::to_owned).collect()
            },
            base: anchor.base.clone(),
            blob: anchor.blob_sha.clone(),
            detached: anchor.detached,
        },
        text: comment.text.clone(),
        status,
        created_at: seconds(&comment.created_at),
        revision: comment.revision.as_ref().map_or(1, |revision| revision.value.max(1)),
        sent_at: comment.sent_at.as_ref().map(|at| at.seconds),
        addressed: comment.addressed.as_ref().map(|addressed| Addressed {
            at: seconds(&addressed.at),
            now: if comment.addressed_text.is_empty() {
                Vec::new()
            } else {
                comment.addressed_text.lines().map(str::to_owned).collect()
            },
        }),
        resolved: comment.resolved.as_ref().map(|resolved| CommentStamp {
            by: author_of(resolved.by.as_ref()),
            at: seconds(&resolved.at),
        }),
        archived: comment.archived_at.as_ref().map(|at| at.seconds),
        replies: comment
            .replies
            .iter()
            .map(|reply| CommentReply {
                id: reply.id.clone(),
                by: author_of(reply.author.as_ref()),
                text: reply.text.clone(),
                at: seconds(&reply.at),
            })
            .collect(),
        unconfirmed: comment.delivery_unconfirmed,
        author: author_of(comment.author.as_ref()),
        pull_request: match comment.destination.as_ref().and_then(|destination| destination.destination.as_ref()) {
            Some(pb::comment_destination::Destination::PullRequest(pr)) => {
                Some((pr.repo.as_ref().map(wire::repo_key).unwrap_or_default(), pr.number))
            }
            _ => None,
        },
        posted_url: (!comment.posted_url.is_empty()).then(|| comment.posted_url.clone()),
        outdated: anchor.outdated,
        post_unconfirmed: comment.post_unconfirmed,
    })
}

/// A batch as its row keeps it: the delivery's fields, apart from its
/// state, which has a column of its own.
#[derive(Clone, PartialEq, prost::Message)]
struct DeliveryBytes {
    #[prost(string, tag = "1")]
    key: String,
    #[prost(string, repeated, tag = "2")]
    comments: Vec<String>,
    #[prost(string, tag = "3")]
    text: String,
    #[prost(int64, tag = "4")]
    made_unix_s: i64,
    #[prost(string, tag = "5")]
    tab: String,
    #[prost(uint64, tag = "6")]
    holder: u64,
}

/// What of a session's comments was last saved or read, so the next save
/// writes only what differs from it.
#[derive(Default)]
pub(super) struct Kept {
    /// Each comment as saved, with the state of the batch it was in.
    comments: BTreeMap<String, (UserComment, CommentDeliveryState)>,
    deliveries: BTreeMap<String, Delivery>,
    /// The last comment and delivery numbers saved.
    numbers: (u64, u64),
}

fn store_state(state: DeliveryState) -> CommentDeliveryState {
    match state {
        DeliveryState::Sending => CommentDeliveryState::Sending,
        DeliveryState::Unconfirmed => CommentDeliveryState::Unconfirmed,
    }
}

/// What saving `ledger` writes, given what was `kept`, and what is kept
/// once it is written, and what WatchComments is told (a changed comment
/// per put, a dropped id per removal); `None` when nothing differs.
fn difference(
    session: &str,
    ledger: &CommentLedger,
    kept: &Kept,
) -> Option<(CommentChange, Kept, Vec<pb::CommentFeedEvent>)> {
    let in_batch: BTreeMap<&str, CommentDeliveryState> = ledger
        .deliveries()
        .iter()
        .flat_map(|delivery| delivery.comments.iter().map(|id| (id.as_str(), store_state(delivery.state))))
        .collect();
    let at = now().saturating_mul(1000);
    let mut change = CommentChange {
        session: session.to_owned(),
        put: Vec::new(),
        removed: Vec::new(),
        put_deliveries: Vec::new(),
        removed_deliveries: Vec::new(),
        numbers: CommentNumbers::default(),
    };
    let mut now_kept = Kept::default();
    let mut told = Vec::new();
    for comment in ledger.comments() {
        let state = in_batch.get(comment.id.as_str()).copied().unwrap_or(CommentDeliveryState::None);
        let entry = (comment.clone(), state);
        if kept.comments.get(&comment.id) != Some(&entry) {
            let encoded = to_pb(comment, session);
            change.put.push(CommentRecord {
                id: comment.id.clone(),
                archived: comment.archived.is_some(),
                delivery_state: state,
                comment: encoded.encode_to_vec(),
                updated_unix_ms: at,
            });
            told.push(pb::CommentFeedEvent {
                event: Some(pb::comment_feed_event::Event::Changed(encoded)),
            });
        }
        now_kept.comments.insert(comment.id.clone(), entry);
    }
    change.removed = kept.comments.keys().filter(|id| !now_kept.comments.contains_key(*id)).cloned().collect();
    told.extend(change.removed.iter().map(|id| pb::CommentFeedEvent {
        event: Some(pb::comment_feed_event::Event::Dropped(pb::CommentId {
            value: wire_id(session, id),
        })),
    }));
    for delivery in ledger.deliveries() {
        if kept.deliveries.get(&delivery.key) != Some(delivery) {
            change.put_deliveries.push(CommentDeliveryRecord {
                key: delivery.key.clone(),
                state: store_state(delivery.state),
                delivery: DeliveryBytes {
                    key: delivery.key.clone(),
                    comments: delivery.comments.clone(),
                    text: delivery.text.clone(),
                    made_unix_s: delivery.made_at,
                    tab: delivery.tab.clone(),
                    holder: delivery.holder,
                }
                .encode_to_vec(),
            });
        }
        now_kept.deliveries.insert(delivery.key.clone(), delivery.clone());
    }
    change.removed_deliveries =
        kept.deliveries.keys().filter(|key| !now_kept.deliveries.contains_key(*key)).cloned().collect();
    let parts = ledger.parts();
    now_kept.numbers = (parts.last_comment, parts.last_delivery);
    change.numbers = CommentNumbers {
        last_comment: parts.last_comment,
        last_delivery: parts.last_delivery,
    };
    let unchanged = change.put.is_empty()
        && change.removed.is_empty()
        && change.put_deliveries.is_empty()
        && change.removed_deliveries.is_empty()
        && now_kept.numbers == kept.numbers;
    (!unchanged).then_some((change, now_kept, told))
}

/// A session's saved comments as a ledger, and what that saved. What can't
/// be read is logged and left out: the review still opens.
pub(super) fn load(store: &Store, session: &str) -> Result<(CommentLedger, Kept), StoreError> {
    let saved = store.comments(session)?;
    let lost = |what: &str| {
        crate::log::say!(
            "humatad2: session {session}'s saved comments had {what} this build can't read, so the review starts without it"
        );
    };
    let mut kept = Kept {
        numbers: (saved.numbers.last_comment, saved.numbers.last_delivery),
        ..Kept::default()
    };
    let mut comments = Vec::new();
    for row in saved.comments {
        let comment = pb::Comment::decode(row.comment.as_slice()).ok().and_then(|comment| from_pb(&comment));
        match comment {
            Some(comment) => {
                kept.comments.insert(comment.id.clone(), (comment.clone(), row.delivery_state));
                comments.push(comment);
            }
            None => lost("a comment"),
        }
    }
    let mut deliveries = Vec::new();
    for row in saved.deliveries {
        let Ok(bytes) = DeliveryBytes::decode(row.delivery.as_slice()) else {
            lost("a batch");
            continue;
        };
        let delivery = Delivery {
            key: bytes.key,
            comments: bytes.comments,
            text: bytes.text,
            // A state this build doesn't know is never typed again on its
            // own.
            state: if row.state == CommentDeliveryState::Sending {
                DeliveryState::Sending
            } else {
                DeliveryState::Unconfirmed
            },
            made_at: bytes.made_unix_s,
            tab: bytes.tab,
            holder: bytes.holder,
        };
        kept.deliveries.insert(delivery.key.clone(), delivery.clone());
        deliveries.push(delivery);
    }
    // Ids are never reused, even when a comment couldn't be read back.
    let numbered = |id: &str| id.rsplit('-').next().and_then(|n| n.parse::<u64>().ok()).unwrap_or(0);
    let last_comment = comments
        .iter()
        .filter(|comment| comment.id.starts_with("c-"))
        .map(|comment| numbered(&comment.id))
        .fold(saved.numbers.last_comment, u64::max);
    let last_delivery =
        deliveries.iter().map(|delivery| numbered(&delivery.key)).fold(saved.numbers.last_delivery, u64::max);
    let ledger = CommentLedger::from_parts(LedgerParts {
        comments,
        deliveries,
        last_comment,
        last_delivery,
    });
    Ok((ledger, kept))
}

/// A refused comment change, in the API's terms. Each message already says
/// what to do.
fn comment_failed(error: &CommentError) -> Status {
    let message = error.to_string();
    match error {
        CommentError::Unknown { id } => status::not_found(
            pb::EntityKind::Comment,
            id,
            message,
            "list the session's comments and pick one of those",
        ),
        CommentError::UnknownDelivery { key } => status::not_found(
            pb::EntityKind::Comment,
            key,
            message,
            "list the session's comments to see where they are now",
        ),
        CommentError::Stale { id, expected, actual } => status::failure(
            Code::Aborted,
            status::error(
                message,
                "read the comment again, then repeat the change",
                pb::error::Kind::RevisionConflict(pb::RevisionConflict {
                    kind: pb::EntityKind::Comment as i32,
                    id: id.clone(),
                    expected: Some(pb::Revision { value: *expected }),
                    actual: Some(pb::Revision { value: *actual }),
                }),
            ),
        ),
        _ => {
            let fix = match error {
                CommentError::Blank { .. } | CommentError::NothingToSend => {
                    "write the comment's text, then send it"
                }
                CommentError::InDelivery { .. } => "wait for the comments to be typed, or take them back",
                CommentError::NotDraft { .. } | CommentError::Archived { .. } => {
                    "list the session's comments to see what state this one is in"
                }
                CommentError::ReplyNeeded { .. } => "pass `reply` with a short note on what you did",
                CommentError::NotSent { .. } => "answer only comments that were sent: list them with comments_list",
                CommentError::AlreadyResolved { .. } => "nothing to do; the user reopens it if more is needed",
                CommentError::ForPullRequest { .. } => {
                    "post it from Submit review; to tell the agent something, make a comment for the agent instead"
                }
                CommentError::AlreadyPosted { .. } => "open the review on GitHub and change the comment there",
                CommentError::NotYours { .. } => "change only drafts you wrote; draft your own with review_draft",
                _ => "",
            };
            status::failure(
                Code::FailedPrecondition,
                status::error(
                    message,
                    fix,
                    pb::error::Kind::InvalidField(pb::InvalidField {
                        field: "id".to_owned(),
                        problem: pb::invalid_field::Problem::NotAllowedHere as i32,
                    }),
                ),
            )
        }
    }
}

#[cfg(test)]
mod repo_rule_tests {
    use super::repo_holds_pull;

    #[test]
    fn a_draft_fits_the_same_repository_or_its_bare_name_only() {
        assert!(repo_holds_pull("acme/api", "acme/api"));
        assert!(repo_holds_pull("api", "acme/api"));
        assert!(!repo_holds_pull("other/api", "acme/api"));
        assert!(!repo_holds_pull("web", "acme/api"));
        assert!(!repo_holds_pull("api", "api2"));
    }
}

//! [`CommentsClient`] for the real daemon, over its CommentService (D151,
//! D161, D284). Every comment the daemon keeps comes through with its
//! destination: terminal drafts, the review chat's notes and pull request
//! drafts. Views that only deal with the terminal filter on
//! [`Comment::destination`].

use humata2_clients::comments::{
    Anchor, Author, Comment, CommentEvent, CommentEvents, CommentId, CommentsClient, Destination,
    Listing, Sent, Side, Status, ThreadReply, Which,
};
use humata2_clients::model::SessionId;
use humata2_clients::{DaemonError, Reply};
use humata2_clients_live_core::{LiveConnection, MESSAGE_MAX_BYTES, Wire, fresh_key, map};
use crate::humata2_grpc::humata::v1 as pb;
use crate::humata2_grpc::humata::v1::comment_service_client::CommentServiceClient;
use tokio::sync::mpsc;
use tonic::Streaming;

/// Comments asked for per page; the daemon may give fewer.
const PAGE_SIZE: u32 = 200;

/// Review comments on the real daemon. Cheap to clone.
#[derive(Clone)]
pub struct LiveComments {
    shared: LiveConnection,
}

impl LiveComments {
    /// Comments over `conn`'s connection.
    #[must_use]
    pub fn new(conn: impl AsRef<LiveConnection>) -> Self {
        Self {
            shared: conn.as_ref().clone(),
        }
    }
}

fn comments_service(shared: &LiveConnection) -> CommentServiceClient<Wire> {
    CommentServiceClient::new(shared.wire())
        .max_decoding_message_size(MESSAGE_MAX_BYTES)
        .max_encoding_message_size(MESSAGE_MAX_BYTES)
}

// ---------------------------------------------------------------------------
// Daemon messages as the app's model, and back. Pure.
// ---------------------------------------------------------------------------

fn author(actor: Option<&pb::Actor>) -> Author {
    use pb::actor::Actor;
    match actor.and_then(|actor| actor.actor.as_ref()) {
        Some(Actor::Agent(agent)) => {
            let name = agent.agent.as_ref().map(|id| id.value.clone()).unwrap_or_default();
            Author::Agent(if name.is_empty() { "agent".to_owned() } else { name })
        }
        Some(Actor::Script(script)) => Author::Agent(script.name.clone()),
        Some(Actor::System(_)) => Author::Agent("Humata".to_owned()),
        Some(Actor::User(_)) | None => Author::User,
    }
}

/// `text` as one entry per line; empty text is no lines.
fn lines_of(text: &str) -> Vec<String> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.split('\n').map(str::to_owned).collect()
    }
}

/// `owner/name`, or just the name when the daemon gave no owner.
fn repo_name(repo: &pb::RepoRef) -> String {
    if repo.owner.is_empty() {
        repo.name.clone()
    } else {
        map::repo_text(repo)
    }
}

/// The daemon's repository for the app's text; text that isn't
/// `owner/name` goes as a bare name, which the daemon refuses in its own
/// words if it doesn't know it.
fn pb_repo(text: &str) -> pb::RepoRef {
    map::parse_repo(text).unwrap_or_else(|| pb::RepoRef {
        owner: String::new(),
        name: text.to_owned(),
    })
}

/// The repository an app anchor names. A document of the session's own
/// folder (a desk session's `rfc-brin.md`) has no repository, so the app's
/// empty text goes as none, and the daemon takes the file as the folder's.
fn pb_repo_named(text: &str) -> Option<pb::RepoRef> {
    (!text.is_empty()).then(|| pb_repo(text))
}

/// The app's destination for a daemon comment. A comment with none is for
/// the session's terminal, as every comment was before notes and pull
/// request drafts.
fn destination(destination: Option<&pb::CommentDestination>) -> Destination {
    use pb::comment_destination::Destination as Wire;
    match destination.and_then(|destination| destination.destination.as_ref()) {
        None | Some(Wire::Terminal(_)) => Destination::Terminal,
        Some(Wire::Note(_)) => Destination::Note,
        Some(Wire::PullRequest(pull_request)) => Destination::PullRequest {
            repo: pull_request.repo.as_ref().map(repo_name).unwrap_or_default(),
            number: pull_request.number,
        },
    }
}

/// The daemon's destination for one of the app's.
fn pb_destination(destination: &Destination) -> pb::CommentDestination {
    use pb::comment_destination::Destination as Wire;
    let wire = match destination {
        Destination::Terminal => Wire::Terminal(pb::SessionTerminal {}),
        Destination::Note => Wire::Note(pb::UserNote {}),
        Destination::PullRequest { repo, number } => Wire::PullRequest(pb::PullRequestRef {
            repo: Some(pb_repo(repo)),
            number: *number,
            note: String::new(),
        }),
    };
    pb::CommentDestination {
        destination: Some(wire),
    }
}

fn anchor(anchor: &pb::CommentAnchor) -> Anchor {
    let (repo, path) = match &anchor.file {
        Some(file) => (
            file.repo.as_ref().map(repo_name).unwrap_or_default(),
            file.path.clone(),
        ),
        None => (String::new(), String::new()),
    };
    let lines = anchor.lines.clone().unwrap_or_default();
    let (first, last) = Anchor::normalized_range(lines.first, lines.last);
    Anchor {
        repo,
        path,
        side: if pb::DiffSide::try_from(anchor.side) == Ok(pb::DiffSide::Old) {
            Side::Old
        } else {
            Side::New
        },
        first,
        last,
        quoted: lines_of(&anchor.quoted),
        base: (!anchor.base.is_empty()).then(|| anchor.base.clone()),
        detached: anchor.detached,
        outdated: anchor.outdated,
    }
}

fn pb_anchor(anchor: &Anchor) -> pb::CommentAnchor {
    let (first, last) = anchor.line_range();
    pb::CommentAnchor {
        file: Some(pb::RepoPath {
            repo: pb_repo_named(&anchor.repo),
            path: anchor.path.clone(),
        }),
        lines: Some(pb::LineRange { first, last }),
        side: match anchor.side {
            Side::Old => pb::DiffSide::Old,
            Side::New => pb::DiffSide::New,
        } as i32,
        quoted: anchor.quoted.join("\n"),
        detached: anchor.detached,
        outdated: anchor.outdated,
        base: anchor.base.clone().unwrap_or_default(),
        ..pb::CommentAnchor::default()
    }
}

/// The daemon's comment as the app's.
fn comment(comment: &pb::Comment) -> Comment {
    let status = match pb::CommentStatus::try_from(comment.status) {
        Ok(pb::CommentStatus::Queued) => Status::Waiting,
        Ok(pb::CommentStatus::Sent) => Status::Sent,
        Ok(pb::CommentStatus::Posted) => Status::Posted,
        Ok(pb::CommentStatus::Addressed) => Status::Addressed,
        Ok(pb::CommentStatus::Resolved) => Status::Resolved,
        Ok(pb::CommentStatus::Draft | pb::CommentStatus::Unspecified) | Err(_) => Status::Draft,
    };
    Comment {
        id: CommentId::new(comment.id.as_ref().map(|id| id.value.clone()).unwrap_or_default()),
        session: map::session_id(comment.session.as_ref()),
        anchor: comment.anchor.as_ref().map(anchor).unwrap_or_else(|| Anchor {
            repo: String::new(),
            path: String::new(),
            side: Side::New,
            first: 0,
            last: 0,
            quoted: Vec::new(),
            base: None,
            detached: false,
            outdated: false,
        }),
        text: comment.text.clone(),
        status,
        revision: comment.revision.as_ref().map_or(0, |revision| revision.value),
        unconfirmed: comment.delivery_unconfirmed,
        archived: comment.archived_at.is_some(),
        replies: comment
            .replies
            .iter()
            .map(|reply| ThreadReply {
                id: reply.id.clone(),
                author: author(reply.author.as_ref()),
                text: reply.text.clone(),
            })
            .collect(),
        resolved_by: (status == Status::Resolved)
            .then(|| comment.resolved.as_ref().map(|resolved| author(resolved.by.as_ref())))
            .flatten(),
        addressed: (status == Status::Addressed).then(|| lines_of(&comment.addressed_text)),
        destination: destination(comment.destination.as_ref()),
        author: author(comment.author.as_ref()),
        posted_url: (!comment.posted_url.is_empty()).then(|| comment.posted_url.clone()),
        post_unconfirmed: comment.post_unconfirmed,
    }
}

pub(crate) fn comments(comments: &[pb::Comment]) -> Vec<Comment> {
    comments.iter().map(comment).collect()
}

fn event(feed: pb::CommentFeedEvent) -> Option<CommentEvent> {
    use pb::comment_feed_event::Event;
    match feed.event? {
        Event::Changed(changed) => Some(CommentEvent::Changed(comment(&changed))),
        Event::Dropped(id) => Some(CommentEvent::Dropped(CommentId::new(id.value))),
        Event::Resync(_) => Some(CommentEvent::Resync),
    }
}

/// The statuses to ask for, and whether to ask for archived comments only.
/// No statuses means every status but resolved.
fn which_request(which: Which) -> (Vec<i32>, bool) {
    use pb::CommentStatus as S;
    let live = [S::Draft, S::Queued, S::Sent, S::Addressed, S::Posted];
    match which {
        Which::Open => (Vec::new(), false),
        Which::WithResolved => (
            live.into_iter().chain([S::Resolved]).map(|status| status as i32).collect(),
            false,
        ),
        // Archived ones of any status, resolved included.
        Which::Archived => (
            live.into_iter().chain([S::Resolved]).map(|status| status as i32).collect(),
            true,
        ),
    }
}

fn comment_id(id: &CommentId) -> Option<pb::CommentId> {
    Some(pb::CommentId {
        value: id.as_str().to_owned(),
    })
}

fn session_id(session: &SessionId) -> Option<pb::SessionId> {
    Some(pb::SessionId {
        value: session.as_str().to_owned(),
    })
}

fn revision(revision: u64) -> Option<pb::Revision> {
    Some(pb::Revision { value: revision })
}

// ---------------------------------------------------------------------------
// Calls.
// ---------------------------------------------------------------------------

async fn list_comments(
    shared: &LiveConnection,
    session: &SessionId,
    which: Which,
) -> Result<Listing, DaemonError> {
    const WHAT: &str = "The comments";
    let (statuses, archived) = which_request(which);
    let mut found = Vec::new();
    let mut draft_count = None;
    let mut token = String::new();
    loop {
        let reply = comments_service(shared)
            .list_comments(pb::ListCommentsRequest {
                session: session_id(session),
                statuses: statuses.clone(),
                file: None,
                page: Some(pb::PageRequest {
                    page_size: PAGE_SIZE,
                    page_token: token,
                }),
                archived,
            })
            .await
            .map_err(|status| shared.error(&status, WHAT))?
            .into_inner();
        // The badge is the first page's; the pages read one snapshot.
        draft_count.get_or_insert(reply.draft_count);
        found.extend(comments(&reply.comments));
        token = reply.page.map(|page| page.next_page_token).unwrap_or_default();
        if token.is_empty() {
            break;
        }
    }
    Ok(Listing {
        comments: found,
        draft_count: draft_count.unwrap_or_default(),
    })
}

/// Opens the daemon's feed and forwards it. The stream is open before
/// this answers, so a change made after it is not missed.
async fn watch_comments(
    shared: &LiveConnection,
    session: &SessionId,
) -> Result<CommentEvents, DaemonError> {
    let stream = comments_service(shared)
        .watch_comments(pb::WatchCommentsRequest {
            session: session_id(session),
        })
        .await
        .map_err(|status| shared.error(&status, "Following review comments"))?
        .into_inner();
    let (sender, receiver) = mpsc::unbounded_channel();
    shared.spawn(forward(stream, sender));
    Ok(receiver)
}

/// Maps the feed until it ends, fails, or nobody listens. A feed that ends
/// or fails tells the app to list again (it can't follow again by itself),
/// and the listing then shows what is true or says the daemon is gone.
async fn forward(
    mut stream: Streaming<pb::CommentFeedEvent>,
    sender: mpsc::UnboundedSender<CommentEvent>,
) {
    loop {
        let next = tokio::select! {
            () = sender.closed() => return,
            next = stream.message() => next,
        };
        match next {
            Ok(Some(feed)) => {
                if let Some(event) = event(feed) {
                    if sender.send(event).is_err() {
                        return;
                    }
                }
            }
            Ok(None) | Err(_) => {
                let _ = sender.send(CommentEvent::Resync);
                return;
            }
        }
    }
}

impl CommentsClient for LiveComments {
    fn list(&self, session: &SessionId, which: Which) -> Reply<Listing> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move { list_comments(&shared, &session, which).await })
    }

    fn watch(&self, session: &SessionId) -> Reply<CommentEvents> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move { watch_comments(&shared, &session).await })
    }

    fn create(&self, session: &SessionId, anchor: Anchor, text: &str, key: &str) -> Reply<Comment> {
        const WHAT: &str = "This comment";
        let shared = self.shared.clone();
        let (session, text, key) = (session.clone(), text.to_owned(), key.to_owned());
        Box::pin(async move {
            let answer = comments_service(&shared)
                .create_comment(pb::CreateCommentRequest {
                    idempotency_key: Some(pb::IdempotencyKey { value: key }),
                    session: session_id(&session),
                    anchor: Some(pb_anchor(&anchor)),
                    text,
                    destination: Some(pb_destination(&Destination::Terminal)),
                })
                .await
                .map_err(|status| shared.error(&status, WHAT))?
                .into_inner();
            Ok(comment(&answer))
        })
    }

    fn create_for_pull_request(
        &self,
        session: &SessionId,
        anchor: Anchor,
        text: &str,
        repo: &str,
        number: u32,
        key: &str,
    ) -> Reply<Comment> {
        const WHAT: &str = "This draft";
        let shared = self.shared.clone();
        let (session, text, key) = (session.clone(), text.to_owned(), key.to_owned());
        let destination = Destination::PullRequest { repo: repo.to_owned(), number };
        Box::pin(async move {
            let answer = comments_service(&shared)
                .create_comment(pb::CreateCommentRequest {
                    idempotency_key: Some(pb::IdempotencyKey { value: key }),
                    session: session_id(&session),
                    anchor: Some(pb_anchor(&anchor)),
                    text,
                    destination: Some(pb_destination(&destination)),
                })
                .await
                .map_err(|status| shared.error(&status, WHAT))?
                .into_inner();
            Ok(comment(&answer))
        })
    }

    fn edit(&self, id: &CommentId, text: &str, revision_seen: u64) -> Reply<Comment> {
        const WHAT: &str = "This comment";
        let shared = self.shared.clone();
        let (id, text) = (id.clone(), text.to_owned());
        Box::pin(async move {
            let answer = comments_service(&shared)
                .edit_comment(pb::EditCommentRequest {
                    idempotency_key: fresh_key(),
                    id: comment_id(&id),
                    text,
                    expected_revision: revision(revision_seen),
                })
                .await
                .map_err(|status| shared.error(&status, WHAT))?
                .into_inner();
            Ok(comment(&answer))
        })
    }

    fn drop_comment(&self, id: &CommentId) -> Reply<()> {
        let shared = self.shared.clone();
        let id = id.clone();
        Box::pin(async move {
            comments_service(&shared)
                .drop_comment(pb::DropCommentRequest {
                    idempotency_key: fresh_key(),
                    id: comment_id(&id),
                    // The app's drop names no revision; the daemon checks none then.
                    expected_revision: None,
                })
                .await
                .map_err(|status| shared.error(&status, "This comment"))?;
            Ok(())
        })
    }

    fn reattach(&self, id: &CommentId, anchor: Anchor, revision_seen: u64) -> Reply<Comment> {
        const WHAT: &str = "This comment";
        let shared = self.shared.clone();
        let id = id.clone();
        Box::pin(async move {
            let answer = comments_service(&shared)
                .reattach_comment(pb::ReattachCommentRequest {
                    idempotency_key: fresh_key(),
                    id: comment_id(&id),
                    anchor: Some(pb_anchor(&anchor)),
                    expected_revision: revision(revision_seen),
                })
                .await
                .map_err(|status| shared.error(&status, WHAT))?
                .into_inner();
            Ok(comment(&answer))
        })
    }

    fn send(&self, session: &SessionId, ids: &[CommentId]) -> Reply<Sent> {
        let shared = self.shared.clone();
        let session = session.clone();
        let ids: Vec<_> = ids.iter().filter_map(comment_id).collect();
        Box::pin(async move {
            let answer = comments_service(&shared)
                .send_comments(pb::SendCommentsRequest {
                    idempotency_key: fresh_key(),
                    session: session_id(&session),
                    ids,
                })
                .await
                .map_err(|status| shared.error(&status, "Sending the comments"))?
                .into_inner();
            Ok(Sent {
                comments: comments(&answer.comments),
                queued: answer.queued,
            })
        })
    }

    fn take_back(&self, session: &SessionId, ids: &[CommentId]) -> Reply<Vec<Comment>> {
        let shared = self.shared.clone();
        let session = session.clone();
        let ids: Vec<_> = ids.iter().filter_map(comment_id).collect();
        Box::pin(async move {
            let answer = comments_service(&shared)
                .take_back_comments(pb::TakeBackCommentsRequest {
                    idempotency_key: fresh_key(),
                    session: session_id(&session),
                    ids,
                })
                .await
                .map_err(|status| shared.error(&status, "Taking the comments back"))?
                .into_inner();
            Ok(comments(&answer.comments))
        })
    }

    fn resolve(&self, id: &CommentId, revision_seen: u64) -> Reply<Comment> {
        const WHAT: &str = "This comment";
        let shared = self.shared.clone();
        let id = id.clone();
        Box::pin(async move {
            let answer = comments_service(&shared)
                .resolve_comment(pb::ResolveCommentRequest {
                    idempotency_key: fresh_key(),
                    id: comment_id(&id),
                    expected_revision: revision(revision_seen),
                    reply: String::new(),
                })
                .await
                .map_err(|status| shared.error(&status, WHAT))?
                .into_inner();
            Ok(comment(&answer))
        })
    }

    fn make_note_a_draft(&self, id: &CommentId, pr: Option<(&str, u32)>) -> Reply<Comment> {
        const WHAT: &str = "This comment";
        let shared = self.shared.clone();
        let id = id.clone();
        let pr = pr.map(|(repo, number)| pb::PullRequestRef {
            repo: Some(pb_repo(repo)),
            number,
            note: String::new(),
        });
        Box::pin(async move {
            let answer = comments_service(&shared)
                .make_note_a_draft(pb::MakeNoteADraftRequest {
                    idempotency_key: fresh_key(),
                    id: comment_id(&id),
                    pr,
                })
                .await
                .map_err(|status| shared.error(&status, WHAT))?
                .into_inner();
            Ok(comment(&answer))
        })
    }

    fn resolve_addressed(
        &self,
        session: &SessionId,
        file: Option<(&str, &str)>,
    ) -> Reply<Vec<Comment>> {
        let shared = self.shared.clone();
        let session = session.clone();
        let file = file.map(|(repo, path)| pb::RepoPath {
            repo: pb_repo_named(repo),
            path: path.to_owned(),
        });
        Box::pin(async move {
            let answer = comments_service(&shared)
                .resolve_addressed_comments(pb::ResolveAddressedCommentsRequest {
                    idempotency_key: fresh_key(),
                    session: session_id(&session),
                    file,
                })
                .await
                .map_err(|status| shared.error(&status, "The addressed comments"))?
                .into_inner();
            Ok(comments(&answer.comments))
        })
    }

    fn reopen(&self, id: &CommentId, revision_seen: u64) -> Reply<Comment> {
        const WHAT: &str = "This comment";
        let shared = self.shared.clone();
        let id = id.clone();
        Box::pin(async move {
            let answer = comments_service(&shared)
                .reopen_comment(pb::ReopenCommentRequest {
                    idempotency_key: fresh_key(),
                    id: comment_id(&id),
                    expected_revision: revision(revision_seen),
                })
                .await
                .map_err(|status| shared.error(&status, WHAT))?
                .into_inner();
            Ok(comment(&answer))
        })
    }

    fn archive(&self, id: &CommentId) -> Reply<Comment> {
        const WHAT: &str = "This comment";
        let shared = self.shared.clone();
        let id = id.clone();
        Box::pin(async move {
            let answer = comments_service(&shared)
                .archive_comment(pb::ArchiveCommentRequest {
                    idempotency_key: fresh_key(),
                    id: comment_id(&id),
                })
                .await
                .map_err(|status| shared.error(&status, WHAT))?
                .into_inner();
            Ok(comment(&answer))
        })
    }

    fn unarchive(&self, id: &CommentId) -> Reply<Comment> {
        const WHAT: &str = "This comment";
        let shared = self.shared.clone();
        let id = id.clone();
        Box::pin(async move {
            let answer = comments_service(&shared)
                .unarchive_comment(pb::UnarchiveCommentRequest {
                    idempotency_key: fresh_key(),
                    id: comment_id(&id),
                })
                .await
                .map_err(|status| shared.error(&status, WHAT))?
                .into_inner();
            Ok(comment(&answer))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terminal() -> Option<pb::CommentDestination> {
        Some(pb::CommentDestination {
            destination: Some(pb::comment_destination::Destination::Terminal(
                pb::SessionTerminal {},
            )),
        })
    }

    fn agent(name: &str) -> Option<pb::Actor> {
        Some(pb::Actor {
            actor: Some(pb::actor::Actor::Agent(pb::AgentActor {
                agent: Some(pb::AgentId {
                    value: name.to_owned(),
                }),
                ..pb::AgentActor::default()
            })),
        })
    }

    fn daemon_comment(status: pb::CommentStatus) -> pb::Comment {
        pb::Comment {
            id: Some(pb::CommentId {
                value: "s1/c-1".to_owned(),
            }),
            session: Some(pb::SessionId {
                value: "s1".to_owned(),
            }),
            text: "Rename this".to_owned(),
            status: status as i32,
            destination: terminal(),
            revision: Some(pb::Revision { value: 3 }),
            ..pb::Comment::default()
        }
    }

    #[test]
    fn a_queued_comment_waits_and_may_be_unconfirmed() {
        let mut queued = daemon_comment(pb::CommentStatus::Queued);
        queued.delivery_unconfirmed = true;
        let mapped = comment(&queued);
        assert_eq!(mapped.status, Status::Waiting);
        assert!(mapped.unconfirmed);
        assert_eq!(mapped.revision, 3);
        assert_eq!(mapped.session, SessionId::new("s1"));
    }

    #[test]
    fn an_archived_stamp_marks_the_comment_archived() {
        let mut archived = daemon_comment(pb::CommentStatus::Draft);
        assert!(!comment(&archived).archived);
        archived.archived_at = Some(Default::default());
        let mapped = comment(&archived);
        assert!(mapped.archived);
        assert_eq!(mapped.status, Status::Draft);
    }

    #[test]
    fn a_note_keeps_its_destination_and_agent_author() {
        let mut note = daemon_comment(pb::CommentStatus::Draft);
        note.destination = Some(pb::CommentDestination {
            destination: Some(pb::comment_destination::Destination::Note(pb::UserNote {})),
        });
        note.author = agent("claude");
        let mapped = comment(&note);
        assert_eq!(mapped.destination, Destination::Note);
        assert_eq!(mapped.author, Author::Agent("claude".to_owned()));
        assert_eq!(mapped.status, Status::Draft);
        assert_eq!(Some(pb_destination(&mapped.destination)), note.destination);
    }

    #[test]
    fn a_pull_request_draft_keeps_its_repository_number_and_posted_link() {
        let destination = Some(pb::CommentDestination {
            destination: Some(pb::comment_destination::Destination::PullRequest(
                pb::PullRequestRef {
                    repo: Some(pb::RepoRef {
                        owner: "acme".to_owned(),
                        name: "api".to_owned(),
                    }),
                    number: 12,
                    note: String::new(),
                },
            )),
        });
        let mut posted = daemon_comment(pb::CommentStatus::Posted);
        posted.destination = destination.clone();
        posted.posted_url = "https://github.com/acme/api/pull/12#discussion_r1".to_owned();
        let mapped = comment(&posted);
        assert_eq!(
            mapped.destination,
            Destination::PullRequest { repo: "acme/api".to_owned(), number: 12 }
        );
        assert_eq!(mapped.status, Status::Posted);
        assert_eq!(
            mapped.posted_url.as_deref(),
            Some("https://github.com/acme/api/pull/12#discussion_r1")
        );
        assert_eq!(Some(pb_destination(&mapped.destination)), destination);

        let mut unposted = daemon_comment(pb::CommentStatus::Draft);
        unposted.destination = destination;
        let mapped = comment(&unposted);
        assert_eq!(mapped.status, Status::Draft);
        assert_eq!(mapped.posted_url, None);
    }

    #[test]
    fn a_failed_post_and_an_outdated_draft_come_from_the_daemon() {
        let mut draft = daemon_comment(pb::CommentStatus::Draft);
        draft.post_unconfirmed = true;
        draft.anchor = Some(pb::CommentAnchor {
            outdated: true,
            ..pb::CommentAnchor::default()
        });
        let mapped = comment(&draft);
        assert!(mapped.post_unconfirmed);
        assert!(mapped.anchor.outdated);

        let plain = comment(&daemon_comment(pb::CommentStatus::Draft));
        assert!(!plain.post_unconfirmed);
        assert!(!plain.anchor.outdated);
    }

    #[test]
    fn a_comment_without_a_destination_is_for_the_terminal() {
        let mut plain = daemon_comment(pb::CommentStatus::Draft);
        plain.destination = None;
        assert_eq!(comment(&plain).destination, Destination::Terminal);
        assert_eq!(comment(&plain).author, Author::User);
    }

    #[test]
    fn a_resolved_comment_names_who_resolved_it() {
        let mut resolved = daemon_comment(pb::CommentStatus::Resolved);
        resolved.resolved = Some(pb::Timestamped {
            at: None,
            by: agent("claude"),
        });
        let mapped = comment(&resolved);
        assert_eq!(mapped.resolved_by, Some(Author::Agent("claude".to_owned())));

        let mut by_user = daemon_comment(pb::CommentStatus::Resolved);
        by_user.resolved = Some(pb::Timestamped::default());
        assert_eq!(comment(&by_user).resolved_by, Some(Author::User));

        // Only a resolved comment says who resolved it.
        let mut draft = daemon_comment(pb::CommentStatus::Draft);
        draft.resolved = Some(pb::Timestamped::default());
        assert_eq!(comment(&draft).resolved_by, None);
    }

    #[test]
    fn an_addressed_comment_has_its_lines_after_the_change() {
        let mut addressed = daemon_comment(pb::CommentStatus::Addressed);
        addressed.addressed_text = "one\ntwo".to_owned();
        let mapped = comment(&addressed);
        assert_eq!(mapped.addressed, Some(vec!["one".to_owned(), "two".to_owned()]));

        addressed.addressed_text = String::new();
        assert_eq!(comment(&addressed).addressed, Some(Vec::new()));

        let sent = daemon_comment(pb::CommentStatus::Sent);
        assert_eq!(comment(&sent).addressed, None);
    }

    #[test]
    fn replies_keep_their_authors() {
        let mut with_replies = daemon_comment(pb::CommentStatus::Sent);
        with_replies.replies = vec![
            pb::CommentReply {
                id: "r-1".to_owned(),
                author: agent("claude"),
                text: "Done".to_owned(),
                at: None,
            },
            pb::CommentReply {
                id: "r-2".to_owned(),
                author: None,
                text: "Thanks".to_owned(),
                at: None,
            },
        ];
        let replies = comment(&with_replies).replies;
        assert_eq!(replies[0].author, Author::Agent("claude".to_owned()));
        assert_eq!(replies[1].author, Author::User);
        assert_eq!(replies[1].text, "Thanks");
    }

    #[test]
    fn an_anchor_survives_the_trip_to_the_daemon_and_back() {
        let original = Anchor {
            repo: "acme/api".to_owned(),
            path: "src/lib.rs".to_owned(),
            side: Side::Old,
            first: 4,
            last: 6,
            quoted: vec!["fn a() {".to_owned(), "".to_owned(), "}".to_owned()],
            base: Some("origin/main".to_owned()),
            detached: true,
            outdated: true,
        };
        assert_eq!(anchor(&pb_anchor(&original)), original);

        let outside_a_diff = Anchor {
            base: None,
            side: Side::New,
            detached: false,
            outdated: false,
            ..original
        };
        assert_eq!(anchor(&pb_anchor(&outside_a_diff)), outside_a_diff);
    }

    #[test]
    fn single_line_backward_compat_and_reversed_multi_line_anchors_normalize() {
        // Legacy/single-line wire anchor with `last == 0` maps to `first..=first`.
        let legacy_single = pb::CommentAnchor {
            file: Some(pb::RepoPath {
                repo: Some(pb::RepoRef {
                    owner: "acme".to_owned(),
                    name: "api".to_owned(),
                }),
                path: "src/lib.rs".to_owned(),
            }),
            lines: Some(pb::LineRange { first: 10, last: 0 }),
            side: pb::DiffSide::New as i32,
            ..pb::CommentAnchor::default()
        };
        let mapped = anchor(&legacy_single);
        assert_eq!((mapped.first, mapped.last), (10, 10));
        assert!(!mapped.is_multi_line());

        // Reversed range `18..=10` normalizes to `10..=18` on both decode and encode.
        let reversed_wire = pb::CommentAnchor {
            lines: Some(pb::LineRange { first: 18, last: 10 }),
            ..legacy_single
        };
        let mapped = anchor(&reversed_wire);
        assert_eq!((mapped.first, mapped.last), (10, 18));
        assert!(mapped.is_multi_line());

        let reversed_app = Anchor {
            first: 18,
            last: 10,
            ..mapped
        };
        let encoded = pb_anchor(&reversed_app);
        assert_eq!(
            encoded.lines,
            Some(pb::LineRange { first: 10, last: 18 })
        );
    }

    #[test]
    fn an_anchor_on_a_session_folder_file_names_no_repository() {
        let document = Anchor {
            repo: String::new(),
            path: "rfc-brin.md".to_owned(),
            side: Side::New,
            first: 7,
            last: 7,
            quoted: Vec::new(),
            base: None,
            detached: false,
            outdated: false,
        };
        let wire = pb_anchor(&document);
        let named = wire
            .file
            .as_ref()
            .and_then(|file| file.repo.as_ref())
            .is_some_and(|repo| !repo.name.is_empty());
        assert!(!named);
        assert_eq!(anchor(&wire), document);

        // The daemon may answer with an empty repository instead of none.
        let mut empty = wire;
        if let Some(file) = empty.file.as_mut() {
            file.repo = Some(pb::RepoRef::default());
        }
        assert_eq!(anchor(&empty), document);
    }

    #[test]
    fn open_lists_no_statuses_and_archived_lists_every_one() {
        let (statuses, archived) = which_request(Which::Open);
        assert!(statuses.is_empty());
        assert!(!archived);

        let (statuses, archived) = which_request(Which::WithResolved);
        assert!(statuses.contains(&(pb::CommentStatus::Resolved as i32)));
        assert!(statuses.contains(&(pb::CommentStatus::Posted as i32)));
        assert!(!archived);

        let (statuses, archived) = which_request(Which::Archived);
        assert!(statuses.contains(&(pb::CommentStatus::Resolved as i32)));
        assert!(statuses.contains(&(pb::CommentStatus::Draft as i32)));
        assert!(statuses.contains(&(pb::CommentStatus::Posted as i32)));
        assert!(archived);
    }

    #[test]
    fn feed_events_become_comment_events() {
        use pb::comment_feed_event::Event;
        let changed = pb::CommentFeedEvent {
            event: Some(Event::Changed(daemon_comment(pb::CommentStatus::Draft))),
        };
        assert!(matches!(event(changed), Some(CommentEvent::Changed(c)) if c.text == "Rename this"));

        let dropped = pb::CommentFeedEvent {
            event: Some(Event::Dropped(pb::CommentId {
                value: "s1/c-2".to_owned(),
            })),
        };
        assert_eq!(event(dropped), Some(CommentEvent::Dropped(CommentId::new("s1/c-2"))));

        let resync = pb::CommentFeedEvent {
            event: Some(Event::Resync(true)),
        };
        assert_eq!(event(resync), Some(CommentEvent::Resync));

        assert_eq!(event(pb::CommentFeedEvent { event: None }), None);

        // A pull request draft's change is heard like any other.
        let mut draft = daemon_comment(pb::CommentStatus::Draft);
        draft.destination = Some(pb::CommentDestination {
            destination: Some(pb::comment_destination::Destination::PullRequest(
                pb::PullRequestRef::default(),
            )),
        });
        assert!(matches!(
            event(pb::CommentFeedEvent {
                event: Some(Event::Changed(draft)),
            }),
            Some(CommentEvent::Changed(c))
                if matches!(c.destination, Destination::PullRequest { .. })
        ));
    }
}

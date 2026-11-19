//! The review comments parity cases against `FakeComments` and `LiveComments`
//! on a real `humatad2`, so the fake cannot drift from what the daemon does.

use std::sync::Arc;

use humata2_clients::comments::{
    Anchor, Author, Comment, CommentEvent, CommentEvents, CommentId, CommentsClient, Destination,
    FakeComments, Side, Status, Which,
};
use humata2_clients::model::SessionId;
use humata2_clients::{Daemon, DaemonError};
use humata2_clients_live_review::LiveComments;
use humata2_clients_live_test_support::cases::{self, World, within};
use humata2_clients_live_test_support::fake::{self, FakeWorld};
use humata2_clients_live_test_support::real::{self, RealWorld};

fn start_fake_comments() -> (FakeWorld, Arc<dyn Daemon>, Arc<dyn CommentsClient>) {
    let (world, daemon, _terminals) = fake::start();
    (world, daemon, Arc::new(FakeComments::new()))
}

async fn start_real_comments() -> (RealWorld, Arc<dyn Daemon>, Arc<dyn CommentsClient>) {
    let (world, live) = real::start_daemon().await;
    let comments: Arc<dyn CommentsClient> = Arc::new(LiveComments::new(&live));
    let daemon: Arc<dyn Daemon> = Arc::new(live);
    (world, daemon, comments)
}

async fn session_to_comment_on(
    world: &dyn World,
    daemon: &Arc<dyn Daemon>,
) -> (SessionId, String) {
    let (channel, repo, mac) = cases::channel_with_repo(world, daemon, "payments").await;
    let id = cases::start_session(world, daemon, &channel, &repo, &mac, "Fix the ledger").await;
    (id, repo)
}

fn anchor_in(repo: &str) -> Anchor {
    Anchor {
        repo: repo.to_owned(),
        path: "README.md".to_owned(),
        side: Side::New,
        first: 1,
        last: 1,
        quoted: vec![repo.to_owned()],
        base: None,
        detached: false,
        outdated: false,
    }
}

async fn draft_in(
    comments: &Arc<dyn CommentsClient>,
    session: &SessionId,
    repo: &str,
    key: &str,
) -> Comment {
    within(
        "create",
        comments.create(session, anchor_in(repo), "Rename this", key),
    )
    .await
    .expect("create a comment")
}

async fn comment_is_listed_as_a_draft(
    world: &dyn World,
    daemon: Arc<dyn Daemon>,
    comments: Arc<dyn CommentsClient>,
) {
    let (session, repo) = session_to_comment_on(world, &daemon).await;
    let made = draft_in(&comments, &session, &repo, "key-1").await;
    assert_eq!(made.status, Status::Draft);
    let listing = within("list", comments.list(&session, Which::Open))
        .await
        .expect("list");
    assert_eq!(listing.comments.len(), 1);
    assert_eq!(listing.comments[0].id, made.id);
    assert_eq!(listing.comments[0].text, "Rename this");
    assert_eq!(listing.comments[0].anchor.path, "README.md");
    assert_eq!(listing.draft_count, 1);
}

async fn a_comment_for_the_terminal_lists_with_the_users_shape(
    world: &dyn World,
    daemon: Arc<dyn Daemon>,
    comments: Arc<dyn CommentsClient>,
) {
    let (session, repo) = session_to_comment_on(world, &daemon).await;
    let made = draft_in(&comments, &session, &repo, "key-1").await;
    let listing = within("list", comments.list(&session, Which::Open))
        .await
        .expect("list");
    let listed = listing
        .comments
        .iter()
        .find(|comment| comment.id == made.id)
        .expect("the comment is listed");
    assert_eq!(listed.destination, Destination::Terminal);
    assert_eq!(listed.author, Author::User);
    assert_eq!(listed.posted_url, None);
    assert!(!listed.post_unconfirmed);
    assert!(!listed.anchor.outdated);
}

async fn a_draft_for_a_pull_request_the_session_does_not_review_is_refused(
    world: &dyn World,
    daemon: Arc<dyn Daemon>,
    comments: Arc<dyn CommentsClient>,
) {
    let (session, repo) = session_to_comment_on(world, &daemon).await;
    let refused = within(
        "create_for_pull_request",
        comments.create_for_pull_request(
            &session,
            anchor_in(&repo),
            "Rename this",
            &repo,
            12,
            "key-pr",
        ),
    )
    .await;
    assert!(
        matches!(refused, Err(DaemonError::Refused { .. })),
        "expected a refusal, got {refused:?}"
    );
    let listing = within("list", comments.list(&session, Which::Open))
        .await
        .expect("list");
    assert!(listing.comments.is_empty());
}

async fn a_comment_for_the_terminal_does_not_become_a_pull_request_draft(
    world: &dyn World,
    daemon: Arc<dyn Daemon>,
    comments: Arc<dyn CommentsClient>,
) {
    let (session, repo) = session_to_comment_on(world, &daemon).await;
    let made = draft_in(&comments, &session, &repo, "key-1").await;
    let refused = within("make_note_a_draft", comments.make_note_a_draft(&made.id, None)).await;
    assert!(
        matches!(refused, Err(DaemonError::Refused { .. })),
        "expected a refusal, got {refused:?}"
    );
    let listing = within("list", comments.list(&session, Which::Open))
        .await
        .expect("list");
    assert_eq!(listing.comments[0].destination, Destination::Terminal);
}

async fn edit_with_a_stale_revision_is_refused(
    world: &dyn World,
    daemon: Arc<dyn Daemon>,
    comments: Arc<dyn CommentsClient>,
) {
    let (session, repo) = session_to_comment_on(world, &daemon).await;
    let made = draft_in(&comments, &session, &repo, "key-1").await;
    let edited = within("edit", comments.edit(&made.id, "Rename that", made.revision))
        .await
        .expect("edit at the current revision");
    assert_eq!(edited.text, "Rename that");
    assert!(edited.revision > made.revision);
    let stale = within("edit", comments.edit(&made.id, "Too late", made.revision)).await;
    assert!(
        matches!(stale, Err(DaemonError::Conflict { .. })),
        "expected a conflict, got {stale:?}"
    );
}

async fn archived_comment_moves_to_the_archive(
    world: &dyn World,
    daemon: Arc<dyn Daemon>,
    comments: Arc<dyn CommentsClient>,
) {
    let (session, repo) = session_to_comment_on(world, &daemon).await;
    let made = draft_in(&comments, &session, &repo, "key-1").await;
    let archived = within("archive", comments.archive(&made.id))
        .await
        .expect("archive");
    assert!(archived.archived);
    let open = within("list", comments.list(&session, Which::Open))
        .await
        .expect("list open");
    assert!(open.comments.is_empty());
    assert_eq!(open.draft_count, 0);
    let kept = within("list", comments.list(&session, Which::Archived))
        .await
        .expect("list archived");
    assert_eq!(kept.comments.len(), 1);
    assert_eq!(kept.comments[0].id, made.id);
}

/// The next `Changed` for `id`; a `Resync` on the way is ignored (the
/// daemon opens its feed with one, the fake's feed has none).
async fn changed_comment(events: &mut CommentEvents, id: &CommentId) -> Comment {
    within("a comment event", async {
        loop {
            match events.recv().await {
                Some(CommentEvent::Changed(comment)) if &comment.id == id => return comment,
                Some(_) => {}
                None => panic!("the comment feed ended"),
            }
        }
    })
    .await
}

async fn watch_follows_a_new_comment(
    world: &dyn World,
    daemon: Arc<dyn Daemon>,
    comments: Arc<dyn CommentsClient>,
) {
    let (session, repo) = session_to_comment_on(world, &daemon).await;
    let mut events = within("watch", comments.watch(&session))
        .await
        .expect("watch");
    let made = draft_in(&comments, &session, &repo, "key-1").await;
    let heard = changed_comment(&mut events, &made.id).await;
    assert_eq!(heard.text, "Rename this");
    assert_eq!(heard.status, Status::Draft);
}

macro_rules! parity_comments {
    ($($case:ident => $fake:ident, $real:ident;)*) => {$(
        #[tokio::test(flavor = "multi_thread")]
        async fn $fake() {
            let (world, daemon, comments) = start_fake_comments();
            $case(&world, daemon, comments).await;
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn $real() {
            let (world, daemon, comments) = start_real_comments().await;
            $case(&world, daemon, comments).await;
        }
    )*};
}

// Sending needs a session whose agent terminal is up and typed into; the
// support has no wait for that yet, so it is not covered here.
parity_comments! {
    comment_is_listed_as_a_draft => comment_is_listed_as_a_draft_on_fake, comment_is_listed_as_a_draft_on_real;
    a_comment_for_the_terminal_lists_with_the_users_shape => a_comment_for_the_terminal_lists_with_the_users_shape_on_fake, a_comment_for_the_terminal_lists_with_the_users_shape_on_real;
    a_draft_for_a_pull_request_the_session_does_not_review_is_refused => a_draft_for_a_pull_request_the_session_does_not_review_is_refused_on_fake, a_draft_for_a_pull_request_the_session_does_not_review_is_refused_on_real;
    a_comment_for_the_terminal_does_not_become_a_pull_request_draft => a_comment_for_the_terminal_does_not_become_a_pull_request_draft_on_fake, a_comment_for_the_terminal_does_not_become_a_pull_request_draft_on_real;
    edit_with_a_stale_revision_is_refused => edit_with_a_stale_revision_is_refused_on_fake, edit_with_a_stale_revision_is_refused_on_real;
    archived_comment_moves_to_the_archive => archived_comment_moves_to_the_archive_on_fake, archived_comment_moves_to_the_archive_on_real;
    watch_follows_a_new_comment => watch_follows_a_new_comment_on_fake, watch_follows_a_new_comment_on_real;
}

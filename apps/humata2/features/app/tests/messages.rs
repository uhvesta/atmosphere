//! SM-9/10 and SE-23: review drafts transfer to the persistent session queue.
use humata2_app::{App, AppAction as A, AppState, Route, saved};
use humata2_clients::{
    Clients, Fakes, Operation, fixture,
    model::{Comment, SessionId},
};
use humata2_review::{Delegate as R, ReviewAction};
use humata2_session::{Delegate as S, QueuedMessage, SessionAction, SessionView};
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn setup() -> (TestStore<App>, Clients, Fakes, Comment) {
    let mut workspace = fixture::design();
    let duplicate = workspace
        .diffs
        .iter()
        .find(|diff| diff.repo == "api")
        .unwrap()
        .files
        .iter()
        .find(|file| file.path == "src/client.rs")
        .unwrap()
        .clone();
    workspace
        .diffs
        .iter_mut()
        .find(|diff| diff.repo == "ledger")
        .unwrap()
        .files
        .push(duplicate);
    let workspace = Arc::new(workspace);
    let (clients, fakes) = Clients::fake((*workspace).clone(), Arc::new(TestClock::new()));
    let mut state = AppState::launch(&clients);
    let id = SessionId::new("ledger");
    state.workspace = workspace.clone();
    state.route = Route::Session(id.clone());
    state.session.workspace = workspace.clone();
    state.session.open = Some(id.clone());
    state
        .session
        .views
        .insert(id.clone(), SessionView::default());
    state.review.workspace = workspace;
    state.review.session = Some(id.clone());
    let comment = Comment {
        path: "src/client.rs".into(),
        line: 43,
        body: "Keep the two writes atomic.".into(),
        sent: false,
        context: Some(humata2_clients::model::CommentContext {
            repo: "api".into(),
            base: "main".into(),
            quoted: Some("        tx.insert_legacy(&entry).await?;".into()),
        }),
    };
    state.review.saved.queued.insert(id, vec![comment.clone()]);
    (
        TestStore::new(state, App::new(clients.clone())).non_exhaustive(),
        clients,
        fakes,
        comment,
    )
}

fn queue(store: &mut TestStore<App>) {
    store.send(A::Review(ReviewAction::Send), |s| s.review.sending = true);
    store.receive_matching(
        "batch retained",
        |a| matches!(a, A::Review(ReviewAction::Delegate(R::QueueComments(_, _)))),
        |s| s.review.sending = false,
    );
}

fn delivery(store: &mut TestStore<App>, ok: bool) {
    store.receive_matching(
        "comment delivery result",
        move |a| matches!(a, A::Session(SessionAction::Sent { result, .. }) if result.is_ok() == ok),
        |s| s.session.sending = false,
    );
}

fn write_draft(store: &mut TestStore<App>, line: u32, body: &str) {
    use humata2_review::Anchor;
    store.send(
        A::Review(ReviewAction::StartComment(Anchor {
            path: "src/client.rs".into(),
            line,
        })),
        |_| {},
    );
    store.send(A::Review(ReviewAction::CommentText(body.into())), |_| {});
}

/// Bodies of comments delivered to ledger during the test: the fixture's
/// own already-sent comments are not deliveries.
fn delivered_bodies(fakes: &Fakes) -> Vec<String> {
    let ledger = |workspace: &humata2_clients::Workspace| -> Vec<String> {
        workspace
            .diffs
            .iter()
            .filter(|diff| diff.session == SessionId::new("ledger"))
            .flat_map(|diff| &diff.comments)
            .map(|c| c.body.clone())
            .collect()
    };
    let before = ledger(&fixture::design());
    ledger(&fakes.daemon.workspace())
        .into_iter()
        .filter(|body| !before.contains(body))
        .collect()
}

#[test]
fn a_failed_delivery_keeps_the_batch_in_the_session_outbox_across_a_restart() {
    let (mut store, clients, fakes, comment) = setup();
    fakes
        .daemon
        .fail_next(Operation::SendComments, "link unavailable");
    queue(&mut store);
    delivery(&mut store, false);
    let id = SessionId::new("ledger");
    assert!(store.state().review.queued().is_empty());
    assert_eq!(
        store.state().session.views[&id].queue,
        [QueuedMessage::Comments {
            comments: vec![comment.clone()]
        }]
    );
    saved::Saver::new(clients.storage.clone())
        .save(1, &store.state().saved())
        .unwrap();
    let restored = AppState::launch(&clients);
    assert_eq!(
        restored.session.views[&id].queue,
        store.state().session.views[&id].queue
    );
    assert!(restored.review.saved.queued[&id].is_empty());
    store.finish();
}

#[test]
fn removing_a_failed_batch_returns_its_comments_to_drafts() {
    let (mut store, _, fakes, comment) = setup();
    fakes
        .daemon
        .fail_next(Operation::SendComments, "link unavailable");
    queue(&mut store);
    delivery(&mut store, false);
    assert_eq!(store.state().session.view().unwrap().queue.len(), 1);
    assert!(store.state().review.queued().is_empty());
    store.send(A::Session(SessionAction::RemoveQueued(0)), |_| {});
    store.receive_matching(
        "comments returned",
        |a| {
            matches!(
                a,
                A::Session(SessionAction::Delegate(S::CommentsUnqueued(_, _)))
            )
        },
        |_| {},
    );
    assert_eq!(store.state().review.queued(), [comment]);
    assert!(store.state().session.view().unwrap().queue.is_empty());
    // Nothing failed is left, so sending again goes at once.
    queue(&mut store);
    delivery(&mut store, true);
    assert!(store.state().session.view().unwrap().queue.is_empty());
    assert!(store.state().review.queued().is_empty());
    store.finish();
}

#[test]
fn explicit_send_delivers_the_batch_during_a_turn_once_and_removes_only_it() {
    let (mut store, _, fakes, comment) = setup();
    let mut workspace = (*store.state().session.workspace).clone();
    workspace
        .sessions
        .iter_mut()
        .find(|session| session.id == SessionId::new("ledger"))
        .unwrap()
        .turn = humata2_clients::model::Turn::Running;
    store.send(A::Session(SessionAction::Show(Arc::new(workspace))), |_| {});
    queue(&mut store);
    delivery(&mut store, true);
    assert!(store.state().session.view().unwrap().queue.is_empty());
    assert!(store.state().review.queued().is_empty());
    let workspace = fakes.daemon.workspace();
    let delivered: Vec<_> = workspace
        .diffs
        .iter()
        .filter(|diff| diff.session == SessionId::new("ledger"))
        .flat_map(|diff| &diff.comments)
        .filter(|c| c.body == comment.body)
        .collect();
    assert_eq!(delivered.len(), 1);
    assert!(delivered[0].sent);
    assert!(
        workspace
            .session(&SessionId::new("ledger"))
            .unwrap()
            .transcript
            .iter()
            .any(|line| line.text.contains(&comment.body) && line.text.contains(&comment.path))
    );
    store.finish();
}

#[test]
fn send_draft_delivers_only_that_draft_and_keeps_the_others() {
    let (mut store, _, fakes, first) = setup();
    write_draft(&mut store, 44, "A second, separate note.");
    store.send(A::Review(ReviewAction::SaveComment), |_| {});
    assert_eq!(store.state().review.queued().len(), 2);
    store.send(A::Review(ReviewAction::SendDraft(0)), |s| {
        s.review.sending = true
    });
    store.receive_matching(
        "one draft handed over",
        |a| matches!(a, A::Review(ReviewAction::Delegate(R::QueueComments(_, _)))),
        |_| {},
    );
    delivery(&mut store, true);
    assert_eq!(store.state().review.queued().len(), 1);
    assert_eq!(store.state().review.queued()[0].line, 44);
    assert_eq!(delivered_bodies(&fakes), [first.body]);
    assert!(store.state().session.view().unwrap().queue.is_empty());
    store.send(A::Review(ReviewAction::SendDraft(5)), |_| {});
    store.finish();
}

#[test]
fn send_comment_delivers_the_comment_being_written_and_leaves_drafts_alone() {
    let (mut store, _, fakes, earlier) = setup();
    write_draft(&mut store, 44, "Send this one now.");
    store.send(A::Review(ReviewAction::SendComment), |s| {
        s.review.sending = true
    });
    store.receive_matching(
        "comment handed over",
        |a| matches!(a, A::Review(ReviewAction::Delegate(R::QueueComments(_, _)))),
        |_| {},
    );
    delivery(&mut store, true);
    assert_eq!(store.state().review.queued(), [earlier]);
    assert!(store.state().review.draft().is_none());
    assert_eq!(delivered_bodies(&fakes), ["Send this one now."]);
    store.finish();
}

#[test]
fn send_comment_with_an_empty_body_does_nothing() {
    let (mut store, _, fakes, earlier) = setup();
    write_draft(&mut store, 44, "   ");
    store.send(A::Review(ReviewAction::SendComment), |_| {});
    assert_eq!(store.state().review.queued(), [earlier]);
    assert!(!store.state().review.sending);
    assert!(delivered_bodies(&fakes).is_empty());
    store.finish();
}

#[test]
fn the_batch_send_still_sends_every_draft_together() {
    let (mut store, _, fakes, first) = setup();
    write_draft(&mut store, 44, "Second note.");
    store.send(A::Review(ReviewAction::SaveComment), |_| {});
    queue(&mut store);
    delivery(&mut store, true);
    assert!(store.state().review.queued().is_empty());
    // Both drafts went in the one delivery; the fake files each under its
    // own repo's diff, so compare without the cross-repo order.
    let mut delivered = delivered_bodies(&fakes);
    delivered.sort();
    let mut expected = vec![first.body, "Second note.".to_owned()];
    expected.sort();
    assert_eq!(delivered, expected);
    store.finish();
}

#[test]
fn string_queues_saved_before_comment_messages_still_load() {
    let (clients, _) = Clients::fake(fixture::design(), Arc::new(TestClock::new()));
    clients.storage.save(saved::KEY, br#"{"sessions":{"ledger":{"tab":0,"panel":null,"widened":false,"draft":"","queue":["older waiting text"]}}}"#).unwrap();
    let state = AppState::launch(&clients);
    assert!(state.restore_error.is_none());
    assert_eq!(
        state.session.views[&SessionId::new("ledger")].queue,
        [QueuedMessage::Text("older waiting text".into())]
    );
}

#[test]
fn a_comment_keeps_the_repository_base_and_code_that_were_shown_when_written() {
    use humata2_review::Anchor;
    let (mut store, _, _, _) = setup();
    store.send(A::Review(ReviewAction::RemoveComment(0)), |_| {});
    store.send(
        A::Review(ReviewAction::SelectFile {
            repo: "api".into(),
            path: "src/client.rs".into(),
        }),
        |_| {},
    );
    store.send(
        A::Review(ReviewAction::StartComment(Anchor {
            path: "src/client.rs".into(),
            line: 43,
        })),
        |_| {},
    );
    store.send(
        A::Review(ReviewAction::CommentText("Keep the transaction.".into())),
        |_| {},
    );
    store.send(A::Review(ReviewAction::SaveComment), |_| {});
    let comment = store.state().review.queued()[0].clone();
    let context = comment.context.as_ref().unwrap();
    assert_eq!(context.repo, "api");
    assert_eq!(context.base, "main");
    assert_eq!(
        context.quoted.as_deref(),
        Some("        tx.insert_legacy(&entry).await?;")
    );
    let mut workspace = (*store.state().review.workspace).clone();
    for diff in &mut workspace.diffs {
        diff.base = "a different base".into();
    }
    store.send(A::Review(ReviewAction::Show(Arc::new(workspace))), |_| {});
    assert_eq!(store.state().review.queued(), [comment]);
    store.finish();
}

#[test]
fn retaining_one_batch_does_not_clear_a_newer_draft() {
    use humata2_review::Anchor;
    let (mut store, _, _, original) = setup();
    store.send(A::Review(ReviewAction::Send), |s| s.review.sending = true);
    store.send(
        A::Review(ReviewAction::StartComment(Anchor {
            path: "src/client.rs".into(),
            line: 44,
        })),
        |_| {},
    );
    store.send(
        A::Review(ReviewAction::CommentText(
            "A newer draft while the first batch is pending.".into(),
        )),
        |_| {},
    );
    store.send(A::Review(ReviewAction::SaveComment), |_| {});
    store.receive_matching(
        "first batch retained",
        |a| matches!(a, A::Review(ReviewAction::Delegate(R::QueueComments(_, _)))),
        |_| {},
    );
    assert_eq!(
        store.state().session.view().unwrap().queue,
        [QueuedMessage::Comments {
            comments: vec![original]
        }]
    );
    assert_eq!(store.state().review.queued().len(), 1);
    assert_eq!(store.state().review.queued()[0].line, 44);
    delivery(&mut store, true);
    store.finish();
}

#[test]
fn a_missing_session_keeps_the_drafts_when_the_parent_cannot_queue_them() {
    let (mut store, _, _, comment) = setup();
    store.send(A::Session(SessionAction::Show(Arc::default())), |_| {});
    store.send(A::Review(ReviewAction::Send), |s| s.review.sending = true);
    store.receive_matching(
        "queue rejected",
        |a| matches!(a, A::Review(ReviewAction::Delegate(R::QueueComments(_, _)))),
        |s| s.review.sending = false,
    );
    assert_eq!(store.state().review.queued(), [comment]);
    assert!(store.state().review.error.is_some());
    assert!(store.state().session.view().unwrap().queue.is_empty());
    store.finish();
}

#[test]
fn a_batch_queued_after_navigation_still_belongs_to_the_session_that_authored_it() {
    let (mut store, _, fakes, comment) = setup();
    store.send(A::Review(ReviewAction::Send), |s| s.review.sending = true);
    store.send(A::Navigate(Route::Session(SessionId::new("brin"))), |_| {});
    store.receive_matching(
        "original session batch retained",
        |a| matches!(a, A::Review(ReviewAction::Delegate(R::QueueComments(_, _)))),
        |_| {},
    );
    assert_eq!(store.state().review.session, Some(SessionId::new("brin")));
    assert!(store.state().session.view().unwrap().queue.is_empty());
    assert_eq!(
        store.state().session.views[&SessionId::new("ledger")].queue,
        [QueuedMessage::Comments {
            comments: vec![comment.clone()]
        }]
    );
    assert!(store.state().review.saved.queued[&SessionId::new("ledger")].is_empty());
    delivery(&mut store, true);
    assert!(
        store.state().session.views[&SessionId::new("ledger")]
            .queue
            .is_empty()
    );
    assert_eq!(delivered_bodies(&fakes), [comment.body]);
    store.finish();
}

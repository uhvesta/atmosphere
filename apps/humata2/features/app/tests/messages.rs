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

#[test]
fn sending_during_a_turn_queues_metadata_and_a_restart_keeps_it() {
    let (mut store, clients, fakes, comment) = setup();
    queue(&mut store);
    let id = SessionId::new("ledger");
    assert!(store.state().review.queued().is_empty());
    assert_eq!(
        store.state().session.views[&id].queue,
        [QueuedMessage::Comments {
            comments: vec![comment.clone()]
        }]
    );
    assert!(
        !fakes
            .daemon
            .workspace()
            .session(&id)
            .unwrap()
            .transcript
            .iter()
            .any(|line| line.text.contains(&comment.body))
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
    queue(&mut store);
    fakes
        .daemon
        .fail_next(Operation::SendComments, "link unavailable");
    store.send(A::Session(SessionAction::SendQueued(0)), |s| {
        s.session.sending = true
    });
    store.receive_matching(
        "failed comment delivery",
        |a| matches!(a, A::Session(SessionAction::Sent { result: Err(_), .. })),
        |s| s.session.sending = false,
    );
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
    store.finish();
}

#[test]
fn explicit_send_delivers_the_comment_batch_once_and_removes_only_it() {
    let (mut store, _, fakes, comment) = setup();
    queue(&mut store);
    store.send(A::Session(SessionAction::SendQueued(0)), |s| {
        s.session.sending = true
    });
    store.receive_matching(
        "comments delivered",
        |a| matches!(a, A::Session(SessionAction::Sent { result: Ok(()), .. })),
        |s| s.session.sending = false,
    );
    assert!(store.state().session.view().unwrap().queue.is_empty());
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
    store.finish();
}

#[test]
fn a_hook_turn_end_delivers_the_waiting_review_batch_without_send_now() {
    let (mut store, _, fakes, comment) = setup();
    queue(&mut store);
    let mut workspace = (*store.state().session.workspace).clone();
    workspace
        .sessions
        .iter_mut()
        .find(|session| session.id == SessionId::new("ledger"))
        .unwrap()
        .turn = humata2_clients::model::Turn::Ended;
    store.send(A::Session(SessionAction::Show(Arc::new(workspace))), |s| {
        s.session.sending = true
    });
    store.receive_matching(
        "hook-backed comment delivery",
        |a| matches!(a, A::Session(SessionAction::Sent { result: Ok(()), .. })),
        |s| s.session.sending = false,
    );
    assert!(store.state().session.view().unwrap().queue.is_empty());
    assert_eq!(
        fakes
            .daemon
            .workspace()
            .diffs
            .iter()
            .flat_map(|diff| &diff.comments)
            .filter(|c| c.body == comment.body)
            .count(),
        1
    );
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
    let (mut store, _, _, comment) = setup();
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
            comments: vec![comment]
        }]
    );
    assert!(store.state().review.saved.queued[&SessionId::new("ledger")].is_empty());
    store.finish();
}

//! Review comments through the app: written in a session they reach the
//! daemon's comments client, archiving offers Undo, and comments older
//! builds saved are handed over once.
use humata2_app::{App, AppAction as A, AppState, Route, ToastAction, saved};
use humata2_clients::comments::{Anchor, FakeComments, Side};
use humata2_clients::model::{Comment, CommentContext, SessionId};
use humata2_clients::{Clients, fixture};
use humata2_review::ReviewAction;
use humata2_review::comments::CommentsAction;
use humata2_session::{QueuedMessage, SessionView};
use std::sync::Arc;
use tca::{TestClock, TestStore};

struct Rig {
    store: TestStore<App>,
    comments: FakeComments,
    id: SessionId,
}

fn rig(prepare: impl FnOnce(&mut AppState)) -> Rig {
    let workspace = Arc::new(fixture::design());
    let (clients, fakes) = Clients::fake((*workspace).clone(), Arc::new(TestClock::new()));
    let id = SessionId::new("ledger");
    fakes.comments.set_repos(&id, &["api", "ledger"]);
    let mut state = AppState::launch(&clients);
    state.workspace = workspace.clone();
    state.route = Route::Session(id.clone());
    state.session.workspace = workspace.clone();
    state.session.open = Some(id.clone());
    state.session.views.insert(id.clone(), SessionView::default());
    state.review.workspace = workspace;
    state.review.session = Some(id.clone());
    prepare(&mut state);
    Rig {
        store: TestStore::new(state, App::new(clients)).non_exhaustive(),
        comments: fakes.comments,
        id,
    }
}

impl Rig {
    fn act(&mut self, action: A) {
        self.store.send(action, |_| {});
        for _ in 0..8 {
            self.store.skip_received();
        }
    }
}

fn anchor(line: u32) -> Anchor {
    Anchor {
        repo: "api".into(),
        path: "src/client.rs".into(),
        side: Side::New,
        first: line,
        last: line,
        quoted: vec!["        tx.insert_legacy(&entry).await?;".into()],
        base: None,
        detached: false,
    }
}

fn legacy(body: &str) -> Comment {
    Comment {
        path: "src/client.rs".into(),
        line: 43,
        body: body.into(),
        sent: false,
        context: Some(CommentContext {
            repo: "api".into(),
            base: "main".into(),
            quoted: None,
        }),
    }
}

#[test]
fn a_comment_written_in_a_session_reaches_the_comments_client() {
    let mut rig = rig(|_| {});
    let id = rig.id.clone();
    rig.act(A::Comments(CommentsAction::Open(id.clone())));
    rig.act(A::Comments(CommentsAction::OpenBox(id.clone(), anchor(43))));
    rig.act(A::Comments(CommentsAction::Typed(
        id.clone(),
        "Keep the two writes atomic.".into(),
    )));
    rig.act(A::Comments(CommentsAction::Comment(id.clone())));

    let held = rig.comments.comments(&id);
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].text, "Keep the two writes atomic.");
    assert_eq!(rig.comments.typed(&id).len(), 1);
    assert_eq!(rig.store.state().comments.sessions[&id].comments.len(), 1);
}

#[test]
fn archiving_a_comment_offers_undo_that_brings_it_back() {
    let mut rig = rig(|_| {});
    let id = rig.id.clone();
    rig.act(A::Comments(CommentsAction::Open(id.clone())));
    rig.act(A::Comments(CommentsAction::OpenBox(id.clone(), anchor(43))));
    rig.act(A::Comments(CommentsAction::Typed(id.clone(), "Rename this.".into())));
    rig.act(A::Comments(CommentsAction::Close(id.clone())));
    let comment = rig.comments.comments(&id)[0].id.clone();

    rig.act(A::Comments(CommentsAction::Archive(id.clone(), comment.clone())));
    assert!(rig.comments.comments(&id)[0].archived);
    let toast = rig
        .store
        .state()
        .toasts
        .iter()
        .find(|toast| matches!(toast.action, Some(ToastAction::Comments(_))))
        .map(|toast| toast.id)
        .expect("archiving offers Undo");

    rig.act(A::UndoToast(toast));
    assert!(!rig.comments.comments(&id)[0].archived);
}

#[test]
fn comments_saved_by_older_builds_are_handed_over_once() {
    let mut rig = rig(|state| {
        state.review.saved.legacy_queued.insert(
            SessionId::new("ledger"),
            vec![legacy("Keep the two writes atomic.")],
        );
    });
    let id = rig.id.clone();
    let saved = rig.store.state().review.saved.clone();

    rig.act(A::Review(ReviewAction::Restore(saved)));
    let held = rig.comments.comments(&id);
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].text, "Keep the two writes atomic.");
    assert!(rig.store.state().review.saved.legacy_queued.is_empty());

    let saved = rig.store.state().review.saved.clone();
    rig.act(A::Review(ReviewAction::Restore(saved)));
    assert_eq!(rig.comments.comments(&id).len(), 1);
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

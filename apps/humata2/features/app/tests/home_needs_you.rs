//! Home's "Needs you" rows: a session that needs the person, or that has a
//! pending Inbox request, and a pending request with no session of its own.
use humata2_app::{NeedsRow, needs_you};
use humata2_clients::model::{Request, Session, Status};
use humata2_clients::{Workspace, fixture};

fn pending_request(session: &Session) -> Request {
    let mut request = fixture::standard()
        .requests
        .into_iter()
        .next()
        .expect("the standard fixture has a request");
    request.session = session.id.clone();
    request.channel = session.channel.clone();
    request.answer = None;
    request
}

#[test]
fn a_pending_request_on_an_idle_session_puts_that_session_in_needs_you() {
    let mut workspace = fixture::design();
    let session = workspace
        .sessions
        .iter()
        .find(|session| session.status == Status::Idle && !session.archived)
        .expect("the design fixture has an idle session")
        .clone();
    workspace.sessions = vec![session.clone()];
    let request = pending_request(&session);
    workspace.requests = vec![request.clone()];

    let summary = needs_you(&workspace);

    assert_eq!(summary.len(), 1);
    assert!(matches!(
        summary.rows.as_slice(),
        [NeedsRow::Session { session: listed, request: Some(asked) }]
            if listed.id == session.id && asked.id == request.id
    ));
}

#[test]
fn a_pending_request_with_no_session_gets_its_own_row() {
    // The request's session is not in the workspace, so the request stands alone.
    let mut workspace = fixture::empty();
    let session = fixture::design().sessions[0].clone();
    let request = pending_request(&session);
    workspace.requests = vec![request.clone()];

    let summary = needs_you(&workspace);

    assert_eq!(summary.len(), 1);
    assert!(matches!(
        summary.rows.as_slice(),
        [NeedsRow::Request(asked)] if asked.id == request.id
    ));
}

#[test]
fn nothing_pending_and_nothing_needing_you_is_empty() {
    let workspace: Workspace = fixture::empty();
    assert!(needs_you(&workspace).is_empty());
}

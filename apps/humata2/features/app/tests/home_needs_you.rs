//! Home's "Needs you" rows (HM-1, D158.7): a session whose agent waits on its
//! own request, that exited or that is offline, and a pending request with no
//! session on Home. A request on a working or idle session leaves it where it is.
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

/// A workspace with one session in `status` and one pending request on it.
fn one_session(status: Status) -> (Workspace, Session, Request) {
    let mut workspace = fixture::design();
    let mut session = workspace
        .sessions
        .iter()
        .find(|session| !session.archived)
        .expect("the design fixture has a session")
        .clone();
    session.status = status;
    workspace.sessions = vec![session.clone()];
    let request = pending_request(&session);
    workspace.requests = vec![request.clone()];
    (workspace, session, request)
}

#[test]
fn a_session_waiting_on_its_own_request_needs_you_with_that_request() {
    let (workspace, session, request) = one_session(Status::NeedsYou);

    let summary = needs_you(&workspace);

    assert!(matches!(
        summary.rows.as_slice(),
        [NeedsRow::Session { session: listed, request: Some(asked) }]
            if listed.id == session.id && asked.id == request.id
    ));
}

#[test]
fn a_request_on_a_working_or_idle_session_leaves_needs_you_empty() {
    for status in [Status::Working, Status::Idle] {
        let (workspace, _, _) = one_session(status);
        assert!(needs_you(&workspace).is_empty(), "{status:?}");
    }
}

#[test]
fn an_exited_or_offline_session_needs_you_without_a_request_to_review() {
    for status in [Status::Exited { code: 1 }, Status::Offline] {
        let (workspace, session, _) = one_session(status);

        let summary = needs_you(&workspace);

        assert!(
            matches!(
                summary.rows.as_slice(),
                [NeedsRow::Session { session: listed, request: None }] if listed.id == session.id
            ),
            "{status:?}"
        );
    }
}

#[test]
fn a_pending_request_with_no_session_gets_its_own_row() {
    // The request's session is not in the workspace, so the request stands alone.
    let mut workspace = fixture::empty();
    let session = fixture::design().sessions[0].clone();
    let request = pending_request(&session);
    workspace.requests = vec![request.clone()];

    let summary = needs_you(&workspace);

    assert!(matches!(
        summary.rows.as_slice(),
        [NeedsRow::Request(asked)] if asked.id == request.id
    ));
}

#[test]
fn the_design_workspace_lists_the_sessions_frame_25_draws() {
    let workspace = fixture::design();

    let titles: Vec<&str> = needs_you(&workspace)
        .rows
        .iter()
        .map(|row| match row {
            NeedsRow::Session { session, .. } => session.title.as_str(),
            NeedsRow::Request(request) => request.title.as_str(),
        })
        .collect();

    assert_eq!(
        titles,
        ["Ledger schema migration", "Try BRIN index", "Upgrade Postgres 17"]
    );
}

#[test]
fn nothing_pending_and_nothing_needing_you_is_empty() {
    let workspace: Workspace = fixture::empty();
    assert!(needs_you(&workspace).is_empty());
}

#[test]
fn a_session_blocked_only_on_a_to_do_ask_stays_off_home_needs_you() {
    let (mut workspace, _session, mut request) = one_session(Status::NeedsYou);
    request.to_do = Some(humata2_clients::model::ToDoAsk::NewItem);
    workspace.requests = vec![request];

    assert!(needs_you(&workspace).is_empty());
}

#[test]
fn a_session_with_both_a_to_do_ask_and_an_inbox_request_still_needs_you() {
    let (mut workspace, session, inbox_req) = one_session(Status::NeedsYou);
    let mut todo_req = inbox_req.clone();
    todo_req.id = humata2_clients::model::RequestId::new("todo-ask-1");
    todo_req.to_do = Some(humata2_clients::model::ToDoAsk::NewItem);
    workspace.requests = vec![todo_req, inbox_req.clone()];

    let summary = needs_you(&workspace);
    assert!(matches!(
        summary.rows.as_slice(),
        [NeedsRow::Session { session: listed, request: Some(asked) }]
            if listed.id == session.id && asked.id == inbox_req.id
    ));
}

//! CD-2: the channel Desk sessions looking after a session, as its Info panel
//! lists them, read from the workspace.
use std::sync::Arc;

use humata2_clients::model::SessionId;
use humata2_clients::{Workspace, fixture};
use humata2_desk::DeskState;

fn state(change: impl FnOnce(&mut Workspace)) -> DeskState {
    let mut workspace = fixture::design();
    fixture::channel_desk(&mut workspace);
    change(&mut workspace);
    DeskState {
        workspace: Arc::new(workspace),
        ..DeskState::default()
    }
}

/// Another channel Desk session looking after `target`, last used at `order`.
fn add(workspace: &mut Workspace, id: &str, target: Option<&str>, order: u64) {
    let (Some(session), Some(record)) = (
        workspace
            .sessions
            .iter()
            .find(|found| found.id.as_str() == "cdesk-backfill")
            .cloned(),
        workspace
            .desk
            .channel_sessions
            .get(&SessionId::new("cdesk-backfill"))
            .cloned(),
    ) else {
        panic!("the fixture has a Desk session looking after ledger")
    };
    let mut session = session;
    session.id = SessionId::new(id);
    session.title = id.to_owned();
    let mut record = record;
    record.desk.session = SessionId::new(id);
    record.desk.last_used.order = order;
    record.target = target.map(SessionId::new);
    workspace.sessions.push(session);
    workspace.desk.channel_sessions.insert(SessionId::new(id), record);
}

fn ids(state: &DeskState, target: &str) -> Vec<String> {
    state
        .targeting_rows(&SessionId::new(target))
        .iter()
        .map(|row| row.session.id.to_string())
        .collect()
}

#[test]
fn only_the_desk_sessions_that_target_this_session_are_listed() {
    let state = state(|workspace| add(workspace, "cdesk-other", Some("api"), 9));
    assert_eq!(ids(&state, "ledger"), ["cdesk-backfill"]);
    assert_eq!(ids(&state, "api"), ["cdesk-other"]);
    assert!(ids(&state, "not-targeted").is_empty());
}

#[test]
fn the_most_recently_used_is_listed_first() {
    let state = state(|workspace| {
        add(workspace, "cdesk-newer", Some("ledger"), 50);
        add(workspace, "cdesk-older", Some("ledger"), 0);
    });
    assert_eq!(
        ids(&state, "ledger"),
        ["cdesk-newer", "cdesk-backfill", "cdesk-older"]
    );
}

#[test]
fn a_closed_desk_session_is_left_out() {
    let state = state(|workspace| {
        add(workspace, "cdesk-done", Some("ledger"), 50);
        if let Some(record) = workspace
            .desk
            .channel_sessions
            .get_mut(&SessionId::new("cdesk-done"))
        {
            record.desk.closed = Some(humata2_clients::desk::Closed {
                when: "today".to_owned(),
                days_left: None,
            });
        }
    });
    assert_eq!(ids(&state, "ledger"), ["cdesk-backfill"]);
}

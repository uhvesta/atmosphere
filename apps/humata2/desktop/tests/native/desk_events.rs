//! DESKWATCH: the Desk follows changes another client makes, with no action
//! of the app's own.
use super::*;
use humata2_clients::model::ChannelId;

#[test]
fn a_channel_desk_session_removed_by_another_client_leaves_the_desk_tab() {
    let mut workspace = fixture::design();
    fixture::channel_desk(&mut workspace);
    let mut app = App::new(workspace);
    app.send(AppAction::OpenChannelDesk(ChannelId::new("payments-rewrite")));
    assert!(app.contains("channel-desk-row-cdesk-triage"));

    app.fakes.daemon.change_desk_elsewhere(|desk| {
        desk.channel_sessions.remove(&SessionId::new("cdesk-triage"));
    });
    app.settle();

    assert!(!app.contains("channel-desk-row-cdesk-triage"));
    assert!(app.contains("channel-desk-row-cdesk-backfill"));
}

#[test]
fn a_session_reopened_by_another_client_moves_to_the_open_table() {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    let mut app = App::new(workspace);
    app.click("sidebar-desk");
    assert!(app.contains("desk-closed-desk-release-checklist"));

    app.fakes.daemon.change_desk_elsewhere(|desk| {
        if let Some(found) = desk.sessions.get_mut(&SessionId::new("desk-release-checklist")) {
            found.closed = None;
        }
    });
    app.settle();

    assert!(app.contains("desk-open-desk-release-checklist"));
    assert!(!app.contains("desk-closed-desk-release-checklist"));
}

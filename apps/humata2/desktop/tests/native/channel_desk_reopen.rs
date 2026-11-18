//! Reopening a closed kept channel Desk session through the real window
//! (DV-13): its row is back in the open table and gone from Closed.
use super::*;
use humata2_clients::model::ChannelId;

fn payments() -> ChannelId {
    ChannelId::new("payments-rewrite")
}

#[test]
fn clicking_reopen_on_a_closed_kept_row_brings_it_back_to_the_open_table() {
    let mut workspace = fixture::design();
    fixture::channel_desk(&mut workspace);
    if let Some(record) = workspace
        .desk
        .channel_sessions
        .get_mut(&SessionId::new("cdesk-triage"))
    {
        record.desk.closed = Some(humata2_clients::desk::Closed {
            when: "today".to_owned(),
            days_left: None,
        });
        record.desk.version_count = 2;
    }
    let mut app = App::new(workspace);
    app.send(AppAction::OpenChannelDesk(payments()));
    assert!(!app.contains("channel-desk-row-cdesk-triage"));
    assert!(app.contains("channel-desk-closed-cdesk-triage"));

    app.click("channel-desk-reopen-cdesk-triage");
    app.send(AppAction::OpenChannelDesk(payments()));

    assert!(app.contains("channel-desk-row-cdesk-triage"));
    assert!(!app.contains("channel-desk-closed-cdesk-triage"));
    assert!(!app.contains("channel-desk-reopen-cdesk-triage"));
}

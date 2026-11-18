//! Messages waiting to send.
use super::*;

/// SE-24: the tab bar says how many others can type into a tab as its link
/// tells it, and stops saying so when they leave.
#[test]
fn the_tab_bar_follows_who_else_can_type_as_the_link_says() {
    use humata2_clients::TerminalNotice;
    let mut app = App::new(fixture::design_session(Status::Working));
    app.open("ledger", Panel::Inbox);
    app.send(AppAction::Session(SessionAction::SelectTab(1)));
    app.settle();
    assert!(!app.contains("terminal-peers-0"));

    let ledger = SessionId::new("ledger");
    app.fakes
        .daemon
        .terminal_notice(&ledger, 1, TerminalNotice::Writers { others: 2 });
    app.settle();
    assert!(app.contains("terminal-peers-0"));

    app.fakes
        .daemon
        .terminal_notice(&ledger, 1, TerminalNotice::Writers { others: 0 });
    app.settle();
    assert!(!app.contains("terminal-peers-0"));
}

/// SE-23/24: pending messages are inspectable; writers belong to one terminal.
#[test]
fn queued_messages_can_be_shown_removed_and_sent_and_presence_follows_the_tab() {
    // Other writers are named while the agent works (frame 01).
    let mut app = App::new(fixture::design_session(Status::Working));
    app.open("ledger", Panel::Inbox);
    assert!(app.contains("terminal-peers-0"));
    app.send(AppAction::Session(SessionAction::SelectTab(1)));
    assert!(!app.contains("terminal-peers-0"));
    app.send(AppAction::Session(SessionAction::SelectTab(0)));
    assert!(app.contains("terminal-peers-0"));
    for message in [
        "Keep both indexes until the backfill ends.",
        "Then check the read path before dropping the old index.",
    ] {
        app.send(AppAction::Session(SessionAction::Draft(message.into())));
        app.send(AppAction::Session(SessionAction::Submit));
    }
    assert!(app.contains("session-queue"));
    assert!(!app.contains("queued-message-0"));
    app.click("show-queued");
    assert!(app.contains("queued-message-1"));
    app.snapshot("queued-messages-expanded");
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    app.snapshot("queued-messages-expanded-dark");
    app.click("remove-queued-0");
    assert_eq!(
        app.ui.read(|s| s
            .session
            .view()
            .unwrap()
            .queue
            .iter()
            .map(|message| message.text())
            .collect::<Vec<_>>()),
        ["Then check the read path before dropping the old index."]
    );
    app.click("send-queued-0");
    assert!(!app.contains("session-queue"));
    assert!(app.ui.read(|s| {
        s.workspace
            .session(&SessionId::new("ledger"))
            .unwrap()
            .transcript
            .iter()
            .any(|l| l.text == "Then check the read path before dropping the old index.")
    }));
    assert!(!app.ui.read(|s| {
        s.workspace
            .session(&SessionId::new("ledger"))
            .unwrap()
            .transcript
            .iter()
            .any(|l| l.text == "Keep both indexes until the backfill ends.")
    }));
}

/// SE-23: output-fallback agents keep waiting until the person presses Send now.
#[test]
fn uncertain_turn_end_keeps_the_queue_visible_until_explicit_send() {
    let mut workspace = fixture::design();
    let session = workspace
        .sessions
        .iter_mut()
        .find(|s| s.id == SessionId::new("ledger"))
        .unwrap();
    session.agent = humata2_clients::model::Agent::Codex;
    session.turn = humata2_clients::model::Turn::Uncertain;
    session.status = Status::ProbablyIdle;
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Inbox);
    app.send(AppAction::Session(SessionAction::Draft(
        "Wait for me to send this.".into(),
    )));
    app.send(AppAction::Session(SessionAction::Submit));
    assert_eq!(app.ui.read(|s| s.session.view().unwrap().queue.len()), 1);
    assert!(app.contains("send-next-queued"));
    app.snapshot("queued-message-uncertain");
    app.click("send-next-queued");
    assert!(!app.contains("session-queue"));
}

/// SE-23: a transport failure leaves a visible retry and preserves the message.
#[test]
fn a_failed_queued_message_stays_visible_for_retry() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Inbox);
    app.send(AppAction::Session(SessionAction::Draft(
        "Retry this message intact.".into(),
    )));
    app.send(AppAction::Session(SessionAction::Submit));
    app.click("show-queued");
    app.fakes.daemon.fail_next(
        humata2_clients::Operation::Send,
        "The connection dropped. Reconnect the machine, then send again.",
    );
    app.click("send-queued-0");
    assert!(app.contains("session-error"));
    assert_eq!(
        app.ui.read(|s| s
            .session
            .view()
            .unwrap()
            .queue
            .iter()
            .map(|message| message.text())
            .collect::<Vec<_>>()),
        ["Retry this message intact."]
    );
    assert!(app.contains("send-next-queued"));
    app.snapshot("queued-message-failed");
    app.cx
        .update_window(app.window, |_, window, cx| {
            let dismiss = window.find(("session-error", 1usize)).bounds();
            let banner = window.find("session-error").bounds();
            assert!(dismiss.origin.x + dismiss.size.width <= banner.origin.x + banner.size.width);
            window.click(("session-error", 1usize), cx);
        })
        .unwrap();
    app.settle();
    assert!(!app.contains("session-error"));
    assert!(app.contains("session-queue"));
    app.click("send-next-queued");
    assert!(!app.contains("session-queue"));
    assert!(!app.contains("session-error"));
}

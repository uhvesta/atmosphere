//! To-do items in the window: an agent's ask on one is answered from its
//! drawer and touches only that item, and a channel's To-do tab lists only
//! that channel's items (D834, D874).
use super::*;

use humata2_clients::todo::Call;

#[test]
fn approving_an_items_ask_answers_it_and_leaves_the_other_asks() {
    let mut app = App::new(fixture::design());
    app.send(AppAction::Switch(Route::Todo));
    app.click("todo-row PAY-3");
    app.click("todo-yes");
    assert!(
        app.fakes
            .todo
            .calls()
            .contains(&Call::Answer("req-pay-3".to_owned(), true))
    );
    let entries = app.fakes.todo.entries();
    let entry = |key: &str| {
        entries
            .iter()
            .find(|e| e.todo.key == key)
            .expect("the fixture item")
    };
    assert!(entry("PAY-3").todo.ask.is_none());
    assert!(entry("PAY-3").ask_id.is_none());
    assert!(entry("INF-2").todo.ask.is_some());
    assert!(app.ui.read(|s| {
        s.todo
            .items
            .iter()
            .find(|e| e.todo.key == "PAY-3")
            .is_none_or(|e| e.todo.ask.is_none())
    }));
}

#[test]
fn a_channels_todo_tab_lists_only_that_channels_items() {
    let mut app = App::new(fixture::design());
    app.send(AppAction::Switch(Route::Channel(
        humata2_clients::model::ChannelId::new("infra"),
    )));
    app.click("channel-tab-Board");
    assert!(app.contains("todo-row INF-2"));
    assert!(!app.contains("todo-row PAY-3"));
}

#[test]
fn starting_a_session_from_the_todo_drawer_opens_the_prefilled_new_session_dialog() {
    let mut app = App::new(fixture::design());
    app.send(AppAction::Switch(Route::Todo));

    // Suggested and pull-request review items do not offer Start a session on this.
    app.click("todo-row SUG-1");
    assert!(!app.contains("todo-start-session"));
    app.click("todo-row REV-1");
    assert!(app.contains("todo-start-review"));
    assert!(!app.contains("todo-start-session"));

    // A regular item opens the prefilled New session sheet for that item.
    app.click("todo-row PAY-7");
    assert!(app.contains("todo-start-session"));
    app.click("todo-start-session");
    assert!(app.contains("new-session-create"));
    app.ui.read(|s| match s.overlay() {
        Some(humata2_app::Overlay::NewSession(sheet)) => {
            assert_eq!(sheet.title, "Refund retry with backoff");
            assert_eq!(
                sheet.item,
                Some(humata2_clients::model::ItemId::new("PAY-7"))
            );
            assert_eq!(
                sheet.channel,
                Some(humata2_clients::model::ChannelId::new("payments-rewrite"))
            );
            assert!(sheet.machine.is_some());
        }
        other => panic!("expected the New session overlay, got {other:?}"),
    });
}

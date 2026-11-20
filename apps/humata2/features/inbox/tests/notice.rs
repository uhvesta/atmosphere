//! The notification content for an agent that stopped to ask (frame 209).
use humata2_clients::fixture;
use humata2_clients::model::{ItemId, Preview, ToDoAsk};
use humata2_inbox::{notification, to_do_notification};

#[test]
fn an_agent_inbox_ask_makes_a_notice_that_names_the_ask() {
    let workspace = fixture::design();
    let Some(request) = workspace.requests.iter().find(|r| r.answer.is_none()) else {
        panic!("the design fixture has a waiting request");
    };
    let Some(notice) = notification(&workspace, request) else {
        panic!("an agent-inbox ask notifies");
    };
    assert!(notice.title.contains(&request.title), "{notice:?}");
    assert!(!notice.body.is_empty());
}

#[test]
fn a_to_do_ask_makes_no_notice() {
    let workspace = fixture::design();
    let Some(request) = workspace.requests.iter().find(|r| r.answer.is_none()) else {
        panic!("the design fixture has a waiting request");
    };
    let mut ask = request.clone();
    ask.to_do = Some(ToDoAsk::Item(ItemId::new("item-1")));
    assert_eq!(notification(&workspace, &ask), None);
}

#[test]
fn an_answered_ask_makes_no_notice() {
    let workspace = fixture::design();
    let Some(request) = workspace.requests.iter().find(|r| r.answer.is_none()) else {
        panic!("the design fixture has a waiting request");
    };
    let mut ask = request.clone();
    ask.answer = Some(humata2_clients::model::Answer::Approved);
    assert_eq!(notification(&workspace, &ask), None);
}

fn waiting_ask() -> (humata2_clients::Workspace, humata2_clients::model::Request) {
    let workspace = fixture::design();
    let Some(request) = workspace.requests.iter().find(|r| r.answer.is_none()) else {
        panic!("the design fixture has a waiting request");
    };
    (workspace.clone(), request.clone())
}

#[test]
fn a_suggested_item_notifies_on_the_to_do() {
    let (workspace, mut ask) = waiting_ask();
    ask.to_do = Some(ToDoAsk::NewItem);
    ask.preview = Preview::Text("Ship the thing\nmore words".to_owned());
    let Some(notice) = to_do_notification(&workspace, &ask) else {
        panic!("a To-do ask notifies");
    };
    assert!(notice.title.contains("suggests a To-do: Ship the thing"), "{notice:?}");
    assert!(!notice.title.contains("more words"));
}

#[test]
fn a_change_to_an_item_notifies_on_the_to_do() {
    let (workspace, mut ask) = waiting_ask();
    ask.to_do = Some(ToDoAsk::Item(ItemId::new("item-1")));
    ask.preview = Preview::Text("Ship the thing".to_owned());
    let Some(notice) = to_do_notification(&workspace, &ask) else {
        panic!("a To-do ask notifies");
    };
    assert!(notice.title.contains("wants to change 'Ship the thing'"), "{notice:?}");
}

#[test]
fn an_answered_or_inbox_ask_makes_no_to_do_notice() {
    let (workspace, mut ask) = waiting_ask();
    assert_eq!(to_do_notification(&workspace, &ask), None);
    ask.to_do = Some(ToDoAsk::NewItem);
    ask.answer = Some(humata2_clients::model::Answer::Approved);
    assert_eq!(to_do_notification(&workspace, &ask), None);
}

#[test]
fn a_to_do_ask_is_no_inbox_card_chip_or_count() {
    use humata2_inbox::{InboxState, waiting};
    use std::sync::Arc;
    let (mut workspace, ask) = waiting_ask();
    let before = waiting(&workspace);
    let cards_before = InboxState { workspace: Arc::new(workspace.clone()), ..InboxState::default() }
        .shown()
        .len();
    let chips_before = InboxState { workspace: Arc::new(workspace.clone()), ..InboxState::default() }.chips();
    let Some(found) = workspace.requests.iter_mut().find(|r| r.id == ask.id) else {
        panic!("the request is in the workspace");
    };
    found.to_do = Some(ToDoAsk::NewItem);
    let inbox = InboxState { workspace: Arc::new(workspace.clone()), ..InboxState::default() };
    assert_eq!(waiting(&workspace), before - 1);
    assert_eq!(inbox.shown().len(), cards_before - 1);
    let count = |chips: &[(humata2_clients::model::ChannelId, usize)]| {
        chips.iter().map(|(_, n)| n).sum::<usize>()
    };
    assert_eq!(count(&inbox.chips()), count(&chips_before) - 1);
}

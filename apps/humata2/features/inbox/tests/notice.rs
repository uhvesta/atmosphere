//! The notification content for an agent that stopped to ask (frame 209).
use humata2_clients::fixture;
use humata2_inbox::notification;

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
    ask.operation = "board.item_update".to_owned();
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

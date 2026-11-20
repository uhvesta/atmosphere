//! A To-do ask waits on the To-do, never in the waiting requests.
use humata2_clients::fixture;
use humata2_clients::model::ToDoAsk;

#[test]
fn waiting_leaves_out_to_do_asks() {
    let mut workspace = fixture::design();
    let before = workspace.waiting().count();
    let Some(request) = workspace.requests.iter_mut().find(|r| r.answer.is_none()) else {
        panic!("the design fixture has a waiting request");
    };
    request.to_do = Some(ToDoAsk::NewItem);
    assert_eq!(workspace.waiting().count(), before - 1);
}

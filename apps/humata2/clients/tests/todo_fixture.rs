//! BOARDSIMPLE: the fake To-do list ties items to the sessions the fixture
//! workspace has, so a session's To-do panel shows its slice.
use humata2_clients::model::Status;
use humata2_clients::{Workspace, fixture, todo::FakeTodo};

fn slice(fake: &FakeTodo, session: &str) -> Vec<String> {
    let mut keys: Vec<String> = fake
        .entries()
        .into_iter()
        .filter(|e| e.todo.sessions.iter().any(|s| s == session))
        .map(|e| e.todo.key)
        .collect();
    keys.sort();
    keys
}

#[test]
fn the_ledger_session_works_on_pay_3_and_its_ask_waits_there() {
    let fake = FakeTodo::for_workspace(&fixture::design_session(Status::Working));
    assert_eq!(slice(&fake, "ledger"), ["PAY-3", "PAY-7", "PAY-9"]);
    let asking: Vec<String> = fake
        .entries()
        .into_iter()
        .filter(|e| e.ask_id.is_some() && e.todo.sessions.iter().any(|s| s == "ledger"))
        .map(|e| e.todo.key)
        .collect();
    assert_eq!(asking, ["PAY-3"]);
}

#[test]
fn token_refresh_retry_gets_its_three_items() {
    let fake = FakeTodo::for_workspace(&fixture::standard());
    assert_eq!(slice(&fake, "s-auth"), ["A-1", "A-2", "A-3"]);
}

#[test]
fn a_workspace_without_those_sessions_keeps_the_plain_list() {
    let fake = FakeTodo::for_workspace(&Workspace::default());
    assert_eq!(fake.entries(), FakeTodo::frame_b1_1().entries());
}

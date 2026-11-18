//! MP-23, D792: an agent's ask to import a provider file is answered one by
//! one, never by Approve all or Deny all.
use humata2_clients::model::PROVIDER_IMPORT_OPERATION;
use humata2_clients::{Clients, fixture};
use humata2_inbox::{Inbox, InboxState};
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn store(workspace: humata2_clients::Workspace) -> TestStore<Inbox> {
    let (clients, _fakes) = Clients::fake(workspace.clone(), Arc::new(TestClock::new()));
    let state = InboxState {
        workspace: Arc::new(workspace),
        ..InboxState::default()
    };
    TestStore::new(state, Inbox::new(clients))
}

#[test]
fn approve_all_leaves_an_agents_provider_import_out() {
    let workspace = fixture::design();
    let before = store(workspace.clone()).state().bulk_count();

    let mut asked = workspace;
    let Some(request) = asked.requests.iter_mut().find(|request| request.answer.is_none()) else {
        panic!("the design fixture has a waiting request");
    };
    request.operation = PROVIDER_IMPORT_OPERATION.to_owned();

    assert_eq!(store(asked).state().bulk_count(), before - 1);
}

//! BROWSER: Inbox offers `AllowFor30Min` ONLY for `BrowserRead` with
//! `supports_30m_grant`, never offers `AlwaysAllow` on browser cards, and
//! leaves `BrowserWrite` out of `ApproveAll`.

use std::sync::Arc;

use humata2_clients::fixture;
use humata2_clients::model::{Answer, Preview, Request, RequestId};
use humata2_clients::{Fakes, Workspace};
use humata2_inbox::{Inbox, InboxAction as A, InboxState, Offer};
use tca::{TestClock, TestStore};

fn setup(requests: Vec<Request>) -> (TestStore<Inbox>, Fakes) {
    let mut workspace = Workspace {
        requests,
        ..fixture::design()
    };
    // Keep only our explicit requests so bulk assertions are exact.
    let (clients, fakes) = humata2_clients::fake_clients(workspace.clone(), Arc::new(TestClock::new()));
    workspace = fakes.daemon.workspace();
    let state = InboxState {
        workspace: Arc::new(workspace),
        ..InboxState::default()
    };
    (TestStore::new(state, Inbox::new(clients)), fakes)
}

#[test]
fn browser_read_offers_allow_once_allow_30m_and_deny_only_when_supported() {
    let read_30m = fixture::browser_read_request("r-read-30m", "ledger", "payments");
    let mut read_once = fixture::browser_read_request("r-read-once", "ledger", "payments");
    if let Preview::Browser(ref mut p) = read_once.preview {
        p.supports_30m_grant = false;
    }
    let write = fixture::browser_write_request("r-write", "ledger", "payments");

    let (store, _) = setup(vec![read_30m, read_once, write]);

    assert_eq!(
        store.state().offers(&RequestId::new("r-read-30m")),
        vec![Offer::Allow, Offer::AllowFor30Min, Offer::Deny]
    );
    assert_eq!(
        store.state().offers(&RequestId::new("r-read-once")),
        vec![Offer::Allow, Offer::Deny]
    );
    assert_eq!(
        store.state().offers(&RequestId::new("r-write")),
        vec![Offer::Allow, Offer::Deny]
    );
}

#[test]
fn browser_read_accepts_allow_for_30_min_while_browser_write_rejects_it() {
    let read = fixture::browser_read_request("r-read", "ledger", "payments");
    let write = fixture::browser_write_request("r-write", "ledger", "payments");
    let (mut store, fakes) = setup(vec![read, write]);

    let read_id = RequestId::new("r-read");
    let write_id = RequestId::new("r-write");

    // BrowserWrite ignores AllowFor30Min and Always.
    store.send(A::Answer(write_id.clone(), Answer::AllowFor30Min), |_| {});
    store.send(A::Always(write_id.clone()), |_| {});
    assert!(fakes.daemon.answers().is_empty());

    // BrowserRead sends AllowFor30Min cleanly.
    store.send(A::Answer(read_id.clone(), Answer::AllowFor30Min), |s| {
        s.pending.insert(read_id.clone());
    });
    store.receive(A::Answered(read_id.clone(), Ok(())), |s| {
        s.pending.remove(&read_id);
    });
    assert_eq!(
        fakes.daemon.answers(),
        vec![(read_id, Answer::AllowFor30Min)]
    );
    store.finish();
}

#[test]
fn approve_all_approves_browser_read_and_skips_browser_write() {
    let read = fixture::browser_read_request("r-read", "ledger", "payments");
    let write = fixture::browser_write_request("r-write", "ledger", "payments");
    assert!(read.answered_in_bulk());
    assert!(!write.answered_in_bulk());

    let (mut store, fakes) = setup(vec![read, write]);
    let bulk_ids: Vec<&str> = store
        .state()
        .bulk_approvable()
        .iter()
        .map(|r| r.id.as_str())
        .collect();
    assert_eq!(bulk_ids, vec!["r-read"]);

    let read_id = RequestId::new("r-read");
    store.send(A::ApproveAll, |s| {
        s.pending.insert(read_id.clone());
    });
    store.receive(A::Answered(read_id.clone(), Ok(())), |s| {
        s.pending.remove(&read_id);
    });
    assert_eq!(fakes.daemon.answers(), vec![(read_id, Answer::Approved)]);
    store.finish();
}

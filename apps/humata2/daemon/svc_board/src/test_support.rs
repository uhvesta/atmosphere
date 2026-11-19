//! What the unit tests of this crate share: a guard that allows everything,
//! a feed and an Inbox that do nothing, and a service over a temp store.

use humata2_daemon_ports::{Caller, UserVia};

use super::*;

pub(crate) struct AllowAllGuard;

impl BoardGuard for AllowAllGuard {
    fn check(&self, _caller: &Caller, _call: &Call) -> Result<Checked, Status> {
        Ok(Checked::Allowed)
    }
    fn check_away(&self, _caller: &Caller, _call: &Call) -> Result<AwayCall, Status> {
        Ok(AwayCall::Reachable)
    }
    fn lists_low_risk(&self, _asked: &AccessRequest) -> bool {
        false
    }
    fn policy_changed(&self) {}
    fn host_link(&self) -> String {
        String::new()
    }
}

pub(crate) struct NoopFeed;

impl BoardFeed for NoopFeed {
    fn publish_in(
        &self,
        _change: pb::Change,
        _payload: pb::event::Payload,
        _scope: Scope,
        _by: Option<pb::Actor>,
    ) {
    }
}

pub(crate) struct NoopInbox;

#[tonic::async_trait]
impl BoardInbox for NoopInbox {
    async fn withdraw_about_item(&self, _key: &str) -> Result<(), Status> {
        Ok(())
    }
    async fn defer_board(
        &self,
        _caller: &Caller,
        _asked: AccessRequest,
        _idempotency: Option<String>,
        _board: BoardAsk,
        _run: Run,
    ) -> Status {
        Status::permission_denied("deferred")
    }
    async fn pending_board_asks(&self) -> Vec<pb::InboxRequest> {
        Vec::new()
    }
    async fn wait_final(
        &self,
        _caller: Caller,
        _request: pb::WaitForAnswerRequest,
    ) -> Result<pb::WaitResponse, Status> {
        Err(Status::unimplemented("noop"))
    }
    async fn withdraw_as(
        &self,
        _caller: Caller,
        _request: pb::WithdrawRequestRequest,
    ) -> Result<pb::InboxRequest, Status> {
        Err(Status::unimplemented("noop"))
    }
    async fn approved_on(
        &self,
        _machine: &str,
        _id: &str,
        _session: &str,
    ) -> Option<pb::InboxRequest> {
        None
    }
}

pub(crate) fn user_req<T>(msg: T) -> Request<T> {
    let mut req = Request::new(msg);
    req.extensions_mut().insert(Caller::User(UserVia::App));
    req
}

pub(crate) fn test_svc(dir: &tempfile::TempDir) -> BoardSvc {
    let store = Arc::new(Store::open(dir.path().join("state")).expect("open store"));
    BoardSvc::new(
        Arc::new(AllowAllGuard),
        store,
        Arc::new(NoopFeed),
        Arc::new(NoopInbox),
        "local".to_owned(),
    )
}


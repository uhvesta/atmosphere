//! Start fresh with a summary (PER-15, D143) through a real `humatad2`.
//! Main moves this to `tests/start_fresh.rs` once the seams are applied.

mod common;

use common::{
    Attach, Daemon, World, agent_tab, create_channel, finished, get_session, kill_holders,
    open, start_session, start_world, stored_tabs, wait_status,
};
use humata2_grpc::humata::v1 as pb;
use pb::SessionStatus as Status;

struct Fixture {
    daemon: Daemon,
    world: World,
    session: pb::Session,
}

impl Fixture {
    async fn new() -> Self {
        let (daemon, world) = start_world().await;
        world.remote("acme", "api");
        let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        let (id, _) = start_session(&daemon, &channel, "Fix login").await;
        let session = wait_status(&daemon, &id, Status::Working).await;
        Self {
            daemon,
            world,
            session,
        }
    }

    fn id(&self) -> pb::SessionId {
        self.session.id.clone().unwrap()
    }

    async fn say(&self, line: &str) {
        let tab = agent_tab(&self.daemon, &self.id()).await;
        let mut attach = Attach::start(&self.daemon, open(&tab, 0)).await;
        attach.type_text(&format!("{line}\r")).await;
        attach.wait_for(&format!("got: {line}")).await;
    }

    /// A reboot that also lost the conversation: the session ends up
    /// RESUME_FAILED.
    async fn lose_conversation(&mut self) {
        let id = self.id().value;
        let tabs: Vec<_> = stored_tabs(&self.daemon.state)
            .into_iter()
            .filter(|tab| tab.session.as_deref() == Some(id.as_str()))
            .collect();
        self.daemon.kill();
        kill_holders(&tabs).await;
        std::fs::remove_file(self.world.conversation(&self.session.conversation_id)).unwrap();
        self.daemon.restart().await;
        wait_status(&self.daemon, &self.id(), Status::ResumeFailed).await;
    }

    async fn start_fresh(&self, key: &str) -> Result<pb::Operation, tonic::Status> {
        let operation = self
            .daemon
            .session_service()
            .await
            .resume_session(pb::ResumeSessionRequest {
                idempotency_key: Some(pb::IdempotencyKey {
                    value: key.to_owned(),
                }),
                id: Some(self.id()),
                mode: pb::resume_session_request::Mode::StartFreshWithSummary as i32,
            })
            .await?
            .into_inner();
        Ok(finished(&self.daemon, &operation).await)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_resume_starts_fresh_on_a_new_conversation_and_hears_the_summary() {
    let mut fix = Fixture::new().await;
    fix.say("one").await;
    fix.lose_conversation().await;
    let failed = get_session(&fix.daemon, &fix.id()).await;
    assert!(failed.resume_failed.is_some());
    assert!(failed.interrupted_at.is_some());

    let operation = fix.start_fresh("k-1").await.unwrap();

    assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{:?}", operation.error);
    let session = wait_status(&fix.daemon, &fix.id(), Status::Working).await;
    assert_ne!(session.conversation_id, fix.session.conversation_id);
    assert_eq!(session.id, fix.session.id);
    assert!(session.resume_failed.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_same_key_returns_the_same_operation() {
    let mut fix = Fixture::new().await;
    fix.lose_conversation().await;
    let first = fix.start_fresh("same").await.unwrap();
    let again = fix.start_fresh("same").await.unwrap();
    assert_eq!(first.id, again.id);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_working_session_cannot_start_fresh() {
    let fix = Fixture::new().await;
    let refused = fix.start_fresh("k-2").await.unwrap_err();
    assert_eq!(refused.code(), tonic::Code::FailedPrecondition);
}

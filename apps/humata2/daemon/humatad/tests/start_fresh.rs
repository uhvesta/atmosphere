//! Start fresh with a summary (PER-15, D143) through a real `humatad2`.

mod common;

use common::{
    Attach, Daemon, World, agent_tab, create_channel, finished, get_session, kill_holders,
    open, start_session, start_world, stored_tabs, wait_status,
};
use std::time::{Duration, Instant};

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
        self.lose_conversation_then_start(&[]).await;
    }

    /// As [`Self::lose_conversation`], with `env` for the daemon that comes
    /// back, such as `HUMATA2_STOP_AT`.
    async fn lose_conversation_then_start(&mut self, env: &[(&str, &str)]) {
        let id = self.id().value;
        let tabs: Vec<_> = stored_tabs(&self.daemon.state)
            .into_iter()
            .filter(|tab| tab.session.as_deref() == Some(id.as_str()))
            .collect();
        self.daemon.kill();
        kill_holders(&tabs).await;
        std::fs::remove_file(self.world.conversation(&self.session.conversation_id)).unwrap();
        self.daemon.restart_env(env).await;
        wait_status(&self.daemon, &self.id(), Status::ResumeFailed).await;
    }

    /// Asks to start fresh without waiting for the answer.
    async fn ask_start_fresh(&self, key: &str) {
        let _ = self
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
            .await;
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

#[tokio::test(flavor = "multi_thread")]
async fn a_lost_conversation_is_summarized_from_the_saved_terminal_output() {
    let mut fix = Fixture::new().await;
    fix.say("remember-the-blue-door").await;
    fix.lose_conversation().await;

    let operation = fix.start_fresh("k-3").await.unwrap();

    assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{:?}", operation.error);
    let session = wait_status(&fix.daemon, &fix.id(), Status::Working).await;
    // The new agent keeps every line typed to it: the seed carried the old
    // terminal's answer.
    let file = fix.world.conversation(&session.conversation_id);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let heard = std::fs::read_to_string(&file).unwrap_or_default();
        if heard.contains("got: remember-the-blue-door") {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "the new conversation holds {heard:?}");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_resume_says_what_start_fresh_can_summarize() {
    let mut fix = Fixture::new().await;
    fix.say("one").await;
    assert_eq!(fix.session.summary_source, pb::session::SummarySource::Unspecified as i32);

    fix.lose_conversation().await;

    // The conversation file is gone, but the terminal's output was saved.
    let failed = get_session(&fix.daemon, &fix.id()).await;
    assert_eq!(failed.summary_source, pb::session::SummarySource::Scrollback as i32);
}

/// How often `needle` is in `text`.
fn times(text: &str, needle: &str) -> usize {
    text.matches(needle).count()
}

/// Waits until the conversation file holds `needle`, and returns what it holds.
async fn heard_with(file: &std::path::Path, needle: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let heard = std::fs::read_to_string(file).unwrap_or_default();
        if heard.contains(needle) {
            return heard;
        }
        assert!(Instant::now() < deadline, "the new conversation holds {heard:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A start fresh whose daemon dies once the new agent runs and before its
/// first message is typed; then the daemon starts again. With
/// `holder_lost`, the new agent's holder dies with it, so the restart
/// resumes the agent instead of finding it running.
async fn killed_after_launch(holder_lost: bool) {
    let mut fix = Fixture::new().await;
    fix.say("remember-the-blue-door").await;
    fix.lose_conversation_then_start(&[("HUMATA2_STOP_AT", "start_fresh.launched")]).await;

    fix.ask_start_fresh("k-kill").await;
    let deadline = Instant::now() + Duration::from_secs(20);
    while std::os::unix::net::UnixStream::connect(&fix.daemon.socket).is_ok() {
        assert!(Instant::now() < deadline, "the daemon never died at the fault point");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    fix.daemon.kill();
    if holder_lost {
        let id = fix.id().value;
        let tabs: Vec<_> = stored_tabs(&fix.daemon.state)
            .into_iter()
            .filter(|tab| tab.session.as_deref() == Some(id.as_str()))
            .collect();
        kill_holders(&tabs).await;
    }

    fix.daemon.restart().await;

    let session = wait_status(&fix.daemon, &fix.id(), Status::Working).await;
    assert_ne!(session.conversation_id, fix.session.conversation_id);
    let file = fix.world.conversation(&session.conversation_id);
    let tag = format!("[start fresh {}]", session.conversation_id);
    heard_with(&file, &tag).await;
    // Give a second typing time to show itself.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let heard = std::fs::read_to_string(&file).unwrap_or_default();
    assert_eq!(times(&heard, &tag), 1, "the new conversation holds {heard:?}");
    // The seed is the agent's first message: nothing, such as the note that
    // it was interrupted, came before it.
    assert!(
        heard.lines().next().is_some_and(|first| first.starts_with(&tag)),
        "the new conversation holds {heard:?}"
    );
    // It carried the old terminal's output.
    assert!(heard.contains("remember-the-blue-door"), "the new conversation holds {heard:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_start_fresh_killed_after_launch_types_its_seed_once_after_the_restart() {
    killed_after_launch(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_start_fresh_killed_after_launch_with_its_holder_lost_still_hears_the_seed_first() {
    killed_after_launch(true).await;
}

/// Every step a start fresh takes, as the points a cut can land on. The
/// two `revive` points are the new agent's launch, record then process.
const START_FRESH_POINTS: &[&str] = &[
    "start_fresh.recorded",
    "start_fresh.busy",
    "start_fresh.seed_kept",
    "revive.launch.recorded",
    "revive.launch.spawned",
    "start_fresh.launched",
    "start_fresh.typed",
];

/// The session once it is no longer in flight: working again, or showing
/// the failed resume it began with.
async fn settled(fix: &Fixture) -> pb::Session {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let session = get_session(&fix.daemon, &fix.id()).await;
        if session.status == Status::Working as i32 || session.status == Status::ResumeFailed as i32 {
            return session;
        }
        assert!(Instant::now() < deadline, "the session never settled: {session:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A start fresh cut at any step either finished (a new conversation that
/// heard its first message once) or left the session as it was, and asking
/// again from there works. Either way the session has one agent tab.
#[tokio::test(flavor = "multi_thread")]
async fn a_start_fresh_cut_at_any_point_finishes_or_leaves_the_session_as_it_was() {
    for point in START_FRESH_POINTS {
        let mut fix = Fixture::new().await;
        fix.say("remember-the-blue-door").await;
        fix.lose_conversation_then_start(&[("HUMATA2_STOP_AT", point)]).await;
        fix.ask_start_fresh("k-cut").await;
        let deadline = Instant::now() + Duration::from_secs(60);
        while std::os::unix::net::UnixStream::connect(&fix.daemon.socket).is_ok() {
            assert!(Instant::now() < deadline, "the daemon never died at {point}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        fix.daemon.restart().await;

        let mut session = settled(&fix).await;
        if session.status == Status::ResumeFailed as i32 {
            // Left as it was: asking again works.
            assert_eq!(session.conversation_id, fix.session.conversation_id, "cut at {point}");
            let ended = fix.start_fresh("k-again").await.expect("start fresh is asked again");
            assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "cut at {point}: {ended:?}");
            session = wait_status(&fix.daemon, &fix.id(), Status::Working).await;
        }
        assert_ne!(session.conversation_id, fix.session.conversation_id, "cut at {point}");
        let file = fix.world.conversation(&session.conversation_id);
        let tag = format!("[start fresh {}]", session.conversation_id);
        let heard = heard_with(&file, &tag).await;
        assert_eq!(times(&heard, &tag), 1, "cut at {point}: the new conversation holds {heard:?}");
        let id = fix.id().value;
        let agent_tabs: Vec<_> = stored_tabs(&fix.daemon.state)
            .into_iter()
            .filter(|tab| tab.session.as_deref() == Some(id.as_str()) && tab.kind == humata2_store::TabKind::Agent)
            .collect();
        assert_eq!(agent_tabs.len(), 1, "cut at {point}: one agent tab, no orphan");
        assert!(common::holder_answers(&agent_tabs[0]), "cut at {point}: its holder runs");
    }
}

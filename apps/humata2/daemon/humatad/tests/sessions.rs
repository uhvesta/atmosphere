//! SessionService through a real `humatad2` (SE-1..4, FC-11, FC-14, PER-3,
//! PER-23): a session gets its clone on a branch named from its title, its
//! holder keeps the agent's terminal, a reattach replays, and the kill
//! matrix (the app, the daemon, the holder, a reboot) brings the session
//! back in its conversation or says what was lost.

mod common;

use std::path::PathBuf;

use common::{
    Attach, Daemon, World, agent_tab, create_channel, finished, get_session, git, holder_answers,
    humata, kill_holders, last_start, machine_id, open, start_session, start_world, stored_tabs,
    wait_status,
};
use humata2_grpc::humata::v1 as pb;
use pb::SessionStatus as Status;

/// A world with one repository, one channel, and one working session.
struct Fixture {
    daemon: Daemon,
    world: World,
    session: pb::Session,
}

impl Fixture {
    async fn new() -> Self {
        let (daemon, world) = start_world().await;
        world.remote("acme", "api");
        let (channel, setup) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        assert_eq!(
            setup.state,
            pb::OperationState::Succeeded as i32,
            "{:?}",
            setup.error
        );
        let (id, operation) = start_session(&daemon, &channel, "Fix login").await;
        assert_eq!(
            operation.state,
            pb::OperationState::Succeeded as i32,
            "{:?}",
            operation.error
        );
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

    async fn tab(&self) -> pb::TabId {
        agent_tab(&self.daemon, &self.id()).await
    }

    /// Types `line` into the agent and waits for its answer.
    async fn say(&self, line: &str) {
        let mut attach = Attach::start(&self.daemon, open(&self.tab().await, 0)).await;
        attach.type_text(&format!("{line}\r")).await;
        attach.wait_for(&format!("got: {line}")).await;
    }

    fn conversation(&self) -> String {
        std::fs::read_to_string(self.world.conversation(&self.session.conversation_id)).unwrap()
    }

    /// The holders of this session's tabs, as the store has them.
    fn agent_holders(&self) -> Vec<humata2_store::TabRecord> {
        let id = self.id().value;
        stored_tabs(&self.daemon.state)
            .into_iter()
            .filter(|tab| tab.session.as_deref() == Some(id.as_str()))
            .collect()
    }
}

async fn preview(daemon: &Daemon, channel: &pb::Channel, title: &str) -> Vec<String> {
    daemon
        .session_service()
        .await
        .preview_branch_names(pb::PreviewBranchNamesRequest {
            channel: channel.id.clone(),
            title: title.to_owned(),
            machine: Some(machine_id(daemon).await),
            repos: Vec::new(),
        })
        .await
        .unwrap()
        .into_inner()
        .names
        .into_iter()
        .map(|name| name.branch)
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_started_session_has_its_clone_on_a_branch_named_from_its_title() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    assert_eq!(preview(&daemon, &channel, "Fix login").await, ["fix-login"]);

    let (id, operation) = start_session(&daemon, &channel, "Fix login").await;

    assert_eq!(
        operation.state,
        pb::OperationState::Succeeded as i32,
        "{:?}",
        operation.error
    );
    assert!(
        operation
            .steps
            .iter()
            .all(|step| step.state == pb::StepState::Done as i32),
        "{:?}",
        operation.steps
    );
    let session = wait_status(&daemon, &id, Status::Working).await;
    assert_eq!(session.title, "Fix login");
    assert!(!session.conversation_id.is_empty());
    let repo = &session.repos[0];
    assert_eq!(repo.branch, "fix-login");
    let clone: PathBuf = world
        .channels
        .join(&channel.folder)
        .join("sessions/fix-login/api");
    assert_eq!(
        std::fs::canonicalize(&repo.path).unwrap(),
        std::fs::canonicalize(&clone).unwrap()
    );
    assert_eq!(
        git(&[
            "-C",
            &clone.display().to_string(),
            "rev-parse",
            "--abbrev-ref",
            "HEAD"
        ]),
        "fix-login"
    );

    let listed = daemon
        .session_service()
        .await
        .list_sessions(pb::ListSessionsRequest::default())
        .await
        .unwrap()
        .into_inner()
        .sessions;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, session.id);
    assert_eq!(listed[0].status, Status::Working as i32);
    // The same title again would now get the numbered name (FC-14).
    assert_eq!(
        preview(&daemon, &channel, "Fix login").await,
        ["fix-login-2"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn typed_input_reaches_the_agent_and_a_reattach_replays_it() {
    let fix = Fixture::new().await;
    let tab = fix.tab().await;
    let mut first = Attach::start(&fix.daemon, open(&tab, 0)).await;
    first.type_text("ping-1\r").await;
    first.wait_for("got: ping-1").await;
    drop(first);

    let mut again = Attach::start(&fix.daemon, open(&tab, 0)).await;
    again.wait_for("got: ping-1").await;

    assert_eq!(again.snapshots, 1);
    assert!(fix.conversation().contains("ping-1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn dropping_the_app_leaves_the_session_working_and_replayable() {
    let fix = Fixture::new().await;
    let tab = fix.tab().await;
    let mut app = Attach::start(&fix.daemon, open(&tab, 0)).await;
    app.type_text("before\r").await;
    app.wait_for("got: before").await;
    // The app goes away mid-stream, with its connection.
    drop(app);

    assert_eq!(
        get_session(&fix.daemon, &fix.id()).await.status,
        Status::Working as i32
    );
    let mut back = Attach::start(&fix.daemon, open(&tab, 0)).await;
    back.wait_for("got: before").await;
    back.type_text("after\r").await;
    back.wait_for("got: after").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn killing_the_daemon_leaves_the_holder_and_the_conversation_alone() {
    let mut fix = Fixture::new().await;
    fix.say("one").await;
    let id = fix.id();

    fix.daemon.kill();
    let before = fix.agent_holders();
    assert!(!before.is_empty());
    assert!(
        before.iter().all(holder_answers),
        "a holder died with the daemon"
    );
    fix.daemon.restart().await;

    let start = last_start(&fix.daemon).await;
    assert!(
        start.not_brought_back.is_empty(),
        "{:?}",
        start.not_brought_back
    );
    let session = wait_status(&fix.daemon, &id, Status::Working).await;
    assert_eq!(session.conversation_id, fix.session.conversation_id);
    fix.say("two").await;
    let conversation = fix.conversation();
    assert!(conversation.contains("one") && conversation.contains("two"));
    fix.daemon.kill();
    let after = fix.agent_holders();
    let pids = |tabs: &[humata2_store::TabRecord]| {
        tabs.iter().map(|tab| tab.holder_pid).collect::<Vec<_>>()
    };
    assert_eq!(pids(&before), pids(&after));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_lost_holder_is_resumed_in_its_conversation_after_a_restart() {
    let mut fix = Fixture::new().await;
    fix.say("one").await;
    let id = fix.id();

    fix.daemon.kill();
    kill_holders(&fix.agent_holders()).await;
    fix.daemon.restart().await;

    let start = last_start(&fix.daemon).await;
    assert!(
        start.not_brought_back.is_empty(),
        "{:?}",
        start.not_brought_back
    );
    let session = wait_status(&fix.daemon, &id, Status::Working).await;
    assert_eq!(session.conversation_id, fix.session.conversation_id);
    let mut attach = Attach::start(&fix.daemon, open(&fix.tab().await, 0)).await;
    attach.wait_for("agent up --resume").await;
    assert!(fix.conversation().contains("one"));
    fix.say("two").await;
    assert!(fix.conversation().contains("two"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reboot_brings_back_the_session_and_a_shell() {
    let mut fix = Fixture::new().await;
    fix.say("one").await;
    let id = fix.id();
    let shell = fix
        .daemon
        .terminals()
        .await
        .open_machine_shell(pb::OpenMachineShellRequest::default())
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap();

    // Everything goes at once: the daemon and every holder.
    fix.daemon.kill();
    kill_holders(&stored_tabs(&fix.daemon.state)).await;
    fix.daemon.restart().await;

    let start = last_start(&fix.daemon).await;
    assert!(
        start.not_brought_back.is_empty(),
        "{:?}",
        start.not_brought_back
    );
    let session = wait_status(&fix.daemon, &id, Status::Working).await;
    assert_eq!(session.conversation_id, fix.session.conversation_id);
    assert!(fix.conversation().contains("one"));
    fix.say("two").await;
    let mut after = Attach::start(&fix.daemon, open(&shell, 0)).await;
    after.type_text("echo bac''k\n").await;
    after.wait_for("back").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reboot_that_lost_the_conversation_says_the_resume_failed() {
    let mut fix = Fixture::new().await;
    fix.say("one").await;
    let id = fix.id();

    fix.daemon.kill();
    kill_holders(&fix.agent_holders()).await;
    std::fs::remove_file(fix.world.conversation(&fix.session.conversation_id)).unwrap();
    fix.daemon.restart().await;

    let start = last_start(&fix.daemon).await;
    let line = start
        .not_brought_back
        .iter()
        .find(|line| line.session == id.value)
        .unwrap_or_else(|| {
            panic!(
                "the start doesn't name the session: {:?}",
                start.not_brought_back
            )
        });
    assert!(!line.problem.is_empty());
    let session = wait_status(&fix.daemon, &id, Status::ResumeFailed).await;
    assert_eq!(session.conversation_id, fix.session.conversation_id);
    // Nothing was started in its place.
    assert!(fix.agent_holders().iter().all(|tab| !holder_answers(tab)));
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_ends_the_agent_and_resuming_brings_it_back_in_its_conversation() {
    let fix = Fixture::new().await;
    fix.say("one").await;
    let id = fix.id();
    let holders = fix.agent_holders();
    assert!(holders.iter().all(holder_answers));

    let stopped = fix
        .daemon
        .session_service()
        .await
        .stop_session(pb::StopSessionRequest {
            idempotency_key: None,
            id: Some(id.clone()),
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(stopped.status, Status::Idle as i32);
    assert_eq!(
        get_session(&fix.daemon, &id).await.status,
        Status::Idle as i32
    );
    let deadline = tokio::time::Instant::now() + common::WAIT;
    while holders.iter().any(holder_answers) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the holder never went"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let operation = fix
        .daemon
        .session_service()
        .await
        .resume_session(pb::ResumeSessionRequest {
            idempotency_key: None,
            id: Some(id.clone()),
            mode: pb::resume_session_request::Mode::Resume as i32,
        })
        .await
        .unwrap()
        .into_inner();
    let operation = finished(&fix.daemon, &operation).await;

    assert_eq!(
        operation.state,
        pb::OperationState::Succeeded as i32,
        "{:?}",
        operation.error
    );
    let session = wait_status(&fix.daemon, &id, Status::Working).await;
    assert_eq!(session.conversation_id, fix.session.conversation_id);
    let mut attach = Attach::start(&fix.daemon, open(&fix.tab().await, 0)).await;
    attach.wait_for("agent up --resume").await;
    assert!(fix.conversation().contains("one"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_cli_starts_a_session_and_lists_it() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let channel_id = channel.id.clone().unwrap().value;
    let machine = machine_id(&daemon).await.value;

    let (code, json) = humata(
        &daemon,
        &[
            "--channel",
            &channel_id,
            "--machine",
            &machine,
            "session",
            "start",
            "--title",
            "Fix login",
        ],
    );

    assert_eq!(code, 0, "{json}");
    let listed = daemon
        .session_service()
        .await
        .list_sessions(pb::ListSessionsRequest::default())
        .await
        .unwrap()
        .into_inner()
        .sessions;
    let session = listed
        .iter()
        .find(|session| session.title == "Fix login")
        .unwrap();
    let id = session.id.clone().unwrap();
    wait_status(&daemon, &id, Status::Working).await;
    let (code, json) = humata(&daemon, &["ls"]);
    assert_eq!(code, 0, "{json}");
    assert!(json.to_string().contains(&id.value), "{json}");
}

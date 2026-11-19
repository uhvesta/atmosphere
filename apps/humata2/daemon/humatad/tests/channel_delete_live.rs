//! D801 H: deleting a channel keeps the running terminals of its sessions
//! running by default. Each running session is filed on the Desk with its
//! folder renamed under the Desk's folder and its holders untouched (same
//! pids); a session that can't move goes on without a session, under "Not
//! in a channel". Ending them is a choice (`end_live`), and an agent may
//! make it only with the destroy switch. Through a real `humatad2`.

use humatad_test_common as common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::{
    Daemon, World, create_channel, finished, folder, holder_answers, shell_token, start_session, state_folder,
    stored_tabs, wait_running,
};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{Store, TabRecord};
use tonic::metadata::MetadataValue;
use tonic::{Code, Request};

struct Fixture {
    daemon: Daemon,
    world: World,
    channel: pb::Channel,
}

impl Fixture {
    async fn new() -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        let args = world.args();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with(common::Build::Admin, dir, &refs).await;
        world.remote("acme", "api");
        let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        Self { daemon, world, channel }
    }

    fn store(&self) -> Store {
        Store::open(state_folder(self.daemon.dir.path()).join("store")).expect("the store opens")
    }

    fn desk_root(&self) -> PathBuf {
        self.world.channels.join("desk")
    }

    /// A session with its agent running and one shell open in it.
    async fn session_with_shell(&self, title: &str) -> (pb::SessionId, Vec<TabRecord>, String) {
        let (id, _) = start_session(&self.daemon, &self.channel, title).await;
        wait_running(&self.daemon, &id).await;
        let (token, _attach) = shell_token(&self.daemon, &id).await;
        let tabs: Vec<TabRecord> = stored_tabs(&state_folder(self.daemon.dir.path()))
            .into_iter()
            .filter(|tab| tab.session.as_deref() == Some(id.value.as_str()))
            .collect();
        assert_eq!(tabs.len(), 2, "an agent tab and a shell tab: {tabs:?}");
        assert!(tabs.iter().all(holder_answers), "both run before the delete");
        (id, tabs, token)
    }

    async fn plan(&self) -> pb::DeleteChannelPlan {
        self.daemon
            .channel_service()
            .await
            .plan_delete_channel(pb::PlanDeleteChannelRequest { channel: self.channel.id.clone() })
            .await
            .unwrap()
            .into_inner()
    }

    fn request(plan: &pb::DeleteChannelPlan, end_live: Option<bool>) -> pb::DeleteChannelRequest {
        pb::DeleteChannelRequest {
            idempotency_key: None,
            channel: plan.channel.clone(),
            plan: plan.token.clone(),
            presence: None,
            delete_unpushed_work: false,
            end_live,
        }
    }

    /// The user deletes the channel and the operation succeeds.
    async fn delete(&self, end_live: Option<bool>) {
        let plan = self.plan().await;
        let deleted = self
            .daemon
            .channel_service()
            .await
            .delete_channel(Self::request(&plan, end_live))
            .await
            .unwrap()
            .into_inner();
        let operation = finished(&self.daemon, deleted.operation.as_ref().unwrap()).await;
        assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{operation:?}");
    }

    async fn listed(&self) -> bool {
        self.daemon
            .channel_service()
            .await
            .list_channels(pb::ListChannelsRequest::default())
            .await
            .unwrap()
            .into_inner()
            .channels
            .iter()
            .any(|channel| channel.id == self.channel.id)
    }

    /// The owner ends each holder, as this test started them.
    async fn end(&self, tabs: &[TabRecord]) {
        let mut machines = pb::machine_service_client::MachineServiceClient::new(self.daemon.channel().await);
        for tab in tabs {
            let _ = machines
                .end_running_holder(pb::EndRunningHolderRequest {
                    machine: None,
                    tab: Some(pb::TabId { value: tab.id.clone() }),
                })
                .await;
        }
    }
}

fn as_agent<T>(message: T, token: &str) -> Request<T> {
    let mut request = Request::new(message);
    request
        .metadata_mut()
        .insert("humata-session-token", MetadataValue::try_from(token).unwrap());
    request
}

async fn until_gone(tabs: &[TabRecord]) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while tabs.iter().any(holder_answers) {
        assert!(Instant::now() < deadline, "a holder still runs");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The default: the session is on the Desk, its folder under the Desk's, the
/// channel is gone, and both holders are the very processes they were.
#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_channel_moves_running_sessions_to_the_desk_and_keeps_their_holders() {
    let fixture = Fixture::new().await;
    let (id, tabs, _) = fixture.session_with_shell("Ledger").await;
    let before = fixture.store().session(&id.value).unwrap().expect("the session is recorded");
    assert!(before.folder.exists());

    let plan = fixture.plan().await;
    assert_eq!(plan.moves_to_desk, vec![id.clone()], "the plan says the session moves");
    assert!(plan.cannot_move.is_empty(), "{plan:?}");
    fixture.delete(None).await;

    assert!(!fixture.listed().await, "the channel is gone");
    let store = fixture.store();
    let after = store.session(&id.value).unwrap().expect("the session is kept");
    assert_eq!(after.channel, "desk", "it is a Desk session now");
    assert!(after.folder.starts_with(fixture.desk_root()), "{:?}", after.folder);
    assert!(after.folder.is_dir(), "its folder is there");
    assert!(!before.folder.exists(), "and no longer in the channel's folder");
    assert!(store.desk(&id.value).unwrap().is_some(), "it has a Desk row");
    let now = stored_tabs(&state_folder(fixture.daemon.dir.path()));
    for tab in &tabs {
        let kept = now.iter().find(|found| found.id == tab.id).expect("the tab is kept");
        assert_eq!(kept.holder_pid, tab.holder_pid, "the same holder");
        assert_eq!(kept.session.as_deref(), Some(id.value.as_str()), "still the session's");
        assert!(holder_answers(kept), "tab {} still runs", tab.id);
    }
    fixture.end(&tabs).await;
}

/// The red choice: ending them is the owner's explicit pick.
#[tokio::test(flavor = "multi_thread")]
async fn ending_them_is_a_choice_and_ends_the_holders() {
    let fixture = Fixture::new().await;
    let (id, tabs, _) = fixture.session_with_shell("Ledger").await;

    fixture.delete(Some(true)).await;

    until_gone(&tabs).await;
    assert!(!fixture.listed().await);
    assert!(
        fixture.store().session(&id.value).unwrap().is_none(),
        "an ended session goes with its channel"
    );
}

/// An agent can't end running terminals without the destroy switch, and the
/// refusal leaves everything as it was.
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_asking_to_end_them_is_refused_and_nothing_ends() {
    let fixture = Fixture::new().await;
    let (_, tabs, token) = fixture.session_with_shell("Ledger").await;
    let plan = fixture.plan().await;

    let refused = fixture
        .daemon
        .channel_service()
        .await
        .delete_channel(as_agent(Fixture::request(&plan, Some(true)), &token))
        .await
        .unwrap_err();

    assert_eq!(refused.code(), Code::PermissionDenied, "{refused:?}");
    assert!(fixture.listed().await, "the channel stays");
    assert!(
        stored_tabs(&state_folder(fixture.daemon.dir.path())).iter().filter(|tab| tabs.iter().any(|t| t.id == tab.id)).all(holder_answers),
        "nothing ended"
    );
    fixture.end(&tabs).await;
}

/// A session whose folder is the channel's own can't move: the plan names it
/// with a reason, and the delete leaves its terminals running with no
/// session instead of ending them.
#[tokio::test(flavor = "multi_thread")]
async fn a_session_that_cannot_move_is_named_and_its_terminals_run_on_without_it() {
    let fixture = Fixture::new().await;
    let (id, tabs, _) = fixture.session_with_shell("Shared").await;
    // The session shares the channel's folder instead of having its own.
    let store = fixture.store();
    let mut record = store.session(&id.value).unwrap().expect("recorded");
    record.folder = fixture.world.channels.join(&fixture.channel.folder);
    store.put_session(&record).unwrap();

    let plan = fixture.plan().await;
    assert!(plan.moves_to_desk.is_empty(), "{plan:?}");
    let named: Vec<_> = plan.cannot_move.iter().map(|stays| stays.session.clone()).collect();
    assert_eq!(named, vec![Some(id.clone())], "{plan:?}");
    assert!(!plan.cannot_move[0].reason.is_empty(), "it says why");
    fixture.delete(None).await;

    assert!(!fixture.listed().await);
    let now = stored_tabs(&state_folder(fixture.daemon.dir.path()));
    for tab in &tabs {
        let kept = now.iter().find(|found| found.id == tab.id).expect("the tab is kept");
        assert_eq!(kept.holder_pid, tab.holder_pid);
        assert!(kept.session.is_none(), "it belongs to no session now");
        assert!(holder_answers(kept), "tab {} still runs", tab.id);
    }
    fixture.end(&tabs).await;
}

/// What the session keeps of its own place follows it to the Desk: a note
/// in its `.humata` that named its folder in the channel names the new one.
#[tokio::test(flavor = "multi_thread")]
async fn a_session_moved_to_the_desk_has_its_notes_name_its_new_folder() {
    let fixture = Fixture::new().await;
    let (id, tabs, _) = fixture.session_with_shell("Notes").await;
    let before = fixture.store().session(&id.value).unwrap().expect("the session is recorded");
    let notes = before.folder.join(".humata");
    std::fs::create_dir_all(&notes).unwrap();
    std::fs::write(notes.join("note.txt"), format!("work in {}/api\n", before.folder.display())).unwrap();

    fixture.delete(None).await;

    let after = fixture.store().session(&id.value).unwrap().expect("the session is kept");
    let note = std::fs::read_to_string(after.folder.join(".humata").join("note.txt")).unwrap();
    assert!(note.contains(&format!("{}/api", after.folder.display())), "{note}");
    assert!(!note.contains(&before.folder.display().to_string()), "{note}");
    fixture.end(&tabs).await;
}

/// The destroy switch, off by default, lets an agent delete its channel:
/// its sessions still go to the Desk with their terminals running.
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_with_the_destroy_switch_on_deletes_the_channel_and_the_terminals_run_on() {
    let fixture = Fixture::new().await;
    let (_, tabs, token) = fixture.session_with_shell("Ledger").await;
    let plan = fixture.plan().await;
    let refused = fixture
        .daemon
        .channel_service()
        .await
        .delete_channel(as_agent(Fixture::request(&plan, None), &token))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::PermissionDenied, "off by default: {refused:?}");

    pb::access_service_client::AccessServiceClient::new(fixture.daemon.channel().await)
        .set_channel_rule(pb::SetChannelRuleRequest {
            channel: fixture.channel.id.clone(),
            capability: pb::Capability::MachineDestroy as i32,
            decision: pb::Decision::Allow as i32,
            ..Default::default()
        })
        .await
        .expect("the user turns the switch on");
    let plan = fixture.plan().await;
    let deleted = fixture
        .daemon
        .channel_service()
        .await
        .delete_channel(as_agent(Fixture::request(&plan, None), &token))
        .await
        .expect("an agent deletes with the switch on")
        .into_inner();
    let operation = finished(&fixture.daemon, deleted.operation.as_ref().unwrap()).await;
    assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{operation:?}");

    assert!(!fixture.listed().await, "the channel is gone");
    let now = stored_tabs(&state_folder(fixture.daemon.dir.path()));
    for tab in &tabs {
        let kept = now.iter().find(|found| found.id == tab.id).expect("the tab is kept");
        assert_eq!(kept.holder_pid, tab.holder_pid, "the same holder");
        assert!(holder_answers(kept), "tab {} still runs", tab.id);
    }
    fixture.end(&tabs).await;
}

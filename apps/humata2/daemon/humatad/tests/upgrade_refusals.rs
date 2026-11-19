//! UP-1, D281 c: nothing runs across releases. A machine at another release
//! than the host is linked in limited mode: Hello, reads and the upgrade
//! itself go through; starting, typing, a writable attach (and writes and
//! moves) are refused with `migration_needed` naming the Migrate item. A
//! CLI that names another release in `humata-bundle`, or an agent's CLI
//! that names none, is refused everything but ServerInfo.

use humatad_test_common as common;
use humatad_test_common::remote_rig;
use humatad_test_common::upgrade;
use humatad_test_common::upgrade_rig;

use common::{Attach, Daemon, agent_tab, error_of, folder, open, shell_token, start_request, state_folder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{SessionRecord, Store};
use remote_rig::Rig;
use tonic::Status;
use upgrade_rig::{BOX, box_id};

/// A session started on `gpu` while it was at the host's release, then
/// `gpu` at another release, with the Migrate item that says so.
struct Split {
    rig: Rig,
    channel: pb::Channel,
    session: pb::SessionId,
    tab: pb::TabId,
    item: pb::InboxRequest,
}

impl Split {
    async fn start() -> Self {
        let (mut rig, _) = upgrade_rig::at_one_release().await;
        let channel = rig.create_channel("Payments", &[("acme", "api")], &[box_id()]).await;
        let session = rig.start_on(&channel, &box_id(), "On gpu").await;
        let tab = agent_tab(&rig.host, &session).await;
        // Its agent is up before the machine changes release, so the
        // read-only attach later has something to show.
        let mut watch = Attach::start(&rig.host, read_only(&tab, &session)).await;
        watch.wait_for("agent up").await;
        drop(watch);
        upgrade_rig::box_at_release(&mut rig, "b").await;
        let item = upgrade_rig::daemon_item(&rig).await;
        Self { rig, channel, session, tab, item }
    }

    fn channel_id(&self) -> &pb::ChannelId {
        self.channel.id.as_ref().expect("a channel has an id")
    }

    /// What a refusal of work on `gpu` must say: the daemon there needs
    /// migrating, and the item that asks for it.
    fn assert_names_the_item(&self, needed: &pb::MigrationNeeded) {
        assert_eq!(needed.thing(), pb::MigrateThing::Daemon, "{needed:?}");
        assert_eq!(needed.machine, Some(box_id()), "{needed:?}");
        assert_eq!(needed.request, self.item.id, "{needed:?}");
    }
}

/// A writable attach from the host to `tab` on gpu: naming the session
/// is what makes the host relay it there.
fn remote(tab: &pb::TabId, session: &pb::SessionId) -> pb::AttachOpen {
    pb::AttachOpen { session: Some(session.clone()), ..open(tab, 0) }
}

fn read_only(tab: &pb::TabId, session: &pb::SessionId) -> pb::AttachOpen {
    pb::AttachOpen { read_only: true, ..remote(tab, session) }
}

/// The refusal a call ended in, whether the call failed or its operation
/// did.
async fn refusal_of(rig: &Rig, started: Result<pb::StartSessionResponse, Status>) -> pb::Error {
    match started {
        Err(status) => error_of(&status),
        Ok(response) => {
            let operation = response.operation.expect("a start has an operation");
            let ended = rig.ended(&operation).await;
            assert_eq!(ended.state(), pb::OperationState::Failed, "the start went through: {ended:?}");
            ended.error.expect("a failed operation says why")
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn starting_a_session_on_a_machine_at_another_release_is_refused_with_its_item() {
    let split = Split::start().await;
    let tabs_before = split.rig.the(BOX).tabs().len();

    let started = split
        .rig
        .host
        .session_service()
        .await
        .start_session(start_request(split.channel_id(), &box_id(), "Another"))
        .await
        .map(tonic::Response::into_inner);
    let refused = refusal_of(&split.rig, started).await;

    split.assert_names_the_item(&upgrade::migration_needed(&refused));
    assert_eq!(split.rig.the(BOX).tabs().len(), tabs_before, "nothing started on {BOX}");
}

#[tokio::test(flavor = "multi_thread")]
async fn typing_into_a_session_on_a_machine_at_another_release_is_refused_with_its_item() {
    let split = Split::start().await;

    let typed = split
        .rig
        .host
        .terminals()
        .await
        .r#type(pb::TypeRequest {
            idempotency_key: Some(pb::IdempotencyKey { value: "type-across".to_owned() }),
            session: Some(split.session.clone()),
            text: "hello\r".to_owned(),
            when: pb::type_request::When::Now as i32,
            ..Default::default()
        })
        .await;

    let refused = typed.expect_err("typing across releases is refused");
    split.assert_names_the_item(&upgrade::migration_needed(&error_of(&refused)));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_writable_attach_to_a_machine_at_another_release_is_refused_with_its_item() {
    let split = Split::start().await;

    let refused = match Attach::try_start(&split.rig.host, remote(&split.tab, &split.session)).await {
        Err(status) => status,
        // Opened: the first keystroke ends it.
        Ok(mut attach) => {
            attach.type_text("hello\r").await;
            attach.wait_for_error().await
        }
    };

    split.assert_names_the_item(&upgrade::migration_needed(&error_of(&refused)));
}

/// A session on this Mac at the host's release, then `gpu` at another
/// release: moving the session to `gpu` is refused with its Migrate item,
/// and nothing moves. The host is the same-channel machine at the daemon's
/// release, so no second rig machine is needed.
#[tokio::test(flavor = "multi_thread")]
async fn moving_a_session_to_a_machine_at_another_release_is_refused_with_its_item() {
    let (mut rig, _) = upgrade_rig::at_one_release().await;
    let host = rig.host_id().await;
    let channel = rig.create_channel("Payments", &[("acme", "api")], &[host.clone(), box_id()]).await;
    let session = rig.start_on(&channel, &host, "On this Mac").await;
    let tab = agent_tab(&rig.host, &session).await;
    upgrade_rig::box_at_release(&mut rig, "b").await;
    let item = upgrade_rig::daemon_item(&rig).await;
    let split = Split { rig, channel, session, tab, item };
    let machine_before = split.rig.session(&split.session).await.machine;

    let moved = split
        .rig
        .host
        .session_service()
        .await
        .move_session(pb::MoveSessionRequest {
            idempotency_key: Some(pb::IdempotencyKey { value: "move-across".to_owned() }),
            id: Some(split.session.clone()),
            to: Some(box_id()),
        })
        .await;

    let refused = moved.expect_err("moving across releases is refused");
    split.assert_names_the_item(&upgrade::migration_needed(&error_of(&refused)));
    let operations = split
        .rig
        .host
        .operation_service()
        .await
        .list_operations(pb::ListOperationsRequest::default())
        .await
        .expect("the host lists its operations")
        .into_inner();
    assert!(
        !operations.operations.iter().any(|op| op.kind == pb::OperationKind::MoveSession as i32),
        "no move was started: {operations:?}"
    );
    assert_eq!(
        split.rig.session(&split.session).await.machine,
        machine_before,
        "the session stayed where it was"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_machine_at_another_release_can_still_be_read() {
    let split = Split::start().await;
    let rig = &split.rig;

    let session = rig.session(&split.session).await;
    assert_eq!(session.id.as_ref(), Some(&split.session));
    let tabs = rig
        .host
        .terminals()
        .await
        .list_tabs(pb::ListTabsRequest { session: Some(split.session.clone()) })
        .await
        .expect("the session's tabs can be listed")
        .into_inner()
        .tabs;
    assert!(tabs.iter().any(|tab| tab.id.as_ref() == Some(&split.tab)), "{tabs:?}");
    let machines = pb::machine_service_client::MachineServiceClient::new(rig.channel().await)
        .list_machines(pb::ListMachinesRequest::default())
        .await
        .expect("the host lists its machines")
        .into_inner()
        .machines;
    assert!(machines.iter().any(|machine| machine.name == BOX), "{machines:?}");
    let mut watch = Attach::start(&rig.host, read_only(&split.tab, &split.session)).await;
    watch.wait_for("agent up").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn writing_a_file_to_a_machine_at_another_release_is_refused_with_its_item() {
    use pb::write_file_message::Message as Part;

    let split = Split::start().await;
    let header = pb::WriteFileHeader {
        location: Some(pb::FileLocation {
            machine: Some(box_id()),
            root: Some(pb::file_location::Root::ChannelShared(pb::ChannelShared {
                channel: split.channel.id.clone(),
            })),
            path: "across.txt".to_owned(),
        }),
        size: 5,
        ..Default::default()
    };
    let messages = vec![pb::WriteFileMessage { message: Some(Part::Header(header)) }];

    let written = pb::file_service_client::FileServiceClient::new(split.rig.channel().await)
        .write_file(tokio_stream::iter(messages))
        .await;

    let refused = written.expect_err("writing across releases is refused");
    split.assert_names_the_item(&upgrade::migration_needed(&error_of(&refused)));
    assert!(
        !split.rig.the(BOX).path(&split.channel, "across.txt").exists(),
        "nothing was written on {BOX}"
    );
}

// ---------------------------------------------------------------------------
// The CLI's release, named in `humata-bundle`.

const SESSION: &str = "se_upgrade";

/// The admin daemon at a release, with a session an agent's shell runs in.
struct AtRelease {
    daemon: Daemon,
    bundle: String,
    token: String,
    _shell: Attach,
}

impl AtRelease {
    async fn start() -> Self {
        let dir = folder();
        let bundle = upgrade::install(&dir.path().join("bin"), &upgrade::manifest(&upgrade::admin_sha(), "a"));
        let store = Store::open(state_folder(dir.path()).join("store")).expect("the store opens");
        store
            .put_session(&SessionRecord {
                id: SESSION.to_owned(),
                channel: "ch_upgrade".to_owned(),
                machine: String::new(),
                title: SESSION.to_owned(),
                agent: "zsh".to_owned(),
                folder: dir.path().to_owned(),
                created_unix_ms: 0,
            })
            .expect("the session is seeded");
        drop(store);
        let daemon = Daemon::start_in(dir).await;
        let (token, shell) = shell_token(&daemon, &pb::SessionId { value: SESSION.to_owned() }).await;
        Self { daemon, bundle, token, _shell: shell }
    }

    async fn list_tabs(&self, headers: &[(&'static str, &str)]) -> Result<pb::ListTabsResponse, Status> {
        self.daemon
            .terminals()
            .await
            .list_tabs(upgrade::with_headers(pb::ListTabsRequest::default(), headers))
            .await
            .map(tonic::Response::into_inner)
    }

    async fn who_am_i(&self, headers: &[(&'static str, &str)]) -> Result<pb::WhoAmIResponse, Status> {
        pb::daemon_service_client::DaemonServiceClient::new(self.daemon.channel().await)
            .who_am_i(upgrade::with_headers(pb::WhoAmIRequest {}, headers))
            .await
            .map(tonic::Response::into_inner)
    }

    async fn server_info(&self, headers: &[(&'static str, &str)]) -> Result<pb::ServerInfo, Status> {
        pb::daemon_service_client::DaemonServiceClient::new(self.daemon.channel().await)
            .get_server_info(upgrade::with_headers(pb::GetServerInfoRequest {}, headers))
            .await
            .map(tonic::Response::into_inner)
    }

    /// A refusal for the CLI's release on this daemon's machine.
    async fn assert_cli_refused(&self, refused: &Status) {
        let needed = upgrade::migration_needed(&error_of(refused));
        assert_eq!(needed.thing(), pb::MigrateThing::Cli, "{needed:?}");
        assert_eq!(needed.machine, Some(common::machine_id(&self.daemon).await), "{needed:?}");
    }
}

/// Another release than the daemon's.
fn other_release() -> String {
    upgrade::bundle_id(&upgrade::manifest(&upgrade::admin_sha(), "b"))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cli_at_another_release_is_refused_with_migration_needed() {
    let at = AtRelease::start().await;
    let other = other_release();

    let refused = at
        .list_tabs(&[(upgrade::BUNDLE_HEADER, other.as_str())])
        .await
        .expect_err("a CLI at another release is refused");

    at.assert_cli_refused(&refused).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cli_at_another_release_still_gets_server_info() {
    let at = AtRelease::start().await;
    let other = other_release();

    let info = at
        .server_info(&[(upgrade::BUNDLE_HEADER, other.as_str())])
        .await
        .expect("ServerInfo answers whatever the CLI's release");

    let build = info.build.expect("ServerInfo carries the daemon's build");
    assert_eq!(build.bundle, at.bundle, "the CLI learns which release to be");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cli_at_the_daemons_release_is_served() {
    let at = AtRelease::start().await;

    let tabs = at.list_tabs(&[(upgrade::BUNDLE_HEADER, at.bundle.as_str())]).await;

    assert!(tabs.is_ok(), "{tabs:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_with_no_credential_and_no_release_is_not_a_cli_and_is_served() {
    // The app and the tests' admin calls name no release and carry no CLI
    // credential; only a CLI is held to the daemon's release.
    let at = AtRelease::start().await;

    let tabs = at.list_tabs(&[]).await;

    assert!(tabs.is_ok(), "{tabs:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_cli_that_names_no_release_is_refused() {
    let at = AtRelease::start().await;

    let refused = at
        .who_am_i(&[("humata-session-token", at.token.as_str())])
        .await
        .expect_err("a CLI with an agent's token and no release is refused");

    at.assert_cli_refused(&refused).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_cli_at_the_daemons_release_is_served_as_the_agent() {
    let at = AtRelease::start().await;

    let me = at
        .who_am_i(&[("humata-session-token", at.token.as_str()), (upgrade::BUNDLE_HEADER, at.bundle.as_str())])
        .await
        .expect("a CLI at the daemon's release is served");

    assert!(matches!(me.actor.and_then(|actor| actor.actor), Some(pb::actor::Actor::Agent(_))));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_cli_that_names_no_release_still_gets_server_info() {
    let at = AtRelease::start().await;

    let info = at.server_info(&[("humata-session-token", at.token.as_str())]).await;

    assert_eq!(info.ok().and_then(|info| info.build).map(|build| build.bundle), Some(at.bundle.clone()));
}

//! UP-1, D281 f, g: `DaemonService.Upgrade` moves this Mac's daemon to a
//! staged release in place. With the release's files staged in `bin` as
//! `.<name>.next` (and `.bundle.json.next`, whose sha256 is the release's
//! id), Upgrade checks them, records the upgrade, swaps them in, answers,
//! and the daemon starts itself again from the new file: the same process,
//! the same holders and sessions, now at the new release. Files that are
//! missing or aren't that release are refused with nothing changed, and
//! only the user may ask.
//!
//! The daemon restarts itself, so the waits here ask the socket until the
//! new run answers with what only it can say (its new bundle id); a call
//! that fails while the daemon is between runs is asked again.

use humatad_test_common as common;
use humatad_test_common::upgrade;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{
    Attach, Build, Daemon, World, as_script, binary, create_channel, create_script_token, error_of, folder,
    get_session, holder_answers, is_running, open, shell_token, start_session, stored_tabs, wait_running,
};
use humata2_grpc::humata::v1 as pb;
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;
use tonic::metadata::MetadataValue;
use tonic::transport::{Channel, Endpoint, Uri};
use tonic::{Code, Request, Status};

/// How long a daemon may take to start itself again on a loaded machine: a
/// safety net, never a time anything is expected to take.
const LONG: Duration = Duration::from_secs(90);

const TOKEN_HEADER: &str = "humata-session-token";

/// Where the staged and the installed files are, as the daemon names them.
const FILES: [(&str, &str); 5] = [
    ("humatad2", "HUMATAD2"),
    ("humata2_holder", "HUMATA2_HOLDER"),
    ("humata", "HUMATA"),
    ("stack", "STACK"),
    ("humata2_fusefs", "HUMATA2_FUSEFS"),
];

/// A host daemon at release `a`, its release installed in `bin` beside its
/// state, with a channel holding one running agent session.
struct Mac {
    daemon: Daemon,
    bin: PathBuf,
    /// The bundle id of release `a`, the one the daemon runs.
    from: String,
    session: pb::SessionId,
}

/// Copies the test binary `variable` names to `to`, as an install does.
fn put_file(variable: &str, to: &Path) {
    std::fs::copy(binary(variable), to).unwrap_or_else(|error| panic!("couldn't copy {variable} to {}: {error}", to.display()));
    std::fs::set_permissions(to, std::fs::Permissions::from_mode(0o755)).expect("the file is made runnable");
}

/// The files of release `a` installed in `bin`, with its `bundle.json`.
fn install_release_a(bin: &Path) -> String {
    std::fs::create_dir_all(bin).expect("the bin folder is made");
    for (name, variable) in FILES {
        put_file(variable, &bin.join(name));
    }
    upgrade::install(bin, &upgrade::manifest(&upgrade::admin_sha(), "a"))
}

/// Stages release `b` in `bin`; returns its bundle id.
fn stage_release_b(bin: &Path) -> String {
    for (name, variable) in FILES {
        put_file(variable, &bin.join(format!(".{name}.next")));
    }
    let bundle = upgrade::manifest(&upgrade::admin_sha(), "b");
    let staged = bin.join(".bundle.json.next");
    std::fs::write(&staged, &bundle).expect("the staged bundle is written");
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o644)).expect("the bundle is readable");
    upgrade::bundle_id(&bundle)
}

impl Mac {
    async fn start() -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        world.remote("acme", "api");
        let bin = dir.path().join("bin");
        let from = install_release_a(&bin);
        let args = world.args();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with(Build::Admin, dir, &refs).await;
        assert_eq!(upgrade::build_of(&daemon).await.bundle, from, "the daemon runs release a");
        let (channel, setup) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        assert_eq!(setup.state, pb::OperationState::Succeeded as i32, "{setup:?}");
        let (session, _) = start_session(&daemon, &channel, "Fix login").await;
        wait_running(&daemon, &session).await;
        let tab = common::agent_tab(&daemon, &session).await;
        // The agent's program runs, not only its holder.
        Attach::start(&daemon, open(&tab, 0)).await.wait_for("agent up").await;
        Self {
            daemon,
            bin,
            from,
            session,
        }
    }

    async fn start_manual() -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        world.remote("acme", "api");
        let bin = dir.path().join("bin");
        let from = install_release_a(&bin);
        let args = world.args();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with_env(Build::Admin, dir, &refs, &[("HUMATA2_AUTO_MIGRATE", "0")]).await;
        assert_eq!(upgrade::build_of(&daemon).await.bundle, from, "the daemon runs release a");
        let (channel, setup) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        assert_eq!(setup.state, pb::OperationState::Succeeded as i32, "{setup:?}");
        let (session, _) = start_session(&daemon, &channel, "Fix login").await;
        wait_running(&daemon, &session).await;
        let tab = common::agent_tab(&daemon, &session).await;
        Attach::start(&daemon, open(&tab, 0)).await.wait_for("agent up").await;
        Self {
            daemon,
            bin,
            from,
            session,
        }
    }

    /// A daemon with nothing running in it: for the calls that are refused.
    async fn bare() -> Self {
        let dir = folder();
        let bin = dir.path().join("bin");
        let from = install_release_a(&bin);
        let daemon = Daemon::start_env(dir, &[("HUMATA2_AUTO_MIGRATE", "0")]).await;
        Self {
            daemon,
            bin,
            from,
            session: pb::SessionId::default(),
        }
    }

    fn upgrade_dir(&self) -> PathBuf {
        self.daemon.state.join("upgrade")
    }

    /// Nothing was swapped: release `a` is installed whole, nothing was
    /// recorded, and the daemon still runs release `a`.
    async fn assert_unchanged(&self) {
        assert_eq!(
            std::fs::read(self.bin.join("bundle.json")).expect("bundle.json is installed"),
            upgrade::manifest(&upgrade::admin_sha(), "a")
        );
        for (name, variable) in FILES {
            assert_eq!(
                upgrade::sha256_of(&self.bin.join(name)),
                upgrade::sha256_of(&binary(variable)),
                "{name} is still the installed one"
            );
        }
        assert!(!self.bin.join("humatad2.previous").exists(), "no old daemon was set aside");
        assert!(!self.upgrade_dir().join("this.json").exists(), "no upgrade was recorded");
        assert_eq!(upgrade::build_of(&self.daemon).await.bundle, self.from);
    }

    async fn upgrade(&self, to_bundle: &str) -> Result<pb::UpgradeResponse, Status> {
        self.upgrade_with(to_bundle, Request::new).await
    }

    async fn upgrade_with(
        &self,
        to_bundle: &str,
        wrap: impl FnOnce(pb::UpgradeRequest) -> Request<pb::UpgradeRequest>,
    ) -> Result<pb::UpgradeResponse, Status> {
        pb::daemon_service_client::DaemonServiceClient::new(self.daemon.channel().await)
            .upgrade(wrap(pb::UpgradeRequest {
                to_bundle: to_bundle.to_owned(),
                upgrade_id: "up_test".to_owned(),
                thing: pb::MigrateThing::Unspecified.into(),
            }))
            .await
            .map(tonic::Response::into_inner)
    }
}

/// GetServerInfo, or `None` while the daemon is between two runs of itself.
async fn try_info(daemon: &Daemon) -> Option<pb::ServerInfo> {
    let socket = daemon.socket.clone();
    let channel: Channel = Endpoint::try_from("http://[::]:50051")
        .ok()?
        .connect_with_connector(tower::service_fn(move |_: Uri| {
            let socket = socket.clone();
            async move { Ok::<_, std::io::Error>(TokioIo::new(UnixStream::connect(socket).await?)) }
        }))
        .await
        .ok()?;
    pb::daemon_service_client::DaemonServiceClient::new(channel)
        .get_server_info(pb::GetServerInfoRequest {})
        .await
        .ok()
        .map(tonic::Response::into_inner)
}

/// Waits until the daemon answers as release `bundle`.
async fn wait_at_release(daemon: &Daemon, bundle: &str) -> pb::BuildInfo {
    let deadline = Instant::now() + LONG;
    loop {
        if let Some(build) = try_info(daemon).await.and_then(|info| info.build).filter(|build| build.bundle == bundle) {
            return build;
        }
        assert!(Instant::now() < deadline, "the daemon never answered as release {bundle}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Waits until `path` exists.
async fn wait_for(path: &Path) {
    let deadline = Instant::now() + LONG;
    while !path.exists() {
        assert!(Instant::now() < deadline, "{} never appeared", path.display());
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The stored tabs' holders as (tab, process id, generation), in a stable
/// order: a holder that was replaced or revived shows as another.
fn holders(daemon: &Daemon) -> Vec<(String, u32, u64)> {
    let mut all: Vec<_> = stored_tabs(&daemon.state)
        .iter()
        .map(|tab| (tab.id.clone(), tab.holder_pid, tab.holder_generation))
        .collect();
    all.sort();
    all
}

#[tokio::test(flavor = "multi_thread")]
async fn an_upgrade_swaps_the_staged_release_in_and_restarts_the_daemon_with_its_holders_untouched() {
    let mac = Mac::start().await;
    let pid = mac.daemon.pid();
    let holders_before = holders(&mac.daemon);
    assert!(!holders_before.is_empty(), "the session has a holder");
    let to = stage_release_b(&mac.bin);
    assert_ne!(to, mac.from);

    // Auto-migrate detects `.bundle.json.next`, swaps the release in, and
    // restarts the daemon in place without requiring a manual Upgrade call.
    let build = wait_at_release(&mac.daemon, &to).await;
    assert_eq!(build.own_sha256, upgrade::admin_sha(), "the new run is the swapped-in file");
    assert_eq!(mac.daemon.pid(), pid, "the daemon started itself again in its own process");
    assert_eq!(holders(&mac.daemon), holders_before, "the same holders, the same processes");
    for tab in stored_tabs(&mac.daemon.state) {
        assert!(holder_answers(&tab), "holder {} still answers", tab.holder_pid);
    }
    assert!(is_running(&get_session(&mac.daemon, &mac.session).await), "the session kept running");
    let tabs = mac
        .daemon
        .terminals()
        .await
        .list_tabs(pb::ListTabsRequest {
            session: Some(mac.session.clone()),
        })
        .await
        .expect("the tabs list")
        .into_inner()
        .tabs;
    assert!(tabs.iter().any(|tab| tab.kind == pb::tab::Kind::Agent as i32), "{tabs:?}");

    // The release is whole on disk, and the old daemon is unlinked once settled.
    assert_eq!(
        std::fs::read(mac.bin.join("bundle.json")).expect("bundle.json is installed"),
        upgrade::manifest(&upgrade::admin_sha(), "b")
    );
    for (name, _) in FILES {
        assert!(!mac.bin.join(format!(".{name}.next")).exists(), "{name} is no longer staged");
    }
    assert!(!mac.bin.join(".bundle.json.next").exists());
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    while mac.bin.join("humatad2.previous").exists() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    assert!(!mac.bin.join("humatad2.previous").exists(), "the old daemon is unlinked after settle");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_upgrade_record_is_settled_into_last_json_once_the_new_daemon_is_ready() {
    let mac = Mac::bare().await;
    let to = stage_release_b(&mac.bin);

    mac.upgrade(&to).await.expect("the upgrade is taken");
    wait_at_release(&mac.daemon, &to).await;

    let last = mac.upgrade_dir().join("last.json");
    wait_for(&last).await;
    assert!(!mac.upgrade_dir().join("this.json").exists(), "nothing is left under way");
    let record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&last).expect("last.json reads")).expect("last.json is JSON");
    assert_eq!(record["to_bundle"], to.as_str());
    assert_eq!(record["from_bundle"], mac.from.as_str());
    assert_eq!(record["upgrade_id"], "up_test");
}

#[tokio::test(flavor = "multi_thread")]
async fn files_that_are_not_staged_are_refused_with_nothing_changed() {
    let mac = Mac::bare().await;
    let wanted = upgrade::bundle_id(&upgrade::manifest(&upgrade::admin_sha(), "b"));

    let refused = mac.upgrade(&wanted).await.expect_err("nothing is staged");

    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");
    assert!(!error_of(&refused).message.is_empty());
    mac.assert_unchanged().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_staged_file_that_is_not_the_one_the_bundle_names_is_refused_with_nothing_changed() {
    let mac = Mac::bare().await;
    let to = stage_release_b(&mac.bin);
    std::fs::write(mac.bin.join(".humata2_holder.next"), b"somebody else's holder").expect("the file is replaced");

    let refused = mac.upgrade(&to).await.expect_err("the staged holder is wrong");

    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");
    mac.assert_unchanged().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_release_that_is_not_the_staged_one_is_refused_with_nothing_changed() {
    let mac = Mac::bare().await;
    stage_release_b(&mac.bin);
    let another = upgrade::bundle_id(&upgrade::manifest(&upgrade::admin_sha(), "c"));

    let refused = mac.upgrade(&another).await.expect_err("release c isn't staged");

    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");
    mac.assert_unchanged().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_upgrade_to_the_release_the_daemon_already_runs_changes_nothing() {
    let mac = Mac::bare().await;

    let reply = mac.upgrade(&mac.from).await.expect("nothing to do is not a failure");

    assert_eq!(reply.from.expect("the reply says what the daemon is").bundle, mac.from);
    mac.assert_unchanged().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_or_a_script_cannot_upgrade_the_daemon() {
    let mac = Mac::start_manual().await;
    // Before the release is staged: a staged release posts this Mac's
    // Migrate item, and the gate then refuses a shell and a writable attach
    // until it is migrated (D370).
    let (token, _shell) = shell_token(&mac.daemon, &mac.session).await;
    let to = stage_release_b(&mac.bin);
    let (_, script) = create_script_token(&mac.daemon, "ci", &[pb::Capability::GlobalRead], &[], 1)
        .await
        .expect("the user makes a script token");

    let by_agent = mac
        .upgrade_with(&to, |message| {
            let mut request = Request::new(message);
            request
                .metadata_mut()
                .insert(TOKEN_HEADER, MetadataValue::try_from(token.as_str()).expect("a header value"));
            request
        })
        .await
        .expect_err("an agent can't upgrade the daemon");
    let by_script = mac
        .upgrade_with(&to, |message| as_script(message, &script))
        .await
        .expect_err("a script can't upgrade the daemon");

    for refused in [by_agent, by_script] {
        assert_eq!(refused.code(), Code::PermissionDenied, "{refused:?}");
        assert!(matches!(error_of(&refused).kind, Some(pb::error::Kind::UserOnly(_))), "{refused:?}");
    }
    assert!(mac.bin.join(".humatad2.next").exists(), "the staged release is left as it was");
    assert!(!mac.upgrade_dir().join("this.json").exists());
    assert_eq!(upgrade::build_of(&mac.daemon).await.bundle, mac.from);
}

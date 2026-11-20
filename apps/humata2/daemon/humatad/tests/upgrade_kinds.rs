//! UP-1, D281 b, j, D370, D371: the Migrate items other than "a machine
//! whose Hello names another release".
//!
//! - **This Mac's daemon (D370).** The app stages a newer release beside the
//!   running daemon as `.<name>.next` and `.bundle.json.next`. The daemon
//!   sees it and posts one VIP "Migrate the daemon" item for its own
//!   machine. Approving it moves the daemon to the staged release in place,
//!   and taking the staged files away withdraws the item.
//! - **The CLI and the host tools (D371).** `<bin>/humata` and
//!   `<bin>/humata2_holder` are checked against the daemon's own release. A
//!   file that differs is a CLI or a HOST_TOOLS item, and Migrate puts the
//!   release's file back with no restart. A machine reports what it has
//!   installed in its build, so the host posts the same items for it.
//!   While a daemon item waits, a differing tool is a line of that item,
//!   not an item of its own.
//!
//! Every wait ends on a pushed Inbox change. The one exception is a daemon
//! between two runs of itself, which is asked again until it answers.
//! Something that must not happen is checked after a later thing that must
//! happen has happened, never after a pause.

use humatad_test_common as common;
use humatad_test_common::remote_rig;
use humatad_test_common::upgrade;
use humatad_test_common::upgrade_rig;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{
    Attach, Build, Daemon, World, binary, create_channel, folder, get_session, holder_answers, is_running, open,
    start_session, stored_tabs, wait_running,
};
use humata2_grpc::humata::v1 as pb;
use humata2_machines::{InstalledFile, Payload, Platform, StagedRelease};
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;
use tonic::transport::{Channel, Endpoint, Uri};
use upgrade_rig::BOX;

/// These tests click Migrate themselves: with this Mac's auto-migrate on,
/// the daemon would take the item first and restart under the test.
const MANUAL: &[(&str, &str)] = &[("HUMATA2_AUTO_MIGRATE", "0")];

/// How long a daemon may take to start itself again on a loaded machine: a
/// safety net, never a time anything is expected to take.
const LONG: Duration = Duration::from_secs(90);

/// A release's five files: the installed name, the test binary that is
/// this platform's file, and the file as the machines crate names it.
const FILES: [(&str, &str, InstalledFile); 5] = [
    ("humatad2", "HUMATAD2", InstalledFile::Humatad),
    ("humata2_holder", "HUMATA2_HOLDER", InstalledFile::Holder),
    ("humata", "HUMATA", InstalledFile::Cli),
    ("stack", "STACK", InstalledFile::Stack),
    ("humata2_fusefs", "HUMATA2_FUSEFS", InstalledFile::FuseFs),
];

// ---------------------------------------------------------------------------
// Files in a bin folder.

/// Copies the test binary `variable` names to `to`, as an install does.
fn put_file(variable: &str, to: &Path) {
    std::fs::copy(binary(variable), to)
        .unwrap_or_else(|error| panic!("couldn't copy {variable} to {}: {error}", to.display()));
    std::fs::set_permissions(to, std::fs::Permissions::from_mode(0o755)).expect("the file is made runnable");
}

/// This platform's files installed in `bin` with no `bundle.json`.
fn install_files(bin: &Path) {
    std::fs::create_dir_all(bin).expect("the bin folder is made");
    for (name, variable, _) in FILES {
        put_file(variable, &bin.join(name));
    }
}

/// Release `release` installed in `bin`, its files and its `bundle.json`;
/// returns its bundle id.
fn install_release(bin: &Path, release: &str) -> String {
    install_files(bin);
    upgrade::install(bin, &upgrade::manifest(&upgrade::admin_sha(), release))
}

/// Stages release `release` in `bin` as the app does: each file whole as
/// `.<name>.next`, then `.bundle.json.next` last, so the bundle is never
/// there without the files it names. Returns its bundle id.
fn stage(bin: &Path, release: &str) -> String {
    let platform = Platform::host().expect("the tests run on a platform Humata ships");
    let files = FILES
        .iter()
        .map(|(_, variable, file)| {
            let bytes = std::fs::read(binary(variable))
                .unwrap_or_else(|error| panic!("couldn't read {variable} to stage it: {error}"));
            Payload::new(*file, Arc::from(bytes))
        })
        .collect();
    let staged = StagedRelease::new(platform, upgrade::manifest(&upgrade::admin_sha(), release), files)
        .expect("the test's release names this platform's files");
    humata2_machines::stage_next(bin, &staged).expect("the release is staged")
}

/// Takes the staged release out of `bin` without migrating: the bundle
/// first, so what is left is never a staged release.
fn unstage(bin: &Path) {
    std::fs::remove_file(bin.join(".bundle.json.next")).expect("the staged bundle is taken away");
    for (name, _, _) in FILES {
        let next = bin.join(format!(".{name}.next"));
        if next.exists() {
            std::fs::remove_file(&next).unwrap_or_else(|error| panic!("couldn't remove {}: {error}", next.display()));
        }
    }
}

/// Puts `bytes` in as `bin/<name>` whole: written beside `bin` and renamed
/// in, so the daemon never reads a part of it.
fn swap_in(bin: &Path, name: &str, bytes: &[u8]) {
    let beside = bin
        .parent()
        .expect("a bin folder is in a folder of its own")
        .join(format!("{name}.swap"));
    std::fs::write(&beside, bytes).unwrap_or_else(|error| panic!("couldn't write {}: {error}", beside.display()));
    std::fs::set_permissions(&beside, std::fs::Permissions::from_mode(0o755)).expect("the file is made runnable");
    std::fs::rename(&beside, bin.join(name))
        .unwrap_or_else(|error| panic!("couldn't put {name} in {}: {error}", bin.display()));
}

/// A file named `name` that is no release's.
fn foreign(name: &str) -> Vec<u8> {
    format!("#!/bin/sh\n# a {name} from no release\nexit 3\n").into_bytes()
}

// ---------------------------------------------------------------------------
// The Inbox.

/// Every Migrate item for `thing` on `machine`, whatever its state.
fn migrates<'a>(
    items: &'a [pb::InboxRequest],
    thing: pb::MigrateThing,
    machine: &pb::MachineId,
) -> Vec<&'a pb::InboxRequest> {
    items
        .iter()
        .filter(|item| {
            item.kind() == pb::RequestKind::Migrate
                && upgrade::migrate_of(item)
                    .is_some_and(|card| card.thing() == thing && card.machine.as_ref() == Some(machine))
        })
        .collect()
}

/// How many pending Migrate items for `thing` on `machine` there are.
fn pending(items: &[pb::InboxRequest], thing: pb::MigrateThing, machine: &pb::MachineId) -> usize {
    items
        .iter()
        .filter(|item| upgrade::pending_migrate(item, thing, machine))
        .count()
}

fn the_id(item: &pb::InboxRequest) -> pb::RequestId {
    item.id.clone().expect("an Inbox item has an id")
}

fn card(item: &pb::InboxRequest) -> &pb::MigratePreview {
    upgrade::migrate_of(item).unwrap_or_else(|| panic!("not a Migrate item: {item:?}"))
}

/// The user clicks Migrate on `item`.
async fn approve(daemon: &Daemon, item: &pb::InboxRequest, key: &str) {
    daemon
        .inbox_service()
        .await
        .answer_request(pb::AnswerRequestRequest {
            idempotency_key: Some(pb::IdempotencyKey { value: key.to_owned() }),
            id: item.id.clone(),
            answer: Some(pb::RequestAnswer {
                answer: Some(pb::request_answer::Answer::Approve(pb::Approve {})),
            }),
            expected_revision: item.revision.clone(),
            ..Default::default()
        })
        .await
        .expect("the user's Migrate is taken");
}

/// Item `id` once it is no longer pending.
async fn ended(daemon: &Daemon, id: &pb::RequestId) -> pb::InboxRequest {
    upgrade::request_until(daemon, id, "ended", |request| request.state() != pb::RequestState::Pending).await
}

/// Waits until item `id` ends, and fails unless it was withdrawn as
/// resolved.
async fn assert_withdrawn_as_resolved(daemon: &Daemon, id: &pb::RequestId) {
    let done = ended(daemon, id).await;
    assert_eq!(done.state(), pb::RequestState::Withdrawn, "{done:?}");
    assert_eq!(done.withdraw_reason(), pb::WithdrawReason::Resolved, "{done:?}");
}

// ---------------------------------------------------------------------------
// The daemon between runs.

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

// ---------------------------------------------------------------------------
// This Mac.

/// A host daemon at release `a`, installed in `bin` beside its state.
struct Mac {
    daemon: Daemon,
    bin: PathBuf,
    /// Release `a`'s bundle id, the one the daemon runs.
    from: String,
    /// The daemon's own machine.
    machine: pb::MachineId,
    session: Option<pb::SessionId>,
}

impl Mac {
    /// With a channel holding one running agent session.
    async fn start() -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        world.remote("acme", "api");
        let bin = dir.path().join("bin");
        let from = install_release(&bin, "a");
        let args = world.args();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with_env(Build::Admin, dir, &refs, MANUAL).await;
        assert_eq!(upgrade::build_of(&daemon).await.bundle, from, "the daemon runs release a");
        let (channel, setup) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        assert_eq!(setup.state, pb::OperationState::Succeeded as i32, "{setup:?}");
        let (session, _) = start_session(&daemon, &channel, "Fix login").await;
        wait_running(&daemon, &session).await;
        let tab = common::agent_tab(&daemon, &session).await;
        // The agent's program runs, not only its holder.
        Attach::start(&daemon, open(&tab, 0)).await.wait_for("agent up").await;
        let machine = common::machine_id(&daemon).await;
        Self {
            daemon,
            bin,
            from,
            machine,
            session: Some(session),
        }
    }

    /// With nothing running in it.
    async fn bare() -> Self {
        let dir = folder();
        let bin = dir.path().join("bin");
        let from = install_release(&bin, "a");
        let daemon = Daemon::start_env(dir, MANUAL).await;
        assert_eq!(upgrade::build_of(&daemon).await.bundle, from, "the daemon runs release a");
        let machine = common::machine_id(&daemon).await;
        Self {
            daemon,
            bin,
            from,
            machine,
            session: None,
        }
    }

    async fn item(&self, thing: pb::MigrateThing) -> pb::InboxRequest {
        upgrade::migrate_item(&self.daemon, thing, &self.machine).await
    }

    fn upgrade_dir(&self) -> PathBuf {
        self.daemon.state.join("upgrade")
    }
}

// --- D370: This Mac's daemon -----------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_release_staged_beside_the_running_daemon_is_one_vip_daemon_item_for_its_own_machine() {
    let mac = Mac::bare().await;

    let to = stage(&mac.bin, "b");
    assert_ne!(to, mac.from);

    let item = mac.item(pb::MigrateThing::Daemon).await;
    assert!(item.vip, "{item:?}");
    assert_eq!(card(&item).to.as_ref().map(|to| to.bundle.as_str()), Some(to.as_str()));
    assert_eq!(card(&item).from.as_ref().map(|from| from.bundle.as_str()), Some(mac.from.as_str()));
    let items = upgrade::inbox(&mac.daemon).await;
    assert_eq!(pending(&items, pb::MigrateThing::Daemon, &mac.machine), 1, "{items:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_release_staged_before_the_daemon_starts_is_posted_once_the_daemon_is_ready() {
    let dir = folder();
    let bin = dir.path().join("bin");
    let from = install_release(&bin, "a");
    let to = stage(&bin, "b");

    let daemon = Daemon::start_env(dir, MANUAL).await;

    assert_eq!(upgrade::build_of(&daemon).await.bundle, from, "the daemon runs release a");
    let machine = common::machine_id(&daemon).await;
    let item = upgrade::migrate_item(&daemon, pb::MigrateThing::Daemon, &machine).await;
    assert_eq!(card(&item).to.as_ref().map(|to| to.bundle.as_str()), Some(to.as_str()));
}

#[tokio::test(flavor = "multi_thread")]
async fn staging_the_same_release_again_posts_no_second_item() {
    let mac = Mac::bare().await;
    stage(&mac.bin, "b");
    let first = mac.item(pb::MigrateThing::Daemon).await;

    stage(&mac.bin, "b");
    // Then another release, which the same item shows once the daemon has
    // seen it: by then it has seen the second staging of b too.
    let c = stage(&mac.bin, "c");
    let shown = upgrade::request_until(&mac.daemon, &the_id(&first), "showed release c", |request| {
        upgrade::migrate_of(request)
            .and_then(|card| card.to.as_ref())
            .is_some_and(|to| to.bundle == c)
    })
    .await;

    assert_eq!(shown.state(), pb::RequestState::Pending, "{shown:?}");
    let items = upgrade::inbox(&mac.daemon).await;
    let all = migrates(&items, pb::MigrateThing::Daemon, &mac.machine);
    assert_eq!(all.len(), 1, "one item, answered ones counted: {all:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn approving_the_item_moves_the_daemon_to_the_staged_release_with_its_holders_untouched() {
    let mac = Mac::start().await;
    let session = mac.session.clone().expect("the Mac runs a session");
    let pid = mac.daemon.pid();
    let holders_before = holders(&mac.daemon);
    assert!(!holders_before.is_empty(), "the session has a holder");
    let to = stage(&mac.bin, "b");
    let item = mac.item(pb::MigrateThing::Daemon).await;

    approve(&mac.daemon, &item, "migrate-this-mac").await;

    let build = wait_at_release(&mac.daemon, &to).await;
    assert_eq!(build.own_sha256, upgrade::admin_sha(), "the new run is the swapped-in file");
    assert_eq!(mac.daemon.pid(), pid, "the daemon started itself again in its own process");
    assert_eq!(holders(&mac.daemon), holders_before, "the same holders, the same processes");
    for tab in stored_tabs(&mac.daemon.state) {
        assert!(holder_answers(&tab), "holder {} still answers", tab.holder_pid);
    }
    assert!(is_running(&get_session(&mac.daemon, &session).await), "the session kept running");
    let done = ended(&mac.daemon, &the_id(&item)).await;
    assert_eq!(done.state(), pb::RequestState::Approved, "{done:?}");
    assert_eq!(
        std::fs::read(mac.bin.join("bundle.json")).expect("bundle.json is installed"),
        upgrade::manifest(&upgrade::admin_sha(), "b")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn taking_the_staged_release_away_withdraws_its_item_as_resolved() {
    let mac = Mac::bare().await;
    stage(&mac.bin, "b");
    let item = mac.item(pb::MigrateThing::Daemon).await;

    unstage(&mac.bin);

    assert_withdrawn_as_resolved(&mac.daemon, &the_id(&item)).await;
    assert_eq!(upgrade::build_of(&mac.daemon).await.bundle, mac.from, "nothing was migrated");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_daemon_that_names_no_release_posts_nothing_for_a_staged_one() {
    let dir = folder();
    let bin = dir.path().join("bin");
    install_files(&bin);
    let mut daemon = Daemon::start_env(dir, MANUAL).await;
    assert_eq!(upgrade::build_of(&daemon).await.bundle, "", "with no bundle.json the daemon names no release");
    let machine = common::machine_id(&daemon).await;

    stage(&bin, "b");
    // Starting again checks what is staged once it is ready, before it
    // answers; the start's report is the last thing it does.
    daemon.restart().await;
    common::last_start(&daemon).await;

    let items = upgrade::inbox(&daemon).await;
    for thing in [pb::MigrateThing::Daemon, pb::MigrateThing::Cli, pb::MigrateThing::HostTools] {
        let posted = migrates(&items, thing, &machine);
        assert!(posted.is_empty(), "{thing:?}: {posted:?}");
    }
}

// --- D371: the CLI and the host tools, on this Mac -------------------------

#[tokio::test(flavor = "multi_thread")]
async fn an_installed_cli_that_is_not_the_releases_is_one_cli_item_for_this_machine() {
    let mac = Mac::bare().await;

    swap_in(&mac.bin, "humata", &foreign("humata"));

    let item = mac.item(pb::MigrateThing::Cli).await;
    assert_eq!(card(&item).machine.as_ref(), Some(&mac.machine));
    let items = upgrade::inbox(&mac.daemon).await;
    assert_eq!(pending(&items, pb::MigrateThing::Cli, &mac.machine), 1, "{items:?}");
    assert_eq!(pending(&items, pb::MigrateThing::Daemon, &mac.machine), 0, "{items:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_installed_holder_that_is_not_the_releases_is_one_host_tools_item_for_this_machine() {
    let mac = Mac::bare().await;

    swap_in(&mac.bin, "humata2_holder", &foreign("humata2_holder"));

    let item = mac.item(pb::MigrateThing::HostTools).await;
    assert_eq!(card(&item).machine.as_ref(), Some(&mac.machine));
    let items = upgrade::inbox(&mac.daemon).await;
    assert_eq!(pending(&items, pb::MigrateThing::HostTools, &mac.machine), 1, "{items:?}");
    assert_eq!(pending(&items, pb::MigrateThing::Daemon, &mac.machine), 0, "{items:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_installed_fusefs_helper_that_is_not_the_releases_is_one_host_tools_item_for_this_machine() {
    let mac = Mac::bare().await;

    swap_in(&mac.bin, "humata2_fusefs", &foreign("humata2_fusefs"));

    let item = mac.item(pb::MigrateThing::HostTools).await;
    assert_eq!(card(&item).machine.as_ref(), Some(&mac.machine));
    let items = upgrade::inbox(&mac.daemon).await;
    assert_eq!(pending(&items, pb::MigrateThing::HostTools, &mac.machine), 1, "{items:?}");
    assert_eq!(pending(&items, pb::MigrateThing::Daemon, &mac.machine), 0, "{items:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn putting_the_releases_cli_back_withdraws_its_item_as_resolved() {
    let mac = Mac::bare().await;
    swap_in(&mac.bin, "humata", &foreign("humata"));
    let item = mac.item(pb::MigrateThing::Cli).await;

    swap_in(&mac.bin, "humata", &std::fs::read(binary("HUMATA")).expect("the CLI reads"));

    assert_withdrawn_as_resolved(&mac.daemon, &the_id(&item)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn putting_the_releases_holder_back_withdraws_its_item_as_resolved() {
    let mac = Mac::bare().await;
    swap_in(&mac.bin, "humata2_holder", &foreign("humata2_holder"));
    let item = mac.item(pb::MigrateThing::HostTools).await;

    swap_in(&mac.bin, "humata2_holder", &std::fs::read(binary("HUMATA2_HOLDER")).expect("the holder reads"));

    assert_withdrawn_as_resolved(&mac.daemon, &the_id(&item)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn approving_the_cli_item_puts_the_staged_cli_in_place_without_restarting_the_daemon() {
    let mac = Mac::bare().await;
    let pid = mac.daemon.pid();
    let started = common::last_start(&mac.daemon).await.started_at;
    swap_in(&mac.bin, "humata", &foreign("humata"));
    let item = mac.item(pb::MigrateThing::Cli).await;
    // The app stages the release the daemon already runs.
    assert_eq!(stage(&mac.bin, "a"), mac.from);

    approve(&mac.daemon, &item, "migrate-cli").await;

    let done = ended(&mac.daemon, &the_id(&item)).await;
    assert_eq!(done.state(), pb::RequestState::Approved, "{done:?}");
    assert_eq!(
        upgrade::sha256_of(&mac.bin.join("humata")),
        upgrade::sha256_of(&binary("HUMATA")),
        "the release's CLI is installed"
    );
    assert_eq!(
        upgrade::sha256_of(&mac.bin.join("humata2_holder")),
        upgrade::sha256_of(&binary("HUMATA2_HOLDER")),
        "the holder that matched is left as it was"
    );
    // No restart: the same process, the same start, the same release, and
    // no upgrade was recorded or old daemon set aside.
    assert_eq!(mac.daemon.pid(), pid);
    assert_eq!(common::last_start(&mac.daemon).await.started_at, started, "the daemon didn't start again");
    assert_eq!(upgrade::build_of(&mac.daemon).await.bundle, mac.from);
    assert!(!mac.bin.join("humatad2.previous").exists(), "no old daemon was set aside");
    assert!(!mac.upgrade_dir().join("this.json").exists(), "no upgrade was recorded");
    assert!(!mac.upgrade_dir().join("last.json").exists(), "no upgrade was settled");
}

#[tokio::test(flavor = "multi_thread")]
async fn approving_the_cli_item_with_nothing_staged_leaves_it_waiting_as_failed() {
    let mac = Mac::bare().await;
    let wrong = foreign("humata");
    swap_in(&mac.bin, "humata", &wrong);
    let item = mac.item(pb::MigrateThing::Cli).await;

    approve(&mac.daemon, &item, "migrate-cli").await;

    let failed = upgrade::request_until(&mac.daemon, &the_id(&item), "showed the Migrate failed", |request| {
        upgrade::migrate_of(request).is_some_and(|card| card.phase() == pb::MigratePhase::Failed)
    })
    .await;
    assert_eq!(failed.state(), pb::RequestState::Pending, "it can be migrated again: {failed:?}");
    assert_eq!(card(&failed).thing(), pb::MigrateThing::Cli);
    assert!(card(&failed).last_error.is_some(), "the card says what went wrong: {failed:?}");
    assert_eq!(std::fs::read(mac.bin.join("humata")).expect("humata is there"), wrong, "nothing was changed");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cli_that_differs_while_a_daemon_item_waits_is_a_line_of_that_item_not_an_item_of_its_own() {
    let mac = Mac::bare().await;
    stage(&mac.bin, "b");
    let daemon_item = mac.item(pb::MigrateThing::Daemon).await;
    let lines = card(&daemon_item).changes.len();

    swap_in(&mac.bin, "humata", &foreign("humata"));

    upgrade::request_until(&mac.daemon, &the_id(&daemon_item), "gained a line for the CLI", |request| {
        upgrade::migrate_of(request).is_some_and(|card| card.changes.len() == lines + 1)
    })
    .await;
    let items = upgrade::inbox(&mac.daemon).await;
    let posted = migrates(&items, pb::MigrateThing::Cli, &mac.machine);
    assert!(posted.is_empty(), "{posted:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_daemon_item_folds_in_the_cli_item_that_was_waiting() {
    let mac = Mac::bare().await;
    swap_in(&mac.bin, "humata", &foreign("humata"));
    let cli = mac.item(pb::MigrateThing::Cli).await;

    stage(&mac.bin, "b");

    let daemon_item = mac.item(pb::MigrateThing::Daemon).await;
    assert_withdrawn_as_resolved(&mac.daemon, &the_id(&cli)).await;
    let items = upgrade::inbox(&mac.daemon).await;
    assert_eq!(pending(&items, pb::MigrateThing::Cli, &mac.machine), 0, "{items:?}");
    assert_eq!(upgrade::request(&mac.daemon, &the_id(&daemon_item)).await.state(), pb::RequestState::Pending);
}

// --- D371: the CLI and the host tools, on a machine -------------------------

/// The host at the release its tools bundle names, and `gpu` at that same
/// release with its holder and CLI as that release has them, with nothing
/// left to migrate. Returns the rig, `gpu`'s bin folder, and the folder the
/// release's files for `gpu` come from.
async fn box_at_the_hosts_release() -> (remote_rig::Rig, PathBuf, PathBuf) {
    let mut rig = remote_rig::Rig::start_seen(&[BOX], &[("acme", "api")]).await;
    let release = upgrade_rig::host_at_tools_release(&mut rig).await;
    let tools = rig.host.dir.path().join("tools").join("linux-x86_64");
    let bin = upgrade::machine_bin(rig.the(BOX).home());
    for name in ["humata2_holder", "humata", "humata2_fusefs"] {
        let bytes = std::fs::read(tools.join(name)).unwrap_or_else(|error| panic!("the release's {name} reads: {error}"));
        swap_in(&bin, name, &bytes);
    }
    let bundle = std::fs::read(upgrade::bin_of(&rig.host).join("bundle.json")).expect("the host's bundle.json reads");
    assert_eq!(upgrade_rig::box_at(&mut rig, &bundle).await, release, "gpu names the host's release");
    upgrade::inbox_until(&rig.host, "nothing was left to migrate on gpu", |items| {
        (!items.iter().any(upgrade_rig::pending_on_box)).then_some(())
    })
    .await;
    (rig, bin, tools)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_machine_at_the_hosts_release_with_another_cli_is_one_cli_item_once_it_links() {
    let (mut rig, bin, _) = box_at_the_hosts_release().await;

    swap_in(&bin, "humata", &foreign("humata"));
    rig.restart_box_env(BOX, &[]).await;

    let item = upgrade::migrate_item(&rig.host, pb::MigrateThing::Cli, &upgrade_rig::box_id()).await;
    assert_eq!(card(&item).machine, Some(upgrade_rig::box_id()));
    let items = upgrade::inbox(&rig.host).await;
    assert_eq!(pending(&items, pb::MigrateThing::Cli, &upgrade_rig::box_id()), 1, "{items:?}");
    assert_eq!(pending(&items, pb::MigrateThing::Daemon, &upgrade_rig::box_id()), 0, "{items:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_machines_build_lists_the_tools_it_has_installed() {
    let (mut rig, bin, _) = box_at_the_hosts_release().await;
    // A CLI no release names, so what is listed is what is there, not what
    // the bundle says.
    swap_in(&bin, "humata", &foreign("humata"));
    rig.restart_box_env(BOX, &[]).await;

    let build = upgrade::build_of(rig.the(BOX).daemon()).await;

    let installed = |name: &str| {
        build
            .installed
            .iter()
            .find(|file| file.name == name)
            .map(|file| file.sha256.clone())
    };
    assert_eq!(installed("humata"), Some(upgrade::sha256_of(&bin.join("humata"))), "{build:?}");
    assert_eq!(
        installed("humata2_holder"),
        Some(upgrade::sha256_of(&bin.join("humata2_holder"))),
        "{build:?}"
    );
    let named = build.files.iter().find(|file| file.name == "humata").map(|file| file.sha256.clone());
    assert_ne!(installed("humata"), named, "the CLI installed isn't the one the bundle names");
}

#[tokio::test(flavor = "multi_thread")]
async fn approving_a_machines_cli_item_stages_over_ssh_and_puts_the_releases_cli_in_place() {
    let (mut rig, bin, tools) = box_at_the_hosts_release().await;
    swap_in(&bin, "humata", &foreign("humata"));
    rig.restart_box_env(BOX, &[]).await;
    let item = upgrade::migrate_item(&rig.host, pb::MigrateThing::Cli, &upgrade_rig::box_id()).await;

    approve(&rig.host, &item, "migrate-gpu-cli").await;

    let done = ended(&rig.host, &the_id(&item)).await;
    assert_eq!(done.state(), pb::RequestState::Approved, "{done:?}");
    assert_eq!(
        upgrade::sha256_of(&bin.join("humata")),
        upgrade::sha256_of(&tools.join("humata")),
        "gpu has the release's CLI"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn relinking_a_machine_whose_cli_was_put_back_withdraws_its_item_as_resolved() {
    let (mut rig, bin, tools) = box_at_the_hosts_release().await;
    swap_in(&bin, "humata", &foreign("humata"));
    rig.restart_box_env(BOX, &[]).await;
    let item = upgrade::migrate_item(&rig.host, pb::MigrateThing::Cli, &upgrade_rig::box_id()).await;

    swap_in(&bin, "humata", &std::fs::read(tools.join("humata")).expect("the release's CLI reads"));
    rig.restart_box_env(BOX, &[]).await;

    assert_withdrawn_as_resolved(&rig.host, &the_id(&item)).await;
}

// --- D379: the steps a Migrate card shows ----------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn migrating_this_macs_daemon_shows_five_steps_that_only_advance_and_all_end_done() {
    let mac = Mac::bare().await;
    let to = stage(&mac.bin, "b");
    let item = mac.item(pb::MigrateThing::Daemon).await;
    let id = the_id(&item);

    approve(&mac.daemon, &item, "migrate-this-mac").await;

    // Read on each change until the daemon starts itself again, then once
    // the new run has settled the item.
    let mut seen = vec![item];
    seen.extend(upgrade::request_history_while_up(&mac.daemon, &id).await);
    wait_at_release(&mac.daemon, &to).await;
    let done = ended(&mac.daemon, &id).await;
    assert_eq!(done.state(), pb::RequestState::Approved, "{done:?}");
    assert_eq!(upgrade::step_states(&done), vec![pb::MigrateStepState::Done; 5], "{done:?}");
    seen.push(done);
    upgrade::assert_steps_only_advance(&seen);
}

#[tokio::test(flavor = "multi_thread")]
async fn migrating_the_cli_shows_one_step_that_ends_done() {
    let mac = Mac::bare().await;
    swap_in(&mac.bin, "humata", &foreign("humata"));
    let item = mac.item(pb::MigrateThing::Cli).await;
    let id = the_id(&item);
    assert_eq!(stage(&mac.bin, "a"), mac.from);

    approve(&mac.daemon, &item, "migrate-cli").await;

    let mut seen = vec![item];
    seen.extend(
        upgrade::request_history(&mac.daemon, &id, "ended", |request| {
            request.state() != pb::RequestState::Pending
        })
        .await,
    );
    let done = seen.last().expect("the history ends with the item ended");
    assert_eq!(done.state(), pb::RequestState::Approved, "{done:?}");
    assert_eq!(upgrade::step_states(done), vec![pb::MigrateStepState::Done], "{done:?}");
    upgrade::assert_steps_only_advance(&seen);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_migrate_that_fails_marks_the_step_it_failed_at() {
    let mac = Mac::bare().await;
    swap_in(&mac.bin, "humata", &foreign("humata"));
    let item = mac.item(pb::MigrateThing::Cli).await;
    let id = the_id(&item);

    // Nothing is staged, so swapping the files in fails.
    approve(&mac.daemon, &item, "migrate-cli").await;

    let mut seen = vec![item];
    seen.extend(
        upgrade::request_history(&mac.daemon, &id, "showed the Migrate failed", |request| {
            upgrade::migrate_of(request).is_some_and(|card| card.phase() == pb::MigratePhase::Failed)
        })
        .await,
    );
    let failed = seen.last().expect("the history ends with the item failed");
    assert_eq!(failed.state(), pb::RequestState::Pending, "it can be migrated again: {failed:?}");
    assert_eq!(upgrade::step_states(failed), vec![pb::MigrateStepState::Failed], "{failed:?}");
    upgrade::assert_steps_only_advance(&seen);
}

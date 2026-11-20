//! FUSEFSWIRE (D895, T13 case 1): the host's daemon restarts in place (an
//! upgrade swaps the staged release in and the daemon starts itself again,
//! the same process) with the FUSE mount on in Settings. The
//! `humata2_fusefs` helper lives in a session of its own, so it survives:
//! the new run adopts it and starts no second one, Settings says the same
//! thing about the mount as before, and a session running across the
//! restart keeps its environment (`HUMATA_FUSE_ROOT` is there exactly when
//! the mount is up).
//!
//! What this machine reaches is not fixed: where the kernel can't mount the
//! tree the status is "cannot set up" before and after. The test asserts the
//! two are the same, and that a mounted tree is told to the session.
//!
//! The helper is found by its command line, which names this test's own
//! state folder. Nothing sleeps: the waits poll every 2 ms up to a safety
//! net.

use humatad_test_common as common;
use humatad_test_common::upgrade;

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use common::{
    AGENT_NAME, Attach, Build, Daemon, World, agent_tab, binary, create_channel, folder, get_session, is_running,
    kill_holders, open, start_session, stored_tabs, wait_running,
};
use humata2_grpc::humata::v1 as pb;
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;
use tonic::transport::{Channel, Endpoint, Uri};

/// A safety net for a daemon that starts itself again or a mount that comes
/// up on a loaded machine, never a time anything is expected to take.
const LONG: Duration = Duration::from_secs(90);

/// Release files, as the daemon names them in `bin`.
const FILES: [(&str, &str); 5] = [
    ("humatad2", "HUMATAD2"),
    ("humata2_holder", "HUMATA2_HOLDER"),
    ("humata", "HUMATA"),
    ("stack", "STACK"),
    ("humata2_fusefs", "HUMATA2_FUSEFS"),
];

fn put_file(variable: &str, to: &Path) {
    std::fs::copy(binary(variable), to).unwrap_or_else(|error| panic!("couldn't copy {variable} to {}: {error}", to.display()));
    std::fs::set_permissions(to, std::fs::Permissions::from_mode(0o755)).expect("the file is made runnable");
}

fn install_release_a(bin: &Path) -> String {
    std::fs::create_dir_all(bin).expect("the bin folder is made");
    for (name, variable) in FILES {
        put_file(variable, &bin.join(name));
    }
    upgrade::install(bin, &upgrade::manifest(&upgrade::admin_sha(), "a"))
}

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

async fn wait_at_release(daemon: &Daemon, bundle: &str) {
    let deadline = Instant::now() + LONG;
    loop {
        if try_info(daemon).await.and_then(|info| info.build).is_some_and(|build| build.bundle == bundle) {
            return;
        }
        assert!(Instant::now() < deadline, "the daemon never answered as release {bundle}");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

/// The processes of `humata2_fusefs serve` on this test's own socket.
fn helpers(state: &Path) -> Vec<u32> {
    let socket = state.join("fusefs.sock").display().to_string();
    let listing = Command::new("ps").args(["-ww", "-axo", "pid=,command="]).output().expect("ps lists the processes");
    let mut found: Vec<u32> = String::from_utf8_lossy(&listing.stdout)
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let (pid, command) = line.split_once(' ')?;
            let mine = command.contains("humata2_fusefs") && command.contains(" serve ") && command.contains(&socket);
            if mine { pid.parse().ok() } else { None }
        })
        .collect();
    found.sort_unstable();
    found
}

async fn settings(daemon: &Daemon) -> pb::settings_service_client::SettingsServiceClient<Channel> {
    pb::settings_service_client::SettingsServiceClient::new(daemon.channel().await)
}

async fn set_mount(daemon: &Daemon, on: bool) {
    settings(daemon)
        .await
        .update_settings(pb::UpdateSettingsRequest { fusefs_enabled: Some(on), ..Default::default() })
        .await
        .expect("the mount switch is saved");
}

/// The mount's state and folder once Settings says it is up or can't be.
async fn settled_status(daemon: &Daemon) -> (i32, String) {
    let deadline = Instant::now() + LONG;
    loop {
        let shown = settings(daemon).await.get_settings(pb::GetSettingsRequest {}).await;
        if let Some(status) = shown.ok().and_then(|reply| reply.into_inner().fusefs_status) {
            if status.state == pb::fuse_mount_status::State::Mounted as i32
                || status.state == pb::fuse_mount_status::State::CannotSetUp as i32
            {
                return (status.state, status.mount_root);
            }
        }
        assert!(Instant::now() < deadline, "the mount never came up or said it can't");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

/// Waits for `path` and reads it as an environment.
async fn environment(path: &Path) -> BTreeMap<String, String> {
    let deadline = Instant::now() + LONG;
    while !path.exists() {
        assert!(Instant::now() < deadline, "{} never appeared", path.display());
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    std::fs::read_to_string(path)
        .expect("the environment reads")
        .lines()
        .filter_map(|line| line.split_once('=').map(|(name, value)| (name.to_owned(), value.to_owned())))
        .collect()
}

/// An agent that writes its environment once when it starts, and again
/// whenever a line is typed to it.
fn write_agent(dir: &Path, world: &World, envs: &Path) -> PathBuf {
    let script = dir.join("env-agent.sh");
    std::fs::write(
        &script,
        format!(
            "env > '{e}/'$2'.start.part'; mv '{e}/'$2'.start.part' '{e}/'$2'.start'\n\
             stty -icanon 2>/dev/null\n\
             while IFS= read -r line; do env > '{e}/'$2'.again.part'; mv '{e}/'$2'.again.part' '{e}/'$2'.again'; done\n",
            e = envs.display()
        ),
    )
    .expect("the agent script is written");
    let agents = serde_json::json!([{
        "name": "codex",
        "program": "/bin/sh",
        "start_args": [script, "--session-id", "{conversation}"],
        "resume_args": [script, "--resume", "{conversation}"],
        "conversation_file": format!("{}/{{conversation}}.jsonl", world.conv.display()),
    }]);
    std::fs::write(&world.agents, agents.to_string()).expect("the agents file is written");
    script
}

fn env_files(envs: &Path, suffix: &str) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(envs)
        .expect("the environments folder reads")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|extension| extension == suffix))
        .collect();
    files.sort();
    files
}

#[tokio::test(flavor = "multi_thread")]
async fn a_daemon_that_restarts_in_place_adopts_the_fusefs_helper_and_keeps_the_mount_and_the_session() {
    let dir = folder();
    let world = World::new(dir.path());
    let home = dir.path().join("home");
    std::fs::create_dir_all(home.join(".codex")).expect("the home is made");
    std::fs::write(home.join(".codex/auth.json"), "{}").expect("the login is written");
    let envs = dir.path().join("envs");
    std::fs::create_dir_all(&envs).expect("the environments folder is made");
    write_agent(dir.path(), &world, &envs);
    world.remote("acme", "api");
    let bin = dir.path().join("bin");
    let from = install_release_a(&bin);

    let mut args: Vec<String> = world
        .args()
        .into_iter()
        .map(|arg| if arg == AGENT_NAME { "codex".to_owned() } else { arg })
        .collect();
    args.extend(["--fusefs".to_owned(), binary("HUMATA2_FUSEFS").display().to_string()]);
    args.extend(["--humata".to_owned(), binary("HUMATA").display().to_string()]);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let home_text = home.display().to_string();
    let daemon = Daemon::start_with_env(Build::Admin, dir, &refs, &[("HOME", &home_text)]).await;
    assert_eq!(upgrade::build_of(&daemon).await.bundle, from, "the daemon runs release a");

    // The mount is turned on and settles (mounted, or this machine can't).
    set_mount(&daemon, true).await;
    let status_before = settled_status(&daemon).await;
    let helper_before = helpers(&daemon.state);
    assert_eq!(helper_before.len(), 1, "one helper serves the mount: {helper_before:?}");

    // A session started now has the environment the mount decides.
    let (channel, setup) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    assert_eq!(setup.state, pb::OperationState::Succeeded as i32, "{:?}", setup.error);
    let (session, started) = start_session(&daemon, &channel, "env").await;
    assert_eq!(started.state, pb::OperationState::Succeeded as i32, "{:?}", started.error);
    wait_running(&daemon, &session).await;
    let started_files = {
        let deadline = Instant::now() + LONG;
        while env_files(&envs, "start").is_empty() {
            assert!(Instant::now() < deadline, "the agent never wrote its environment");
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        env_files(&envs, "start")
    };
    let first = environment(&started_files[0]).await;
    let mounted = status_before.0 == pb::fuse_mount_status::State::Mounted as i32;
    assert_eq!(first.contains_key("HUMATA_FUSE_ROOT"), mounted, "the session's environment follows the mount");
    let holders_before: Vec<(String, u32)> =
        stored_tabs(&daemon.state).iter().map(|tab| (tab.id.clone(), tab.holder_pid)).collect();
    assert!(!holders_before.is_empty(), "the session has a holder");
    let pid = daemon.pid();

    // The daemon restarts in place.
    let to = stage_release_b(&bin);
    wait_at_release(&daemon, &to).await;
    assert_eq!(daemon.pid(), pid, "the daemon started itself again in its own process");

    // The new run adopts the helper: the same process, and no second one.
    let status_after = settled_status(&daemon).await;
    assert_eq!(status_after, status_before, "Settings says the same about the mount");
    assert_eq!(helpers(&daemon.state), helper_before, "the same helper serves the mount, and no other starts");

    // The running session kept its process and its environment.
    assert!(is_running(&get_session(&daemon, &session).await), "the session kept running");
    let holders_after: Vec<(String, u32)> =
        stored_tabs(&daemon.state).iter().map(|tab| (tab.id.clone(), tab.holder_pid)).collect();
    assert_eq!(holders_after, holders_before, "the same holders");
    let tab = agent_tab(&daemon, &session).await;
    let mut attach = Attach::start(&daemon, open(&tab, 0)).await;
    attach.type_text("again\n").await;
    let again = {
        let deadline = Instant::now() + LONG;
        while env_files(&envs, "again").is_empty() {
            assert!(Instant::now() < deadline, "the agent never answered after the restart");
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        env_files(&envs, "again")
    };
    let kept = environment(&again[0]).await;
    assert_eq!(kept.contains_key("HUMATA_FUSE_ROOT"), mounted, "the session kept its mount environment");
    assert_eq!(kept.get("HUMATA_FUSE_ROOT"), first.get("HUMATA_FUSE_ROOT"));

    // Done: the mount is turned off, and what this test started ends.
    drop(attach);
    set_mount(&daemon, false).await;
    // The helper is killed only once it has unmounted, so no stale mount is left.
    let deadline = Instant::now() + LONG;
    loop {
        let shown = settings(&daemon).await.get_settings(pb::GetSettingsRequest {}).await;
        let state = shown.ok().and_then(|reply| reply.into_inner().fusefs_status).map(|status| status.state);
        if state == Some(pb::fuse_mount_status::State::Off as i32) {
            break;
        }
        assert!(Instant::now() < deadline, "the mount never turned off");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    kill_holders(&stored_tabs(&daemon.state)).await;
    for helper in helpers(&daemon.state) {
        let _ = Command::new("kill").arg(helper.to_string()).status();
    }
}

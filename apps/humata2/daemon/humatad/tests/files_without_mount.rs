//! FUSEFSWIRE: when the FUSE mount can't be set up on a machine, sessions
//! there reach every Humata location through `humata files` (the same calls
//! the MCP files tools make), and a launched agent is not given
//! `HUMATA_FUSE_ROOT`. The mount here is on in Settings but its helper can't
//! start: `--fusefs` names a file that isn't there.

use humatad_test_common as common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::mcp_client::Mcp;
use common::{
    AGENT_NAME, Build, Daemon, World, approve, binary, create_channel, folder, humata, machine_id,
    shell_token, start_session, state_folder, wait_running,
};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{ChannelRecord, ChannelRepo, MainClone as MainState, SessionRecord, Store};
use serde_json::{Value, json};

const CHANNEL: &str = "ch_nm";
const CHANNEL_FOLDER: &str = "nm-channel";
const REPO: &str = "api";
const MINE: &str = "se_mine";
const OTHER: &str = "se_other";

const SUCCESS: i32 = 0;

/// Turns the mount on in Settings; its helper can't start, so nothing mounts.
async fn turn_mount_on(daemon: &Daemon) {
    pb::settings_service_client::SettingsServiceClient::new(daemon.channel().await)
        .update_settings(pb::UpdateSettingsRequest { fusefs_enabled: Some(true), ..Default::default() })
        .await
        .unwrap();
}

/// The `serve` flag that names a helper that doesn't exist.
fn missing_helper(dir: &Path) -> String {
    dir.join("no-such-dir").join("humata2_fusefs").display().to_string()
}

struct Rig {
    daemon: Daemon,
    machine: String,
    root: PathBuf,
    local: PathBuf,
}

/// A daemon with one channel and two sessions, each with its folder.
async fn rig() -> Rig {
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    let root = dir.path().to_owned();
    for (id, at) in [(MINE, root.join("mine")), (OTHER, root.join("other"))] {
        store
            .put_session(&SessionRecord {
                id: id.to_owned(),
                channel: CHANNEL.to_owned(),
                machine: String::new(),
                title: id.to_owned(),
                agent: "zsh".to_owned(),
                folder: at.clone(),
                created_unix_ms: 0,
            })
            .unwrap();
        std::fs::create_dir_all(at.join(REPO)).unwrap();
    }
    store
        .put_channel(&ChannelRecord {
            id: CHANNEL.to_owned(),
            name: "no mount".to_owned(),
            folder: CHANNEL_FOLDER.to_owned(),
            branch_prefix: None,
            default_agent: "zsh".to_owned(),
            repos: vec![ChannelRepo { owner: "acme".to_owned(), name: REPO.to_owned(), main: MainState::Ready }],
            machines: Vec::new(),
            revision: 1,
            created_unix_ms: 0,
        })
        .unwrap();
    drop(store);
    let channel = root.join("Humata").join(CHANNEL_FOLDER);
    for path in [channel.join("shared"), channel.join("main").join(REPO), channel.join("sessions")] {
        std::fs::create_dir_all(path).unwrap();
    }
    std::fs::write(channel.join("main").join(REPO).join("readme.md"), b"from main").unwrap();
    let local = root.join("local");
    std::fs::create_dir_all(&local).unwrap();
    let helper = missing_helper(&root);
    let daemon = Daemon::start_with(Build::Admin, dir, &["--fusefs", &helper]).await;
    turn_mount_on(&daemon).await;
    let machine = machine_id(&daemon).await.value;
    Rig { daemon, machine, root, local }
}

impl Rig {
    fn ok(&self, args: &[&str]) -> Value {
        let mut all = vec!["--channel", CHANNEL];
        all.extend_from_slice(args);
        let (code, json) = humata(&self.daemon, &all);
        assert_eq!(code, SUCCESS, "humata {all:?} failed: {json}");
        json["result"].clone()
    }

    /// Writes `text` at `location` and reads it back.
    fn round_trip(&self, location: &str, text: &str) {
        let source = self.local.join("source.txt");
        std::fs::write(&source, text).unwrap();
        self.ok(&["files", "put", source.to_str().unwrap(), location]);
        let read = self.ok(&["files", "cat", location]);
        assert_eq!(read["bytes"], text.len(), "{location}: {read}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn every_location_is_reachable_through_the_files_commands() {
    let rig = rig().await;
    let at = |rest: &str| format!("{}:{rest}", rig.machine);

    rig.round_trip(&at("shared/notes.txt"), "shared text");
    rig.round_trip(&at("channel/plan.txt"), "channel text");
    rig.round_trip(&at(&format!("{MINE}/acme/{REPO}/own.txt")), "my clone");
    rig.round_trip(&at(&format!("{OTHER}/acme/{REPO}/theirs.txt")), "another session");
    rig.round_trip(&at(&format!("folder/{OTHER}/doc.md")), "a session folder");
    let main = rig.ok(&["files", "cat", &at(&format!("main/acme/{REPO}/readme.md"))]);
    assert_eq!(main["bytes"], "from main".len(), "{main}");
    assert!(rig.root.join("other").join("doc.md").is_file());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_trashed_file_is_restored_through_the_trash_commands() {
    let rig = rig().await;
    let location = format!("{}:shared/gone.txt", rig.machine);
    rig.round_trip(&location, "come back");

    let removed = rig.ok(&["files", "rm", &location]);
    let id = removed["items"][0].as_str().unwrap_or_else(|| panic!("no trash id in {removed}")).to_owned();
    rig.ok(&["trash", "restore", &id]);

    let read = rig.ok(&["files", "cat", &location]);
    assert_eq!(read["bytes"], "come back".len(), "{read}");
}

/// A session of the daemon's own: an agent the daemon composes a config home
/// for (named `codex`) that writes its environment to a file.
async fn agent_environment() -> Vec<(String, String)> {
    let dir = folder();
    let world = World::new(dir.path());
    let home = dir.path().join("home");
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::fs::write(home.join(".codex/auth.json"), "{}").unwrap();
    let out = dir.path().join("agent-env");
    let script = dir.path().join("env-agent.sh");
    std::fs::write(
        &script,
        format!(
            "env > '{out}.part'; mv '{out}.part' '{out}'\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do :; done\n",
            out = out.display()
        ),
    )
    .unwrap();
    let agents = serde_json::json!([{
        "name": "codex",
        "program": "/bin/sh",
        "start_args": [script, "--session-id", "{conversation}"],
        "resume_args": [script, "--resume", "{conversation}"],
        "conversation_file": format!("{}/{{conversation}}.jsonl", world.conv.display()),
    }]);
    std::fs::write(&world.agents, agents.to_string()).unwrap();
    world.remote("acme", REPO);
    let mut args: Vec<String> = world
        .args()
        .into_iter()
        .map(|arg| if arg == AGENT_NAME { "codex".to_owned() } else { arg })
        .collect();
    args.extend(["--fusefs".to_owned(), missing_helper(dir.path())]);
    args.extend(["--humata".to_owned(), binary("HUMATA").display().to_string()]);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let home_text = home.display().to_string();
    let daemon = Daemon::start_with_env(Build::Admin, dir, &refs, &[("HOME", &home_text)]).await;
    turn_mount_on(&daemon).await;

    let (channel, setup) = create_channel(&daemon, "Payments", &[("acme", REPO)]).await;
    assert_eq!(setup.state, pb::OperationState::Succeeded as i32, "{:?}", setup.error);
    let (_session, started) = start_session(&daemon, &channel, "env").await;
    assert_eq!(started.state, pb::OperationState::Succeeded as i32, "{:?}", started.error);

    let deadline = Instant::now() + Duration::from_secs(60);
    while !out.exists() {
        assert!(Instant::now() < deadline, "the agent never wrote its environment");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    std::fs::read_to_string(&out)
        .unwrap()
        .lines()
        .filter_map(|line| line.split_once('=').map(|(name, value)| (name.to_owned(), value.to_owned())))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_launched_agent_gets_no_fuse_root_when_the_mount_can_t_be_set_up() {
    let env = agent_environment().await;

    let has = |name: &str| env.iter().any(|(var, _)| var == name);
    assert!(has("CODEX_HOME"), "the agent runs in its composed home");
    assert!(has("HUMATA_MACHINE_NAME"), "the machine's name is still given");
    assert!(!has("HUMATA_FUSE_ROOT"), "no mount, so no folder to point at");
}

/// A tool call as an agent makes it. A call that waits on the user is
/// approved as the user would, then its answer is waited for.
async fn reach(daemon: &Daemon, mcp: &mut Mcp, id: u64, tool: &str, mut arguments: Value) -> Value {
    arguments["wait_seconds"] = json!(0);
    let first = mcp.call(id, tool, arguments).await;
    if first["structuredContent"]["outcome"] != "pending" {
        return first;
    }
    let request = first["structuredContent"]["request_id"].as_str().expect("a request id").to_owned();
    approve(daemon, &pb::RequestId { value: request.clone() }).await;
    mcp.call(id + 1000, "inbox_wait", json!({ "request_id": request })).await
}

fn succeeded(result: &Value) {
    assert_eq!(result["isError"], false, "{result}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_reaches_every_location_through_the_files_tools() {
    let dir = folder();
    let world = World::new(dir.path());
    world.remote("acme", REPO);
    let mut args = world.args();
    args.extend(["--fusefs".to_owned(), missing_helper(dir.path())]);
    args.extend(["--humata".to_owned(), binary("HUMATA").display().to_string()]);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let daemon = Daemon::start_with(Build::Admin, dir, &refs).await;
    turn_mount_on(&daemon).await;
    let (channel, setup) = create_channel(&daemon, "Payments", &[("acme", REPO)]).await;
    assert_eq!(setup.state, pb::OperationState::Succeeded as i32, "{:?}", setup.error);
    let (mine, _) = start_session(&daemon, &channel, "mine").await;
    let (other, _) = start_session(&daemon, &channel, "other").await;
    wait_running(&daemon, &mine).await;
    wait_running(&daemon, &other).await;
    let (mine_token, _mine_shell) = shell_token(&daemon, &mine).await;
    let (other_token, _other_shell) = shell_token(&daemon, &other).await;
    let mut me = Mcp::start(&daemon.socket, &mine_token).await;
    let mut them = Mcp::start(&daemon.socket, &other_token).await;
    let repo = json!(format!("acme/{REPO}"));

    // With no mount the agent is told to use the files tools, not a folder.
    let told = me.instructions().expect("the server gives instructions");
    for tool in ["files_list", "file_read", "file_write"] {
        assert!(told.contains(tool), "the instructions don't name {tool}: {told}");
    }
    assert!(!told.contains("HUMATA_FUSE_ROOT"), "{told}");

    // Where the agent may write directly: shared and its own clone.
    for (n, location) in [
        json!({ "root": "shared", "path": "notes.txt" }),
        json!({ "root": "clone", "repo": repo, "path": "own.txt" }),
    ]
    .into_iter()
    .enumerate()
    {
        let n = n as u64 * 10;
        let wrote = reach(&daemon, &mut me, n + 1, "file_write", json!({ "location": location, "text": "hello" })).await;
        succeeded(&wrote);
        let read = reach(&daemon, &mut me, n + 2, "file_read", json!({ "location": location })).await;
        assert_eq!(read["structuredContent"]["result"]["text"], "hello", "{read}");
    }

    // Another session's clone, which that session wrote.
    let theirs = json!({ "root": "clone", "repo": repo, "path": "theirs.txt" });
    succeeded(&reach(&daemon, &mut them, 1, "file_write", json!({ "location": theirs, "text": "another" })).await);
    let across = json!({ "root": "clone", "session": other.value, "repo": repo, "path": "theirs.txt" });
    let read = reach(&daemon, &mut me, 30, "file_read", json!({ "location": across })).await;
    assert_eq!(read["structuredContent"]["result"]["text"], "another", "{read}");

    // The channel folder, the main clone and the session's own folder list.
    for (n, location) in [
        json!({ "root": "channel_folder", "path": "" }),
        json!({ "root": "main_clone", "repo": repo, "path": "" }),
        json!({ "root": "session_folder", "path": "" }),
    ]
    .into_iter()
    .enumerate()
    {
        let listed = reach(&daemon, &mut me, 40 + n as u64, "files_list", json!({ "location": location })).await;
        succeeded(&listed);
    }
}

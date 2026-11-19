//! D801 D through a real `humatad2`: renaming a channel's folder on one
//! filesystem keeps its running agent and shell running. Their holders and
//! programs keep their pids, the old folder name keeps resolving as a link
//! to the new folder while they run, the link goes when the last holder that
//! started under it ends, and an agent that is stopped and resumed, or
//! resumed after a power cut, finds its conversation under the new folder's
//! key.
//!
//! The agent keeps each conversation at `<conv>/<folder, dashed>/<id>.jsonl`
//! as Claude does, so it resumes only if its conversation moved to the new
//! folder's key. Everything runs under a HOME of the test's own.

use humatad_test_common as common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use common::{
    Attach, Build, Daemon, World, agent_tab, binary, create_channel, folder, get_session, kill_holders, open, shell_token,
    start_session, stored_tabs, wait_running,
};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{AgentState, Store, TabRecord};

/// The test agent: it starts or resumes the conversation kept by folder,
/// writes the folder it was given as the conversation's first line, as
/// Claude writes its `cwd`, and logs every start to `<root>/conv.log` as
/// `mode, conversation, key, folder`. A resume whose conversation isn't at
/// its key fails, as Claude's does.
const AGENT: &str = r#"CONV_DIR='@CONV@'
mode=$1; id=$2; key=$3; place=$4
f="$CONV_DIR/$key/$id.jsonl"
printf '%s\t%s\t%s\t%s\n' "$mode" "$id" "$key" "$place" >> "$CONV_DIR.log"
case "$mode" in
  --session-id) mkdir -p "$CONV_DIR/$key"; printf '{"cwd":"%s"}\n' "$place" > "$f" ;;
  --resume) [ -f "$f" ] || { echo "no conversation at $f"; exit 3; } ;;
esac
# Reads keys itself, as agent CLIs do, so typing never waits out the
# holder's keys limit (see common/mod.rs).
stty -icanon 2>/dev/null
echo "agent up $mode"
while IFS= read -r line; do echo "got: $line"; printf '%s\n' "$line" >> "$f"; done
"#;

/// How an agent that keys its conversations by folder names that folder.
fn dashed(path: &Path) -> String {
    path.display().to_string().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

/// The variables that keep every agent's own files inside `home`.
fn home_env(home: &Path) -> Vec<(String, String)> {
    let at = |rest: &str| home.join(rest).display().to_string();
    let env = vec![
        ("HOME".to_owned(), home.display().to_string()),
        ("CLAUDE_CONFIG_DIR".to_owned(), at(".claude")),
        ("CODEX_HOME".to_owned(), at(".codex")),
        ("COPILOT_HOME".to_owned(), at(".copilot")),
        ("COPILOT_CACHE_HOME".to_owned(), at(".cache/copilot")),
        ("XDG_CONFIG_HOME".to_owned(), at(".config")),
        ("XDG_DATA_HOME".to_owned(), at(".local/share")),
        ("XDG_STATE_HOME".to_owned(), at(".local/state")),
        ("XDG_CACHE_HOME".to_owned(), at(".cache")),
    ];
    for (_, path) in &env {
        std::fs::create_dir_all(path).unwrap();
    }
    env
}

/// The processes whose parent is one of `parents`, from `ps`, sorted.
fn children_of(parents: &[u32]) -> Vec<u32> {
    let listed = Command::new("/bin/ps").args(["-A", "-o", "pid=,ppid="]).output().expect("ps runs");
    let mut found: Vec<u32> = String::from_utf8_lossy(&listed.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace().map(str::parse::<u32>);
            let (Some(Ok(pid)), Some(Ok(parent))) = (fields.next(), fields.next()) else {
                return None;
            };
            parents.contains(&parent).then_some(pid)
        })
        .collect();
    found.sort_unstable();
    found
}

struct Fixture {
    daemon: Daemon,
    world: World,
    channel: pb::Channel,
}

/// A session of the test: its id, its conversation and its folder when it
/// started.
struct Started {
    id: pb::SessionId,
    conversation: String,
    folder: PathBuf,
}

impl Fixture {
    async fn new() -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        let script = world.root.join("agent.sh");
        std::fs::write(&script, AGENT.replace("@CONV@", &world.conv.display().to_string())).unwrap();
        let agent = serde_json::json!({
            "name": common::AGENT_NAME,
            "program": "/bin/sh",
            "start_args": [script, "--session-id", "{conversation}", "{folder_dashed}", "{folder}"],
            "resume_args": [script, "--resume", "{conversation}", "{folder_dashed}", "{folder}"],
            "conversation_file": format!("{}/{{folder_dashed}}/{{conversation}}.jsonl", world.conv.display()),
        });
        std::fs::write(&world.agents, serde_json::json!([agent]).to_string()).unwrap();
        world.remote("acme", "api");
        let env = home_env(&dir.path().join("home"));
        let env: Vec<(&str, &str)> = env.iter().map(|(name, value)| (name.as_str(), value.as_str())).collect();
        let mut args = world.args();
        args.extend(["--humata".to_owned(), binary("HUMATA").display().to_string()]);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with_env(Build::Admin, dir, &refs, &env).await;
        let (channel, setup) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        assert_eq!(setup.state, pb::OperationState::Succeeded as i32, "{:?}", setup.error);
        Self { daemon, world, channel }
    }

    fn old(&self) -> PathBuf {
        self.world.channels.join(&self.channel.folder)
    }

    fn store(&self) -> Store {
        Store::open(self.daemon.state.join("store")).unwrap()
    }

    async fn session(&self, title: &str) -> Started {
        let (id, operation) = start_session(&self.daemon, &self.channel, title).await;
        assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{:?}", operation.error);
        let session = wait_running(&self.daemon, &id).await;
        let folder = self.store().session(&id.value).unwrap().expect("the store has the session").folder;
        Started { id, conversation: session.conversation_id, folder }
    }

    /// Renames the channel to Billing, with no confirming.
    async fn rename(&self) -> pb::Channel {
        self.daemon
            .channel_service()
            .await
            .rename_channel(pb::RenameChannelRequest {
                idempotency_key: None,
                id: self.channel.id.clone(),
                name: "Billing".to_owned(),
                expected_revision: None,
                confirm_running: false,
            })
            .await
            .expect("a rename of a channel with a running agent needs no confirming")
            .into_inner()
    }

    /// Types `line` to the session's agent, so its conversation holds it.
    async fn say(&self, id: &pb::SessionId, line: &str) {
        let tab = agent_tab(&self.daemon, id).await;
        let mut attach = Attach::start(&self.daemon, open(&tab, 0)).await;
        attach.type_text(&format!("{line}\r")).await;
        attach.wait_for(&format!("got: {line}")).await;
    }

    /// Every start of the agent of `conversation`: mode, key and folder.
    fn starts(&self, conversation: &str) -> Vec<(String, String, PathBuf)> {
        let log = std::fs::read_to_string(self.world.root.join("conv.log")).unwrap_or_default();
        log.lines()
            .filter_map(|line| {
                let fields: Vec<&str> = line.split('\t').collect();
                match fields.as_slice() {
                    [mode, id, key, place] if *id == conversation => {
                        Some(((*mode).to_owned(), (*key).to_owned(), PathBuf::from(place)))
                    }
                    _ => None,
                }
            })
            .collect()
    }

    async fn wait_starts(&self, conversation: &str, times: usize) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while self.starts(conversation).len() < times {
            assert!(Instant::now() < deadline, "the agent started {} times, not {times}", self.starts(conversation).len());
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// The tabs of `session`'s holders.
    fn tabs_of(&self, id: &pb::SessionId) -> Vec<TabRecord> {
        stored_tabs(&self.daemon.state).into_iter().filter(|tab| tab.session.as_deref() == Some(id.value.as_str())).collect()
    }

    /// Checks the agent resumed in `folder` with its conversation: started
    /// by a resume at `folder`'s key, the conversation kept there with what
    /// was said and naming `folder`, and nothing left at the old key.
    async fn resumed_under(&self, session: &Started, folder: &Path, said: &str) {
        self.wait_starts(&session.conversation, 2).await;
        let (mode, key, place) = self.starts(&session.conversation).pop().unwrap();
        assert_eq!(mode, "--resume", "it took up its conversation");
        assert_eq!(key, dashed(folder), "its conversation is keyed by its new folder");
        assert_eq!(place, folder);
        wait_running(&self.daemon, &session.id).await;
        let at = |folder: &Path| self.world.conv.join(dashed(folder)).join(format!("{}.jsonl", session.conversation));
        let kept = std::fs::read_to_string(at(folder)).expect("the conversation is at the new folder's key");
        assert!(kept.lines().any(|line| line == said), "it holds what was said before: {kept:?}");
        assert!(kept.contains(&folder.display().to_string()), "it names its new folder: {kept:?}");
        assert!(!at(&session.folder).exists(), "nothing is left at the old key");
    }
}

async fn until(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn is_link(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
}

/// Renaming a channel with a running agent and a shell stops neither: the
/// holders and the programs under them keep their pids, the agent is not
/// started again, and the old folder name still reaches the folder.
#[tokio::test(flavor = "multi_thread")]
async fn a_rename_keeps_the_agent_and_a_shell_running_and_the_old_path_resolves() {
    let fixture = Fixture::new().await;
    let old = fixture.old();
    let session = fixture.session("Ledger fix").await;
    let (_token, mut shell) = shell_token(&fixture.daemon, &session.id).await;
    let before = fixture.tabs_of(&session.id);
    assert_eq!(before.len(), 2, "an agent tab and a shell tab: {before:?}");
    let holders: Vec<u32> = before.iter().map(|tab| tab.holder_pid).collect();
    let programs = children_of(&holders);
    assert!(!programs.is_empty(), "the holders run programs");

    let renamed = fixture.rename().await;

    let new = fixture.world.channels.join(&renamed.folder);
    assert!(new.join("main/api").is_dir());
    assert!(is_link(&old), "the old name is a link to the new folder");
    assert!(old.join("main/api").is_dir(), "the old name resolves");
    let after = fixture.tabs_of(&session.id);
    assert_eq!(after.iter().map(|tab| tab.holder_pid).collect::<Vec<_>>(), holders, "the holders are the same processes");
    assert_eq!(children_of(&holders), programs, "so are the programs under them");
    assert_eq!(fixture.starts(&session.conversation).len(), 1, "the agent was not started again");
    let agent = fixture.store().agent(&session.id.value).unwrap().expect("the agent has a record");
    assert!(matches!(agent.state, AgentState::Running { .. }), "{agent:?}");
    assert_eq!(fixture.store().session(&session.id.value).unwrap().unwrap().folder, new.join(session.folder.strip_prefix(&old).unwrap()));
    shell.type_text("echo alive-$((6*7))\r").await;
    shell.wait_for("alive-42").await;
}

/// The old name goes when the last holder that started under it ends.
#[tokio::test(flavor = "multi_thread")]
async fn the_old_name_goes_when_the_last_holder_started_under_it_ends() {
    let fixture = Fixture::new().await;
    let old = fixture.old();
    let session = fixture.session("Ledger fix").await;
    let (_token, _shell) = shell_token(&fixture.daemon, &session.id).await;
    fixture.rename().await;
    assert!(is_link(&old));
    let tabs = fixture.tabs_of(&session.id);

    kill_holders(&tabs[..1]).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(is_link(&old), "a holder that started under it still runs");

    kill_holders(&tabs[1..]).await;
    until("the old name to go", || std::fs::symlink_metadata(&old).is_err()).await;
    assert!(fixture.world.channels.join("billing/main/api").is_dir(), "the folder itself stays");
}

/// A stopped and resumed agent finds its conversation under the new
/// folder's key, and runs in the new folder.
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_that_is_stopped_and_resumed_after_a_rename_takes_up_its_conversation_in_the_new_folder() {
    let fixture = Fixture::new().await;
    let old = fixture.old();
    let session = fixture.session("Ledger fix").await;
    fixture.say(&session.id, "ledger-line").await;
    let renamed = fixture.rename().await;
    let new = fixture.world.channels.join(&renamed.folder);
    let folder = new.join(session.folder.strip_prefix(&old).unwrap());

    let mut sessions = fixture.daemon.session_service().await;
    sessions
        .stop_session(pb::StopSessionRequest { idempotency_key: None, id: Some(session.id.clone()), for_move: false })
        .await
        .expect("the agent stops");
    sessions
        .resume_session(pb::ResumeSessionRequest { id: Some(session.id.clone()), ..Default::default() })
        .await
        .expect("the agent resumes");

    fixture.resumed_under(&session, &folder, "ledger-line").await;
    let launch = fixture.store().agent(&session.id.value).unwrap().unwrap().launch;
    assert_eq!(launch.folder, folder, "its record names the new folder once it has resumed");
    until("the old name to go", || std::fs::symlink_metadata(&old).is_err()).await;
}

/// After a power cut the holders are gone; the daemon starts and the agent
/// resumes under the new folder's key, with the old name removed.
#[tokio::test(flavor = "multi_thread")]
async fn after_a_power_cut_the_agent_resumes_with_its_conversation_under_the_new_folder() {
    let mut fixture = Fixture::new().await;
    let old = fixture.old();
    let session = fixture.session("Ledger fix").await;
    fixture.say(&session.id, "ledger-line").await;
    let renamed = fixture.rename().await;
    let new = fixture.world.channels.join(&renamed.folder);
    let folder = new.join(session.folder.strip_prefix(&old).unwrap());

    kill_holders(&fixture.tabs_of(&session.id)).await;
    fixture.daemon.restart().await;

    fixture.resumed_under(&session, &folder, "ledger-line").await;
    assert_eq!(get_session(&fixture.daemon, &session.id).await.conversation_id, session.conversation);
    until("the old name to go", || std::fs::symlink_metadata(&old).is_err()).await;
}

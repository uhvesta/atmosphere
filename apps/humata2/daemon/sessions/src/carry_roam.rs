//! A carry of a session whose agent's conversation an adapter knows (MV-16,
//! CV-4): only that conversation's own files travel, not another
//! conversation of the same folder; on the new machine no file of it names
//! the old home or folder and the adapter reads it there; the old machine's
//! discards leave the other conversation alone; a Copilot conversation
//! carries the same way, and a Copilot session with no session state yet
//! carries its folder alone; and a conversation whose records its adapter
//! doesn't know is refused before anything is staged.

use std::sync::Arc;
use std::time::Duration;

use humata2_conversation::{Format, Placement, adapter};
use humata2_revive::{ReviveConfig, RestartReports};
use humata2_store::{AgentLaunch, AgentRecord, ChannelRecord, ChannelRepo, MainClone, SessionRecord, Store};

use super::*;
use crate::agents::Agents;
use crate::layout::Layout;

/// The session's conversation, and another one of the same folder.
const ID: &str = "4f6d2c1e-8a3b-4c5d-9e7f-1a2b3c4d5e6f";
const OTHER: &str = "9e8d7c6b-5a4f-4e3d-8c2b-1a0f9e8d7c6b";
const CLAUDE_VERSION: &str = "2.1.325";

/// Two turns as Claude Code writes them. `{folder}`, `{home}` and `{id}`
/// are filled per conversation.
const TWO_TURNS: &str = concat!(
    r#"{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"{folder}","sessionId":"{id}","version":"2.1.325","gitBranch":"humata/fix","type":"user","message":{"role":"user","content":"Fix the login redirect\nMy notes are in {home}/notes/login.md."},"uuid":"7c0e5a10-0001-4a8e-9b1d-3f2a6c8d0001","timestamp":"2026-11-16T09:01:00.000Z"}"#,
    "\n",
    r#"{"parentUuid":"7c0e5a10-0001-4a8e-9b1d-3f2a6c8d0001","isSidechain":false,"userType":"external","cwd":"{folder}","sessionId":"{id}","version":"2.1.325","gitBranch":"humata/fix","message":{"id":"msg_01A","type":"message","role":"assistant","model":"claude-sonnet-4-5","content":[{"type":"text","text":"Fixed {folder}/api/src/login.rs."}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":812,"output_tokens":64}},"requestId":"req_01A","type":"assistant","uuid":"7c0e5a10-0011-4a8e-9b1d-3f2a6c8d0011","timestamp":"2026-11-16T09:02:00.000Z"}"#,
    "\n",
    r#"{"parentUuid":"7c0e5a10-0011-4a8e-9b1d-3f2a6c8d0011","isSidechain":false,"userType":"external","cwd":"{folder}","sessionId":"{id}","version":"2.1.325","gitBranch":"humata/fix","type":"user","message":{"role":"user","content":"Now add a test for it"},"uuid":"7c0e5a10-0002-4a8e-9b1d-3f2a6c8d0002","timestamp":"2026-11-16T09:05:00.000Z"}"#,
    "\n",
    r#"{"parentUuid":"7c0e5a10-0002-4a8e-9b1d-3f2a6c8d0002","isSidechain":false,"userType":"external","cwd":"{folder}","sessionId":"{id}","version":"2.1.325","gitBranch":"humata/fix","message":{"id":"msg_01B","type":"message","role":"assistant","model":"claude-sonnet-4-5","content":[{"type":"text","text":"Added a test."}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":812,"output_tokens":64}},"requestId":"req_01B","type":"assistant","uuid":"7c0e5a10-0012-4a8e-9b1d-3f2a6c8d0012","timestamp":"2026-11-16T09:06:00.000Z"}"#,
    "\n",
);

/// A machine under `root` with its home at `root/home`, a channel
/// `payments`, and Claude and Copilot as agents that keep their
/// conversations in that home.
fn machine(root: &Path) -> Starter {
    let store = Arc::new(Store::open(root.join("state")).unwrap());
    store
        .put_channel(&ChannelRecord {
            id: "ch_1".to_owned(),
            name: "Payments".to_owned(),
            folder: "payments".to_owned(),
            branch_prefix: None,
            default_agent: "claude".to_owned(),
            repos: vec![ChannelRepo { owner: "acme".to_owned(), name: "api".to_owned(), main: MainClone::Ready }],
            machines: Vec::new(),
            revision: 1,
            created_unix_ms: 1,
        })
        .unwrap();
    let agents = r#"[
        {"name":"claude","program":"/bin/sh","start_args":[],"resume_args":[],
         "conversation_file":"{home}/.claude/projects/{folder_dashed}/{conversation}.jsonl",
         "conversation_format":"claude-jsonl","version_args":[]},
        {"name":"copilot","program":"/bin/sh","start_args":[],"resume_args":[],
         "conversation_file":"{home}/.copilot/session-state/{conversation}/events.jsonl",
         "conversation_format":"copilot-session-state","version_args":[]}
    ]"#;
    let home = root.join("home");
    Starter {
        store,
        layout: Layout::new(root.join("Humata")),
        remote_template: String::new(),
        agents: Agents::from_json(agents.as_bytes()).unwrap(),
        home: home.clone(),
        revive: ReviveConfig {
            contain: None,
            builds: None,
            versions: None,
            gate: None,
            holder: PathBuf::from("/nonexistent"),
            logs: root.join("logs"),
            scrollback_bytes: 1 << 16,
            shell: PathBuf::from("/bin/sh"),
            shell_args: Vec::new(),
            home,
            resume_timeout: Duration::from_secs(1),
            settle: Duration::from_millis(10),
            reports: RestartReports {
                folder: root.join("restarts"),
                version: "0".to_owned(),
                machine: "ma".to_owned(),
                os: "test".to_owned(),
                daemon_log: None,
            },
        },
        holders: root.join("h"),
        first_message: Default::default(),
    }
}

fn fill(template: &str, starter: &Starter, folder: &Path, id: &str) -> String {
    template
        .replace("{folder}", &folder.to_string_lossy())
        .replace("{home}", &starter.home.to_string_lossy())
        .replace("{id}", id)
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// Session `se_1` on `starter` running `agent` in conversation `ID`, its
/// folder holding a working file. Returns the folder and the conversation
/// file.
fn session(starter: &Starter, agent: &str) -> (PathBuf, PathBuf) {
    let folder = starter.layout.session("payments", "fix");
    write(&folder.join("api/work.txt"), "dirty\n");
    starter
        .store
        .put_session(&SessionRecord {
            id: "se_1".to_owned(),
            channel: "ch_1".to_owned(),
            machine: String::new(),
            title: "Fix".to_owned(),
            agent: agent.to_owned(),
            folder: folder.clone(),
            created_unix_ms: 1,
        })
        .unwrap();
    let plan = starter.agents.get(agent).unwrap().launch(&starter.home, &folder, ID);
    starter
        .store
        .put_agent(&AgentRecord {
            session: "se_1".to_owned(),
            tab: "tb_old".to_owned(),
            launch: AgentLaunch {
                agent: agent.to_owned(),
                folder: folder.clone(),
                argv: plan.argv,
                resume_argv: plan.resume_argv,
                conversation_id: ID.to_owned(),
                conversation_file: plan.conversation_file.clone(),
                holder_socket: starter.holders.join("tb_old.sock"),
                ..AgentLaunch::default()
            },
            state: AgentState::Stopped,
            generation: 1,
            interruptions: Vec::new(),
            updated_unix_ms: 1,
        })
        .unwrap();
    (folder, plan.conversation_file)
}

/// A Claude session `se_1` on `starter`, and another conversation of its
/// folder with a subagent's transcript. Returns the folder, the session's
/// transcript and the other conversation's files.
fn claude_session(starter: &Starter) -> (PathBuf, PathBuf, Vec<PathBuf>) {
    let (folder, transcript) = session(starter, "claude");
    write(&transcript, &fill(TWO_TURNS, starter, &folder, ID));
    let project = transcript.parent().unwrap();
    let other = vec![project.join(format!("{OTHER}.jsonl")), project.join(OTHER).join("subagents/agent-1.jsonl")];
    write(&other[0], &fill(TWO_TURNS, starter, &folder, OTHER));
    write(&other[1], &fill(TWO_TURNS, starter, &folder, OTHER));
    (folder, transcript, other)
}

/// What the engine does between two machines: the session's folder and
/// the staged files appear under the new machine's channel.
fn copy_over(old: &Starter, new: &Starter, folder: &Path) {
    copy_tree(folder, &new.layout.sessions("payments").join("fix")).unwrap();
    let staged = old.layout.channel("payments").join(CARRY_DIR).join("op_1").join(STAGE);
    copy_tree(&staged, &new.incoming(&new.carry_channel("ch_1").unwrap(), "op_1")).unwrap();
}

/// Every plain file under `dir` with its bytes.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut found = BTreeMap::new();
    let mut folders = vec![dir.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let Ok(entries) = fs::read_dir(&folder) else { continue };
        for entry in entries {
            let path = entry.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                folders.push(path);
            } else if meta.is_file() {
                found.insert(path.clone(), fs::read(&path).unwrap());
            }
        }
    }
    found
}

/// The files under `dir` whose name or bytes hold `needle`.
fn files_naming(dir: &Path, needle: &str) -> Vec<PathBuf> {
    snapshot(dir)
        .into_iter()
        .filter(|(path, bytes)| {
            path.to_string_lossy().contains(needle) || String::from_utf8_lossy(bytes).contains(needle)
        })
        .map(|(path, _)| path)
        .collect()
}

#[test]
fn a_carry_stages_the_sessions_conversation_and_not_another_of_its_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = fs::canonicalize(tmp.path()).unwrap();
    let old = machine(&dir.join("old"));
    let (_, transcript, _) = claude_session(&old);

    old.pack("op_1", "se_1").unwrap();

    let stage = old.layout.channel("payments").join(CARRY_DIR).join("op_1").join(STAGE);
    assert_eq!(files_naming(&stage, OTHER), Vec::<PathBuf>::new(), "another conversation was staged");
    assert!(!files_naming(&stage, ID).is_empty(), "the session's own conversation is staged");
    assert!(transcript.exists());
}

#[test]
fn a_carried_conversation_names_no_old_path_and_reads_on_the_new_machine() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = fs::canonicalize(tmp.path()).unwrap();
    let (old, new) = (machine(&dir.join("old")), machine(&dir.join("new")));
    let (folder, _, other) = claude_session(&old);

    old.pack("op_1", "se_1").unwrap();
    copy_over(&old, &new, &folder);
    new.unpack("op_1", "se_1", "ch_1", Take::Everything).unwrap();

    let moved = new.layout.session("payments", "fix");
    let agent_home = new.home.join(".claude");
    let old_root = dir.join("old").to_string_lossy().into_owned();
    for needle in [old_root, dashed(&folder.to_string_lossy())] {
        assert_eq!(files_naming(&agent_home, &needle), Vec::<PathBuf>::new(), "{needle} is still named");
    }
    let at = Placement { home: new.home.clone(), cli_home: None, folder: moved };
    let found = adapter(Format::ClaudeJsonl).locate(&at, ID).unwrap();
    adapter(Format::ClaudeJsonl).check(&found, Some(CLAUDE_VERSION)).unwrap();
    assert_eq!(adapter(Format::ClaudeJsonl).turns(&found).unwrap().len(), 2);
    let transcript = fs::read_to_string(&found.files[0].path).unwrap();
    assert!(transcript.contains(&format!("{}/notes/login.md", new.home.display())), "the home is the new one");
    assert_eq!(new.store.agent("se_1").unwrap().unwrap().launch.conversation_file, found.files[0].path);

    // The other conversation of the old folder stayed where it was.
    for path in &other {
        assert!(path.exists(), "{} is still on the old machine", path.display());
    }
    assert_eq!(files_naming(&agent_home, OTHER), Vec::<PathBuf>::new(), "another conversation was carried");
}

#[test]
fn the_old_machine_discards_the_sessions_conversation_and_not_another_of_its_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = fs::canonicalize(tmp.path()).unwrap();
    let old = machine(&dir.join("old"));
    let (_, transcript, other) = claude_session(&old);

    let found = old.discards("se_1").unwrap();

    assert!(found.iter().any(|discard| transcript.starts_with(&discard.path)), "the conversation is discarded");
    for discard in &found {
        for path in &other {
            assert!(!path.starts_with(&discard.path), "{} would go with {}", path.display(), discard.path.display());
        }
    }
}

/// Two turns as Copilot CLI 1.0.112-0 writes them (the conversation crate's
/// `testdata/copilot/1.0.112-0/two_turns.jsonl`), cut to the records the
/// turns are read from; `{folder}` and `{id}` are filled.
const COPILOT_TWO_TURNS: &str = concat!(
    r#"{"type":"session.start","data":{"sessionId":"{id}","version":1,"producer":"copilot-agent","copilotVersion":"1.0.112-0","startTime":"2026-11-17T02:42:42.725Z","selectedModel":"gpt-4.1","contextTier":null,"context":{"cwd":"{folder}"},"alreadyInUse":false,"remoteSteerable":false},"id":"ccf77867-e1d1-42aa-a40e-3862c7cf317e","timestamp":"2026-11-17T02:42:43.111Z","parentId":null}"#,
    "\n",
    r#"{"type":"user.message","data":{"content":"Fix the failing payments test","messageId":"c67fd535-b1d9-470c-9b5f-266e9ca10ed1","turnId":"0"},"id":"4e63bab4-8cd7-48a3-a04d-79258ab044cd","timestamp":"2026-11-17T02:42:43.615Z","parentId":"ccf77867-e1d1-42aa-a40e-3862c7cf317e"}"#,
    "\n",
    r#"{"type":"assistant.turn_start","data":{"turnId":"0","agentMode":"interactive"},"id":"66ba045d-bed7-4c16-8cec-c8dc162061d2","timestamp":"2026-11-17T02:42:43.701Z","parentId":"4e63bab4-8cd7-48a3-a04d-79258ab044cd"}"#,
    "\n",
    r#"{"type":"assistant.message","data":{"messageId":"7af06098-43c1-40b4-bd5a-d3f7cb777ca4","content":"Fixed the assertion in the payments test.","turnId":"0"},"id":"e27e8443-82da-4a63-9ca9-8a5ed55db405","timestamp":"2026-11-17T02:42:43.790Z","parentId":"66ba045d-bed7-4c16-8cec-c8dc162061d2"}"#,
    "\n",
    r#"{"type":"assistant.turn_end","data":{"turnId":"0"},"id":"4d487e5f-a1e8-4b56-9fc8-7ef755139952","timestamp":"2026-11-17T02:42:43.828Z","parentId":"e27e8443-82da-4a63-9ca9-8a5ed55db405"}"#,
    "\n",
    r#"{"type":"session.resume","data":{"resumeTime":"2026-11-17T02:42:47.001Z","eventCount":5,"selectedModel":"gpt-4.1","context":{"cwd":"{folder}"},"alreadyInUse":false,"sessionWasActive":false},"id":"f5f24644-bd0d-4077-8630-e5d87aeae693","timestamp":"2026-11-17T02:42:47.001Z","parentId":"4d487e5f-a1e8-4b56-9fc8-7ef755139952"}"#,
    "\n",
    r#"{"type":"user.message","data":{"content":"Now add a test for the refund path","messageId":"506051ec-b7cc-4efe-bf93-a887ee968c36","turnId":"0"},"id":"be9b7c01-6fce-48e6-b541-adfa23c9f8ba","timestamp":"2026-11-17T02:42:48.413Z","parentId":"f5f24644-bd0d-4077-8630-e5d87aeae693"}"#,
    "\n",
    r#"{"type":"assistant.turn_start","data":{"turnId":"0","agentMode":"interactive"},"id":"92f65f15-05c8-46e6-92a3-23624f3dacf5","timestamp":"2026-11-17T02:42:48.429Z","parentId":"be9b7c01-6fce-48e6-b541-adfa23c9f8ba"}"#,
    "\n",
    r#"{"type":"assistant.message","data":{"messageId":"8ca1be9f-34d8-4e10-9016-9aa1211e1dd5","content":"Added a refund path test.","turnId":"0"},"id":"530a4f2b-c314-4cb8-9a86-3af8d1b299b2","timestamp":"2026-11-17T02:42:48.560Z","parentId":"92f65f15-05c8-46e6-92a3-23624f3dacf5"}"#,
    "\n",
    r#"{"type":"assistant.turn_end","data":{"turnId":"0"},"id":"c8a7c560-c68d-44b7-8be1-9ff591e32c8e","timestamp":"2026-11-17T02:42:48.590Z","parentId":"530a4f2b-c314-4cb8-9a86-3af8d1b299b2"}"#,
    "\n",
);

#[test]
fn a_carried_copilot_conversation_names_no_old_path_and_reads_on_the_new_machine() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = fs::canonicalize(tmp.path()).unwrap();
    let (old, new) = (machine(&dir.join("old")), machine(&dir.join("new")));
    let (folder, events) = session(&old, "copilot");
    write(&events, &fill(COPILOT_TWO_TURNS, &old, &folder, ID));

    old.pack("op_1", "se_1").unwrap();
    copy_over(&old, &new, &folder);
    new.unpack("op_1", "se_1", "ch_1", Take::Everything).unwrap();

    let old_root = dir.join("old").to_string_lossy().into_owned();
    let state = new.home.join(".copilot");
    assert_eq!(files_naming(&state, &old_root), Vec::<PathBuf>::new(), "the old machine is still named");
    let at = Placement { home: new.home.clone(), cli_home: None, folder: new.layout.session("payments", "fix") };
    let copilot = adapter(Format::CopilotSessionState);
    let found = copilot.locate(&at, ID).unwrap();
    copilot.check(&found, Some("1.0.112-0")).unwrap();
    let turns = copilot.turns(&found).unwrap();
    let firsts: Vec<&str> = turns.iter().map(|turn| turn.first_line.as_str()).collect();
    assert_eq!(firsts, ["Fix the failing payments test", "Now add a test for the refund path"]);
    let moved = at.folder.to_string_lossy().into_owned();
    assert!(!files_naming(&state, &moved).is_empty(), "the session state names the new folder");
}

/// A Copilot conversation with a record its adapter doesn't know (a
/// start naming the folder in a field Copilot 1.0.112-0 doesn't write).
#[test]
fn a_copilot_conversation_its_adapter_does_not_know_is_refused_before_anything_is_staged() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = fs::canonicalize(tmp.path()).unwrap();
    let old = machine(&dir.join("old"));
    let (folder, events) = session(&old, "copilot");
    write(&events, &format!("{{\"type\":\"session.start\",\"data\":{{\"cwd\":\"{}\"}}}}\n", folder.display()));
    let (home_before, folder_before) = (snapshot(&old.home), snapshot(&folder));

    let refused = old.pack("op_1", "se_1").unwrap_err();

    assert!(matches!(refused, CarryError::Conversation { .. }), "{refused:?}");
    let carry = old.layout.channel("payments").join(CARRY_DIR);
    assert_eq!(snapshot(&carry), BTreeMap::new(), "nothing was staged");
    assert_eq!(snapshot(&old.home), home_before, "the conversation is as it was");
    assert_eq!(snapshot(&folder), folder_before, "the folder is as it was");
    assert!(old.store.session("se_1").unwrap().is_some());
}

/// A Copilot session whose CLI wrote no session state yet carries its
/// folder alone: there is no conversation to take.
#[test]
fn a_copilot_session_with_no_session_state_yet_carries_its_folder_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = fs::canonicalize(tmp.path()).unwrap();
    let (old, new) = (machine(&dir.join("old")), machine(&dir.join("new")));
    let (folder, events) = session(&old, "copilot");
    assert!(!events.exists());

    old.pack("op_1", "se_1").unwrap();
    copy_over(&old, &new, &folder);
    new.unpack("op_1", "se_1", "ch_1", Take::Everything).unwrap();

    let moved = new.layout.session("payments", "fix");
    assert_eq!(fs::read_to_string(moved.join("api/work.txt")).unwrap(), "dirty\n");
    assert_eq!(snapshot(&new.home.join(".copilot")), BTreeMap::new(), "no conversation was made up");
    assert!(new.store.session("se_1").unwrap().is_some());
}

#[test]
fn a_moved_session_takes_its_earlier_conversations_and_the_old_machine_discards_them() {
    const FIRST: &str = "1a2b3c4d-0001-4a5b-8c6d-7e8f9a0b1c2d";
    const SECOND: &str = "1a2b3c4d-0002-4a5b-8c6d-7e8f9a0b1c2d";
    let tmp = tempfile::tempdir().unwrap();
    let dir = fs::canonicalize(tmp.path()).unwrap();
    let (old, new) = (machine(&dir.join("old")), machine(&dir.join("new")));
    let (folder, transcript, other) = claude_session(&old);
    let project = transcript.parent().unwrap().to_path_buf();
    let earlier: Vec<PathBuf> = [FIRST, SECOND]
        .iter()
        .enumerate()
        .map(|(n, id)| {
            let path = project.join(format!("{id}.jsonl"));
            write(&path, &fill(TWO_TURNS, &old, &folder, id));
            old.store.push_previous_conversation("se_1", id, &path, i64::try_from(n).unwrap() + 1).unwrap();
            path
        })
        .collect();

    old.pack("op_1", "se_1").unwrap();
    copy_over(&old, &new, &folder);
    new.unpack("op_1", "se_1", "ch_1", Take::Everything).unwrap();

    let at = Placement { home: new.home.clone(), cli_home: None, folder: new.layout.session("payments", "fix") };
    let rows = new.store.previous_conversations("se_1").unwrap();
    assert_eq!(rows.iter().map(|row| row.conversation_id.as_str()).collect::<Vec<_>>(), [FIRST, SECOND]);
    for row in &rows {
        let found = adapter(Format::ClaudeJsonl).locate(&at, &row.conversation_id).unwrap();
        assert_eq!(row.path, found.files[0].path, "the row names the placed transcript");
        assert!(row.path.exists());
        assert!(row.path.starts_with(new.home.join(".claude")));
    }
    let agent_home = new.home.join(".claude");
    assert_eq!(files_naming(&agent_home, &dashed(&folder.to_string_lossy())), Vec::<PathBuf>::new());
    assert_eq!(files_naming(&agent_home, OTHER), Vec::<PathBuf>::new(), "another conversation was carried");

    // Nothing of the session stays behind except what the old machine trashes.
    let discards = old.discards("se_1").unwrap();
    for path in earlier.iter().chain(std::iter::once(&transcript)) {
        assert!(discards.iter().any(|discard| path.starts_with(&discard.path)), "{} isn't discarded", path.display());
    }
    for path in &other {
        assert!(discards.iter().all(|discard| !path.starts_with(&discard.path)));
    }
}

#[test]
fn earlier_conversations_travel_when_the_current_one_has_no_files() {
    const FIRST: &str = "1a2b3c4d-0003-4a5b-8c6d-7e8f9a0b1c2d";
    let tmp = tempfile::tempdir().unwrap();
    let dir = fs::canonicalize(tmp.path()).unwrap();
    let (old, new) = (machine(&dir.join("old")), machine(&dir.join("new")));
    let (folder, transcript) = session(&old, "claude");
    assert!(!transcript.exists(), "the current conversation has no files");
    let path = transcript.parent().unwrap().join(format!("{FIRST}.jsonl"));
    write(&path, &fill(TWO_TURNS, &old, &folder, FIRST));
    old.store.push_previous_conversation("se_1", FIRST, &path, 1).unwrap();

    old.pack("op_1", "se_1").unwrap();
    copy_over(&old, &new, &folder);
    new.unpack("op_1", "se_1", "ch_1", Take::Everything).unwrap();

    let at = Placement { home: new.home.clone(), cli_home: None, folder: new.layout.session("payments", "fix") };
    let found = adapter(Format::ClaudeJsonl).locate(&at, FIRST).unwrap();
    let rows = new.store.previous_conversations("se_1").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].path, found.files[0].path, "the row names the placed transcript");
    assert!(rows[0].path.exists());
    let discards = old.discards("se_1").unwrap();
    assert!(discards.iter().any(|discard| path.starts_with(&discard.path)), "the old copy is discarded");
}

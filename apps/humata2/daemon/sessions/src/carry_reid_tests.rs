//! A fork staged for another machine carries its own conversation id inside
//! the files (CV-2, FK-8), as a fork on this machine does: every record of
//! the staged transcript names the fork's id and none names the parent's,
//! and the parent's files are as they were. A plain carry of a session
//! keeps the id it had.

use std::sync::Arc;
use std::time::Duration;

use humata2_revive::{ReviveConfig, RestartReports};
use humata2_store::{
    AgentLaunch, AgentRecord, ChannelRecord, ChannelRepo, MainClone, SessionDetail, SessionRecord, StartStep, Store,
};

use super::*;
use crate::agents::Agents;
use crate::fork::ForkCopy;
use crate::layout::Layout;

/// The parent session's conversation.
const ID: &str = "4f6d2c1e-8a3b-4c5d-9e7f-1a2b3c4d5e6f";

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

/// One finished turn of a Codex rollout as codex-cli 0.169.0 writes it (the
/// conversation crate's `testdata/codex/three_turns.jsonl`, its first
/// fourteen lines); `{codex}`, `{folder}` and `{id}` are filled.
const CODEX_ONE_TURN: &str = r####"{"timestamp":"2026-11-16T21:48:57.265Z","ordinal":0,"type":"session_meta","payload":{"account_scoped":true,"session_id":"{id}","id":"{id}","timestamp":"2026-11-16T21:48:56.953Z","cwd":"{folder}","runtime_workspace_roots":["{folder}"],"originator":"codex_exec","cli_version":"0.169.0","source":"exec","thread_source":"user","model_provider":"fixture","base_instructions":{"text":"You are a coding agent running in a terminal. Keep changes small and explain them.","provenance":{"type":"model","model":"fixture-layout"}},"history_mode":"paginated","context_window":{"window_id":"01a1e655-acbb-72a2-8790-a870d884381a"},"git":{"commit_hash":"3f2a6c8d0e1b4a5c9d7e8f0a1b2c3d4e5f607182","branch":"main","repository_url":"https://git.example.com/team/web.git"}}}
{"timestamp":"2026-11-16T21:48:57.269Z","ordinal":1,"type":"event_msg","payload":{"type":"task_started","turn_attribution":{"turn_id":"01a1e655-add5-79e0-91ea-3d39f626a868","turn_trigger":"exec","parent_turn_id":null,"initiating_agent_path":null,"root_turn_id":"01a1e655-add5-79e0-91ea-3d39f626a868"},"turn_id":"01a1e655-add5-79e0-91ea-3d39f626a868","root_turn_id":"01a1e655-add5-79e0-91ea-3d39f626a868","started_at":1794865737,"model_context_window":258400,"collaboration_mode_kind":"default"}}
{"timestamp":"2026-11-16T21:48:57.376Z","ordinal":2,"type":"response_item","payload":{"type":"message","id":"msg_01a1e655-ae98-7b81-8dd6-dec4c61e65b2","role":"developer","content":[{"type":"input_text","text":"<skills_instructions>\n## Skills\nA skill is a set of local instructions stored in a `SKILL.md` file.\n### Skill roots\n- `r0` = `{codex}/skills/.system`\n### Available skills\n- skill-creator: Create or update a skill. (file: r0/skill-creator/SKILL.md)\n</skills_instructions>"},{"type":"input_text","text":"<permissions instructions>\nFilesystem sandboxing defines which files can be read or written. `sandbox_mode` is `read-only`: The sandbox only permits reading files. Network access is restricted.\nApproval policy is currently never. Do not provide the `sandbox_permissions` for any reason, commands will be rejected.\n</permissions instructions>"}],"internal_chat_message_metadata_passthrough":{"turn_id":"01a1e655-add5-79e0-91ea-3d39f626a868","create_time":1794865737.368328,"content_item_kinds":["host_skills.instructions","permissions.instructions"],"content_item_metadata":[{"provenance":{"type":"skills"}},{"provenance":{"type":"harness","hook":false}}]}},"metadata":{"history_sequence":1,"sampled_through_sequence":0,"client_authored":false,"mcp_attribution":{"status":"none"}}}
{"timestamp":"2026-11-16T21:48:57.376Z","ordinal":3,"type":"response_item","payload":{"type":"message","id":"msg_01a1e655-ae98-7b81-8dd6-ded0ae6f884a","role":"user","content":[{"type":"input_text","text":"# AGENTS.md instructions for {folder}\n\n<INSTRUCTIONS>\n# Agent guidelines\n\n- Run the tests before you say a change is done.\n</INSTRUCTIONS>"},{"type":"input_text","text":"<environment_context>\n  <cwd>{folder}</cwd>\n  <shell>zsh</shell>\n  <current_date>2026-11-16</current_date>\n  <timezone>America/Los_Angeles</timezone>\n  <filesystem><workspace_roots><root>{folder}</root></workspace_roots><permission_profile type=\"managed\"><file_system type=\"restricted\"><entry access=\"read\"><special>:root</special></entry></file_system></permission_profile></filesystem>\n</environment_context>"}],"internal_chat_message_metadata_passthrough":{"turn_id":"01a1e655-add5-79e0-91ea-3d39f626a868","create_time":1794865737.368894,"content_item_kinds":["agents_md.instructions","environments.environment_context"],"content_item_metadata":[{"provenance":{"type":"agents_md"}},{"provenance":{"type":"harness","hook":false}}]}},"metadata":{"history_sequence":2,"sampled_through_sequence":0,"client_authored":false}}
{"timestamp":"2026-11-16T21:48:57.385Z","ordinal":4,"type":"world_state","payload":{"full":true,"state":{"agents_md":{"directory":"{folder}","text":"# Agent guidelines\n\n- Run the tests before you say a change is done.\n"},"apps_instructions":false,"collaboration_mode":{"mode":"default","model":"fixture-layout","instructions":"dfd53114f5f6f9f5fd7816370d0bf2847806246d"},"context_window_guidance":"","environments":{"environments":{"local":{"cwd":"{folder}","status":"available","shell":"zsh"}},"current_date":"2026-11-16","timezone":"America/Los_Angeles","filesystem":"<filesystem><workspace_roots><root>{folder}</root></workspace_roots><permission_profile type=\"managed\"><file_system type=\"restricted\"><entry access=\"read\"><special>:root</special></entry></file_system></permission_profile></filesystem>"},"environments_instructions":false,"git_attribution":false,"host_skills":{"body":"\n## Skills\nA skill is a set of local instructions stored in a `SKILL.md` file.\n### Skill roots\n- `r0` = `{codex}/skills/.system`\n","includeInstructions":true},"managed_developer_instructions":{},"model":"fixture-layout","model_catalog":"ea1a55de2973a50e2e404222c5499c24df5591ed","multi_agent_mode":{},"permissions":{"instructions":"fed2f53df24dd05a217b734cf7ad9abefa7cc6a2","approved_command_prefixes":[]},"persistent_mode":{},"plugins_instructions":false,"realtime":{"active":false},"skills":{"includeInstructions":true}}}}
{"timestamp":"2026-11-16T21:48:57.389Z","ordinal":5,"type":"turn_context","payload":{"turn_id":"01a1e655-add5-79e0-91ea-3d39f626a868","root_turn_id":"01a1e655-add5-79e0-91ea-3d39f626a868","disabled_plugin_ids":[],"cwd":"{folder}","approval_policy":"never","approvals_reviewer":"user","permission_profile":{"type":"managed","file_system":{"type":"restricted","entries":[{"path":{"type":"special","value":{"kind":"root"}},"access":"read"}]},"network":"restricted"},"active_permission_profile":{"id":":read-only"},"model":"fixture-layout","collaboration_mode":{"mode":"default","settings":{"model":"fixture-layout","reasoning_effort":null,"developer_instructions":null}},"multi_agent_version":"v1","realtime_active":false,"summary":"none","sandbox_policy":{"type":"read-only"}}}
{"timestamp":"2026-11-16T21:48:57.389Z","ordinal":6,"type":"event_msg","payload":{"type":"thread_settings_applied","thread_id":"{id}","thread_settings":{"model":"fixture-layout","model_provider_id":"fixture","approval_policy":"never","approvals_reviewer":"user","requested_approvals_reviewer":"user","permission_profile":{"type":"managed","file_system":{"type":"restricted","entries":[{"path":{"type":"special","value":{"kind":"root"}},"access":"read"}]},"network":"restricted"},"active_permission_profile":{"id":":read-only"},"cwd":"{folder}","runtime_workspace_roots":["{folder}"],"collaboration_mode":{"mode":"default","settings":{"model":"fixture-layout","reasoning_effort":null,"developer_instructions":null}},"disabled_plugin_ids":[]}}}
{"timestamp":"2026-11-16T21:48:57.410Z","ordinal":7,"type":"response_item","payload":{"type":"message","id":"msg_01a1e655-aec0-7d31-a119-e89ae1668e0f","role":"user","content":[{"type":"input_text","text":"Fix the login redirect"}],"internal_chat_message_metadata_passthrough":{"turn_id":"01a1e655-add5-79e0-91ea-3d39f626a868","create_time":1794865737.408862,"content_item_kinds":["user.text"],"content_item_metadata":[{"provenance":{"type":"user"}}]}},"metadata":{"history_sequence":3,"sampled_through_sequence":0,"retained_source":{"id":{"message_id":"msg_01a1e655-aec0-7d31-a119-e89ae1668e0f","turn_id":"01a1e655-add5-79e0-91ea-3d39f626a868","role":"user"},"revision":"retained_01a1e655-aec2-75e0-8912-f6dce8cf4458","complete":true},"client_authored":false,"user_input_order":0,"mcp_attribution":{"status":"none"}}}
{"timestamp":"2026-11-16T21:48:57.415Z","ordinal":8,"type":"event_msg","payload":{"type":"item_completed","thread_id":"{id}","turn_id":"01a1e655-add5-79e0-91ea-3d39f626a868","item":{"type":"UserMessage","id":"01a1e655-aec0-7d31-a119-e8aa30679f0f","content":[{"type":"text","text":"Fix the login redirect","text_elements":[]}]},"started_at_ms":1794865737415,"completed_at_ms":1794865737415}}
{"timestamp":"2026-11-16T21:48:57.459Z","ordinal":9,"type":"event_msg","payload":{"type":"item_completed","thread_id":"{id}","turn_id":"01a1e655-add5-79e0-91ea-3d39f626a868","item":{"type":"AgentMessage","id":"msg_fixture_1","content":[{"type":"Text","text":"The redirect now goes back to the page you came from ({folder}/web/src/login.ts)."}],"phase":"final_answer"},"started_at_ms":1794865737459,"completed_at_ms":1794865737459}}
{"timestamp":"2026-11-16T21:48:57.461Z","ordinal":10,"type":"response_item","payload":{"type":"message","status":"completed","id":"msg_fixture_1","role":"assistant","content":[{"type":"output_text","text":"The redirect now goes back to the page you came from ({folder}/web/src/login.ts).","annotations":[]}],"phase":"final_answer","internal_chat_message_metadata_passthrough":{"turn_id":"01a1e655-add5-79e0-91ea-3d39f626a868","content_item_kinds":["unknown"]}},"metadata":{"history_sequence":4,"sampled_through_sequence":3,"retained_source":{"id":{"message_id":"msg_fixture_1","turn_id":"01a1e655-add5-79e0-91ea-3d39f626a868","role":"assistant"},"revision":"retained_01a1e655-aef5-7a01-826b-475a1944721a","complete":true},"client_authored":false,"user_input_order":1}}
{"timestamp":"2026-11-16T21:48:57.462Z","ordinal":11,"type":"token_usage_record","payload":{"thread_id":"{id}","turn_id":"01a1e655-add5-79e0-91ea-3d39f626a868","session_id":"{id}","root_turn_id":"01a1e655-add5-79e0-91ea-3d39f626a868","response_id":"resp_fixture_1","usage":{"input_tokens":1,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":1,"reasoning_output_tokens":0,"total_tokens":2},"turn_token_usage":{"input_tokens":1,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":1,"reasoning_output_tokens":0,"total_tokens":2},"thread_token_usage":{"input_tokens":1,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":1,"reasoning_output_tokens":0,"total_tokens":2}}}
{"timestamp":"2026-11-16T21:48:57.463Z","ordinal":12,"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":1,"reasoning_output_tokens":0,"total_tokens":2},"last_token_usage":{"input_tokens":1,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":1,"reasoning_output_tokens":0,"total_tokens":2},"last_usage_includes_reasoning":false,"model_context_window":258400},"rate_limits":{"limit_id":"codex","limit_name":null,"primary":null,"secondary":null,"credits":null,"individual_limit":null,"spend_control_reached":null,"plan_type":null,"rate_limit_reached_type":null}}}
{"timestamp":"2026-11-16T21:48:57.468Z","ordinal":13,"type":"event_msg","payload":{"type":"task_complete","root_turn_id":"01a1e655-add5-79e0-91ea-3d39f626a868","turn_id":"01a1e655-add5-79e0-91ea-3d39f626a868","last_agent_message":"The redirect now goes back to the page you came from ({folder}/web/src/login.ts).","started_at":1794865737,"completed_at":1794865737,"duration_ms":279,"time_to_first_token_ms":270}}
"####;

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
        {"name":"codex","program":"/bin/sh","start_args":[],"resume_args":[],
         "conversation_file":"{home}/.codex/sessions/2026/11/16/rollout-2026-11-16T13-48-56-{conversation}.jsonl",
         "conversation_format":"codex-rollout","version_args":[]},
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
        .replace("{codex}", &starter.home.join(".codex").to_string_lossy())
        .replace("{id}", id)
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// Session `se_1` of `starter` running `agent` in conversation `ID`, whose
/// transcript holds `transcript` (with `{folder}`, `{home}` and `{id}` filled).
/// Returns the transcript's path.
fn parent(starter: &Starter, agent: &str, transcript: &str) -> PathBuf {
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
    starter
        .store
        .put_session_detail(&SessionDetail {
            session: "se_1".to_owned(),
            repos: Vec::new(),
            model: None,
            first_message: None,
            start: StartStep::Done,
            operation: None,
            revision: 1,
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
    write(&plan.conversation_file, &fill(transcript, starter, &folder, ID));
    plan.conversation_file
}

/// Stages a fork of `se_1` as `se_2` on `starter`, from before turn
/// `before_turn` (0: from now).
fn stage_fork(starter: &Starter, before_turn: u32) -> ForkPacked {
    starter
        .pack_fork(&ForkPack {
            operation: "op_1".to_owned(),
            parent: "se_1".to_owned(),
            session: "se_2".to_owned(),
            title: "Try again".to_owned(),
            copy: ForkCopy::CodeAndContext,
            before_turn,
            branch: None,
        })
        .unwrap()
}

/// The agent files of the operation's stage.
fn staged_agent(starter: &Starter) -> PathBuf {
    starter.layout.channel("payments").join(CARRY_DIR).join("op_1").join(STAGE).join(AGENT)
}

/// The JSON records of the staged transcript `path`.
fn records(path: &Path) -> Vec<serde_json::Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// Whether any file under `dir` holds `needle` in its name or bytes.
fn holds(dir: &Path, needle: &str) -> bool {
    snapshot(dir).iter().any(|(path, bytes)| {
        path.to_string_lossy().contains(needle) || String::from_utf8_lossy(bytes).contains(needle)
    })
}

#[test]
fn a_claude_fork_staged_for_another_machine_names_its_own_id_in_every_record() {
    let tmp = tempfile::tempdir().unwrap();
    let old = machine(&fs::canonicalize(tmp.path()).unwrap().join("old"));
    let transcript = parent(&old, "claude", TWO_TURNS);
    let (before, home_before) = (fs::read(&transcript).unwrap(), snapshot(&old.home));

    let forked = stage_fork(&old, 0);

    assert_ne!(forked.conversation_id, ID);
    let staged = staged_agent(&old).join(format!("{}.jsonl", forked.conversation_id));
    let found = records(&staged);
    assert_eq!(found.len(), 4, "every record of the conversation is there");
    for record in &found {
        assert_eq!(record["sessionId"], forked.conversation_id.as_str());
    }
    assert!(!holds(&staged_agent(&old), ID), "the parent's id is named nowhere in the staged files");
    let text = fs::read_to_string(&staged).unwrap();
    assert!(text.contains(&format!("{}/notes/login.md", old.home.display())), "the home is left for the new machine to place");
    assert_eq!(fs::read(&transcript).unwrap(), before, "the parent's transcript is as it was");
    assert_eq!(snapshot(&old.home), home_before, "nothing of the parent's home changed");
}

#[test]
fn a_fork_staged_from_before_a_turn_holds_only_the_turns_before_it_under_its_own_id() {
    let tmp = tempfile::tempdir().unwrap();
    let old = machine(&fs::canonicalize(tmp.path()).unwrap().join("old"));
    parent(&old, "claude", TWO_TURNS);

    let forked = stage_fork(&old, 2);

    let staged = staged_agent(&old).join(format!("{}.jsonl", forked.conversation_id));
    let found = records(&staged);
    let prompts = found.iter().filter(|record| record["type"] == "user").count();
    assert_eq!(prompts, 1, "only the first turn is carried");
    assert!(found.iter().all(|record| record["sessionId"] == forked.conversation_id.as_str()));
    assert!(!holds(&staged_agent(&old), ID));
}

#[test]
fn a_copilot_fork_staged_for_another_machine_names_its_own_id_in_its_session_start() {
    let tmp = tempfile::tempdir().unwrap();
    let old = machine(&fs::canonicalize(tmp.path()).unwrap().join("old"));
    let transcript = parent(&old, "copilot", COPILOT_TWO_TURNS);
    let (before, home_before) = (fs::read(&transcript).unwrap(), snapshot(&old.home));

    let forked = stage_fork(&old, 0);

    let found = records(&staged_agent(&old).join("events.jsonl"));
    assert_eq!(found[0]["type"], "session.start");
    assert_eq!(found[0]["data"]["sessionId"], forked.conversation_id.as_str());
    assert!(!holds(&staged_agent(&old), ID), "the parent's id is named nowhere in the staged files");
    assert_eq!(fs::read(&transcript).unwrap(), before, "the parent's events are as they were");
    assert_eq!(snapshot(&old.home), home_before, "nothing of the parent's home changed");
}

#[test]
fn a_codex_fork_staged_for_another_machine_names_its_own_id_in_its_session_meta() {
    let tmp = tempfile::tempdir().unwrap();
    let old = machine(&fs::canonicalize(tmp.path()).unwrap().join("old"));
    let transcript = parent(&old, "codex", CODEX_ONE_TURN);
    let (before, home_before) = (fs::read(&transcript).unwrap(), snapshot(&old.home));

    let forked = stage_fork(&old, 0);

    let staged: Vec<PathBuf> = fs::read_dir(staged_agent(&old)).unwrap().map(|entry| entry.unwrap().path()).collect();
    assert_eq!(staged.len(), 1, "the rollout is the one staged file");
    let found = records(&staged[0]);
    assert_eq!(found[0]["type"], "session_meta");
    assert_eq!(found[0]["payload"]["id"], forked.conversation_id.as_str());
    assert_eq!(found[0]["payload"]["session_id"], forked.conversation_id.as_str());
    assert!(!holds(&staged_agent(&old), ID), "the parent's id is named nowhere in the staged files");
    assert_eq!(fs::read(&transcript).unwrap(), before, "the parent's rollout is as it was");
    assert_eq!(snapshot(&old.home), home_before, "nothing of the parent's home changed");
}

#[test]
fn a_plain_carry_of_a_session_keeps_the_conversations_own_id() {
    let tmp = tempfile::tempdir().unwrap();
    let old = machine(&fs::canonicalize(tmp.path()).unwrap().join("old"));
    parent(&old, "claude", TWO_TURNS);

    old.pack("op_1", "se_1").unwrap();

    let staged = staged_agent(&old).join(format!("{ID}.jsonl"));
    let found = records(&staged);
    assert_eq!(found.len(), 4);
    assert!(found.iter().all(|record| record["sessionId"] == ID), "a move leaves the id alone");
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


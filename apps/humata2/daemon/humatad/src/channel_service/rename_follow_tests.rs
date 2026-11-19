//! MV-16: the conversation of a Codex agent whose working folder is two
//! levels under its clone follows a channel's rename.

use std::fs;

use humata2_store::{
    AgentLaunch, AgentRecord, AgentState, ChannelRecord, ChannelRepo, MainClone, SessionDetail, SessionRecord, SessionRepo,
    StartStep,
};

use super::*;

const ID: &str = "01a1e655-aca1-7d40-9236-87fee361b5eb";
const EARLIER: &str = "01a1e655-aca1-7d40-9236-87fee361b5ec";

/// The CLI's thread index (`state_5.sqlite`, table `threads`, the columns
/// codex-cli 0.169.0 has) with a row for each of `rows`: id, rollout, cwd.
fn write_threads(home: &Path, rows: &[(&str, &Path, &Path)]) -> PathBuf {
    let path = home.join(".codex/state_5.sqlite");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE threads (
                 id TEXT PRIMARY KEY,
                 rollout_path TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 source TEXT NOT NULL,
                 model_provider TEXT NOT NULL,
                 cwd TEXT NOT NULL,
                 title TEXT NOT NULL,
                 sandbox_policy TEXT NOT NULL,
                 approval_mode TEXT NOT NULL,
                 git_sha TEXT,
                 git_branch TEXT,
                 git_origin_url TEXT,
                 cli_version TEXT NOT NULL DEFAULT '',
                 first_user_message TEXT NOT NULL DEFAULT ''
             );",
        )
        .unwrap();
    for (id, rollout, cwd) in rows {
        connection
            .execute(
                "INSERT INTO threads (id, rollout_path, created_at, updated_at, source, model_provider, cwd, title,
                     sandbox_policy, approval_mode, cli_version, first_user_message)
                 VALUES (?1, ?2, 1, 2, 'exec', 'fixture', ?3, 'Fix', 'workspace-write', 'never', '0.169.0', 'Fix')",
                rusqlite::params![id, rollout.to_string_lossy(), cwd.to_string_lossy()],
            )
            .unwrap();
    }
    path
}

fn thread_cwd(db: &Path, id: &str) -> String {
    let connection = rusqlite::Connection::open(db).unwrap();
    connection.query_row("SELECT cwd FROM threads WHERE id = ?1", [id], |row| row.get(0)).unwrap()
}

/// A Codex rollout of three turns; `{codex}`, `{folder}` and `{id}` are filled.
const CODEX_THREE_TURNS: &str = r####"{"timestamp":"2026-11-16T21:48:57.265Z","ordinal":0,"type":"session_meta","payload":{"account_scoped":true,"session_id":"{id}","id":"{id}","timestamp":"2026-11-16T21:48:56.953Z","cwd":"{folder}","runtime_workspace_roots":["{folder}"],"originator":"codex_exec","cli_version":"0.169.0","source":"exec","thread_source":"user","model_provider":"fixture","base_instructions":{"text":"You are a coding agent running in a terminal. Keep changes small and explain them.","provenance":{"type":"model","model":"fixture-layout"}},"history_mode":"paginated","context_window":{"window_id":"01a1e655-acbb-72a2-8790-a870d884381a"},"git":{"commit_hash":"3f2a6c8d0e1b4a5c9d7e8f0a1b2c3d4e5f607182","branch":"main","repository_url":"https://git.example.com/team/web.git"}}}
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
{"timestamp":"2026-11-16T21:48:57.802Z","ordinal":14,"type":"event_msg","payload":{"type":"thread_settings_applied","thread_id":"{id}","thread_settings":{"model":"fixture-layout","model_provider_id":"fixture","approval_policy":"never","approvals_reviewer":"user","requested_approvals_reviewer":"user","permission_profile":{"type":"managed","file_system":{"type":"restricted","entries":[{"path":{"type":"special","value":{"kind":"root"}},"access":"read"}]},"network":"restricted"},"active_permission_profile":{"id":":read-only"},"cwd":"{folder}","runtime_workspace_roots":["{folder}"],"collaboration_mode":{"mode":"default","settings":{"model":"fixture-layout","reasoning_effort":null,"developer_instructions":null}},"disabled_plugin_ids":[]}}}
{"timestamp":"2026-11-16T21:48:57.809Z","ordinal":15,"type":"event_msg","payload":{"type":"task_started","turn_attribution":{"turn_id":"01a1e655-b049-7cb1-90cb-229217d5b99c","turn_trigger":"exec","parent_turn_id":null,"initiating_agent_path":null,"root_turn_id":"01a1e655-b049-7cb1-90cb-229217d5b99c"},"turn_id":"01a1e655-b049-7cb1-90cb-229217d5b99c","root_turn_id":"01a1e655-b049-7cb1-90cb-229217d5b99c","started_at":1794865737,"model_context_window":258400,"collaboration_mode_kind":"default"}}
{"timestamp":"2026-11-16T21:48:57.815Z","ordinal":16,"type":"turn_context","payload":{"turn_id":"01a1e655-b049-7cb1-90cb-229217d5b99c","root_turn_id":"01a1e655-b049-7cb1-90cb-229217d5b99c","disabled_plugin_ids":[],"cwd":"{folder}","approval_policy":"never","approvals_reviewer":"user","permission_profile":{"type":"managed","file_system":{"type":"restricted","entries":[{"path":{"type":"special","value":{"kind":"root"}},"access":"read"}]},"network":"restricted"},"active_permission_profile":{"id":":read-only"},"model":"fixture-layout","collaboration_mode":{"mode":"default","settings":{"model":"fixture-layout","reasoning_effort":null,"developer_instructions":null}},"multi_agent_version":"v1","realtime_active":false,"summary":"none","sandbox_policy":{"type":"read-only"}}}
{"timestamp":"2026-11-16T21:48:57.823Z","ordinal":17,"type":"response_item","payload":{"type":"message","id":"msg_01a1e655-b05f-7ca0-a99c-ef7a04129292","role":"user","content":[{"type":"input_text","text":"Now add a test for it\nCover the expired-cookie case too."}],"internal_chat_message_metadata_passthrough":{"turn_id":"01a1e655-b049-7cb1-90cb-229217d5b99c","create_time":1794865737.823213,"content_item_kinds":["user.text"],"content_item_metadata":[{"provenance":{"type":"user"}}]}},"metadata":{"history_sequence":5,"sampled_through_sequence":3,"retained_source":{"id":{"message_id":"msg_01a1e655-b05f-7ca0-a99c-ef7a04129292","turn_id":"01a1e655-b049-7cb1-90cb-229217d5b99c","role":"user"},"revision":"retained_01a1e655-b05f-7ca0-a99c-ef9ce76dc48e","complete":true},"client_authored":false,"user_input_order":2,"mcp_attribution":{"status":"none"}}}
{"timestamp":"2026-11-16T21:48:57.824Z","ordinal":18,"type":"event_msg","payload":{"type":"item_completed","thread_id":"{id}","turn_id":"01a1e655-b049-7cb1-90cb-229217d5b99c","item":{"type":"UserMessage","id":"01a1e655-b05f-7ca0-a99c-ef8b11f5d947","content":[{"type":"text","text":"Now add a test for it\nCover the expired-cookie case too.","text_elements":[]}]},"started_at_ms":1794865737824,"completed_at_ms":1794865737824}}
{"timestamp":"2026-11-16T21:48:57.832Z","ordinal":19,"type":"event_msg","payload":{"type":"item_completed","thread_id":"{id}","turn_id":"01a1e655-b049-7cb1-90cb-229217d5b99c","item":{"type":"AgentMessage","id":"msg_fixture_2","content":[{"type":"Text","text":"Added a test for the redirect and the expired cookie."}],"phase":"final_answer"},"started_at_ms":1794865737832,"completed_at_ms":1794865737832}}
{"timestamp":"2026-11-16T21:48:57.836Z","ordinal":20,"type":"response_item","payload":{"type":"message","status":"completed","id":"msg_fixture_2","role":"assistant","content":[{"type":"output_text","text":"Added a test for the redirect and the expired cookie.","annotations":[]}],"phase":"final_answer","internal_chat_message_metadata_passthrough":{"turn_id":"01a1e655-b049-7cb1-90cb-229217d5b99c","content_item_kinds":["unknown"]}},"metadata":{"history_sequence":6,"sampled_through_sequence":5,"retained_source":{"id":{"message_id":"msg_fixture_2","turn_id":"01a1e655-b049-7cb1-90cb-229217d5b99c","role":"assistant"},"revision":"retained_01a1e655-b06c-7690-a971-d9726aa6ad35","complete":true},"client_authored":false,"user_input_order":3}}
{"timestamp":"2026-11-16T21:48:57.836Z","ordinal":21,"type":"token_usage_record","payload":{"thread_id":"{id}","turn_id":"01a1e655-b049-7cb1-90cb-229217d5b99c","session_id":"{id}","root_turn_id":"01a1e655-b049-7cb1-90cb-229217d5b99c","response_id":"resp_fixture_2","usage":{"input_tokens":1,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":1,"reasoning_output_tokens":0,"total_tokens":2},"turn_token_usage":{"input_tokens":1,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":1,"reasoning_output_tokens":0,"total_tokens":2},"thread_token_usage":{"input_tokens":2,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":2,"reasoning_output_tokens":0,"total_tokens":4}}}
{"timestamp":"2026-11-16T21:48:57.837Z","ordinal":22,"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":2,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":2,"reasoning_output_tokens":0,"total_tokens":4},"last_token_usage":{"input_tokens":1,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":1,"reasoning_output_tokens":0,"total_tokens":2},"last_usage_includes_reasoning":false,"model_context_window":258400},"rate_limits":{"limit_id":"codex","limit_name":null,"primary":null,"secondary":null,"credits":null,"individual_limit":null,"spend_control_reached":null,"plan_type":null,"rate_limit_reached_type":null}}}
{"timestamp":"2026-11-16T21:48:57.839Z","ordinal":23,"type":"event_msg","payload":{"type":"task_complete","root_turn_id":"01a1e655-b049-7cb1-90cb-229217d5b99c","turn_id":"01a1e655-b049-7cb1-90cb-229217d5b99c","last_agent_message":"Added a test for the redirect and the expired cookie.","started_at":1794865737,"completed_at":1794865737,"duration_ms":30,"time_to_first_token_ms":23}}
{"timestamp":"2026-11-16T21:52:10.101Z","ordinal":24,"type":"event_msg","payload":{"type":"task_started","turn_attribution":{"turn_id":"01a1e655-b3c2-7e40-8a11-5c2d7e9f0a33","turn_trigger":"exec","parent_turn_id":null,"initiating_agent_path":null,"root_turn_id":"01a1e655-b3c2-7e40-8a11-5c2d7e9f0a33"},"turn_id":"01a1e655-b3c2-7e40-8a11-5c2d7e9f0a33","root_turn_id":"01a1e655-b3c2-7e40-8a11-5c2d7e9f0a33","started_at":1794865930,"model_context_window":258400,"collaboration_mode_kind":"default"}}
{"timestamp":"2026-11-16T21:52:10.107Z","ordinal":25,"type":"turn_context","payload":{"turn_id":"01a1e655-b3c2-7e40-8a11-5c2d7e9f0a33","root_turn_id":"01a1e655-b3c2-7e40-8a11-5c2d7e9f0a33","disabled_plugin_ids":[],"cwd":"{folder}","approval_policy":"never","approvals_reviewer":"user","permission_profile":{"type":"managed","file_system":{"type":"restricted","entries":[{"path":{"type":"special","value":{"kind":"root"}},"access":"read"}]},"network":"restricted"},"active_permission_profile":{"id":":read-only"},"model":"fixture-layout","collaboration_mode":{"mode":"default","settings":{"model":"fixture-layout","reasoning_effort":null,"developer_instructions":null}},"multi_agent_version":"v1","realtime_active":false,"summary":"none","sandbox_policy":{"type":"read-only"}}}
{"timestamp":"2026-11-16T21:52:10.115Z","ordinal":26,"type":"response_item","payload":{"type":"message","id":"msg_01a1e655-b3d1-7a20-b2c4-6e8f0a1b3c55","role":"user","content":[{"type":"input_text","text":"Run the tests"}],"internal_chat_message_metadata_passthrough":{"turn_id":"01a1e655-b3c2-7e40-8a11-5c2d7e9f0a33","create_time":1794865930.115213,"content_item_kinds":["user.text"],"content_item_metadata":[{"provenance":{"type":"user"}}]}},"metadata":{"history_sequence":7,"sampled_through_sequence":5,"retained_source":{"id":{"message_id":"msg_01a1e655-b3d1-7a20-b2c4-6e8f0a1b3c55","turn_id":"01a1e655-b3c2-7e40-8a11-5c2d7e9f0a33","role":"user"},"revision":"retained_01a1e655-b3d3-7c42-94e6-8a0b2c3d5e77","complete":true},"client_authored":false,"user_input_order":4,"mcp_attribution":{"status":"none"}}}
{"timestamp":"2026-11-16T21:52:10.116Z","ordinal":27,"type":"event_msg","payload":{"type":"item_completed","thread_id":"{id}","turn_id":"01a1e655-b3c2-7e40-8a11-5c2d7e9f0a33","item":{"type":"UserMessage","id":"01a1e655-b3d2-7b31-83d5-7f9a1b2c4d66","content":[{"type":"text","text":"Run the tests","text_elements":[]}]},"started_at_ms":1794865930116,"completed_at_ms":1794865930116}}
{"timestamp":"2026-11-16T21:52:14.402Z","ordinal":28,"type":"event_msg","payload":{"type":"item_completed","thread_id":"{id}","turn_id":"01a1e655-b3c2-7e40-8a11-5c2d7e9f0a33","item":{"type":"AgentMessage","id":"msg_fixture_3","content":[{"type":"Text","text":"All 42 tests pass."}],"phase":"final_answer"},"started_at_ms":1794865934402,"completed_at_ms":1794865934402}}
{"timestamp":"2026-11-16T21:52:14.406Z","ordinal":29,"type":"response_item","payload":{"type":"message","status":"completed","id":"msg_fixture_3","role":"assistant","content":[{"type":"output_text","text":"All 42 tests pass.","annotations":[]}],"phase":"final_answer","internal_chat_message_metadata_passthrough":{"turn_id":"01a1e655-b3c2-7e40-8a11-5c2d7e9f0a33","content_item_kinds":["unknown"]}},"metadata":{"history_sequence":8,"sampled_through_sequence":7,"retained_source":{"id":{"message_id":"msg_fixture_3","turn_id":"01a1e655-b3c2-7e40-8a11-5c2d7e9f0a33","role":"assistant"},"revision":"retained_01a1e655-b4f0-7d53-a5f7-9b1c3d4e6f88","complete":true},"client_authored":false,"user_input_order":5}}
{"timestamp":"2026-11-16T21:52:14.407Z","ordinal":30,"type":"token_usage_record","payload":{"thread_id":"{id}","turn_id":"01a1e655-b3c2-7e40-8a11-5c2d7e9f0a33","session_id":"{id}","root_turn_id":"01a1e655-b3c2-7e40-8a11-5c2d7e9f0a33","response_id":"resp_fixture_3","usage":{"input_tokens":1,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":1,"reasoning_output_tokens":0,"total_tokens":2},"turn_token_usage":{"input_tokens":1,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":1,"reasoning_output_tokens":0,"total_tokens":2},"thread_token_usage":{"input_tokens":3,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":3,"reasoning_output_tokens":0,"total_tokens":6}}}
{"timestamp":"2026-11-16T21:52:14.408Z","ordinal":31,"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":3,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":3,"reasoning_output_tokens":0,"total_tokens":6},"last_token_usage":{"input_tokens":1,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":1,"reasoning_output_tokens":0,"total_tokens":2},"last_usage_includes_reasoning":false,"model_context_window":258400},"rate_limits":{"limit_id":"codex","limit_name":null,"primary":null,"secondary":null,"credits":null,"individual_limit":null,"spend_control_reached":null,"plan_type":null,"rate_limit_reached_type":null}}}
{"timestamp":"2026-11-16T21:52:14.410Z","ordinal":32,"type":"event_msg","payload":{"type":"task_complete","root_turn_id":"01a1e655-b3c2-7e40-8a11-5c2d7e9f0a33","turn_id":"01a1e655-b3c2-7e40-8a11-5c2d7e9f0a33","last_agent_message":"All 42 tests pass.","started_at":1794865930,"completed_at":1794865934,"duration_ms":4309,"time_to_first_token_ms":23}}
"####;

#[test]
fn a_codex_conversation_two_levels_under_its_clone_names_the_new_folder_after_a_rename() {
    let tmp = tempfile::tempdir().unwrap();
    let home = fs::canonicalize(tmp.path()).unwrap();
    let (old, new) = (home.join("payments"), home.join("billing"));
    let (deep, now_deep) = (old.join("sessions/fix/api/src/pkg"), new.join("sessions/fix/api/src/pkg"));
    let rollout = home.join(format!(".codex/sessions/2026/11/16/rollout-2026-11-16T13-48-56-{ID}.jsonl"));
    fs::create_dir_all(rollout.parent().unwrap()).unwrap();
    let text = CODEX_THREE_TURNS
        .replace("{codex}", &home.join(".codex").to_string_lossy())
        .replace("{folder}", &deep.to_string_lossy())
        .replace("{id}", ID);
    fs::write(&rollout, &text).unwrap();
    // An earlier conversation of the session, recorded in another folder under the clone.
    let (other, now_other) = (old.join("sessions/fix/api/docs"), new.join("sessions/fix/api/docs"));
    let earlier = rollout.with_file_name(format!("rollout-2026-11-15T10-00-00-{EARLIER}.jsonl"));
    fs::write(&earlier, text.replace(ID, EARLIER).replace(&*deep.to_string_lossy(), &other.to_string_lossy())).unwrap();
    let threads = write_threads(&home, &[(ID, &rollout, &deep), (EARLIER, &earlier, &other)]);
    let store = Store::open(home.join("state")).unwrap();
    store
        .put_channel(&ChannelRecord {
            id: "ch_1".to_owned(),
            name: "Payments".to_owned(),
            folder: "payments".to_owned(),
            branch_prefix: None,
            default_agent: "codex".to_owned(),
            repos: vec![ChannelRepo { owner: "acme".to_owned(), name: "api".to_owned(), main: MainClone::Ready }],
            machines: vec!["ma_1".to_owned()],
            revision: 1,
            created_unix_ms: 1,
        })
        .unwrap();
    store
        .put_session(&SessionRecord {
            id: "se_1".to_owned(),
            channel: "ch_1".to_owned(),
            machine: "ma_1".to_owned(),
            title: "Fix".to_owned(),
            agent: "codex".to_owned(),
            folder: old.join("sessions/fix"),
            created_unix_ms: 1,
        })
        .unwrap();
    store
        .put_session_detail(&SessionDetail {
            session: "se_1".to_owned(),
            repos: vec![SessionRepo {
                owner: "acme".to_owned(),
                name: "api".to_owned(),
                branch: "fix".to_owned(),
                path: old.join("sessions/fix/api"),
            }],
            model: None,
            first_message: None,
            start: StartStep::Done,
            operation: None,
            revision: 1,
        })
        .unwrap();
    store
        .put_agent(&AgentRecord {
            session: "se_1".to_owned(),
            tab: "tab_1".to_owned(),
            launch: AgentLaunch {
                agent: "codex".to_owned(),
                folder: deep.clone(),
                conversation_id: ID.to_owned(),
                conversation_file: rollout.clone(),
                holder_socket: home.join("h"),
                cols: 80,
                rows: 24,
                ..AgentLaunch::default()
            },
            state: AgentState::Stopped,
            generation: 1,
            interruptions: Vec::new(),
            updated_unix_ms: 1,
        })
        .unwrap();
    store.push_previous_conversation("se_1", EARLIER, &earlier, 3).unwrap();
    let placement = Placement {
        name: "Billing".to_owned(),
        folder: "billing".to_owned(),
        old_path: old.clone(),
        new_path: new,
        now_unix_ms: 1,
    };
    let (agents, stage) = (Agents::builtin(), home.join("stage"));

    follow_conversations(&store, &agents, &home, &stage, &placement, "ch_1", "ma_1").unwrap();
    let followed = fs::read_to_string(&rollout).unwrap();
    follow_conversations(&store, &agents, &home, &stage, &placement, "ch_1", "ma_1").unwrap();

    assert!(followed.contains(&*now_deep.to_string_lossy()), "the rollout names the new folder");
    assert!(!followed.contains(&*old.to_string_lossy()), "the rollout still names the old folder");
    assert_eq!(fs::read_to_string(&rollout).unwrap(), followed, "again changes nothing");
    assert_eq!(thread_cwd(&threads, ID), now_deep.to_string_lossy(), "the CLI's thread row follows");
    let before = fs::read_to_string(&earlier).unwrap();
    assert!(before.contains(&*now_other.to_string_lossy()), "the earlier conversation names its own new folder");
    assert!(!before.contains(&*old.to_string_lossy()));
    assert_eq!(thread_cwd(&threads, EARLIER), now_other.to_string_lossy(), "its thread row names its own folder, not the agent's");
}

/// A conversation file that is there but not where the CLI keeps it is an
/// error that says so; one that is gone is nothing to follow.
#[test]
fn a_conversation_file_that_is_not_where_its_cli_keeps_it_is_refused_and_a_missing_one_is_skipped() {
    let tmp = tempfile::tempdir().unwrap();
    let home = fs::canonicalize(tmp.path()).unwrap();
    let kind = Agents::builtin().get("codex").unwrap().clone();
    let (from, to) = (home.join("payments/sessions/fix"), home.join("billing/sessions/fix"));
    let stray = home.join("stray.jsonl");
    fs::write(&stray, "{}\n").unwrap();

    let refused = kind.follow_folder(&home, ID, &stray, &from, &to, &home.join("stage"));
    let gone = kind.follow_folder(&home, ID, &home.join("gone.jsonl"), &from, &to, &home.join("stage"));

    assert!(matches!(refused, Err(FollowError::Misplaced { .. })), "{refused:?}");
    assert!(matches!(gone, Ok(false)), "{gone:?}");
}

/// D801 D: a session whose folder moves on its own (a Desk session into a
/// channel, a deleted channel's session to the Desk) gets steps that, run
/// at its agent's next resume, make its Codex conversation and thread row
/// name the new folder. A session with no agent has none.
#[test]
fn a_moved_session_s_codex_conversation_follows_it_by_the_steps_its_note_keeps() {
    let tmp = tempfile::tempdir().unwrap();
    let home = fs::canonicalize(tmp.path()).unwrap();
    let (old, new) = (home.join("desk/fix"), home.join("payments/sessions/fix"));
    let (deep, now_deep) = (old.join("api/src"), new.join("api/src"));
    let rollout = home.join(format!(".codex/sessions/2026/11/16/rollout-2026-11-16T13-48-56-{ID}.jsonl"));
    fs::create_dir_all(rollout.parent().unwrap()).unwrap();
    let text = CODEX_THREE_TURNS
        .replace("{codex}", &home.join(".codex").to_string_lossy())
        .replace("{folder}", &deep.to_string_lossy())
        .replace("{id}", ID);
    fs::write(&rollout, &text).unwrap();
    let threads = write_threads(&home, &[(ID, &rollout, &deep)]);
    let store = Store::open(home.join("state")).unwrap();
    let agent = |kind: &str, session: &str| AgentRecord {
        session: session.to_owned(),
        tab: format!("tab_{session}"),
        launch: AgentLaunch {
            agent: kind.to_owned(),
            folder: deep.clone(),
            conversation_id: ID.to_owned(),
            conversation_file: rollout.clone(),
            holder_socket: home.join(session),
            cols: 80,
            rows: 24,
            ..AgentLaunch::default()
        },
        state: AgentState::Running { holder_pid: 1 },
        generation: 1,
        interruptions: Vec::new(),
        updated_unix_ms: 1,
    };
    store
        .put_session(&SessionRecord {
            id: "se_1".to_owned(),
            channel: "desk".to_owned(),
            machine: "ma_1".to_owned(),
            title: "Fix".to_owned(),
            agent: "codex".to_owned(),
            folder: old.clone(),
            created_unix_ms: 1,
        })
        .unwrap();
    store.put_agent(&agent("codex", "se_1")).unwrap();
    let agents = Agents::builtin();
    let channels = home.join("channels");

    let steps = session_follows(&store, &agents, &home, &channels, "se_1", &old, &new).unwrap();
    assert!(!steps.is_empty(), "a Codex conversation has steps to follow");
    let kind = agents.get("codex").unwrap();
    for step in &steps {
        kind.follow_folder(&step.home, &step.id, &step.file, &step.from, &step.to, &step.scratch).unwrap();
    }
    let followed = fs::read_to_string(&rollout).unwrap();
    assert!(followed.contains(&*now_deep.to_string_lossy()), "the rollout names the new folder");
    assert!(!followed.contains(&*old.to_string_lossy()), "the rollout still names the old folder");
    assert_eq!(thread_cwd(&threads, ID), now_deep.to_string_lossy(), "the CLI's thread row follows");

    assert!(session_follows(&store, &agents, &home, &channels, "se_9", &old, &new).unwrap().is_empty(), "no agent, no steps");
}

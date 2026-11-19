//! A Desk session moved into channel Payments on the machine it is on
//! (DK-15..DK-17, MV-16, D305, D315), by the carry: packed for its landing,
//! copied as the Mover copies, unpacked, and its old place released. It
//! lands in `<root>/payments/sessions/notes`, its conversation at that
//! folder's key with every turn, and once what the old place keeps is
//! released nothing is left at the old folder or the old key, and no row
//! names them. A Codex rollout and the CLI's thread row name the new
//! folder; a clone's `.git` and its worktree name the new place; a picked
//! folder the user ticked moves too; a Copilot session's state, kept by
//! id, is rewritten where it is to name the new folder; a landing that is
//! no session folder is refused before anything is staged.

use std::sync::Arc;
use std::time::Duration;

use humata2_conversation::{Format, adapter};
use humata2_revive::{ReviveConfig, RestartReports};
use humata2_store::{
    AgentLaunch, AgentRecord, ChannelRecord, ChannelRepo, DeskFolderKind, DeskRow, MainClone, SessionRecord, Store, Take,
};

use super::*;
use crate::CarryStarter;
use humata2_sessions_core::agents::Agents;
use humata2_sessions_core::layout::Layout;
use humata2_sessions_core::names::DESK_CHANNEL;

const ID: &str = "6a1d0c2e-3b4f-4d5e-8f7a-9b0c1d2e3f40";
const CODEX_ID: &str = "01a1e655-aca1-7d40-9236-87fee361b5eb";
const CODEX_DAY: &str = "sessions/2026/11/16";
const CODEX_STAMP: &str = "2026-11-16T13-48-56";

/// Three turns as Claude Code writes them; `{folder}`, `{home}` and `{id}`
/// are filled per conversation.
const THREE_TURNS: &str = concat!(
    r#"{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"{folder}","sessionId":"{id}","version":"2.1.325","type":"user","message":{"role":"user","content":"Sketch the Q4 plan in {folder}/notes.md"},"uuid":"5b1e0c20-0001-4a8e-9b1d-3f2a6c8d0001","timestamp":"2026-11-16T09:01:00.000Z"}"#,
    "\n",
    r#"{"parentUuid":"5b1e0c20-0001-4a8e-9b1d-3f2a6c8d0001","isSidechain":false,"userType":"external","cwd":"{folder}","sessionId":"{id}","version":"2.1.325","message":{"id":"msg_01A","type":"message","role":"assistant","model":"claude-sonnet-4-5","content":[{"type":"text","text":"Wrote {folder}/notes.md."}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":812,"output_tokens":64}},"requestId":"req_01A","type":"assistant","uuid":"5b1e0c20-0011-4a8e-9b1d-3f2a6c8d0011","timestamp":"2026-11-16T09:02:00.000Z"}"#,
    "\n",
    r#"{"parentUuid":"5b1e0c20-0011-4a8e-9b1d-3f2a6c8d0011","isSidechain":false,"userType":"external","cwd":"{folder}","sessionId":"{id}","version":"2.1.325","type":"user","message":{"role":"user","content":"Add the payments milestones"},"uuid":"5b1e0c20-0002-4a8e-9b1d-3f2a6c8d0002","timestamp":"2026-11-16T09:05:00.000Z"}"#,
    "\n",
    r#"{"parentUuid":"5b1e0c20-0002-4a8e-9b1d-3f2a6c8d0002","isSidechain":false,"userType":"external","cwd":"{folder}","sessionId":"{id}","version":"2.1.325","message":{"id":"msg_01B","type":"message","role":"assistant","model":"claude-sonnet-4-5","content":[{"type":"text","text":"Added three milestones."}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":812,"output_tokens":64}},"requestId":"req_01B","type":"assistant","uuid":"5b1e0c20-0012-4a8e-9b1d-3f2a6c8d0012","timestamp":"2026-11-16T09:06:00.000Z"}"#,
    "\n",
    r#"{"parentUuid":"5b1e0c20-0012-4a8e-9b1d-3f2a6c8d0012","isSidechain":false,"userType":"external","cwd":"{folder}","sessionId":"{id}","version":"2.1.325","type":"user","message":{"role":"user","content":"Now it belongs with the payments work"},"uuid":"5b1e0c20-0003-4a8e-9b1d-3f2a6c8d0003","timestamp":"2026-11-16T09:09:00.000Z"}"#,
    "\n",
    r#"{"parentUuid":"5b1e0c20-0003-4a8e-9b1d-3f2a6c8d0003","isSidechain":false,"userType":"external","cwd":"{folder}","sessionId":"{id}","version":"2.1.325","message":{"id":"msg_01C","type":"message","role":"assistant","model":"claude-sonnet-4-5","content":[{"type":"text","text":"Agreed."}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":812,"output_tokens":64}},"requestId":"req_01C","type":"assistant","uuid":"5b1e0c20-0013-4a8e-9b1d-3f2a6c8d0013","timestamp":"2026-11-16T09:10:00.000Z"}"#,
    "\n",
);

/// A Codex rollout of three turns as codex-cli 0.169.0 writes it (the
/// conversation crate's `testdata/codex/three_turns.jsonl`); `{codex}`,
/// `{folder}` and `{id}` are filled.
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

/// A machine under `root` with its home at `root/home`, channel Payments
/// (`payments`), and Claude, Codex and Copilot as agents that keep their
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
         "conversation_file":"{home}/.codex/sessions/{conversation}.jsonl",
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

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// The scratch folder of Desk session Notes on `starter`.
fn scratch(starter: &Starter) -> PathBuf {
    starter.layout.root().join("desk").join("notes")
}

/// Desk session `se_1`, Notes, working in `folder` (a `kind` Desk folder)
/// with `agent` in `conversation`, its folder holding notes.md. Returns the
/// conversation file the agent keeps.
fn desk_session(starter: &Starter, agent: &str, folder: &Path, kind: DeskFolderKind, conversation: &str) -> PathBuf {
    write(&folder.join("notes.md"), "# Q4\n");
    starter
        .store
        .put_session(&SessionRecord {
            id: "se_1".to_owned(),
            channel: DESK_CHANNEL.to_owned(),
            machine: String::new(),
            title: "Notes".to_owned(),
            agent: agent.to_owned(),
            folder: folder.to_owned(),
            created_unix_ms: 1,
        })
        .unwrap();
    starter
        .store
        .put_desk(&DeskRow {
            session_id: "se_1".to_owned(),
            kind,
            boards: true,
            channels_and_machines: false,
            last_used_unix_ms: 10,
            closed_unix_ms: None,
            deleted_unix_ms: None,
            held_work: String::new(),
        })
        .unwrap();
    let plan = starter.agents.get(agent).unwrap().launch(&starter.home, folder, conversation);
    starter
        .store
        .put_agent(&AgentRecord {
            session: "se_1".to_owned(),
            tab: "tb_old".to_owned(),
            launch: AgentLaunch {
                agent: agent.to_owned(),
                folder: folder.to_owned(),
                argv: plan.argv,
                resume_argv: plan.resume_argv,
                conversation_id: conversation.to_owned(),
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
    plan.conversation_file
}

/// A Claude Desk session in `folder` with three turns. Returns its
/// transcript.
fn claude_desk_session(starter: &Starter, folder: &Path, kind: DeskFolderKind) -> PathBuf {
    let transcript = desk_session(starter, "claude", folder, kind, ID);
    let text = THREE_TURNS
        .replace("{folder}", &folder.to_string_lossy())
        .replace("{home}", &starter.home.to_string_lossy())
        .replace("{id}", ID);
    write(&transcript, &text);
    transcript
}

/// Where the Codex CLI of `starter` keeps rollout `CODEX_ID`.
fn rollout(starter: &Starter) -> PathBuf {
    starter
        .home
        .join(".codex")
        .join(CODEX_DAY)
        .join(format!("rollout-{CODEX_STAMP}-{CODEX_ID}.jsonl"))
}

/// The Codex CLI's thread index of `starter` (`state_5.sqlite`, table
/// `threads`, with the columns codex-cli 0.169.0 has) holding `CODEX_ID`
/// in `folder`.
fn write_threads(starter: &Starter, folder: &Path) -> PathBuf {
    let path = starter.home.join(".codex/state_5.sqlite");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
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
    connection
        .execute(
            "INSERT INTO threads (id, rollout_path, created_at, updated_at, source, model_provider, cwd, title,
                 sandbox_policy, approval_mode, git_sha, git_branch, git_origin_url, cli_version, first_user_message)
             VALUES (?1, ?2, 1763329736, 1763329930, 'exec', 'fixture', ?3, 'Sketch the Q4 plan',
                 'workspace-write', 'never', '1111111111111111111111111111111111111111', 'me/notes', NULL, '0.169.0',
                 'Sketch the Q4 plan')",
            rusqlite::params![CODEX_ID, rollout(starter).to_string_lossy(), folder.to_string_lossy()],
        )
        .unwrap();
    path
}

fn thread_cwd(db: &Path) -> String {
    let connection = rusqlite::Connection::open(db).unwrap();
    connection.query_row("SELECT cwd FROM threads WHERE id = ?1", [CODEX_ID], |row| row.get(0)).unwrap()
}

/// The thread row's `git_sha` and `git_branch`.
fn thread_git(db: &Path) -> (Option<String>, Option<String>) {
    let connection = rusqlite::Connection::open(db).unwrap();
    connection
        .query_row("SELECT git_sha, git_branch FROM threads WHERE id = ?1", [CODEX_ID], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap()
}

/// What the unpack does to the row when the landing's clone is on another
/// head: the commit and branch change.
fn move_thread_head(db: &Path) {
    let connection = rusqlite::Connection::open(db).unwrap();
    connection
        .execute(
            "UPDATE threads SET git_sha = '2222222222222222222222222222222222222222', git_branch = 'me/landed'",
            [],
        )
        .unwrap();
}

fn landing() -> Landing {
    Landing { channel: "ch_1".to_owned(), rel: "sessions/notes".to_owned() }
}

/// Moves `se_1` from `old` to its landing as the Mover does on one
/// machine: pack for the landing, copy the folder and the stage into the
/// channel, unpack. Returns where it landed.
fn move_in(starter: &Starter, old: &Path) -> PathBuf {
    let packed = starter.pack_landing("op_1", "se_1", &landing()).unwrap();
    let channel = starter.layout.channel("payments");
    let new = channel.join(&packed.folder);
    copy_tree(old, &new).unwrap();
    let ops = crate::CarryOps(starter);
    let incoming = ops.incoming(&ops.carry_channel("ch_1").unwrap(), "op_1");
    copy_tree(&channel.join(&packed.stage), &incoming).unwrap();
    starter.unpack("op_1", "se_1", "ch_1", Take::Everything).unwrap();
    new
}

/// Releases what the old place keeps once the move is done, as the Mover
/// does after the switch.
fn release(starter: &Starter) {
    for discard in starter.left_behind("op_1", "ch_1").unwrap() {
        let meta = fs::symlink_metadata(&discard.path).unwrap();
        if meta.is_dir() {
            fs::remove_dir_all(&discard.path).unwrap();
        } else {
            fs::remove_file(&discard.path).unwrap();
        }
    }
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

/// What the store holds of every session, agent, session start, tab and
/// earlier conversation.
fn rows(starter: &Starter) -> String {
    let store = &starter.store;
    let sessions = store.sessions().unwrap();
    let mut out = format!(
        "{sessions:?}\n{:?}\n{:?}\n{:?}\n",
        store.agents().unwrap(),
        store.session_details().unwrap(),
        store.tabs().unwrap()
    );
    for session in &sessions {
        out.push_str(&format!("{:?}\n", store.previous_conversations(&session.id).unwrap()));
    }
    out
}

/// Checks that no row names `old` (the folder) or `key` (its conversation
/// key) and that nothing is left at either.
fn nothing_at(starter: &Starter, old: &Path, key: &Path) {
    let dump = rows(starter);
    for needle in [old.to_string_lossy(), key.to_string_lossy()] {
        assert!(!dump.contains(needle.as_ref()), "a row still names {needle}: {dump}");
    }
    assert!(!old.exists(), "the old folder is still there: {}", old.display());
    assert_eq!(snapshot(key), BTreeMap::new(), "the old key still holds files");
}

#[test]
fn a_scratch_desk_session_moved_into_a_channel_lands_in_its_session_folder_with_every_turn() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = fs::canonicalize(tmp.path()).unwrap();
    let m = machine(&dir);
    let old = scratch(&m);
    let transcript = claude_desk_session(&m, &old, DeskFolderKind::Scratch { kept: false });
    let old_key = transcript.parent().unwrap().to_path_buf();

    let new = move_in(&m, &old);
    release(&m);

    assert_eq!(new, m.layout.session("payments", "notes"), "it lands in the channel's session place");
    assert_eq!(fs::read_to_string(new.join("notes.md")).unwrap(), "# Q4\n");
    assert_eq!(m.store.session("se_1").unwrap().unwrap().folder, new);
    let agent = m.store.agent("se_1").unwrap().unwrap();
    assert_eq!(agent.launch.folder, new);
    assert_eq!(agent.launch.conversation_id, ID, "the same conversation");
    let kind = m.agents.get("claude").unwrap();
    let at = kind.placement(&m.home, &new).unwrap();
    let found = adapter(Format::ClaudeJsonl).locate(&at, ID).expect("the conversation is at the new folder's key");
    assert_eq!(adapter(Format::ClaudeJsonl).turns(&found).unwrap().len(), 3, "every turn came along");
    assert!(found.files.iter().all(|file| !file.path.starts_with(&old_key)), "{:?}", found.files);
    assert_eq!(agent.launch.conversation_file, kind.launch(&m.home, &new, ID).conversation_file);
    nothing_at(&m, &old, &old_key);
    assert_eq!(files_naming(&m.home, &old.to_string_lossy()), Vec::<PathBuf>::new());
    assert_eq!(files_naming(&new, &old.to_string_lossy()), Vec::<PathBuf>::new());
}

#[test]
fn a_codex_desk_session_moved_into_a_channel_has_its_rollout_and_thread_row_in_the_new_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = fs::canonicalize(tmp.path()).unwrap();
    let m = machine(&dir);
    let old = scratch(&m);
    desk_session(&m, "codex", &old, DeskFolderKind::Scratch { kept: false }, CODEX_ID);
    let text = CODEX_THREE_TURNS
        .replace("{codex}", &m.home.join(".codex").to_string_lossy())
        .replace("{folder}", &old.to_string_lossy())
        .replace("{id}", CODEX_ID);
    write(&rollout(&m), &text);
    let threads = write_threads(&m, &old);

    let new = move_in(&m, &old);
    release(&m);

    assert_eq!(new, m.layout.session("payments", "notes"));
    let at = m.agents.get("codex").unwrap().placement(&m.home, &new).unwrap();
    let found = adapter(Format::CodexRollout).locate(&at, CODEX_ID).expect("the rollout is found for the new folder");
    assert_eq!(adapter(Format::CodexRollout).turns(&found).unwrap().len(), 3, "every turn came along");
    let kept = fs::read_to_string(&found.files[0].path).unwrap();
    assert!(kept.contains(&*new.to_string_lossy()), "the rollout's cwd is the new folder");
    assert!(!kept.contains(&*old.to_string_lossy()), "the rollout still names the old folder");
    assert_eq!(thread_cwd(&threads), new.to_string_lossy(), "the CLI's thread row names the new folder");
    assert!(!old.exists(), "the old folder is still there");
    let dump = rows(&m);
    assert!(!dump.contains(&*old.to_string_lossy()), "a row still names the old folder: {dump}");
}

#[test]
fn a_moved_desk_folder_s_clone_and_its_worktree_name_the_new_place() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = fs::canonicalize(tmp.path()).unwrap();
    let m = machine(&dir);
    let old = scratch(&m);
    let o = old.to_string_lossy().into_owned();
    write(&old.join("api/.git/HEAD"), "ref: refs/heads/main\n");
    write(&old.join("api/.git/config"), &format!("[core]\n\trepositoryformatversion = 0\n\tworktree = {o}/api\n"));
    write(&old.join("api/.git/worktrees/wt/gitdir"), &format!("{o}/wt/.git\n"));
    write(&old.join("api/.git/worktrees/wt/HEAD"), "ref: refs/heads/spike\n");
    write(&old.join("wt/.git"), &format!("gitdir: {o}/api/.git/worktrees/wt\n"));
    let transcript = claude_desk_session(&m, &old, DeskFolderKind::Scratch { kept: false });
    let old_key = transcript.parent().unwrap().to_path_buf();

    let new = move_in(&m, &old);
    release(&m);

    let n = new.to_string_lossy().into_owned();
    assert!(fs::read_to_string(new.join("api/.git/config")).unwrap().contains(&format!("{n}/api")));
    assert_eq!(fs::read_to_string(new.join("api/.git/worktrees/wt/gitdir")).unwrap(), format!("{n}/wt/.git\n"));
    assert_eq!(fs::read_to_string(new.join("wt/.git")).unwrap(), format!("gitdir: {n}/api/.git/worktrees/wt\n"));
    assert_eq!(files_naming(&new, &o), Vec::<PathBuf>::new(), "a file of the clone still names the old place");
    nothing_at(&m, &old, &old_key);
}

#[test]
fn a_picked_folder_the_user_ticked_moves_into_the_channel_and_leaves_nothing_where_it_was() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = fs::canonicalize(tmp.path()).unwrap();
    let m = machine(&dir);
    let picked = dir.join("picked").join("notes");
    let transcript = claude_desk_session(&m, &picked, DeskFolderKind::Picked);
    let old_key = transcript.parent().unwrap().to_path_buf();

    let new = move_in(&m, &picked);
    release(&m);

    assert_eq!(new, m.layout.session("payments", "notes"));
    assert_eq!(fs::read_to_string(new.join("notes.md")).unwrap(), "# Q4\n");
    assert_eq!(m.store.session("se_1").unwrap().unwrap().folder, new);
    let at = m.agents.get("claude").unwrap().placement(&m.home, &new).unwrap();
    let found = adapter(Format::ClaudeJsonl).locate(&at, ID).expect("the conversation is at the new folder's key");
    assert_eq!(adapter(Format::ClaudeJsonl).turns(&found).unwrap().len(), 3);
    nothing_at(&m, &picked, &old_key);
    assert!(dir.join("picked").is_dir(), "only the picked folder went, not the folder it was in");
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
fn a_copilot_desk_session_moved_into_a_channel_has_its_session_state_name_the_new_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = fs::canonicalize(tmp.path()).unwrap();
    let m = machine(&dir);
    let old = scratch(&m);
    let events = desk_session(&m, "copilot", &old, DeskFolderKind::Scratch { kept: false }, ID);
    write(&events, &COPILOT_TWO_TURNS.replace("{folder}", &old.to_string_lossy()).replace("{id}", ID));

    let new = move_in(&m, &old);
    release(&m);

    assert_eq!(new, m.layout.session("payments", "notes"), "it lands in the channel's session place");
    assert_eq!(fs::read_to_string(new.join("notes.md")).unwrap(), "# Q4\n");
    let agent = m.store.agent("se_1").unwrap().unwrap();
    assert_eq!(agent.launch.folder, new);
    assert_eq!(agent.launch.conversation_file, events, "Copilot keeps it by id, not by folder");
    let at = m.agents.get("copilot").unwrap().placement(&m.home, &new).unwrap();
    let copilot = adapter(Format::CopilotSessionState);
    let found = copilot.locate(&at, ID).expect("the session state is where it was");
    assert_eq!(copilot.turns(&found).unwrap().len(), 2, "every turn is kept");
    assert!(!old.exists(), "the old folder is released");
    assert_eq!(files_naming(&m.home, &old.to_string_lossy()), Vec::<PathBuf>::new());
    assert!(!files_naming(&m.home, &new.to_string_lossy()).is_empty(), "it names the new folder");
}

#[test]
fn a_copilot_conversation_with_a_record_humata_does_not_know_is_refused_by_the_landing_check_and_a_known_one_is_not() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = fs::canonicalize(tmp.path()).unwrap();
    let m = machine(&dir);
    let old = scratch(&m);
    let events = desk_session(&m, "copilot", &old, DeskFolderKind::Scratch { kept: false }, ID);
    let two_turns = COPILOT_TWO_TURNS.replace("{folder}", &old.to_string_lossy()).replace("{id}", ID);
    // Copilot 1.0.113-1 writes this in every turn; Humata knows it.
    let delivery = r#"{"type":"instruction.update_delivery","data":{"deliveryId":"0261e66c-6243-4d7e-a7db-9efc607a92bd","updateCount":1,"outcome":"fullRebuild"},"id":"77ff210c-d33c-4ca7-a411-7084eb3b4f5e","timestamp":"2026-11-17T22:25:15.217Z","parentId":"c8a7c560-c68d-44b7-8be1-9ff591e32c8e"}"#;
    write(&events, &format!("{two_turns}{delivery}\n"));
    m.check_landing("se_1").unwrap();

    // A record kind no capture has shown yet: the move is refused before it starts.
    let unknown = r#"{"type":"system.unseen","data":{"content":"x"},"id":"e8d1b5c8-2c0a-4160-a00f-047af95fdf76","timestamp":"2026-11-17T22:30:59.353Z","parentId":"c8a7c560-c68d-44b7-8be1-9ff591e32c8e"}"#;
    write(&events, &format!("{two_turns}{unknown}\n"));
    let folder_before = snapshot(&old);

    let refused = m.check_landing("se_1").unwrap_err();

    assert!(
        matches!(refused, CarryError::Conversation { source: ConversationError::Unsupported { .. }, .. }),
        "{refused:?}"
    );
    assert_eq!(snapshot(&old), folder_before);
}

#[test]
fn a_landing_that_is_not_a_session_folder_of_the_channel_is_refused_before_anything_is_staged() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = fs::canonicalize(tmp.path()).unwrap();
    let m = machine(&dir);
    let old = scratch(&m);
    claude_desk_session(&m, &old, DeskFolderKind::Scratch { kept: false });
    let folder_before = snapshot(&old);

    for rel in ["sessions/../desk", "main/api", "sessions/a/b", "desk/notes"] {
        let refused = m
            .pack_landing("op_1", "se_1", &Landing { channel: "ch_1".to_owned(), rel: rel.to_owned() })
            .unwrap_err();

        assert!(matches!(refused, CarryError::BadFolder { .. }), "{rel}: {refused:?}");
    }
    assert_eq!(snapshot(&m.layout.channel("payments")), BTreeMap::new(), "nothing was staged");
    assert_eq!(snapshot(&old), folder_before);
}

#[test]
fn a_copied_folder_keeps_its_links_as_links_and_one_into_the_folder_points_into_the_copy() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = fs::canonicalize(tmp.path()).unwrap();
    let (from, to) = (dir.join("from"), dir.join("to"));
    write(&from.join("docs/plan.md"), "# Q4\n");
    write(&dir.join("elsewhere.md"), "not in the folder\n");
    std::os::unix::fs::symlink("docs/plan.md", from.join("relative")).unwrap();
    std::os::unix::fs::symlink(from.join("docs"), from.join("inside")).unwrap();
    std::os::unix::fs::symlink(dir.join("elsewhere.md"), from.join("outside")).unwrap();
    std::os::unix::fs::symlink("missing.md", from.join("dangling")).unwrap();

    copy_tree(&from, &to).unwrap();

    let link = |name: &str| fs::read_link(to.join(name)).expect("a link, neither followed nor left out");
    assert_eq!(link("relative"), PathBuf::from("docs/plan.md"), "a relative link stays relative");
    assert_eq!(link("inside"), to.join("docs"), "an absolute link into the folder points into the copy");
    assert_eq!(link("outside"), dir.join("elsewhere.md"), "a link out of the folder points where it did");
    assert_eq!(link("dangling"), PathBuf::from("missing.md"));
    assert_eq!(fs::read_to_string(to.join("relative")).unwrap(), "# Q4\n");
    assert!(fs::symlink_metadata(to.join("inside")).unwrap().is_symlink(), "the folder it names was not copied in its place");
}

#[test]
fn a_landed_folder_s_links_are_taken_up_as_the_copy_made_them() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = fs::canonicalize(tmp.path()).unwrap();
    let m = machine(&dir);
    let old = scratch(&m);
    claude_desk_session(&m, &old, DeskFolderKind::Scratch { kept: false });
    std::os::unix::fs::symlink("notes.md", old.join("relative")).unwrap();
    std::os::unix::fs::symlink(old.join("notes.md"), old.join("inside")).unwrap();

    let packed = m.pack_landing("op_1", "se_1", &landing()).unwrap();
    let new = m.land_copy("op_1", "se_1", "ch_1", &packed).unwrap();
    m.unpack_landed("op_1", "se_1", "ch_1").expect("the landed links pass the check");

    assert_eq!(fs::read_link(new.join("relative")).unwrap(), PathBuf::from("notes.md"));
    assert_eq!(fs::read_link(new.join("inside")).unwrap(), new.join("notes.md"));
    assert_eq!(fs::read_to_string(new.join("inside")).unwrap(), "# Q4\n");
}

#[test]
fn what_the_session_wrote_after_the_copy_lands_once_the_landing_is_synced_again() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = fs::canonicalize(tmp.path()).unwrap();
    let m = machine(&dir);
    let old = scratch(&m);
    claude_desk_session(&m, &old, DeskFolderKind::Scratch { kept: false });
    write(&old.join("gone.md"), "removed after the copy\n");
    let packed = m.pack_landing("op_1", "se_1", &landing()).unwrap();
    let new = m.land_copy("op_1", "se_1", "ch_1", &packed).unwrap();

    write(&old.join("drafts/late.md"), "written after the copy\n");
    fs::remove_file(old.join("gone.md")).unwrap();
    // The same size and time, other bytes: only its bytes tell.
    let notes = old.join("notes.md");
    let time = fs::metadata(&notes).unwrap().modified().unwrap();
    fs::write(&notes, "# Q5\n").unwrap();
    fs::File::open(&notes).unwrap().set_modified(time).unwrap();
    m.resync_landing("op_1", "se_1", &landing()).unwrap();
    m.unpack_landed("op_1", "se_1", "ch_1").expect("the landing matches its manifest");

    assert_eq!(fs::read_to_string(new.join("drafts/late.md")).unwrap(), "written after the copy\n");
    assert_eq!(fs::read_to_string(new.join("notes.md")).unwrap(), "# Q5\n");
    assert!(!new.join("gone.md").exists(), "a file gone from the session's folder is gone from the landing");
}

#[test]
fn a_codex_thread_row_that_was_missing_stays_missing_when_the_landing_is_taken_back() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = fs::canonicalize(tmp.path()).unwrap();
    let m = machine(&dir);
    let old = scratch(&m);
    desk_session(&m, "codex", &old, DeskFolderKind::Scratch { kept: false }, CODEX_ID);
    let text = CODEX_THREE_TURNS
        .replace("{codex}", &m.home.join(".codex").to_string_lossy())
        .replace("{folder}", &old.to_string_lossy())
        .replace("{id}", CODEX_ID);
    write(&rollout(&m), &text);
    let threads = write_threads(&m, &old);
    rusqlite::Connection::open(&threads).unwrap().execute("DELETE FROM threads", []).unwrap();
    let packed = m.pack_landing("op_1", "se_1", &landing()).unwrap();
    m.land_copy("op_1", "se_1", "ch_1", &packed).unwrap();
    m.unpack_landed("op_1", "se_1", "ch_1").unwrap();

    m.take_landing_back("op_1", "se_1", "ch_1", true).unwrap();

    assert_eq!(fs::read_to_string(rollout(&m)).unwrap(), text, "the rollout is as it was");
    let rows: i64 = rusqlite::Connection::open(&threads)
        .unwrap()
        .query_row("SELECT count(*) FROM threads", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0, "no row was made for it");
}

#[test]
fn a_codex_landing_taken_back_after_the_unpack_leaves_its_rollout_and_thread_row_as_they_were() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = fs::canonicalize(tmp.path()).unwrap();
    let m = machine(&dir);
    let old = scratch(&m);
    desk_session(&m, "codex", &old, DeskFolderKind::Scratch { kept: false }, CODEX_ID);
    let text = CODEX_THREE_TURNS
        .replace("{codex}", &m.home.join(".codex").to_string_lossy())
        .replace("{folder}", &old.to_string_lossy())
        .replace("{id}", CODEX_ID);
    write(&rollout(&m), &text);
    let threads = write_threads(&m, &old);
    let packed = m.pack_landing("op_1", "se_1", &landing()).unwrap();
    let new = m.land_copy("op_1", "se_1", "ch_1", &packed).unwrap();
    m.unpack_landed("op_1", "se_1", "ch_1").unwrap();
    move_thread_head(&threads);

    m.take_landing_back("op_1", "se_1", "ch_1", true).unwrap();

    assert_eq!(fs::read_to_string(rollout(&m)).unwrap(), text, "the rollout is as it was, byte for byte");
    assert_eq!(thread_cwd(&threads), old.to_string_lossy(), "the CLI's thread row names the old folder");
    assert_eq!(
        thread_git(&threads),
        (Some("1111111111111111111111111111111111111111".to_owned()), Some("me/notes".to_owned())),
        "the row has the commit and branch it had before the unpack"
    );
    assert!(!new.exists(), "the landing is gone");
    let agent = m.store.agent("se_1").unwrap().unwrap();
    assert_eq!(agent.launch.folder, old, "the agent resumes where it was");
}

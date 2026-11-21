//! `lore_mcp` — Backend abstraction (`BR-1`), stdio JSON-RPC 2.0 MCP server
//! exposing the 5 `lore` tools (`SPEC.md §5.1`), and fail-open prompt/cue hook
//! handlers (`HOOK-1`..`HOOK-3`).

pub mod backend;
pub mod hooks;
pub mod server;

#[cfg(test)]
mod tests;

pub use backend::{LocalBackend, LoreBackend};
pub use hooks::{
    CUE_HOOK_TOKEN_BUDGET, PROMPT_HOOK_TOKEN_BUDGET, handle_cue_hook, handle_prompt_hook,
};
pub use server::{
    DEFAULT_DISABLED_REASON, DISABLED_INSTRUCTION, McpServer, disabled_tool_payload,
    format_repo_scope, parse_claim_kind, parse_feedback_action, resolve_write_scope,
};

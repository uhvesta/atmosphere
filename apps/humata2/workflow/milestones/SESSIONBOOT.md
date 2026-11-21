# SESSIONBOOT (ATM-54): Deterministic TUI startup authorization and prompt handling

- **Status:** implemented & verified (2026-11-21).
- **Outcome sentence:** After starting a TUI agent (`agy`, `claude`, `codex`, `copilot`), startup waits with bounded, deterministically jittered retries, parses first-screen authorization/trust prompts, deterministically presses only safe in-scope keys when owner-preapproved, fails safe by pausing for smart-agent/owner review whenever out of scope, destructive, external, or ambiguous, and records a structured audit trail.

## Implementation Summary

### 1. Screen prompt parser (`apps/humata2/daemon/runtime/src/screen_question.rs`)
- Added `PromptKind::Authorization` (`"authorization"`) and `AUTHORIZATION_PHRASES` (`"do you authorize"`, `"authorize me"`, `"authorize antigravity"`, `"authorize agy"`, `"authorize this workspace"`, `"authorize this session"`) evaluated after `Permission` so all 318 recorded corpus snapshots in `screen_replay_test` remain unchanged.
- Added unit tests covering Antigravity (`agy`) `"Do you authorize me"` prompts.

### 2. Startup authorization engine (`apps/humata2/daemon/svc_inbox/src/startup_auth.rs`)
- **Explicit errors:** `StartupAuthError` (`thiserror::Error` with `ScreenRead`, `KeySend`, `InvalidConfig`).
- **Bounded startup & deterministic jitter:** `StartupConfig` + `StartupSchedule` (validated bounds, deterministic `splitmix64` ±jittered exponential backoff, `seed_for_session` via `Sha256`, hard `max_attempts` and `max_total_wait` ceilings).
- **Owner consent & granted scope:** `OwnerConsentScope::strict()` (fail-safe default) and `OwnerConsentScope::local_workspace(path)` (single-session trust/auth/onboarding scoped strictly to `granted_workspace`; `prefer_remember_folder = false` by default).
- **Prompt parsing & classification:** `parse_startup_prompt` + `is_ready_screen` supporting Antigravity (`agy`) `[y/N]`, `(yes/no)`, numbered, and `(y)/(n)` hotkey forms; Claude Code unnumbered cursor lists (`❯ No, exit` / `Yes, I trust this folder`) and onboarding pickers; Copilot numbered folder trust; Codex folder trust, MCP, command permission, and login screens.
- **Fail-safe policy & smart-agent approval boundary:** `decide_startup_action` never infers destructive or external authorization (any destructive/external risk signal, workspace mismatch, tool/MCP permission, login prompt, or ambiguous/uncovered prompt returns `StartupDecision::PauseForReview(ApprovalReviewSummary)` showing `action`, `target`, `scope`, `duration`, `reversibility`, `changed_risk_factors`, `reason`, and `requires_smart_agent_review: true`).
- **Deterministic key mapping:** `KeyInput` (`Enter`, `Escape`, `Up`, `Down`, `Char`, `Text`) with `to_bytes()` and exact cursor/numbered/inline/hotkey resolution.
- **Structured audit trail & bounded async runner:** `StartupAuditTrail` / `AuditEntry` recording `attempt`, `elapsed_ms`, `scheduled_delay_ms`, SHA-256 `screen_hash`, `screen_excerpt`, `classification`, `key_mapping`, `keys_pressed`, `outcome`, and `rationale`. `StartupAuthRunner` drives `StartupTerminalPort` with paused-clock-friendly `tokio::time::sleep`, stuck-screen protection (`PauseReason::StuckAfterKeyPress`), and timeout fail-safe (`PauseReason::TimeoutExceeded`).

### 3. ScreenAsk integration hook (`apps/humata2/daemon/svc_inbox/src/screen_ask.rs` & `src/lib.rs`)
- Exposed `pub mod startup_auth;` in `apps/humata2/daemon/svc_inbox/src/lib.rs`.
- Added `ScreenAsk::inspect_startup_screen(screen, scope)` delegating to `startup_auth::{parse_startup_prompt, decide_startup_action}` without altering existing `ScreenAsk` behavior.

### 4. Deterministic fast-tier tests (`apps/humata2/daemon/svc_inbox/tests/startup_auth.rs`)
- Zero real sleeps, zero wall-clock asserts (`#[tokio::test(start_paused = true)]`).
- Covers `agy` authorization variants + strict scope, Claude/Copilot/Codex folder trust key selection, workspace scope mismatch, destructive/external/MCP/login pause summaries, ambiguous prompt fail-safe, deterministic jittered schedule & bounds, stuck-screen protection, timeout handling, structured audit trail, and `ScreenAsk::inspect_startup_screen`.

## Verification Targets (Full Lint)
- `//apps/humata2/daemon/runtime:runtime_unit_test`
- `//apps/humata2/daemon/svc_inbox:svc_inbox_unit_test`
- `//apps/humata2/daemon/svc_inbox:screen_replay_test`
- `//apps/humata2/daemon/svc_inbox:startup_auth_test`

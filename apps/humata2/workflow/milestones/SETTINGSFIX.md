# SETTINGSFIX: Every setting editable in the app, agent settings edits via Inbox, and working `humata login`

- **Status:** queued — **0.10.1** (slots with AGENTOFF and PROVIDERCLI; D917)
- **Depends on:** FUSEFSWIRE (0.10.0). Coordinates `ui/settings` hunk ordering with AGENTOFF B15.
- **Outcome sentence:** Every global and per-channel setting (including default branch prefix) is editable in the app with zero split-brain, agents asking to change settings appear in my Inbox for 1-click approval instead of telling me to run a terminal command, and `humata login` prompts in the app and works.

## Owner's words (2026-11-20)
1. "why can't i set the default branch prefix in the app? I want to be able to do that please so let me do it."
2. "There's a lot of settings that the agent cant edit, it says to edit via terminal. I run it via terminal and it wont let me and login wont work. The app wont work."
3. "If an agent is doing something it needs authorization to do just make it appear in my inbox but also make sure there are no UX deadends."

## Root causes found (2026-11-20 audit)
1. **Default branch prefix & Channel settings missing from app:** `api/proto/humata/v1/settings.proto` (`branch_prefix`, `branch_prefix_is_default`, `ChannelSettings`, `RefreshGhLogin`), `settings_service.rs`, and CLI already implement them, but `clients/`, `features/`, and `ui/` have zero references to them. `change_default_agent` (`clients/live_daemon/src/settings.rs:40`) also wipes `default_agent.model` by sending `String::new()`.
2. **4-stage CLI / login deadlock:**
   - `Capability::SettingsChange` is `Tier::UserOnly` (`daemon/access/src/capability.rs:256-266`) and `mcp/src/user_only.rs:22-25` blocks `settings_update`/`channel_settings_update`/`agents_add`/`agents_update`, telling agents to tell the user to run a terminal command.
   - Running `humata` inside a session terminal sends `HUMATA_SESSION_TOKEN` -> `Caller::Agent` -> refused again.
   - External terminal `humata settings get` isn't in `LOCAL_READ_ONLY` (`identity.rs:243-251`), so it requires `humata login`.
   - `CliAuthService.BeginCliSession` waits 5 min for the desktop app to call `ListPendingCliLogins` / `AnswerCliLogin`, which has **0 callers** anywhere in `clients/`, `features/`, `ui/`, or `desktop/`.

## Scope

### Slice 1 — `humata login` App Prompt & `LOCAL_READ_ONLY` Reads
- `daemon/humatad/src/identity.rs`: add `(SettingsService, "GetSettings")` and `(SettingsService, "GetChannelSettings")` to `LOCAL_READ_ONLY`.
- `daemon/humatad/src/cli_login.rs` & events: publish event when a pending CLI login arrives or settles.
- `clients/src/daemon.rs` + `clients/live_daemon/`: wire `pending_cli_logins()`, `answer_cli_login(id, allow)`, `cli_sessions()`, `end_cli_session(id)`, `script_tokens()`, `revoke_script_token(id)`.
- `features/inbox` / `features/app` + `ui/`: surface pending CLI login approval banner/card in Inbox (and visible toast/modal when the app is open) with terminal name, duration, Approve / Deny.
- `cli/src/error.rs`: clear non-circular message when `HUMATA_SESSION_TOKEN` is set in a session terminal.

### Slice 2 — Full `SettingsService` + `ChannelSettings` Client Wiring & UI
- `clients/src/daemon.rs` & `clients/live_daemon/src/settings.rs`: full `Settings` + `ChannelSettings` read/update + `RefreshGhLogin`, preserving `default_agent.model` and `expected_revision`.
- `features/settings/src/` & `ui/settings/src/`:
  - **General:** Default branch prefix input (with live preview, "Reset to @user default", "Refresh GitHub login"), Security › Contain agents toggle, Retention defaults (`archive_pr_reviews_after_days`, `archive_idle_sessions_after_days`).
  - **Agents:** Default model field alongside Default agent; custom agent add/edit.
  - **Layout:** Sync `terminal.font`, `terminal.font_size`, `terminal.theme`, `terminal.scrollback_lines`, `long_lines` with `SettingsService` (end split-brain with `humata2-ui.json`).
  - **Notifications:** Quiet hours (`quiet_from`, `quiet_until`) and VIP breaks quiet hours.
  - **Sync:** `skills.auto_sync`, `skills.exclude_patterns`, `skills.opted_out`.
  - **Agent access (`ui/settings/src/access.rs`):** While I'm away (`SetAwayRule`), Approval passphrase (`SetApprovalPassphrase`), CLI sessions & script tokens revocation list.
- `ui/workspace/src/channel.rs`: Channel settings UI for per-channel `branch_prefix` (`Inherit` vs custom), `default_agent.{agent, model}`, `contain_agents`, `retention`, `todo_automation`.
- `cli/src/commands/settings.rs`: add `todo_automation.*` and `sync.*` / `fusefs_enabled` for CLI-UI parity.

### Slice 3 — Inbox-Gated Settings & Safe Workspace Mutations (`AskNoAllow`)
- `daemon/access/src/capability.rs`: change `Capability::SettingsChange` from `Tier::UserOnly` to `Tier::AskNoAllow` (`Ask` default or `Never`; **never** `Allow` / `"Always allow"`).
- `daemon/humatad/src/settings_service.rs`, `daemon/svc_catalog/src/agent_service.rs`, `daemon/humatad/src/repo_service.rs`, `daemon/humatad/src/skills_service.rs`, `daemon/svc_review/src/pull_requests.rs`: route agent mutations through `answer_gated` with an explicit `scope`, `field`, `old -> new` Inbox preview (plus a strong warning when turning `contain_agents` off).
- `daemon/mcp/src/{user_only.rs,tools.rs}` & `daemon/svc_mcp/`: remove `settings_update`, `channel_settings_update`, `agents_add`, `agents_update`, `repo_scope_update` from `user_only::NAMES` and expose them with `reason` / `wait_seconds`.
- Update `skills/humata-inbox/inbox.md` & `skills/shared/basics.md` so agents propose settings changes via Inbox instead of refusing.
- Keep security boundaries (`AccessService`, `CliAuthService`, `HostSecrets`, `InboxAnswer`, `ChannelDelete`, `TrashEmpty`, `TerminalInput`, `PrSubmit`, `UserReviewState`) strictly `UserOnly`, with working App UI + working `humata login`.

## Exit criteria
- Fast behavioural tests for: global & channel `branch_prefix` UI/reducer/daemon, preserving `default_agent.model`, agent `UpdateSettings` / MCP `settings_update` -> Inbox `AskNoAllow` Approve/Deny/Never (never `"Always allow"`), external `humata settings get` without login, and `humata login` approved in-app within <1 s.

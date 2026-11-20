# 21. Antigravity (`agy`) as a supported agent

> "I want to start Antigravity (`agy`) in a session, toggle it in Settings › Agents, and resume it across restarts."

**Milestone:** AGY (`apps/humata2/workflow/milestones/AGY.md`)

## Starting state
- `humatad2` running with built-in agent kinds (`claude`, `codex`, `copilot`, `opencode`, `agy`, `zsh`).
- Default `agents.toml` (missing): `agy` is enabled, `install_when_missing` is off (per-agent opt-in rule), `asks_in_inbox` is on, and `install_command` is `curl -fsSL https://antigravity.google/cli/install.sh | bash`.
- Isolated `HOME` in `/tmp` so real `~/.gemini/antigravity-cli` and `~/.gemini/config` are never modified.

## Steps

1. **Pick Antigravity in New session / New desk session / New agent tab**
   - **Do:** Open **New session** or **New desk session**.
   - **See:** **Antigravity** appears in the agent picker alongside Claude Code, Codex, Copilot, OpenCode, and Terminal.
   - **Expect:** Selecting **Antigravity** starts an `agy` session.
   - **Status:** Works in desktop native tests (`desktop/tests/native/overlays.rs::palette_new_session_and_toast_reference_images`, `desktop/tests/native/desk.rs::a_new_desk_session_starts_from_the_form_and_opens`, snapshots `31-new-session.view.txt` and `40-new-desk-session.view.txt`).

2. **Manage Antigravity in Settings › Agents**
   - **Do:** Open **Settings › Agents**. Toggle **Install when missing**, **Ask in the Inbox**, and the main **Antigravity** switch off, then leave and return to **Settings › Agents**.
   - **See:** Switches reflect each change; turning **Antigravity** off removes it from pickers and hides its **Ask in the Inbox** row.
   - **Expect:** `agents.toml` persists `[agents.agy]` (`enabled`, `install_when_missing`, `asks_in_inbox`); built-in install command is never run unless opted in.
   - **Status:** Works in daemon unit tests (`daemon/runtime/src/agent_switches.rs`) and desktop native tests (`desktop/tests/native/agent_switches.rs::antigravity_switches_control_offering_install_and_inbox_asks`).

3. **Launch, resume, and hooks**
   - **Do:** Start an `agy` session and resume it after a restart or move.
   - **See:** `agy` starts via the built-in `agy` kind and resumes its conversation; any permission/question asks delivered through hooks reach the Inbox (no screen scraping, per INBOXHOOKS).
   - **Expect:** Built-in `agy` kind definition, hook wiring, and resume arguments work without touching `~/.gemini/antigravity-cli` or `~/.gemini/config`.
   - **Status:** Works in daemon tests (`daemon/sessions_core:sessions_core_unit_test`, `daemon/agent_hooks/...`, `daemon/runtime:runtime_unit_test`).

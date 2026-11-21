# DESK Wave 2 — DESKSPAWN, DESKTABS, MULTIAGENT, DESKTODO

Milestone tracking for Channel Desk Wave 2 (`DESKSPAWN` ATM-40..45, `DESKTABS` ATM-64/68, `MULTIAGENT` ATM-65, `DESKTODO` ATM-66/67).

## Locked-In Product Decisions

1. **DESKSPAWN (`daemon/access/src/{channel_desk.rs,desk.rs}`, `daemon/sessions_desk`, `daemon/svc_desk`):**
   - `ChannelDeskSwitches::default()` flips to:
     - `read_files: true`
     - `start_and_fork: true` (for Desk's own channel on same machine: `SessionStartSameMachine`; never another channel; another machine `SessionStartOtherMachine` still follows the channel rule)
     - `send_notes: false` (explicit Inbox consent: `Yes, this once` / `Yes, for this Desk session` / `No`)
     - `type_into_terminal: false` (explicit Inbox consent)
     - `stop_and_archive: false`
     - `change_board_items: false`
   - **Hard safety rule:** Typing into another session that is showing a **permission prompt** is hard-refused automatically (`permission_prompt_protected`, exit 4 — never offered as an approval button).
   - No switch or grant ever answers Inbox requests (`IN-23`).
   - Drop rejected `"New desk session"` setup modal (`ATM-39-01`).

2. **DESKTABS + MULTIAGENT (`ATM-64/65/68`):**
   - Keep shipped Channel Desk screen + agent tab strip with `+` (including plain `Terminal` (`zsh`) capped at max 2 like every other kind).
   - **Flat cap enforced in backend + UI:** total cap = `enabled_kinds + 2`, max 2 per kind (no shifting-cap algebra).
   - Remove `"New desk session"` from Channel Desk UI (`+` on the tab strip owns it).
   - Do not duplicate Desk tabs under top-level `Desk` in sidebar.

3. **DESKTODO (`ATM-66/67`):**
   - Live To-do **List-only** split pane beside Channel Desk (cut 5-column Kanban).
   - Bounded width `320–460px` (default `360px`), remembered per channel along with visibility.
   - Stripped header: `To-do · N` + `🔍` `↗` `✕` (no `Columns | List`, no `• Live`, no duplicate `+`).
   - Grouped vertically: `In progress`, `In review`, `Next`, capped `Backlog`, folded `Done`.
   - To-do toggle button counts only agent/Desk board changes made while the pane was hidden.
   - Full desk session views remain unchanged.

## Verification Targets

- `//apps/humata2/daemon/access:access_unit_test`
- `//apps/humata2/daemon/sessions_desk:sessions_desk_unit_test`
- `//apps/humata2/daemon/svc_desk:svc_desk_unit_test`
- `//apps/humata2/features/desk:all`
- `//apps/humata2/ui/desk:desk_unit_test`

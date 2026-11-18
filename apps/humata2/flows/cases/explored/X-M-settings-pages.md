# Settings: every pane except Agents switches and Machines › Scripts, plus the palette's Settings search

Layer: explored · macOS · harness: `//apps/humata2/desktop:native_test`, `//apps/humata2/desktop:live_flows_test`, `//apps/humata2/desktop:root_change_test`, and the `//apps/humata2/features/settings:*_test` unit targets

Phase 1 (map from code and existing snapshots, no run). Every step below is read from `features/settings/src/lib.rs`, `ui/src/settings.rs`, `ui/src/settings_repositories.rs`, `ui/src/access.rs`, `ui/src/overlay_root.rs`, `features/hosts/src/lib.rs`, `features/app/src/{persist,saved,commands}.rs` and `features/palette/src/lib.rs`. Steps marked PENDING RUN need phase 2 to fill the screen and evidence.

## Setup

- Native screens: `native/29-settings.view.txt` (Layout selected, the 8 sidebar tabs, the font button, the two toggles), `native/29-permission-route.view.txt` (Agents section, Notifications switched in), `native/30-palette.view.txt`, `native/28-hosts.view.txt`. Design goldens (do not touch): `design/29-settings.png`, `design/28-hosts.png`, `design/30-palette.png`, `design/49-channel-agent-access.png`, `design/60-machine-storage.png`.
- Headless harness: `//apps/humata2/desktop:native_test` (filter `hosts_settings`), `//apps/humata2/desktop:live_flows_test` (filter `find`), `//apps/humata2/desktop:root_change_test`.
- Feature tests: `//apps/humata2/features/settings:preferences_test`, `hidden_terminals_test`, `option_as_alt_test`, `repositories_test`, `repo_owners_test`, `agent_asks_test`, `codex_trust_test`, `access_test`.
- Evidence folder (phase 2): `verify/evidence/X-M/` under the scratchpad.
- The Settings page's section is kept between visits; `Route::Settings` does not reset it.

## Steps

### 1. Open Settings and see the eight sections
- **Screen:** a left column of 8 tabs (General, Agents, Machines, Layout, Agent access, Storage, Notifications, Keyboard) and a content pane. Layout is selected by default (`#[default]` on `Section::Layout`). Snapshot `native/29-settings.view.txt`.
- **Offered:** the 8 section tabs; the sidebar's Settings link; ⌘, and the palette's "Open Settings".
- **Action:** open Settings.
- **Result:** the Settings route; the section that was last open (Layout on a fresh run).
- **Evidence:** `native/29-settings.png`, `native/29-settings.view.txt`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=hosts_settings::hosts_and_settings_reference_images`

### 2. Appearance: System, Light or Dark
- **Screen:** Layout › Appearance card: "Follows the system unless you pick one." with a three-tab control System / Light / Dark. The selected tab shows as selected.
- **Offered:** System, Light, Dark.
- **Action:** click Light.
- **Result:** `preferences.theme` becomes Light; the window switches to light; the choice is saved with the rest of the UI state (`humata2-ui.json`, `SavedUi.preferences`). Dark and System are the same reducer path (`SetTheme`).
- **Evidence:** existing test asserts Light turns the theme off dark: `native/hosts_settings.rs` `settings_visit_keeps_both_tab_groups_and_layout_rows_work`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=hosts_settings::settings_visit_keeps`
- **PENDING RUN:** to see the Dark and System tab states, `bazel test //apps/humata2/desktop:native_test --test_filter=hosts_settings::hosts_and_settings_reference_images` (already draws 29-settings in Dark) and copy `bazel-testlogs/.../test.outputs/` if any `.new.png` appears.

### 3. Terminal font: a pop-up with Menlo 10 to 20
- **Screen:** Layout › Terminal font control reads "Menlo 12" with a chevron. Open, it shows a list "Menlo 10" through "Menlo 20", the current one with a check mark.
- **Offered:** the control (open/close); each size; clicking outside (overlay path, `features/app/src/overlay.rs`).
- **Action:** open it, pick Menlo 13.
- **Result:** the size becomes 13 (`SetFontSize` clamps to 10..=20), the list closes, and the terminals and diffs use it (`ui/src/terminals.rs`, `panel_changes.rs`). Switching section also closes an open list (`FontMenu` is reset on `Section`/`Reveal`).
- **Evidence:** `native/hosts_settings.rs` (asserts `settings-font-13`, closes, reads 13).
- **Replay check:** `bazel test //apps/humata2/features/settings:preferences_test --test_filter=picking_a_font_size`
- **Replay check (section close):** `bazel test //apps/humata2/features/settings:preferences_test --test_filter=switching_section_closes`

### 4. Sidebar shows Pinned
- **Screen:** Layout › "Sidebar shows Pinned", a toggle, note "Pinned sessions, reviews and machines at the top."
- **Offered:** the toggle.
- **Action:** click it (on to off).
- **Result:** `show_pinned` flips; the sidebar's Pinned section disappears (`sidebar-pinned` not in the tree) and returns when turned back on. Saved with preferences.
- **Evidence:** `native/hosts_settings.rs` `settings_visit_keeps_both_tab_groups_and_layout_rows_work`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=hosts_settings::settings_visit_keeps`

### 5. Long lines: Wrap or Scroll
- **Screen:** Layout › Long lines, a two-tab control Wrap / Scroll; note "Code in the diff, the editor, Files and reviews."
- **Offered:** Wrap, Scroll. Clicking the selected one does nothing (`if !on`).
- **Action:** click Wrap.
- **Result:** `wrap_diffs` true and `review.wrap` true.
- **Evidence:** the same native test.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=hosts_settings::settings_visit_keeps`
- **Note:** the palette row "Long lines" is `Row::LongLines` (see step 16).

### 6. General: Confirm bulk approvals
- **Screen:** General › card "Confirm bulk approvals", note "Ask before approving requests that cannot be undone.", toggle on by default.
- **Offered:** the toggle.
- **Action:** toggle off.
- **Result:** `confirm_destructive` flips. Nothing else changes on the page. Note the Settings text says only "requests that cannot be undone" while the row title says "bulk approvals"; the consumer of the flag is outside this area.
- **Evidence:** feature test only (no snapshot).
- **Replay check:** `bazel test //apps/humata2/features/settings:preferences_test --test_filter=preferences_change_without_losing`

### 7. General: Free memory of hidden terminals
- **Screen:** General › "Free memory of hidden terminals", note explaining a 30-second rule and that terminals with unsent typing are never freed. Toggle on by default.
- **Offered:** the toggle.
- **Action:** toggle off.
- **Result:** `free_hidden_terminals` false; terminals keep every screen in memory. Turning it back on restores the default.
- **Evidence:** feature test; the setting is read by `ui/src/terminals.rs` (`memory_saver(free_hidden)`).
- **Replay check:** `bazel test //apps/humata2/features/settings:hidden_terminals_test`

### 8. General: Repositories (default scope and library)
- **Screen:** under General, a "Repositories" block: "Search scope" chips (owners, each with a ✕), "+ <org>" pills for owners the library has but the scope does not, an "Include my repos" switch, then "Repo library" with rows (most recently used first), a "Remove" per row, and "Forget all…". Empty state: "No repos yet. A repo you add to a channel or the Desk shows up here." While the library reads: "Reading the library…".
- **Offered:** ✕ per owner; "+ owner"; Include my repos switch; Remove per repo; Forget all… → Cancel / Forget all.
- **Action:** Open General (reads library, scope and GitHub orgs in parallel); add "+ <org>"; toggle the switch; remove a repo; Forget all… then Forget all.
- **Result:** each scope change saves at once through `save_repo_scope`; the chips update from what the host kept. Remove takes one repo out. Forget all is ignored unless the confirm is open and the library has repos. Failures set `repos.error`, shown as `settings-repos-error` under the block; `busy` stops a second change until the host answers.
- **Evidence:** feature tests only. Note: Remove and Forget all are not chained into a single state in the feature tests beyond the listed names.
- **Replay check:** `bazel test //apps/humata2/features/settings:repositories_test`
- **Replay check (owners):** `bazel test //apps/humata2/features/settings:repo_owners_test`
- **PENDING RUN:** no screenshot exists for General with repos. Phase 2 may take `bazel test //apps/humata2/desktop:native_test --test_filter=hosts_settings::hosts_and_settings_reference_images` only if the General view is added; otherwise record as code-only.

### 9. Notifications: permission request alerts
- **Screen:** Notifications section: intro "Requests that need you appear in Inbox and beside the session in the sidebar." Card "Permission request alerts", a note "Show brief alerts for new permission requests. Choose where approvals appear in Agents.", toggle off by default.
- **Offered:** the toggle.
- **Action:** toggle on, then off.
- **Result:** `permission_request_alerts` flips; the waiting-requests count does not change. The alert itself is raised in `features/app/src/events.rs` when on.
- **Evidence:** `native/hosts_settings.rs` `permission_request_alerts_can_be_toggled_without_removing_requests`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=hosts_settings::permission_request_alerts`

### 10. Agents pane (not switches): Default agent
- **Screen:** Agents › "Default agent" card with one button per offered agent plus Shell; the current one is filled in the accent colour. Snapshot `native/29-permission-route.view.txt` lists Claude, Codex, Copilot, OpenCode, Shell.
- **Offered:** each agent button.
- **Action:** click Codex.
- **Result:** `default_agent` becomes Codex; new sessions and the new-channel default use it (`ui/src/channel_desk.rs`, `features/app/src/new_session.rs`). Disabled agents (from the switches) are not offered here.
- **Evidence:** `native/29-permission-route.view.txt`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=hosts_settings::permission_requests_can_use`
- **Note:** this list shows every built-in agent before the daemon's switches load (`offered()` falls back to all).

### 11. Agents pane: Ask for permission in the Inbox
- **Screen:** card "Ask for permission in the Inbox", note fixed as "Off: each agent asks in its own terminal" whatever the state; toggle on until the daemon says otherwise (`asks_in_inbox != Some(false)`).
- **Offered:** the toggle.
- **Action:** toggle off.
- **Result:** the switch shows off at once (`asks_in_inbox = Some(false)`), the daemon saves; `asks_saving` blocks a second toggle while in flight.
- **Failure:** the daemon refuses → the switch goes back to its old value and `settings.error` shows under the Agents pane as `settings-agents-error` (alert role) with the daemon's wording.
- **Evidence:** `native/29-permission-route.view.txt` (after the toggle, Inbox off).
- **Replay check:** `bazel test //apps/humata2/features/settings:agent_asks_test --test_filter=a_refused_change`
- **Replay check (toggle):** `bazel test //apps/humata2/features/settings:agent_asks_test --test_filter=turning_it_off_and_on`

### 12. Agents pane: Trust Humata's hooks in Codex (step, when needed)
- **Screen:** a card "Trust Humata's hooks in Codex" with the text "Start a Codex session and run /hooks once, then trust Humata's". It shows only when the daemon says Codex hooks need trusting.
- **Offered:** nothing (no button; the card's control is an empty element).
- **Action:** none in the app.
- **Result:** the card goes away when the daemon hears a hook (`codex_needs_trust` false).
- **Evidence:** feature test.
- **Replay check:** `bazel test //apps/humata2/features/settings:codex_trust_test --test_filter=hooks_not_yet_heard`

### 13. Agent access section: a stub that points at Inbox
- **Screen:** the "Agent access" tab shows the heading "Agent access", the text "Requests to change items you own wait in Inbox. Review each request before allowing it.", and one button "Review Inbox". No rules are listed here.
- **Offered:** Review Inbox (switches to the Inbox route).
- **Action:** click Review Inbox.
- **Result:** the Inbox route.
- **Evidence:** code only (`ui/src/settings.rs`, `Section::Access`). PENDING RUN: none needed to confirm the stub; a headless look is `bazel test //apps/humata2/desktop:native_test --test_filter=hosts_settings::settings_visit_keeps` after clicking the tab if phase 2 adds a click.
- **Replay check:** `bazel test //apps/humata2/features/settings:access_test --test_filter=setting_a_rule` (the reducer exists; the Settings page does not render it).

### 14. Per-channel rules live on the channel's Agent access tab, not in Settings
- **Screen:** channel page tab "Agent access" (the intro: "What agents in this channel may do through Humata..."), one row per capability with a three-way control Allow / Ask me / Never; capabilities with no setting show "Always · no setting"; grants under a rule show "Always allow · <session>, <scope> · granted <when>" with Remove. Snapshot `design/49-channel-agent-access.png` (golden).
- **Offered:** Allow, Ask me, Never per rule; Remove per grant; the error banner's ✕.
- **Action:** pick Never on a rule.
- **Result:** the rule's row is pending (dimmed) until the daemon answers; a second change to the same rule is ignored while pending. A refused change shows `access-error` with the daemon's wording and a dismiss.
- **Evidence:** golden `design/49-channel-agent-access.png`; feature test.
- **Replay check:** `bazel test //apps/humata2/features/settings:access_test --test_filter=a_failed_set_shows_an_error`
- **Replay check (second set ignored):** `bazel test //apps/humata2/features/settings:access_test --test_filter=a_second_set_for_the_same_row`
- **Replay check (remove grant):** `bazel test //apps/humata2/features/settings:access_test --test_filter=removing_a_grant_runs_and_clears`

### 15. Machine page: "Change in Agent access" leads to the stub
- **Screen:** the machine page has a button "Change in Agent access".
- **Action:** click it.
- **Result:** Settings opens at the Agent access section, which shows only the "Review Inbox" button (step 13). The per-channel rules are not shown; the person must go to a channel's Agent access tab.
- **Evidence:** code (`features/app/src/machines.rs` `PageDelegate::ChangeAgentAccess`; `ui/src/machine_page.rs` label).
- **Replay check:** none exists in the tests; PENDING RUN is the flow itself: `bazel test //apps/humata2/desktop:machines_flow_test` (look for the machine-change-access id), then confirm the Settings view.

### 16. Palette: settings search
- **Screen:** the palette lists "Settings › <section>" (8 entries, hint "Settings") and "Settings › <row>" (10 entries, hint = the row's section name). Rows: Appearance, Terminal font, Sidebar shows Pinned, Long lines, Confirm bulk approvals, Default agent, Ask for permission in the Inbox, Permission request alerts, Free memory of hidden terminals, Option acts as Alt. Snapshot `native/30-palette.view.txt` (palette open; Settings item in the rail).
- **Offered:** each entry; typing filters them.
- **Action:** type "long", choose "Settings › Long lines".
- **Result:** Settings opens; section Layout; `revealed = Some(LongLines)`. Recorded in the state only: nothing on the screen is highlighted or scrolled to (see Surprises).
- **Evidence:** live flow `desktop/tests/live_flows/find.rs` step 4.
- **Replay check:** `bazel test //apps/humata2/desktop:live_flows_test --test_filter=find::from_the_palette`
- **PENDING RUN:** to see the palette list with the Settings entries, phase 2 runs the same live flow and copies the palette screen.
- **Not in the palette:** Machines, Storage and Agent access rows (no `Row` for them); their sections are listed, and Scripts is not part of this record.

### 17. Keyboard: Option acts as Alt
- **Screen:** Keyboard section: a "Keyboard shortcuts" button (opens the shortcuts sheet, ⌘/), and a card "Option acts as Alt" with a long note: for Mac keyboards; ⌥ keys send Esc and the key; some layouts then cannot type characters that need ⌥; ⌥←, ⌥→, ⌥⌫ move by word either way. Toggle off by default.
- **Offered:** Keyboard shortcuts; the toggle.
- **Action:** toggle on.
- **Result:** `option_as_alt` true; terminals send Esc+key for ⌥ (`ui/src/terminals.rs`). Off types the layout's characters.
- **Evidence:** feature test; shortcuts sheet snapshots `native/shortcuts-sheet-light.png` (existing).
- **Replay check:** `bazel test //apps/humata2/features/settings:option_as_alt_test --test_filter=toggling_turns_it_on`
- **Replay check (palette row):** `bazel test //apps/humata2/features/settings:option_as_alt_test --test_filter=the_row_is_in_keyboard`

### 18. Storage: a list of machines with no storage on it
- **Screen:** "Storage by machine" heading, then one button per machine (name). No per-machine detail appears on this page.
- **Offered:** each machine button.
- **Action:** click a machine button.
- **Result:** the app leaves Settings and opens the Hosts route with that machine selected (`AppAction::Hosts(Select)` switches to `Route::Hosts`). Storage numbers are on the machine page (`design/60-machine-storage.png`), not here.
- **Evidence:** code (`ui/src/settings.rs`, `Section::Machines | Section::Storage`; `features/app/src/lib.rs` `Hosts(Select)`).
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=hosts_settings::hosts_and_settings_reference_images` (covers Hosts; the Storage click is not in a test). PENDING RUN: none.

### 19. Machines: the Humata folder (Change… sheet)
- **Screen:** Machines › per machine a card "Humata folder" showing the root path and a "Change…" button. The sheet "Humata folder dialog": heading "Humata folder on <machine>", text "New channels are made under this folder. Type a full path or one that starts with ~/.", a text field, "Plan". After a plan: a summary ("N channels under <old>: …" or "No channels are under <old>. Only new channels will use <new>."), and buttons per plan: "Change" (no channels), or "Use for new channels only" and "Move them" (channels exist; Move them disabled while a session runs there, with a note). Cancel always.
- **Offered:** Change…; typing the folder; Plan; Change / Use for new channels only / Move them; Cancel.
- **Action:** Change…, type `~/work`, Plan, then Use for new channels only.
- **Result:** Plan asks the daemon for the plan; editing the text drops the plan; apply runs the change; on success the sheet closes and the row shows the new root and a `RootChanged` delegate fires. A failed plan or change shows `root-error` (alert) in the daemon's words and keeps the sheet open; Cancel is disabled while a change runs.
- **Evidence:** existing golden text is in `desktop/tests/root_change.rs` and `desktop/tests/design/machine_page.rs`; no run yet.
- **Replay check:** `bazel test //apps/humata2/desktop:root_change_test`
- **PENDING RUN:** `bazel test //apps/humata2/desktop:root_change_test` then a headless snapshot of the sheet is not in the test; phase 2 records the sheet only if a test draws it.
- **Note:** the sheet asks the person to type a full path. See Surprises.

### 20. Machines: Scripts (out of scope, listed for the reader)
- Covered by another area (MACHINESCRIPTS). Shown on the same page under Machines. Not walked here.

### 21. Failed preference save (any Layout or General toggle)
- **Screen:** none specific. A save that fails shows a toast with `couldn't save where you are: <cause>. If Humata restarts now it may open where you were a moment ago.` (`features/app/src/saved.rs` `SaveError`, `persist.rs` `save_finished`).
- **Action:** would need storage that refuses the write.
- **Result:** toast only; the preference stays changed in memory.
- **Evidence:** none. No headless test injects a storage failure.
- **Not reachable headless:** no fault-injecting storage is wired into the native harness that I found; a test would need one. Surface to the coach as a gap.

### 22. Restore after an unreadable saved file
- **Screen:** a banner "the saved window state from the last run couldn't be read (<cause>), so Humata starts on Home. It is replaced the next time anything changes." (`RestoreError::Decode`), shown by `restore-error` with a dismiss.
- **Result:** all preferences come back at their defaults (theme System, font 12, etc.) because the whole file is ignored, and the file is replaced on the next change.
- **Evidence:** code (`features/app/src/saved.rs`, `ui/src/shell.rs`). The corrupt-file path is written in `ui/src/starting.rs` for a test save (`b"not json"`), not in a settings test.
- **Not reachable headless as a settings check:** no settings test reads the banner.

## Not reachable headless
- Save and restore failures (steps 21, 22): no test with failing storage found.
- OpenCode API key entry: see Surprises (no control exists).
- Native screenshots of General with repos, of the Humata folder sheet, and of Dark and System theme tabs: no existing golden; phase 2 can add only what an existing test draws.

## Surprises
- **Agent access in Settings is a stub.** The Settings "Agent access" pane shows only "Review Inbox". The working rules UI is on each channel's Agent access tab (`ui/src/access.rs`), and the machine page's "Change in Agent access" lands on the stub (step 15).
- **The API key reducer has no UI.** `SettingsAction::Key` and `SaveKey`, `SettingsState.key` and `has_key` exist, and `KEY_NAME` is "opencode-api-key", but no view dispatches them. A person cannot enter an OpenCode key from Settings.
- **Notifications copy points at a control that is not there.** "Choose where approvals appear in Agents" and the Agents row note "Off: each agent asks in its own terminal" is fixed text whatever the switch says.
- **The palette reveal does not reveal.** `Reveal(row)` sets `revealed` and the section, but nothing in `ui/` reads `revealed`. The row is not highlighted or scrolled to.
- **Storage is a machine list with no storage in it.** Its heading says "Storage by machine"; clicking a machine leaves Settings for the Hosts page.
- **Machine buttons navigate away.** In Machines and Storage, each button calls Hosts Select, which switches route; the person loses the Settings page.
- **Humata folder is typed as a path.** The sheet asks for a full path or `~/…`. This is free text, not a picker; the requests file says no folder picking for owner-facing placement, so the coach should check this against that rule.
- **Toggles are unlabelled to assistive tech.** Every toggle is `aria_label` "On" or "Off" with no name (`settings.rs` `toggle`), so a screen reader hears no setting name. The snapshot shows "Button On selected" alone.
- **Confirm bulk approvals copy is narrow.** The row is titled "bulk approvals" but its note says only "requests that cannot be undone".
- **Preferences have no error path on the page.** Only a save toast (step 21) reports a failed write; the Settings page itself never shows it.
- **Font size list is unlabelled beyond "Menlo N".** It has no preview; the code shows only the names (`settings.rs` `font_menu`).

# LOOK2: Every other screen looks like the goldens

- **Status:** done (look_test passes every frame under 0.5%; `app_fakes` built and installed as Humata 2 (fakes))
- **Depends on:**
  - LOOK1: shares the shell, theme and sidebar; and C1 must release the UI files
  - P0: the panels and native tests must be split before a second manager edits them
  - W1 (to close, not to open): frames 18, 29, 31, 32, 33 change; the app matches the redrawn ones before this closes
- **Unblocks:** DESK, BOARD, FIND
- **Layer:** L1 · **Lane:** UI

## Outcome
On made-up data, the owner clicks through the channel page, boards, Home, the Inbox, Hosts, Settings, the palette, New session, toasts and the empty states, and they look like frames 18–35.

## Exit criterion
For each of frames 18–35 the native capture matches the design frame within tolerance and one flow reaches it. After W1: no Panes setting and no Fixed/Simple switch (D111), no model picker, the Last used column (SE-26), and D113 items 5, 7, 8, 9 applied. Platform behaviour holds: standard shortcuts, ⌘, for Settings, the native menu bar. A release of the app on fakes, labelled as such, is installed.

## Read these (pointers, nothing else)
- SPEC: CH-11–CH-15, CH-9, BD-2–BD-5, BD-11–BD-13, HM-1–HM-5, IN-3, IN-5, IN-7, HO-3, ST-18–ST-20, KP-1–KP-3, SE-2, SE-26, NV-7–NV-11, NV-27, DK-6, MV-15; flow §26.22 (on fakes). Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: 18–35 (`desktop/tests/__snapshots__/design/NN-*.png`)
- Decisions: D76, D99, D106, D107, D109, D110, D111, D112 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `ui/src/channel.rs`, `ui/src/board.rs`, `ui/src/pages.rs`, `ui/src/hosts.rs`, `ui/src/settings.rs`, `ui/src/overlays.rs`, `ui/src/lists.rs`, `features/board`, `features/hosts`, `features/inbox`, `features/palette`, `features/settings`, `desktop/tests/native.rs`, `desktop/tests/design/pages.rs`
- Also: OVERVIEW.md › "N1: A native macOS look and feel" (open items)
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Carried over from P0
Split `ui/src/panels.rs` and `desktop/tests/native.rs` the same mechanical way P0 split the others (see the P0 commit 1a3831dd), first, once C1 has released them.

## Out of scope
The Inbox card's full context and grants (ASK). Board behaviour beyond what is drawn (BOARD). Desk frames 36–40 and markdown frames 39, 41–43 (DESK, REVIEW). Palette search across machines and pins (FIND).

## Polish carried from LOOK1's audit (non-blocking)
- Composer input box in the fake TUI (all session frames): golden is a rounded box about 25 pt tall with "> " and a gap before the cursor, hint lower; the native fake draws a square cell box about 35 pt tall with the cursor touching ">". Fix in the fake program's drawing (clients/src/fixture/terminal.rs or the fake terminal), not the view.
- "⎿" sits against the next text: the fallback glyph is wider than its cell (it already stands alone, frame.rs `is_technical`); centring or scaling a too-wide fallback glyph in its cell would fix it.
- Frame 14: a block cursor shows in the unfocused codex pane below "Working…"; golden has none (unfocused panes might draw a hollow cursor or none).
- Banner buttons (Open it, Later, Not now) in 03/04/15 sit about 17 pt left of the golden; Info rows in 05 are 3–8 pt lower with slightly smaller text; "Channel rules" link indented 7 pt.
- Header right cluster and panel-header icons offset by 1–6 pt; text baseline about 1 px lower everywhere.
- Frame 11: "Open in editor" draws dimmed (no editor wired); "against main" selector wider; Show the terminal / Send 1 comment off by a few pt.
- Frame 06: Files filter about 10 pt shorter (gpui_kit Input height); golden greys the composer while offline.
- Frame 15: pinned card ellipsis differs.
- Frame 07: board rows 1–3 pt off.

## Lane (written by the manager)
Manager: minion LOOK2 (Fable 5.2 high), opened 2026-11-15 while LOOK1's last batch commits.

- **Goal / exit:** as above. Each of frames 18–35 (33 is retired, D128.4) has a native capture that matches its golden within tolerance, reached by a flow; the W1 changes are in (no Panes switch, no model picker, Last used, D113 items 5, 7, 8, 9); shortcuts, ⌘, and the menu bar hold; main installs a release on fakes.
- **Design notes:**
  - The goldens are drawn by code (`desktop/tests/design/{channel,board,pages,overlays,desk,kit,sidebar}.rs`), so every size, gap, colour and string is read from there, not guessed from pixels. Workers compare the golden PNG with the latest native capture, then read the drawing code for the numbers.
  - One view file per worker (`ui/src/<screen>.rs`), so workers run side by side. A worker never renames or removes a test id a flow uses. What a view can't fix alone (fixture data, shared chrome, new state) comes back in its report and becomes a batch 2 task.
  - Split first, once LOOK1 is committed: `ui/src/panels.rs` → `panel_{changes,inbox,board,journal,files,info}.rs` as `#[path]` children (the overlays.rs pattern; the ui target already globs `src/*.rs`); `desktop/tests/native.rs` keeps the `App` harness and `snapshot()` (its `snapshots!()` must stay in the root file: the folder comes from `file!()`), tests move to `tests/native/{shell,session,terminal,queue,review,channel,board,lists,hosts_settings,overlays}.rs` as `#[path]` children. Seam for main: `native_test` `srcs = ["tests/native.rs"] + glob(["tests/native/*.rs"])`.
  - Last used (SE-26, D99): `Session.last_used: String` beside `started` in `clients/src/model.rs` (main-only seam), set in the fixtures, drawn as the Sessions table's last column with the sort mark (frame 18). Real values come with FL's daemon events; on fakes it is fixture text.
  - No Panes switch (D111): `SplitGroups` leaves `features/settings` and the view; a session may always split (SE-27). It goes in batch 2 with its test, because the native flow clicks `settings-panes-Fixed`.
  - Frame 33 is retired: an empty workspace opens Home with Get set up (DK-6, frame 34) and the sidebar stays (No channels yet, New channel…, Add a machine…). The `first-channel-create` and `first-machine-add` ids move onto the Get set up rows so the flow keeps working.
- **Picks (reversible; logged here, D-entry when main next touches DECISIONS.md):**
  - K1: the Desk pieces the goldens draw on these screens (sidebar Desk row in every frame, "Use the Desk / Open Desk" and "New desk session" in 34) are DESK's, as LOOK1 ruled for its frames: the route doesn't exist yet and a row with no action breaks NV-7. LOOK2 leaves the slots; DESK fills them and re-records. Reverse: draw them disabled now.
  - K2: the Inbox card's inside (frames 26, 27) stays as ASK built it (`ui/src/request_card.rs`); LOOK2 matches the list, the switch, the split and the card's frame only.
  - K3: frame 32's toast file (`ui/src/overlay_toasts.rs`) is BUG's; LOOK2 sends BUG/main the exact lines rather than editing it.
- **Claims (now):** `ui/src/{channel,board,lists,pages,hosts,settings,overlay_palette,overlay_new_session}.rs`, `features/{board,inbox,palette}/`, `workflow/milestones/LOOK2.md` Lane, OVERVIEW block "Pick up here (minion LOOK2)".
- **Claims (from "LOOK1 committed"):** `ui/src/{panels,session_tabs,sidebar,terminals,theme,chrome,widgets}.rs`, new `ui/src/panel_*.rs`, `desktop/tests/native.rs`, new `desktop/tests/native/`, `desktop/tests/{panels,shell}.rs`, `desktop/tests/__snapshots__/{native,panels,shell}/` (main records), `clients/src/fixture/{design_session,sessions,requests,terminal,machines,items,journal,files}.rs`, `clients/src/fixture.rs`, `features/settings/` (`features/sidebar/src/lib.rs` is DESK's since 2026-11-15: changes there go through main). Seams go to main as exact lines: `ui/src/{lib,shell,overlays}.rs`, `features/app/src/lib.rs`, `clients/src/model.rs`, `desktop/BUILD.bazel`, `desktop/src/main.rs` (FL, BUG). Not: `ui/src/{request_card,access}.rs` (ASK), `ui/src/overlay_toasts.rs` (BUG), `features/hosts/` (MACH), `features/review` (REVIEW), goldens, `desktop/tests/design*`.
- **Tasks:**
  - Batch 1 (view-only, test ids kept; written and checked by the manager, not built):
    - [ ] T1 channel page, frames 18, 19, 20, 35 · worker ch (Fable) · `ui/src/channel.rs` · written, checked. Header button and badges, filter field 26 pt, switch, Expand/Collapse as links, ✉ count, Activity buttons, Repos grid links; new empty state (frame 35) when the channel never had a session. Sessions tab hides its count at zero (frame 35). "Add a machine…" on the Repos tab now opens Add machine (was disabled).
    - [ ] T2 boards, frames 21–24 · worker bd (Fable) · `ui/src/board.rs` · written, checked. Scope and Status as pop-ups, switch, filter, button shape, lane "+", chips, drawer icons/headings/dependency chips, table cells and Review, dashboard colours, headings and the two tile notes from BD-13 ("n sessions exited →", "+n on last week").
    - [ ] T3 Home, Inbox, empty Home, frames 25, 26, 27, 34 · worker hm (Fable) · `ui/src/lists.rs`, `ui/src/pages.rs` · written, checked. Button shape, empty groups, Get set up (ids `first-channel-create`, `first-machine-add` kept; `first_run` removed), Allow all n / Deny all strip with its confirm (IN-6, existing actions), switch metrics, answered rows say how they were answered.
    - [ ] T4 Hosts, frame 28 · worker ho (Sonnet, medium) · `ui/src/hosts.rs` · written, checked. Button and chip metrics only.
    - [ ] T5 Settings, frame 29 · worker st (Sonnet, medium) · `ui/src/settings.rs` · written, checked. Long lines row in Layout (`settings-long-lines-{Wrap,Scroll}`), segment metrics, font control in a pop-up-shaped box. Panes block and General's `settings-wrap` stay until T9.
    - [ ] T6 palette and New session, frames 30, 31 · worker ov (Sonnet, medium) · `ui/src/overlay_palette.rs`, `ui/src/overlay_new_session.rs` · written, checked. Palette field without a ring, shortcut chips; New session title field, Runs switch and its note, pop-ups with the chevrons on the right, button shape. Toast (32) audited only.
    - Manager: `Input::h` only applies to multi-line inputs in the component crate, so the board filter and the New session title use `min_h(26)`.
  - Splits (mechanical, no behaviour change; scripts in the scratchpad `look2/split/`):
    - [ ] T7 `ui/src/panels.rs` → root (helpers, `panel`, `strip`) plus `panel_{changes,inbox,board,journal,files,info}.rs` as `#[path]` children that start with `use super::*;`; moved items became `pub(super)`. Checked: the old file's lines equal the new files' lines apart from those.
    - [ ] T8 `desktop/tests/native.rs` → root (imports, `App`, new `folder()`) plus `tests/native/{shell,session,terminal,queue,review,channel,board,lists,hosts_settings,overlays}.rs`; one line changed (the shell test takes its snapshot folder from `folder()`, since `snapshots!()` names the folder after the calling file). Needs the BUILD seam.
  - Gate g34 (batch 1 + splits): build clean; ui_unit, native, panels, shell, report and design tests pass; native 18–33 re-recorded; native 01–17 and panels unchanged; `shell/home.png` changed because it draws Home (T3's `lists.rs`). The record run's one failure was batch 2's T9 landing mid-gate.
  - Batch 2 (written and checked by the manager, not built):
    - [ ] T9 no Panes switch (D111) · worker panes (Sonnet, medium) · `features/settings/{src/lib.rs,tests/preferences.rs}`, `features/app/tests/presentation.rs`, `ui/src/{settings,session_tabs,terminals}.rs`, `desktop/tests/native/hosts_settings.rs` · `split_groups` and `SplitGroups` are gone, a session always splits; General's `settings-wrap` went, Layout's Long lines row is the control; the flow is now `settings_visit_keeps_both_tab_groups_and_layout_rows_work`.
    - [ ] T10 Last used (SE-26) · manager · seam script `look2/seams/seam_last_used.py` (model field, three Session literals, DESK's fake if present, the design fixture's six values, the Sessions column, the palette's "used <when>"); main applies it.
    - [ ] T11 fixture data per the advisor's rulings (D147) · worker fixture (Fable) · `clients/src/fixture/design_session.rs` · written, checked: Try BRIN's Home line and time; gpu-box "last seen 14:02"; the Inbox lists by age; two answered requests (`create-pay-9`, `open-port-5432`) for the All view (every count filters on unanswered, so Inbox 7 and the badges hold); pr-reviews ages; INF-1's chip is a label. Manager: `features/inbox/tests/selection.rs` follows (the design workspace now ends with answered requests); answered rows carry no VIP mark (`ui/src/lists.rs`).
    - [ ] FL's T9 view lines (main's ask) · manager · `ui/src/lib.rs` (`mod restart_report; mod resume_failed;`), `ui/src/session_tabs.rs` (the reboot line and the couldn't-resume card over the agent terminal's notice), `ui/src/lists.rs` (Home's restart groups first). They compile only with FL's model and features/app seams.
    - [ ] T12 empty states · worker empty (Fable) · `ui/src/sidebar.rs`, `clients/src/fixture.rs` (`fresh_channel()`), `desktop/tests/native/{lists,channel}.rs` · an empty workspace gets the normal sidebar with "No channels yet", "New channel…" (`sidebar-first-channel`) and "Add a machine…" (`sidebar-add-machine`); a channel with no session shows "No sessions yet" unless folded; captures `34-empty-home` (light, from `fixture::empty()`) and `35-empty-channel`; frame 27's flow switches to All. Manager: `App::fold_quiet_channels()` in native.rs, used by the 21–24, 28–32 flows (their goldens draw pr-reviews and web folded).
    - [ ] T13 toast · manager · Undo has no pad of its own (`ui/src/overlay_toasts.rs`); the "Frees 1.4 GB when this closes." wording is a seam proposal (`look2/seams/seam_toast_frees.md`).
    - [ ] T15 controls with no action yet, drawn disabled (D147 L4) · manager · `ui/src/channel.rs`: Pin and "…" in the header, "Add a GitHub repo…" on the empty channel.
    - [ ] T16 platform · manager · `ui/src/keys.rs`: `OpenSettings` on ⌘,; the Shortcuts sheet lost five pane shortcuts that had no binding. Seams: the shell handler and the Settings… menu item. Edit and Window menus: next batch.
    - [ ] App rules from the rulings · manager · Home rows name their machine when it isn't This Mac (`ui/src/lists.rs`, HM-1); the dashboard's By session lists by need (`ui/src/board.rs`).
  - Gate (batch 2): committed with batch 1 as 781bcf58; native, panels and shell re-recorded; 34 and 35 added, 33 removed; goldens unchanged.
  - Batch 3 (second pass on main's fresh captures; written and checked by the manager, not built):
    - [ ] T14 one push button · manager · `ui/src/widgets.rs` (`push`, `push_quiet`, `push_primary` and the two `_shape` forms) · every frame button in channel, board, lists, hosts, New session and the compact board's Review uses it; local helpers `shape`, `quiet`, `dress` are gone. Files filter is 26 pt (`ui/src/panel_files.rs`, `min_h`).
    - [ ] T18 re-audit 18–35 · workers ch2, bd2, hm2 (Fable), hs2, ov2 (Sonnet, medium) · pixel comparison of each golden with its fresh capture, page area only. Result: outside the buttons (4 pt narrow, fixed by T14) and the logged data differences (D147), boards 21–24 are identical; channel 18–20 and 35 are within 1 pt once Pin and "…" draw at 15 pt (fixed) and frame 35's Add a GitHub repo… takes the filled shape, dimmed (fixed); Home and Inbox 25–27 match; 34 gains the Desk row and "New desk session" (now that the Desk exists); Hosts 28 matches; Settings 29 matches but for the font control (L10); palette 30's query text moved 8 pt left to the frame's place; New session 31 matches but for the First message box (L13); toast 32 matches but for its wording (seam proposal).
    - [ ] T17 Edit and Window menus · manager · exact lines for `desktop/src/main.rs` in the scratchpad `look2/seams/seam_menus.md` (FL's file; main applies).
    - Manager: the empty-Home flow clicks `home-new-desk-session` and `first-desk-open` (`desktop/tests/native/lists.rs`).
  - Frame wave (D158; the owner handed it to a Fable advisor, whose rulings are approved; written and checked by the manager, not built):
    - [ ] W1 frame code · worker frames (Fable) · `desktop/tests/design/{pages,resume,channel,channel_tabs,overlays,board,machine_page,sidebar,session}.rs` · rulings 1–6, 8, 12 and the frames that follow (05, 49's read row, 59). Goldens to re-record: 05, 18, 19, 20, 22, 25, 28, 30, 32, 44, 45, 47, 48, 49, 56, 57, 58, 59, 67, 71.
    - [ ] W2 font size pop-up (ruling 9) · worker font (Sonnet, medium) · `features/settings/{src/lib.rs,tests/preferences.rs}`, `ui/src/settings.rs`, `desktop/tests/native/{hosts_settings,review}.rs` · `SettingsState.font_menu`, `FontMenu`, `SetFontSize(u8)` (10–20); the control is the frame's pop-up (`settings-font`) with a list (`settings-font-N`); the stepper and `FontSize(delta)` are gone.
    - [ ] W3 dot rule B (ruling 7) · manager · SPEC CH-16 and D131/D130.1 reworded; no view change (the dot is the status the daemon reports); the hold-through-the-gap note is for the daemon lanes.
    - [ ] W4 Agent access (ruling 12) · manager · SPEC IN-17 row; `clients/src/fixture/rules.rs` gains "Global board, or other channels' boards: change" (Ask me).
    - [ ] W5 toast (T13) · manager · `look2/seams/seam_toast_frees.py`: the archive reply carries the freed size; main applies it.
  - Left for the close:
    - [ ] a last look at the captures main records for batch 3 and the wave, then main cuts and installs the release on fakes
- **Close pass (minion LOOK2, Opus 5.5, 2026-11-16).** Audit of 18–35 on the g80 captures (page area, 2 pt blocks; Haiku audits plus `look2/close/sim.py`): 18–23, 28, 34, 35 match; 24, 26, 27, 29, 31 differ only where a ruling accepts it once masked; 25 and 32 miss D158.7 on Home; 30 misses on row pitch and an empty Conversations group; 32's Undo is a button. Plan, with the seams in the scratchpad `look2/close/seams/SEAMS.md`:
    - [x] C1 Home follows dot rule B · manager · `ui/src/lists.rs` (`asks()`: "Asks to move PAY-3 to Done") · seam S1 for main: `features/app/src/home.rs` and `features/app/tests/home_needs_you.rs` (Needs you is NeedsYou, Exited or Offline; exited rows say why and offer Resume).
    - [x] C2 New session and Settings nits · manager · `ui/src/overlay_new_session.rs` ("11 GB free" and the Branch line top-aligned, First message 12.5 pt), `ui/src/settings.rs` (nav icons 15 pt).
    - [x] C3 the automated comparison · manager · new `desktop/tests/look.rs` (`look_test`: page right of the sidebar, 2 pt blocks, channel 48, budget 0.5%, one mask per accepted ruling) · seam S2: the BUILD target.
    - [ ] C4 palette (Fable ruling: hide an empty Conversations group; rows don't shrink; 11 pt headers, 15 pt icons) · FIND's files, seam S4.
    - [ ] C5 toast Undo as text (BUG2's `ui/src/overlay_toasts.rs`), board Filter placeholder colour (BOARD's `ui/src/board.rs`), request card radius 9 · seams S5, S6.
    - [ ] C6 release on fakes · seam S3: `:humata2_fakes` binary (`HUMATA2_BACKEND=fakes`), `//apps/humata2/app:app_fakes` ("Humata 2 (fakes)", bundle id `.fakes`), installed beside the live app. 0.1.0 always runs live from its bundle, so it doesn't count.
  - Exit: `look_test` passes, `native_test` passes but the queue test (another lane's), goldens unchanged, `Humata 2 (fakes).app` in /Applications.
- **Waiting on:** main: seams S1–S3 and the gate in `look2/close/seams/SEAMS.md`; FIND, BUG2 and BOARD (or main) for S4–S6.
- **Seams for main:** the scratchpad `look2/seams/seams_wave.md`: Escape closes the font list (`features/app/src/overlay.rs`), and `seam_toast_frees.py`. It also lists every file the wave changed, the goldens that re-record and what the ruling left inconsistent (frames 46, 47, 49, 71).
- **Build gates (batch 3 + wave):** `bazel build //apps/humata2/clients //apps/humata2/ui //apps/humata2/desktop:humata2`; `bazel test //apps/humata2/clients/... //apps/humata2/features/... //apps/humata2/ui:ui_unit_test`; `bazel run //apps/humata2/desktop:design_test_record` then `bazel test //apps/humata2/desktop:design_test` (only the twenty goldens above, plus DESK's own, may change); record `//apps/humata2/desktop:native_test_record`, `:panels_test_record`, `:shell_test_record`; then `bazel test //apps/humata2/desktop:native_test //apps/humata2/desktop:panels_test //apps/humata2/desktop:shell_test //apps/humata2/desktop:report_test`.
- **Close check (minion BUG2-LOOK2, 2026-11-15, after g74):** every W1 item and seam is in the tree (no `SplitGroups`, no model picker, `Session.last_used` drawn in the Sessions table, ⌘, and the Settings… menu item, Edit and Window menus, Escape closes the font list, the toast's "Frees n when this closes."). g74: `features/{settings,board,inbox,palette}`, `ui:ui_unit_test`, `desktop:shell_test` pass. `desktop:native_test`, `:panels_test` and `:design_test` fail only on other lanes' intended view changes that need re-recording: MACH's tool versions on the Hosts row (28-hosts), REVIEW3's diff gutter as comment buttons and the placed comment (01, 30 and the changes captures behind them), the Preview/Source/Side by side RadioGroup, and DESK's frames (37, 40, 55, desk-document). No LOOK2 regression is among them.
- **GAPS_REVIEW note:** LOOK2 closes on 781bcf58 and 22df2b44 plus main's re-recording; no code change in this pass.
- **Ready to commit:** yes, once main re-records native, panels and design and the diffs are the ones listed above

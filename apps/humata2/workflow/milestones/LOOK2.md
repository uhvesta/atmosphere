# LOOK2: Every other screen looks like the goldens

- **Status:** blocked
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
- **Claims (from "LOOK1 committed"):** `ui/src/{panels,session_tabs,sidebar,terminals,theme,chrome,widgets}.rs`, new `ui/src/panel_*.rs`, `desktop/tests/native.rs`, new `desktop/tests/native/`, `desktop/tests/{panels,shell}.rs`, `desktop/tests/__snapshots__/{native,panels,shell}/` (main records), `clients/src/fixture/{design_session,sessions,requests,terminal,machines,items,journal,files}.rs`, `clients/src/fixture.rs`, `features/settings/`, `features/sidebar/`. Seams go to main as exact lines: `ui/src/{lib,shell,overlays}.rs`, `features/app/src/lib.rs`, `clients/src/model.rs`, `desktop/BUILD.bazel`, `desktop/src/main.rs` (FL, BUG). Not: `ui/src/{request_card,access}.rs` (ASK), `ui/src/overlay_toasts.rs` (BUG), `features/hosts/` (MACH), `features/review` (REVIEW), goldens, `desktop/tests/design*`.
- **Tasks:**
  - Batch 1 (files no lane claims; view-only, test ids kept):
    - [ ] T1 channel page, frames 18, 19, 20, 35 · worker ch (Fable) · `ui/src/channel.rs` · running
    - [ ] T2 boards, frames 21–24 · worker bd (Fable) · `ui/src/board.rs` · running
    - [ ] T3 Home, Inbox list and split, empty Home, frames 25, 26, 27, 34 · worker hm (Fable) · `ui/src/lists.rs`, `ui/src/pages.rs` (`first_run` and the Home arm only) · running
    - [ ] T4 Hosts, frame 28 · worker ho (Sonnet, medium) · `ui/src/hosts.rs` · running
    - [ ] T5 Settings, frame 29 (Panes block stays until T9) · worker st (Sonnet, medium) · `ui/src/settings.rs` · running
    - [ ] T6 palette and New session, frames 30, 31; read-only audit of 32 · worker ov (Sonnet, medium) · `ui/src/overlay_palette.rs`, `ui/src/overlay_new_session.rs` · running
  - Batch 2 (after "LOOK1 committed"):
    - [ ] T7 split `ui/src/panels.rs` (mechanical, no behaviour change)
    - [ ] T8 split `desktop/tests/native.rs` (mechanical; BUILD seam above)
    - [ ] T9 D111: remove the Fixed / Simple switch (`features/settings`, `ui/src/settings.rs`, the native layout flow)
    - [ ] T10 SE-26 Last used: model seam, fixtures, Sessions column and sort
    - [ ] T11 fixture data to frames 18–35 (from the batch 1 reports)
    - [ ] T12 empty states: sidebar on an empty workspace, native captures 34 and 35, retire native 33 (D113.5)
    - [ ] T13 D113.7 toast wording (lines for BUG), D113.8 and .9 checked against the app
    - [ ] T14 shared chrome on these pages (page header buttons, filter height) and the polish list above
    - [ ] T15 platform: standard shortcuts, ⌘, opens Settings, native menu bar (`desktop/src/main.rs` lines via main); one flow per frame
    - [ ] T16 re-audit 18–35 on main's fresh captures until each matches; main cuts and installs the fakes release
- **Waiting on:** "LOOK1 committed" from main (batch 2); batch 1 gate.
- **Build gates:** batch 1: `bazel build //apps/humata2/ui //apps/humata2/desktop:humata2`; `bazel test //apps/humata2/ui:ui_unit_test`; `bazel run //apps/humata2/desktop:native_test_record` then `bazel test //apps/humata2/desktop:native_test //apps/humata2/desktop:panels_test //apps/humata2/desktop:shell_test //apps/humata2/desktop:report_test`. No golden changes. Captures expected to change: native 18–33.
- **Ready to commit:** no

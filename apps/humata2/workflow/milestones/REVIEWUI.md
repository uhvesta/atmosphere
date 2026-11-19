# REVIEWUI

**Outcome:** Reviewing a pull request feels like reading the Diff panel: Submit review sits where the drafts are counted, full screen and back keep my exact place, every control says what it does on hover, and the diff can be driven from the keyboard.

**Why:** Owner, 2026-11-18:

- "Make sure the full screen button for the diff view always appears and other things in that line truncate instead when there is too much."
- A back button out of review, on every session page.
- Show the terminal from full screen keeps the file, line, scroll and drafts, and the terminal keeps running.
- Word wrap on vs off, from the file I'm reading.
- Big diffs (big files, many files, long names) stay smooth.
- The prepared PR view works (12:40: "in scope").
- Stacks work across multiple repos.
- Plain language, and no status noise I can't act on.

Most of this is already planned in [DIFFVIEW.md](DIFFVIEW.md) (DV-5, DV-10 to DV-19) and [STACKVIEW.md](STACKVIEW.md) (T7, T8). REVIEWUI holds what neither owns: the PR review's own gaps, per-session memory, tooltips, keys, the file filter and the budgets. It also lists which DS rules each DV and T task must meet, so nothing is decided twice.

**Exit:** Every DS rule in [DIFFVIEW-spec.md](DIFFVIEW-spec.md) is Built, or is named below as owned by an open DV or T task. Each case in [flows/cases/diff/](../../flows/cases/diff/README.md) has no Not built or Likely bug row left that this milestone owns. Every new or changed frame has Fable's pick (D76). The P-1 to P-9 budgets are measured on the 4-repo rig, and any miss has a follow-up.

## Read these (pointers, nothing else)

- [DIFFVIEW-spec.md](DIFFVIEW-spec.md): the rules (DS-1 to DS-84), keys (§3), budgets (§5) and how source contradictions were settled (§8).
- [flows/cases/diff/](../../flows/cases/diff/README.md): 16 cases, each step tagged from the code.
- [DIFFVIEW-design.md](DIFFVIEW-design.md) and the mockups `mockups/review.png` (§1–§5) and `mockups/diff-proposal.png`.
- Code: `ui/src/panel_changes.rs` (the panel; DV-5 is changing it now), `ui/src/panels.rs`, `ui/src/pages.rs`, `ui/src/comment_cards.rs`, `ui/src/document_tab.rs`, `ui/src/keys.rs`, `features/app/src/lib.rs`, `features/review/src/lib.rs`.
- Tests: `desktop/tests/native/{pr_session,review,diff_view,diff_big}.rs`, `desktop/tests/live_flows/pr_review.rs`.

## Tasks

Each task lists its DS rules and the case that checks them. Tests follow the AGENTS.md rule: behaviour, not wording.

### RV-1. Submit review sits in the footer, and drafts look like drafts

DS-55, DS-54, DS-11; spec §8 items 4 and 8. Case [11](../../flows/cases/diff/11-pr-review-session.md).

- The review footer reads "4 drafts · 3 by Copilot, 1 by you" (the agent's name, not "agent") and holds the one blue **Submit review…**, which opens the existing sheet.
- While the Diff panel shows, the session bar's Submit review (`pages.rs:316-330`) is hidden. With the panel closed it stays.
- An agent's draft card has the blue outline (`comment_cards.rs:86` today outlines only the open box).
- Tests (`pr_session.rs`): `submit_review_in_the_footer_opens_the_sheet`, `the_session_bar_hides_submit_review_while_the_diff_shows`, `an_agent_draft_is_outlined`. Re-record frame 68 (Fable).
- The full-screen footer gets the same button in DV-18; this task leaves the footer in a shape DV-18 can reuse.

### RV-2. A PR review scrolls like the Diff panel

DS-26, DS-42. Cases [07](../../flows/cases/diff/07-a-big-file-stays-smooth.md), [10](../../flows/cases/diff/10-wrap-or-scroll.md). After DV-5 (its step 5 gives files over 1,500 lines their own box, DS-29).

- In a review, the open file's head stays pinned while its lines scroll (today the head scrolls with the list, `review_repo`, `panel_changes.rs:889`).
- Scroll is honoured in a review: the gutter stays put and the code slides (today `self.wrap || review` forces Wrap, `:2386`).
- Tests: `a_review_file_head_stays_while_its_lines_scroll`, `scroll_in_a_review_keeps_the_gutter_still`.

### RV-3. Each session keeps its place, and the panel doesn't reload

DS-69, DS-73, DS-76, DS-83. Cases [04](../../flows/cases/diff/04-full-screen-and-back.md), [13](../../flows/cases/diff/13-the-diff-remembers-each-session.md).

- Key `Documents::diff_view` by session as well as repo and path (`document_tab.rs:553`, called at `panel_changes.rs:475`). This is the Likely bug: two sessions on one file share a scroll.
- Opening, closing, widening or going full screen on the panel sends no diff fetch. Only a base pick, Try again, a refresh event or new commits do (report finding 6).
- After Show the terminal, focus returns to where it was before full screen.
- Tests: `two_sessions_on_one_file_keep_their_own_scroll`, `show_the_terminal_keeps_the_line_scroll_drafts_and_open_box` (this extends `show_the_terminal_returns_the_review_beside_the_terminal`, which checks only the file), `toggling_the_panel_sends_no_fetch` (a reducer test in `features/app`), `show_the_terminal_returns_focus_to_the_terminal`.

### RV-4. Every icon and every shortened name says what it is on hover

DS-79, DS-80, and the tooltip half of DS-17. Case [05](../../flows/cases/diff/05-full-screen-never-disappears.md).

- `icon_action` (`panels.rs:92`) takes a tooltip with the key: "Full screen ⌥⌘↩", "Show the terminal ⌥⌘↩", "Close panel", "Open in editor".
- Each ellipsized text (path, repo, branch, base chip, PR line, title-bar names) shows its full text on hover. Answer DV-10's question first: one shared helper in `gpui_kit`, not one per call site.
- Tests: `the_full_screen_button_names_its_key`, `a_shortened_path_shows_in_full_on_hover`.

### RV-5. The keys are bound where they can't clash, and all are listed

Spec §3; DS-40 (⌥Z), DS-13 (Esc); spec §8 items 1 and 2. Case [14](../../flows/cases/diff/14-keys-for-the-diff.md).

- ⌥Z flips Wrap only in the diff's own key context (code or file list focused), never in a text box or the terminal.
- Esc leaves full screen only after every menu, sheet and comment box has closed (`overlay.rs:56`).
- The ⌘/ sheet (`keys.rs:84`) lists every diff key: ⌥⌘↩, ⇧⌘↩ (bound today but missing), ⌥⌘↓/↑, ⇧⌥⌘↓/↑, ⌥Z, ⌘[, ⌥⌘] / ⌥⌘[.
- Layers use ⌥⌘] / ⌥⌘[ so files keep ⌥⌘↓/↑. Hand this to STACKVIEW T8 before it binds keys. Fable confirms.
- Binding ⌥⌘↩ is DV-18's work and the arrows are DV-15's; this task adds ⌥Z, Esc and the sheet, and checks the clashes once both have landed.
- Tests: `option_z_in_the_comment_box_types_and_does_not_flip_wrap`, `escape_closes_the_menu_before_full_screen`, `every_diff_key_is_in_the_shortcuts_sheet`.

### RV-6. A long file list can be filtered

DS-9. Case [08](../../flows/cases/diff/08-many-files-and-a-big-pull-request.md).

- Above 20 changed files, a "Filter files" field sits above the list. It matches the name and folder and keeps repo rows for repos with matches. Esc clears it.
- Tests: `a_filter_shows_above_twenty_files`, `filtering_keeps_only_matching_files_and_their_repos`. New frame (Fable).

### RV-7. The base menu never clips

DS-50. Case [05](../../flows/cases/diff/05-full-screen-never-disappears.md). This is the follow-up the DV list deferred to its DV-14 recheck. Do it here, whatever that recheck finds.

- `base_menu` and `repo_base_menu` (`panel_changes.rs:1283`) anchor by their right edge to the chip.
- Test: `the_base_menu_stays_inside_a_narrow_panel`.

### RV-8. The budgets are measured

Spec §5, P-1 to P-9. Cases [07](../../flows/cases/diff/07-a-big-file-stays-smooth.md), [08](../../flows/cases/diff/08-many-files-and-a-big-pull-request.md), [09](../../flows/cases/diff/09-empty-loading-and-errors.md). After DV-5. Runs with DV-14's live rerun, not as a second rig.

- Record time to first rows, reselect time, scroll CPU and RSS for big.txt, the 586-file load, a base switch, and the 194k-char wrap. Main runs the bench through Bazel. A number marked "proposed" becomes the budget only once Fable or main accepts it.
- Each miss opens a follow-up with its number, not a new design.

## What DIFFVIEW already owns (must meet these DS rules)

| DV task | DS rules it must meet | Cases |
|---|---|---|
| DV-5 virtual rows (in progress) | DS-25, DS-29, DS-42, DS-43, P-1–P-4, P-9 | 07, 08, 10 |
| DV-10 file and repo rows | DS-5 (branch), DS-7, DS-46, DS-47, DS-48, DS-49, DS-74 (UI) | 02, 03, 05 |
| DV-11, DV-12 notes and states | DS-8, DS-23, DS-24, DS-28, DS-33, DS-34, DS-36, DS-82 | 03, 06, 07, 09 |
| DV-13 stress frames | DS-51 | 10 |
| DV-14 live rerun | P-5, P-7 (with RV-8) | 08, 09 |
| DV-15 file bar and stepping | DS-2, DS-3, DS-27, DS-53, DS-78, DS-84; the ⌥⌘↓/↑ and ⇧⌥⌘↓/↑ keys | 01, 07, 11, 14 |
| DV-16 repo row and footer | DS-5, DS-6, DS-10, DS-11; Add repo… in the footer (spec §8 item 3) | 02 |
| DV-17 hunk look and Wrap switch | DS-15 (void), DS-19, DS-20, DS-21, DS-22, DS-40 | 06, 10 |
| DV-18 full screen | DS-12, DS-13 (button and key), DS-14, DS-16, DS-17 (name), DS-44, DS-58 | 04, 12 |
| DV-19 empty and error states | DS-32, DS-35 | 09 |

## What STACKVIEW owns (must meet these DS rules)

| T task | DS rules | Cases |
|---|---|---|
| T4, T6 (service, repo filter) | DS-63, DS-64, DS-65 | 15, 16 |
| T7 (reducer: PickLayer, NextLayer, PrevLayer) | DS-62; layer keys ⌥⌘] / ⌥⌘[ (spec §8 item 1) | 14, 15 |
| T8 (`panel_stack.rs`, frames 119–121) | DS-60, DS-61, DS-62, DS-63, DS-65, DS-66, DS-67, DS-68 | 15, 16 |
| T3c (importance) | DS-67 | 15 |

## Out of scope

- The browser and Linux clients (cut, 2026-11-16).
- A side-by-side (two-pane) diff. The design keeps one column with two number columns in full screen.
- Posting anything to GitHub before Submit review. Drafts stay local until then (PR.md).
- Reading a file's lines only when it is picked. It is a follow-up only if DV-14 or RV-8 shows 586 files still take over 5 s.

## Lane

- 2026-11-18: spec, 16 cases and this milestone written by the coach. Not started. Depends on DIFFVIEW (DV-5 for RV-2 and RV-8; DV-16 and DV-18 for RV-1's footer). Runs in parallel with STACKVIEW, sharing only the layer-key decision (RV-5 → T8).

### Plan (minion REVIEWUI, 2026-11-18 18:10)

Verified against the tree (uncommitted DIFFVIEW batch included):
- Already done by DIFFVIEW, only checks left: Submit review… in the footer (`diff-submit-review`, W-D), icon tooltips with keys ("Full screen ⌃⌘↩", "Close panel", "Open in editor"; DV-18), Esc leaves full screen after the comment box, base menu, repo picker and push menu (`overlay.rs` dismiss order), layer keys ⌥⌘] / ⌥⌘[ in `DIFF_STACK` (STACKVIEW), file/change keys and their sheet rows (DV-15). The full-screen chord is ⌃⌘↩, not ⌥⌘↩ (DIFFVIEW 17:05).
- Left for RV: footer count wording with the agent's name (DS-55), session-bar Submit review hidden while the Diff shows, agent draft outline (DS-54), per-session scroll key (DS-73), no refetch on panel toggle (DS-76: `ReviewAction::Open` always fetches today, `features/review/src/lib.rs:1127`), focus back after Show the terminal (DS-83), full text on hover for shortened names (DS-80), review file head pinned and Scroll honoured (RV-2), file filter (RV-6), base menu right anchor (RV-7), ⌥Z context (after DV-17), budgets (RV-8).
- `ui/src/panel_changes.rs`, `features/review/src/lib.rs`, `ui/src/pages.rs` belong to other lanes: RV writes seam hunks (scratchpad `reviewui/*.py --check`) for main, and new code in new files.

**Batch 1 (now; nothing in panel_changes.rs)**

| Task | Worker | Files (edit) | Seams for main | Tests |
|---|---|---|---|---|
| B1 DS-76 no refetch | Haiku high | new `features/review/tests/open_again.rs` | `features/review/src/lib.rs` Open arm (seam_open.py) | `opening_again_with_the_diff_held_reads_nothing`, `try_again_and_a_base_pick_still_read` |
| B2 DS-73 + DS-69 place per session | Haiku high | `ui/src/document_tab.rs` (additive `session_diff_view`), new `desktop/tests/native/review_ui.rs` | panel_changes.rs 3 `diff_view` call sites (seam_scroll_key.py); `mod review_ui;` in native.rs | `two_sessions_on_one_file_keep_their_own_scroll`, `show_the_terminal_keeps_the_line_scroll_drafts_and_open_box`, `toggling_the_panel_sends_no_fetch` |
| B3 RV-1 drafts | Haiku high | `ui/src/comment_cards.rs`, `desktop/tests/native/pr_session.rs` | panel_changes.rs footer label; pages.rs Submit review hide (seam_submit.py) | `submit_review_in_the_footer_opens_the_sheet`, `the_session_bar_hides_submit_review_while_the_diff_shows`; outline via frame 68 (Fable) |
| B4 DS-80 full text on hover | Sonnet medium | new `ui/src/full_text.rs` | `mod full_text;` in ui/src/lib.rs | unit/native test that a shortened text carries its full text |

**Batch 2 (once DIFFVIEW hands over panel_changes.rs after DV-17):** RV-2, RV-6 (filter state in a new `features/review/src/file_filter.rs`), RV-7, DS-80 call sites, ⌥Z context check, DS-83, then RV-8 with DV-14.

### Workers

| Worker | Task | Status |
|---|---|---|
| manager | B1 DS-76 reopen: `features/review/tests/open_again.rs`, `src/reopen.rs`, `seam_open.py` | done; seam waits for DIFFVIEW |
| Haiku a7527158 | B2 DS-73 per-session place: `session_diff_view`, `desktop/tests/native/review_ui.rs`; call sites in `seam_panel_changes.py` | done; 2 tests red until seam applied |
| Haiku a17f280d | B3 DS-54 outline + DS-11 footer Submit review tests (`comment_cards.rs`, `pr_session.rs`) | done; frames 68/68b need re-record |
| Haiku ac840a35 | B4 DS-80 `ui/src/full_text.rs` helper | done; call sites in batch 2 |
| Sonnet a48c5124 | 2c RV-6 file filter (`features/review/src/file_filter.rs`, file list in panel_changes.rs) | running |
| Sonnet a98e1d7d | 2b RV-2 review head pinned + Scroll (`review_repo*`, code column in panel_changes.rs) | running |
| manager | 2a DS-15 cards start at the code column (640 cap stays, spec §8.6); 2d RV-7; 2f ⌥Z; 2g DS-83 tests in review_ui.rs | tests written, red run queued |

### Batch 2 plan (minion REVIEWUI, 2026-11-18 19:40; panel_changes.rs + features/review/src/lib.rs handed over by main once the seams are in)

Order, each TDD with its failing test written first (manager), code by Haiku workers, one file owner at a time on panel_changes.rs:

| # | Task | Where (today's lines) | Tests |
|---|---|---|---|
| 2a | DS-15 cards start at the code column (DIFFVIEW leftover; spec §8 item 6 settles the 640 cap as inside 720, so it stays) | `fit` in panel_changes.rs | `review_ui::the_comment_box_starts_at_the_code_column` |
| 2b | RV-2 review file head pinned, Scroll honoured | `review_repo` :1415 / `review_repo_head` :1570, `self.wrap \|\| review` :3146 | `pr_session::a_review_file_head_stays_while_its_lines_scroll`, `scroll_in_a_review_keeps_the_gutter_still` |
| 2c | RV-6 file filter above 20 files | state in new `features/review/src/file_filter.rs`; field above the list | `a_filter_shows_above_twenty_files`, `filtering_keeps_only_matching_files_and_their_repos`; new frame (Fable) |
| 2d | RV-7 base menus anchor right | `base_menu` :1880, `repo_base_menu` :2265 | `the_base_menu_stays_inside_a_narrow_panel` |
| 2e | DS-80 call sites via `full_text` | the `text_ellipsis()` sites (:256, 265, 591, 605, 1003, 1028, 1103, 1369, 1605, 1835, 2086, 2097, 2203, 3004) | `a_shortened_path_shows_in_full_on_hover` (label-based) |
| 2f | ⌥Z context check | `keys.rs:276` binds in DIFF; check text box and terminal | `option_z_in_the_comment_box_types_and_does_not_flip_wrap` |
| 2g | DS-83 focus back after Show the terminal | features/app overlay/full-screen + pages focus | `show_the_terminal_returns_focus_to_the_terminal` |
| 2h | RV-8 budgets with DV-14 (main tracks DV-14) | bench rig via Bazel, numbers "proposed" until accepted | P-1..P-9 recorded |

- 2026-11-18 21:00: after 0.3.4, 2e (DS-80) re-applied and its test restored against `repo-branch-0`'s label; 2h (RV-8) written as `review_ui::budgets::` ignored measurement tests, run by main with `-c opt --test_arg=--ignored --test_arg=--nocapture --test_arg=review_ui::budgets:: --test_arg=--test-threads=1`; numbers stay "proposed" until Fable or main accepts them. P-5/P-7 (daemon) and P-8 come from DV-14's live rerun.

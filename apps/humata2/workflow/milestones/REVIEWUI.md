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

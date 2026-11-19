# DIFFVIEW spec: how the diff and review UI looks and behaves

Written 2026-11-18 from the sources below. This is the one place the diff and
review UI is written down as rules. Each rule has a stable ID (DS-n) that
cases (`flows/cases/diff/`) and milestones (DIFFVIEW, REVIEWUI, STACKVIEW)
cite.

**Sources, highest first:** the owner's words (2026-11-18, quoted in
[REVIEWUI.md](REVIEWUI.md)); [DIFFVIEW-design.md](DIFFVIEW-design.md)
(Fable's approved design and the owner additions at its end);
[DIFFVIEW.md](DIFFVIEW.md) (the DV fix list); [DIFFVIEW-report.md](DIFFVIEW-report.md)
(the live test); [STACKVIEW.md](STACKVIEW.md) (stacks UI); [PR.md](PR.md);
and the mockups in [mockups/](mockups/): `diff-proposal` (§1 beside the
terminal, §2 full screen, §3 big file, §4 states), `review` (§1 PR review
session, §2 Submit review, §3 Wrap and Scroll, §4 Full screen never
disappears, §5 Show the terminal) and `stacks` (§1 Whole stack, §2 one
layer, §3 narrow with drift and the Inbox offer).

**Status of each rule** was set by reading the code on 2026-11-18, not the
design:

- **Built**: the code does it. A test name follows where one exists.
- **Partly built**: some of it is there; the rule says which part is missing.
- **Not built**: nothing does it yet. The owning task is named.
- **In progress**: another agent is changing it now (DV-5 virtual rows in
  `ui/src/panel_changes.rs` and `ui/src/document_tab.rs`). Do not count it
  as done.

Paths are under `apps/humata2/`. Line numbers are from 2026-11-18 and drift.

## 1. Purpose

I can read any change in a session, across one or many repos, at any size,
and act on it: comment, push, or review a pull request with an agent beside
me. The code is the page; the file list is a navigator. One row of chrome per
thing, one blue action per screen. Problems show only where they block an
action. Full screen is always one click away, and leaving it puts me back
exactly where I was, with the terminal still running.

## 2. Surfaces

### 2.1 Beside the terminal (narrow)

Mockup: `diff-proposal` §1 and §4. Frames: 84 `84-diff-per-repo-beside-terminal`,
41 `41-review-beside-terminal`, native `changes-pushed`.

| ID | Rule | Status | Where |
|---|---|---|---|
| DS-1 | The panel header is one 36 pt row: "Diff", the base chip, then ⤢ Full screen and ✕ Close at the right. | Built: `diff_view.rs::a_long_base_keeps_expand_and_close_inside_the_panel`. | `ui/core/src/chrome.rs`: `panel_header` |
| DS-2 | With a file open, the list folds into a one-line **file bar**: ⌄, repo · file name · dim folder, "2 of 4", ‹ ›, and ↗ (Open in editor, an icon button). The code gets the rest of the height. | Built (DV-15): `diff_view.rs::with_a_file_open_the_list_folds_into_the_file_bar`, `::the_file_bar_controls_stay_inside_a_narrow_panel`. Full screen and the inline PR review keep a list column instead (the bar there is the step bar, DS-53). The ↗ is drawn but does nothing (DS-24). | `ui/changes/src/panel_changes/file_list.rs`: `file_bar`, `open_in_editor` |
| DS-3 | Clicking the file bar or ⌄ opens the list in place, in a box of at most 220 pt that scrolls. Picking a file folds it again. | Built (DV-1, DV-15): `diff_view.rs::clicking_the_file_bar_opens_the_list_and_picking_a_file_folds_it`; the 220 pt scrolling box by `::a_long_file_list_leaves_room_for_the_code`. | `ui/changes/src/panel_changes/mod.rs`: `changes` (`changes-files`); `features/review/src/lib.rs`: `file_list_open` |
| DS-4 | With no file open, the list fills the panel. | Partly built: with no file picked yet the narrow list shows in its 220 pt box and the first file's code fills the rest (`selected` falls back to the first file), so the list never fills the panel. Once a file is picked the list folds (DS-2). No test for the unpicked state. | `ui/changes/src/panel_changes/mod.rs`: `changes` (`bar`, `file_open`) |
| DS-5 | Each repo is one row: a chevron that collapses it, the name without a leading "/", and its totals at the right. Its branch shows only when it differs from the session's branch. Its base shows ("vs release/2.4 ⌄") only when it differs from the header's. | Partly built: one row per repo with a working chevron (`repo-toggle-{index}` sends `ToggleRepo`), name without a leading "/", branch only when the clone's differs, totals pinned. Tests: `diff_view.rs::a_collapsed_repo_hides_its_files_and_opens_again`, `::each_repo_shows_its_own_clones_branch`, `::a_long_repo_and_branch_stay_inside_the_repo_row`, `::repo_totals_stay_inside_their_row_at_any_width`. Base: settled by D848, every row says "against X" in words, even when it equals the header's. Missing: the inline PR review's repo row still draws a chevron with no click (Likely bug; no open task). | `ui/changes/src/panel_changes/review_header.rs`: `repo_head`, `repo_base`; `review_repo_head` (the dead chevron) |
| DS-6 | The "N files · M repos +a −r" summary row goes. **Add repo…** stays reachable (see §8, item 3). | Built, no test: the summary row is gone and **Add repo…** sits in the footer beside Push & PR… (spec §8 item 3). No test asserts the row is absent; frame 98 holds the footer. | `ui/changes/src/panel_changes/mod.rs`: `changes` footer (`changes-add-repo`) |
| DS-7 | A file row shows the file name first, then the folder dimmed and ellipsized, then the comment count and +/−. Its tooltip and accessibility label carry the full path. | Built: name first, folder dimmed and ellipsized, comment count and +/− (`diff_view.rs::two_long_paths_that_differ_in_the_file_name_both_show_it`); the row goes through `full_text`, so it carries `repo/path` as its label and hover (`diff_view.rs::a_rename_names_where_it_came_from`). | `ui/changes/src/panel_changes/file_list.rs`: `file_row`, `split_path` |
| DS-8 | A 14 pt status column replaces the file icon: A (green), D (red), R (secondary); modified files show nothing. A binary file shows "bin" where +/− would be; a mode-only change shows "mode". | Built (DV-12): `diff_view.rs::a_binary_file_says_binary_instead_of_an_empty_diff`, `::a_rename_names_where_it_came_from` (asserts `change-file-mark`), `::a_mode_only_change_shows_both_modes`. | `ui/changes/src/panel_changes/file_list.rs`: `file_row` (`mark`, `stat_word`) |
| DS-9 | A "Filter files" field appears above the list once a session has more than 20 changed files. | Partly built (RV-6): the Diff panel shows the field above 20 files: `review_ui.rs::a_filter_shows_above_twenty_files`; `features/review/tests/file_filter.rs::filtering_keeps_only_matching_files_and_their_repos`, `::the_field_needs_more_than_twenty_files`. Missing: the inline PR review leaves it out (`panel_changes/mod.rs::changes`, `!inline`), and that is where a 300-file pull request lists its files. No open task. | `features/review/src/file_filter.rs`; `ui/changes/src/panel_changes/file_list.rs`: `file_filter_field` |
| DS-10 | The footer shows only when it has an action. **Push & PR…** is one quiet text button at its left (it opens the push plan for every repo). It shows when the plan reports unpushed commits or uncommitted files, and also while no plan is known; never hidden on a guess. **Send N comments** is the one blue button at its right. | Settled by D850: the footer always shows in a session; **Send to <agent>** is disabled when nothing waits. **Push & PR…** is one quiet button that opens `changes-push-menu`; it shows whenever the session lists a diff, because no push plan is ever held in state to hide it on (the rule's "no plan is known" case). Test: `diff_view.rs::push_and_pr_opens_one_menu_listing_every_repo`. Not built: hiding Push & PR… when a plan says nothing is unpushed; no open task. | `ui/changes/src/panel_changes/mod.rs`: `changes` footer; `ui/changes/src/panel_changes/review_header.rs`: `push_menu`, `push_control` |
| DS-11 | One blue action per screen. Counts, chips and the agent's name are not blue. | Built, with D850 and D852: Send (when something waits) or Submit review… (PR review) is the one blue button; Push & PR…, Add repo… and counts are quiet; Looks good is green (D852, Fable's pick). Test: `pr_session.rs::a_review_footer_offers_send_as_a_secondary_left_of_submit`. | `ui/changes/src/panel_changes/mod.rs`: `changes` footer |

### 2.2 Full screen

Mockup: `diff-proposal` §2, `review` §4 and §5. Frames: 85
`85-diff-per-repo-full-screen`, 11 `11-panel-full-screen-diff`, 68b
`68b-pr-review-full-screen`, 12 (the shared title bar, Board).

| ID | Rule | Status | Where |
|---|---|---|---|
| DS-12 | Three ways in: the ⤢ button, ⌥⌘↩, and a double-click on the panel header. | Built: the ⤢ button, ⌃⌘↩ (not ⌥⌘↩; DIFFVIEW lane 17:05, off a Mac ⌥⌘↩ clashed with ⇧⌘↩) and a double-click on the panel header. Tests: `diff_full_screen.rs::alt_cmd_enter_enters_and_leaves_full_screen` (presses ctrl-cmd-enter), `::double_clicking_the_panel_header_enters_full_screen`. | `ui/core/src/chrome.rs`: `panel_header`; `ui/core/src/keys.rs`: `ctrl-cmd-enter` |
| DS-13 | Ways back: **Show the terminal**, ⌥⌘↩ again, or Esc when no menu, sheet or comment box is open. | Built: **Exit full screen** (D848 renamed Show the terminal), ⌃⌘↩ again, and Esc after any menu or box. Tests: `diff_full_screen.rs::escape_leaves_full_screen_but_not_while_the_base_menu_is_open`, `::escape_closes_the_new_agent_tab_menu_first`. | `ui/core/src/chrome.rs`: `panel_header_actions`; `features/app/src/overlay.rs` |
| DS-14 | Full screen has **one** title bar (52 pt): the window lights, "# channel › session" (ellipsized), "Diff" with the base chip or the PR line beside it, then **Show the terminal** at the right. The panel's own 36 pt "Diff" header with its second ⤢ is not drawn. | Built (DV-18, D860): one 52 pt bar with the names ellipsizing before Exit full screen and Close; the panel's 36 pt header is not drawn. Test: `diff_full_screen.rs::full_screen_draws_one_header`. | `ui/core/src/chrome.rs`: `panel_header_actions` |
| DS-15 | The list stays 300 pt wide at the left; the code fills the rest in the content colour from top to bottom (no grey void). Comment cards and the comment box are at most 720 pt wide and start at the code column. | Built: 300 pt list, code in the content colour end to end (frames only, no test for the light-mode void), cards at most 720 pt and starting at the code column (`review_ui.rs::the_comment_box_starts_at_the_code_column`). | `ui/changes/src/panel_changes/mod.rs`: `changes`; `ui/changes/src/panel_changes/diff_body.rs`: `fit` |
| DS-16 | The full-screen footer is the narrow footer: the same actions, including **Submit review…** in a PR review. | Built (DV-18): the footer is one function for both widths and holds `diff-submit-review` in a PR review. Test: `diff_view.rs::a_pull_request_review_footer_has_submit_review` runs beside the terminal only; no test in full screen. | `ui/changes/src/panel_changes/mod.rs`: `changes` footer |
| DS-17 | The panel's arrow is named **Full screen** and has a tooltip "Full screen ⌥⌘↩". The session header's arrow (terminal full screen) has "⇧⌘↩". In full screen the panel's button reads Show the terminal. | Built: the panel's arrow is **Full screen** with the tooltip "Full screen  ⌃⌘↩"; in full screen it reads Exit full screen (D848) with "Exit full screen  ⌃⌘↩"; the session header's terminal arrow has "Expand terminal  ⇧⌘↩" / "Return to columns  ⇧⌘↩" (`diff_full_screen.rs::the_full_screen_button_names_its_key`, `the_terminal_arrow_names_its_key`). | `ui/core/src/chrome.rs`: `panel_header`, `panel_action`; `ui/src/pages.rs`: `terminal-full-screen` |
| DS-18 | Nothing switches to full screen by itself, however wide the content. | Built (nothing does), no test. |  |

### 2.3 How code reads

Mockup: `diff-proposal` §1–§2. Frames: native `changes-wrapped-long-line`
and `-dark`, 50 `50-diff-comment-box`.

| ID | Rule | Status | Where |
|---|---|---|---|
| DS-19 | The gutter has two number columns (old, new) in full screen and one beside the terminal (the new number, or the old one on a removed line). Numbers are secondary, right-aligned, and are the comment click target. | Built (DV-17): `diff_view.rs::full_screen_has_two_number_columns`; the click target by `review.rs::clicking_a_diff_line_opens_the_box_under_it_and_comment_types_it_at_once`. | `ui/changes/src/panel_changes/diff_body.rs`: `Metrics`, `CodeRows::line` |
| DS-20 | After the numbers: a one-character sign column, one space, then the code. Removed and added rows share one geometry. Row tints are the status green and red at about 12% (light) and 16% (dark), full bleed, the gutter tinted too, no borders between rows. | Built (DV-17): `diff_view.rs::added_and_removed_code_starts_at_one_edge`. Tints are the status green and red at 12% (light) and 16% (dark) over the code colour; no test asserts the values. | `ui/changes/src/panel_changes/diff_body.rs`: `tint`, `blend` |
| DS-21 | Word-level highlight only where a run of removed lines is replaced by as many added lines and under half the line changed; the changed span gets about 28%. Computed only for rows in view. | Built (DV-17): unit tests `diff_body.rs::one_changed_word_is_the_span_on_both_lines`, `::lines_mostly_different_have_no_changed_span`, `::a_run_of_removed_lines_pairs_with_as_many_added_lines`, `::runs_of_different_lengths_or_alone_have_no_partner` (target `ui/changes:changes_unit_test`). Only the rows built (the rows in view) are computed. | `ui/changes/src/panel_changes/diff_body.rs`: `paired`, `changed_span` |
| DS-22 | Hunks are separated by a 24 pt dim band showing only the section text ("impl Client {"), or a thin line when there is none. No `@@` numbers. | Built (DV-17): `diff_view.rs::a_hunk_header_draws_no_at_numbers`; unit `diff_body.rs::a_hunk_shows_only_its_section_text`. | `ui/changes/src/panel_changes/diff_body.rs`: `hunk_section` |
| DS-23 | "No newline at end of file" is a dim row with no number and no click target. | Built (DV-11, DV-12): `diff_view.rs::a_missing_final_newline_is_a_note_row_without_a_number`; the daemon half by `daemon/review_diff/tests/modes.rs`. | `clients/live_diff/src/diff.rs`: `LineKind::Note`; `ui/changes/src/panel_changes/diff_body.rs` |
| DS-24 | The file header reads `repo · folder/name` with the name in primary text and ↗ at the right (an icon button with a tooltip). Under it, one dim note line only when there is something to say: "Renamed from src/http.rs (92% alike)", "Binary file · 12 MB", "Mode 644 → 755", "New file", "Deleted". A file with no lines shows the note as its body. | Partly built: the header reads `repo · folder/name` and the dim note line is built in the Diff panel (tests in DS-8, DS-36). The inline PR review has no file header and draws no note (`review_header.rs::review_code`). No open task. The ↗ is an icon button with a tooltip but has no action: no editor client exists. No open task owns the editor client. | `ui/changes/src/panel_changes/mod.rs`: `changes` (file header); `ui/changes/src/panel_changes/file_list.rs`: `file_note`, `open_in_editor` |

### 2.4 Big files and many files

Mockup: `diff-proposal` §3. Owner's stress list: [DIFFVIEW.md](DIFFVIEW.md) "Stress cases".

| ID | Rule | Status | Where |
|---|---|---|---|
| DS-25 | Only the rows on screen are built, so a 40,000-line diff opens at once. No "load anyway" gate. | Built (DV-5): files over 1,000 lines use a virtual list. Tests: `diff_big.rs::a_huge_diff_draws_only_the_lines_in_view`, `::a_huge_diff_scrolls_to_its_last_line`. Measured P-1 71 ms. Does not cover a PR review (DS-29). | `ui/changes/src/panel_changes/mod.rs`: `changes`; `ui/changes/src/panel_changes/diff_body.rs`: `VIRTUAL_FROM` |
| DS-26 | The file header stays put while the code scrolls, in every mode. | Built (RV-2): `pr_session.rs::a_review_file_head_stays_while_its_lines_scroll`; the Diff panel's header sits outside the scroll (no test). | `ui/changes/src/panel_changes/review_header.rs`: `review_repo`, `pinned_shift` |
| DS-27 | When a file has more than one screen of changes, the file header gains "Change 3 of 212 ‹ ›" (⇧⌥⌘↓ / ⇧⌥⌘↑). It is how you move, not a statistic, so it never shows for a short file. | Built (DV-15): `diff_view.rs::a_long_file_shows_a_change_stepper`; `features/review/tests/stepping.rs::next_and_previous_change_stay_within_the_files_hunks`. It shows for a file with more than one hunk and more than 60 lines. The keys are ⌃⌘↓ / ⌃⌘↑ (not ⇧⌥⌘; DIFFVIEW lane 17:45). | `ui/changes/src/panel_changes/file_list.rs`: `change_stepper` |
| DS-28 | While a big file's lines arrive, its body shows a dim "Loading big.txt…". | Not built: no per-file loading line. A big file's pages are read inside one `session_diffs` call (`clients/live_diff/src/diff.rs` loops `get_file_diff` before returning), so the panel shows the whole-panel sweep and dimmed list (D807, DS-34), never "Loading big.txt…". No open task. | `clients/live_diff/src/diff.rs`; `ui/changes/src/changes_loading.rs` |
| DS-29 | In a PR review, a file over 1,500 lines is drawn in its own box of about 60% of the panel's height, with its own scroll; the agent's focus scrolls inside it. Shorter files keep frame 68's inline look. | Not built: `review_code` builds every line of every code block and has no own box or 60% height; the 1,500-line rule is not in the code. Test only draws the huge file inside the panel (`diff_stress.rs::a_pull_request_huge_file_stays_inside_the_panel`). No open task (DV-5 step 5 was never taken). | `ui/changes/src/panel_changes/review_header.rs`: `review_code` |
| DS-30 | A long file list scrolls in both widths, so the last repo of 586 files is reachable. | Built (DV-1): `diff_view.rs::a_long_file_list_leaves_room_for_the_code`, `::the_file_list_scrolls_in_the_expanded_panel`; `diff_stress.rs::many_files_scroll_to_the_last_repositorys_last_file`. | `ui/changes/src/panel_changes/mod.rs`: `changes` (`changes-files`) |
| DS-31 | A pull request with a 20,000-line change, 300 files, a 200-char path, a rename, a binary and a mode change lists every file and pages the big file to its end. `pr_demo` opens that PR in the app with no network. | Built (DV-9): `daemon/humatad/tests/pr_diff_stress.rs::a_huge_pull_request_lists_every_file_and_pages_its_big_file_to_the_end`, `testing/pr_demo/`. | `apps/humata2/testing/pr_demo/` |

### 2.5 States: loading, empty, error, partial

Mockup: `diff-proposal` §4. Frames: 98 `98-diff-no-default-branch`, 86
`86-diff-repo-base-picker`; new frames `empty` and `loading` (DV-13).

| ID | Rule | Status | Where |
|---|---|---|---|
| DS-32 | **Empty:** centred secondary text, "No changes against main", and one button, **Compare with…**, that opens the base menu. No icon art. | Built (DV-19): `diff_view.rs::an_empty_diff_offers_compare_with`. | `ui/changes/src/panel_changes/states.rs`: `nothing_against` |
| DS-33 | **First load:** a dim "Reading changes…". No spinner. | Built, no test: "Reading changes…" (`changes-reading`) shows before the first diff arrives, with no spinner. | `ui/changes/src/panel_changes/mod.rs`: `changes` |
| DS-34 | **Base switch:** the chip names the picked base at once; the previous files stay, dimmed, until the new diff arrives. | Built (DV-12): `diff_view.rs::picking_a_base_shows_loading_until_the_diff_arrives` (checks the loading line, not the chip's text). The chip names the picked base while loading and the old files stay dimmed. | `ui/changes/src/panel_changes/mod.rs`: `changes` (`base`, `loading`) |
| DS-35 | **One repo fails:** its message and **Try again** sit under its own row; the other repos still work. | Built (DV-19): `diff_keys.rs::a_failing_repo_has_its_own_row_and_try_again_brings_it_back`; `features/review/tests/repo_errors.rs`. | `ui/changes/src/panel_changes/states.rs`: `repo_error_row`; `clients/live_diff/src/diff.rs`: `Daemon::repo_diff` |
| DS-36 | **One file fails:** the reason is that file's body. | Partly built: `file_note` shows `FileDiff.error` as the file's body, but nothing ever sets it: `clients/live_diff/src/diff.rs` writes `error: None`, and a failing file read fails the whole repo read (then DS-35). No open task. | `ui/changes/src/panel_changes/file_list.rs`: `file_note`; `clients/live_diff/src/diff.rs` |
| DS-37 | **The whole diff fails:** a failed first read names the error with Try again; a failed refresh keeps the last diff under a line with Try again. | Built: `review.rs::a_failed_first_read_names_the_error_with_try_again`, `::a_failed_refresh_keeps_the_last_diff_with_try_again`. | `ui/changes/src/panel_changes/states.rs`: `read_failed_block`; `ui/changes/src/panel_changes/mod.rs`: `refresh_failed_row` |
| DS-38 | A file deleted while the diff is read is left out; the diff does not fail. | Built (DV-3): `daemon/review_diff/tests/vanished.rs`. | `daemon/review_diff/src/checkout.rs` |
| DS-39 | A file's lines come from the snapshot the list was made from, so a file that changes back still shows the change I was shown. | Built (DV-4): `daemon/humatad/tests/review_file_trees.rs`. | `daemon/humatad/src/wire_review.rs` |

### 2.6 Long lines: Wrap and Scroll

Mockup: `review` §3. Frames: native `changes-wrapped-long-line` and `-dark`.

| ID | Rule | Status | Where |
|---|---|---|---|
| DS-40 | The file header has a **Wrap / Scroll** switch (and ⌥Z) that flips the same preference as Settings › Long lines. | Built (DV-17): `diff_view.rs::the_wrap_switch_flips_long_lines` (the switch is `diff-wrap` and flips the Settings preference); ⌥Z in the diff only: `review_ui.rs::option_z_in_the_comment_box_types_and_does_not_flip_wrap`. | `ui/changes/src/panel_changes/file_list.rs`: `wrap_switch`; `ui/core/src/keys.rs`: `alt-z` |
| DS-41 | **Wrap:** a long line breaks under the code; continuation rows have no number. | Built: `review.rs::wrapped_changes_align_the_gutter_and_switch_back_to_one_scrolling_row`. |  |
| DS-42 | **Scroll:** the numbers and +/− stay put; only the code column slides sideways, as one block per file, as far as its longest line. | Built (DV-17, RV-2): `pr_session.rs::scroll_in_a_review_keeps_the_gutter_still`; the Diff panel shifts only the code column (no test for that panel). | `ui/changes/src/panel_changes/mod.rs`: `changes`; `ui/changes/src/panel_changes/review_header.rs`: `review_code` |
| DS-43 | A 194,000-character line wraps without freezing the app. | Built: `diff_big.rs::a_huge_line_wraps_in_a_long_file`; measured 85 ms against the 500 ms budget (P-6, RV-8, an ignored measurement test `review_ui.rs::budgets::wrapping_a_194_000_character_line`). | `ui/core/src/code_wrap.rs` |
| DS-44 | Widening, narrowing, or going full screen and back keeps the top line in place under Wrap. | Partly built: a width change keeps the top line (`review.rs::widening_a_wrapped_diff_keeps_its_top_line_in_place`) and full screen and back keeps the scroll line (`review_ui.rs::show_the_terminal_keeps_the_line_scroll_drafts_and_open_box`), but that test does not turn Wrap on. No test of full screen and back under Wrap. Owner: none named; REVIEWUI follow-up. | `ui/changes/src/document_tab.rs`: `session_diff_view` |

### 2.7 Long names and truncation

Mockup: `review` §4 ("Full screen never disappears"). Frames: 68, 68b, 68c.

**Owner (2026-11-18):** "Make sure the full screen button for the diff view
always appears and other things in that line truncate instead when there is
too much."

| ID | Rule | Status | Where |
|---|---|---|---|
| DS-45 | **Controls that never shrink**, in every header and bar of the diff: in the panel header, ⤢ Full screen and ✕ Close; in the full-screen title bar, Show the terminal and "Change N of M ‹ ›"; in the file bar and file header, the "2 of 4" counter, ‹ ›, the Wrap switch and ↗; in the footer, every button (Push & PR…, Send N comments, Submit review…); on a repo row, the chevron and totals; on a file row, the status mark, the comment count and +/−. | Built: `diff_view.rs::a_long_base_keeps_expand_and_close_inside_the_panel`, `::the_file_bar_controls_stay_inside_a_narrow_panel`, `::repo_totals_stay_inside_their_row_at_any_width`; `pr_session.rs::a_narrow_review_panel_keeps_full_screen_and_close`. No test for the file row's status mark and counts or the full-screen title bar at a narrow width. | `ui/core/src/chrome.rs`; `ui/changes/src/panel_changes/` |
| DS-46 | **What gives way, first to last:** the base chip's text, then the PR line, then branch, then repo name, then the folder, each with an ellipsis. The full text is in the tooltip and the accessibility label. A PR line can shrink down to just "PR #482". | Partly built: the base chip, PR line, repo, branch, file bar names, file row path, file note and the full-screen title line ellipsize and carry `full_text` hover (`diff_view.rs::a_rename_names_where_it_came_from`, `diff_full_screen.rs::the_full_screen_title_line_keeps_its_whole_text`). Missing: the staged "PR #482" shrink; the PR line ellipsizes from the right. Owner: none. | `ui/core/src/full_text.rs`; `ui/changes/src/panel_changes/file_list.rs`: `file_row`; `ui/core/src/chrome.rs` |
| DS-47 | A long path keeps its file name: the folder ellipsizes before the name does, so two long paths that differ only at the end both read differently. | Built (DV-10): `diff_view.rs::two_long_paths_that_differ_in_the_file_name_both_show_it`. | `ui/changes/src/panel_changes/file_list.rs`: `file_row`, `split_path` |
| DS-48 | A local repo reads "large", not "/large". | Built (DV-10), display only: `diff_view.rs::a_local_repo_reads_without_a_leading_slash`. `repo_text` still writes "/name" on the wire side by design; `repo_label` strips it where drawn. | `ui/changes/src/panel_changes/file_list.rs`: `repo_label`; `clients/live_core/src/map.rs`: `repo_text` |
| DS-49 | Each repo row shows its own clone's branch when it differs from the session's. | Built (DV-10): `diff_view.rs::each_repo_shows_its_own_clones_branch`. | `ui/changes/src/panel_changes/review_header.rs`: `repo_head` |
| DS-50 | The base menu opens anchored by its right edge and never clips at the panel edge. | Built (RV-7) for a repo's menu: `review_ui.rs::the_base_menu_stays_inside_a_narrow_panel`. The header's menu is anchored the same way (`.right(0)`) but no test opens it narrow. | `ui/changes/src/panel_changes/review_header.rs`: `base_menu`, `repo_base_menu` |
| DS-51 | Unicode and emoji names, CRLF files and minified lines draw correctly. | Built (DV-13), bounds and frames only: `diff_stress.rs::unicode_paths_stay_inside_the_panel`, `::crlf_lines_stay_inside_the_panel`, `::a_minified_line_wrapped_stays_inside_the_panel`, `::a_minified_line_unwrapped_stays_inside_the_panel`. No test reads the glyphs. | `desktop/tests/native/diff_stress.rs` |

### 2.8 PR review session

Mockup: `review` §1, §2, §5. Frames: 67 `67-pr-review-dialog`, 68
`68-pr-review-session`, 68b, 68c, 69 `69-pr-submit-review`.

| ID | Rule | Status | Where |
|---|---|---|---|
| DS-52 | The panel header names what is reviewed: "PR #482 against main · 2 repos", which ellipsizes before ⤢ and ✕. | Built: `pr_session.rs::the_diff_names_the_pull_request_and_each_repository`, `::a_narrow_review_panel_keeps_full_screen_and_close`. | `ui/changes/src/panel_changes/review_header.rs`: `review_pulls`, the PR line (`review-pr-line-text`) |
| DS-53 | The file bar steps files across every PR of the review ("1 of 6 ‹ ›"). | Built (DV-15): `diff_keys.rs::the_inline_review_steps_files_across_its_pull_requests`; `features/review/tests/stepping.rs::next_and_previous_file_walk_the_files_across_repos_and_stop_at_the_ends`. | `ui/changes/src/panel_changes/file_list.rs`: `file_bar` |
| DS-54 | An agent's draft is outlined in blue and marked "Draft for GitHub", with Edit and Drop. | Settled by D856 and D861: an agent's GitHub draft reads "<agent> [Draft] line N" with Edit and Drop, on the plain grey line; only the open box is outlined. Test: `pr_session.rs::a_draft_for_github_has_edit_and_drop_and_the_footer_counts_the_drafts`. | `ui/changes/src/comment_cards.rs`: `agent_github_draft` |
| DS-55 | The footer counts the drafts ("4 drafts · 3 by Copilot, 1 by you") and holds the one blue **Submit review…**, beside the terminal and in full screen. | Built (D861, RV-1): the footer reads "4 drafts · 3 by claude, 1 by you" (bare "4 drafts" if it can't fit) beside **Submit review…**, with the session bar's button hidden while the Diff shows. Tests: `pr_session.rs::submit_review_in_the_footer_opens_the_sheet`, `::the_session_bar_hides_submit_review_while_the_diff_shows`; unit `review_header.rs::the_count_names_who_wrote_the_drafts_when_it_fits`, `::a_count_that_does_not_fit_falls_back_to_the_bare_total`. No full-screen test. | `ui/changes/src/panel_changes/mod.rs`: `changes` footer; `ui/changes/src/panel_changes/review_header.rs`: `drafts_count` |
| DS-56 | **Submit review** opens one sheet for every PR of the review: what goes out and who wrote it, an optional summary, Comment / Approve / Request changes, then Cancel or Submit to GitHub. Nothing is sent before this. | Built: `ui/overlays/src/overlay_submit_review.rs`; `pr_review.rs::submit_posts_one_review_per_pull_request`, `::cancel_closes_submit_review_without_posting`; live: `desktop/tests/live_flows/pr_review.rs`. | `ui/overlays/src/overlay_submit_review.rs` |
| DS-57 | New commits on a PR show as a bar under the terminal ("2 new commits on the PR branch", or "The PR branch was force-pushed") with **Update the clone**. A merged or closed PR offers Archive. | Built: `pr_session.rs::update_the_clone_sends_the_update_and_clears_the_notice`, `::a_merged_pull_request_offers_archive`; a force-push with new commits names both ("The PR branch was force-pushed with 2 new commits", Fable 2026-11-19), `::a_force_push_with_new_commits_names_both`. | `ui/src/pages.rs` (`pr-update-clone`) |
| DS-58 | **Back:** every session page has a ‹ ghost button at the start of its top bar, tooltip "Back ⌘[", that goes to wherever I was. | Built (DV-18, D848): the 24 pt ghost Back with the tooltip "Back  ⌘[" on the session header and every top bar. Test: `diff_full_screen.rs::a_session_page_has_a_back_button` (checks the button exists). | `ui/src/pages.rs` (`session-back`); `ui/src/shell.rs` (`back`); `ui/core/src/keys.rs`: `cmd-[` |
| DS-59 | The line the agent focuses is marked and scrolled into view; files are listed bottom-up. | Built: `pr_session.rs::the_line_the_agent_focuses_is_marked`, `::a_focus_below_the_fold_is_scrolled_into_view`, `::files_are_listed_bottom_up`. | `ui/changes/src/panel_changes/review_header.rs`: `reveal_focus` |

### 2.9 Stacks across repos

Mockup: `stacks` §1–§3. Frames to come: 119 `119-diff-stack-whole`, 120
`120-diff-stack-layer`, 121 (the skim fold open). The stacks UI is owned by
[STACKVIEW.md](STACKVIEW.md); its tasks are named per rule.

| ID | Rule | Status | Where |
|---|---|---|---|
| DS-60 | A stack shows a layer list above the files: **Whole stack** first, then the layers top-down with 1 at the bottom. Each row has its number, title, its PRs per repo ("api #412 · web #88", collapsing to "+N"), and its file count. About 6 rows show; the list scrolls and keeps the picked row in view. | Not built (STACKVIEW T8). | |
| DS-61 | The picked row is highlighted like a picked file and shows its one-line purpose. | Not built (T8). | |
| DS-62 | The header reads "Whole stack · 3 layers" or "Layer 2 of 3 · journal"; the base picker is hidden while a stack shows. | Not built (T7, T8). | |
| DS-63 | A layer shows only the repos it touches; a repo it doesn't touch gets one quiet line ("web isn't in this layer"). | Not built (T6 repo filter, T8). | |
| DS-64 | A layer's diff is its own base..head, right whatever is checked out. | Partly built: the engine (STACKVIEW T1, T2; `daemon/review_diff/tests/stack.rs`); the daemon service and app are T4, T6, T7. | |
| DS-65 | A repo that moved after the stack was sent says "web moved since it was sent" on that layer, with one action, **Ask the agent to send it again**. | Partly built: the engine's drift check (T2 `drift_blocking`); the row is T4, T8. | |
| DS-66 | A stack with no notes draws no review chrome: no note gutters, no draft affordances, no verdicts. | Not built (T8 test). | |
| DS-67 | Key files carry a dot; skim files fold into one row at the end of each repo ("3 files to skim · Cargo.lock, …") that opens in place. | Not built (T3c, T8; frame 121). | |
| DS-68 | Outside a review chat, a stack arrives as an Inbox offer ("3 layers over 2 repos", Open the stack, Later); opening it starts on Whole stack. | Not built (T4, T8). | |

## 3. Keyboard

Every key below goes in the ⌘/ shortcuts sheet (`ui/src/keys.rs:84`,
`SHORTCUTS`). None of the new ones is bound yet.

| Keys | Does | Where it works | Status |
|---|---|---|---|
| ⌥⌘↩ | Diff full screen on and off (DS-12, DS-13) | A session with its panel open | Built as ⌃⌘↩ (`ctrl-cmd-enter`, DIFFVIEW lane 17:05; off a Mac ⌥⌘↩ clashed with ⇧⌘↩). In the ⌘/ sheet. `diff_full_screen.rs::alt_cmd_enter_enters_and_leaves_full_screen` |
| ⇧⌘↩ | Terminal full screen | A session | Built (`keys.rs`: `shift-cmd-enter`) and now in the ⌘/ sheet ("Terminal full screen"). No test of the sheet row |
| Esc | Leave full screen, after any menu, sheet or comment box has closed first (DS-13) | Full screen | Built: `diff_full_screen.rs::escape_leaves_full_screen_but_not_while_the_base_menu_is_open`. The sheet reads "Close or cancel" |
| ⌥⌘↓ / ⌥⌘↑ | Next / previous file (DS-2, DS-53) | Diff panel open | Built (`alt-cmd-down`, `alt-cmd-up`; STACKVIEW's layer keys moved to ⌥⌘] / ⌥⌘[). `diff_view.rs::next_file_steps_across_repos`; `features/review/tests/stepping.rs` |
| ⇧⌥⌘↓ / ⇧⌥⌘↑ | Next / previous change (DS-27) | Diff panel open | Built as ⌃⌘↓ / ⌃⌘↑ (`ctrl-cmd-down`, `ctrl-cmd-up`; ⇧⌥⌘ would clash off a Mac). `diff_view.rs::a_long_file_shows_a_change_stepper` |
| ⌥⌘] / ⌥⌘[ | Next / previous layer (DS-60) | A stack shows | Built (`DIFF_STACK` context). `stack.rs::the_next_layer_key_picks_the_bottom_layer_from_whole_stack` |
| ⌥Z | Wrap / Scroll (DS-40) | The diff has focus | Built, only in the diff's key context; `Input` rebinds it to nothing. `review_ui.rs::option_z_in_the_comment_box_types_and_does_not_flip_wrap` |
| ↑ ↓ ↩, ← → | In the focused file list: move, open, collapse or expand a repo | File list focused | Built (`DIFF_LIST`). `diff_keys.rs::the_focused_file_list_takes_the_arrows_and_enter` |
| ⌘[ | Back (DS-58) | Anywhere | Built (`keys.rs`: `cmd-[`), in the sheet |
| Double-click the panel header | Full screen (DS-12) | Panel header | Built. `diff_full_screen.rs::double_clicking_the_panel_header_enters_full_screen` |

The palette gets "Next layer", "Previous layer" and "Whole stack"
(STACKVIEW T8). The four ⌥⌘ arrows need a session context so they never
shadow the sidebar's ⌥⌘← and ⌥⌘→ (`keys.rs:211-212`).

## 4. Behaviour rules

| ID | Rule | Status | Where |
|---|---|---|---|
| DS-69 | **Show the terminal keeps my place.** Going full screen and back keeps the open file, the layer, the focused line, the scroll position, the sideways shift, the Wrap choice, any open comment box and every draft. The terminal keeps running underneath; nothing is reloaded. | Partly built: the file, line scroll, draft and open box are kept, and no fetch is sent: `review_ui.rs::show_the_terminal_keeps_the_line_scroll_drafts_and_open_box`, `::full_screen_and_back_sends_no_fetch`, `diff_full_screen.rs::show_the_terminal_keeps_the_selected_file_and_its_scroll`. Not tested: the layer, the sideways shift, the Wrap choice, and that the terminal keeps running underneath. | `ui/changes/src/document_tab.rs`; `features/app/src/lib.rs` |
| DS-70 | Full screen is remembered per session: another session opens in its own state, and coming back restores this one's. | Built: `AppState.full_screens` per session; `features/app/tests/presentation.rs::expanding_either_area_and_returning_preserves_the_columns` (saves and restores it). No test switches sessions while one is full screen. | `features/app/src/lib.rs`: `full_screens` |
| DS-71 | Closing the panel ends full screen. | Built: `diff_full_screen.rs::close_in_full_screen_closes_the_panel`; `features/app/tests/presentation.rs::closing_the_full_screen_panel_returns_to_the_terminal`. | `features/session/src/lib.rs`: `SessionAction::SetPanel` |
| DS-72 | The open file is remembered per session and survives a restart. | Built: `features/app/tests/presentation.rs::a_restart_restores_both_tab_groups_and_the_selected_file`. | `features/review/src/lib.rs`: `ReviewSaved::selected` |
| DS-73 | Scroll position is remembered per session and file. | Built (RV-3): fixed by `session_diff_view`, which keys scroll by session, repo and path. Test: `review_ui.rs::two_sessions_on_one_file_keep_their_own_scroll`. | `ui/changes/src/document_tab.rs`: `session_diff_view` |
| DS-74 | Collapsed repos are per session and are not kept across a restart. | Built (DV-7, DV-10): `diff_view.rs::a_collapsed_repo_hides_its_files_and_opens_again`; `features/review/tests/collapse.rs`. `collapsed` is not in `ReviewSaved`, so a restart opens every repo (no test of the restart). | `features/review/src/lib.rs`: `ReviewState::collapsed` |
| DS-75 | Wrap is one app-wide preference. | Built: `features/app/tests/presentation.rs::long_lines_survive_a_restart_both_ways`. | `features/app/src/lib.rs`: `wrap_diffs` |
| DS-76 | Opening, closing or widening the panel does not read the whole diff again. | Built (RV-3): `review_ui.rs::closing_and_reopening_the_panel_sends_no_fetch`, `::full_screen_and_back_sends_no_fetch`; `features/review/tests/open_again.rs::opening_again_with_the_diff_held_reads_nothing`, `::try_again_and_a_base_pick_still_read`. | `features/review/src/reopen.rs` |
| DS-77 | No status that doesn't help me act: no summary row, no repeated session branch, no "N behind". A problem shows where it blocks: under its repo, in its file, or on the action it stops. | Partly built: no summary row, no repeated session branch, per-repo errors under their row (DS-35). Missing: a per-file failure shows nowhere (DS-36), and the PR review repo row's chevron is a dead control (DS-5). | see DS-5, DS-35, DS-36 |
| DS-78 | Picking a file in a collapsed repo, or stepping onto it with ⌥⌘↓, opens that repo. Collapsing the repo that holds the open file keeps the file open. | Built (DV-7, DV-15): `features/review/tests/stepping.rs::stepping_onto_a_file_in_a_collapsed_repo_opens_the_repo`; `features/review/tests/collapse.rs::collapsing_the_repository_of_the_selected_file_keeps_the_selection`. | `features/review/src/lib.rs` |

## 5. Performance budgets

Numbers marked **source** come from the report or the DV list. Numbers
marked **proposed** are not in any source; they are this spec's proposal and
need main's bench to confirm (DV-14).

| ID | Case | Budget | Origin | Today (report) |
|---|---|---|---|---|
| P-1 | First open of a 40,000-line file (+20,000 −20,000) | Opens "at once": proposed under 300 ms to first rows | source (design §2 "opens at once"); number proposed | ~70 s |
| P-2 | Reselecting that file | Proposed under 100 ms | proposed | ~25 s at 100% CPU |
| P-3 | Scrolling that file | Proposed: no dropped frames at 60 Hz, under 30% of one core | proposed | 72–99% CPU |
| P-4 | Memory with that file open | Proposed: under 300 MB above the idle app | proposed | 0.5 → 4.8 GB |
| P-5 | Loading 586 files over 4 repos | Under 5 s, or a follow-up opens to load a file's lines only when it is picked | source (DV-14) | 177 files ~30 s |
| P-6 | Wrapping a 194,000-character line | Measure again after DV-5; proposed under 500 ms | source (design, "measure it again"); number proposed | ~2 s |
| P-7 | Base switch | The chip names the new base in the same frame; proposed under 5 s for the new diff on the 4-repo session | source (DS-34); number proposed | 30–60 s+ |
| P-8 | Idle | About 0% CPU with the panel open | source (report "What works") | ~0% |
| P-9 | Word highlight and wrap | Computed only for rows in view | source (design §2, DV-5 spec 2) | n/a |

## 6. Accessibility

| ID | Rule | Status | Where |
|---|---|---|---|
| DS-79 | Every icon button has an accessibility label and a tooltip that names it and its key ("Full screen ⌥⌘↩", "Close panel", "Open in editor", "Previous file ⌥⌘↑"). | Built: icon buttons carry an aria label and a tooltip naming the key (`icon_action`, ‹ ›, Wrap "Wrap long lines  ⌥Z", Back "Back  ⌘[", Open in editor, Full screen / Exit full screen "⌃⌘↩", the terminal arrow "⇧⌘↩"). Tests: `diff_full_screen.rs::{the_full_screen_button_names_its_key, the_terminal_arrow_names_its_key}`; every diff key is in the shortcuts sheet (`ui/core keys::tests::every_diff_key_is_in_the_shortcuts_sheet`). | `ui/core/src/chrome.rs`: `icon_action`, `panel_action`; `ui/src/pages.rs` |
| DS-80 | Every ellipsized text shows its full text on hover and carries it in its label (file paths, repo, branch, base, PR line, session title). | Built: `full_text` gives hover and a label to the base chip, per-repo base chip, PR line, file bar names, repo, branch, repo error name, file header, footer count, file row path, file note, full-screen title line and session title. Tests: `review_ui.rs::{a_shortened_path_shows_in_full_on_hover, a_shortened_session_title_shows_in_full_on_hover}`, `diff_view.rs::a_rename_names_where_it_came_from`, `diff_full_screen.rs::the_full_screen_title_line_keeps_its_whole_text`. | `ui/core/src/full_text.rs`; `ui/changes/src/panel_changes/file_list.rs`: `file_row` |
| DS-81 | The code area is a labelled code region; file rows are list options that say whether they are picked. | Built, no test: the code is a labelled `Role::Code` region and file rows are `ListBoxOption` with `aria_selected`. | `ui/changes/src/panel_changes/file_list.rs`: `file_row`; `ui/changes/src/panel_changes/review_header.rs`: `review_code` |
| DS-82 | Status marks are letters (A, D, R, bin, mode), never colour alone. | Built (DV-12): letters A, D, R, "bin" and "mode" in the file row, never colour alone. `diff_view.rs::a_rename_names_where_it_came_from`, `::a_binary_file_says_binary_instead_of_an_empty_diff`. | `ui/changes/src/panel_changes/file_list.rs`: `file_row` |
| DS-83 | After Show the terminal, focus goes back to where it was before full screen (the terminal, or the diff). | Partly built (RV-3): focus returns to the terminal: `review_ui.rs::show_the_terminal_returns_focus_to_the_terminal`. A diff-focused return is not tested. | `features/app/src/overlay.rs` |
| DS-84 | The file list can be driven by keys alone (DS keyboard map), and the focus ring is visible. | Built (DV-15): `diff_keys.rs::the_focused_file_list_takes_the_arrows_and_enter`, `::an_unfocused_file_list_ignores_the_arrows`; `features/review/tests/file_list_keys.rs`. The focus ring is `ringed`. | `ui/changes/src/panel_changes/file_list.rs`: `ringed`; `ui/core/src/keys.rs`: `DIFF_LIST` |

## 7. Frames that hold these rules

Design (`desktop/tests/__snapshots__/design/`): 11, 12, 41, 50, 51–53, 67,
68, 69, 84, 85, 86, 98. Native (`desktop/tests/__snapshots__/native/`):
`11-panel-full-screen-diff`, `41-review-beside-terminal`, `68-pr-review-session`,
`68b-pr-review-full-screen`, `68c-pr-review-after-show-terminal`,
`69-pr-submit-review`, `changes-pushed`, `changes-wrapped-long-line`,
`changes-wrapped-long-line-dark`. New frames planned: `narrow-list-folded`,
`narrow-list-open`, `empty`, `loading` (DV-13), every `diff-stress-*`
(DV-13), 119–121 (STACKVIEW). Each new or changed frame goes to Fable for a
pick before it goes to the owner (D76).

## 8. Contradictions between sources, and how this spec settles them

1. **⌥⌘↓ / ⌥⌘↑ mean two things.** The design (and mockup `diff-proposal` §1)
   uses them for next / previous file; STACKVIEW T8 and mockup `stacks` use
   them for next / previous layer, and T8's test expects "alt-cmd-down picks
   layer 1". Files move far more often than layers, and the design ranks
   higher. **Settled:** files keep ⌥⌘↓/↑; layers move to ⌥⌘] / ⌥⌘[ (free in
   `keys.rs`), and the palette keeps Next layer and Previous layer. STACKVIEW
   T8's test and the `stacks` mockup's key hint change with it. Fable to
   confirm before T8.
2. **⌥Z for Wrap.** Option is not Alt in the terminal (`terminal/vt/src/terminal.rs:223`),
   so ⌥Z types "Ω" there and in text boxes. **Settled:** ⌥Z is bound only in
   the diff's own key context (the code or the file list focused), never
   app-wide.
3. **The summary row goes, but Add repo… lives in it** (`panel_changes.rs:424-445`,
   REPOS CH-21). **Settled:** Add repo… moves to the footer's left, beside
   Push & PR…, as a second quiet text button; it is always an action, so a
   session's footer always shows. Fable to pick in DV-16's frames.
4. **Where Submit review lives.** Frame 68 puts it in the session bar
   (`pages.rs:316-330`); the owner-approved mockup (`review` §1) puts it in
   the panel footer. **Settled:** the footer holds it beside the terminal and
   in full screen (DS-55). While the Diff panel shows, the session bar's
   button is hidden, so the screen has one blue action; with the panel closed,
   the session bar keeps it. Frame 68 is re-recorded.
5. **Where "Change N of M" sits.** The design puts it in the pinned file
   header, only for files with more than a screen of changes; mockup `review`
   §5 draws "Change 3 of 12 ‹ ›" in the full-screen title bar of a small
   review. **Settled:** the file header, under the design's condition
   (DS-27). The title-bar placement is treated as a sketch.
6. **Card width.** The design caps cards at 720 pt; the code caps them at
   640 pt (`comment_cards.rs:82`). 640 is inside the cap, so it stays; only the
   start at the code column is new.
7. **When rows go virtual.** The Diff panel switches at 1,000 lines
   (`panel_changes.rs:2110`); DV-5 keeps PR review files whole up to 1,500
   lines so frame 68 doesn't move. Both stand: they are different surfaces
   (DS-25, DS-29).
8. **Draft count wording.** Code: "Drafts: 4 (3 by agent, 1 by you)";
   mockup: "4 drafts · 3 by Copilot, 1 by you". The mockup's wording is the
   target, naming the agent. Tests don't assert wording.

# A 40,000-line file opens at once and scrolls smoothly, with its header pinned and a way to step between changes.

Layer: Diff and review. Platforms: macOS desktop.

**DV-5 in progress.** Another agent is changing `ui/src/panel_changes.rs` and `ui/src/document_tab.rs` now, for DV-5 (virtual rows). Every line number and every "today" below describes the code as it stood on 2026-11-18, before DV-5 lands. Recheck each row after DV-5.

## Starting state

- `acme/ledger` has a session "Refunds" in `#payments`, with its Diff panel open beside the terminal.
- `big.txt` in that session changes 40,000 lines: 20,000 removed and 20,000 added, in one hunk. It is the owner's stress file.
- A second, small file is in the same repo, so I can select something else.
- The app is idle.

## Budgets

The numbers come from the spec's section 5 (`workflow/milestones/DIFFVIEW-spec.md`). "Source" means the number comes from a source. "Proposed" means it is the spec's own proposal, and it needs the bench (DV-14) to confirm it.

| ID | What | Budget | Origin | Today (report, 2026-11-18) |
|---|---|---|---|---|
| P-1 | First open of big.txt | Opens "at once": under 300 ms to the first rows | "opens at once" is source (design); "300 ms" is **proposed** | About 70 s |
| P-2 | Reselecting big.txt | Under 100 ms | **proposed** | About 25 s at 100% CPU |
| P-3 | Scrolling big.txt | No dropped frames at 60 Hz, under 30% of one core | **proposed** | 72–99% CPU |
| P-4 | Memory with big.txt open | Under 300 MB above the idle app | **proposed** | 0.5 GB to 4.8 GB |
| P-9 | Word highlight and wrap | Computed only for rows in view | source (design §2, DV-5 spec 2) | Word highlight not built (DS-21). Wrap is computed per visible row in the virtual list. |

## Reproduce headless

- Native, on fakes: `bazel test //apps/humata2/desktop:native_test --test_filter=a_huge_diff_draws_only_the_lines_in_view` (`desktop/tests/native/diff_big.rs`).
- Native, on fakes: `bazel test //apps/humata2/desktop:native_test --test_filter=a_huge_diff_scrolls_to_its_last_line`.
- Native, on fakes: `bazel test //apps/humata2/desktop:native_test --test_filter=a_huge_line_wraps_in_a_long_file`.
- Native, on fakes: `bazel test //apps/humata2/desktop:native_test --test_filter=widening_a_wrapped_diff_keeps_its_top_line_in_place` (`desktop/tests/native/review.rs`). That test uses a small file.
- Not headless: any timing. No test in `diff_big.rs` measures time, memory or CPU. The budgets need the live bench. The report's numbers came from a live run on a local clone with the real daemon. No windows on the owner's screen.
- Mockup target: `workflow/milestones/mockups/diff-proposal.png`, section 3 "Big files".

## Steps

| # | I do | I see | State and events | Status |
|---|---|---|---|---|
| 1 | Select big.txt. | The first screen of lines, at once. No "load anyway" gate and no spinner. (DS-25) | Files over 1,000 lines use the virtual list: `VIRTUAL_FROM` is 1,000 (`ui/src/panel_changes.rs:2112`), and the branch is at `:522`. The list is `gpui_kit::list` (`:544`). Only the rows in view are built. The test asserts 0 < drawn < 200 rows. DV-5 in progress; recheck. | Fakes only: `desktop/tests/native/diff_big.rs::a_huge_diff_draws_only_the_lines_in_view`. Built (DV-5). Measured P-1: 71 ms for the first open (`review_ui.rs::budgets::a_forty_thousand_line_file`, ignored, panel only). |
| 2 | Watch the body while the file's lines arrive. | A dim "Loading big.txt…" over the lines. (DS-28) | No loading text. The file's lines are read in one go by the client (`clients/live/src/diff.rs:430-463`). No `changes-loading` element exists. | Not built (DS-28). No per-file loading line: a big file's pages are read inside one `session_diffs` call (`clients/live_diff/src/diff.rs`), so the panel shows only the whole-panel sweep (`ui/changes/src/changes_loading.rs`) and a dimmed list. No open task. |
| 3 | Time from the click to the first rows. | Under 300 ms (P-1, proposed). | Not timed. The report measured about 70 s before DV-5. | Fakes only, measured by hand: P-1 71 ms for the panel's first open of 40,000 lines against the 300 ms budget (`review_ui.rs::budgets::a_forty_thousand_line_file`, ignored; RV-8, DIFFUI2). The daemon's side is DV-14's live rerun, not yet run. |
| 4 | Select the small file, then big.txt again. | The lines come back at once. (P-2, proposed under 100 ms) | The line facts are cached per read of the lines (`ui/src/document_tab.rs:356-370`, `facts`), and the list state is kept (`:374-404`, `rows`). No test times it. | Fakes only, measured by hand: P-2 12.5 ms to reselect, within the 100 ms budget (`review_ui.rs::budgets::a_forty_thousand_line_file`, ignored). |
| 5 | Scroll to the end of big.txt. | The last added line is drawn. The first lines are gone. Fewer than 200 rows are built at the end. | The list is scrolled to the end, and the test asserts `change-line-40000` is drawn and `change-line-1` is not. | Fakes only: `desktop/tests/native/diff_big.rs::a_huge_diff_scrolls_to_its_last_line`. |
| 6 | Scroll big.txt with the trackpad, up and down, for a few seconds. | The scroll is smooth. No dropped frames at 60 Hz. (P-3) | Not measured. The report measured 72–99% of one core before DV-5. Each row is looked up with `file_lines` on every row built (`ui/src/panel_changes.rs:2023`, `:545-550`). | Fakes only, measured by hand and over budget: P-3 mean frame 11.1 ms against 5 ms (slowest 12.4 ms, within 16.7 ms; `review_ui.rs::budgets::scrolling_a_forty_thousand_line_file`, ignored). Follow-up RV-8a is open: split the 11.1 ms between the panel and the rest of the frame. No dropped frames, so it is CPU, not smoothness. |
| 7 | Watch Activity Monitor with big.txt open. | About 300 MB above the idle app. (P-4, proposed) | Not measured. The report measured 0.5 GB rising to 4.8 GB before DV-5. | Fakes only, measured by hand: P-4 +12 MB with the file open, within 300 MB (`review_ui.rs::budgets::a_forty_thousand_line_file`, ignored, panel only). Not measured on the live app. |
| 8 | Scroll the code while the file header is on screen. | The file header (name and "Open in editor") stays put. Only the code scrolls under it. (DS-26) | The header row sits above the code, and the code is the scroll region (`ui/src/panel_changes.rs:588-604`). The scroll is `change-code <name>` (`:533`). | Code, no test. The header sits outside the scroll region (`panel_changes/mod.rs::changes`). No test checks that it stays put in the Diff panel. |
| 9 | In a PR review, scroll a big file. | The file's head is pinned. (DS-26) | The repo's group row is a child of the list, and the review list scrolls as one (`ui/src/panel_changes.rs:716-721`, `review_repo` at `:891`, `review_repo_head` call at `:904`). The file head scrolls with the list. | Fakes only. Built (RV-2): `desktop/tests/native/pr_session.rs::a_review_file_head_stays_while_its_lines_scroll`. A review file over 1,500 lines still builds every line (DS-29, not built). |
| 10 | Look at the file header of a file with more than a screen of changes. | "Change 3 of 212 ‹ ›" in the header, and ⇧⌥⌘↓ / ⇧⌥⌘↑ step between changes. (DS-27) | No "Change" counter in `ui/src/`. No key binding for ⇧⌥⌘↓ or ⇧⌥⌘↑ in `ui/src/keys.rs`. | Fakes only. Built (DV-15): `desktop/tests/native/diff_view.rs::a_long_file_shows_a_change_stepper`; `features/review/tests/stepping.rs::next_and_previous_change_stay_within_the_files_hunks`. The keys are ⌃⌘↓ / ⌃⌘↑ (not ⇧⌥⌘). The stepper shows for a file with more than one hunk and more than 60 lines. |
| 11 | Turn Wrap on and select a file whose one line has 50,000 characters. | The line wraps into rows, and the first rows show at once. (P-6) | The test asserts the huge line's first rows and its second row are drawn. P-6's 500 ms is proposed, and the report measured about 2 s. | Fakes only: `desktop/tests/native/diff_big.rs::a_huge_line_wraps_in_a_long_file`. Measured P-6: 85 ms for a 194,000-character line against 500 ms (`review_ui.rs::budgets::wrapping_a_194_000_character_line`, ignored). |
| 12 | Scroll big.txt to a line with a comment on it, far down. | The comment card sits under its line. (DS-25, DV-5 test) | `CodeRows::item` draws the cards under their line (`ui/src/panel_changes.rs:2267-2274`). The planned test, `a_comment_card_far_down_shows_under_its_line_when_scrolled_to`, is not written. | Code, no test. `CodeRows::line` draws the cards under their line (`panel_changes/diff_body.rs`). The planned `a_comment_card_far_down_shows_under_its_line_when_scrolled_to` was never written. |
| 13 | Resize the panel so the wrapped lines reflow. | The line at the top of the view stays at the top. | The virtual list remeasures and keeps the top line (`ui/src/document_tab.rs:374-404`). The test covers a small file (`desktop/tests/native/review.rs::widening_a_wrapped_diff_keeps_its_top_line_in_place`). | Fakes only, for a small file: `desktop/tests/native/review.rs::widening_a_wrapped_diff_keeps_its_top_line_in_place`. Not tested on big.txt. |
| 14 | Leave the panel open and do nothing. | About 0% CPU. (P-8) | Not measured. The report says about 0%. | Code, no test. P-8 (idle CPU) is not measured headless; it comes from DV-14's live rerun, not yet run. |
| 15 | Switch the base on big.txt while its lines are still loading. | The chip names the new base in the same frame, and the diff follows. (DS-34) | The chip's base is the first loaded diff's base, not the picked one (`ui/src/panel_changes.rs:611-615`). While the new diff loads, the chip shows the old base. | Fakes only. Likely bug cleared (DV-12): `desktop/tests/native/diff_view.rs::picking_a_base_shows_loading_until_the_diff_arrives`. While a read runs the chip names the picked base (`panel_changes/mod.rs::changes`, `picked_base_name`). The live time (P-7) is not measured. |

## Branches

### Wrap is switched off on the big file

When I turn Wrap off with big.txt open, and then scroll sideways. The number and sign stay put. The code moves left, as far as its widest line.
**I see:** the gutter fixed, the code shifted.
**Expected:** `max_shift` is the widest line's width, counted once per read of the file (`ui/src/panel_changes.rs:2465-2480`, `line_facts`, cached in `document_tab.rs:356`).
**Status:** Code, no test. `panel_changes/diff_body.rs::line_facts` counts the widest line once per read of the lines; the sideways shift of the Diff panel has no test.

### Lines of big.txt change on disk while it is open

When the file changes on disk and I refresh. The list's line count changes, and the list keeps the top line by its index (`ui/src/document_tab.rs:374-404`: `reset_with_uniform_height` plus `scroll_to`).
**Expected:** the view stays where it was, in index terms, and the rows are drawn again.
**Status:** Code, no test. A refresh changes the list's line count and the list keeps its top index (`ui/changes/src/document_tab.rs`).

### The base changes while big.txt is open

When I change the base while the lines are still loading. The chip should name the picked base at once.
**I see:** the chip shows the old base until the new diff arrives. The report saw a 30–60 s wait.
**Status:** Fakes only. Likely bug cleared (step 15): `diff_view.rs::picking_a_base_shows_loading_until_the_diff_arrives`.

## Expected outcomes

- [ ] Selecting big.txt shows its first rows with no load gate.
- [ ] Only the rows in view are built, at the start and the end of the file.
- [ ] Scrolling to the last line shows it, and the first lines are gone.
- [ ] The file header stays put while the code scrolls, beside the terminal.
- [ ] A line with 50,000 characters wraps, and its first rows show.
- [ ] A comment card far down the file shows under its line.
- [ ] Reselecting and scrolling each stay within the proposed budgets (P-2, P-3), measured by the bench, not by these tests.
- [ ] Memory stays within P-4 (proposed) above the idle app, measured by the bench.
- [ ] A change counter and keys step between changes in a file with more than a screen of them.

## Status table

Counts per tag. A step or branch counts once, under its tag.

| Tag | Steps (of 15) | Branches (of 3) | Total |
|---|---|---|---|
| Works (real shell) | 0 | 0 | 0 |
| Works (real daemon) | 0 | 0 | 0 |
| Fakes only | 11 (steps 1, 3, 4, 5, 6, 7, 9, 10, 11, 13, 15) | 1 | 12 |
| Code, no test | 3 (steps 8, 12, 14) | 2 | 5 |
| Not built | 1 (step 2) | 0 | 1 |
| Likely bug | 0 | 0 | 0 |

## Gaps and likely bugs

- Not built: a per-file "Loading big.txt…" line (step 2, DS-28). No open task.
- Over budget: P-3. The mean scroll frame is 11.1 ms against 5 ms (headless, the whole window). Follow-up RV-8a (REVIEWUI) is open: time the panel's own share first.
- Not run: DV-14's live rerun on the 4-repo rig (P-5 daemon side, P-7, P-8). The panel-side numbers (P-1, P-2, P-4, P-6) come from the ignored `review_ui.rs::budgets::` tests and are within budget.
- Not built: a PR review's big file in its own box (step 9's second half, DS-29): `review_header.rs::review_code` builds every line. No open task.
- Gap in tests: the Diff panel's pinned header, a comment card far down a big file, and a reflow on big.txt.
- Cleared: the base chip naming the old base while loading (step 15).
- The "State and events" cells above (key names, line numbers) describe the 2026-11-18 code; only the Status cells and the counts were re-audited.

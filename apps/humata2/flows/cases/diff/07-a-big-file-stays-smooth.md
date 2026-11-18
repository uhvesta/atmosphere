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
| 1 | Select big.txt. | The first screen of lines, at once. No "load anyway" gate and no spinner. (DS-25) | Files over 1,000 lines use the virtual list: `VIRTUAL_FROM` is 1,000 (`ui/src/panel_changes.rs:2112`), and the branch is at `:522`. The list is `gpui_kit::list` (`:544`). Only the rows in view are built. The test asserts 0 < drawn < 200 rows. DV-5 in progress; recheck. | Fakes only: `desktop/tests/native/diff_big.rs::a_huge_diff_draws_only_the_lines_in_view`. The open time (P-1) is not tested. |
| 2 | Watch the body while the file's lines arrive. | A dim "Loading big.txt…" over the lines. (DS-28) | No loading text. The file's lines are read in one go by the client (`clients/live/src/diff.rs:430-463`). No `changes-loading` element exists. | Not built (DV-12). |
| 3 | Time from the click to the first rows. | Under 300 ms (P-1, proposed). | Not timed. The report measured about 70 s before DV-5. | Code, no test: `ui/src/panel_changes.rs:522`. |
| 4 | Select the small file, then big.txt again. | The lines come back at once. (P-2, proposed under 100 ms) | The line facts are cached per read of the lines (`ui/src/document_tab.rs:356-370`, `facts`), and the list state is kept (`:374-404`, `rows`). No test times it. | Code, no test: `ui/src/document_tab.rs:356-404`. |
| 5 | Scroll to the end of big.txt. | The last added line is drawn. The first lines are gone. Fewer than 200 rows are built at the end. | The list is scrolled to the end, and the test asserts `change-line-40000` is drawn and `change-line-1` is not. | Fakes only: `desktop/tests/native/diff_big.rs::a_huge_diff_scrolls_to_its_last_line`. |
| 6 | Scroll big.txt with the trackpad, up and down, for a few seconds. | The scroll is smooth. No dropped frames at 60 Hz. (P-3) | Not measured. The report measured 72–99% of one core before DV-5. Each row is looked up with `file_lines` on every row built (`ui/src/panel_changes.rs:2023`, `:545-550`). | Code, no test: `ui/src/panel_changes.rs:2023`. |
| 7 | Watch Activity Monitor with big.txt open. | About 300 MB above the idle app. (P-4, proposed) | Not measured. The report measured 0.5 GB rising to 4.8 GB before DV-5. | Code, no test: `ui/src/document_tab.rs:298-450`. |
| 8 | Scroll the code while the file header is on screen. | The file header (name and "Open in editor") stays put. Only the code scrolls under it. (DS-26) | The header row sits above the code, and the code is the scroll region (`ui/src/panel_changes.rs:588-604`). The scroll is `change-code <name>` (`:533`). | Code, no test: `ui/src/panel_changes.rs:588-604`. No test checks that the header stays put. |
| 9 | In a PR review, scroll a big file. | The file's head is pinned. (DS-26) | The repo's group row is a child of the list, and the review list scrolls as one (`ui/src/panel_changes.rs:716-721`, `review_repo` at `:891`, `review_repo_head` call at `:904`). The file head scrolls with the list. | Not built (DV-5 step 5). Spec DS-26 says partly built. |
| 10 | Look at the file header of a file with more than a screen of changes. | "Change 3 of 212 ‹ ›" in the header, and ⇧⌥⌘↓ / ⇧⌥⌘↑ step between changes. (DS-27) | No "Change" counter in `ui/src/`. No key binding for ⇧⌥⌘↓ or ⇧⌥⌘↑ in `ui/src/keys.rs`. | Not built (DV-15). |
| 11 | Turn Wrap on and select a file whose one line has 50,000 characters. | The line wraps into rows, and the first rows show at once. (P-6) | The test asserts the huge line's first rows and its second row are drawn. P-6's 500 ms is proposed, and the report measured about 2 s. | Fakes only: `desktop/tests/native/diff_big.rs::a_huge_line_wraps_in_a_long_file`. Timing not tested. |
| 12 | Scroll big.txt to a line with a comment on it, far down. | The comment card sits under its line. (DS-25, DV-5 test) | `CodeRows::item` draws the cards under their line (`ui/src/panel_changes.rs:2267-2274`). The planned test, `a_comment_card_far_down_shows_under_its_line_when_scrolled_to`, is not written. | Code, no test: `ui/src/panel_changes.rs:2267-2274`. |
| 13 | Resize the panel so the wrapped lines reflow. | The line at the top of the view stays at the top. | The virtual list remeasures and keeps the top line (`ui/src/document_tab.rs:374-404`). The test covers a small file (`desktop/tests/native/review.rs::widening_a_wrapped_diff_keeps_its_top_line_in_place`). | Fakes only, for a small file. Not tested on big.txt. |
| 14 | Leave the panel open and do nothing. | About 0% CPU. (P-8) | Not measured. The report says about 0%. | Code, no test: `ui/src/panel_changes.rs:522`. |
| 15 | Switch the base on big.txt while its lines are still loading. | The chip names the new base in the same frame, and the diff follows. (DS-34) | The chip's base is the first loaded diff's base, not the picked one (`ui/src/panel_changes.rs:611-615`). While the new diff loads, the chip shows the old base. | Likely bug: `ui/src/panel_changes.rs:611-615`. See the gaps. |

## Branches

### Wrap is switched off on the big file

When I turn Wrap off with big.txt open, and then scroll sideways. The number and sign stay put. The code moves left, as far as its widest line.
**I see:** the gutter fixed, the code shifted.
**Expected:** `max_shift` is the widest line's width, counted once per read of the file (`ui/src/panel_changes.rs:2465-2480`, `line_facts`, cached in `document_tab.rs:356`).
**Status:** Code, no test: `ui/src/panel_changes.rs:2465-2480`. The widest line is computed over all 40,000 lines, once.

### Lines of big.txt change on disk while it is open

When the file changes on disk and I refresh. The list's line count changes, and the list keeps the top line by its index (`ui/src/document_tab.rs:374-404`: `reset_with_uniform_height` plus `scroll_to`).
**Expected:** the view stays where it was, in index terms, and the rows are drawn again.
**Status:** Code, no test: `ui/src/document_tab.rs:374-404`.

### The base changes while big.txt is open

When I change the base while the lines are still loading. The chip should name the picked base at once.
**I see:** the chip shows the old base until the new diff arrives. The report saw a 30–60 s wait.
**Status:** Likely bug: `ui/src/panel_changes.rs:611-615` (step 15).

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
| Fakes only | 4 (steps 1, 5, 11, 13) | 0 | 4 |
| Code, no test | 7 (steps 3, 4, 6, 7, 8, 12, 14) | 2 | 9 |
| Not built | 3 (steps 2, 9, 10) | 0 | 3 |
| Likely bug | 1 (step 15) | 1 | 2 |

## Gaps and likely bugs

Not built:

- `ui/src/panel_changes.rs` (the file body): no "Loading big.txt…" line while lines arrive (DS-28, DV-12).
- `ui/src/panel_changes.rs:891-930` and `:716-721`: in a PR review, a file's head is not pinned, and a big file is not in its own box (DS-26, DS-29; DV-5 step 5).
- `ui/src/panel_changes.rs` and `ui/src/keys.rs`: no "Change N of M" counter and no ⇧⌥⌘↓ / ⇧⌥⌘↑ (DS-27, DV-15).
- `ui/src/panel_changes.rs` (`review_code`, `:950`): the PR review builds every line of a file, with no virtual list (DS-29, DV-5 step 5). Recheck after DV-5.

Likely bugs:

- `ui/src/panel_changes.rs:611-615`: the chip's base comes from `diffs.first()`, not from the picked base, so the chip names the old base until the new diff loads. Confirm: switch the base on a 40,000-line file in a native test and check the chip's label before the fake's reply.
- `ui/src/panel_changes.rs:2023` and `:545-550`: each row of the virtual list takes a read of the whole workspace and looks up its file by name (`file_lines`). That may cost a lot per frame. It is a suspicion from the code, not a measurement. Confirm with the bench (P-3).

Tests the budgets need, and do not yet have:

- A timing test for P-1 and P-2 on the 40,000-line file, in `diff_big.rs`.
- A memory or CPU check for P-3 and P-4.
- The DV-5 test `a_comment_card_far_down_shows_under_its_line_when_scrolled_to`, and a big-file version of `widening_a_wrapped_diff_keeps_its_top_line_in_place`.

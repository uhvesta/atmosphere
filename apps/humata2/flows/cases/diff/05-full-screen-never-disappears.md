# However long the names get, Full screen and Close stay on screen and the names shorten instead.

Layer: Diff and review. Platforms: macOS desktop.

The owner's ask, 2026-11-18: "Make sure the full screen button for the diff view always appears and other things in that line truncate instead when there is too much."

## Starting state

- `acme/api` has a session "Refunds" in `#payments`. Its Diff panel is open beside the terminal.
- The Diff's base is `origin/main...HEAD + staged + unstaged + untracked`, a long base on purpose.
- The panel is dragged to 220 pt, its narrowest useful width.
- A second session, "Ledger", is in `#payments`, with a PR review (`acme/api` #482) open in the same panel for step 6.
- Nothing is in full screen.

## Reproduce headless

- Native, on fakes: `bazel test //apps/humata2/desktop:native_test --test_filter=a_long_base_keeps_expand_and_close_inside_the_panel` (`desktop/tests/native/diff_view.rs`).
- Native, on fakes: `bazel test //apps/humata2/desktop:native_test --test_filter=a_narrow_review_panel_keeps_full_screen_and_close` (`desktop/tests/native/pr_session.rs`).
- Full screen and Show the terminal: `bazel test //apps/humata2/desktop:native_test --test_filter=show_the_terminal_returns_the_review_beside_the_terminal`.
- Not headless: how the ellipsis looks on a real display, and the hover tooltip. The native tests check bounds, not the drawn text. No windows on the owner's screen.
- Target look: mockup `workflow/milestones/mockups/review.png`, section 4 "Full screen never disappears". The button stays at the right at every width, and the names are cut before it.

## Steps

| # | I do | I see | State and events | Status |
|---|---|---|---|---|
| 1 | Open the Diff beside the terminal on "Refunds" with the long base. Drag the panel to 220 pt. | The header reads "Diff", then "against", then the base cut with an ellipsis. ⤢ and ✕ sit inside the panel's right edge. (DS-45, DS-46) | `expand-panel` and `close-panel` lie inside the `side-panel` bounds. The base text has `min_w_0`, `overflow_hidden` and `text_ellipsis` (`ui/src/panel_changes.rs:635-668`). The header's buttons are `flex_shrink_0` (`ui/src/panels.rs:122-178`, buttons at ~:152). | Fakes only: `desktop/tests/native/diff_view.rs::a_long_base_keeps_expand_and_close_inside_the_panel` (bounds only, not the ellipsis text). |
| 2 | Hover the base chip and wait. | No tooltip. The full base is not shown anywhere on hover. (DS-80) | The full base is in the accessibility label, `Against {base}` (`panel_changes.rs`, chip `aria_label`). The only `.tooltip(` in `panel_changes.rs` is at `:1569`, on a repo row. | Code, no test. Built (RV-4): the `diff-base` chip is wrapped in `full_text` (`panel_changes/mod.rs::changes`), so the full base shows on hover and in its label. No test hovers it; `review_ui.rs::a_shortened_path_shows_in_full_on_hover` covers a repo branch only. |
| 3 | With a session titled with a 150-character name, click ⤢. | Full screen opens. Its title bar reads "# channel › session" cut with an ellipsis, then "Diff", then **Show the terminal** at the right. (DS-14) | The title bar's names are `min_w_0`, `flex_shrink(1)`, `overflow_hidden`, `text_ellipsis` (`ui/src/panels.rs:297-354`). **Show the terminal** is an `action()` with `flex_shrink_0` (`panels.rs:83`). | Code, no test: `ui/core/src/chrome.rs::panel_header_actions` ellipsizes the names and keeps Exit full screen and Close (D848). No test uses a long session title in full screen. The title-bar names' hover is tested: `diff_full_screen.rs::the_full_screen_title_line_keeps_its_whole_text`. |
| 4 | In full screen, look under the title bar. | The panel's own 36 pt "Diff" header, with its own ⤢, is still drawn. (DS-14, DS-17) | The spec cites `panel_changes.rs:622` and `:695` for the second `header()` call; the call sites are now `:624` and `:699`. Not re-checked on the full-screen path. | Fakes only. Likely bug cleared (DV-18, D860): `diff_full_screen.rs::full_screen_draws_one_header` asserts there is no `panel-header` in full screen. |
| 5 | Click **Show the terminal**. | Back beside the terminal, with the same file open. | `show-terminal` sends `SessionAction::ToggleWiden`. | Fakes only: `desktop/tests/native/pr_session.rs::show_the_terminal_returns_the_review_beside_the_terminal`; the button now reads Exit full screen (id `show-terminal`). |
| 6 | Open PR #482 for "Ledger" as a review, with the panel at 220 pt. | The header's PR line reads "PR #482 against main · 2 repos", cut with an ellipsis. ⤢ and ✕ stay inside. (DS-52, DS-46) | `review-pr-line` is `review_line` (`panel_changes.rs:820-848`), an ellipsis on the whole line. Buttons as in step 1. | Fakes only: `desktop/tests/native/pr_session.rs::a_narrow_review_panel_keeps_full_screen_and_close`. |
| 7 | Make the panel narrower still, with the PR line long. | The PR line drops to "PR #482" before it gives up any more room. (DS-46) | No staged shrink. `review_line` shows the whole string and ellipsizes it (`panel_changes.rs:820-848`). | Not built (DS-46). `review_header.rs::review_line` ellipsizes the whole "PR #482 against main · 2 repos" from the right, so "PR #482" is the part that lasts, but there is no staged form. No open task. |
| 8 | Look at the PR's header with a 150-character PR title. | The header shows "PR #482" and the count, never the title. A long title cannot break the line. | `review_line` builds its text from the number, the base and the repo count only (`panel_changes.rs:820-848`). | Code, no test: `review_header.rs::review_line` builds its text from the number, the base and the repo count only. |
| 9 | Open the per-repo Diff on "Refunds", where a repo is named with 150 characters. | The repo row's name runs on, and the "+n −n" totals and the chevron can be pushed past the panel's right edge. (DS-45) | `repo_head` draws `text(diff.repo.clone())` with no `min_w_0` and no ellipsis, in a row with a `flex_1` spacer (`panel_changes.rs:1448`). DS-45 says the totals and the chevron never shrink. | Fakes only. Likely bug cleared (DV-10): `diff_view.rs::a_long_repo_and_branch_stay_inside_the_repo_row`; `diff_stress.rs::a_long_repo_and_branch_stay_inside_the_panel`. `repo_head` gives the name `min_w_0` and an ellipsis and pins the totals. |
| 10 | Open a PR review where a repo has a 150-character name. | The group row's name runs on past the row's room, and the row's right side may leave the panel. | `review_repo_head` draws `text(name.to_owned())` with no `min_w_0` and no ellipsis (`panel_changes.rs:1072`). | Fakes only. `review_header.rs::review_repo_head` ellipsizes the name after the branch and carries it whole on hover (`full_text`); `diff_stress.rs::a_pull_request_long_repo_name_stays_inside_the_panel` (160 characters). Fixed by REVIEWUI batch 3. |
| 11 | Open a per-repo Diff where the session's branch is 150 characters long. | The branch text runs on or is cut, with no ellipsis. (DS-46, DS-49) | `repo_head` passes the session's branch (`panel_changes.rs:321-326`) and draws it with no ellipsis (`:1450`). DS-49 wants each clone's own branch. | Fakes only. Built (DV-10): `diff_view.rs::each_repo_shows_its_own_clones_branch` and `::a_long_repo_and_branch_stay_inside_the_repo_row`. The branch is the clone's, with an ellipsis and a hover. |
| 12 | Open a PR review where a clone's branch is 150 characters long. | The branch is cut hard, with no "…". | `review_repo_head` draws the clone's own branch (`panel_changes.rs:1056-1060`), in a box with `min_w_0`, `overflow_hidden` and `whitespace_nowrap` but no `text_ellipsis` (`:1077-1085`). | Code, no test. `review_header.rs::review_repo_head` now ends the clone's branch with an ellipsis and carries the full text on hover (`full_text`). No test uses a long branch in a review. |
| 13 | Look at a file whose path is 150 characters long. | The file row cuts the path, with no ellipsis. Two long paths that differ only at the end read the same. (DS-47) | The file name is `min_w_0`, `overflow_hidden`, `whitespace_nowrap`, and has no `text_ellipsis` (`panel_changes.rs:67-140`, `file_row`). DS-47 wants the folder cut first. | Fakes only. Built (DV-10): `diff_view.rs::two_long_paths_that_differ_in_the_file_name_both_show_it`. |
| 14 | Open the base menu on the 220 pt panel. | The menu opens under the chip. It may run past the panel's right edge. (DS-50) | `base_menu` is deferred, with no right-edge anchor (`panel_changes.rs:1285`). The report says it is clipped at the panel edge. | Fakes only for a repo's menu (RV-7): `review_ui.rs::the_base_menu_stays_inside_a_narrow_panel`. The header's menu (`review_header.rs::base_menu`) is anchored the same way (`.right(0)`) with no test. |
| 15 | With one draft comment, look at the footer at 220 pt. | "Send 1 comment" and "Push & PR…" stay whole. The draft count may shrink. (DS-45, DS-55) | Footer buttons are `action()`, which is `flex_shrink_0` (`panels.rs:64-90`). | Fakes only: `desktop/tests/native/diff_view.rs::a_long_file_list_leaves_room_for_the_code` (the Send button sits inside the footer). The footer buttons are `flex_shrink_0`; the draft count shrinks. |
| 16 | Hover ⤢ or ✕. | A tooltip names the button and its key ("Full screen ⌥⌘↩", "Close panel"). (DS-17, DS-79) | `icon_action` sets `accessibility_label` only, with no tooltip (`ui/src/panels.rs:92-111`). The label reads "Expand panel", not "Full screen" (`panels.rs:157`). | Fakes only. `diff_full_screen.rs::the_full_screen_button_names_its_key` checks the Full screen and Exit full screen tooltips (⌃⌘↩). The ✕ tooltip "Close panel" is Code, no test. |

## Branches

### The base is a 150-character path

When I pick a base with a 150-character path, such as a long `origin/...` branch name joined with `+` terms. The chip shows as much of the start as fits and an ellipsis. ⤢ and ✕ stay inside the panel. The full base is in the accessibility label.
**I see:** the same header as step 1, with a longer base cut earlier.
**Expected:** the buttons are inside the panel. The chip is the only thing that shrinks.
**Status:** Code, no test: `ui/core/src/chrome.rs::panel_header` and `diff_view.rs::a_long_base_keeps_expand_and_close_inside_the_panel` (about 50 characters); no test uses a 150-character base.

### The window is narrowed while full screen is open

When I narrow the window with a long session title on screen in full screen. The title bar's names shorten, and **Show the terminal** keeps its place at the right.
**Expected:** **Show the terminal** is always on screen. Its names shorten first.
**Status:** Code, no test: `ui/core/src/chrome.rs::panel_header_actions`. No test narrows the window in full screen.

### The base shortens again

When I pick a short base after a long one. The chip grows back to fit the text, and the buttons do not move.
**Expected:** the chip's width follows its text up to the room left. The buttons stay put.
**Status:** Code, no test: `ui/changes/src/panel_changes/mod.rs::changes` (the chip's width follows its text).

## Expected outcomes

- [ ] At any panel width the panel can take, ⤢ (Full screen) and ✕ (Close) are inside the panel.
- [ ] A long base, PR line, repo name, branch or path shortens with an ellipsis and never pushes a button out.
- [ ] The full text of a shortened name is in its accessibility label, and on hover.
- [ ] In full screen, **Show the terminal** stays on screen, and the session and file names shorten first.
- [ ] A PR line shortens to "PR #482" before any other part of it is lost.
- [ ] A repo row's "+n −n" totals and chevron never shrink, however long the repo name or branch.
- [ ] The footer's Send and Push buttons stay whole, and the base menu stays inside the panel.

## Status table

Counts per tag. A step or branch counts once, under its tag.

| Tag | Steps (of 16) | Branches (of 3) | Total |
|---|---|---|---|
| Works (real shell) | 0 | 0 | 0 |
| Works (real daemon) | 0 | 0 | 0 |
| Fakes only | 11 (steps 1, 4, 5, 6, 9, 10, 11, 13, 14, 15, 16) | 0 | 11 |
| Code, no test | 4 (steps 2, 3, 8, 12) | 3 | 7 |
| Not built | 1 (step 7) | 0 | 1 |
| Likely bug | 0 | 0 | 0 |

## Gaps and likely bugs

- Not built: the PR line has no staged shrink to "PR #482" (step 7; DS-46). It ellipsizes from the right. No open task.
- Fixed: a 150-character repo name in a PR review's repo row now ellipsizes and keeps its whole name on hover (`review_header.rs::review_repo_head`, step 10; `diff_stress.rs::a_pull_request_long_repo_name_stays_inside_the_panel`).
- Fakes only: hovers. The file row's path (`diff_view.rs::a_rename_names_where_it_came_from`) and the full-screen title line (`diff_full_screen.rs::the_full_screen_title_line_keeps_its_whole_text`) carry `full_text`, as do the base chip, PR line, repo, branch and file-bar names (DS-46, DS-80).
- Fakes only: tooltips with the key on Full screen and Exit full screen (`diff_full_screen.rs::the_full_screen_button_names_its_key`, DS-17). The ✕ tooltip "Close panel" is Code, no test.
- Gap in tests: no 150-character base, session title or review repo name; no hover test beyond a repo's branch, the file row and the full-screen title line; the header's base menu is untested at 220 pt.
- Cleared: the double header (step 4), the repo-name push (step 9), the missing ellipsis on paths and branches (steps 11 and 13), the repo-menu clip (step 14). The "I see" and "State" cells in the table above describe the 2026-11-18 code; only the Status cells were re-audited.

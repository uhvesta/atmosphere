# Changed code reads cleanly: numbers, a sign, the code, quiet tints and no @@ noise.

Layer: Diff and review. Platforms: macOS desktop.

## Starting state

- `acme/api` has a session "Refunds" in `#payments`, with its Diff panel open beside the terminal.
- The changed file is `src/client.rs`. It has a removed line, an added line and a context line, with at least two hunks.
- `src/config.rs` is changed, and its last line has lost its newline.
- Wrap is on, the default.
- Nothing is in full screen, and no comment box is open.

## Reproduce headless

- Native, on fakes: `bazel test //apps/humata2/desktop:native_test --test_filter=clicking_a_diff_line_opens_the_box_under_it_and_comment_types_it_at_once` (`desktop/tests/native/review.rs`).
- Native, on fakes: `bazel test //apps/humata2/desktop:native_test --test_filter=wrapped_changes_align_the_gutter_and_switch_back_to_one_scrolling_row` (same file).
- Daemon, real: `bazel test //apps/humata2/daemon/review_diff:modes_test --test_filter=losing_the_final_newline_marks_the_right_line_on_the_right_side` (`daemon/review_diff/tests/modes.rs`).
- Not headless: the colours on a real screen, and how the tints look in light and dark. The native tests check positions and text, not colours. No windows on the owner's screen.
- Target look: mockup `workflow/milestones/mockups/diff-proposal.png`, sections 1 and 2. The tints and hunk bands in the mockup are what DS-20 to DS-22 describe.

## Steps

| # | I do | I see | State and events | Status |
|---|---|---|---|---|
| 1 | Open `src/client.rs` beside the terminal. | Each code line has a number on the left, then a sign ("+", "−" or nothing), then the code. Removed lines take the old number, all others the new one. (DS-19, DS-20) | The number is the gutter, `change-gutter-N`. Its number comes from `side_of` (`ui/src/panel_changes.rs:1978`). The sign is drawn in a `cell_width` column (`panel_changes.rs:2400-2415`). The test checks gutter alignment on a wrapped line, not the sign. | Fakes only: `desktop/tests/native/diff_view.rs::added_and_removed_code_starts_at_one_edge` (removed, added and context code share one edge); the gutter order by `::full_screen_has_two_number_columns`. |
| 2 | Click the number of an added line. | A comment box opens under that line. Typing and Send put the comment on that line. (DS-19, the click target) | `change-gutter-N` sends the comment target (`panel_changes.rs:2358-2372`). The box sits between that line and the next. | Fakes only: `desktop/tests/native/review.rs::clicking_a_diff_line_opens_the_box_under_it_and_comment_types_it_at_once`. |
| 3 | Shift-click a second line's number. | The box stretches over the range, from the first line to the second. | `shift` on the click extends the box's `first` to `last` (`panel_changes.rs:2369-2372`). | Fakes only: `desktop/tests/native/review.rs::shift_clicking_a_second_diff_line_stretches_the_box_to_a_range`. |
| 4 | Click the number of a removed line. | The box opens on the old side, and the comment says so. | `side_of` gives `Side::Old` for a removed line (`panel_changes.rs:1979-1980`). | Fakes only: `desktop/tests/native/review.rs::a_removed_line_takes_its_comment_on_the_old_side`. |
| 5 | Open the file in full screen. | Two number columns: the old number, then the new one. Context lines show both. (DS-19) | One number column only. `side_of` gives one number, and `gutter` draws one (`panel_changes.rs:2339-2372`). | Fakes only. Built (DV-17): `desktop/tests/native/diff_view.rs::full_screen_has_two_number_columns`; beside the terminal there is one column. |
| 6 | Look at added and removed rows in full screen. | Added rows get a green tint and removed rows a red one, about 12% light and 16% dark, full bleed. The gutter is tinted too, with no border between rows. (DS-20) | Row tints are the theme's solid `add` and `del` colours, not the spec's alpha values (`ui/src/theme.rs:72-73` dark, `:92-93` light). The row's `bg` covers the gutter because the gutter is a child of the row (`panel_changes.rs:2395-2400`). No test checks the colours. | Code, no test. Built (DV-17): `panel_changes/diff_body.rs::tint` lays the status green or red at 12% (light) or 16% (dark) over the code colour, full bleed; `::added_and_removed_code_starts_at_one_edge` checks the shared edge, not the colours or that the gutter is tinted. |
| 7 | Look at a removed line followed by an added line that changed a few words. | The changed words get a stronger tint of about 28%. Other text stays plain. (DS-21) | No word-level diff exists in `ui/src` or `clients/`. A grep for word or intraline highlighting finds nothing in the Diff code. | Fakes only. Built (DV-17): unit tests in `panel_changes/diff_body.rs` (`one_changed_word_is_the_span_on_both_lines`, `lines_mostly_different_have_no_changed_span`, `a_run_of_removed_lines_pairs_with_as_many_added_lines`, `runs_of_different_lengths_or_alone_have_no_partner`). No test draws the highlight, and the 28% tone is not asserted. |
| 8 | Look at a hunk header. | A dim band with only the section text, such as "impl Client {", or a thin line when there is none. No "@@" text. (DS-22) | The raw header is drawn as text: the daemon's `@@ ... @@` string, with no counts (`clients/live/src/diff.rs:448`, `daemon/review_diff/src/wire.rs:579`). The Hunk row draws `line.text` in `header` (`ui/src/panel_changes.rs:2302-2305`). DV-6, which added counts, was dropped. | Fakes only. Built (DV-17): `desktop/tests/native/diff_view.rs::a_hunk_header_draws_no_at_numbers`; unit `diff_body.rs::a_hunk_shows_only_its_section_text`. |
| 9 | Scroll past the second hunk. | The two hunks are separated by the dim band from step 8, with no "@@" numbers. | As step 8. | Fakes only. Built: same as step 8; hunks are the band in `diff_body.rs`. |
| 10 | Open `src/config.rs`, whose last line has lost its newline. | A dim row reads "No newline at end of file", with no number and no click target. (DS-23) | The daemon sets `no_newline` on the line (`daemon/review_diff/src/diff.rs:403`, `wire.rs:605`), and the daemon test checks it. The client drops it: `clients/src/model.rs`'s `DiffLine` has no such field, and nothing in `clients/` or `ui/src` reads `no_newline`. | Fakes only. Built (DV-11, DV-12): `desktop/tests/native/diff_view.rs::a_missing_final_newline_is_a_note_row_without_a_number`; the daemon half by `daemon/review_diff/tests/modes.rs`. |
| 11 | Turn on a screen reader and go to the code. | The code is a labelled region, named "acme/api/src/client.rs". File rows are options and say whether they are picked. (DS-81) | `Role::Code` with `aria_label(name)` on the code element (`panel_changes.rs:533 and 563`). File rows are `Role::ListBoxOption` with `aria_selected` (`panel_changes.rs:111`). | Code, no test. `Role::Code` with the file's name as its label (`review_header.rs::review_code`, `panel_changes/mod.rs::changes`); file rows are `ListBoxOption` with `aria_selected` (`file_list.rs::file_row`). |
| 12 | Turn Wrap off. | The number and the sign stay where they are. Only the code moves sideways. | The code is shifted left by the scroll offset, and the gutter and sign are not (`panel_changes.rs:2386-2394`). | Fakes only for a review: `pr_session.rs::scroll_in_a_review_keeps_the_gutter_still`; `review.rs::wrapped_changes_align_the_gutter_and_switch_back_to_one_scrolling_row`. The Diff panel's own sideways shift (`panel_changes/mod.rs::changes`) has no test. |
| 13 | Press ⌥Z. | Wrap and Scroll switch, when the diff has focus. ⌥Z does nothing in a text box. (DS-40) | No binding for ⌥Z. The spec says it binds only in the diff's own key context (§8, item 2). | Fakes only. Built (DV-17, RV-5): `diff_view.rs::the_wrap_switch_flips_long_lines`; `review_ui.rs::option_z_in_the_comment_box_types_and_does_not_flip_wrap` (⌥Z flips Wrap in the code and leaves a text box alone). ⌥Z is bound only in the diff's key context (`ui/core/src/keys.rs`). |
| 14 | Open a line with leading spaces in a PR review. | The sign moves into the code's first cell, so it never covers the code. The number column stays put. (DS-20) | `review && line.text.starts_with([' ', '\t'])` moves the sign one cell right (`panel_changes.rs:2412`). | Code, no test. Settled otherwise: the sign now has its own two-cell column before the code (DV-17, D861), so it never covers the code and the old leading-space shift is gone. |

## Branches

### A line too long for the panel wraps

When a line is longer than the panel. With Wrap on, the code wraps to more rows, and the number stays on its first row.
**I see:** the number on the first visual row only, the code wrapped below it, and the sign on the first row.
**Expected:** the number and sign line up with the first row of the wrapped line.
**Status:** Fakes only: `desktop/tests/native/review.rs::wrapped_changes_align_the_gutter_and_switch_back_to_one_scrolling_row`. It checks the gutter's alignment on a wrapped line.

### A line starts with a tab

When a line is indented with tabs. The code's columns stop every four cells, and the number and sign stay put.
**I see:** code aligned to four-cell stops, and the gutter unchanged.
**Expected:** `line_facts` counts the widest line in cells, with tabs at four-column stops, so the Scroll limit is right.
**Status:** Code, no test. `ui/changes/src/panel_changes/diff_body.rs::line_facts` counts the widest line with tabs at four-cell stops. No test passes a tab through the view.

### A removed line with an indented text is shown in a PR review

When a removed line starts with spaces in a PR review. The sign moves one cell right, so it never covers the code (step 14).
**Status:** Code, no test. Moot after DV-17: the sign has its own column, so it cannot cover indented code (step 14).

### The file loses its final newline

When the new side of a file has no final newline. The daemon marks the line, and the Diff should show the dim row from step 10.
**I see:** no dim row. The line looks like any other added line.
**Expected:** a dim row with no number, under the line that lost its newline.
**Status:** Fakes only. `desktop/tests/native/diff_view.rs::a_missing_final_newline_is_a_note_row_without_a_number` (the note row has no number). Built (DV-11, DV-12).

## Expected outcomes

- [ ] Each changed code line shows its number, its sign and its code, in that order.
- [ ] A click on a number opens a comment box under that line, and a shift-click extends it over a range.
- [ ] A removed line's comment goes on the old side.
- [ ] In full screen, each row has an old and a new number.
- [ ] Added and removed rows carry quiet tints, and the changed words in a changed pair stand out.
- [ ] A hunk header shows its section text in a dim band, with no "@@" text.
- [ ] A line that lost its newline gets a dim "No newline at end of file" row, with no number and no click target.
- [ ] With Wrap off, the number and sign stay put while the code scrolls.

## Status table

Counts per tag. A step or branch counts once, under its tag.

| Tag | Steps (of 14) | Branches (of 4) | Total |
|---|---|---|---|
| Works (real shell) | 0 | 0 | 0 |
| Works (real daemon) | 0 | 0 | 0 |
| Fakes only | 11 (steps 1, 2, 3, 4, 5, 7, 8, 9, 10, 12, 13) | 2 | 13 |
| Code, no test | 3 (steps 6, 11, 14) | 2 | 5 |
| Not built | 0 | 0 | 0 |
| Likely bug | 0 | 0 | 0 |

## Gaps and likely bugs

- Nothing is Not built or a Likely bug in this case after the audit. DV-11, DV-12 and DV-17 landed.
- Gap in tests: the tint and word-highlight colours, the tinted gutter, the sideways shift in the Diff panel (only a review's is tested), a tab in a line, and the light-mode frame have no assertion.
- The "I see" and "State" cells above describe the 2026-11-18 code (one number column, `@@` text, no ⌥Z); only the Status cells and the counts were re-audited.

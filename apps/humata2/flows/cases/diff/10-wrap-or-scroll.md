# I switch long lines between Wrap and Scroll from the file I'm reading, and Scroll keeps the numbers still

Layer: Diff and review. Platforms: macOS desktop.

Covers DS-40, DS-41, DS-42, DS-43, DS-44, DS-51, DS-75, P-6 in
`workflow/milestones/DIFFVIEW-spec.md`, and the ⌥Z conflict in spec §8 item 2.
Mockup: `review` §3. Frames: native `changes-wrapped-long-line` and
`changes-wrapped-long-line-dark`.

Note: DV-5 (virtual rows) is in progress on `ui/src/panel_changes.rs` and
`ui/src/document_tab.rs`. Every line number here is from 2026-11-18. Recheck
them after DV-5 lands.

## Starting state

- Machine `box-1` runs the session "Refunds" in channel #payments, on repo `acme/api`.
- The Diff panel is open beside the terminal on `src/ledger/shard.rs`. The file has a line of about 300 characters, and one line of 194,000 characters in a second file, `src/ledger/dump.rs`.
- Wrap is the setting (the app default, `features/settings/src/lib.rs:148`).
- Focus is in the diff, not in the terminal.
- A PR review session on `acme/api` #482 is open in a second window of the test app, for step 7.

## Reproduce headless

- `bazel test //apps/humata2/desktop:native_test --test_filter=wrapped_changes` runs `desktop/tests/native/review.rs::wrapped_changes_align_the_gutter_and_switch_back_to_one_scrolling_row`. It draws the wrapped file, checks the gutter, toggles back to one scrolling row, and writes the two frames.
- `bazel test //apps/humata2/desktop:native_test --test_filter=widening_a_wrapped` runs `review.rs::widening_a_wrapped_diff_keeps_its_top_line_in_place`.
- `bazel test //apps/humata2/desktop:native_test --test_filter=a_huge_line_wraps` runs `desktop/tests/native/diff_big.rs::a_huge_line_wraps_in_a_long_file`. It wraps a 50,000-character line; the spec's 194,000 is not in the test, and the test does not time the wrap.
- `bazel test //apps/humata2/desktop:native_test --test_filter=long_lines_switch` runs `desktop/tests/native/document.rs::long_lines_switch_the_open_source_between_wrap_and_scroll`, for the open source file, not the diff.
- Not headless: the feel of the Scroll shift under a trackpad, and the 500 ms wrap budget (P-6). The headless tests draw frames; they do not time them.
- The tests are headless. None opens a window on the owner's screen.

## Steps

| # | I do | I see | State and events | Status |
|---|---|---|---|---|
| 1 | Open `src/ledger/shard.rs` in the Diff panel. | The long line breaks under the code. Continuation rows have no line number. The gutter lines up with the first row. (DS-41) | `review.wrap` is on. The default comes from `features/settings/src/lib.rs:148` (`wrap_diffs: true`), copied to `review.wrap` by `features/app/src/lib.rs:1213-1216`. The row wrap uses `ui/src/panel_changes.rs:2346` (`code_wrap::wrapped`). | Fakes only: `desktop/tests/native/review.rs::wrapped_changes_align_the_gutter_and_switch_back_to_one_scrolling_row`. |
| 2 | Open Settings, under Long lines, and pick **Scroll**. | The Settings tab **Scroll** is selected (`settings-long-lines-Scroll`). The Diff panel stops wrapping. (DS-75) | The settings click sends `SettingsAction::ToggleWrap` (`ui/src/settings.rs:515-545`). The reducer flips `preferences.wrap_diffs` (`features/settings/src/lib.rs:503`). The app copies the new value into `review.wrap` (`features/app/src/lib.rs:1213-1216`). | Fakes only: `desktop/tests/native/document.rs::long_lines_switch_the_open_source_between_wrap_and_scroll`; and `diff_view.rs::the_wrap_switch_flips_long_lines` asserts the Diff panel follows the preference. |
| 3 | Look for a Wrap / Scroll switch in the Diff panel's file header. | No switch there. The only control is in Settings. (DS-40) | `ReviewAction::ToggleWrap` exists (`features/review/src/lib.rs:934`, the field at `:208`). No code in `ui/src` sends it. `grep` for `ReviewAction::ToggleWrap` in `ui/src` finds nothing. | Fakes only. Built (DV-17): the `diff-wrap` switch in the file header sends the Settings action, so the preference and `review.wrap` flip together (`desktop/tests/native/diff_view.rs::the_wrap_switch_flips_long_lines`). `file_list.rs::wrap_switch`. |
| 4 | With the diff focused, press ⌥Z. | Nothing happens. There is no binding for ⌥Z in `ui/src/keys.rs`. (DS-40, §3) | The spec says ⌥Z is bound only in the diff's own key context, never app-wide (§8 item 2). No such context binding exists yet. | Fakes only. Built (DV-17, RV-5): ⌥Z is bound in the diff's key context only (`ui/core/src/keys.rs`, `alt-z` with `DIFF`; `Input` rebinds it to nothing). `desktop/tests/native/review_ui.rs::option_z_in_the_comment_box_types_and_does_not_flip_wrap`. |
| 5 | In Scroll, look at a file with a line longer than the panel. | The line number and the +/− sign stay where they are. Only the code column slides sideways, as one block for the whole file. (DS-42) | `max_shift` (`ui/src/panel_changes.rs:484-517`) is the widest line less the view. It is applied where the code column is drawn (`:2383-2391`): with Scroll, the code column is `overflow_hidden` and the gutter stays out of the shift. DV-5 is in progress on this code. | Code, no test for the Diff panel's own shift (`panel_changes/mod.rs::changes`, `max_shift`). A review's is tested: `desktop/tests/native/pr_session.rs::scroll_in_a_review_keeps_the_gutter_still`. |
| 6 | In Scroll, scroll the code column to its far right. | It stops at the widest line, not past it. | The shift is capped by `max_shift` (`panel_changes.rs:484-517`). | Code, no test. The shift is capped by `max_shift` (the widest line less the view) in `panel_changes/mod.rs::changes` and `review_header.rs::review_code`. |
| 7 | Open a PR review session (`acme/api` #482) with Scroll chosen in Settings. Find a long line. | The long line wraps anyway. The review does not follow the Scroll choice. (DS-75, DS-42) | `ui/src/panel_changes.rs:2386` draws `if self.wrap || review`, so a review always wraps. DS-75 says Wrap is one app-wide preference. | Fakes only. Built (RV-2): a review follows the Wrap choice now (`|| review` is gone): `pr_session.rs::scroll_in_a_review_keeps_the_gutter_still` turns Wrap off and checks the number stays put. |
| 8 | Open the 194,000-character line in `src/ledger/dump.rs` with Wrap on. | The first rows show, then the wrapped line. The app does not freeze. (DS-43) | The row wrap is computed for the rows in view. The report measured about 2 s today (finding 5). P-6 proposes under 500 ms after DV-5. | Fakes only. `desktop/tests/native/diff_big.rs::a_huge_line_wraps_in_a_long_file` (a 50,000-character line, first two rows). Measured by hand (P-6): 85 ms for 194,000 characters against 500 ms (`review_ui.rs::budgets::wrapping_a_194_000_character_line`, ignored). |
| 9 | Widen the Diff panel while a wrapped file is open. | The top visible line stays where it was. (DS-44) | The scroll keeps the top line across a rewrap (`ui/src/panel_changes.rs:524-525`, `:579`). | Fakes only. `desktop/tests/native/review.rs::widening_a_wrapped_diff_keeps_its_top_line_in_place`. |
| 10 | Go full screen and back, under Wrap. | The top line stays in place. (DS-44) | The spec cites `ui/src/document_tab.rs:439` for the top-line keep. Full screen is DV-18. | Code, no test under Wrap. `review_ui.rs::show_the_terminal_keeps_the_line_scroll_drafts_and_open_box` keeps the scroll line over a full-screen round trip but does not turn Wrap on (DS-44). |
| 11 | Open a source file (not a diff) with a long line, and flip the setting. | The open source wraps or scrolls to match the setting. | The test flips the setting by `AppAction::Settings(ToggleWrap)` (`desktop/tests/native/document.rs` near line 408). I found no switch in the document tab's file header. | Fakes only. `desktop/tests/native/document.rs::long_lines_switch_the_open_source_between_wrap_and_scroll`. |
| 12 | Look at a file with Unicode and emoji names, CRLF line endings and minified lines. | They draw correctly. (DS-51) | The row wrap handles grapheme clusters (`ui/src/code_wrap.rs`, with its own unit test). The rest is the live run, not a test. | Fakes only, bounds and frames: `desktop/tests/native/diff_stress.rs::unicode_paths_stay_inside_the_panel`, `::crlf_lines_stay_inside_the_panel`, `::a_minified_line_wrapped_stays_inside_the_panel`, `::a_minified_line_unwrapped_stays_inside_the_panel` (DV-13). No test reads the glyphs. |

## Branches

### I press ⌥Z where it types into the terminal or a text box

When my focus is in the terminal, or in a text box such as the chat composer, and I press ⌥Z.
I see the character Ω typed. The diff does not change.
Expected: ⌥Z changes nothing in a text box or in the terminal, because it is bound only in the diff's own key context (§8 item 2).
Why: the terminal's option key is not Alt. `terminal/vt/src/terminal.rs:223` sets `option_as_alt: OptionAsAlt::Off`, so ⌥Z sends Ω. The keyboard reference is `terminal/view/src/keys.rs`.
**Status:** Fakes only for a text box: `review_ui.rs::option_z_in_the_comment_box_types_and_does_not_flip_wrap`. Code, no test for the terminal (`terminal/vt/src/terminal.rs` sets `option_as_alt: Off`, so ⌥Z types Ω there; the diff's binding is not active with the terminal focused).

### The app restarts with Scroll chosen

When I quit and open the app again after step 2.
I see the Diff panel still in Scroll, with the numbers still.
Expected: the Wrap choice is kept with the rest of the preferences.
Why: the app has a `SaveDue` path (`features/app/src/lib.rs`). I did not confirm that it saves `wrap_diffs`, and I found no test that reads `wrap_diffs` back after a restart.
**Status:** Fakes only. `features/app/tests/presentation.rs::long_lines_survive_a_restart_both_ways`.

### The diff switch and the Settings switch disagree

When I flip Wrap in Settings, and then a test or a future control flips `review.wrap` directly with `ReviewAction::ToggleWrap`.
I see the diff and the Settings tab disagree, because `ToggleWrap` on the review changes only `review.wrap` and not `preferences.wrap_diffs`.
Expected: one app-wide preference (DS-75) that the Settings tab and the diff both show.
Why: the test `desktop/tests/native/diff_big.rs::a_huge_line_wraps_in_a_long_file` and the wrap test send `AppAction::Review(R::ToggleWrap)`, which bypasses the Settings reducer (`features/app/src/lib.rs:1213-1216` is the only place that copies the setting into the review).
**Status:** Fakes only. Cleared: the switch sends the Settings action, and `diff_view.rs::the_wrap_switch_flips_long_lines` asserts the preference and `review.wrap` flip together.

## Expected outcomes

- [ ] A long line in the Diff panel wraps under the code, and its continuation rows have no number (DS-41).
- [ ] Scroll keeps the line numbers and the +/− signs still, and only the code column slides sideways (DS-42).
- [ ] The Scroll shift stops at the widest line in the file (DS-42).
- [ ] Wrap and Scroll are one app-wide preference, shown the same in Settings and in the diff (DS-75).
- [ ] A PR review follows the Wrap or Scroll choice (DS-75, DS-42).
- [ ] A 194,000-character line wraps without freezing the app (DS-43). Budget: under 500 ms (P-6, proposed; not measured yet).
- [ ] Widening the panel, or going full screen and back, keeps the top line in place under Wrap (DS-44).
- [ ] Wrap or Scroll can be switched from the file header, and ⌥Z does it only in the diff's own key context (DS-40, §8 item 2).
- [ ] Unicode, CRLF and minified lines draw correctly (DS-51).

## Status table

Counts per tag: Works (real shell) 0; Works (real daemon) 0; Fakes only 12;
Code, no test 3; Not built 0; Likely bug 0. Total 15 rows: 12 steps and 3
branches.

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Wrap, gutter aligned | Fakes only | `review.rs::wrapped_changes_align_the_gutter_and_switch_back_to_one_scrolling_row` |
| 2 Pick Scroll in Settings | Fakes only | `document.rs::long_lines_switch_the_open_source_between_wrap_and_scroll`; `diff_view.rs::the_wrap_switch_flips_long_lines` |
| 3 Switch in the file header | Fakes only | `diff_view.rs::the_wrap_switch_flips_long_lines` |
| 4 ⌥Z in the diff | Fakes only | `review_ui.rs::option_z_in_the_comment_box_types_and_does_not_flip_wrap` |
| 5 Scroll keeps numbers still | Code, no test | Diff panel: `panel_changes/mod.rs::changes`; review: `pr_session.rs::scroll_in_a_review_keeps_the_gutter_still` |
| 6 Scroll stops at widest line | Code, no test | `panel_changes/mod.rs::changes` (`max_shift`) |
| 7 PR review follows Scroll | Fakes only | `pr_session.rs::scroll_in_a_review_keeps_the_gutter_still` |
| 8 194,000-character line | Fakes only | `diff_big.rs::a_huge_line_wraps_in_a_long_file`; P-6 85 ms by `review_ui.rs::budgets::` |
| 9 Widen keeps top line | Fakes only | `review.rs::widening_a_wrapped_diff_keeps_its_top_line_in_place` |
| 10 Full screen and back | Code, no test | `review_ui.rs::show_the_terminal_keeps_the_line_scroll_drafts_and_open_box` (Wrap not turned on) |
| 11 Open source flips | Fakes only | `document.rs::long_lines_switch_the_open_source_between_wrap_and_scroll` |
| 12 Unicode and CRLF | Fakes only | `diff_stress.rs::unicode_paths_stay_inside_the_panel`, `::crlf_lines_stay_inside_the_panel` (bounds) |
| ⌥Z in a text box or the terminal | Fakes only | `review_ui.rs::option_z_in_the_comment_box_types_and_does_not_flip_wrap` (the terminal half has no test) |
| Restart keeps Scroll | Fakes only | `features/app/tests/presentation.rs::long_lines_survive_a_restart_both_ways` |
| Diff and Settings disagree | Fakes only | `diff_view.rs::the_wrap_switch_flips_long_lines` (cleared) |

## Gaps and likely bugs

- Nothing is Not built or a Likely bug in this case after the audit. DV-17, RV-2 and RV-5 landed.
- Gap in tests: the Diff panel's own sideways shift and its stop at the widest line; full screen and back under Wrap (DS-44); ⌥Z with the terminal focused; the 194,000-character line is only in an ignored timing test.
- The step narratives above (no switch, no ⌥Z, a review always wraps) describe the 2026-11-18 code; only the Status cells and the counts were re-audited.

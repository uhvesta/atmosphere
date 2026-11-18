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
| 1 | Open `src/ledger/shard.rs` in the Diff panel. | The long line breaks under the code. Continuation rows have no line number. The gutter lines up with the first row. (DS-41) | `review.wrap` is on. The default comes from `features/settings/src/lib.rs:148` (`wrap_diffs: true`), copied to `review.wrap` by `features/app/src/lib.rs:1213-1216`. The row wrap uses `ui/src/panel_changes.rs:2346` (`code_wrap::wrapped`). | Fakes only. `desktop/tests/native/review.rs::wrapped_changes_align_the_gutter_and_switch_back_to_one_scrolling_row`. |
| 2 | Open Settings, under Long lines, and pick **Scroll**. | The Settings tab **Scroll** is selected (`settings-long-lines-Scroll`). The Diff panel stops wrapping. (DS-75) | The settings click sends `SettingsAction::ToggleWrap` (`ui/src/settings.rs:515-545`). The reducer flips `preferences.wrap_diffs` (`features/settings/src/lib.rs:503`). The app copies the new value into `review.wrap` (`features/app/src/lib.rs:1213-1216`). | Fakes only. The Settings path is tested by `desktop/tests/native/document.rs::long_lines_switch_the_open_source_between_wrap_and_scroll` (it sends the same action), and the diff's switch back by `review.rs::wrapped_changes_...`. The click on the Settings tab itself is not tested. |
| 3 | Look for a Wrap / Scroll switch in the Diff panel's file header. | No switch there. The only control is in Settings. (DS-40) | `ReviewAction::ToggleWrap` exists (`features/review/src/lib.rs:934`, the field at `:208`). No code in `ui/src` sends it. `grep` for `ReviewAction::ToggleWrap` in `ui/src` finds nothing. | Not built. DV-17. |
| 4 | With the diff focused, press ⌥Z. | Nothing happens. There is no binding for ⌥Z in `ui/src/keys.rs`. (DS-40, §3) | The spec says ⌥Z is bound only in the diff's own key context, never app-wide (§8 item 2). No such context binding exists yet. | Not built. DV-17. |
| 5 | In Scroll, look at a file with a line longer than the panel. | The line number and the +/− sign stay where they are. Only the code column slides sideways, as one block for the whole file. (DS-42) | `max_shift` (`ui/src/panel_changes.rs:484-517`) is the widest line less the view. It is applied where the code column is drawn (`:2383-2391`): with Scroll, the code column is `overflow_hidden` and the gutter stays out of the shift. DV-5 is in progress on this code. | Code, no test. DV-5 in progress; recheck. No headless test scrolls the code column. |
| 6 | In Scroll, scroll the code column to its far right. | It stops at the widest line, not past it. | The shift is capped by `max_shift` (`panel_changes.rs:484-517`). | Code, no test. DV-5 in progress; recheck. |
| 7 | Open a PR review session (`acme/api` #482) with Scroll chosen in Settings. Find a long line. | The long line wraps anyway. The review does not follow the Scroll choice. (DS-75, DS-42) | `ui/src/panel_changes.rs:2386` draws `if self.wrap || review`, so a review always wraps. DS-75 says Wrap is one app-wide preference. | Not built (REVIEWUI RV-2). `panel_changes.rs:2386` wraps review rows on purpose today; the spec asks for Scroll there too. Set Scroll, open a PR review with a line over 200 characters, and its rows wrap. |
| 8 | Open the 194,000-character line in `src/ledger/dump.rs` with Wrap on. | The first rows show, then the wrapped line. The app does not freeze. (DS-43) | The row wrap is computed for the rows in view. The report measured about 2 s today (finding 5). P-6 proposes under 500 ms after DV-5. | Fakes only. `desktop/tests/native/diff_big.rs::a_huge_line_wraps_in_a_long_file` checks the first two rows of a 50,000-character line. It does not time it, and it does not use 194,000 characters. P-6 is not measured. |
| 9 | Widen the Diff panel while a wrapped file is open. | The top visible line stays where it was. (DS-44) | The scroll keeps the top line across a rewrap (`ui/src/panel_changes.rs:524-525`, `:579`). | Fakes only. `desktop/tests/native/review.rs::widening_a_wrapped_diff_keeps_its_top_line_in_place`. |
| 10 | Go full screen and back, under Wrap. | The top line stays in place. (DS-44) | The spec cites `ui/src/document_tab.rs:439` for the top-line keep. Full screen is DV-18. | Code, no test. The spec says full screen and back is not tested (DV-18). |
| 11 | Open a source file (not a diff) with a long line, and flip the setting. | The open source wraps or scrolls to match the setting. | The test flips the setting by `AppAction::Settings(ToggleWrap)` (`desktop/tests/native/document.rs` near line 408). I found no switch in the document tab's file header. | Fakes only. `desktop/tests/native/document.rs::long_lines_switch_the_open_source_between_wrap_and_scroll`. |
| 12 | Look at a file with Unicode and emoji names, CRLF line endings and minified lines. | They draw correctly. (DS-51) | The row wrap handles grapheme clusters (`ui/src/code_wrap.rs`, with its own unit test). The rest is the live run, not a test. | Code, no test. The report's "Stress cases" passed in a live run; the frames are DV-13. |

## Branches

### I press ⌥Z where it types into the terminal or a text box

When my focus is in the terminal, or in a text box such as the chat composer, and I press ⌥Z.
I see the character Ω typed. The diff does not change.
Expected: ⌥Z changes nothing in a text box or in the terminal, because it is bound only in the diff's own key context (§8 item 2).
Why: the terminal's option key is not Alt. `terminal/vt/src/terminal.rs:223` sets `option_as_alt: OptionAsAlt::Off`, so ⌥Z sends Ω. The keyboard reference is `terminal/view/src/keys.rs`.
**Status:** Code, no test. `terminal/vt/src/terminal.rs:223`. No test checks that ⌥Z reaches the terminal as Ω.

### The app restarts with Scroll chosen

When I quit and open the app again after step 2.
I see the Diff panel still in Scroll, with the numbers still.
Expected: the Wrap choice is kept with the rest of the preferences.
Why: the app has a `SaveDue` path (`features/app/src/lib.rs`). I did not confirm that it saves `wrap_diffs`, and I found no test that reads `wrap_diffs` back after a restart.
**Status:** Code, no test.

### The diff switch and the Settings switch disagree

When I flip Wrap in Settings, and then a test or a future control flips `review.wrap` directly with `ReviewAction::ToggleWrap`.
I see the diff and the Settings tab disagree, because `ToggleWrap` on the review changes only `review.wrap` and not `preferences.wrap_diffs`.
Expected: one app-wide preference (DS-75) that the Settings tab and the diff both show.
Why: the test `desktop/tests/native/diff_big.rs::a_huge_line_wraps_in_a_long_file` and the wrap test send `AppAction::Review(R::ToggleWrap)`, which bypasses the Settings reducer (`features/app/src/lib.rs:1213-1216` is the only place that copies the setting into the review).
**Status:** Code, no test. Nothing in `ui/src` sends `ReviewAction::ToggleWrap` today (coach-checked), so the two can't disagree yet. When DV-17 adds the file-header switch, it must send the Settings action (`features/app/src/lib.rs:1213-1216`), not `ReviewAction::ToggleWrap` (`features/review/src/lib.rs:934`).

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

Counts per tag: Works (real shell) 0; Works (real daemon) 0; Fakes only 5;
Code, no test 7; Not built 3; Likely bug 0. Total 15 rows: 12 steps and 3
branches.

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Wrap, gutter aligned | Fakes only | `review.rs::wrapped_changes_align_the_gutter_and_switch_back_to_one_scrolling_row` |
| 2 Pick Scroll in Settings | Fakes only | `document.rs::long_lines_switch_the_open_source_between_wrap_and_scroll`; Settings tab click untested |
| 3 Switch in the file header | Not built | DV-17; `features/review/src/lib.rs:934` has no sender |
| 4 ⌥Z in the diff | Not built | DV-17; no ⌥Z in `ui/src/keys.rs` |
| 5 Scroll keeps numbers still | Code, no test | `panel_changes.rs:484-517`, `:2383-2391`; DV-5 in progress |
| 6 Scroll stops at widest line | Code, no test | `panel_changes.rs:484-517`; DV-5 in progress |
| 7 PR review ignores Scroll | Not built | `panel_changes.rs:2386` (RV-2) |
| 8 194,000-character line | Fakes only | `diff_big.rs::a_huge_line_wraps_in_a_long_file`; P-6 not timed |
| 9 Widen keeps top line | Fakes only | `review.rs::widening_a_wrapped_diff_keeps_its_top_line_in_place` |
| 10 Full screen and back | Code, no test | `document_tab.rs:439`; DV-18 |
| 11 Open source flips | Fakes only | `document.rs::long_lines_switch_the_open_source_between_wrap_and_scroll` |
| 12 Unicode and CRLF | Code, no test | `code_wrap.rs`; live run only |
| ⌥Z types Ω in terminal | Code, no test | `terminal/vt/src/terminal.rs:223` |
| Restart keeps Scroll | Code, no test | `SaveDue` path in `features/app/src/lib.rs` |
| Diff and Settings disagree | Code, no test | `features/review/src/lib.rs:934`; `features/app/src/lib.rs:1213-1216` |

## Gaps and likely bugs

- **Not built, DV-17:** the Wrap / Scroll switch in the Diff panel's file header (DS-40). Settings is the only control. The spec's mockup `review` §3 draws the switch in the file header.
- **Not built, DV-17:** ⌥Z. The spec settles it as bound only in the diff's key context (§8 item 2). Nothing binds it yet. `ReviewAction::ToggleWrap` (`features/review/src/lib.rs:934`) has no UI sender.
- **Not built (RV-2):** a PR review always wraps, `if self.wrap || review` (`ui/src/panel_changes.rs:2386`). That breaks DS-75 (one app-wide preference) and DS-42 (Scroll keeps the numbers still in the review too). Confirm by setting Scroll and opening a review with a long line.
- **Risk for DV-17, not a bug today:** two sources of the Wrap choice. Settings changes `preferences.wrap_diffs`, and the diff reads `review.wrap`, which the app copies only on the Settings action (`features/app/src/lib.rs:1213-1216`). `ReviewAction::ToggleWrap` changes `review.wrap` alone (`features/review/src/lib.rs:934`). Tests use the review action, so they never check the setting.
- **Not proven:** P-6, the 194,000-character wrap in under 500 ms. The test wraps 50,000 characters and does not time it. The report measured about 2 s (finding 5). Measure it again after DV-5, as the spec asks.
- **Not built, DV-18:** the top line kept through full screen and back (DS-44). The code is at `ui/src/document_tab.rs:439`; no test covers it.
- **Code, no test, DV-5 in progress:** the Scroll shift (DS-42). `ui/src/panel_changes.rs:484-517` and `:2383-2391`. No headless test scrolls the code column. Add one after DV-5 lands, and recheck these lines.
- **Terminal conflict, settled but not built:** `terminal/vt/src/terminal.rs:223` sets `option_as_alt: OptionAsAlt::Off`, so ⌥Z types Ω in the terminal and in text boxes. Bind ⌥Z only in the diff's key context, as §8 item 2 says.

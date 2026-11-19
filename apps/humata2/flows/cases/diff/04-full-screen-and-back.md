# Full screen opens three ways and Show the terminal puts me back exactly where I was, with the terminal still running

Layer: Diff and review. Platforms: macOS desktop.

## Starting state

- Machine `box-1`, channel `#payments`, session "Refunds" with an agent running in its terminal. Repos `acme/api` and `acme/ledger`.
- The Diff panel is open beside the terminal. The file `acme/api` › `src/client.rs` is open. Its line 140 is the focused line, and the code is scrolled to about line 120.
- Wrap is off. A comment box is open on line 43 with the draft "Keep these writes in one transaction." (not sent).
- A second session, "Deploy", runs in `#infra`, with its own panel closed.
- Nothing is in full screen.

## Reproduce headless

- Fakes: `bazel test //apps/humata2/desktop:native_test --test_filter=show_the_terminal_returns_the_review_beside_the_terminal` (`desktop/tests/native/pr_session.rs`: full screen on a PR review, then back, with the same file kept). This test checks only `saved.selected`, not scroll, focus or drafts.
- Fakes: `bazel test //apps/humata2/desktop:native_test --test_filter=a_narrow_review_panel_keeps_full_screen_and_close` (the ⤢ and ✕ buttons fit a 220 pt panel).
- Fakes: `bazel test //apps/humata2/desktop:native_test --test_filter=terminal_and_panel_full_screen_return_to_the_same_columns` (`desktop/tests/native/shell.rs`, the Board panel: the layout is the same after the round trip).
- Fakes: `bazel test //apps/humata2/desktop:native_test --test_filter=full_screen_draws_each_repository_base_too` (`desktop/tests/native/targets.rs`).
- Fakes: `bazel test //apps/humata2/desktop:native_test --test_filter=widening_a_wrapped_diff_keeps_its_top_line_in_place` (`desktop/tests/native/review.rs`: a width change under Wrap keeps the top line in place).
- Fakes: `bazel test //apps/humata2/features/app/...` with `--test_filter=closing_the_full_screen_panel_returns_to_the_terminal` (`features/app/tests/presentation.rs`).
- Frames: `desktop/tests/__snapshots__/native/11-panel-full-screen-diff.view.txt`, and the PR review frames `68b-pr-review-full-screen` and `68c-pr-review-after-show-terminal` (PNG and view text).
- Not headless: that the terminal process keeps running is proven by the lab tests (case micro/17), not by any full-screen test. The real window's feel is not checked. No windows on the owner's screen.

## Steps

1. **I do:** press the ⤢ button at the right of the Diff header.
   **I see:** the panel fills the window. The terminal goes away. The title bar is at the top. Only this press makes it full screen: nothing switches by itself, however wide the content (DS-18, Code, no test: no other sender of `ToggleWiden` in `ui/src`).
   **State and events:** `SessionAction::ToggleWiden` → `AppAction::ToggleFullScreen(FullScreen::Panel)` (`features/app/src/lib.rs:1153-1180`). `full_screens` for this session is set to `Panel` (`:356-366`). `widened` is set on the session view.
   **Status:** Fakes only. DS-12 is Built: the button (`targets.rs::full_screen_draws_each_repository_base_too` clicks `expand-panel`), ⌃⌘↩ and a double-click (steps 7 and 8). Nothing else sends `ToggleWiden` (DS-18, no test).

2. **I do:** look at the title bar, which is 52 pt tall.
   **I see:** the window lights, a panel icon, "# payments › Refunds" (ellipsized if long), "Diff" beside it in larger text, and "Show the terminal" at the right.
   **State and events:** `ui/src/panels.rs:250-360`, the `when(full)` branch: a 52 pt row (`:255-262`), the names in a column that shrinks and ellipsizes, and `show-terminal` (`:343-350`).
   **Status:** Fakes only. Built (DV-18, D860): `diff_full_screen.rs::full_screen_draws_one_header` (one 52 pt bar, the base chip inside it). The button reads "Exit full screen" now (D848), not Show the terminal.

3. **I do:** look below the title bar.
   **I see:** a second "Diff" header row, with its own "Diff" text, its own base chip "Against main", and its own "Show the terminal" button and "Close panel" button, right under the title bar. So there are two "Diff" titles and two "Show the terminal" buttons on one screen.
   **State and events:** the body draws `header(...)` again with `full` true (`ui/src/panel_changes.rs:622` for a local diff and `:695` for a review). The header's button is still id `expand-panel`, but its label is "Show the terminal" (`ui/src/panels.rs:157-161`). The spec says the panel's own 36 pt header is not drawn in full screen (DS-14, DV-18).
   **Status:** Fakes only. Likely bug cleared (DV-18): `diff_full_screen.rs::full_screen_draws_one_header` asserts there is no `panel-header` and no `expand-panel` in full screen, and one `show-terminal`. The narrative above describes the old snapshot.

4. **I do:** look at the file list and the code.
   **I see:** the list on the left, about 300 pt wide, with a border on its right. The code fills the rest, down to the footer, with no gap.
   **State and events:** in full screen the list is `w(300)`, `flex_shrink_0`, with a right border (`ui/src/panel_changes.rs:728-735`).
   **Status:** Code, no test. `panel_changes/mod.rs::changes` makes the list 300 pt with a right border in full screen. `diff_view.rs::the_file_list_scrolls_in_the_expanded_panel` checks only that the list is taller than 220 pt and inside the window.

5. **I do:** switch to the light theme and look at the empty part of the code column.
   **I see:** in light mode, the code column shows a grey band where the content colour should run from top to bottom (the light-mode grey void).
   **State and events:** DS-15 says the code fills from top to bottom in the content colour.
   **Status:** Code, no test. Built (DV-17): the code area takes the content colour end to end (`panel_changes/mod.rs::changes`, DS-15). No test or light-mode frame checks the grey void is gone.

6. **I do:** look at the footer in full screen.
   **I see:** the same footer as beside the terminal: a draft line at the left ("1 draft comment") and the "Send 1 comment" button at the right. In a pull request review, "Submit review…" should be here too (DS-16), and it is not.
   **State and events:** the footer is the same code (`ui/src/panel_changes.rs:745-792`). Submit review is in the session bar, not the footer (spec §8 item 4).
   **Status:** Code, no test. The footer is one function for both widths and carries `diff-submit-review` in a PR review (`panel_changes/mod.rs::changes`). `diff_view.rs::a_pull_request_review_footer_has_submit_review` runs beside the terminal only; no test opens it in full screen.

7. **I do:** press ⌥⌘↩.
   **I see:** nothing happens.
   **State and events:** there is no `alt-cmd-enter` binding in `ui/src/keys.rs` (the bindings at `:198-237`). ⇧⌘↩ is the terminal's full screen (`keys.rs`, `TerminalFullScreen`), not the panel's.
   **Status:** Fakes only. Built as ⌃⌘↩ (`ctrl-cmd-enter`; DIFFVIEW lane 17:05, because off a Mac ⌥⌘↩ clashes with ⇧⌘↩): `diff_full_screen.rs::alt_cmd_enter_enters_and_leaves_full_screen`.

8. **I do:** double-click the Diff header.
   **I see:** nothing happens.
   **State and events:** no double-click handler on the panel header (`ui/src/panels.rs` has none).
   **Status:** Fakes only. Built: `diff_full_screen.rs::double_clicking_the_panel_header_enters_full_screen`.

9. **I do:** hover the ⤢ button in the narrow panel, or the "Show the terminal" button in full screen.
   **I see:** no tooltip. The label for accessibility reads "Expand panel" in the narrow panel and "Show the terminal" in full screen.
   **State and events:** `icon_action` sets no tooltip (`ui/src/panels.rs:92-111`). The spec wants "Full screen ⌥⌘↩" for the panel and "⇧⌘↩" for the session's header (DS-17).
   **Status:** Fakes only. Built: the ⤢ tooltip "Full screen  ⌃⌘↩" (`ui/core/src/chrome.rs::panel_header`), Exit full screen (`panel_action`) and the session header's terminal arrow (`ui/src/pages.rs`, `terminal-full-screen`) carry their keyed tooltips. `diff_full_screen.rs::the_full_screen_button_names_its_key` and `diff_full_screen.rs::the_terminal_arrow_names_its_key` check them.

10. **I do:** press Esc with nothing open (no menu, no sheet, no comment box).
    **I see:** the panel stays in full screen. If the focus is in the terminal, Esc goes to the program there.
    **State and events:** `features/app/src/overlay.rs:56-100` does not leave full screen. `ui/src/shell.rs:823` `escape_closes_something` does not check full screen. The spec wants Esc to leave full screen once nothing else is open (DS-13).
    **Status:** Fakes only. Built (DV-18): `diff_full_screen.rs::escape_leaves_full_screen_but_not_while_the_base_menu_is_open`, `::escape_closes_the_new_agent_tab_menu_first`.

11. **I do:** click "Show the terminal" in the title bar.
    **I see:** the panel goes back beside the terminal. The same file is open ("api · src/client.rs"), the list is 220 pt tall above the code, and the terminal is back.
    **State and events:** `ToggleFullScreen(Panel)` runs again and removes the session's entry in `full_screens` (`features/app/src/lib.rs:1153-1180`). The toggle sends no fetch (it returns `Effect::none()`). `saved.selected` is unchanged.
    **Status:** Fakes only. `diff_full_screen.rs::show_the_terminal_keeps_the_selected_file_and_its_scroll` clicks `show-terminal` (the Exit full screen button) and checks the file; `pr_session.rs::show_the_terminal_returns_the_review_beside_the_terminal` checks the saved file; `shell.rs::terminal_and_panel_full_screen_return_to_the_same_columns` checks the layout.

12. **I do:** look at where the code was, and at my comment box and draft.
    **I see:** the code is scrolled to about line 120, line 140 is still focused, Wrap is still off, the comment box on line 43 is still open, and the draft is still there (DS-69).
    **State and events:** the scroll is in `Documents` (`ui/src/document_tab.rs:553`, `keep_top`). The drafts are in `state.comments`, not in the view, so they survive a toggle. Nothing resets them in `ToggleFullScreen`.
    **Status:** Fakes only. `review_ui.rs::show_the_terminal_keeps_the_line_scroll_drafts_and_open_box` keeps the scroll line and offset, the draft and the open box. Not tested: the focused line, Wrap and the sideways shift (DS-69).

13. **I do:** while the panel is full, let the agent keep working, then come back.
    **I see:** the terminal is not drawn while the panel is full. When I come back, the terminal shows the output the agent wrote meanwhile, and the agent did not restart.
    **State and events:** the terminal element is removed from the tree in full screen (`shell.rs` checks `!app.contains("terminal")`, `shell.rs:149-160`). The terminal process lives in the holder, not in the view, so it is not reloaded. DS-69 says "The terminal keeps running underneath; nothing is reloaded."
    **Status:** Code, no test for this round trip. `review_ui.rs::full_screen_and_back_sends_no_fetch` shows nothing is reloaded; that the process keeps running is proven for the terminal itself in the lab (micro case 17), not for this toggle.

14. **I do:** while the panel is full, switch to the "Deploy" session, then come back to "Refunds".
    **I see:** "Deploy" opens in its own state, with its panel closed. Coming back to "Refunds" restores its full screen.
    **State and events:** `AppState.full_screens` is keyed by session (`features/app/src/lib.rs:356-366`). `full_screen()` returns `None` for a session whose route is not open.
    **Status:** Code, no test for the switch (DS-70). `features/app/tests/presentation.rs::expanding_either_area_and_returning_preserves_the_columns` saves and restores `full_screens` per session; no test switches sessions while one is full.

15. **I do:** click ✕ (Close panel) while the panel is full.
    **I see:** the panel closes, and the terminal is back full width in the session. Full screen ends with the panel.
    **State and events:** `SessionAction::SetPanel(None)` clears `widened` (`features/session/src/lib.rs:1260-1270`), and `AppState::full_screen()` returns `None` when no panel is open (`features/app/src/lib.rs:356-366`).
    **Status:** Fakes only. `diff_full_screen.rs::close_in_full_screen_closes_the_panel`; `features/app/tests/presentation.rs::closing_the_full_screen_panel_returns_to_the_terminal`.

16. **I do:** click "Show the terminal" and then type.
    **I see:** the focus should go back to where it was before full screen: the terminal, or the diff (DS-83).
    **State and events:** `ToggleFullScreen` sets no focus. The focus is not restored.
    **Status:** Fakes only for the terminal: `review_ui.rs::show_the_terminal_returns_focus_to_the_terminal`. Returning to the diff when it had focus is not tested (DS-83).

## Branches

### A PR review goes full screen

When I open a pull request review (PR #482) and press ⤢.
I see frame `68b-pr-review-full-screen`: the PR line (for example "#482") takes the base chip's place in the header, and the title bar and Show the terminal are the same as in step 2. Show the terminal returns to frame `68c-pr-review-after-show-terminal`, beside the terminal, with the same file open.
**Status:** Fakes only. `pr_session.rs::show_the_terminal_returns_the_review_beside_the_terminal` (it asserts the saved file, and the frames are recorded).

### The panel is narrow when I press ⤢

When the Diff panel is dragged to 220 pt and I press ⤢, or when I press ✕ with the panel narrow.
I see the ⤢ and ✕ buttons inside the panel's edge. The PR line and base chip ellipsize. Full screen still works.
**Status:** Fakes only. `pr_session.rs::a_narrow_review_panel_keeps_full_screen_and_close`.

### The window is narrow in full screen

When I narrow the window while the panel is full.
I see the names in the title bar ellipsize, and Show the terminal stays at the right (`panels.rs:255-290`, the comment "The names ellipsize; Show the terminal always stays").
**Status:** Code, no test. `ui/core/src/chrome.rs::panel_header_actions` ellipsizes the names and pins Exit full screen and Close; no test narrows the window.

### I toggle Wrap, or the width changes under Wrap, while full

When the code is wrapped and the width changes.
I see the line at the top of the view stays at the top (`document_tab.rs:439`, `keep_top`).
**Status:** Fakes only. `review.rs::widening_a_wrapped_diff_keeps_its_top_line_in_place` covers a width change under Wrap; a full-screen round trip under Wrap is not tested (DS-44).

### Terminal full screen instead of panel full screen

When I press ⇧⌘↩ (terminal full screen) instead of ⤢.
I see the terminal full screen, with the panel hidden. The terminal's own full-screen button brings the layout back (the test clicks `terminal-full-screen` again).
**Status:** Fakes only. `shell.rs::terminal_and_panel_full_screen_return_to_the_same_columns`. Keys: `ui/core/src/keys.rs` `shift-cmd-enter` (`TerminalFullScreen`), now in the ⌘/ sheet.

### ⤢ with no panel open

When no panel is open and I press ⤢.
I see nothing changes (the reducer returns `Effect::none()` when the panel is not open and the side is the panel).
**Status:** Code, no test. `features/app/src/lib.rs` (`ToggleFullScreen` returns `Effect::none()` with no panel open).

## Expected outcomes

- [ ] ⤢ on the Diff header, ⌥⌘↩, and a double-click on the header each open full screen (DS-12). Only ⤢ does today.
- [ ] The title bar is 52 pt, with "# channel › session", "Diff" and the base chip or PR line, then Show the terminal (DS-14).
- [ ] Full screen draws one title bar, and no second Diff header (DS-14).
- [ ] The list is 300 pt wide at the left, and the code fills the rest with the content colour (DS-15).
- [ ] The footer is the narrow footer, with the same actions, including Submit review in a PR review (DS-16).
- [ ] The ⤢ button reads "Full screen" with a tooltip "Full screen ⌥⌘↩" (DS-17).
- [ ] Show the terminal, ⌥⌘↩ again, or Esc (with nothing open) leave full screen (DS-13).
- [ ] Show the terminal keeps the open file, the layer, the focused line, the scroll, Wrap, any comment box and every draft (DS-69).
- [ ] The terminal keeps running underneath, and nothing is reloaded (DS-69).
- [ ] Full screen is remembered per session, and coming back restores it (DS-70).
- [ ] Closing the panel ends full screen (DS-71).
- [ ] Focus goes back to where it was before full screen (DS-83).

## Status table

| Step or branch | Status | Evidence |
|---|---|---|
| 1 ⤢ enters full screen | Fakes only | `targets.rs::full_screen_draws_each_repository_base_too` |
| 2 Title bar | Fakes only | `diff_full_screen.rs::full_screen_draws_one_header` |
| 3 Second header in full screen | Fakes only | `diff_full_screen.rs::full_screen_draws_one_header` (Likely bug cleared) |
| 4 List 300 pt, code fills | Code, no test | `panel_changes/mod.rs::changes`; `diff_view.rs::the_file_list_scrolls_in_the_expanded_panel` checks height only |
| 5 Light-mode grey void | Code, no test | `panel_changes/mod.rs::changes` (DV-17); no light frame |
| 6 Footer in full screen | Code, no test | `panel_changes/mod.rs::changes` footer; the Submit review test runs narrow only |
| 7 ⌃⌘↩ (was ⌥⌘↩) | Fakes only | `diff_full_screen.rs::alt_cmd_enter_enters_and_leaves_full_screen` |
| 8 Double-click the header | Fakes only | `diff_full_screen.rs::double_clicking_the_panel_header_enters_full_screen` |
| 9 Tooltips and labels | Fakes only | `diff_full_screen.rs::the_full_screen_button_names_its_key`; `diff_full_screen.rs::the_terminal_arrow_names_its_key`; `ui/core/src/chrome.rs::panel_header`, `ui/src/pages.rs` |
| 10 Esc leaves full screen | Fakes only | `diff_full_screen.rs::escape_leaves_full_screen_but_not_while_the_base_menu_is_open` |
| 11 Exit full screen (Show the terminal) | Fakes only | `diff_full_screen.rs::show_the_terminal_keeps_the_selected_file_and_its_scroll`; `pr_session.rs::show_the_terminal_returns_the_review_beside_the_terminal` |
| 12 Scroll, focus, drafts kept | Fakes only | `review_ui.rs::show_the_terminal_keeps_the_line_scroll_drafts_and_open_box` (not the focused line or Wrap) |
| 13 Terminal runs under full screen | Code, no test | `review_ui.rs::full_screen_and_back_sends_no_fetch` (no reload); micro case 17 for the process |
| 14 Switch session and back | Code, no test | `features/app/src/lib.rs` (`full_screens`); `presentation.rs` saves it |
| 15 Close the panel | Fakes only | `diff_full_screen.rs::close_in_full_screen_closes_the_panel`; `presentation.rs::closing_the_full_screen_panel_returns_to_the_terminal` |
| 16 Focus after Show the terminal | Fakes only | `review_ui.rs::show_the_terminal_returns_focus_to_the_terminal` (terminal only) |
| Branch: PR review full | Fakes only | `pr_session.rs::show_the_terminal_returns_the_review_beside_the_terminal`; frames 68b, 68c |
| Branch: narrow panel | Fakes only | `pr_session.rs::a_narrow_review_panel_keeps_full_screen_and_close` |
| Branch: narrow window | Code, no test | `ui/core/src/chrome.rs::panel_header_actions` |
| Branch: Wrap while full | Fakes only | `review.rs::widening_a_wrapped_diff_keeps_its_top_line_in_place` (width only) |
| Branch: terminal full screen | Fakes only | `shell.rs::terminal_and_panel_full_screen_return_to_the_same_columns` |
| Branch: ⤢ with no panel | Code, no test | `features/app/src/lib.rs` |

Counts (22 rows): Works (real shell) 0; Works (real daemon) 0; Fakes only 15; Code, no test 7; Not built 0; Likely bug 0.

## Gaps and likely bugs

- Fakes only: tooltips with keys on Exit full screen and on the session header's terminal arrow (DS-17, DS-79), `diff_full_screen.rs::the_full_screen_button_names_its_key` and `::the_terminal_arrow_names_its_key`.
- Cleared: the double header in full screen (`full_screen_draws_one_header`).
- Gap in tests: the full-screen footer, the light-mode void, a full-screen round trip under Wrap, the focused line and sideways shift kept, the terminal running underneath, switching sessions while full, and returning focus to a focused diff are not tested (DS-15, DS-16, DS-44, DS-69, DS-70, DS-83).
- Wording after D848 and D860: the button reads "Exit full screen" and the full-screen Close stays beside it; the narrative above still says "Show the terminal" and ⌥⌘↩ (the key is ⌃⌘↩).

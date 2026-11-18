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
   **Status:** Fakes only (partly built, DS-12). `targets.rs::full_screen_draws_each_repository_base_too` clicks `expand-panel` and checks the full screen state. The ways in other than the button are Not built (steps 7 and 8).

2. **I do:** look at the title bar, which is 52 pt tall.
   **I see:** the window lights, a panel icon, "# payments › Refunds" (ellipsized if long), "Diff" beside it in larger text, and "Show the terminal" at the right.
   **State and events:** `ui/src/panels.rs:250-360`, the `when(full)` branch: a 52 pt row (`:255-262`), the names in a column that shrinks and ellipsizes, and `show-terminal` (`:343-350`).
   **Status:** Fakes only. `desktop/tests/native/session.rs::session_lifecycle_and_full_screen_reference_images` records frame `11-panel-full-screen-diff.view.txt`, which holds the title bar. DS-14 says the title bar is built.

3. **I do:** look below the title bar.
   **I see:** a second "Diff" header row, with its own "Diff" text, its own base chip "Against main", and its own "Show the terminal" button and "Close panel" button, right under the title bar. So there are two "Diff" titles and two "Show the terminal" buttons on one screen.
   **State and events:** the body draws `header(...)` again with `full` true (`ui/src/panel_changes.rs:622` for a local diff and `:695` for a review). The header's button is still id `expand-panel`, but its label is "Show the terminal" (`ui/src/panels.rs:157-161`). The spec says the panel's own 36 pt header is not drawn in full screen (DS-14, DV-18).
   **Status:** Likely bug. The snapshot `11-panel-full-screen-diff.view.txt` lines 6-12 show both. To confirm: open that snapshot and count "Show the terminal" (two) and "Diff" (two).

4. **I do:** look at the file list and the code.
   **I see:** the list on the left, about 300 pt wide, with a border on its right. The code fills the rest, down to the footer, with no gap.
   **State and events:** in full screen the list is `w(300)`, `flex_shrink_0`, with a right border (`ui/src/panel_changes.rs:728-735`).
   **Status:** Code, no test. `diff_view.rs::the_file_list_scrolls_in_the_expanded_panel` checks only that the list is taller than 220 pt and inside the window.

5. **I do:** switch to the light theme and look at the empty part of the code column.
   **I see:** in light mode, the code column shows a grey band where the content colour should run from top to bottom (the light-mode grey void).
   **State and events:** DS-15 says the code fills from top to bottom in the content colour.
   **Status:** Not built (DV-17). The spec says so; the code was not changed for it.

6. **I do:** look at the footer in full screen.
   **I see:** the same footer as beside the terminal: a draft line at the left ("1 draft comment") and the "Send 1 comment" button at the right. In a pull request review, "Submit review…" should be here too (DS-16), and it is not.
   **State and events:** the footer is the same code (`ui/src/panel_changes.rs:745-792`). Submit review is in the session bar, not the footer (spec §8 item 4).
   **Status:** Not built (DV-18, DS-16) for Submit review in the footer. The shared footer is Code, no test.

7. **I do:** press ⌥⌘↩.
   **I see:** nothing happens.
   **State and events:** there is no `alt-cmd-enter` binding in `ui/src/keys.rs` (the bindings at `:198-237`). ⇧⌘↩ is the terminal's full screen (`keys.rs`, `TerminalFullScreen`), not the panel's.
   **Status:** Not built (DV-18, DS-12).

8. **I do:** double-click the Diff header.
   **I see:** nothing happens.
   **State and events:** no double-click handler on the panel header (`ui/src/panels.rs` has none).
   **Status:** Not built (DV-18, DS-12).

9. **I do:** hover the ⤢ button in the narrow panel, or the "Show the terminal" button in full screen.
   **I see:** no tooltip. The label for accessibility reads "Expand panel" in the narrow panel and "Show the terminal" in full screen.
   **State and events:** `icon_action` sets no tooltip (`ui/src/panels.rs:92-111`). The spec wants "Full screen ⌥⌘↩" for the panel and "⇧⌘↩" for the session's header (DS-17).
   **Status:** Not built (DV-18, DS-17).

10. **I do:** press Esc with nothing open (no menu, no sheet, no comment box).
    **I see:** the panel stays in full screen. If the focus is in the terminal, Esc goes to the program there.
    **State and events:** `features/app/src/overlay.rs:56-100` does not leave full screen. `ui/src/shell.rs:823` `escape_closes_something` does not check full screen. The spec wants Esc to leave full screen once nothing else is open (DS-13).
    **Status:** Not built (DV-18, DS-13).

11. **I do:** click "Show the terminal" in the title bar.
    **I see:** the panel goes back beside the terminal. The same file is open ("api · src/client.rs"), the list is 220 pt tall above the code, and the terminal is back.
    **State and events:** `ToggleFullScreen(Panel)` runs again and removes the session's entry in `full_screens` (`features/app/src/lib.rs:1153-1180`). The toggle sends no fetch (it returns `Effect::none()`). `saved.selected` is unchanged.
    **Status:** Fakes only. `pr_session.rs::show_the_terminal_returns_the_review_beside_the_terminal` clicks `expand-panel`, then `show-terminal`, and checks the saved file. `shell.rs::terminal_and_panel_full_screen_return_to_the_same_columns` checks the layout is the same after the round trip.

12. **I do:** look at where the code was, and at my comment box and draft.
    **I see:** the code is scrolled to about line 120, line 140 is still focused, Wrap is still off, the comment box on line 43 is still open, and the draft is still there (DS-69).
    **State and events:** the scroll is in `Documents` (`ui/src/document_tab.rs:553`, `keep_top`). The drafts are in `state.comments`, not in the view, so they survive a toggle. Nothing resets them in `ToggleFullScreen`.
    **Status:** Code, no test. DS-69 is partly built; the spec says the scroll and the focus are not tested. `review.rs::widening_a_wrapped_diff_keeps_its_top_line_in_place` covers a width change under Wrap, not a full-screen round trip.

13. **I do:** while the panel is full, let the agent keep working, then come back.
    **I see:** the terminal is not drawn while the panel is full. When I come back, the terminal shows the output the agent wrote meanwhile, and the agent did not restart.
    **State and events:** the terminal element is removed from the tree in full screen (`shell.rs` checks `!app.contains("terminal")`, `shell.rs:149-160`). The terminal process lives in the holder, not in the view, so it is not reloaded. DS-69 says "The terminal keeps running underneath; nothing is reloaded."
    **Status:** Code, no test for this round trip. The process-keeps-running claim is proven for the terminal itself in the lab (micro case 17), not for this toggle.

14. **I do:** while the panel is full, switch to the "Deploy" session, then come back to "Refunds".
    **I see:** "Deploy" opens in its own state, with its panel closed. Coming back to "Refunds" restores its full screen.
    **State and events:** `AppState.full_screens` is keyed by session (`features/app/src/lib.rs:356-366`). `full_screen()` returns `None` for a session whose route is not open.
    **Status:** Code, no test for the switch (DS-70). The data is built; no test switches sessions while full.

15. **I do:** click ✕ (Close panel) while the panel is full.
    **I see:** the panel closes, and the terminal is back full width in the session. Full screen ends with the panel.
    **State and events:** `SessionAction::SetPanel(None)` clears `widened` (`features/session/src/lib.rs:1260-1270`), and `AppState::full_screen()` returns `None` when no panel is open (`features/app/src/lib.rs:356-366`).
    **Status:** Fakes only. `features/app/tests/presentation.rs::closing_the_full_screen_panel_returns_to_the_terminal`.

16. **I do:** click "Show the terminal" and then type.
    **I see:** the focus should go back to where it was before full screen: the terminal, or the diff (DS-83).
    **State and events:** `ToggleFullScreen` sets no focus. The focus is not restored.
    **Status:** Not built (DV-18, DS-83). The spec calls this proposed.

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
**Status:** Code, no test. `panels.rs` has the comment, no test checks it.

### I toggle Wrap, or the width changes under Wrap, while full

When the code is wrapped and the width changes.
I see the line at the top of the view stays at the top (`document_tab.rs:439`, `keep_top`).
**Status:** Fakes only. `review.rs::widening_a_wrapped_diff_keeps_its_top_line_in_place`.

### Terminal full screen instead of panel full screen

When I press ⇧⌘↩ (terminal full screen) instead of ⤢.
I see the terminal full screen, with the panel hidden. The terminal's own full-screen button brings the layout back (the test clicks `terminal-full-screen` again).
**Status:** Fakes only. `shell.rs::terminal_and_panel_full_screen_return_to_the_same_columns`. Keys: `ui/src/keys.rs` `shift-cmd-enter` (`TerminalFullScreen`).

### ⤢ with no panel open

When no panel is open and I press ⤢.
I see nothing changes (the reducer returns `Effect::none()` when the panel is not open and the side is the panel).
**Status:** Code, no test. `features/app/src/lib.rs:1156-1158`.

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
| 2 Title bar | Fakes only | frame `11-panel-full-screen-diff.view.txt` |
| 3 Second header in full screen | Likely bug | `panel_changes.rs:622`, `:695`; `panels.rs:157-161`; snapshot lines 6-12 |
| 4 List 300 pt, code fills | Code, no test | `panel_changes.rs:728-735`; `diff_view.rs::the_file_list_scrolls_in_the_expanded_panel` checks height only |
| 5 Light-mode grey void | Not built | DV-17, DS-15 |
| 6 Footer in full screen | Not built | DV-18, DS-16 (Submit review); `panel_changes.rs:745-792` |
| 7 ⌥⌘↩ | Not built | `ui/src/keys.rs:198-237` has no binding; DV-18 |
| 8 Double-click the header | Not built | `ui/src/panels.rs` has no handler; DV-18 |
| 9 Tooltips and labels | Not built | `panels.rs:92-111`; DV-18, DS-17 |
| 10 Esc leaves full screen | Not built | `features/app/src/overlay.rs:56-100`; `ui/src/shell.rs:823`; DV-18, DS-13 |
| 11 Show the terminal | Fakes only | `pr_session.rs::show_the_terminal_returns_the_review_beside_the_terminal` |
| 12 Scroll, focus, drafts kept | Code, no test | `document_tab.rs:553`; DS-69 |
| 13 Terminal runs under full screen | Code, no test | `shell.rs:149` (element only); micro case 17 for the process |
| 14 Switch session and back | Code, no test | `features/app/src/lib.rs:356-366`; DS-70 |
| 15 Close the panel | Fakes only | `features/app/tests/presentation.rs::closing_the_full_screen_panel_returns_to_the_terminal` |
| 16 Focus after Show the terminal | Not built | DS-83 (proposed) |
| Branch: PR review full | Fakes only | `pr_session.rs::show_the_terminal_returns_the_review_beside_the_terminal`; frames 68b, 68c |
| Branch: narrow panel | Fakes only | `pr_session.rs::a_narrow_review_panel_keeps_full_screen_and_close` |
| Branch: narrow window | Code, no test | `panels.rs` comment |
| Branch: Wrap while full | Fakes only | `review.rs::widening_a_wrapped_diff_keeps_its_top_line_in_place` |
| Branch: terminal full screen | Fakes only | `shell.rs::terminal_and_panel_full_screen_return_to_the_same_columns` |
| Branch: ⤢ with no panel | Code, no test | `features/app/src/lib.rs:1156-1158` |

Counts (22 rows): Works (real shell) 0; Works (real daemon) 0; Fakes only 8; Code, no test 6; Not built 7; Likely bug 1.

## Gaps and likely bugs

- Likely bug: in full screen, the panel's own header is drawn under the title bar (`ui/src/panel_changes.rs:622` and `:695` call `header(...)` with `full` true). That gives two "Diff" titles, two base or PR chips, and two "Show the terminal" buttons (one with id `expand-panel`, one `show-terminal`). The snapshot `desktop/tests/__snapshots__/native/11-panel-full-screen-diff.view.txt` shows it. The pull request test asserts `show-terminal` exists, not that the header is gone.
- Not built: ⌥⌘↩ and the double-click on the header (DS-12, DV-18), Esc to leave (DS-13), tooltips and the "Full screen" label (DS-17), and focus after Show the terminal (DS-83).
- Not built: Submit review in the full-screen footer (DS-16). The spec's §8 item 4 puts it in the session bar, so the footer test would need the decision first.
- Not built: the light-mode grey void (DV-17).
- Gap in tests: `show_the_terminal_returns_the_review_beside_the_terminal` checks only the saved file. No test checks the scroll, the focused line, Wrap, the comment box or a draft across the round trip, and none switches sessions while full (DS-69, DS-70).
- Gap in tests: no test shows the terminal process still running after the round trip. That claim is only in the lab tests for the terminal.

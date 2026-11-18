# A changed file opens beside the terminal and the code gets the room

Layer: Diff and review. Platforms: macOS desktop.

## Starting state

- Machine `box-1` has the channel `#payments` with the session "Refunds". Its repos are `acme/api` and `acme/ledger`.
- "Refunds" has 586 changed files against `origin/main`: 12 in `acme/api` and 574 in `acme/ledger`. The last repo is the long one.
- The app shows the "Refunds" session. The terminal is on the left, running an agent. The Diff panel is open on the right, at its normal width (about 360 pt, beside the terminal).
- No file is open. The panel shows the file list, and the code area is empty below it.
- The headless fixture (`desktop/tests/native/diff_view.rs::many_files_app`) holds a handful of files, not 586. The 586 case is proven only by reading the list code.

## Reproduce headless

- Fakes: `bazel test //apps/humata2/desktop:native_test --test_filter=a_long_file_list_leaves_room_for_the_code` (the list is at most 220 pt, and the code sits under it above the footer).
- Fakes: `bazel test //apps/humata2/desktop:native_test --test_filter=the_file_list_scrolls_in_the_expanded_panel` (the list scrolls when the panel is wide).
- Fakes: `bazel test //apps/humata2/desktop:native_test --test_filter=a_long_base_keeps_expand_and_close_inside_the_panel` (the header row and its buttons).
- Fakes: `bazel test //apps/humata2/desktop:native_test --test_filter=the_panels_draw_their_client_data` (`desktop/tests/panels.rs`, which picks a file through `SessionAction::SelectFile`).
- Real daemon, for the data only: `bazel test //apps/humata2/daemon/humatad:pr_diff_stress_test` lists all 300 files of a pull request. That is a review diff, not the local Diff panel, so it does not prove this layout.
- Not headless: how the scroll and the drag feel in a real window. The headless tests check bounds. No windows on the owner's screen.

## Steps

1. **I do:** open the "Refunds" session. The Diff panel is already beside the terminal.
   **I see:** a 36 pt header row with "Diff", the base chip ("vs origin/main"), a ⤢ button and a ✕ button at the right (DS-1). Under it, the file list in a box at most 220 pt tall, with the first file already open under it. The code area takes the rest of the panel.
   **State and events:** nothing saved for this session yet, so the panel opens the first file of the first repo (`selected`, `ui/src/panels.rs:201-226`, falls back to `diff.files.first()`). The list is `changes-files`, capped by `max_h(220)` when the panel is not full (`ui/src/panel_changes.rs:739-741`). The empty-state line (`panel_changes.rs:606-607`) shows only when no repo has a file, because the fallback always picks one.
   **Status:** Fakes only. DS-1: `diff_view.rs::a_long_file_list_leaves_room_for_the_code`; `diff_view.rs::a_long_base_keeps_expand_and_close_inside_the_panel`. DS-4 (the list fills the panel with no file open) never arises while files exist: a file is always open.

2. **I do:** scroll the file list down to `acme/ledger`, the last repo.
   **I see:** the list scrolls inside its 220 pt box. The header and the footer stay put. The last repo's rows are reachable (DS-30).
   **State and events:** the list is an `overflow_y_scroll` div (`panel_changes.rs:733-741`). The scroll position is not saved across a restart.
   **Status:** Code, no test. The 220 pt cap is tested, but no test scrolls the narrow list to its last row. The expanded scroll is tested (`diff_view.rs::the_file_list_scrolls_in_the_expanded_panel`).

3. **I do:** click the file row `acme/api` › `src/client.rs`.
   **I see:** the row is lit. The code for that file appears under the list, with a header line "api · src/client.rs" (DS-24 partly built). The list stays 220 pt tall above the code. It does not fold (DS-3 folding is not built, step 5).
   **State and events:** `ReviewAction::SelectFile { repo: "api", path: "src/client.rs" }` (`ui/src/panel_changes.rs:160-169`). The reducer stores the pair in `saved.selected` for this session, only when the file is in the diff (`features/review/src/lib.rs:678-687`).
   **Status:** Fakes only. `desktop/tests/panels.rs::the_panels_draw_their_client_data` picks a file.

4. **I do:** look at the code under the list.
   **I see:** the code fills the rest of the panel, down to the footer. Its width is the panel's width. Lines scroll sideways unless Wrap is on. The footer shows "0 draft comments" when there are no drafts (DS-10 is not built; see case 2).
   **State and events:** the code view is `change-code api/src/client.rs`. The footer is `panel_changes.rs:745-792`.
   **Status:** Fakes only. `diff_view.rs::a_long_file_list_leaves_room_for_the_code` finds `change-code api/src/client.rs`, checks it is taller than 100 pt, and checks it sits above the footer's Send button.

5. **I do:** click the file bar, or the ⌄ beside the file name, to open the list again.
   **I see:** nothing happens. There is no file bar. The list stays above the code in its 220 pt box. Picking a file does not fold the list, so the list and the code share the panel's height.
   **State and events:** the one-line file bar (⌄, "api · src/client.rs", "2 of 4", ‹ ›, ↗) is not in the code. `ReviewAction::ToggleFile` folds a file's lines in the code (`features/review/src/lib.rs:905-916`), not the list.
   **Status:** Not built (DV-15). DS-2 and DS-3.

6. **I do:** press ⌥⌘↓ to go to the next file.
   **I see:** nothing moves. The next file is not selected, and the list does not scroll to it.
   **State and events:** there is no `alt-cmd-down` binding in `ui/src/keys.rs` (the bindings at `keys.rs:198-237` have ⌥⌘← and ⌥⌘→ for the sidebar only). DS-53 wants ⌥⌘↓ and ⌥⌘↑ here. The spec §8 item 1 settles that files keep these keys, and STACKVIEW's layer keys move to ⌥⌘] and ⌥⌘[.
   **Status:** Not built (DV-15).

7. **I do:** click ↗ at the right of the code header.
   **I see:** the words "Open in editor", dimmed to half opacity. Clicking them does nothing.
   **State and events:** `panel_changes.rs:594-600`, id `change-open-editor`, with no `on_click`. The label is drawn, not an icon button with a tooltip (DS-2 wants ↗ as an icon button). The code notes "No editor client exists yet".
   **Status:** Code, no test. The control is inert by design until an editor client exists.

8. **I do:** click the same file row again.
   **I see:** the row stays selected and the code stays open. Nothing toggles off.
   **State and events:** `SelectFile` stores the same pair again (`features/review/src/lib.rs:678-687`). This is the behaviour the spec wants for picking (DS-3 does not say a second click folds).
   **Status:** Code, no test. No test re-picks the same file.

## Branches

### The panel is dragged narrower, to 220 pt

When I drag the panel's edge in toward the terminal.
I see the header keeps its ⤢ and ✕ inside the panel. The base chip or the PR line ellipsizes. The list stays 220 pt tall, and the code takes the rest of the width.
**Status:** Fakes only for the header buttons: `pr_session.rs::a_narrow_review_panel_keeps_full_screen_and_close` drags to 220 pt and checks both buttons fit. Code, no test for the list and code at 220 pt.

### The app restarts with a file open

When I quit and reopen the app while "api · src/client.rs" is open.
I see the same file picked for "Refunds" when the Diff panel opens again. The list is back at its 220 pt box. The scroll position is not restored.
State: `saved.selected` holds `("api", "src/client.rs")` for the session (`features/review/src/lib.rs:102-108`).
**Status:** Fakes only. `pr_session.rs::show_the_terminal_returns_the_review_beside_the_terminal` checks only that `saved.selected` is kept across Show the terminal. No test restarts the app. The scroll reset is Code, no test.

### The session has no changes at all

When I open a session whose diff is empty.
I see the empty-state line for its base: "No changes yet" for `origin/main`, "Nothing uncommitted" for uncommitted-only, "No changes since you last reviewed" for since-last-review, "No changes since the fork", "No changes against that branch", "No changes against the pull request's base", or "No changes against each repository's base". No list rows. The code area is empty.
**Status:** Code, no test. `ui/src/panel_changes.rs:1105-1119`.

### The 586-file session: I need the list's last rows

When I open "Refunds" with 586 files, and the last repo is `acme/ledger`.
I see the list of 586 rows in a 220 pt box. I scroll to the bottom and the last row is reachable. Each row keeps its name and its +/− counts (DS-7 is not built; see case 3).
**Status:** Code, no test. No test creates 586 files for this layout. `daemon/humatad/tests/pr_diff_stress.rs` proves the daemon returns 300 files (Works (real daemon), for the data only).

## Expected outcomes

- [ ] The Diff panel header is one 36 pt row: "Diff", the base chip, ⤢ and ✕ (DS-1).
- [ ] With files listed, the first one opens by itself (DS-4 then never arises).
- [ ] The list scrolls, so the last repo's files are reachable (DS-30).
- [ ] Picking a file opens its code under the list and marks the row selected.
- [ ] The selected file is kept in `saved.selected` for its session.
- [ ] The file bar folds the list, and ⌥⌘↓ and ⌥⌘↑ move between files (DS-2, DS-3, DS-53). These are not built yet.
- [ ] The code takes the height the list leaves, down to the footer (DS-2). Today it sits under a 220 pt list.
- [ ] With files listed and none picked, the empty-state line does not say "No changes" (see the Gaps).

## Status table

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Open, header, list cap | Fakes only | `diff_view.rs::a_long_file_list_leaves_room_for_the_code`; `::a_long_base_keeps_expand_and_close_inside_the_panel` |
| 2 Scroll to the last repo | Code, no test | `panel_changes.rs:733-741` |
| 3 Pick a file | Fakes only | `panels.rs::the_panels_draw_their_client_data` |
| 4 Code fills the rest | Fakes only | `diff_view.rs::a_long_file_list_leaves_room_for_the_code` |
| 5 File bar and fold | Not built | DV-15; DS-2, DS-3 |
| 6 ⌥⌘↓ next file | Not built | DV-15; `ui/src/keys.rs:198-237` has no binding |
| 7 Open in editor | Code, no test | `panel_changes.rs:594-600` (inert) |
| 8 Pick the same file again | Code, no test | `features/review/src/lib.rs:678-687` |
| Branch: narrow panel | Fakes only (buttons) | `pr_session.rs::a_narrow_review_panel_keeps_full_screen_and_close` |
| Branch: restart with a file open | Fakes only | `pr_session.rs::show_the_terminal_returns_the_review_beside_the_terminal` |
| Branch: empty session | Code, no test | `panel_changes.rs:1105-1119` |
| Branch: 586 files | Code, no test | `pr_diff_stress.rs` covers the daemon's 300 files only |

Counts (12 rows): Works (real shell) 0; Works (real daemon) 0; Fakes only 5; Code, no test 5; Not built 2; Likely bug 0.

## Gaps and likely bugs

- Not a bug (coach-checked): "No changes yet" never shows under a listed diff. `selected` (`ui/src/panels.rs:201-226`) falls back to the first file, so `picked` is empty only when no repo has files.
- Not built: the file bar (DS-2), folding the list when a file opens (DS-3), and the ⌥⌘↓ / ⌥⌘↑ next and previous file keys (DS-53).
- DS-4 needs a decision once DV-15 lands: today a file is always open, so "no file open" never happens. With a folded file bar, the first file can stay auto-open and DS-4 drops.
- Not built: the ↗ icon button with a tooltip (DS-2, DS-24). The dimmed words are inert (`panel_changes.rs:594-600`).
- Gap in tests: no test scrolls the narrow list to its last row, none checks the empty-state line against a listed diff, and none uses 586 files.

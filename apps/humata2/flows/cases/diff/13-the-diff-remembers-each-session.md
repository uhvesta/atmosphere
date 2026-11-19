# Each session keeps its own open file, scroll and full screen, and opening the panel doesn't read everything again

Layer: Diff and review. Platforms: macOS desktop.

## Starting state

- Machine `box-1` has the repo `acme/api` cloned. It has two sessions on it: "Refunds" and "Ledger". Both have changed `src/client.rs`, and the file is long enough to scroll.
- The app is open on "Refunds". Its Changes panel is open beside the terminal.
- No session is in full screen.
- Nothing else is open. No other repo is in either session.

## Reproduce headless

- Open file, full screen and selection, all with fakes: `bazel test //apps/humata2/features/app:presentation_test --test_filter=restart` (`a_restart_restores_both_tab_groups_and_the_selected_file`), and `bazel test //apps/humata2/features/review:collapse_test` (the selected file is kept when its repo is collapsed).
- The diff panel in headless windows: `bazel test //apps/humata2/desktop:native_test --test_filter=diff_view`. Frames are in `desktop/tests/__snapshots__/native/`.
- The live restart with a real daemon: `bazel test //apps/humata2/desktop:live_flows_test --test_filter=long_lines_switch_between_wrap_and_scroll_and_scroll_survives_a_restart` covers the wrap choice across a restart, not the per-session scroll.
- Not headless: how the scroll looks on a real screen, and idle CPU. Idle CPU was measured by hand in the DIFFVIEW run. There is no automated check.
- No window goes on the owner's screen. Run the native target headless only.

## Steps

| # | I do | I see | State and events | Status |
|---|---|---|---|---|
| 1 | In "Refunds", I click `src/client.rs` in the Changes list. | The file opens in the code area under its header. | `ReviewAction::SelectFile` stores `(acme/api, src/client.rs)` for the session "Refunds" in `saved.selected` (DS-72). | Fakes only. `features/app/tests/presentation.rs::a_restart_restores_both_tab_groups_and_the_selected_file` (one session only). |
| 2 | I scroll `src/client.rs` down to about line 400. | The code moves. The header stays put (DS-26). | Scroll lives in `Documents` (`ui/src/document_tab.rs:553`), keyed by the string `acme/api/src/client.rs`. | Fakes only. `desktop/tests/native/review_ui.rs::two_sessions_on_one_file_keep_their_own_scroll` scrolls a long file in one session. The scroll lives in `Documents` keyed by session, repo and path (`ui/changes/src/document_tab.rs::session_diff_view`). |
| 3 | I switch to "Ledger" and open its Changes panel. | Ledger's panel opens. Its file list is Ledger's. | `ReviewAction::Open` for Ledger runs (`features/review/src/lib.rs:897`). | Fakes only. `review_ui.rs::two_sessions_on_one_file_keep_their_own_scroll` opens the other session's panel and file (`open_long_file`). |
| 4 | In Ledger, I click `src/client.rs`. | The file opens at its top, as Ledger has not scrolled it yet (DS-73). | Expected: scroll is per session and file, so Ledger starts at line 1. Actual: the code reuses the `diff_view` entry for `acme/api/src/client.rs` (`panel_changes.rs:475`, `document_tab.rs:553`), so Ledger opens at about line 400, where Refunds left it. | Fakes only. Likely bug cleared (RV-3): `review_ui.rs::two_sessions_on_one_file_keep_their_own_scroll` asserts the second session's scroll differs from the first's. `session_diff_view` puts the session in the key; the call sites are in `panel_changes/mod.rs` and `review_header.rs`. |
| 5 | I switch back to "Refunds". | Refunds' file is still `src/client.rs`, and it is still scrolled to its place. | `saved.selected` for Refunds is unchanged (DS-72). | Fakes only. The same test goes back to the first session and asserts its line and offset to the pixel. |
| 6 | In Refunds, I click the ⤢ button on the panel header. | The panel fills the window. The title bar reads "Show the terminal". | `AppAction::ToggleFullScreen(Panel)` sets `full_screens[Refunds] = Panel` (`features/app/src/lib.rs:1150-1170`). | Fakes only for the expand-and-return path (`features/app/tests/presentation.rs::expanding_either_area_and_returning_preserves_the_columns`). |
| 7 | I switch to Ledger. | Ledger is not full screen. It shows the columns. | `full_screen()` reads `full_screens[Ledger]`, which is empty (DS-70, `features/app/src/lib.rs:356-363`). | Code, no test for the switch. |
| 8 | I switch back to Refunds. | Refunds is still full screen. | `full_screens[Refunds] = Panel` is still set (DS-70). | Code, no test for the switch. |
| 9 | In Refunds, I close the panel with ⌥⌘0 or ✕. | The panel and full screen both close. Refunds shows the terminal. | `SetPanel(None)` removes `full_screens[Refunds]` (`features/app/src/lib.rs:1218-1224`, DS-71). | Fakes only. `features/app/tests/presentation.rs::closing_the_full_screen_panel_returns_to_the_terminal`. |
| 10 | I open the Changes panel again in Refunds. | The file list is back. The file I had open is still open, and no read of the whole diff appears on the screen. | Opening the panel dispatches `ReviewAction::Open`, which starts a fetch of every file (`features/review/src/lib.rs:897-910`). The "don't read again" rule is not met (DS-76). | Fakes only. Built (RV-3, DS-76): `review_ui.rs::closing_and_reopening_the_panel_sends_no_fetch`, `::full_screen_and_back_sends_no_fetch`; `features/review/tests/open_again.rs::opening_again_with_the_diff_held_reads_nothing`, `::try_again_and_a_base_pick_still_read`. Only a base pick, Try again, a refresh or an old diff reads (`features/review/src/reopen.rs`). |
| 11 | I leave the panel open and do nothing for 30 seconds. | The app is still. | Expected: idle CPU is about 0% with the panel open (P-8). | Code, no test. Idle CPU (P-8) was measured by hand in `DIFFVIEW-report.md` "What works"; DV-14's live rerun has not been run. No automated check. |
| 12 | I quit the app with Refunds on `src/client.rs`, then open it again. | Refunds comes back with `src/client.rs` selected (DS-72). Its scroll starts at the top, since scroll is not saved. | `ReviewSaved.selected` is written to disk and read back (`features/review/src/lib.rs:102-108`). The scroll lives only in memory (`document_tab.rs:553`), so it is lost. | Fakes only for the selection (`features/app/tests/presentation.rs::a_restart_restores_both_tab_groups_and_the_selected_file`). The scroll is not saved, so it starts at the top; that has no test. |

## Branches

### Two sessions on the same repo and file, and one is scrolled

This is step 4. It happens whenever two sessions share a repo and a path in their diffs, which is common when two agents touch the same file.
When: I scroll one session's file, then open the same file in the other session.
What I do: I open the other session's Changes panel and click the same file.
What I see: the other session's file opens at the first session's scroll position (DS-73).
Expected outcome: each session starts at its own scroll. Scroll is kept per session and per file.
**Status:** Fakes only. `desktop/tests/native/review_ui.rs::two_sessions_on_one_file_keep_their_own_scroll`. Likely bug cleared (RV-3).

### Full screen in one session, and the other session is in the columns

This is steps 6 to 8. It happens when I use full screen in one session and then switch to another.
When: I use full screen in Refunds, then switch to Ledger.
What I do: I switch to Ledger, then back to Refunds.
What I see: Ledger shows the columns. Refunds is still full screen (DS-70).
Expected outcome: full screen is kept per session, so the switch restores each session's own state.
**Status:** Code, no test for the switch. `features/app/tests/presentation.rs::expanding_either_area_and_returning_preserves_the_columns` saves and restores `full_screens` per session; no test switches sessions while one is full.

### Restart while the panel is open and the file is gone

When: I quit with `src/client.rs` selected, then the file is deleted from the repo before I open the app again.
What I do: I open the app and the session.
What I see: the app does not select a file that is no longer in the diff. The list shows without a selected file.
Expected outcome: a saved selection is only restored when the file is still in the session's diff.
**Status:** Code, no test. `features/review/src/lib.rs` (`load_saved`) checks that the file is still in the diff before it stores the selection; the restore path is not tested for a missing file.

### Restart with full screen on

When: I quit while Refunds is in full screen, then reopen the app.
What I do: I open the app and the Refunds session.
What I see: expected is that full screen comes back as it was. The code only sets `full_screens` from `view.widened` on load (`features/app/src/lib.rs:296-302`). A plain full-screen Panel state is kept in `saved.full_screens` (`features/app/src/lib.rs:243`, `:340`).
**Status:** Fakes only. `features/app/tests/presentation.rs::expanding_either_area_and_returning_preserves_the_columns` saves with the Panel full screen on, relaunches with `AppState::launch`, and asserts `full_screen()` is `Panel` again.

### Toggling the panel while its read is still in flight

When: I open the panel, then close and reopen it before the first read finishes.
What I do: I click the panel toggle twice quickly.
What I see: no second read starts while the first is in flight (`features/review/src/lib.rs:897-910`, "Opening again while its read is in flight asks for nothing new").
**Status:** Fakes only. `features/review/tests/open_again.rs::opening_again_with_the_diff_held_reads_nothing`.

## Expected outcomes

- [ ] Each session keeps its own open file across a restart (DS-72).
- [ ] Each session keeps its own full-screen state. Switching sessions does not change the other session's full screen (DS-70).
- [ ] Closing the panel ends full screen for that session (DS-71).
- [ ] Two sessions on the same file each keep their own scroll position (DS-73). Not met today.
- [ ] Opening the Changes panel again does not read every file again (DS-76). Not met today.
- [ ] A panel left open does not use CPU while nothing changes (P-8).
- [ ] After a restart, the saved file is selected only if it is still in the diff.

## Status table

Counts (17 rows, by each row's first tag): Works (real shell) 0; Works (real daemon) 0; Fakes only 12; Code, no test 5; Not built 0; Likely bug 0.

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Select a file in a session | Fakes only | `features/app/tests/presentation.rs::a_restart_restores_both_tab_groups_and_the_selected_file` |
| 2 Scroll a file | Fakes only | `review_ui.rs::two_sessions_on_one_file_keep_their_own_scroll` |
| 3 Open another session's panel | Fakes only | the same test |
| 4 Same file in another session | Fakes only | the same test (Likely bug cleared, RV-3) |
| 5 Back to the first session | Fakes only | the same test, to the pixel |
| 6 Full screen in a session | Fakes only | `features/app/tests/presentation.rs::expanding_either_area_and_returning_preserves_the_columns` |
| 7 Other session not full screen | Code, no test | `features/app/src/lib.rs` (`full_screens`, DS-70) |
| 8 Back, still full screen | Code, no test | `features/app/src/lib.rs` (`full_screens`, DS-70) |
| 9 Close the panel ends full screen | Fakes only | `features/app/tests/presentation.rs::closing_the_full_screen_panel_returns_to_the_terminal` (DS-71) |
| 10 Reopen the panel without a full read | Fakes only | `review_ui.rs::closing_and_reopening_the_panel_sends_no_fetch`; `features/review/tests/open_again.rs` (DS-76) |
| 11 Idle CPU about 0% | Code, no test | `DIFFVIEW-report.md` "What works" (manual) (P-8) |
| 12 Restart keeps the selected file | Fakes only (selection); scroll reset is Code, no test | `features/review/src/lib.rs::ReviewSaved` (DS-72) |
| Two sessions, one file | Fakes only | `review_ui.rs::two_sessions_on_one_file_keep_their_own_scroll` |
| Full screen in one session, other in columns | Code, no test | `features/app/src/lib.rs` (`full_screens`) |
| Selection gone after restart | Code, no test | `features/review/src/lib.rs` (`load_saved`) |
| Full screen after restart | Fakes only | `presentation.rs::expanding_either_area_and_returning_preserves_the_columns` |
| Panel toggle while in flight | Fakes only | `features/review/tests/open_again.rs::opening_again_with_the_diff_held_reads_nothing` |

## Gaps and likely bugs

- Nothing is Not built or a Likely bug in this case after the audit. RV-3 landed: the shared scroll is fixed (`session_diff_view`) and opening the panel again sends no fetch.
- Gap in tests: switching sessions while one is full screen (steps 7 and 8), the idle CPU (step 11), a restart with the selected file gone, and the scroll reset after a restart.
- The step narratives for 4 and 10 describe the 2026-11-18 code (a shared scroll, a refetch on every open); only the Status cells and the counts were re-audited.

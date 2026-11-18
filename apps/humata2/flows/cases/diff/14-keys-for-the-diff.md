# I can drive the diff from the keyboard: full screen, next file, next change, wrap, back

Layer: Diff and review. Platforms: macOS desktop.

## Starting state

- Machine `box-1` has `acme/api` and `acme/ledger`. The session "Refunds" has changes in both repos, with four files in `acme/api` and two in `acme/ledger`.
- The app is open on "Refunds". The Changes panel is open beside the terminal. The file `acme/api src/client.rs` is open in the code area, and the focus is in the code area.
- No menu, sheet or comment box is open.
- The keyboard layout is a Mac layout, so ⌥ is Option.

## Reproduce headless

- Shortcut sheet and bindings: `bazel test //apps/humata2/ui:ui_unit_test --test_filter=keys` (the unit tests in `ui/src/keys.rs`, such as `off_a_mac_every_command_part_of_the_shortcuts_sheet_is_a_bound_chord`). These check the sheet and the labels, not whether a key does anything.
- Real-daemon key presses: `bazel test //apps/humata2/desktop:live_flows_test --test_filter=tabs_and_palette` (⇧⌘↩ and ⌥⌘0 with a real `humatad2`), and `--test_filter=navigation_keys` (⌥⌘← and ⌥⌘→ in the sidebar).
- Headless window: `bazel test //apps/humata2/desktop:native_test --test_filter=diff_view` drives the diff panel with `window.press(...)` in some files.
- Not headless: how the focus ring looks, and what a real Option key types into a text box. Those need the screen.
- No window goes on the owner's screen. Use the native target.

## Steps

| # | I do | I see | State and events | Status |
|---|---|---|---|---|
| 1 | I press ⌘/ to open the shortcuts sheet and look for full screen. | The sheet lists ⌘[ Back, Esc, ⌥⌘0 and others. It lists no full screen key for the diff, and no ⇧⌘↩ for the terminal. | `SHORTCUTS` in `ui/src/keys.rs:84-105` has no ⇧⌘↩ entry, although `keys.rs:223` binds `shift-cmd-enter` to `TerminalFullScreen`. | Code, no test. The sheet is missing a bound key; no test checks that every binding is listed. |
| 2 | With the diff open, I press ⇧⌘↩. | The terminal goes full screen. The diff is not full screen. The title bar reads "Show the terminal". | `shift-cmd-enter` is `TerminalFullScreen` with no context (`keys.rs:223`). The session's full screen is the terminal (DS-17 says the session header's arrow is ⇧⌘↩). | Works (real daemon). `desktop/tests/live_flows/tabs_and_palette.rs::the_tab_row_opens_and_closes_files_and_the_panel_and_full_screen_follow_their_keys`. |
| 3 | I press ⇧⌘↩ again. | The columns come back. | `full_screen()` clears (`features/app/src/lib.rs:356-363`). | Works (real daemon). Same test as step 2. |
| 4 | With the diff focused, I press ⌥⌘↩. | Nothing happens. The diff does not go full screen. | No `alt-cmd-enter` binding in `ui/src/keys.rs:198-237` (DS-12). | Not built (DV-18). No test presses this key. |
| 5 | I double-click the Changes panel header. | Nothing happens. | No double-click handler on the panel header (DS-12). `ui/src/panels.rs:150-163` only has the ⤢ button. | Not built (DV-18). |
| 6 | I click the ⤢ button on the panel header. | The diff fills the window. The title bar reads "Show the terminal". | `ToggleFullScreen(Panel)` (`features/app/src/lib.rs:1150-1170`). | Fakes only. `features/app/tests/presentation.rs::expanding_either_area_and_returning_preserves_the_columns`. |
| 7 | In full screen, I press Esc with nothing open. | The diff stays full screen. | `escape()` in `features/app/src/overlay.rs:56` closes overlays and forms. It never touches `full_screens`. | Not built (DV-18, DS-13). |
| 8 | In full screen, I press Esc with a menu open, then Esc again. | The first Esc closes the menu. The second Esc leaves full screen. | Expected order: menu, sheet or comment box first, then full screen (DS-13). The second Esc does nothing today. | Not built (DV-18). The first half works: `desktop/tests/live_flows/tabs_and_palette.rs:293-296` (palette and sheet close on Esc). |
| 9 | I press ⌥⌘↓ with a file open. | Nothing happens. I do not move to the next file. | No `alt-cmd-down` binding in `keys.rs` (DS-2, DS-84). Spec §3 lists it for next file. | Not built (DV-15). Conflict: spec §8 item 1 reserves ⌥⌘↓ for files, and STACKVIEW T8 also wants it for layers. |
| 10 | I press ⌥⌘↑. | Nothing happens. | No `alt-cmd-up` binding (DS-84). | Not built (DV-15). Same conflict as step 9. |
| 11 | I press ⇧⌥⌘↓ with a long file open. | Nothing happens. I do not go to the next change. | No binding (DS-27). The file header has no "Change N of M" control. | Not built (DV-15). |
| 12 | I press ⌥⌘] or ⌥⌘[ with a stack shown. | Nothing happens. | No binding in `keys.rs`. The layer keys are proposed in spec §8 item 1. | Not built (STACKVIEW T8). |
| 13 | With the code area focused, I press ⌥Z. | Nothing changes in the diff. If a text box has focus, it types Ω. | No `alt-z` binding (DS-40). Option is not Alt in the terminal (`terminal/vt/src/terminal.rs:223`, `OptionAsAlt::Off`), so the key is typed. Spec §8 item 2: bind ⌥Z only in the diff's own context. | Not built (DV-17). |
| 14 | I click the file header's Wrap / Scroll switch. | The long lines wrap or scroll. | The switch is not built. Settings > Long lines does the same job (`ui/src/settings.rs:515-545`). `ReviewAction::ToggleWrap` exists but nothing sends it (DS-40). | Partly built. Settings only: `desktop/tests/native/review.rs::wrapped_changes_align_the_gutter_and_switch_back_to_one_scrolling_row`. |
| 15 | I press ↑ and ↓ with the file list focused. | The highlighted file moves. | The sidebar's ↑ ↓ bindings are in the Sidebar context (`keys.rs:225-233`). The file list is not in that context. | Not built (DV-15, DS-84). |
| 16 | I press ⌘[ on a session page. | I go back to the page I came from. | `cmd-[` is `Back` (`keys.rs:204`). No test presses ⌘[; the action is tested in `features/app/tests/navigation.rs` (NV-3, NV-4). | Fakes only for the action. The key press is Code, no test. |
| 17 | I press ⌥⌘→ with the focus in the file list. | Nothing happens. The sidebar does not open every channel. | `alt-cmd-right` is bound only in the Sidebar context (`keys.rs:211`). Spec §3 says the new arrows need a session context so they do not shadow these. | Works (real daemon) in the sidebar: `desktop/tests/live_flows/navigation_keys.rs:194-197`. The panel case is not tested. |
| 18 | I press ⌥⌘0. | The Changes panel hides. Press again and it shows. | `alt-cmd-0` is `TogglePanel` (`keys.rs:219`). | Works (real daemon). `desktop/tests/live_flows/tabs_and_palette.rs:211-221`. |

## Branches

### A key I press in the terminal

When: the terminal has focus, and I press a key that the diff would use, such as ⌥⌘↩ or ⌥Z.
What I do: I press the key with the terminal focused.
What I see: the terminal gets the key. ⌥Z types Ω, because Option is not Alt there (`terminal/vt/src/terminal.rs:223`).
Expected outcome: ⌥⌘↩ and ⇧⌘↩ are app keys and work in the terminal. ⌥Z is bound only in the diff's own context (spec §8 item 2), so it still types Ω in the terminal.
**Status:** Not built for ⌥⌘↩ and ⌥Z. ⇧⌘↩ works in the terminal: `keys.rs:223` binds it with no context, and `tabs_and_palette.rs` presses it with the terminal in use (Works (real daemon)).

### A menu or sheet is open in full screen

When: full screen is on and a menu, sheet or comment box is open.
What I do: I press Esc.
What I see: the menu or sheet closes. Full screen stays.
Expected outcome: Esc closes the menu, sheet or comment box first. A second Esc leaves full screen (DS-13).
**Status:** Not built. The first Esc works (`tabs_and_palette.rs:293-296`). The second Esc does not leave full screen (`features/app/src/overlay.rs:56`).

### ⌥⌘← and ⌥⌘→ with the diff focused

When: the diff has focus, and I press ⌥⌘→ or ⌥⌘←.
What I do: I press the key.
What I see: the sidebar does not change. The diff does not change either.
Expected outcome: these arrows stay with the sidebar (`keys.rs:211-212`). The four new ⌥⌘ arrows must not take them over (spec §3).
**Status:** Works (real daemon) in the sidebar (`navigation_keys.rs:194-197`). The diff-focus case is Code, no test.

### The key that meant two things

When: I press ⌥⌘↓ with a file open, while a stack is also shown.
What I do: I press ⌥⌘↓.
What I see: today, nothing. If the file keys are built first, ⌥⌘↓ goes to the next file, and the layer keys must use ⌥⌘] and ⌥⌘[ (spec §8 item 1).
**Status:** Not built. Settled in the spec, not yet in code.

## Expected outcomes

- [ ] ⇧⌘↩ turns the terminal full screen and back, with the terminal focused or not (DS-17).
- [ ] ⌥⌘↩ turns the diff full screen and back, with the panel open (DS-12, DS-13).
- [ ] Esc closes a menu, sheet or comment box first, then leaves full screen (DS-13).
- [ ] ⌥⌘↓ and ⌥⌘↑ move to the next and previous file (DS-2, DS-84).
- [ ] ⇧⌥⌘↓ and ⇧⌥⌘↑ move to the next and previous change (DS-27).
- [ ] ⌥Z turns Wrap and Scroll on and off, with the diff focused only (DS-40, spec §8 item 2).
- [ ] ⌘[ goes back to where I was (DS-58).
- [ ] ↑ ↓ and ↩ move through the file list and open a file (DS-84).
- [ ] The ⌘/ sheet lists every key above that works, including ⇧⌘↩.

## Status table

Counts (22 rows, by each row's first tag): Works (real shell) 0; Works (real daemon) 6; Fakes only 2; Code, no test 1; Not built 13; Likely bug 0.

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Shortcuts sheet lists ⇧⌘↩ | Code, no test | `ui/src/keys.rs:84-105` (no ⇧⌘↩ entry) |
| 2 ⇧⌘↩ opens terminal full screen | Works (real daemon) | `desktop/tests/live_flows/tabs_and_palette.rs::the_tab_row_opens_and_closes_files_and_the_panel_and_full_screen_follow_their_keys` |
| 3 ⇧⌘↩ again | Works (real daemon) | same test |
| 4 ⌥⌘↩ diff full screen | Not built | DS-12, DV-18; no binding in `keys.rs` |
| 5 Double-click the header | Not built | DS-12, DV-18; `ui/src/panels.rs:150-163` |
| 6 ⤢ button full screen | Fakes only | `features/app/tests/presentation.rs::expanding_either_area_and_returning_preserves_the_columns` |
| 7 Esc leaves full screen | Not built | DS-13, DV-18; `features/app/src/overlay.rs:56` |
| 8 Esc order: menu first, then full screen | Not built (first half works) | DS-13, DV-18; `tabs_and_palette.rs:293-296` |
| 9 ⌥⌘↓ next file | Not built | DS-2, DV-15; `keys.rs` (no binding); spec §8 item 1 |
| 10 ⌥⌘↑ previous file | Not built | DS-84, DV-15 |
| 11 ⇧⌥⌘↓ next change | Not built | DS-27, DV-15 |
| 12 ⌥⌘] ⌥⌘[ layers | Not built | STACKVIEW T8; spec §8 item 1 |
| 13 ⌥Z wrap | Not built | DS-40, DV-17; `terminal/vt/src/terminal.rs:223`; spec §8 item 2 |
| 14 Wrap switch in the header | Partly built (Settings only) | `desktop/tests/native/review.rs::wrapped_changes_align_the_gutter_and_switch_back_to_one_scrolling_row` |
| 15 ↑ ↓ in the file list | Not built | DS-84, DV-15 |
| 16 ⌘[ goes back | Fakes only (action); key Code, no test | `features/app/tests/navigation.rs` (NV-3, NV-4); `keys.rs:204` |
| 17 ⌥⌘→ stays with the sidebar | Works (real daemon) | `desktop/tests/live_flows/navigation_keys.rs:194-197` |
| 18 ⌥⌘0 hides the panel | Works (real daemon) | `desktop/tests/live_flows/tabs_and_palette.rs:211-221` |
| Terminal branch: ⇧⌘↩ | Works (real daemon) | `tabs_and_palette.rs` (step 2) |
| Terminal branch: ⌥Z | Not built | DV-17; `terminal/vt/src/terminal.rs:223` |
| Menu open in full screen | Not built | DS-13, DV-18 |
| Diff focused, ⌥⌘← and → | Works (real daemon) in the sidebar; diff focus is Code, no test | `navigation_keys.rs:194-197`; `keys.rs:211-212` |
| Key that means two things | Not built | spec §8 item 1 |

## Gaps and likely bugs

- Code, no test: the ⌘/ sheet does not list ⇧⌘↩ (`ui/src/keys.rs:84-105`). A test checks that each sheet entry is bound, not the reverse. Add ⇧⌘↩ to `SHORTCUTS`, and add a test that every binding is listed.
- Not built: ⌥⌘↩, the header double-click, Esc to leave full screen, ⌥⌘↓ and ⌥⌘↑, ⇧⌥⌘↓ and ⇧⌥⌘↑, ⌥Z, and the file-list keys (DV-15, DV-17, DV-18). These are all in spec §3.
- Not built, conflict: ⌥⌘↓ and ⌥⌘↑ are named for files in the spec and for layers in STACKVIEW T8. The spec settles it for files, and layers move to ⌥⌘] and ⌥⌘[ (spec §8 item 1). STACKVIEW T8's test "alt-cmd-down picks layer 1" must change with it.
- Not built, conflict: ⌥Z is typed as Ω in the terminal and in text boxes (`terminal/vt/src/terminal.rs:223`). Bind it only in the diff's key context (spec §8 item 2).
- Code, no test: ⌘[ is not pressed by any test. Its action is tested in `features/app/tests/navigation.rs`.
- Code, no test: the diff-focus case for ⌥⌘← and ⌥⌘→ (step 17). The sidebar case is tested.
- Not built: the Wrap / Scroll switch on the file header (DS-40). Only the Settings switch exists.

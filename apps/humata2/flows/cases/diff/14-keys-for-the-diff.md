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
| 1 | I press ⌘/ to open the shortcuts sheet and look for full screen. | The sheet lists ⌘[ Back, Esc, ⌥⌘0 and others. It lists no full screen key for the diff, and no ⇧⌘↩ for the terminal. | `SHORTCUTS` in `ui/src/keys.rs:84-105` has no ⇧⌘↩ entry, although `keys.rs:223` binds `shift-cmd-enter` to `TerminalFullScreen`. | Fakes only. Built (RV-5): `SHORTCUTS` in `ui/core/src/keys.rs` lists ⇧⌘↩ (Terminal full screen) and ⌃⌘↩ (Side panel full screen), ⌥⌘↓ ⌥⌘↑, ⌃⌘↓ ⌃⌘↑, ⌥⌘] ⌥⌘[, ⌥Z, ⌘[ and the file-list keys. `keys::tests::every_diff_key_is_in_the_shortcuts_sheet` checks every diff binding's chord against the sheet; the older test checks sheet entries are bound. Esc is listed as "Close or cancel", and a header double-click is not listed. |
| 2 | With the diff open, I press ⇧⌘↩. | The terminal goes full screen. The diff is not full screen. The title bar reads "Show the terminal". | `shift-cmd-enter` is `TerminalFullScreen` with no context (`keys.rs:223`). The session's full screen is the terminal (DS-17 says the session header's arrow is ⇧⌘↩). | Works (real daemon). `desktop/tests/live_flows/tabs_and_palette.rs::the_tab_row_opens_and_closes_files_and_the_panel_and_full_screen_follow_their_keys`. |
| 3 | I press ⇧⌘↩ again. | The columns come back. | `full_screen()` clears (`features/app/src/lib.rs:356-363`). | Works (real daemon). Same test as step 2. |
| 4 | With the diff focused, I press ⌥⌘↩. | Nothing happens. The diff does not go full screen. | No `alt-cmd-enter` binding in `ui/src/keys.rs:198-237` (DS-12). | Fakes only. Built as ⌃⌘↩ (`ctrl-cmd-enter`; DIFFVIEW lane 17:05, because off a Mac ⌥⌘↩ clashes with ⇧⌘↩): `desktop/tests/native/diff_full_screen.rs::alt_cmd_enter_enters_and_leaves_full_screen` presses `ctrl-cmd-enter` twice. |
| 5 | I double-click the Changes panel header. | Nothing happens. | No double-click handler on the panel header (DS-12). `ui/src/panels.rs:150-163` only has the ⤢ button. | Fakes only. Built (DV-18): `desktop/tests/native/diff_full_screen.rs::double_clicking_the_panel_header_enters_full_screen`. |
| 6 | I click the ⤢ button on the panel header. | The diff fills the window. The title bar reads "Show the terminal". | `ToggleFullScreen(Panel)` (`features/app/src/lib.rs:1150-1170`). | Fakes only. `features/app/tests/presentation.rs::expanding_either_area_and_returning_preserves_the_columns`; `diff_full_screen.rs::full_screen_draws_one_header`. |
| 7 | In full screen, I press Esc with nothing open. | The diff stays full screen. | `escape()` in `features/app/src/overlay.rs:56` closes overlays and forms. It never touches `full_screens`. | Fakes only. Built (DV-18): `diff_full_screen.rs::escape_leaves_full_screen_but_not_while_the_base_menu_is_open`. |
| 8 | In full screen, I press Esc with a menu open, then Esc again. | The first Esc closes the menu. The second Esc leaves full screen. | Expected order: menu, sheet or comment box first, then full screen (DS-13). The second Esc does nothing today. | Fakes only. Built (DV-18, RV-5): the same test, and `diff_full_screen.rs::escape_closes_the_new_agent_tab_menu_first`. The first half also: `desktop/tests/live_flows/tabs_and_palette.rs` (palette). |
| 9 | I press ⌥⌘↓ with a file open. | Nothing happens. I do not move to the next file. | No `alt-cmd-down` binding in `keys.rs` (DS-2, DS-84). Spec §3 lists it for next file. | Fakes only. Built (DV-15): `alt-cmd-down` in `ui/core/src/keys.rs`; `desktop/tests/native/diff_view.rs::next_file_steps_across_repos`; `features/review/tests/stepping.rs`. Layers moved to ⌥⌘] / ⌥⌘[, so there is no conflict. |
| 10 | I press ⌥⌘↑. | Nothing happens. | No `alt-cmd-up` binding (DS-84). | Fakes only. Built (DV-15): `alt-cmd-up`; `features/review/tests/stepping.rs::next_and_previous_file_walk_the_files_across_repos_and_stop_at_the_ends`. |
| 11 | I press ⇧⌥⌘↓ with a long file open. | Nothing happens. I do not go to the next change. | No binding (DS-27). The file header has no "Change N of M" control. | Fakes only. Built as ⌃⌘↓ / ⌃⌘↑ (`ctrl-cmd-down`, `ctrl-cmd-up`; ⇧⌥⌘ would clash off a Mac): `features/review/tests/stepping.rs::next_and_previous_change_stay_within_the_files_hunks`; the stepper `diff_view.rs::a_long_file_shows_a_change_stepper`. |
| 12 | I press ⌥⌘] or ⌥⌘[ with a stack shown. | Nothing happens. | No binding in `keys.rs`. The layer keys are proposed in spec §8 item 1. | Fakes only. Built (STACKVIEW T7, T8): `alt-cmd-]` / `alt-cmd-[` in the `DIFF_STACK` context; `desktop/tests/native/stack.rs::the_next_layer_key_picks_the_bottom_layer_from_whole_stack`. |
| 13 | With the code area focused, I press ⌥Z. | Nothing changes in the diff. If a text box has focus, it types Ω. | No `alt-z` binding (DS-40). Option is not Alt in the terminal (`terminal/vt/src/terminal.rs:223`, `OptionAsAlt::Off`), so the key is typed. Spec §8 item 2: bind ⌥Z only in the diff's own context. | Fakes only. Built (DV-17, RV-5): ⌥Z is bound in the diff's key context only; `Input` rebinds it to nothing. `desktop/tests/native/review_ui.rs::option_z_in_the_comment_box_types_and_does_not_flip_wrap`. |
| 14 | I click the file header's Wrap / Scroll switch. | The long lines wrap or scroll. | The switch is not built. Settings > Long lines does the same job (`ui/src/settings.rs:515-545`). `ReviewAction::ToggleWrap` exists but nothing sends it (DS-40). | Fakes only. Built (DV-17): `diff_view.rs::the_wrap_switch_flips_long_lines` (the `diff-wrap` switch flips the Settings preference). |
| 15 | I press ↑ and ↓ with the file list focused. | The highlighted file moves. | The sidebar's ↑ ↓ bindings are in the Sidebar context (`keys.rs:225-233`). The file list is not in that context. | Fakes only. Built (DV-15, DS-84): `desktop/tests/native/diff_keys.rs::the_focused_file_list_takes_the_arrows_and_enter`, `::an_unfocused_file_list_ignores_the_arrows`; `features/review/tests/file_list_keys.rs`. |
| 16 | I press ⌘[ on a session page. | I go back to the page I came from. | `cmd-[` is `Back` (`keys.rs:204`). No test presses ⌘[; the action is tested in `features/app/tests/navigation.rs` (NV-3, NV-4). | Fakes only for the action (`features/app/tests/navigation.rs`, NV-3, NV-4). The key press is Code, no test: nothing presses ⌘[ in a session. |
| 17 | I press ⌥⌘→ with the focus in the file list. | Nothing happens. The sidebar does not open every channel. | `alt-cmd-right` is bound only in the Sidebar context (`keys.rs:211`). Spec §3 says the new arrows need a session context so they do not shadow these. | Works (real daemon) in the sidebar: `desktop/tests/live_flows/navigation_keys.rs`. The new arrows are bound with no context (`alt-cmd-down`, `alt-cmd-up`), but ⌥⌘→ and ⌥⌘← stay sidebar-only (`ui/core/src/keys.rs`); the diff-focus case has no test. |
| 18 | I press ⌥⌘0. | The Changes panel hides. Press again and it shows. | `alt-cmd-0` is `TogglePanel` (`keys.rs:219`). | Works (real daemon). `desktop/tests/live_flows/tabs_and_palette.rs:211-221`. |

## Branches

### A key I press in the terminal

When: the terminal has focus, and I press a key that the diff would use, such as ⌥⌘↩ or ⌥Z.
What I do: I press the key with the terminal focused.
What I see: the terminal gets the key. ⌥Z types Ω, because Option is not Alt there (`terminal/vt/src/terminal.rs:223`).
Expected outcome: ⌥⌘↩ and ⇧⌘↩ are app keys and work in the terminal. ⌥Z is bound only in the diff's own context (spec §8 item 2), so it still types Ω in the terminal.
**Status:** Works (real daemon) for ⇧⌘↩ in the terminal: `tabs_and_palette.rs`. ⌃⌘↩ is bound with no context like it (`ui/core/src/keys.rs`). ⌥Z is bound only in the diff's context, so the terminal keeps it as Ω; no test presses it with the terminal focused.

### A menu or sheet is open in full screen

When: full screen is on and a menu, sheet or comment box is open.
What I do: I press Esc.
What I see: the menu or sheet closes. Full screen stays.
Expected outcome: Esc closes the menu, sheet or comment box first. A second Esc leaves full screen (DS-13).
**Status:** Fakes only. `desktop/tests/native/diff_full_screen.rs::escape_leaves_full_screen_but_not_while_the_base_menu_is_open`; `::escape_closes_the_new_agent_tab_menu_first`.

### ⌥⌘← and ⌥⌘→ with the diff focused

When: the diff has focus, and I press ⌥⌘→ or ⌥⌘←.
What I do: I press the key.
What I see: the sidebar does not change. The diff does not change either.
Expected outcome: these arrows stay with the sidebar (`keys.rs:211-212`). The four new ⌥⌘ arrows must not take them over (spec §3).
**Status:** Works (real daemon) in the sidebar (`navigation_keys.rs`). The diff-focus case is Code, no test.

### The key that meant two things

When: I press ⌥⌘↓ with a file open, while a stack is also shown.
What I do: I press ⌥⌘↓.
What I see: today, nothing. If the file keys are built first, ⌥⌘↓ goes to the next file, and the layer keys must use ⌥⌘] and ⌥⌘[ (spec §8 item 1).
**Status:** Fakes only. Settled in code (RV-5): files keep ⌥⌘↓ / ⌥⌘↑ and layers use ⌥⌘] / ⌥⌘[ (`ui/core/src/keys.rs`); `stack.rs::the_next_layer_key_picks_the_bottom_layer_from_whole_stack`.

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

Counts (22 rows, by each row's first tag): Works (real shell) 0; Works (real daemon) 6; Fakes only 16; Code, no test 0; Not built 0; Likely bug 0.

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Shortcuts sheet lists the diff keys | Fakes only | `ui/core/src/keys.rs` `SHORTCUTS`; `keys::tests::every_diff_key_is_in_the_shortcuts_sheet` |
| 2 ⇧⌘↩ opens terminal full screen | Works (real daemon) | `desktop/tests/live_flows/tabs_and_palette.rs::the_tab_row_opens_and_closes_files_and_the_panel_and_full_screen_follow_their_keys` |
| 3 ⇧⌘↩ again | Works (real daemon) | same test |
| 4 ⌃⌘↩ (was ⌥⌘↩) diff full screen | Fakes only | `diff_full_screen.rs::alt_cmd_enter_enters_and_leaves_full_screen` |
| 5 Double-click the header | Fakes only | `diff_full_screen.rs::double_clicking_the_panel_header_enters_full_screen` |
| 6 ⤢ button full screen | Fakes only | `features/app/tests/presentation.rs::expanding_either_area_and_returning_preserves_the_columns` |
| 7 Esc leaves full screen | Fakes only | `diff_full_screen.rs::escape_leaves_full_screen_but_not_while_the_base_menu_is_open` |
| 8 Esc order: menu first, then full screen | Fakes only | the same test; `::escape_closes_the_new_agent_tab_menu_first` |
| 9 ⌥⌘↓ next file | Fakes only | `diff_view.rs::next_file_steps_across_repos`; `features/review/tests/stepping.rs` |
| 10 ⌥⌘↑ previous file | Fakes only | `features/review/tests/stepping.rs::next_and_previous_file_walk_the_files_across_repos_and_stop_at_the_ends` |
| 11 ⌃⌘↓ next change (was ⇧⌥⌘↓) | Fakes only | `features/review/tests/stepping.rs::next_and_previous_change_stay_within_the_files_hunks` |
| 12 ⌥⌘] ⌥⌘[ layers | Fakes only | `desktop/tests/native/stack.rs::the_next_layer_key_picks_the_bottom_layer_from_whole_stack` |
| 13 ⌥Z wrap | Fakes only | `review_ui.rs::option_z_in_the_comment_box_types_and_does_not_flip_wrap` |
| 14 Wrap switch in the header | Fakes only | `diff_view.rs::the_wrap_switch_flips_long_lines` |
| 15 ↑ ↓ in the file list | Fakes only | `desktop/tests/native/diff_keys.rs::the_focused_file_list_takes_the_arrows_and_enter` |
| 16 ⌘[ goes back | Fakes only (action); key Code, no test | `features/app/tests/navigation.rs` (NV-3, NV-4); `ui/core/src/keys.rs` |
| 17 ⌥⌘→ stays with the sidebar | Works (real daemon) | `desktop/tests/live_flows/navigation_keys.rs` |
| 18 ⌥⌘0 hides the panel | Works (real daemon) | `desktop/tests/live_flows/tabs_and_palette.rs` |
| Terminal branch: ⇧⌘↩ | Works (real daemon) | `tabs_and_palette.rs` (step 2); ⌥Z in the terminal has no test |
| Menu open in full screen | Fakes only | `diff_full_screen.rs::escape_leaves_full_screen_but_not_while_the_base_menu_is_open` |
| Diff focused, ⌥⌘← and → | Works (real daemon) in the sidebar; diff focus is Code, no test | `navigation_keys.rs`; `ui/core/src/keys.rs` |
| Key that means two things | Fakes only | `stack.rs::the_next_layer_key_picks_the_bottom_layer_from_whole_stack`; `diff_view.rs::next_file_steps_across_repos` |

## Gaps and likely bugs

- Nothing is Not built or a Likely bug in this case after the audit. DV-15, DV-17, DV-18, RV-5 and STACKVIEW T7 and T8 landed. The chords differ from the spec: ⌃⌘↩ for the diff's full screen and ⌃⌘↓ / ⌃⌘↑ for changes (DIFFVIEW lane, 17:05 and 17:45), so each ⌥⌘ chord off a Mac stays distinct.
- Fakes only: `keys::tests::every_diff_key_is_in_the_shortcuts_sheet` checks every diff binding's chord against the SHORTCUTS sheet.
- Code, no test: ⌘[ is not pressed by any test; ⌥Z with the terminal focused; ⌥⌘← and ⌥⌘→ with the diff focused.
- The step narratives above ("no binding", "Nothing happens") describe the 2026-11-18 code; only the Status cells and the counts were re-audited.

# From a focused terminal, the palette moves a session, reaches Settings and opens a file

Micro. Platforms: macOS and Linux. Linux is noted where it differs. Web is not covered by this case.

## Starting state

- Humata 2 is open on this Mac. It is connected to the local daemon, and the design fixture data is loaded.
- The session "API rate limiter" is open. It is in channel `payments-rewrite` and runs on `devbox-1`.
- The same channel also lists `gpu-box` (online) and `cloud-vm-3` (offline).
- The session's clone has `acme/api` `src/client.rs` in its files.
- The session's terminal is focused and shows a shell prompt. Nothing is typed yet.
- The palette is closed. No other overlay is open.

## Steps

1. **I do:** press ⌘K (Ctrl+K on Linux) with the terminal focused.
   **I see:** the command palette opens over the session. Its search box has focus and is empty.
   **State and events:** the palette opens with the session as its context. Its overlay is on top of the stack. The key press does not reach the shell.
   **Status:** Fakes only
   Evidence: `desktop/tests/native/terminal.rs::an_unfocused_terminal_does_not_receive_palette_keystrokes` opens the palette with `AppAction::OpenPalette`, not with the key. The key binding is `ui/src/keys.rs:94` (`cmd-k`). A ⌘ key is passed on to the app by `terminal/view/src/keys.rs:65`. Unproven live: a real ⌘K press while the terminal is focused. The live test `desktop/tests/live_flows/find.rs::from_the_palette_...` presses `secondary-k` on the session page, without focusing the terminal.

2. **I do:** type `mov`.
   **I see:** the search box shows `mov`. The shell prompt does not change, and no text appears in the terminal.
   **State and events:** the palette query is `mov`. The terminal gets no bytes while the palette has focus.
   **Status:** Fakes only
   Evidence: `desktop/tests/native/terminal.rs::an_unfocused_terminal_does_not_receive_palette_keystrokes` types `focus stays in search` and presses `down`, then checks that the terminal's bytes did not change. The same test then clicks the terminal and checks that typing reaches it. Unproven live: typing into the palette while a real terminal is focused.

3. **I do:** look at the first row of the list.
   **I see:** `Move session to another machine…` is first, with the hint `⌥⌘M` on its right.
   **State and events:** the entry is `Command::MoveSession`. A session whose title also matches is still listed, after it.
   **Status:** Works (real daemon)
   Evidence: `desktop/tests/live_flows/find.rs::from_the_palette_a_session_moves_a_turn_is_found_a_pr_reviews_settings_open_and_a_pin_holds` asserts that `MoveSession` is first for `move` and that its hint is not empty, on a real daemon. The live test types `move`, not `mov`. The `mov` prefix itself is not tested. The fixture test is `features/palette/tests/act.rs::typing_move_puts_the_sessions_move_first_with_its_shortcut`.

4. **I do:** press Enter on Move.
   **I see:** the palette stays open and its list now holds only machines. Each machine is online, is in the channel, and is not where the session runs. In this fixture that is `this-mac` and `gpu-box`. `devbox-1` and the offline `cloud-vm-3` are not listed. The query is cleared.
   **State and events:** `PaletteState.asking` is `Ask::MoveSession` for this session. `Run` does not close the palette, and nothing is sent to the app.
   **Status:** Works (real daemon)
   Evidence: `desktop/tests/live_flows/find.rs::from_the_palette_...` waits for `asking` to be set, and checks that every offered command is `MoveTo` and not the current machine. The exact online and channel filter is checked only by fixtures: `features/palette/tests/act.rs::running_move_asks_inline_for_another_online_machine_of_the_channel`.

5. **I do:** type `gpu`, then press Enter.
   **I see:** the list narrows to `gpu-box`. After Enter the palette closes, and the session's Move starts to `gpu-box`. The session shows that it is moving.
   **State and events:** the palette delegates `Closed`, then `Run(MoveTo { session, machine: gpu-box })`. The app opens the session, starts the move, picks the machine and confirms it, all in one step (`features/app/src/commands.rs:142-146`). The user sees no separate Move dialog.
   **Status:** Fakes only
   Evidence: `features/palette/tests/act.rs::typing_while_asking_narrows_the_machines` and `picking_gpu_box_hands_the_move_to_the_app` (reducer only). `features/app/tests/move_from_palette.rs::moving_a_channel_session_from_the_palette_asks_the_daemon` covers the app side with fakes. No live test moves a session from the palette. Unproven live: the move itself on a real daemon.

6. **I do:** press Escape while the machine list is shown.
   **I see:** the palette is still open. The list is back to the `mov` results, and `Move session to another machine…` is first again. The search box shows `move`.
   **State and events:** `asking` is cleared and the query is restored (`features/palette/src/lib.rs:337-341`). Nothing is run.
   **Status:** Works (real daemon)
   Evidence: `desktop/tests/live_flows/find.rs::from_the_palette_...` presses Escape while asking. It checks that `asking` is none, that the palette is open, and that the query is `move`. The live test does not check the restored list beyond the first row. Its first row check is `MoveSession` first.

7. **I do:** press Escape again.
   **I see:** the palette closes. The session page is still in front.
   **State and events:** the palette delegates `Closed`. The route stays `Session(rate)`.
   **Status:** Works (real daemon)
   Evidence: `desktop/tests/live_flows/find.rs::close_palette` sends Escape and waits for the palette to close. The route check is in `from_the_palette_...` after the first close. Fixture: `features/palette/tests/act.rs::escape_while_asking_steps_back_without_running_anything`.

8. **I do:** paste the URL of a pull request that a session here reviews, for example `https://github.com/acme/api/pull/123`.
   **I see:** the first row is `Review this PR`, with the hint `acme/api #123`.
   **State and events:** the palette offers `Command::ReviewPull`, and Enter hands it to the app. A pull request that no session here reviews is not offered as `Review this PR`. Only `Open pull request #123` is offered for the number. This is the rule in D197 (`features/palette/src/lib.rs`, `command_entries`, around line 503).
   **Status:** Fakes only
   Evidence: `features/palette/tests/review_pull_gate.rs::a_pull_request_a_session_reviews_is_offered_first`, and `features/palette/tests/act.rs::a_pasted_pull_request_url_offers_review_this_pr_first` (both URL and `owner/repo#n` forms). The live test `from_the_palette_...` checks the negative case only: an unreviewed PR offers no `ReviewPull`. Unproven live: a reviewed PR, and a real paste. The live test notes that ⌘V does not reach the palette input in the headless window, so it types the URL.

9. **I do:** clear the search and type `long`, then press Enter on `Settings › Long lines`.
   **I see:** Settings opens. Long lines is the revealed row.
   **State and events:** the route is `Settings` and `settings.revealed` is `LongLines`. The palette is closed.
   **Status:** Works (real daemon)
   Evidence: `desktop/tests/live_flows/find.rs::from_the_palette_...` (step 4 of that test) types `long` and clicks the row, then waits for `Route::Settings` and `Some(LongLines)`. The Enter path is the same `Run` action as the fixture test `features/palette/tests/act.rs::typing_long_opens_settings_at_long_lines`, which picks by index. Enter itself is not tested live.

10. **I do:** type `client.rs`, then press Enter on the file row `acme/api/src/client.rs`.
    **I see:** the session opens with its file view showing `src/client.rs`. The row's hint is the session's title.
    **State and events:** the palette lists files of the context session only (`features/palette/src/lib.rs:835-837`). The entry is `Command::File { session, repo, path }`. The app navigates to the session, selects the file and opens it.
    **Status:** Fakes only
    Evidence: `desktop/tests/native/overlays.rs::palette_file_results_open_the_current_sessions_clone`. It asserts the `session-file` element and that the open session is `ledger` (the fixture's id for this session). It does not check the clone path. Unproven live: opening a file from the palette against a real daemon and clone.

## Branches

### No other machine is online for Move

- **When:** the channel has no online machine other than the one the session runs on. For example, `gpu-box` is offline.
- **I do:** press ⌘K, type `move`, press Enter.
- **I see:** the palette is asking for a machine, but the list is empty. The query is cleared, so the empty-state line reads `Nothing matches. Try fewer letters.` (`ui/src/overlay_palette.rs:191`). Nothing says that no other machine is online, and nothing says what to do about it.
- **Expected outcome:** the palette says that no other machine of this channel is online, and says how to fix it (bring a machine online, or move from a machine that is online). Escape still steps back and closes.
- **Status:** Likely bug

### Gibberish matches nothing

- **When:** the query matches no command, session, file, channel or conversation.
- **I do:** type `zqxvw`, then press Enter.
- **I see:** the list is empty, with the line `Nothing matches “zqxvw”. Try fewer letters or fewer words.` (`ui/src/overlay_palette.rs:182-191`). Enter does nothing and the palette stays open.
- **Expected outcome:** the empty-state line is shown, nothing runs, and the palette stays open until Escape.
- **Status:** Code, no test
  Searched for `Nothing matches` in `desktop/tests`, `features/*/tests` and `ui/src`. No test asserts the line or Enter on an empty list.

### Palette opened while another overlay is open

- **When:** the shortcuts sheet (⌘/) is open and I press ⌘K.
- **I do:** press ⌘/, then ⌘K, then Escape, then Escape.
- **I see:** the palette opens over the sheet. The first Escape closes the palette and the sheet stays. The second Escape closes the sheet.
- **Expected outcome:** the overlays close in the order they opened. A second ⌘K while the palette is open does nothing.
- **Status:** Code, no test
  `features/app/src/overlay.rs:110-118` pushes the palette on the overlay stack. `features/app/src/overlay.rs:111` makes a second ⌘K a no-op. `overlay.rs` `escape` pops the top overlay. No test covers the stacked case.

### ⌘K is expected to clear the terminal

- **When:** the person expects ⌘K to clear the terminal's screen, as some terminal apps do.
- **I do:** run `ls` in the terminal, then press ⌘K.
- **I see:** the palette opens. The terminal's output stays on screen. Nothing is sent to the shell.
- **Expected outcome:** the palette opens and the terminal's text is unchanged. ⌘K does not clear the terminal.
- **Status:** Code, no test
  `terminal/view/src/keys.rs:65` returns `KeyIntent::Ignore` for ⌘ keys that the terminal does not use, so the app handles ⌘K. No clear is bound. `terminal/client/src/replica.rs:54-66` would bind `cmd-k` to `CommandPalette`, but the app never calls `register_host_shortcuts` (only `terminal/client/tests/component.rs:151` does). No test checks the terminal bytes after ⌘K.

### Linux: Ctrl+K

- **When:** the app runs on Linux, and the person presses Ctrl+K.
- **I do:** focus the terminal, press Ctrl+K.
- **I see:** the palette does not open. Ctrl+K reaches the shell, where it deletes to the end of the line in bash.
- **Expected outcome:** the palette opens, as the Linux hint says it should.
- **Status:** Likely bug
  `ui/src/keys.rs:94` binds `cmd-k`. In gpui, `cmd` is the platform modifier, which is Super on Linux (`parse_internal` in the gpui `platform/keystroke.rs` in the Bazel cache: `cmd`, `super` and `win` set `platform`). Only `secondary` maps to Ctrl off macOS. On Linux the terminal sends Ctrl+K to the shell (`terminal/view/src/keys.rs`, the non-Mac branch). Confirm on a Linux build: press Ctrl+K with the terminal focused, and see that the palette does not open. The live test `desktop/tests/live_flows/find.rs:64` presses `secondary-k`, which is Ctrl+K on Linux, so it would fail there too.

## Expected outcomes

- [ ] ⌘K (Ctrl+K on Linux) opens the palette with the terminal focused, and the terminal receives no bytes from the keys typed into it.
- [ ] Typing `move` puts Move session to another machine first, with the hint `⌥⌘M`.
- [ ] Enter on Move asks for a machine. The list holds only online machines of the channel, other than the one the session runs on.
- [ ] Enter on a machine moves the session to it, and the palette closes.
- [ ] Escape while asking goes back to the command list, with the query restored, and runs nothing.
- [ ] A second Escape closes the palette, and the session page stays in front.
- [ ] A pasted PR URL for a PR a session here reviews offers `Review this PR` first.
- [ ] A pasted PR URL that no session here reviews does not offer `Review this PR`.
- [ ] `long` followed by Enter or a click opens Settings with Long lines revealed, and closes the palette.
- [ ] A file row opens that file in the session that the palette was opened over.
- [ ] With no other machine online, the palette says why Move cannot go anywhere, and says what to do.
- [ ] A query that matches nothing shows the empty-state line and runs nothing on Enter.
- [ ] ⌘K does not clear the terminal's output.

## Status summary

| Step or branch | Status | Evidence |
| --- | --- | --- |
| 1. ⌘K opens the palette with the terminal focused | Fakes only | `desktop/tests/native/terminal.rs::an_unfocused_terminal_does_not_receive_palette_keystrokes` (sends the action, not the key); `ui/src/keys.rs:94` |
| 2. Typed keys go to the palette, not the shell | Fakes only | `desktop/tests/native/terminal.rs::an_unfocused_terminal_does_not_receive_palette_keystrokes` |
| 3. `mov` ranks Move first with its hint | Works (real daemon) | `desktop/tests/live_flows/find.rs::from_the_palette_a_session_moves_a_turn_is_found_a_pr_reviews_settings_open_and_a_pin_holds` (types `move`) |
| 4. Enter on Move asks for an online machine of the channel | Works (real daemon) | `desktop/tests/live_flows/find.rs::from_the_palette_...`; exact filter in `features/palette/tests/act.rs::running_move_asks_inline_for_another_online_machine_of_the_channel` |
| 5. A machine is picked and the move starts | Fakes only | `features/palette/tests/act.rs::picking_gpu_box_hands_the_move_to_the_app`; `features/app/tests/move_from_palette.rs::moving_a_channel_session_from_the_palette_asks_the_daemon` |
| 6. Escape steps back one level | Works (real daemon) | `desktop/tests/live_flows/find.rs::from_the_palette_...` |
| 7. Escape closes the palette | Works (real daemon) | `desktop/tests/live_flows/find.rs::close_palette` |
| 8. A pasted reviewed PR offers Review this PR | Fakes only | `features/palette/tests/review_pull_gate.rs::a_pull_request_a_session_reviews_is_offered_first`; `features/palette/tests/act.rs::a_pasted_pull_request_url_offers_review_this_pr_first` |
| 9. `long` opens Settings at Long lines | Works (real daemon) | `desktop/tests/live_flows/find.rs::from_the_palette_...` (clicks the row) |
| 10. A file result opens in the session's clone | Fakes only | `desktop/tests/native/overlays.rs::palette_file_results_open_the_current_sessions_clone` |
| Branch: no other machine online for Move | Likely bug | `ui/src/overlay_palette.rs:191` |
| Branch: gibberish matches nothing | Code, no test | `ui/src/overlay_palette.rs:182-191` |
| Branch: palette over another overlay | Code, no test | `features/app/src/overlay.rs:110-118` |
| Branch: ⌘K expected to clear the terminal | Code, no test | `terminal/view/src/keys.rs:65` |
| Branch: Linux Ctrl+K | Likely bug | `ui/src/keys.rs:94` |

Steps: 5 Works (real daemon), 5 Fakes only, 0 Code no test, 0 Not built, 0 Likely bug.
Branches: 1 Likely bug (no machine), 1 Likely bug (Linux), 3 Code no test (gibberish, stacked overlay, clear).

## Gaps and likely bugs

- Likely bug: `ui/src/keys.rs:94` binds `cmd-k` on every platform. On Linux that is Super+K, so the palette has no Ctrl+K key. The live test `desktop/tests/live_flows/find.rs:64` uses `secondary-k` and would not open the palette on Linux either. Confirm on a Linux build.
- Likely bug: `ui/src/overlay_palette.rs:191` shows `Nothing matches. Try fewer letters.` when Move has no other online machine. The message does not say why, and it does not say what to do.
- Likely bug: while Move asks for a machine, `ui/src/overlay_palette.rs` shows only the list, with no label that says the palette is asking for a machine. Confirm by opening Move in a session with two machines and looking at the palette.
- Likely bug: `features/palette/src/lib.rs:673-674` shows the hints `⌘4` for Go to Hosts and `⌘5` for Go to Files. `ui/src/keys.rs` binds only `⌘1` to `⌘3`, and `grep` finds no `cmd-4` or `cmd-5` binding. The SPEC does not mention ⌘4 or ⌘5. Confirm by pressing ⌘4 and ⌘5 in the app.
- Gap: no test presses a key to open the palette with the terminal focused. The native test sends `AppAction::OpenPalette`.
- Gap: no test types `mov`. The live test types `move`.
- Gap: no live test pastes a URL into the palette. The live test says ⌘V does not reach the palette input on the headless window.
- Gap: no live test moves a session from the palette. The move is tested with fakes only.
- Gap: no live test checks that a reviewed PR is offered `Review this PR`.
- Gap: no test covers an empty machine list while Move asks.

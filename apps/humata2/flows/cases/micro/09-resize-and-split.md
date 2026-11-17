# Resizing reaches the program, and a split opens a second group but never a third

Layer: micro. Platforms: macOS, Linux.

## Starting state

- Humata 2 is open on this Mac, with the session `ledger` selected.
- The session has an agent tab and a shell tab. The shell tab runs `htop`.
- The session shows one tab group. The shell tab is selected and has focus.
- The window is 1200 points wide and 760 points tall.
- I hold the mouse over the window's right edge. Nothing is typed.

## Steps

1. **I do:** I drag the window's right edge from 1200 to 900 points while `htop` runs.
   **I see:** `htop` redraws at the narrower width. Its columns and meters fit the new width.
   **State and events:** The replica sends one `Resize` frame to the host. The host sends `Resized` to every viewer. The PTY gets the new column count, so the program receives a size change.
   **Status:** Fakes only. `terminal/pane/tests/pane.rs::resizing_the_window_tells_the_program_the_new_size` proves the pane sizes its PTY, but it uses a recording PTY, not a real shell. The test checks growth only, so narrowing is not checked on a PTY. `htop` is not run.

2. **I do:** I keep dragging until a long line wraps onto a second row. Then I drag the edge wider again.
   **I see:** The long line wraps at the narrow width. When the window widens, the line joins back into one row. The cursor stays on its line.
   **State and events:** The emulator reflows soft-wrapped rows when the size changes. The program is told only the new size.
   **Status:** Fakes only. `terminal/vt/tests/grid.rs::resizing_reflows_wrapped_lines` runs the real terminal engine in-process. No shell or daemon is involved.

3. **I do:** I press ⌘\ (Ctrl+Shift+\ on Linux).
   **I see:** A second tab group opens on the right, with a new shell in it. Focus moves to the new group. The first group keeps `htop`.
   **State and events:** The pane emits a split request and does not send the key to `htop`. The app opens a shell with `split: true` (`ui/src/terminals.rs:190-196`). The session has two groups and one more shell. `focused_group` is 1.
   **Status:** Fakes only. `terminal/pane/tests/pane.rs::the_split_shortcut_asks_for_a_new_shell_and_reaches_no_program` checks the event and the key routing with a fake PTY. The Linux key check (`terminal/pane/src/pane.rs:609-617`) is the same code, but I have not confirmed a Linux run.

4. **I do:** I open the **+** menu on the first group's tab row and choose **Open in second group** on the "New terminal" row.
   **I see:** The same two-group layout as step 3 appears, with a new shell. The menu closes.
   **State and events:** The menu sends `OpenTerminal { agent, split: true }` (`ui/src/session_tabs.rs:319-321`). The rest is as in step 3.
   **Status:** Fakes only. `desktop/tests/native/session.rs::the_plus_menu_splits_once_and_joining_keeps_the_program_running` drives the menu with fake daemon clients. `desktop/tests/native/terminal.rs::clicking_a_split_terminal_focuses_that_group` checks focus moves to the group I click.

5. **I do:** I try a third split. I press ⌘\ again while the second group has focus, or I open the **+** menu in the second group and choose **Open in second group**.
   **I see:** The brief expects a refusal, and the code shows none. The layout stays at two groups. A new shell starts, and its tab lands in the second group beside the tab already there. No message appears.
   **State and events:** The session's shell count goes up by one. `show_tab` reuses group index 1 and adds the new tab there (`features/session/src/tabs.rs:100-125`). `groups.len()` stays 2.
   **Status:** Code, no test. `ui/src/session_tabs.rs:319-321` still offers the split action when two groups exist. It starts a new shell and adds its tab to group 2 (`features/session/src/tabs.rs:100-125`). SE-27 sets two groups as the limit and does not ask for a refusal, so this matches the spec. The desktop test checks the group count after the third split (`desktop/tests/native/session.rs:372-373`), not the shell count.

6. **I do:** I open the palette with ⌘K, type `Join tab groups`, and press Enter.
   **I see:** The two groups become one row. Every tab is in that row. `htop` keeps running in its tab, and no tab closes.
   **State and events:** The palette sends `CloseGroup(1)` (`features/app/src/commands.rs:82-84`). The tabs of group 2 move into group 1. The shell count does not change, and no program exits.
   **Status:** Fakes only. `desktop/tests/native/session.rs::the_plus_menu_splits_once_and_joining_keeps_the_program_running` checks the group count, the shell count and the terminal tab. It does not check that `htop` is still running. `features/session/tests/tabs.rs::split_is_limited_to_two_groups_and_attaches_an_existing_agent` checks that closing group 1 keeps all three tabs.

## Branches

### Two viewers of different sizes share one terminal

- **When:** A second viewer of the same terminal is open, such as the browser view. The two viewers have different sizes.
- **I do:** I resize the viewer that controls the terminal. Then I resize the other viewer.
- **I see:** Only the controlling viewer changes the program's size. The other viewer keeps the controller's size and shows the terminal inside its own frame. If I resize that viewer, the program does not change. If I choose **Take control** on it, its size is used from then on.
- **State and events:** The host allows one controller per terminal. A `Resize` from a viewer that is not the controller is refused with `read_only` (`terminal/host/src/lib.rs:930-937`, `1558-1567`). `TakeControl` moves control, both viewers get `Control` frames, and the new controller's size is applied and sent back as `Resized`. The replica sends a resize only while it holds control (`terminal/client/src/replica.rs:320-330`, `532-538`). The app uses `FitPolicy::Controlling` (`ui/src/terminals.rs:43`).
- **Expected outcome:** The program's size follows the controller only. A refused resize changes nothing. After **Take control**, the new size reaches every viewer.
- **Status:** Works (real shell). `terminal/host/src/lib.rs::two_viewers_share_output_and_explicitly_transfer_control` uses a real holder and `/bin/sh`. It checks that a read-only resize is refused, that `TakeControl` moves control, and that `Resized` (83 by 27) reaches both viewers. Not proven live: how the second viewer draws at a different size. That part is Fakes only, in `terminal/client/tests/replica.rs::authoritative_replica_orders_output_resize_resync_and_never_duplicates_replies`, which checks that a read-only mirror sends no `Resize` and that a narrow mirror keeps the full grid.

### A resize during a big output burst

- **When:** A program prints a lot of output while I drag the window.
- **I do:** I run `seq 1 200000` in the shell. While it prints, I drag the window narrower.
- **I see:** The output finishes. The last line is on screen at the new width. The view does not disconnect and does not keep a stale size.
- **State and events:** The resize is queued in order with the output, using `resize_epoch` (`terminal/host/src/lib.rs:1558-1624`). The host sends `Resized` in order. If a viewer falls behind, the host sends `ResyncRequired` and the viewer takes a new snapshot. The viewer keeps control.
- **Expected outcome:** The last line is visible at the new size. Control stays with the viewer. The screen matches what the program printed.
- **Status:** Code, no test. The burst path is covered alone by `terminal/host/src/feature_tests.rs::output_burst_uses_bounded_resync_without_dropping_controller`, which sends no resize. The ordering test `terminal/client/tests/replica.rs::authoritative_replica_orders_output_resize_resync_and_never_duplicates_replies` sends a few frames, not a burst. No test runs a resize and a burst together.

### A vertical resize over the real SSH test VM

- **When:** The shell tab is an SSH session to the dedicated test VM.
- **I do:** I drag the window height from 760 to 420 points, then to 960, then back to 760. After each step I run `stty size` in the shell.
- **I see:** `stty size` reports the new row count each time, with the same column count. The bottom line of the prompt moves with the bottom of the window. The shell's process ID and environment stay the same. A full-screen screen, if one is open, stays open.
- **State and events:** The host sends the new size to the remote holder over SSH. The PTY on the VM changes its row count. The shell is not restarted.
- **Expected outcome:** Rows follow the window height each time. Columns and the shell process do not change.
- **Status:** Works (real shell). `terminal/lab/tests/vertical_resize_ssh.rs::ssh_vertical_resize_changes_real_pty_rows_without_restarting_shell`. It is a manual target that needs `TERMINAL_LAB_SSH_CONFIG` and the test VM. I did not run it.

## Expected outcomes

- [ ] The program's size follows the window's height and width.
- [ ] A narrower window reflows wrapped lines to the new width.
- [ ] ⌘\ opens one second group, with one new shell.
- [ ] **Open in second group** from the **+** menu does the same.
- [ ] A split while two groups are open keeps two groups.
- [ ] Joining the groups keeps every tab, and every running program keeps running.
- [ ] A refused resize from a read-only viewer changes nothing.
- [ ] **Take control** moves the size to the new controller.
- [ ] A vertical resize over SSH changes the remote PTY rows and keeps the shell.

## Status summary

| Step or branch | Status | Evidence |
| --- | --- | --- |
| 1. Drag the window narrower while `htop` runs | Fakes only | `terminal/pane/tests/pane.rs::resizing_the_window_tells_the_program_the_new_size` (growth only, recording PTY) |
| 2. Long lines rewrap | Fakes only | `terminal/vt/tests/grid.rs::resizing_reflows_wrapped_lines` (real engine, no shell) |
| 3. ⌘\ opens a second group | Fakes only | `terminal/pane/tests/pane.rs::the_split_shortcut_asks_for_a_new_shell_and_reaches_no_program` |
| 4. **+** menu opens a second group | Fakes only | `desktop/tests/native/session.rs::the_plus_menu_splits_once_and_joining_keeps_the_program_running` |
| 5. Third split | Code, no test | `ui/src/session_tabs.rs:319-321`, `features/session/src/tabs.rs:100-125` |
| 6. Join tab groups | Fakes only | `desktop/tests/native/session.rs::the_plus_menu_splits_once_and_joining_keeps_the_program_running`, `features/session/tests/tabs.rs::split_is_limited_to_two_groups_and_attaches_an_existing_agent` |
| Branch: two viewers of different sizes | Works (real shell) | `terminal/host/src/lib.rs::two_viewers_share_output_and_explicitly_transfer_control` |
| Branch: resize during a big output burst | Code, no test | `terminal/host/src/lib.rs:1558-1624`; burst and replica tests are separate |
| Branch: vertical resize over SSH | Works (real shell) | `terminal/lab/tests/vertical_resize_ssh.rs::ssh_vertical_resize_changes_real_pty_rows_without_restarting_shell` (manual, not run) |

## Gaps and likely bugs

- Not built: nothing this case needs is missing. `SPEC.md` SE-27 sets two groups as the limit. It does not ask for a refusal message, so a silent third split is a choice to confirm, not a missing feature.
- Question for the owner: with two groups open, **Open in second group** adds a new shell tab to group 2 (`ui/src/session_tabs.rs:319-321`, `features/session/src/tabs.rs:100-125`). SE-27 allows it. Decide whether the item should read differently once two groups exist.
- Test gap, not a product bug: the narrowing case of the PTY size is not tested on a real PTY. `htop` is not used by any test in this case.

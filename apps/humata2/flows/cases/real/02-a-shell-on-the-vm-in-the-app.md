# A shell on the VM in the real app takes every key, copy, paste and resize

Real. Platforms: macOS (the app and the clipboard). The VM is Debian on arm64. Paths are relative to `apps/humata2`.

This case is for a run by hand or a later test. The terminal component has real-SSH proofs. **The app's own path has never run against real SSH.** That path is: desktop app, then `humatad2` on this Mac, then the provider's `ssh`, then `humatad2 --role machine` on the VM, then the holder and its shell. Every step below says which half is proven. Related cases: [flow 09](../../09-ssh-terminal-session.md), [micro 15 vim and less](../micro/15-vim-and-less.md), [micro 06 paste](../micro/06-paste-text.md), [micro 09 resize and split](../micro/09-resize-and-split.md), [real/01 the VM becomes a machine](01-the-vm-becomes-a-machine.md).

## Starting state

- The VM `h2-terminal-c1` is running and has been added as a machine, as in real/01. Its tools are in `~/.humata2/bin` on the VM and the machine shows online.
- A channel exists with a session on this Mac. The VM machine page is open in the real desktop app.
- `vim` (or `vi`) and `less` are installed on the VM. If not, see branch "vim is missing".
- This Mac's clipboard holds nothing the owner needs.

## Test setup

- Start and check the VM, read-only for a running one: `./terminal-ssh-demo.sh start`, then `./terminal-ssh-demo.sh status`. Never stop or restart the VM; other runs share it. Details: `terminal/platform_test/SSH_DEMO.md`.
- Isolation: run the app with its own temp home and its own `humatad2` state folder, never the owner's `~/.humata2`. Give the daemon the VM's SSH file with `humatad2 serve --ssh-config "$HOME/.lima/h2-terminal-c1/ssh.config"` (`daemon/humatad/src/serve.rs:131-134`), so nothing is written to the owner's `~/.ssh/config`. Whether the desktop app can be pointed at such a daemon, or starts its own with that flag, is unproven. Check `desktop/src/main.rs` first.
- Install the missing tools on the VM, if needed, with `ssh -F ~/.lima/h2-terminal-c1/ssh.config lima-h2-terminal-c1 sudo apt-get install -y vim-tiny less`. The disk is 4 GiB, so check `df -h /` first.
- **Needs a free screen.** Every step with the app window opens a window over the owner's screen. Headless equivalents: the `humata` CLI for starting and listing the shell, and `desktop/tests/live_flows/*` style tests (a fake or stand-in `ssh`). The terminal component's own SSH tests run headless (see the evidence lines).
- Needs the owner: a free screen, and `terminal-ssh-demo.sh start` having run. No model login is needed.

## Steps

1. **I do:** on the VM's machine page, press **Open a shell**. **Needs a free screen.**
   **I see:** a Terminal session with a shell tab and a prompt that comes from the VM.
   **State and events:** the session lives on the VM machine. A holder process for the shell exists on the VM, under the machine daemon.
   **Status:** Code, no test (the VM). The button on a machine page runs against a real local daemon, This Mac only: `desktop/tests/live_flows/terminals.rs::open_a_shell_on_a_machine_page_starts_a_terminal_there`. A remote session's terminal is relayed behind a fake `ssh`: `daemon/humatad/tests/remote.rs::a_session_started_on_the_machine_runs_there_and_the_host_relays_its_terminal`. No test starts a shell on a machine over real SSH. Note the button starts a Terminal session in a channel's session folder, not the machine's home (flow 09, step 4).

2. **I do:** type `uname -m; hostname; echo $SSH_CONNECTION` and press Return.
   **I see:** `aarch64`, the VM's host name, and an SSH connection line or an empty line. That proves the shell is not on this Mac.
   **State and events:** the bytes `uname -m` reach the VM holder in order, once each.
   **Status:** Works (real shell) for the component only: `terminal/host/src/roaming_tests.rs::ssh_outage_and_route_change_reattach_existing_holder_without_input_replay` (target `//apps/humata2/terminal/host:ssh_reconnect_test`, manual, real VM) sends lines and reads the shell's replies in order. The app path is unproven: Code, no test.

3. **I do:** run `vim notes.txt` on the VM. Press `i`, type `hello over ssh`, press Esc.
   **I see:** vim in the full window. After Esc the mode line no longer says INSERT.
   **State and events:** Esc goes to vim as the Escape key, not to an app binding.
   **Status:** Code, no test over SSH. A real Neovim on this Mac passes `terminal/lab/tests/neovim.rs::neovim_colors_mouse_keyboard_resize_and_resume` (Works (real shell), local only, and it checks text, not the mode line). The SSH variant in `terminal/lab/tests/interactive.rs` starts a session and asserts nothing (micro 15, branch "vim over the SSH VM").

4. **I do:** in vim press the arrow keys, then `:q!` and Return.
   **I see:** the cursor moves one cell per arrow. The shell prompt returns as it was.
   **State and events:** each arrow is one escape sequence (cursor-key mode follows what vim asks for).
   **Status:** Fakes only: `terminal/pane/tests/pane.rs::typed_keys_reach_the_program_in_its_encoding`. Leaving the full screen: `terminal/pane/tests/pane.rs::a_full_screen_program_leaves_the_shell_as_it_was`. Neither runs vim over SSH.

5. **I do:** run `sleep 100`, then press ⌃C.
   **I see:** `^C` and a fresh prompt at once. The tab stays open.
   **State and events:** ⌃C is the byte 0x03 to the shell, never a copy.
   **Status:** Fakes only: `terminal/view/src/keys.rs::ctrl_c_goes_to_the_program_on_every_platform`. Over SSH it is Code, no test. `terminal/vt/tests/input.rs` tests the engine only.

6. **I do:** at the prompt type `one two three`, then press ⌥← twice and ⌥→ once. Then press ⌥B.
   **I see:** ⌥← and ⌥→ move by a word if the VM's shell reads the sequences that way. ⌥B types a symbol (`∫`), not a word move.
   **State and events:** Option-as-Alt is Off (`terminal/vt/src/terminal.rs:223`), so ⌥ plus a letter types the macOS symbol.
   **Status:** Fakes only: `terminal/pane/tests/pane.rs::option_types_its_character_unless_it_is_set_to_act_as_alt`. The arrow words are Code, no test (`terminal/view/src/keys.rs:81-113`). Over SSH on bash with readline: Code, no test. See micro 03.

7. **I do:** run `less /var/log/dpkg.log`, press Space twice, type `/install` and Return, press `n`, then `q`.
   **I see:** pages scroll, the match is highlighted and `n` moves to the next. `q` returns to the prompt and the earlier output.
   **State and events:** less switches to the alternate screen and back.
   **Status:** Code, no test. No test runs `less`, locally or over SSH (micro 15 steps 9 to 11).

8. **I do:** run `printf 'alpha\nbeta\n'`. Drag across `alpha`, press ⌘C, then in TextEdit on this Mac press ⌘V.
   **I see:** `alpha` in TextEdit.
   **State and events:** the copy writes plain Unicode with hard newlines to this Mac's clipboard (`ClipboardWrite`, `ui/src/terminals.rs`).
   **Status:** Works (real shell) for the component only: `terminal/lab/tests/clipboard_ssh.rs::ssh_unicode_multiline_color_copy_pasteback_and_focus_are_exact` (targets `clipboard_ssh_test` and `clipboard_ssh_fast_test`, headless clipboard, real VM). The app path with the real macOS clipboard is Code, no test. The app uses `ui/src/terminals.rs` `ClipboardWrite`, not the lab's code.

9. **I do:** copy one line from TextEdit, click the terminal, press ⌘V at the prompt, then Return.
   **I see:** the line at the prompt, then run.
   **State and events:** the line is sent as a bracketed paste when the shell asks for it. Text with the end marker cannot end the paste early.
   **Status:** Works (real shell) for the component only: the same `clipboard_ssh.rs` test pastes the copied value back and checks the exact remote bytes. `terminal/pane/tests/pane.rs::a_one_line_paste_is_sent_and_bracketed_when_asked` is Fakes only. The app path is Code, no test.

10. **I do:** copy three lines from TextEdit and press ⌘V in the terminal.
    **I see:** a confirmation bar over the terminal. Nothing is typed yet. I press the paste-confirm button.
    **State and events:** only after the confirm do the three lines reach the shell. An Enter key cannot confirm it (`terminal/view/src/clipboard_tests.rs::buffered_enter_cannot_approve_a_paste_that_requires_confirmation`).
    **Status:** Fakes only: `terminal/pane/tests/pane.rs::a_multi_line_paste_waits_for_the_user`. The multi-line paste over real SSH is in the `clipboard_ssh.rs` test (Works (real shell), component only). The confirm bar in the real app on the VM tab is unproven.

11. **I do:** use the menu **Edit**, **Copy** and **Edit**, **Paste** with the terminal focused and a selection.
    **I see:** the same as ⌘C and ⌘V.
    **State and events:** the menu sends `input::Copy` (`desktop/src/main.rs:278-280`). The terminal handles `edit::Copy` (`terminal/view/src/view.rs:1247`).
    **Status:** Fakes only: `terminal/view/tests/view.rs::the_edit_menu_copies_pastes_and_selects_all`. The README's "Likely bug 5" (the menu does nothing) may be stale, because the view has handlers now. Whether `gpui_kit::component::input` and `gpui_kit::base::input` are the same actions is not checked here. **Likely bug (unconfirmed)**; confirm by using the menu in the real app.

12. **I do:** drag the window narrower, then taller, while `vim` or `less` is open.
    **I see:** the program redraws to the new size. After `stty size` in the shell, the rows and columns match the grid.
    **State and events:** a resize reaches the real PTY on the VM, with no new shell and the same process id.
    **Status:** Works (real shell) for the component only: `terminal/lab/tests/vertical_resize_ssh.rs::ssh_vertical_resize_changes_real_pty_rows_without_restarting_shell` (targets `vertical_resize_ssh_test` and `_fast_test`, manual). The app's own resize is Fakes only: `terminal/pane/tests/pane.rs::resizing_the_window_tells_the_program_the_new_size` (growth only). Over the app's SSH path it is Code, no test.

13. **I do:** press ⌘\ (or the **+** menu, **Split**) to open a second shell beside the first, then run `echo $$` in both.
    **I see:** two groups, each with its own shell. The two process ids differ.
    **State and events:** a second terminal tab on the same VM session. A second holder process on the VM.
    **Status:** Fakes only: `terminal/pane/tests/pane.rs::the_split_shortcut_asks_for_a_new_shell_and_reaches_no_program` and `desktop/tests/native/session.rs::the_plus_menu_splits_once_and_joining_keeps_the_program_running`. A split on a remote machine is Code, no test.

## Branches

### vim is missing on the VM
When: `vim` says command not found on the minimal Debian image. I install `vim-tiny` as in the test setup, or use `vi`. If the disk is full, `apt` fails and says so. The run stops and reports that.
**Status:** Code, no test. Nothing in the repo installs vim for a test.

### I cancel the multi-line paste
When: I press ⌘V with three lines, then dismiss the bar. Nothing reaches the shell, and the next keys type normally.
**Status:** Fakes only: `terminal/pane/tests/pane.rs::a_multi_line_paste_waits_for_the_user`.

### I type into the tab before its screen has arrived
When: right after step 1, before the prompt shows. The keys are refused, and a toast says why (`ui/src/terminals.rs`, `PaneEvent::InputRefused`; text from `refusal`, line 859). They are never queued.
**Status:** Fakes only: `clients/live/src/link.rs::input_while_reattaching_fails_and_is_never_queued`. The README's "Likely bug 4" (dropped with no message) looks out of date: the toast path exists now. Over real SSH, Code, no test.

### I ⌘-click a link or paste a Finder file
When: a URL is printed by `echo https://example.org`, and I ⌘-click it. The README's "Likely bug 1" says the app drops this. In the current code `ui/src/terminals.rs` handles `OpenLink` and `PasteFiles` (read it again before the run), and only web and mail links open. Pasting a file into a VM tab sends the file to the VM first (`paste_files` on the machine).
**Status:** Code, no test through a VM tab. Bells, title changes and folder changes are still dropped by the `_ => {}` arm (check the end of the event match).

### Two windows of different sizes show the same VM tab
When: the same tab is open in two app windows. Every writable viewer asks for a resize, so the two take turns (`ui/src/terminals.rs`, `FitPolicy::Controlling`; README "Likely bug 14").
**Status:** Likely bug. Confirm: open the tab in two windows of different sizes and run `stty size` in both.

### The shell exits
When: I run `exit` in the VM shell. The tab should say the shell ended. The README's "Likely bugs 2 and 3" say it shows "Lost the connection" and no exit line, because `clients/terminal_link/src/lib.rs:193` drops the Notice frame and the replica treats an exited shell as not authorized (`terminal/client/src/replica.rs:530-536`).
**Status:** Likely bug. Confirm: `exit` in a shell tab on the VM.

## Expected outcomes

- [ ] A shell opened from the VM's machine page runs on the VM, as shown by `uname -m` and `hostname`.
- [ ] vim takes Esc, arrows and ⌃C. The prompt returns after `:q!`, as it was.
- [ ] less opens, searches and quits, and the earlier output is back.
- [ ] Copy from the VM tab reaches this Mac's clipboard exactly. Paste from this Mac reaches the VM shell exactly.
- [ ] A multi-line paste waits for a confirm, and cancel sends nothing.
- [ ] A window resize changes the real PTY size on the VM without a new shell.
- [ ] A split makes a second shell on the VM with a different process id.
- [ ] No step touched the owner's `~/.humata2`, SSH keys or clipboard history beyond the run, and the VM was not stopped.

## Status summary

| Step or branch | Status | Evidence |
| --- | --- | --- |
| 1. Open a shell on the VM page | Code, no test | `desktop/tests/live_flows/terminals.rs::open_a_shell_on_a_machine_page_starts_a_terminal_there` (This Mac only); `daemon/humatad/tests/remote.rs::a_session_started_on_the_machine_runs_there_and_the_host_relays_its_terminal` (fake ssh) |
| 2. Commands run on the VM | Works (real shell), component only | `terminal/host/src/roaming_tests.rs::ssh_outage_and_route_change_reattach_existing_holder_without_input_replay` |
| 3. vim, insert, Esc | Code, no test | `terminal/lab/tests/neovim.rs::neovim_colors_mouse_keyboard_resize_and_resume` (local) |
| 4. Arrows and `:q!` | Fakes only | `terminal/pane/tests/pane.rs::typed_keys_reach_the_program_in_its_encoding` |
| 5. ⌃C | Fakes only | `terminal/view/src/keys.rs::ctrl_c_goes_to_the_program_on_every_platform` |
| 6. ⌥ keys | Fakes only | `terminal/pane/tests/pane.rs::option_types_its_character_unless_it_is_set_to_act_as_alt` |
| 7. less | Code, no test | none |
| 8. Copy to this Mac | Works (real shell), component only | `terminal/lab/tests/clipboard_ssh.rs::ssh_unicode_multiline_color_copy_pasteback_and_focus_are_exact` |
| 9. Paste one line | Works (real shell), component only | same test |
| 10. Multi-line paste confirm | Fakes only | `terminal/pane/tests/pane.rs::a_multi_line_paste_waits_for_the_user` |
| 11. Edit menu copy and paste | Fakes only | `terminal/view/tests/view.rs::the_edit_menu_copies_pastes_and_selects_all` |
| 12. Resize | Works (real shell), component only | `terminal/lab/tests/vertical_resize_ssh.rs::ssh_vertical_resize_changes_real_pty_rows_without_restarting_shell` |
| 13. Split | Fakes only | `desktop/tests/native/session.rs::the_plus_menu_splits_once_and_joining_keeps_the_program_running` |
| Branch: vim missing | Code, no test | none |
| Branch: cancel the paste | Fakes only | `terminal/pane/tests/pane.rs::a_multi_line_paste_waits_for_the_user` |
| Branch: typing before the screen | Fakes only | `clients/live/src/link.rs::input_while_reattaching_fails_and_is_never_queued` |
| Branch: link and file paste | Code, no test | `ui/src/terminals.rs` |
| Branch: two windows resize | Likely bug | `ui/src/terminals.rs` (`FitPolicy::Controlling`) |
| Branch: exit in the tab | Likely bug | `clients/terminal_link/src/lib.rs:193`, `terminal/client/src/replica.rs:530-536` |

## Gaps and likely bugs

Not built:
- The app's own path over real SSH has no test at all. It is the whole point of this case. No milestone owns an app-level real-VM test.
- A shell tied to no channel, run from the machine's home: the daemon can do it, the app never calls it (flow 09; no milestone).
- A Settings switch for Option-as-Alt (no milestone; README "Likely bug 7").
- A test that runs `less` or vim over SSH through any path.

Likely bugs:
- Notice frames are dropped at `clients/terminal_link/src/lib.rs:193` (it was line 191). The "Reconnected" line now has its own path (`clients/live/src/link.rs:188`), so the loss is the exit and disconnect notices.
- Edit menu Copy, Paste and Select All may do nothing: the menu uses `input::*` (`desktop/src/main.rs:278-280`) and the view handles `edit::*` (`terminal/view/src/view.rs:1247`). Unconfirmed.
- `exit` in a shell tab shows "Lost the connection" and no exit line (README 2 and 3).
- Two windows of different sizes take turns sizing the shell (README 14).
- Option-as-Alt is Off with no way to change it (`terminal/vt/src/terminal.rs:223`).
- Stale in the README, to be corrected by its owner: "Likely bug 1" (OpenLink and PasteFiles are handled now) and "Likely bug 4" (a toast refuses the keys).

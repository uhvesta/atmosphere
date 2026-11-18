# Two viewers on one shell share its output, each writer hears about the other, and a refresh keeps the shell running

Micro. macOS and Linux for the daemon. Web: the browser viewer is the lab experiment only; the shipped app has no browser viewer.

Which part is which: the shipped app is the desktop app and `humata attach`, both on the daemon's gRPC attach. The browser viewer, the take-control transfer and the loopback HTTP/WS gateway are the terminal lab (`terminal/lab`, `terminal/host`, `terminal/web`), an experiment that AGENTS.md describes. Each step below says which one it is.

## Starting state

- Humata's daemon and one holder run on this Mac. The holder owns a `/bin/sh` shell in tab 1 of session "Fix the ledger".
- Viewer A: the desktop app on this Mac, a wide window showing tab 1 at about 120 columns. Shipped.
- Viewer B: a second writable attach to tab 1 on this Mac, `humata attach "Fix the ledger" --tab 1`, in a terminal window about 80 columns wide. Shipped.
- Viewer C: a browser tab on another computer showing the same shell. Lab only; not shipped. This is the step that the shipped app cannot do yet.
- Viewer A has focus. Nothing is pasted and no find box is open.

## Steps

1. **I do:** Open the shell in viewer A (desktop app) and in viewer B (`humata attach`).
   - **I see:** Both show the same screen, with the same prompt and the same earlier output.
   - **State and events:** Two attaches share one holder. Each gets the same snapshot and then the same output.
   - **Status:** Works (real daemon). `daemon/humatad/tests/terminal.rs::a_read_only_attach_watches_but_cannot_type` has a writer type `hello`, and the watcher waits for the same `hello` in its output. This shows the output is shared. The test uses a read-only watcher, not a second writer.

2. **I do:** Type `echo from-a` in viewer A, and `echo from-b` in viewer B, at about the same time.
   - **I see:** Both lines show in both viewers, once each, in the order the daemon received them.
   - **State and events:** Two writable attaches send input to the holder. Input is numbered per attach. The daemon sends `InputWritten` to the sender.
   - **Status:** Code, no test. `daemon/humatad/tests/terminal.rs::writers_hear_about_each_other` counts the writers but does not type from both. The lab's `terminal/host/src/lib.rs::two_viewers_share_output_and_explicitly_transfer_control` allows one controller at a time, so it does not prove simultaneous writes.

3. **I do:** Look at the terminal bar of viewer A while B is attached.
   - **I see (code as written):** Nothing. The bar should read "1 other can type here" while the session is working (the LOOK1 decision, `OVERVIEW.md:378`, says peers show only while Working).
   - **State and events:** The daemon counts the writers and sends the count in each state (`daemon/humatad/src/attach.rs:504`, `other_writers`). The app link does not read that field (`clients/terminal_link/src/lib.rs`), and the live client never fills the map the bar reads (`clients/src/workspace.rs:35` `terminal_peers` has no producer in `clients/live`).
   - **Status:** Fakes only (app); Works (real daemon) (daemon). The daemon counts the writers (`daemon/humatad/tests/terminal.rs::writers_hear_about_each_other`); the app's link now reads `other_writers` from each state (`clients/live/src/link.rs::tests::a_state_message_says_how_many_others_can_type`) and the bar shows "1 other can type here" from it while the session works (`desktop/tests/native/queue.rs::queued_messages_can_be_shown_removed_and_sent_and_presence_follows_the_tab`, from a fixture). No test attaches two real viewers and reads the bar.

4. **I do:** Resize viewer B to about 80 columns, then type `stty size` in B.
   - **I see:** The shell reports the size of the last viewer that sent a size. After B's resize, `stty size` shows B's rows and columns. Viewer A shows the same screen at that size.
   - **State and events:** The daemon applies a resize from any writable attach (`daemon/humatad/src/attach.rs:336-345`, comment "D14: every attach that can type may resize"). Read-only attaches cannot resize. In the app, every replica that is a controller and fits its pane sends a resize (`terminal/client/src/replica.rs:326-330`), because the app's fit policy is Controlling for every tab (`ui/src/terminals.rs:43`). D271 point 3 says only the focused, visible replica should ask for a resize.
   - **Status:** Works (real daemon) for the daemon rule: `daemon/humatad/tests/terminal.rs::the_size_a_writer_attaches_with_reaches_the_shell` attaches at 100 by 30, sees `30 100`, resizes to 90 by 20, and sees `20 90`. The app's choice of fit policy goes against D271: Likely bug, see the gaps.

5. **I do:** Quit the desktop app (viewer A) and open it again on the same session, tab 1.
   - **I see:** The same shell, with its output and its prompt. Its program, if any, is still running. The earlier output is still on screen.
   - **State and events:** A's link ends and a new link attaches to the same holder from the checkpoint. The holder and the shell do not change.
   - **Status:** Works (real daemon). `clients/live/tests/term_restart.rs::a_shell_tab_catches_up_after_the_app_restarts_and_types_only_once_synced`. Micro-17 has the detail.

6. **I do (viewer C, lab):** Reload the browser tab.
   - **I see:** The same terminals come back with the same UUIDs and the same shell pid; output and typing continue.
   - **State and events:** The browser viewer reconnects to the lab host over the authenticated loopback WebSocket. The host keeps its screen and its shell.
   - **Status:** Works (real shell), lab only. `terminal/web/browser_smoke.mjs` reloads the page and checks the terminal's UUID and the output after the reload (around lines 307-322). The shell pid is carried through a later line that checks it is unchanged (lines 297 and 326). The target `browser_smoke_test` is tagged `manual` and `external`: it needs a running viewer URL, so it does not run in a default test run. Shipped: Not built (see B4).

## Branches

### B1. The browser viewer is read-only until it takes control (lab)

- **When:** Viewer C attaches to the lab host while viewer A has control.
- **I do:** Type into C.
- **I see:** C shows "Viewing" and its typing is refused with a `read_only` error. C has a Take control button.
- **State and events:** The host refuses input from a viewer that is not the controller. The lab viewer sends `TakeControl`, and the host moves control to it (`terminal/host/src/lib.rs`, `two_viewers_share_output_and_explicitly_transfer_control`; the viewer button is `terminal/client/src/lib.rs:665-676`).
- **Expected outcome:** Refused with a clear message; after Take control, C types and A is a viewer.
- **Status:** Works (real shell), lab only. `terminal/host/src/lib.rs::two_viewers_share_output_and_explicitly_transfer_control` checks the `read_only` error for the mirror's input, transfers control with `TakeControl`, and sees the typed line in both viewers. Shipped path: Not built (see B4).

### B2. One viewer loses its link

- **When:** The network drops for viewer C, or the terminal window running `humata attach` is closed.
- **I do:** Close B's terminal window.
- **I see (shipped):** Viewer A keeps running with the same shell and output. Its count of other writers goes back to 0 at the daemon, but the app does not show that count (see step 3).
- **State and events:** The daemon removes B's attach. The holder and the shell are not affected. A is told `other_writers == 0`.
- **Expected outcome:** A keeps typing and sees nothing else change; the shell keeps running.
- **Status:** Works (real daemon). `daemon/humatad/tests/terminal.rs::writers_hear_about_each_other` drops the second writer and checks the first writer's count goes back to 0. The app's display of that count is Code, no test.
- **Lab:** `terminal/host/src/lib.rs::stalled_viewer_loses_lease_without_interrupting_other_viewers` covers a stalled lab viewer losing its lease with a real shell. Works (real shell), lab only.

### B3. The host is upgraded while both viewers are attached

- **When:** A and B are attached and the daemon is upgraded to a new release.
- **I do:** Start the upgrade from the app.
- **I see (expected):** Both viewers pause, reconnect to the restarted daemon and show the current screen. The shell keeps its pid.
- **State and events:** The upgrade swaps the binary and restarts the daemon. The holders are not touched. The viewers' links end and open again.
- **Expected outcome:** Same holders, same shells, both viewers back on the same screen.
- **Status:** Code, no test for viewers. `daemon/humatad/tests/upgrade_local.rs::an_upgrade_swaps_the_staged_release_in_and_restarts_the_daemon_with_its_holders_untouched` checks the holders are the same processes and the session keeps running, but it attaches no viewer. The lab's live host upgrade is recorded in `terminal/lab/HANDOFF.md` as a manual check, not a test.

### B4. The browser viewer uses the shipped app's daemon (not built)

- **When:** I want viewer C to reach the shipped daemon from another computer.
- **I do:** Open the daemon's address in a browser on the other computer.
- **I see:** Nothing. The shipped daemon listens on its socket only.
- **State and events:** The web listener (D34: gRPC-web on loopback, Origin and Host checks, pairing with a revocable key) is not built. `OVERVIEW.md:114` leaves it unchecked.
- **Expected outcome:** A paired browser sees and types into the shipped daemon's terminals; unpaired and cross-origin callers are refused.
- **Status:** Not built (D34; `OVERVIEW.md:114`). The lab's gateway (`terminal/host/src/gateway.rs`) has the same idea, not the shipped one.

### B5. A hostile or broken client talks to the lab gateway

- **When:** A client sends malformed frames, a wrong origin or host, or an oversized field to the lab's loopback gateway.
- **I do:** Send the frames.
- **I see:** The gateway refuses them, and a valid viewer keeps working.
- **State and events:** Origin and host checks run before authentication. Oversized fields are refused before the actor waits. A connection budget bounds pending upgrades.
- **Expected outcome:** Refused, and the valid viewer is unaffected.
- **Status:** Fakes only. `terminal/host/src/gateway_adversarial.rs` runs the real gateway on a real socket, but with no shell (`Harness::start()` uses `start_with_shell(false)`). It proves the gate, not a live terminal. The one test that uses a real shell is `active_gateway_reader_survives_a_large_plain_output_burst`.

## Expected outcomes

- [ ] Two writable attaches show the same output.
- [ ] Each writer's terminal shows that another writer is present (not true in the app today; see the gaps).
- [ ] Both writers can type; each line is written once, in order (not proven by a test).
- [ ] A resize from any writable attach reaches the shell (`stty size` shows it).
- [ ] Two viewers of different sizes do not fight over the size (not true today; see the gaps).
- [ ] Refreshing or restarting a viewer keeps the holder and the program running.
- [ ] A viewer that leaves does not stop the shell, and the other writer's count goes back to 0.
- [ ] An upgrade of the host keeps the holders and the shells.
- [ ] A read-only viewer cannot type; in the lab, Take control hands over control.

## Status summary

| Step or branch | Status | Evidence |
| --- | --- | --- |
| 1. Both viewers show the same output | Works (real daemon) | `daemon/humatad/tests/terminal.rs::a_read_only_attach_watches_but_cannot_type` |
| 2. Both may type at once | Code, no test | `daemon/humatad/tests/terminal.rs::writers_hear_about_each_other` (counts only) |
| 3. Each writer hears another is present | Fakes only (app); Works (real daemon) (daemon) | `daemon/humatad/tests/terminal.rs::writers_hear_about_each_other`; `clients/live/src/link.rs::tests::a_state_message_says_how_many_others_can_type`; `desktop/tests/native/queue.rs::queued_messages_can_be_shown_removed_and_sent_and_presence_follows_the_tab` |
| 4. Size follows a rule | Works (real daemon) (daemon rule); Likely bug (app fit policy) | `daemon/humatad/tests/terminal.rs::the_size_a_writer_attaches_with_reaches_the_shell`; `ui/src/terminals.rs:43` |
| 5. Desktop refresh keeps the holder | Works (real daemon) | `clients/live/tests/term_restart.rs::a_shell_tab_catches_up_after_the_app_restarts_and_types_only_once_synced` |
| 6. Browser refresh keeps the shell (lab) | Works (real shell), lab only | `terminal/web/browser_smoke.mjs` (manual, external target) |
| B1. Browser read-only, then take control (lab) | Works (real shell), lab only | `terminal/host/src/lib.rs::two_viewers_share_output_and_explicitly_transfer_control` |
| B2. One viewer loses its link | Works (real daemon) | `daemon/humatad/tests/terminal.rs::writers_hear_about_each_other` (count back to 0) |
| B3. Host upgraded while attached | Code, no test (viewers) | `daemon/humatad/tests/upgrade_local.rs::an_upgrade_swaps_the_staged_release_in_and_restarts_the_daemon_with_its_holders_untouched` (no viewer attached) |
| B4. Browser reaches the shipped daemon | Not built | D34; `OVERVIEW.md:114` |
| B5. Hostile client to the lab gateway | Fakes only (no shell) | `terminal/host/src/gateway_adversarial.rs` |

Tag count, steps: Works (real daemon) 2 (steps 1 and 5), Works (real shell, lab only) 1 (step 6), Code, no test 1 (step 2), Likely bug 1 (step 3). Step 3's daemon count is Works (real daemon), but the app never shows it. Step 4 is split: the daemon rule is Works (real daemon), and the app's fit policy is a Likely bug.
Tag count, branches: Works (real shell, lab only) 1 (B1), Works (real daemon) 1 (B2, daemon side), Code, no test 1 (B3), Not built 1 (B4), Fakes only 1 (B5).

## Gaps and likely bugs

- Fixed in code: the app's link reads the writer count (`clients/live/src/link.rs::tests::a_state_message_says_how_many_others_can_type`). Coverage gap: no test attaches two real viewers and reads the bar.
- Likely bug, against D271 point 3: every writable replica asks for a resize when it fits. `ui/src/terminals.rs:43` sets `FitPolicy::Controlling` for all tabs, and `terminal/client/src/replica.rs:326-330` sends a resize whenever the replica is enabled and the policy fits. The daemon applies the last resize (`daemon/humatad/src/attach.rs:336-345`). Two windows of different sizes therefore take turns setting the shell's size. The `ui/src/terminals.rs` comment argues for Controlling; it does not match D271. To confirm: resize one window, then run `stty size` in the other.
- Not built: the browser viewer in the shipped app (D34, `OVERVIEW.md:114`). The lab's browser mirror is an experiment, not the shipped app, and its gateway is not the shipped web listener.
- Not built in the shipped path: Take control. `clients/terminal_link/src/lib.rs:103` ignores `TakeControl`. Only the lab host and viewer implement it.
- Code, no test: two writers typing at once through the shipped attach (`daemon/humatad/src/attach.rs`). The tests count writers and send each line from one writer.
- Lab tests: `terminal/lab/tests/*` (`app.rs`, `interactive.rs`, `neovim.rs`, `clipboard_ssh.rs`, `vertical_resize_ssh.rs`) drive one lab viewer against a real zsh, Neovim or SSH. No test name in that folder mentions two viewers on one shell.
- Code, no test: the viewer behaviour when the host is upgraded while viewers are attached (`daemon/humatad/tests/upgrade_local.rs` attaches none).
- Doc note: `OVERVIEW.md:362-372` describes the browser mirror as built and passing. That is the lab experiment (`terminal/lab`), and it runs with a manual, external browser test. The shipped app does not have it.

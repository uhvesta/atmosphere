# A large paste is refused with a reason, and a pasted image is saved privately for the agent

Layer: micro. Platforms: macOS, Linux, web. The large-text path differs on web, where the clipboard is read asynchronously. Image paste is only built in the daemon, so no platform has an end-to-end test.

## Starting state

- Humata 2 is open on this Mac. The session "Fix the ledger" has an agent tab (tab 0) and a shell tab (tab 1). The shell tab is focused.
- The local `humatad2` is running, and the session is live.
- The clipboard holds a 3 MB text log of about 60,000 lines.
- In a later step, the clipboard holds a PNG screenshot of about 2 MB.
- No paste is waiting for confirmation, and I hold no unsent text.

## Steps

1. **I do:** Press ⌘V with the 3 MB log on the clipboard, in the shell tab.
   **I see:** If zsh asked for bracketed paste, a toast reads "The complete encoded input is N bytes, above the 262144-byte limit. Send a smaller input; nothing was queued." If zsh did not ask, a prompt "Paste 60000 lines?" appears first, and after Paste the same toast appears. Nothing is typed.
   **State and events:** The engine turns the text into bytes. The replica refuses any single input over 256 KiB (`terminal/wire/src/lib.rs:10`, `terminal/client/src/replica.rs:277`) before it reaches the daemon. No input sequence number is used, and no bytes reach the shell. The 1 MiB pending-input cap is not reached, because on this Mac the clipboard read is synchronous.
   **Status:** Fakes only: `terminal/client/tests/component.rs::focused_pane_paste_reports_oversized_and_full_refusal_without_consuming_input_sequence` checks that an oversized paste is refused and consumes no sequence number, using a fake sink. The toast text is not checked, and no real daemon is involved.

2. **I do:** In a browser build, where the clipboard read is asynchronous, press ⌘V with the 3 MB log, and type `x` before the read returns.
   **I see:** A toast reads "The pending clipboard paste and following input were canceled because … exceed the 1024-event/1048576-byte buffer limit. Paste a smaller value or wait for clipboard access before adding more input. Retype only the unsent input." Nothing is sent, including the `x`.
   **State and events:** The pending read, the queued `x` and everything behind them are dropped together. This happens when the completed read cannot fit under 1 MiB (`terminal/view/src/pending_input.rs:8`, `:115-121`).
   **Status:** Fakes only: `terminal/view/src/clipboard_tests.rs::pending_clipboard_typeahead_is_bounded_and_dropped_atomically_on_overflow`. It overflows the queue with 1 MiB of typed text, not with a 3 MB clipboard result, and it runs with a fake host. Browser behaviour is not tested live.

3. **I do:** Copy the screenshot, select the agent tab, press ⌘V.
   **I see:** Expected: the path of the saved image is typed at the agent's prompt. Today: nothing appears.
   **State and events:** Expected: the app asks the daemon to save the image (`TerminalService.PasteImage`), then types the returned path (D30). Today: the pane emits `PaneEvent::PasteImage` (`terminal/pane/src/host.rs:379-380`), and the app's terminal event handler has no arm for it, so it falls through to `_ => {}` (`ui/src/terminals.rs:211`). The image is dropped silently.
   **Status:** Not built. D30 (I6) decides the flow, and `OVERVIEW.md` (terminal audit status) still lists image paste actions as open. The daemon side works, see the next steps. The silent drop of the event is a Likely bug at `ui/src/terminals.rs:211` (see Gaps and likely bugs). I re-checked the event match: it handles Focused, SplitRequested, ClipboardWrite, Failed and InputUncertain, and everything else falls into `_ => {}`. No app, CLI or MCP client calls `PasteImage`. Only the daemon calls it, to forward a paste to a remote machine (`daemon/humatad/src/remote_terminal.rs`), and its tests call it directly.

4. **I do:** A client sends a 2 MB PNG to `PasteImage` for the agent's session. (No app control does this today, so the test is at the daemon API.)
   **I see:** The response carries the path of a file under the session's paste folder, and the size in bytes.
   **State and events:** The daemon checks the PNG signature, saves the bytes into `files/<session>/paste/` through the store, and writes the file with mode 0600.
   **Status:** Works (real daemon): `daemon/humatad/tests/paste.rs::a_pasted_image_is_saved_privately_where_the_path_says`. It checks the path is absolute, inside the state folder, ends in `.png`, has the same bytes, and is mode 0600.

5. **I do:** The same client sends the same PNG again, with the same idempotency key, as a retry.
   **I see:** The same path comes back, and only one file is in the paste folder.
   **State and events:** The first result is remembered under the key and returned on the retry. The memory is in process only (see Gaps and likely bugs).
   **Status:** Works (real daemon): `daemon/humatad/tests/paste.rs::a_retry_with_the_same_key_saves_once`. It checks the path only, and it runs within one daemon run.

6. **I do:** The same client sends a 33 MB image.
   **I see:** A refusal that says the image is 33 MiB and that at most 32 MiB can be pasted. Nothing is saved.
   **State and events:** The check runs before the save. The refusal names the `data` field as too long.
   **Status:** Works (real daemon): `daemon/humatad/tests/paste.rs::an_image_over_the_limit_is_refused`. From the app, this step cannot happen yet (see step 3).

## Branches

### The link is slow

- **When:** The link to the machine is busy, so the replica's send queue is full.
- **I do:** Paste about 200 KB of text.
- **I see:** A toast says the terminal operation could not be queued.
- **Expected outcome:** Nothing is sent in part. No sequence number is used, and the paste can be tried again.
- **Status:** Fakes only: `terminal/client/tests/component.rs::focused_pane_paste_reports_oversized_and_full_refusal_without_consuming_input_sequence` (the second paste, refused by the fake sink) and `terminal/client/tests/component.rs::host_input_refusal_resolves_ticket_without_written_or_replay`. Nothing tests a real slow link.

### Paste into the composer instead

- **When:** I paste into the message box of an agent session rather than the terminal.
- **I do:** Press ⌘V in the composer with the 3 MB log.
- **I see:** Expected: the text goes into the composer. Today: no paste code exists for the composer.
- **Expected outcome:** Humata holds the message until it is sent, and sends it as one bracketed paste when the user sends it.
- **Status:** Not built. SE-23 (`SPEC.md:608-621`) covers held messages and sending them as one bracketed paste. `OVERVIEW.md` (the queue section) says the real paste transport is still open. A search of `features/session` finds no paste code or paste tests.

### The daemon is gone during the paste

- **When:** The daemon stops after I press ⌘V and before the paste is written.
- **I do:** Press ⌘V, then stop the daemon.
- **I see:** A toast that the operation could not be queued, or the disconnected state. Nothing is replayed when the daemon comes back.
- **Expected outcome:** The paste is never sent late. Typing stays off until the screen has synced.
- **Status:** Fakes only: `terminal/client/tests/component.rs::host_input_refusal_resolves_ticket_without_written_or_replay` checks a refused input is never replayed, using a fake sink. `clients/live/tests/term_restart.rs::a_shell_tab_catches_up_after_the_app_restarts_and_types_only_once_synced` shows typing is held until sync, but it does not paste.

## Expected outcomes

- [ ] A paste over 256 KiB into a terminal is refused whole, with a message that says nothing was queued.
- [ ] Refused pastes use no input sequence number and send no bytes.
- [ ] In a browser, a pending paste that overflows the 1 MiB buffer cancels the paste and the input behind it, with a message.
- [ ] A pasted screenshot in an agent tab is saved privately (mode 0600) and its path is typed into the tab.
- [ ] A retry with the same key does not save a second copy.
- [ ] An image over 32 MiB is refused and nothing is saved.
- [ ] A paste sent while the daemon is gone is never delivered later.

## Status summary

Paths are relative to `apps/humata2`.

| Step or branch | Status | Evidence |
|---|---|---|
| 1. 3 MB log into a shell (native) | Fakes only | `terminal/client/tests/component.rs::focused_pane_paste_reports_oversized_and_full_refusal_without_consuming_input_sequence` |
| 2. 3 MB log in a browser, typeahead | Fakes only | `terminal/view/src/clipboard_tests.rs::pending_clipboard_typeahead_is_bounded_and_dropped_atomically_on_overflow` |
| 3. Screenshot into the agent tab | Not built (plus Likely bug: silent drop at `ui/src/terminals.rs:211`) | D30 (I6); `OVERVIEW.md` terminal audit status |
| 4. Daemon saves the image privately | Works (real daemon) | `daemon/humatad/tests/paste.rs::a_pasted_image_is_saved_privately_where_the_path_says` |
| 5. Retry with the same key | Works (real daemon) | `daemon/humatad/tests/paste.rs::a_retry_with_the_same_key_saves_once` |
| 6. Image over 32 MiB refused | Works (real daemon) | `daemon/humatad/tests/paste.rs::an_image_over_the_limit_is_refused` |
| Branch: link is slow | Fakes only | `terminal/client/tests/component.rs::focused_pane_paste_reports_oversized_and_full_refusal_without_consuming_input_sequence` |
| Branch: composer paste | Not built | SE-23 (`SPEC.md:608-621`); `OVERVIEW.md` queue section |
| Branch: daemon gone during paste | Fakes only | `terminal/client/tests/component.rs::host_input_refusal_resolves_ticket_without_written_or_replay` |

## Gaps and likely bugs

- Not built: image paste from the app into an agent or shell tab (D30, I6). The daemon endpoint exists, but no app code calls it.
- Not built: composer paste (SE-23, D126). No code or tests in `features/session`.
- Not built (no milestone): splitting a large paste into pieces. A single input over 256 KiB is refused whole (`terminal/wire/src/lib.rs:10`, `terminal/client/src/replica.rs:277`), so a user with a large log has to split it by hand.
- Likely bug: image and file paste events are dropped with no message. `terminal/pane/src/host.rs:379-382` queues `PaneEvent::PasteImage` and `PaneEvent::PasteFiles`, and `ui/src/terminals.rs` has no arm for them (the match ends with `_ => {}` at line 211). Confirm: copy a screenshot, paste it into a shell tab, and see whether anything appears.
- Likely bug: the idempotency memory for `PasteImage` is in process only (`daemon/humatad/src/terminal.rs:694-710`). A retry that arrives after the daemon restarts saves a second copy. Confirm: send a keyed `PasteImage`, restart `humatad2`, send it again with the same key, and count the files in the paste folder.
- Coverage note: the pending-input 1 MiB cap (`terminal/view/src/pending_input.rs:8`) is only reached while a clipboard read is pending. On this Mac the read is synchronous, so the 256 KiB replica cap decides.

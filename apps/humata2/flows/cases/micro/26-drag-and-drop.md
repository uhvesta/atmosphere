# Drag and drop: a Finder drop uploads into the Files page, and other drops are not built

Layer: micro. Platforms: macOS (Finder drags) first; Linux and web share the same pane code.

## Starting state

- Humata 2 is open on this Mac, on the Files page (route Files). The left pane shows this-mac's home. The right pane shows the shared folder of the payments-rewrite channel on this Mac.
- Machines: this-mac (online) and devbox-1 (online). devbox-1 is not shown yet.
- A session, "Fix login", is open on this Mac with one shell tab, and its Files panel is showing in the right panel.
- Finder is in front, with a local file (small.txt) and a 2 GB folder ready to drag.
- I hold the mouse. Nothing has been dropped yet.

Scope note: the Files page is one app-wide page with two panes. It is not per session. A drop goes into the pane's folder, which can be a session folder if I navigate there (FM-10). The session's own Files panel is a separate view with no drop target.

## Steps

1. **I do:** drag small.txt from Finder onto the right pane's list area ("Drop here, or drag from Finder") and release.
   **I see:** the file appears in the right pane's list when the upload is done.
   **State and events:** the pane sends the dropped paths as `FilesAction::Dropped(Right, paths)`. The client starts one upload transfer per file, into the pane's folder, and the file's checksum is checked on the machine before the transfer counts as done (FM-6).
   **Status:** Works (real daemon). `desktop/tests/live_flows/files.rs::files_dropped_from_finder_upload_into_the_pane` runs a real daemon. It sends the drop action directly, not an OS drag, so the drop wiring in `ui/src/file_manager.rs:412-418` is not exercised by the test. The file's bytes are checked on disk.

2. **I do:** after dropping, watch the transfers tray until the upload ends.
   **I see:** a tray row for the upload, then Done. Progress is shown in the tray (FM-7).
   **State and events:** the tray holds one row per transfer. The row goes from running to Done. The test waits for every row to reach Done and for at least one row to exist.
   **Status:** Works (real daemon). The row and the Done state are asserted in `desktop/tests/live_flows/files.rs::files_dropped_from_finder_upload_into_the_pane` (through `wait_transfers_done`, lines 143-156). The progress figure (bytes or percent) is not asserted by any live test. The row's display is covered only by `desktop/tests/files_page.rs::copying_shows_a_tray_row_and_the_name_arrives_when_it_is_done` (Fakes only).

3. **I do:** drag a file from Finder onto a shell tab in the session, expecting its path to be typed in, quoted.
   **I see (expected):** the quoted path appears at the shell prompt.
   **I see (code):** nothing happens. The terminal does not accept a drop.
   **State and events:** none. No drop target is registered on the terminal.
   **Status:** Not built (no milestone). `grep -rn "ExternalPaths\|on_drop"` over `terminal/` and `ui/src/` finds the Files pane (`ui/src/file_manager.rs:412-418`) and the board, sidebar and tab reorder drags, but nothing on a terminal. SPEC §26 and SE-10 do not define a drop onto a terminal.

4. **I do:** drag a file onto the agent's terminal or onto a composer, expecting it to be attached.
   **I see (expected):** the file is attached to the next message.
   **I see (code):** nothing happens. There is no composer, and the agent's terminal takes no drop.
   **State and events:** none.
   **Status:** Not built (no milestone). SPEC SE-10 says there is no chat layer, so there is no composer to attach to.

5. **I do:** drag a file onto the session's Files panel (the right panel, Files tab).
   **I see (expected):** the file is uploaded into the session's folder, the same as the Files page.
   **I see (code):** nothing happens. The Files panel's list has no drop handler (`ui/src/panel_files.rs` has none), and its Upload… button is disabled (`ui/src/panel_files.rs:313-318`, `.disabled(true)`).
   **State and events:** none.
   **Status:** Not built (no milestone; OVERVIEW.md "Known gaps" for Files lists Upload and Open in editor). The client operation exists (`clients/src/file_manager.rs:425`, live at `clients/live/src/file_manager.rs:838`), so only the panel's wiring is missing.

6. **I do:** set the right pane to devbox-1 and drop a folder from Finder onto it.
   **I see (expected):** the folder appears on devbox-1 with identical bytes.
   **State and events:** the upload goes through the host to devbox-1 over the existing link (FM-5, FM-9). Nothing is staged on the source.
   **Status:** Works (real daemon). The daemon side is `daemon/humatad/tests/remote_transfers.rs::an_upload_to_a_machine_arrives_whole` (fake boxes). The picker that sets the pane to devbox-1 is `desktop/tests/files_page.rs::the_right_pane_picks_another_machine_through_the_popup` (Fakes only). No live test drops onto a remote pane.

7. **I do:** drag a 2 GB folder from Finder onto the pane.
   **I see (expected):** the folder's files are uploaded, each with a tray row, and the progress moves.
   **State and events:** the client walks the folder, creates each sub-folder on the machine, and starts one transfer per file (`clients/live/src/file_manager.rs:849-886`). Each file is hashed first (`clients/live/src/file_manager.rs:1134-1148` reads the whole file once), then streamed through a channel of four chunks (`clients/live/src/file_manager.rs:935`), so memory stays bounded.
   **Status:** Code, no test. No test drops a folder, and the largest live drop is 3 MiB (`desktop/tests/live_flows/files.rs:296-300`). The chunked upload is covered in the daemon by `daemon/humatad/tests/transfers.rs::an_upload_in_chunks_arrives_whole_and_the_transfer_is_done`, but not at 2 GB.

8. **I do:** drag a file from a pane out to Finder.
   **I see (expected):** Finder receives a copy of the file.
   **I see (code):** nothing. The pane has no drag source.
   **State and events:** none.
   **Status:** Not built (FM-4 says "from a pane out to Finder"). `ui/src/file_manager.rs` has only `drag_over` and `on_drop` (lines 412-418), no `on_drag`.

## Branches

### Copied files pasted into a terminal with ⌘V

- **When:** I copy a file in Finder (⌘C), click a shell tab, and press ⌘V. A drop has no part in this path, but it is the only other way a file reaches a terminal.
- **I do:** press ⌘V in the shell tab.
- **I see (expected, per the pane's own comment at `terminal/pane/src/host.rs:78-79`):** the app decides whether to type the quoted paths or upload them first.
- **I see (code):** the pane raises `PaneEvent::PasteFiles` (`terminal/view/src/paste.rs:45`, `terminal/pane/src/host.rs:382`). The app's event handler in `ui/src/terminals.rs` (around lines 178-212) handles only Focused, SplitRequested, ClipboardWrite, Failed and InputUncertain. `PasteFiles` and `PasteImage` fall through to `_ => {}` (`ui/src/terminals.rs:212`). Nothing is typed and nothing is uploaded, and no message is shown.
- **Expected outcome:** the quoted paths are typed, or the files are uploaded and their paths typed, as the comment says.
- **Status:** Likely bug at `ui/src/terminals.rs:212` (`_ => {}`). Confirm by copying a file in Finder, pasting it into a shell tab, and checking that the prompt is unchanged and no toast appears. No test asserts what the app does with `PasteFiles`: the pane test mapping in `terminal/pane/tests/support/mod.rs:98` is the only reference. The lab harness ignores it too (`terminal/lab/src/lib.rs:400`), and the standalone lab client refuses with a message (`terminal/client/src/lib.rs:455`), which is not the shipped app.

### A name clash on upload

- **When:** a file with the same name already sits in the destination folder.
- **I do:** drop the file.
- **I see (expected):** an error that says the name is taken and that nothing was put there. The existing file is untouched.
- **I see (code):** the live client sends `on_existing: Fail` (`clients/live/src/file_manager.rs`, in `upload_file`). The daemon refuses with `P::Exists` and the message "already exists in the destination, so nothing was put there" (`daemon/humatad/src/transfer_service.rs:803-806`). The fake client refuses before anything starts (`clients/src/file_manager.rs:1050-1053`, `NameTaken`). The user has no choice to replace or keep both on a drop; the API has Replace and KeepBoth, but the UI never sends them.
- **Expected outcome:** the existing file is unchanged, and the error names the file.
- **Status:** Code, no test for the upload path. The copy path has `daemon/humatad/tests/transfers.rs::a_taken_name_fails_without_writing_when_the_choice_is_fail`, which uses the same `on_existing` choice, but no upload test covers a taken name.

### A drop onto a stale or unloaded pane

- **When:** the pane is stale (its listing is out of date) or has no folder yet.
- **I do:** drag a file onto the pane.
- **I see:** the pane is dimmed and does not show the drop hint. The drop target is not attached, so Finder shows no drop.
- **Expected outcome:** nothing is uploaded, and the pane stays as it was.
- **Status:** Code, no test. `ui/src/file_manager.rs:410-412` attaches the drop target only when the pane is not stale. `features/files/src/manager/edit.rs:72-74` ignores a drop with no usable folder (`features/files/src/manager/pane.rs:121-127`).

### A drop on a read-only or stopped session

- **When:** the session I am looking at is stopped, archived or read-only.
- **I do:** drop a file on the Files page.
- **I see:** the drop works if the pane's folder is usable. The Files page is not tied to a session's state, so a stopped session does not block it. The session's Files panel has no drop target at all (step 5).
- **Expected outcome:** the drop into a usable folder works. A drop into an archived session's folder is refused by the daemon, not silently accepted.
- **Status:** Code, no test. No test covers a drop while a session is stopped or archived.

### An upload cut by a daemon restart

- **When:** the daemon (humatad) restarts while an upload is part way through.
- **I do:** wait for the daemon to come back.
- **I see (expected):** the upload pauses, then continues from the last confirmed byte, and ends Done.
- **I see (code):** the daemon keeps the transfer and resumes it from its offset (`daemon/humatad/src/transfer_service.rs`).
- **Expected outcome:** the file lands whole, with its checksum matching.
- **Status:** Works (real daemon). `daemon/humatad/tests/transfers.rs::an_interrupted_upload_survives_a_restart_and_can_still_be_resumed`, and the cut-short case `daemon/humatad/tests/transfers.rs::an_upload_cut_short_is_paused_and_resumes_from_where_it_got`.

### An upload cut by an app restart

- **When:** I quit Humata 2 in the middle of a drop.
- **I do:** reopen the app.
- **I see (expected):** the transfer continues or is shown as paused, with a way to resume.
- **I see (code):** the app's map from transfer id to file name is in memory only (`clients/live/src/file_manager.rs`, `upload_names`). The transfer is still on the daemon, but nothing in the app says which drop it was.
- **Expected outcome:** the transfer and its name are shown after the app comes back.
- **Status:** Code, no test. The daemon side is tested (the branch above); the app's side is not.

### The Upload… button

- **When:** I want to upload a file without dragging it.
- **I do:** click Upload… in the session's Files panel.
- **I see:** the button is greyed out and does nothing.
- **Expected outcome (spec):** a file picker opens and the chosen file is uploaded like a drop.
- **Status:** Not built (no milestone; OVERVIEW.md Files "Known gaps"). `ui/src/panel_files.rs:313-318` sets `.disabled(true)`. The Files page has no Upload button at all, only the drop target.

## Expected outcomes

- [ ] A Finder drop onto the Files page uploads the file into that pane's folder, and the file lands whole.
- [ ] The tray shows the upload as a row that ends Done.
- [ ] A drop onto a shell tab does not type the path (not built).
- [ ] ⌘V of copied files into a shell tab types their quoted paths or uploads them (likely bug: nothing happens).
- [ ] A drop onto the agent's terminal or a composer attaches nothing (not built).
- [ ] A drop onto the session's Files panel does nothing (not built).
- [ ] A folder dropped onto a pane on another machine arrives there whole.
- [ ] A 2 GB folder uploads with bounded memory, and its progress moves (not tested).
- [ ] A file dragged out of a pane to Finder is copied to Finder (not built).
- [ ] A name clash on upload fails without changing the existing file (upload path not tested).
- [ ] A drop onto a stale pane changes nothing.
- [ ] An upload cut by a daemon restart resumes and lands whole.
- [ ] An upload cut by an app restart is still shown after the app returns (not built).
- [ ] The Upload… button opens a picker (not built).

## Status summary

| Step or branch | Status | Evidence |
| --- | --- | --- |
| 1. Finder drop lands in the pane | Works (real daemon) | `desktop/tests/live_flows/files.rs::files_dropped_from_finder_upload_into_the_pane` (drop sent as an action) |
| 2. Tray row goes to Done | Works (real daemon) | same test, `wait_transfers_done`; progress figure not asserted |
| 3. Drop on a shell tab types the path | Not built | no milestone; no drop handler in `terminal/` or `ui/src/` |
| 4. Drop on terminal or composer attaches | Not built | no milestone; SPEC SE-10 (no chat layer) |
| 5. Drop on the session's Files panel | Not built | no milestone; `ui/src/panel_files.rs` (no drop handler, Upload… disabled at 313-318) |
| 6. Drop folder onto another machine | Works (real daemon) | `daemon/humatad/tests/remote_transfers.rs::an_upload_to_a_machine_arrives_whole` |
| 7. 2 GB folder drop | Code, no test | `clients/live/src/file_manager.rs:849-886, 935, 1134-1148`; largest live drop 3 MiB |
| 8. Drag a file out to Finder | Not built | FM-4; `ui/src/file_manager.rs` has no `on_drag` |
| Branch: ⌘V of copied files into a terminal | Likely bug | `ui/src/terminals.rs:212` (`_ => {}`); `terminal/view/src/paste.rs:45` |
| Branch: name clash on upload | Code, no test | `daemon/humatad/src/transfer_service.rs:803-806`; copy path `daemon/humatad/tests/transfers.rs::a_taken_name_fails_without_writing_when_the_choice_is_fail` |
| Branch: drop onto stale pane | Code, no test | `ui/src/file_manager.rs:410-412`; `features/files/src/manager/edit.rs:72-74` |
| Branch: drop on read-only or stopped session | Code, no test | `features/files/src/manager/pane.rs:121-127` |
| Branch: upload cut by daemon restart | Works (real daemon) | `daemon/humatad/tests/transfers.rs::an_interrupted_upload_survives_a_restart_and_can_still_be_resumed` |
| Branch: upload cut by app restart | Code, no test | `clients/live/src/file_manager.rs` (`upload_names`, in memory) |
| Branch: Upload… button | Not built | `ui/src/panel_files.rs:313-318`; OVERVIEW.md Files gaps |

## Gaps and likely bugs

- Likely bug: `clients/live/src/file_manager.rs:880` (`let id = self.upload_file(...).await?;`, inside the folder walk in `upload_tree`). When one file in a multi-file or folder drop fails (for example a name clash), the loop returns that one error. The files before it have already started their transfers and keep going, so the drop is not all or nothing, and the error does not say so. Confirm by dropping three files where the second name already exists, then checking the tray and the destination.
- Likely bug: `ui/src/terminals.rs:212` (`_ => {}`) drops `PaneEvent::PasteFiles` and `PaneEvent::PasteImage` silently, so ⌘V of copied files into a shell tab does nothing. Confirm as in the branch above. The terminal's drop path is separate and not built.
- Not built: drag from a pane out to Finder (FM-4).
- Not built: drop onto a terminal (types the quoted path) and onto an agent composer (attaches). No milestone.
- Not built: the session Files panel's drop and its Upload… button (`ui/src/panel_files.rs:313-318`).
- Not built: the app's side of an upload cut by an app restart (the name map is in memory only).
- Code, no test: a 2 GB folder. The whole file is hashed before the first byte moves (`clients/live/src/file_manager.rs:1134-1148`), so the progress bar stays idle for the whole read. This is a likely slowdown for big files, not a bug, but no test measures it.
- Doc drift: OVERVIEW.md line 457 lists "Files Open in editor/Upload" as a known gap, while the Finder drop upload is built. The gap is now the panel's button and its drop target only.
- Fakes only: the tray row and the picker for another machine, which the native tests drive against the fake client.

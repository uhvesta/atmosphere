# Attach and typing: the CLI's attach, tabs, type and read, and the session page's terminal tabs

Layer: explored · macOS · harness: `//apps/humata2/cli:sessions_test`, `//apps/humata2/cli:search_test`, `//apps/humata2/daemon/humatad:terminal_test`, `//apps/humata2/desktop:native_test`, `//apps/humata2/desktop:live_flows_test`

Phase 1 (map from code and snapshots, no Bazel) is done. Steps marked **PENDING RUN** need phase 2 (the coach's "phase 2 go"). Steps marked **mapped** are filled from the code, the existing tests and the snapshots named in Evidence, and have not been run this turn.

## Setup

- **Phase 2, CLI part (throwaway daemon, own root and HOME under the scratchpad):**
  ```
  cd <your checkout>
  export PATH=$PWD/.tools/bin:$PATH
  bazel build --jobs=4 //apps/humata2/cli:humata //apps/humata2/daemon/humatad:humatad2 //apps/humata2/daemon/holder:humata2_holder
  SCR=<scratchpad>/xb
  mkdir -p $SCR/home $SCR/root $SCR/evidence
  HOME=$SCR/home bazel-bin/apps/humata2/daemon/humatad/humatad2 serve \
    --socket $SCR/d.sock --state $SCR/state --root $SCR/root \
    --holder bazel-bin/apps/humata2/daemon/holder/humata2_holder \
    --shell /bin/sh --shell-arg=-i
  # in another shell:
  export HUMATA_SOCKET=$SCR/d.sock; export HOME=$SCR/home
  H=bazel-bin/apps/humata2/cli/humata   # absolute path in practice
  ```
  The flags follow `daemon/humatad/tests/common/mod.rs` (`serve`, `--socket`, `--state`, `--root`, `--holder`, `--shell`, `--shell-arg=-i`). No `--admin`. Stop the daemon when done. `--state` may need to be a folder; check the first serve's error if it refuses.
- **Seeding (phase 2):** two sessions. A shell-only session (for the no-agent-tab cases) started with `$H session start --title "xb shell" --agent <shell kind>`; the shell kind's CLI spelling is not confirmed (the app's Terminal kind runs `zsh`, `clients/src/model.rs:329`), so record which spelling the CLI accepts. A second session with an agent tab, started without `--agent` if the daemon has a default agent here; if it has none, record that and use the shell-only session for the agent-tab steps as well, marking them as such.
- **Desktop part:** headless `App` harnesses only (`native_test` fixtures, `live_flows_test` against a real `humatad2` and holder). No windows on screen.
- **Not used:** `cli/tests/waiting.rs` covers presence, passphrases and the empty-trash plan, not attach. `cli/tests/turn.rs` covers `humata turn`, not typing.

## Steps

### 1. `humata session tabs <session>`
- **Screen:** a JSON list of the session's tabs (`tabs[]` with `id`, `kind`, `session`), no human layout.
- **Offered:** `--json` is the only useful form; without it the same list is printed by the human renderer. No flags.
- **Action:** `humata --json session tabs <session>`.
- **Result:** one `terminal.list_tabs` call; prints the daemon's tabs with their ids for `attach --tab`. Exit 0.
- **Evidence:** `cli/tests/sessions.rs:153` (fake daemon, asserts ids in order).
- **Replay check:** `//apps/humata2/cli:sessions_test --test_filter=session_tabs_prints_the_tabs` (expect pass).
- **PENDING RUN:** `$H --json session tabs <agent-session-id>` against the throwaway daemon; expect exit 0 and an agent tab. Save to `$SCR/evidence/X-B/01-tabs.json`.

### 2. `humata attach <session>` with an agent tab (no `--tab`)
- **Screen:** the terminal stream on this terminal: the agent's snapshot, then live output. A stderr note `Attached. Ctrl-] detaches.` (`cli/src/commands/streams.rs:93`).
- **Offered:** keystrokes go to the tab; Ctrl-] (0x1d) detaches. Nothing else is offered.
- **Action:** the CLI lists tabs (`terminal.list_tabs`), picks the agent tab (`default_tab`, `cli/src/commands/sessions.rs:43`), then opens `terminal.attach`.
- **Result:** on Ctrl-] the CLI prints `Detached.` and exits 0 (`streams.rs:~243`).
- **Evidence:** `cli/tests/sessions.rs:114` (fake daemon). No live evidence yet.
- **Replay check:** `//apps/humata2/cli:sessions_test --test_filter=attach_without_a_tab_attaches_when_the_session_has_an_agent_tab` (expect pass).
- **PENDING RUN:** feed a pty so the CLI sees a keyboard: `printf 'echo WIRE$((40+2))\r\035' | script -q $SCR/evidence/X-B/02-attach.typescript $H attach <agent-session-id>; echo "exit $?"`. Expect `Attached. Ctrl-] detaches.`, `Detached.`, exit 0, and `WIRE42` in the transcript. If `script` cannot feed a non-tty console, record that as the reason and mark the step not reachable.

### 3. `humata attach <session>` with no agent tab and two or more shells
- **Screen:** an error on stderr, nothing attached.
- **Offered:** nothing in the terminal; the fix is in the message.
- **Action:** `humata attach <shell-only-session>` (two shells, no agent tab).
- **Result:** `CliError::NoAgentTab` lists the tab ids and asks for `--tab`; exit 7 (`exit::PRECONDITION`, `cli/src/exit.rs:14`). The attach RPC is never sent.
- **Evidence:** `cli/tests/sessions.rs:134`.
- **Replay check:** `//apps/humata2/cli:sessions_test --test_filter=attach_without_a_tab_stops_when_the_tab_is_ambiguous` (expect pass).
- **PENDING RUN:** `$H attach <shell-only-session>; echo "exit $?"`. Expect exit 7 and the message text quoted in full. Save stderr to `$SCR/evidence/X-B/03-ambiguous.txt`.

### 4. `humata attach <session> --tab <shell-tab-id>`
- **Screen:** the same stream as step 2, on the named shell.
- **Offered:** the same keys as step 2.
- **Action:** `--tab` skips `list_tabs` (`sessions.rs:22`) and sends the id as given.
- **Result:** attaches to that tab. An id from another session is refused by the daemon (`tab(&id, Some(&session))` in `terminal.rs`).
- **Evidence:** code only (`cli/src/commands/sessions.rs:22`, `daemon/humatad/src/terminal.rs` `fn tab`). No CLI test sends `--tab` to attach.
- **Replay check:** none in the CLI; `//apps/humata2/daemon/humatad:terminal_test --test_filter=typed_input_comes_back_as_contiguous_output` covers the attach path itself.
- **PENDING RUN:** `printf '\035' | script -q $SCR/evidence/X-B/04-tab.typescript $H attach <shell-only-session> --tab <tb_id>; echo "exit $?"`. Then the same with a tab id from another session; expect a not-found refusal and its exit code (record it).

### 5. `humata attach --read-only <session>` (watch)
- **Screen:** the same stream and the same `Attached. Ctrl-] detaches.` note. There is no read-only label on screen.
- **Offered:** only Ctrl-] to detach. Typed keys are not sent.
- **Action:** `humata attach --read-only <session>`, then type a line.
- **Result:** the CLI calls `terminal.watch` (`cli/src/commands/mod.rs:237`). The CLI drops typed bytes locally (`streams.rs:163`, `if !typed.is_empty() && !read_only`) and never sends them, so the daemon's refusal (`attach.rs:352`, "this attach is read-only, so it can't type") never reaches the person. The attach is not counted as a writer (`attach.rs` `writer` is `None` when read-only).
- **Evidence:** daemon `a_read_only_attach_watches_but_cannot_type` (`daemon/humatad/tests/terminal.rs:423`) proves the protocol refusal; no CLI test covers the silent drop.
- **Replay check:** `//apps/humata2/daemon/humatad:terminal_test --test_filter=a_read_only_attach_watches_but_cannot_type` (expect pass).
- **PENDING RUN:** in one pty, `printf 'echo xqro$((1+1))\r\035' | script -q $SCR/evidence/X-B/05-readonly.typescript $H attach --read-only <agent-session-id>; echo "exit $?"`. Then, in a second attach as writer, check that `xqro2` was not typed (the output must not contain it). Expect exit 0 and no refusal text on the watcher's screen.

### 6. `humata attach <session>` while the session's machine is stopped
- **Screen:** nothing attached; an error naming the stopped sessions and the fix.
- **Offered:** the fix named in the message (`Start sessions on <machine> first, with the Start button or humata start`).
- **Action:** `humata session stop <session>` for the only session on this machine, then `humata attach <session>`.
- **Result:** `require_running` refuses non-agent callers with `FailedPrecondition` (`daemon/humatad/src/stop/gate.rs:44`); the CLI exits with the precondition code (check `exit.rs`; PRECONDITION = 7 if mapped that way).
- **Evidence:** code (`stop/gate.rs:54`). No CLI test.
- **Replay check:** `//apps/humata2/daemon/humatad:stop_sessions_test` (the stop gate tests; confirm the target name in its BUILD) — expect the refusal test to pass.
- **PENDING RUN:** `$H session stop <s>; $H attach <s>; echo "exit $?"` (use the real id). Record the full message and exit code.

### 7. `humata attach <session>` after `humata session resume` (the screen comes back)
- **Screen:** a snapshot of the restored screen, then live output. The resume's catch-up notice (`caught up N lines of output`) is **not** shown from the CLI.
- **Offered:** same as step 2.
- **Action:** `humata session resume <s>`, then attach.
- **Result:** the CLI never sets `from_offset` (`sessions.rs:32` and `streams.rs:~56` build `AttachOpen` with only session, tab and read_only), so the daemon always sends a full snapshot from 0. The `CaughtUp` notice is only sent when `from != 0` (`daemon/humatad/src/attach.rs:113`).
- **Evidence:** code (`attach.rs:113`, `terminal.proto:115` "0 or older than earliest_offset gets a Snapshot"); daemon test `resuming_from_an_offset_says_how_many_lines_it_caught_up` (`terminal_test.rs:87`) uses a raw AttachOpen, not the CLI.
- **Replay check:** `//apps/humata2/daemon/humatad:terminal_test --test_filter=resuming_from_an_offset_says_how_many_lines_it_caught_up` (expect pass).
- **PENDING RUN:** `$H session stop <s> && $H session resume <s> && printf '\035' | script -q $SCR/evidence/X-B/07-resume.typescript $H attach <s>; echo "exit $?"`. Expect no `caught up` line in the transcript. Record whatever the resumed screen shows first.

### 8. `humata session type <session> "text"` (typed, no newline)
- **Screen:** none (CLI output only).
- **Offered:** `--tab`, `--at-turn-end` (see step 9). No `--help` text for `type` itself (see Surprises).
- **Action:** `humata --json session type <s> "echo xqtyped"`.
- **Result:** `terminal.type` with `When::Now` (`sessions.rs:416`). The daemon types the text into the agent tab, or with no agent tab into the oldest tab (`typing_tab`, `terminal.rs:682`). Response `{typed: true, queued: null, turn_end_uncertain: false}`. Exit 0.
- **Evidence:** `cli/tests/sessions.rs:432` (request shape only); daemon `each_text_typed_into_a_session_reaches_its_shell` (`terminal_test.rs:578`).
- **Replay check:** `//apps/humata2/daemon/humatad:terminal_test --test_filter=each_text_typed_into_a_session_reaches_its_shell` (expect pass).
- **PENDING RUN:** `$H --json session type <shell-only-session> "echo xqtyped"; echo "exit $?"`, then `$H attach ... ` snapshot (step 2 style) and check `xqtyped` appears. Save result JSON.

### 9. `humata session type --at-turn-end <session> "text"`
- **Screen:** none; an error on stderr.
- **Offered:** the flag is accepted by the CLI.
- **Action:** `humata session type <s> "later" --at-turn-end`.
- **Result (from code):** the CLI sends `When::TurnEnds`; the daemon refuses it with `invalid_field("when", NotAllowedHere, ...)` (`terminal.rs:~1347`) and the fix text tells the agent to use `inbox_ask`. Nothing is typed. The CLI test `type_at_turn_end_waits_for_the_turn` (`cli/tests/sessions.rs:432`) only checks the request it sends; it never sees the daemon's refusal.
- **Evidence:** `daemon/humatad/src/terminal.rs` (`type_in`); `cli/tests/sessions.rs:432` (fake daemon).
- **Replay check:** `//apps/humata2/cli:sessions_test --test_filter=type_at_turn_end_waits_for_the_turn` (passes, and proves only the request shape). Expected real result (PENDING): nonzero exit with the NotAllowedHere message.
- **PENDING RUN:** `$H session type <s> "later" --at-turn-end; echo "exit $?"`. Record the exit code and the full stderr. Expect no text typed (check with step 8's attach snapshot).

### 10. `humata session type <s> "text" --tab <id>`
- **Screen:** none.
- **Offered:** `--tab`.
- **Action:** type into a named tab.
- **Result:** `self.tab(&tab, Some(&session.id))` (`terminal.rs:~1370`): the named tab gets the text; an id from another session is refused.
- **Evidence:** code only.
- **Replay check:** `//apps/humata2/daemon/humatad:terminal_test --test_filter=each_text_typed_into_a_session_reaches_its_shell` (covers the default path; no `--tab` test found).
- **PENDING RUN:** type into the shell tab of a two-shell session with `--tab`, then check the other tab shows no `xqtab`.

### 11. `humata session type <s> "text"` with no agent tab and two shells (no `--tab`)
- **Screen:** none; exit 0.
- **Offered:** nothing.
- **Action:** type with no `--tab` into a two-shell session.
- **Result (from code):** the daemon picks the oldest shell (`typing_tab` sorts by `(kind != Agent, created)`), with no error. `attach` refuses this same case (step 3). Surprise, see Surprises.
- **Evidence:** `daemon/humatad/src/terminal.rs:682`.
- **Replay check:** none exists; expected behaviour recorded here.
- **PENDING RUN:** `$H --json session type <two-shell-session> "echo xqpick"; echo "exit $?"`, then check which tab shows `xqpick` (attach each in turn, step 2 style). Record which one.

### 12. `humata session type` into a shell that has exited
- **Screen:** none on success; the refusal on stderr.
- **Offered:** nothing.
- **Action:** `humata session type <s> "exit 3"` then `humata session type <s> "echo xqafter"`.
- **Result (from code):** the daemon refuses typing into an exited tab (`typing_into_a_shell_that_exited_is_refused`, `terminal_test.rs:592`); the message text is the daemon's (not the app's "This tab's shell has exited ..." toast).
- **Evidence:** `daemon/humatad/tests/terminal.rs:592`; `ui/src/terminals.rs:916-919` (the app's copy).
- **Replay check:** `//apps/humata2/daemon/humatad:terminal_test --test_filter=typing_into_a_shell_that_exited_is_refused` (expect pass).
- **PENDING RUN:** `$H session type <shell-only-session> "exit 3"; $H session type <shell-only-session> "echo xqafter"; echo "exit $?"`. Record both exit codes and the second's stderr.

### 13. `humata session read <session> [--from N] [--to M]`
- **Screen:** JSON `entries` of conversation turns; not terminal output.
- **Offered:** `--from`, `--to`, `--json`.
- **Action:** `humata --json session read <s> --from 1 --to 2`.
- **Result:** `history.read_turns` with paging (`cli/src/commands/search.rs:77`). Exit 0; a refused read exits with the kind's code (`NOT_FOUND` = 3, `search_test`).
- **Evidence:** `cli/tests/search.rs:228-338`.
- **Replay check:** `//apps/humata2/cli:search_test --test_filter=read_gathers_every_page_in_order` (expect pass).
- **PENDING RUN:** `$H --json session read <agent-session-id> --from 1 --to 2; echo "exit $?"`. Note that the agent session has no turns yet if nothing was asked; record whatever comes back.

### 14. `humata shell <machine>`
- **Screen:** the terminal stream of a new machine shell tab (no session).
- **Offered:** keys; Ctrl-] detaches. With `--json`, prints the new tab instead.
- **Action:** `humata shell <machine>` (this machine's id).
- **Result:** `terminal.open_machine_shell` then attach (`cli/src/commands/sessions.rs:107`). With `--json` no attach, only the tab JSON.
- **Evidence:** code only; no `cli/tests` file targets `shell` (the grep found none).
- **Replay check:** none; expected behaviour recorded here.
- **PENDING RUN:** `$H --json shell <this-machine-id>`, expect a tab JSON with a `session` unset and an `id`; then `printf 'echo xqmach\r\035' | script -q $SCR/evidence/X-B/14-shell.typescript $H shell <this-machine-id>`, expect `xqmach` in the transcript. Machine id from `$H --json hosts`.

### 15. Session page: the terminal tab row, agent selected
- **Screen:** tab row: `claude` (selected), `zsh`, `client.rs` (a file tab with `Close tab`), then `New terminal` (the + button). Below the tab row, `1 other can type here` at the right while the session is Working and another viewer types. The terminal pane below with the output log.
- **Offered:** click a terminal tab; `Close tab` on a file tab only; `New terminal` (+); `Expand terminal`; `Toggle side panel`; `Fork`; `Session actions`; in the status line the Resume buttons when exited.
- **Action:** none (state).
- **Result:** the row shows the selected tab's output.
- **Evidence:** `desktop/tests/__snapshots__/native/01-session-working-light.view.txt` lines 125–140 and `.png`; `02-session-working-dark.view.txt` for dark.
- **Replay check:** `//apps/humata2/desktop:native_test --test_filter=terminal_tabs_show_only_the_selected_terminals_live_output` (expect pass).

### 16. The + menu ("New terminal")
- **Screen:** a card `new-terminal-menu` below the + button: one row per enabled agent, then the plain shell row (named `New terminal`), each with an icon button `Open in second group`; a `Cancel` button at the bottom.
- **Offered:** the agent or shell row (opens a tab in this group); `Open in second group` (opens it in a new group); `Cancel` (closes, starts nothing).
- **Action:** click `+`, then `Cancel`.
- **Result:** the menu closes; no tab is started (`live_flows/tabs_and_palette.rs` step 1).
- **Evidence:** `ui/src/session_tabs.rs:408-460` (code). No snapshot of the open menu under `native/` (grep found the close-only menu snapshot `session-menu-*` for the session header menu, not this one).
- **Replay check:** `//apps/humata2/desktop:live_flows_test --test_filter=the_tab_row_opens_and_closes_files_and_the_panel_and_full_screen_follow_their_keys` (step 1 of that test is the + and Cancel; expect pass).

### 17. New terminal from the + menu takes typing with no click
- **Screen:** a new shell tab next to the others, selected; the shell prompt appears.
- **Offered:** typing goes to the new tab at once.
- **Action:** open a shell with the menu's shell row (sent as `OpenTerminal { agent: Shell, split: false }` in the test), then type `pwd`.
- **Result:** the prompt in the new tab shows the typed command's output (`~/humata`).
- **Evidence:** `desktop/tests/native/terminal.rs:137` (fake daemon).
- **Replay check:** `//apps/humata2/desktop:native_test --test_filter=new_sessions_and_selected_tabs_receive_input_without_an_extra_click` (expect pass). Real shell: `//apps/humata2/desktop:live_flows_test --test_filter=a_new_terminal_tab_takes_typing_without_a_click_and_runs_the_command` (**PENDING RUN**, real holder and `humatad2`, `exclusive`, can take minutes).

### 18. Split into a second group, and Join tab groups
- **Screen:** two groups; the second has its own tab row (`codex`, `zsh`), with `terminal-secondary` marked. The palette lists `Join tab groups` only while two groups are open.
- **Offered:** `Open in second group` (from step 16), `Join tab groups` (palette), `close-group-0` and `close-group-1` are **not** offered.
- **Action:** `add-tab-0` → `split-terminal-0-codex` (fake), then palette `Join tab groups`.
- **Result:** two groups; after joining, one group, and the shells are unchanged (count the same).
- **Evidence:** `desktop/tests/__snapshots__/native/14-two-tab-groups.view.txt` (lines 45–54) and `.png`.
- **Replay check:** `//apps/humata2/desktop:native_test --test_filter=the_plus_menu_splits_once_and_joining_keeps_the_program_running` (expect pass).

### 19. Selecting a tab shows that tab's output only
- **Screen:** the selected tab's output in the pane; the other tab's output hidden.
- **Offered:** click a terminal tab.
- **Action:** `terminal-tab 1`, print to tab 1; then `terminal-tab 0`.
- **Result:** output shows only for the selected tab.
- **Evidence:** `desktop/tests/native/session.rs:289` (code and test).
- **Replay check:** `//apps/humata2/desktop:native_test --test_filter=terminal_tabs_show_only_the_selected_terminals_live_output` (expect pass).

### 20. Shell exits (`exit`) in a shell tab
- **Screen:** the tab's output stays; a line `terminal-exited` under it: `The shell exited with code N. Its output stays here; open a new terminal to run more commands.` The machine's disconnected banner does not appear.
- **Offered:** the shell tab's `terminal` area only. No close button on the tab (see Surprises). `+` (New terminal) to start another tab.
- **Action:** click the terminal, type `exit` Enter.
- **Result:** the tab is exited; typed input after it is not sent.
- **Evidence:** `ui/src/terminals.rs:792` (copy); `desktop/tests/native/terminal.rs:243` (test).
- **Replay check:** `//apps/humata2/desktop:native_test --test_filter=a_shell_tab_whose_shell_exits_says_so` (expect pass).

### 21. Typing into the exited shell (app)
- **Screen:** a toast (one per try): `This tab's shell has exited, so what you typed was not sent. Open a new terminal to run more commands.` For tab 0 (the agent) the copy says `Resume the session to type into it again.`
- **Offered:** the toast only. No retry.
- **Action:** type `ls` Enter after step 20.
- **Result:** one toast, nothing sent.
- **Evidence:** `ui/src/terminals.rs:915-919`; `desktop/tests/native/terminal.rs:392` (toast count).
- **Replay check:** `//apps/humata2/desktop:native_test --test_filter=typing_into_an_exited_shell_says_why_once` (expect pass).

### 22. A tab whose machine comes back
- **Screen:** a line `terminal-reconnected` under the output saying how many lines it caught up; cleared on the next keystroke.
- **Offered:** typing (clears the line).
- **Action:** the fake daemon sends `Reconnected { caught_up_lines: 12 }`, then type `a`.
- **Result:** the line shows, then goes after typing.
- **Evidence:** `desktop/tests/native/terminal.rs:259`.
- **Replay check:** `//apps/humata2/desktop:native_test --test_filter=a_reconnected_tab_says_so_until_it_is_typed_into` (expect pass).

### 23. Two viewers on one shell
- **Screen:** both viewers show the same tab and the same answers. While the session reads Working, each viewer's bar says `1 other can type here`. The label is hidden when the session is Idle and when the pane is split.
- **Offered:** typing in either viewer.
- **Action:** viewer B opens the newest shell; both type `xq...` commands; `humata turn working`; then viewer B closes.
- **Result:** both see both answers; the bar names the other viewer only while Working; after B closes, A's bar stops naming B.
- **Evidence:** `desktop/tests/live_flows/two_viewers.rs:58`; `ui/src/session_tabs.rs:213-231` (the bar's condition); `desktop/tests/__snapshots__/native/01-session-working-light.view.txt:135` (`1 other can type here`) and `.png`.
- **Replay check:** `//apps/humata2/desktop:live_flows_test --test_filter=two_viewers_type_into_one_shell_and_each_hears_the_other` (**PENDING RUN**, real daemon and holder).

### 24. Keys in a real shell (zsh) on a session's tab
- **Screen:** the tab's zsh prompt; answers computed by the shell (`| tr a-z A-Z`).
- **Offered:** keys: arrows, Ctrl-A/E/W/U/K/H/L, Alt-arrows, Backspace, history; Ctrl-C, Ctrl-Z; `vim` and `less` take their keys.
- **Action:** the editing, history, control and vim/less flows in `live_flows/typing.rs`.
- **Result:** the line the shell runs matches the edits.
- **Evidence:** `desktop/tests/live_flows/typing.rs:119-285`.
- **Replay check:** `//apps/humata2/desktop:live_flows_test --test_filter=editing_keys_fix_the_line_the_real_shell_runs` (**PENDING RUN**). Also `--test_filter=vim_and_less_take_their_keys_from_the_terminal` (**PENDING RUN**).

### 25. Full screen: terminal and back
- **Screen:** the terminal fills the window; the title or button reads `Return to columns`.
- **Offered:** `Expand terminal` (button); `⇧⌘↩` (key); `Return to columns`; the same key again.
- **Action:** `⇧⌘↩`, then the button, then `Return to columns`.
- **Result:** the full-screen state toggles and the columns come back.
- **Evidence:** `desktop/tests/native/shell.rs:149` (terminal full screen returns to the same columns); `snapshots native/10-terminal-full-screen.view.txt`.
- **Replay check:** `//apps/humata2/desktop:native_test --test_filter=terminal_and_panel_full_screen_return_to_the_same_columns` (expect pass); `//apps/humata2/desktop:live_flows_test --test_filter=the_tab_row_opens_and_closes_files_and_the_panel_and_full_screen_follow_their_keys` (**PENDING RUN**, covers the keys).

### 26. File tab in the tab row (Open big, Close tab)
- **Screen:** a file tab (`client.rs`) with a `Close tab` control next to the terminal tabs; selecting it shows the file.
- **Offered:** click the file tab; `Close tab` (closes the file tab only, the terminals stay).
- **Action:** `Open big` on a file, then `Close tab`.
- **Result:** the file tab closes; the terminal is selected again; nothing stopped.
- **Evidence:** `desktop/tests/__snapshots__/native/file-tab-open.view.txt` and `.png`; `desktop/tests/native/session.rs:315`.
- **Replay check:** `//apps/humata2/desktop:native_test --test_filter=files_open_in_the_tab_row_and_close_back_to_the_terminal` (expect pass).

## Not reachable headless

- **A real pty attach from a person's keyboard:** the CLI's live attach needs a console with a keyboard. The phase 2 `script` runs feed a pty by pipe; that is an approximation, and the transcript is the evidence. A real Ctrl-] from a person's terminal is not reachable headless.
- **Real window focus and the on-screen tab row:** the headless `App` harness draws the tab row and reads the view; no window is shown (the no-windows rule). Pixel-level focus rings are not checked.
- **Read-only label in the desktop app:** the `ReplicaPhase::ReadOnly` branch (`ui/src/terminals.rs:921`) is reachable only when the replica is not the controller; no harness in `desktop/tests` sets that state for the session page (the only `Take control` button is in `terminal/client/src/lib.rs:674`, not the session page). Not reached.
- **A session on another machine (relay):** the `elsewhere` branch in attach and type needs the remote rig (`live_flows/remote.rs`, not in this area).
- **Typing while a session moves:** the daemon drops it silently (`attach.rs`, `if self.moving() { return Ok(()) }`), and the app says `This session is moving to ...`. Covered by `live_flows/remote.rs` or move tests (`daemon/humatad/tests/move_typing.rs`), not by this record.
- **Agent typing from MCP or hooks:** `humata turn` and the agent's `inbox_ask` path are in `cli/tests/turn.rs` and `live_flows/two_viewers.rs`; this record does not walk them.

## Surprises

1. **`session type --at-turn-end` is accepted by the CLI and refused by the daemon.** The CLI sends `When::TurnEnds`; the daemon's `type_in` refuses it with `NotAllowedHere` and the message `only the app holds text until a turn ends`. The CLI test named `type_at_turn_end_waits_for_the_turn` passes without the daemon ever seeing the flag run, so the test name promises a wait that never happens. The `--at-turn-end` flag has no help text. (`cli/src/args.rs` `Type`, `daemon/humatad/src/terminal.rs` `type_in`, `cli/tests/sessions.rs:432`)
2. **`session type` has no help text at all.** The `Type` variant in `args.rs` has no doc comment, so `humata session type --help` shows only the flags.
3. **Two rules for "which tab" disagree.** `attach` without `--tab` refuses when there are two shells and no agent tab (exit 7). `session type` without `--tab` in the same session silently types into the oldest shell (`typing_tab`, sort by agent first, then created time). A typed command can reach a tab the person did not pick.
4. **A read-only attach drops typing with no message.** The CLI discards typed bytes when `--read-only` is set (`streams.rs:163`), so a watcher who types sees nothing happen. The daemon's refusal ("this attach is read-only, so it can't type; attach again without read_only") never reaches the person. The stderr note is the same `Attached. Ctrl-] detaches.` with no read-only mention.
5. **Resume from the CLI never shows the catch-up line.** `attach` never sends `from_offset`, so it always gets a full snapshot from 0 and no `caught up N lines` notice. The daemon sends that notice only for resumes from an offset (`daemon/humatad/src/attach.rs:113`).
6. **Shell tabs have no close control in the tab row, as far as the code shows.** The tab row shows `Close tab` only on non-terminal tabs (`ui/src/session_tabs.rs`, the `when(!matches!(tab, Tab::Terminal(_)))` branch). The daemon has a `CloseTab` RPC (`terminal.rs` `close_tab`), but a grep of `clients/`, `ui/`, `cli/` and `features/` found no caller for it. Whether an exited shell tab leaves the row is not checked here; phase 2 can check it on the tab row after step 20.
7. **Two "New terminal" controls.** The + button and the shell row inside its menu have the same label, `New terminal`. The harness says so (`live_flows/tabs_and_palette.rs:~150`): the menu row cannot be clicked by label. A person sees two items with the same name.
8. **Two commands read a session's history.** `humata session read` (turns, `READ_TURNS`) and `humata conversation` (`READ_CONVERSATION`, top level) return conversation entries. Nothing in the help text says which to use or what each returns.
9. **The "other can type" label is hidden while Idle.** `terminal-peers` shows only when the session is Working and the pane is not split (`ui/src/session_tabs.rs:213-231`). Two people can type into one shell while the session is Idle and the bar shows nothing.
10. **Typing into an exited shell gives two different messages.** The app shows its own toast (`ui/src/terminals.rs:916-919`); the CLI and daemon send the daemon's refusal text. Phase 2 records the CLI wording.
11. **The read-only copy in the app is unreachable from the session page.** `ReplicaPhase::ReadOnly` produces `Another window controls this tab, so it is read-only here ... Type in that window instead.` (`ui/src/terminals.rs:921`). No harness reaches that phase, and the only `Take control` button is in `terminal/client/src/lib.rs`, which is not the session page. Phase 2 does not reach it either unless the coach asks for it.
12. **Typing while moving is silent in the daemon and explained in the app.** `attach.rs` drops input while the session moves; the app's copy says `This session is moving to <machine>, so what you typed was not sent`. The CLI attach gets no message.

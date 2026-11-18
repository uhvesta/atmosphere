# Fork: the Fork dialog, its three kinds, Fork from, agent and machine, and what a fork makes

Layer: explored · macOS · harness: desktop/tests/native/fork_move.rs (frames 62 and 63, headless), desktop/tests/native/session_menu.rs, desktop/tests/live_flows/fork_kinds.rs and lifecycle.rs (real humatad2), features/session/tests/fork_move.rs (reducer), cli/tests/sessions.rs (fake daemon), daemon/humatad/tests/fork.rs (daemon)

Phase 1 (map from code and snapshots). Steps marked PENDING RUN are filled in phase 2. The resume, stop and agent-difference screens are in X-A-resume-and-agents.md.

## Setup

- Native frames: `App::new(fixture::design())` in desktop/tests/native/fork_move.rs. The fixture session "ledger" (Ledger schema migration) runs on devbox-1 with agent Claude Code. The Fork dialog then suggests branch `humata/ledger-schema-migration-2`.
- Live: desktop/tests/live_flows/fork_kinds.rs and lifecycle.rs start a real `humatad2` through `World::start()`. The fake agent's conversation is seeded as Claude JSONL with two prompts ("write the ledger path", "add retries to the ledger"). The parent is "Write path". Phase 2 runs these, not a hand-started daemon.
- Reducer: features/session/tests/fork_move.rs drives `Sessions` with `TestStore` and `Clients::fake`.
- CLI: cli/tests/sessions.rs drives `humata session fork` against `FakeDaemon`.
- Throwaway daemon for CLI transcripts: see the last section. The owner's `~/.humata2`, `~/.claude`, `~/.codex` and `~/.copilot` are never used.

## Steps

### 1. The session page's Fork button (the entry point)
- **Screen:** Session header, right side: `Fork` (icon GitFork, id `session-fork`), then `Expand terminal`, `Toggle side panel`, `Session actions`. It is absent on a Desk session (ui/src/pages.rs `.when(!desk)`). The Changes panel is open on the sample session "ledger" in native/session-menu-closed.view.txt.
- **Offered:** Fork (header), Session actions (⋯ menu), ⇧⌘F and the palette commands (step 2).
- **Action:** Press `Fork`. It sends `SessionAction::StartFork(ForkKind::Worktree)`, so the dialog opens on Code + context.
- **Result:** A `Dialog "Fork session"` opens over the page. Title `Fork “<parent title>”`. `fork-kind-0-on` is selected, Fork from is "Now", Agent and Machine show the parent's, the branch is suggested, and a Preview is drawn. The draft title is `<parent> 2` (features/session/src/lib.rs StartFork), and it is never shown (see Surprises).
- **Evidence:** desktop/tests/__snapshots__/native/fork-dialog.view.txt (Dialog "Fork session" focused; RadioButton "fork-kind-0-on" selected; Button "Now"; Button "Claude Code"; Button "devbox-1"; TextInput "Fork branch" value="humata/ledger-schema-migration-2"; Button "Cancel", Button "Fork"). PNG: desktop/tests/__snapshots__/native/fork-dialog.png.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=the_fork_dialog_matches_frame_62_and_forks_what_it_shows` (asserts `fork-kind-0-on`, `fork-from`, `fork-agent`, `fork-machine`, `fork-branch`, `fork-preview` are present and the branch is not empty). Live: `bazel test //apps/humata2/desktop:live_flows_test --test_filter=forking_from_the_fork_dialog_starts_a_child_session_on_the_daemon`.

### 2. Other ways in: ⇧⌘F, the palette, and the Desk
- **Screen:** No screen of its own. ⇧⌘F (ui/src/keys.rs ForkSession) and the palette commands `Fork this session…` (no title) or `Fork “<title>”` (features/palette/src/lib.rs lines 788-795) open the same dialog on Code + context (features/app/src/commands.rs Command::Fork and desk.rs `fork_unless_desk`). On a Desk session both are dropped: `fork_unless_desk` returns no effect, and the palette lists no Fork (lib.rs line 779).
- **Offered:** Palette row, shortcut. Nothing on a Desk session.
- **Action:** Press ⇧⌘F, or pick the palette row, with a channel session open. Or open a Desk session and try the same.
- **Result:** Channel session: the dialog opens as in step 1. Desk session: nothing happens.
- **Evidence:** No snapshot. Palette and shortcut reach: features/app/tests/every_action.rs maps `Command::Fork` to the keys table.
- **Replay check:** PENDING RUN: `bazel test //apps/humata2/features/app:every_action_test` (the test checks the keys table and command routing; it does not open the dialog). For the visible result, the coach may pick a palette-driven live test; none was found that presses ⇧⌘F and checks `state.session.fork`.

### 3. Kind cards: Code + context, Context only, Code only
- **Screen:** Three bordered cards at the top of the dialog, each a radio with a name and a sentence:
  - `Code + context`: "New copies of the files as they are now, uncommitted changes included, and an agent that knows the conversation." (id `fork-kind-0`, `-on` when picked)
  - `Context only`: "An agent that knows the conversation and works in this session's files, with no copies, so its edits land here." (id `fork-kind-1`)
  - `Code only`: "New copies of the files and a fresh agent with no conversation." (id `fork-kind-2`)
  Names come from `ForkKind::name()` (clients/src/model.rs 423-433).
- **Offered:** Click any card. Keyboard focus follows the radio role.
- **Action:** Click a card. Sends `SessionAction::ForkKind(kind)`. A pick of Code only clears `before_turn` and closes the Fork from picker (features/session/src/lib.rs 1475-1488). A pick is ignored while a fork is being sent.
- **Result:** The chosen card gets the accent border and `-on` in its id. The rows under it change:
  - Code + context: Fork from, Agent, Machine, Branch, Preview.
  - Context only: Fork from, Agent, Machine, no Branch (it shares the parent's clones), Preview.
  - Code only: no Fork from, Agent, Machine, Branch, Preview.
- **Evidence:** fork-dialog.view.txt (kind 0 on), fork-dialog-dark.view.txt (kind 1 on: no Branch group, Fork from "Now" still shown).
- **Replay check:** `bazel test //apps/humata2/desktop:live_flows_test --test_filter=a_code_only_fork_copies_the_files_into_a_fresh_conversation` (asserts `Fork from` is absent after `fork-kind-2`); `bazel test //apps/humata2/desktop:native_test --test_filter=the_fork_dialog_matches_frame_62` (clicks fork-kind-2, then fork-kind-1 and the dark snapshot).

### 4. Fork from: Now, or before an earlier turn
- **Screen:** Row `Fork from` with a grey pop-up `Now` (id `fork-from`, chevron). Beside it: "or before an earlier turn". Opening the pop-up shows a wrap row: `Now` (id `fork-from-now`), then one button per turn, `Before <n>: <first 40 characters of the prompt>` (ids `fork-from-<n>`).
- **Offered:** The pop-up (opens and closes), Now, each `Before n` turn. Not on Code only.
- **Action:** Click `Now`. Click the pop-up and then `Before 2: write…` (live test). Opening the pop-up the first time asks the daemon for the turns (`SessionAction::ForkTurns`, once per dialog; features/session/src/lib.rs 1540-1555).
- **Result:** The pop-up label changes to `Before turn 2`; the draft's `before_turn` is set. A daemon that can't list turns gives an empty list, so the pop-up only shows Now (lines 1547-1553). Picking a turn closes the picker.
- **Evidence:** PENDING RUN for the open picker image (no snapshot of the picker exists). The live test asserts the row reads "Before turn 2" after the pick (fork_kinds.rs).
- **Replay check:** `bazel test //apps/humata2/desktop:live_flows_test --test_filter=a_fork_from_before_a_turn_carries_the_conversation_up_to_it`; reducer: `bazel test //apps/humata2/features/session:fork_move_test --test_filter=the_turns_are_read_once_when_the_picker_first_opens`.

### 5. Agent: the parent's agent, or another enabled one
- **Screen:** Row `Agent` with a pop-up showing the agent name (`Claude Code` for Claude; otherwise the agent's name, id `fork-agent`). Opening it lists the agents enabled in Settings > Agents, then `Shell` (AGENTSET D725: ui/src/overlay_fork.rs line ~255). Items are `fork-agent-pick-<index>`.
- **Offered:** The pop-up, each enabled agent, and Shell. A choice of the parent's own agent clears the override (features/session/src/lib.rs 1504-1512).
- **Action:** Click `fork-agent`, then an agent. Sends `ForkAgent(agent)` and closes the picker.
- **Result:** The pop-up shows the new agent. When the agent differs from the parent, the Preview's conversation line changes to "conversation sent to <agent> as a summary" and the note under the Preview turns orange (step 7). The daemon hands over the conversation as a file and summary when the agents differ (daemon/sessions/src/fork.rs `hands_over`).
- **Evidence:** Snapshot shows only the closed state ("Claude Code", fork-dialog.view.txt). The open list and the other-agent preview are not in any snapshot.
- **Replay check:** Reducer: `bazel test //apps/humata2/features/session:fork_move_test --test_filter=picking_the_parents_own_machine_or_agent_clears_the_override`. Daemon handover: `bazel test //apps/humata2/daemon/humatad:fork_test --test_filter=a_fork_with_another_agent_is_handed_the_conversation_and_a_summary`.

### 6. Machine: where the fork runs
- **Screen:** Row `Machine` with a pop-up showing the machine name (the parent's, e.g. `devbox-1`; id `fork-machine`). Opening it lists the online machines only (ui/src/overlay_fork.rs `filter(|m| m.online)`), as `fork-machine-<id>` buttons.
- **Offered:** The pop-up, each online machine. Offline machines are not listed. An offline parent machine still shows its name.
- **Action:** Click `fork-machine`, then `this-mac` (native test sends `ForkMachine`). Picking the parent's own machine clears the override.
- **Result:** The pop-up shows the picked machine, and the daemon receives `machine = this-mac`.
- **Evidence:** fork-dialog.view.txt (`Button "devbox-1"`); fork-dialog-dark.view.txt (`Button "This Mac"`, the forked child's machine).
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=the_fork_dialog_matches_frame_62_and_forks_what_it_shows` (asserts `sent.machine == this-mac`).

### 7. Preview and same-agent note
- **Screen:** Group "Preview" (`fork-preview`), in Menlo 11 pt: parent line `<parent>   [<repos>]  <agent> · <machine> · <n> turns`; then `└─⑂ <title>   <kind name, lower case>`; a place line (`new clones of <repos> on <branch>`, or `works in the files of <parent>, no copies` for Context only); a conversation line. Below it, unless Code only: a green check "Same agent: the conversation is copied exactly." or an orange info "A different agent can't read this conversation, so <agent> starts from a summary of it."
- **Offered:** Nothing. Read-only.
- **Action:** Change the kind, agent, machine or branch and watch the preview update.
- **Result:** Text updates on each change. The orange note appears only when the agent differs from the parent and the kind isn't Code only.
- **Evidence:** The Preview's text is not in the a11y view (the group shows no text children in fork-dialog.view.txt), so only the PNG shows it: desktop/tests/__snapshots__/native/fork-dialog.png, fork-dialog-dark.png.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=the_fork_dialog_matches_frame_62` (asserts `fork-preview` is present). The words are read from the PNG by eye.

### 8. Branch
- **Screen:** Row `Branch`: a text input labelled `Fork branch` (id `fork-branch`), value `humata/<parent-branch-prefix>/<slug>-2`, under it the grey line "Humata creates this branch in each new clone." Absent on Context only.
- **Offered:** Type in the input. The input is disabled while the fork is being sent.
- **Action:** Type a branch name. Sends `ForkBranch(text)` on each change (ui/src/overlays.rs line 157).
- **Result:** The draft branch changes; the preview follows. A typed branch is sent with the fork (fork_move.rs asserts `sent.branch == branch`). The daemon checks the name (`check_fork_branch`, daemon/sessions/src/fork.rs 137-142) and refuses a taken one.
- **Evidence:** fork-dialog.view.txt (`TextInput "Fork branch" value="humata/ledger-schema-migration-2" placeholder="uhvesta/ledger-schema-2"`).
- **Replay check:** `bazel test //apps/humata2/features/session:fork_move_test --test_filter=opening_the_fork_dialog_suggests_a_branch_beside_the_parents`; daemon refusal of a taken branch: `bazel test //apps/humata2/daemon/humatad:fork_test` (PENDING RUN to pick the name).

### 9. Cancel
- **Screen:** Buttons `Cancel` (id `fork-cancel`) and `Fork` (id `fork-create`, accent) at the bottom right.
- **Offered:** Cancel, Fork. Both disabled while the fork is being sent (Fork also disabled when the title is blank, which the app never does).
- **Action:** Press Cancel.
- **Result:** The dialog closes (`state.session.fork` is None). Nothing is sent. The session page is unchanged.
- **Evidence:** fork-dialog-dark.view.txt is the state before Cancel.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=the_fork_dialog_matches_frame_62_and_forks_what_it_shows` (its last lines click fork-cancel and assert the draft is gone).

### 10. Fork (Code + context, from Now, to this Mac)
- **Screen:** While sending, the button reads `Forking…` and the dialog's controls are disabled.
- **Offered:** Nothing while sending.
- **Action:** Press Fork. Sends `ConfirmFork`, which makes `ForkOptions { kind, title, before_turn, agent, machine, branch }` and calls `daemon.fork` (features/session/src/lib.rs 1557-1582).
- **Result:** On success the dialog closes; the app opens the child (`Delegate::OpenSession`); the sidebar nests the child under its parent with the fork glyph; the child's Info panel lists it under Forks. On failure the dialog stays (step 11).
- **Evidence:** desktop/tests/__snapshots__/native/16-sidebar-fork-selected.view.txt (sidebar fork row selected, Changes panel on `api/src/client.rs`).
- **Replay check:** `bazel test //apps/humata2/desktop:live_flows_test --test_filter=forking_from_the_fork_dialog_starts_a_child_session_on_the_daemon` (asserts the child's `parent` is the original and the daemon lists it).

### 11. A fork the daemon refuses keeps the dialog
- **Screen:** The dialog stays with its title and kind. A red `fork-error` line (role Alert) appears above the buttons with the daemon's words. The daemon's refusal texts are in daemon/humatad/src/session_service/fork.rs (for example "the session has N turns, so there is no turn 9 to fork before"; "turn 1 is the first prompt, so there is no conversation before it to carry").
- **Offered:** Edit anything, Cancel, Fork again.
- **Action:** Send a turn the session does not have (`ForkBeforeTurn(Some(9))`) and press Fork.
- **Result:** No child, no clone. The app stays on the parent. A second Fork from Now with the same title starts the child.
- **Evidence:** No snapshot. The error text is in the live test's assertions only (`state.session.error`).
- **Replay check:** `bazel test //apps/humata2/desktop:live_flows_test --test_filter=a_refused_fork_keeps_the_dialog_and_a_retry_makes_the_child`.

### 12. Code only: a copy of the files, a fresh conversation
- **Screen:** The child opens; its page shows the child's title. Its own clone exists under the channel's sessions folder.
- **Offered:** As on any session page.
- **Action:** Pick Code only, Fork.
- **Result:** The daemon keeps untracked files (the test writes `notes.txt` into the parent's clone and checks the copy). The child's conversation holds none of the parent's turns. The child has `fork = Shared`.
- **Evidence:** PENDING RUN for a screen image of the child page. No snapshot.
- **Replay check:** `bazel test //apps/humata2/desktop:live_flows_test --test_filter=a_code_only_fork_copies_the_files_into_a_fresh_conversation`.

### 13. Context only: the conversation, no copies, the parent's branch
- **Screen:** The child opens. Its Info panel shows the parent's clone and branch.
- **Offered:** As on any session page.
- **Action:** Pick Context only, Fork.
- **Result:** No copy of the files. The child works on the parent's branch (`child.branch == parent_branch`) and in its folder. The child's conversation holds the parent's turns. Its edits land in the parent's clone. Freeing it later removes only its handover file (daemon tests freeing_a_context_only_fork...).
- **Evidence:** PENDING RUN. No snapshot.
- **Replay check:** `bazel test //apps/humata2/desktop:live_flows_test --test_filter=a_context_only_fork_carries_the_conversation_and_shares_the_parents_files`.

### 14. Fork from before turn 2
- **Screen:** As in step 4, with `Before turn 2` on the Fork from row.
- **Offered:** As in step 4.
- **Action:** Pick `Before 2: <second prompt>`, Fork.
- **Result:** The child's conversation holds turn 1 only; the parent's copy still holds both. The code in the child's clone is the code as it is now.
- **Evidence:** PENDING RUN. No snapshot.
- **Replay check:** `bazel test //apps/humata2/desktop:live_flows_test --test_filter=a_fork_from_before_a_turn_carries_the_conversation_up_to_it`.

### 15. The Move dialog, the other sheet that picks a machine (for contrast)
- **Screen:** `Dialog "Move session"` (not a fork): radio machines `move-machine-<id>` (this-mac, devbox-1, gpu-box, cloud-vm-3), group "What moves", and the button `Move to <machine>`.
- **Offered:** Pick a machine, Cancel, Move. Offline and current machines cannot be picked.
- **Action:** Session menu › Move to another machine…, pick, Move.
- **Result:** Daemon receives the move. Dialog closes.
- **Evidence:** desktop/tests/__snapshots__/native/move-dialog.view.txt.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=the_move_dialog_matches_frame_63_and_moves_to_the_picked_machine`.

## Not reachable headless
- The agent pop-up's open list (the agents enabled in Settings, plus Shell). No test presses `fork-agent`, and no snapshot has it open.
- The orange "a different agent can't read this conversation" note. No snapshot and no test with a different agent in the dialog.
- The Fork from picker's open state (no snapshot). It is reached through the native harness only as far as the live test's daemon round trip.
- The Preview's words (the Preview group has no text in the view tree; only the PNG shows them).
- ⇧⌘F and the palette fork row: no test presses them and checks the dialog opens.

## CLI: `humata session fork`
Source: cli/src/args.rs lines 439-458 and cli/src/commands/sessions.rs lines 202-240.

### C1. `humata session fork <session> --kind <kind>`
- **Screen:** Terminal. Prints the daemon's `Operation` (or with `--json`, the JSON). No prompt.
- **Offered:** Flags: `--kind` (required; values `code-and-context`, `code-only` (alias `code`), `context-only` (alias `context`); `--copy` is an alias of `--kind`); `--agent <agent>` (default: parent's); `--before-turn <n>` (n >= 2; default: from now); `--title <text>` (default empty); `--branch <name>` (default: named from title and branch prefix); global `--machine <name>`; global `--dry-run`.
- **Action:** Example: `humata session fork ledger --kind code-only --title "Ledger spike"`.
- **Result:** Expected exit code from cli/tests/sessions.rs: 0 on success. `--before-turn 1` exits with the usage code (`exit::USAGE`) and sends nothing (test `fork_before_turn_one_is_refused_and_nothing_forks`). `--dry-run` prints a preview and sends nothing (`fork_dry_run_previews_without_forking`).
- **Evidence:** PENDING RUN (fake-daemon test output; no real-daemon transcript yet).
- **Replay check:** `bazel test //apps/humata2/cli:sessions_test --test_filter=fork`. The eight fork tests in cli/tests/sessions.rs cover the kinds, the typed branch, the agent and turn, the dry run, and turn 1.

### C2. `humata session fork ... --before-turn 1` (CLI refuses, daemon refuses)
- **Result:** The CLI refuses turn 1 before it reaches the daemon (usage exit). The daemon refuses it too with "turn 1 is the first prompt, so there is no conversation before it to carry" (daemon/humatad/src/session_service/fork.rs 509-516). The app's picker offers `Before 1` if turn 1 exists (see Surprises).
- **Replay check:** `bazel test //apps/humata2/cli:sessions_test --test_filter=fork_before_turn_one_is_refused`.

## Surprises
- **The Fork dialog has no title field.** The draft's title is always `<parent title> 2` (features/session/src/lib.rs StartFork). The `ForkTitle` action exists, but no UI code sends it; only the live test sends it ("Retry path"). The Fork button's empty-title guard (`fork.title.trim().is_empty()`) therefore can't be reached by a person. A person cannot name the fork in the dialog; they can only rename it after it starts (Session actions › Rename…).
- **Fork from offers `Before 1`, which the daemon refuses.** The picker lists every turn from `ListTurns` with no filter (ui/src/overlay_fork.rs lines ~268-279), including turn 1. The daemon and the CLI refuse turn 1 (daemon fork.rs 509; cli/tests/sessions.rs `fork_before_turn_one...`). Code reading only; PENDING RUN check: `humata session fork <s> --kind code-and-context --before-turn 1 --dry-run` against a throwaway daemon (see the setup below), and a screenshot of the open picker.
- **Two buttons named "Fork".** The page's `Fork` and the dialog's `Fork` share a label; the native and live tests note the driver can't pick one and send `ConfirmFork` directly. A screen reader user hears the same name twice.
- **Dark dialog's terminal banner says "(Code + context)" while the dialog shows Context only.** The text "Forked from “Ledger schema migration” before turn 2 (Code + context) / Conversation copied" is in the fixture's terminal log, not computed from the dialog (fork-dialog-dark.view.txt, Log "Terminal output"). A reader of the frame would take it as the kind picked.
- **The Preview's same-agent line may be wrong when the parent has no conversation file.** The app says "conversation copied exactly (same agent)" for any same-agent fork. The daemon hands over a summary when the parent's agent has no conversation file, even for the same agent (daemon fork.rs `hands_over`: `agent != parent || conversation_file.is_empty()`). Code reading only.
- **A Shell fork says "as a summary".** For Context only or Code + context with another agent, the line is "conversation sent to <agent> as a summary"; with Shell it reads "conversation sent to Shell as a summary" (ui/src/overlay_fork.rs `conversation` match). Code reading only; not tested.
- **The machine pop-up lists only online machines**, and gives no reason an offline one is missing. The parent's own name still shows if the parent's machine is offline.
- **The preview's words are invisible to the view tree.** The Preview group has no text children in fork-dialog.view.txt; a tester who checks the .view.txt won't see the place or the conversation line.
- **The CLI's `session fork` needs `--kind` (no default), while the app defaults to Code + context**, and the machine is the global `--machine` flag, not a fork flag. `--before-turn` range is 2 or more, so the CLI already forbids the `Before 1` the app offers.

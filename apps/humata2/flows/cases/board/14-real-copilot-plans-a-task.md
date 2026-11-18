# A real Copilot agent on Claude Haiku 5.5 plans a small task on the Board

Layer: verify (Board, a real agent CLI, a real daemon, this Mac). Platforms: macOS.

This is the one case with a real model. A person tells the agent, in plain words, to plan a small task on the Board. The Board is then checked through `humata` output only.

## Starting state

- The rig from the [README](README.md), with `COPILOT_HOME`, `COPILOT_CACHE_HOME`, `CODEX_HOME` and `CLAUDE_CONFIG_DIR` also pointing under `$D`. The Copilot login in that `COPILOT_HOME` is a copy. The owner's `~/.copilot` is never read or written.
- Copilot is switched on in Settings › Agents (`verify/09`), and the installed Copilot CLI is 1.0.113 or later.
- `#payments` exists with one repo, `acme/api`. Its board has one item of the person's: PAY-1 "Ship the version flag", To do. The Global board is empty.
- `H` is the full path of the built `humata`. `P` means a command run by the person in the tester's shell.

## Reproduce headless

- All of it is `humata` against the throwaway daemon. No window. Keep every command's stdout, stderr and exit code, and at the end `humata session read "Plan flag" > $D/conversation.txt`.
- Type into the agent with `humata session type "Plan flag" '<text>'`. Read what it printed with `humata session read`.
- Copilot's own permission asks (to run a shell command) arrive in the Inbox through its `permissionRequest` hook. Answer each with `P: humata inbox` then `humata inbox approve <id>`.
- Not headless: nothing, if the copied login works. If it does not, the nearest equivalent is the loopback model server of `terminal/provider/tests/*_resume.py`, which proves the plumbing but not that a model chooses the Board.
- Cost: a handful of Haiku turns. Stop the session at the end: `humata session stop "Plan flag"`.

## Steps

1. **I do:** `P: humata session start --agent copilot --channel payments --title "Plan flag"`.
   **I see:** a session id. `humata ls` shows it running.
   **State and events:** Copilot starts with `--session-id … --plugin-dir <hooks plugin>`. Session created.
   **Status:** Works (real daemon) with a scripted Copilot: `daemon/humatad/tests/continue_in.rs::a_copilot_session_keeps_the_version_its_cli_said_at_launch`. Real CLI: manual run only (`real/05`).

2. **I do:** type `/model`, pick Claude Haiku 5.5 (`claude-haiku-5.5`), lowest reasoning, "this session only". Accept the folder-trust prompt if it shows.
   **I see:** Copilot prints the model name.
   **State and events:** Humata only types keys. `session start --model` is recorded but never reaches Copilot.
   **Status:** Works (real CLI, manual run in `real/05`); no test. `--model` not passed: Not built (`daemon/humatad/src/session_service.rs:488` records it; `daemon/sessions/src/agents.rs:572` has no placeholder).

3. **I do:** type, in plain words and with no command named: `Plan this small task on the Humata Board before you write any code: add a --version flag to the tool in api/. Make one Board item per step, say which step depends on which, and move the first step to In progress.`
   **I see:** (expected) the agent uses Humata's board tools and says which items it made.
   **I see (likely today):** it writes a plan in the chat or a Markdown file, or says it has no Board tool.
   **State and events:** (expected) BoardItem created ×N by this session.
   **Status:** Not built, no milestone. The agent has no Humata MCP server, no Board skill and no `humata` on its `PATH` (case 9). Record what it did instead; that is the evidence for the gap.

4. **I do:** `P: humata --json board --session "Plan flag"` and `P: humata --json board --channel payments`.
   **I see:** (expected) at least three new items. (Likely today) only PAY-1.
   **Status:** Not built, as step 3. If items do appear, tag the run "Works (real CLI, manual run)" and say how the agent found the command.

5. **I do:** only if step 4 showed nothing new. Type: `Humata has a command for its Board at <H>. Run "<H> board --help" and "<H> whoami", then do the planning I asked for on your channel's board.`
   **I see:** Copilot asks to run each shell command. I approve each from the Inbox (`P: humata inbox approve <id>`). It runs `board add` several times, `board edit … --depends-on …`, and `board move … in-progress`.
   **State and events:** each `humata` call in the tab is bound to this session. `channel_board_add` and `channel_board_edit_own` are Allow, so no board ask is raised. BoardItem created and updated, by this session.
   **Status:** Works (real daemon) for a real `humata board` call from an agent's tab: `desktop/tests/live_boards.rs::approving_an_agents_move_from_the_items_drawer_moves_it_and_returns_the_call` (scripted agent). A real model doing it: never run.

6. **I do:** `P: humata --json board --channel payments > $D/board.json`.
   **I see:** and check, in the JSON:
   - at least three items besides PAY-1, with keys `PAY-2` and up;
   - every new item's `created_by` is the "Plan flag" session, not the user;
   - at least one item has a non-empty `depends_on`, and no item depends on itself;
   - exactly one new item has `column` in progress; the rest are Backlog or To do;
   - PAY-1 is unchanged (same `revision` as at the start).
   **Status:** Code, no test for this whole shape. Each field is covered in the daemon: `daemon/humatad/tests/boards.rs`.

7. **I do:** `P: humata --json board history PAY-2`.
   **I see:** "created" by the session, then any edits and the move, in order.
   **Status:** Works (real daemon) for the daemon: `boards.rs::history_records_what_happened_in_order_with_its_kind`.

8. **I do:** type: `The first step is done. Mark it done on the Board. Also move PAY-1 to In review, and wait for my answer.`
   **I see:** its own item moves to Done at once. The PAY-1 move holds.
   **State and events:** PAY-1 is the person's, so `channel_board_edit_others` asks. InboxRequest created (Pending).
   **Status:** Works (real daemon) with a scripted agent: same `live_boards.rs` test.

9. **I do:** `P: humata --json inbox`, then `P: humata inbox show <id>`, then `P: humata inbox approve <id>`.
   **I see:** a board request from "Plan flag": move PAY-1, To do → In review. No reason line (the CLI can't send one). After the approval, the agent's command returns and it reports the move.
   **State and events:** InboxRequest updated (Approved); BoardItem updated (In review).
   **Status:** Works (real daemon). Same test; `daemon/humatad/tests/boards.rs::an_agents_move_waits_and_applies_on_approval_and_only_the_user_sees_it_pending`.

10. **I do:** `P: humata --json board --channel payments`, then `P: humata session stop "Plan flag"`.
    **I see:** the first step in Done with a `done_at`; PAY-1 in In review; everything else as before.
    **Status:** Works (real daemon) for the daemon half.

## Branches

### The agent runs `humata` without `--wait` and gives up

When Copilot's shell is not a terminal and the move in step 8 exits 12 at once. The agent may report failure. The request is still in the Inbox; approving it still moves PAY-1.
**Status:** Code, no test. `cli/src/call.rs:134`. Note which happened.

### The agent puts items on the wrong board

When it uses `--global`. Each add raises an Inbox card and nothing appears until allowed. Deny them and say "use your channel's board".
**Status:** Works (real daemon). `daemon/humatad/tests/board_tools.rs::creating_on_the_global_board_waits_for_the_user`.

### Copilot asks before every command

When every `humata` call needs a shell approval. That is Copilot's own permission ask, not a board ask. Count them: a plan of four items is about eight approvals.
**Status:** Works (real daemon) with fixture hooks: `daemon/humatad/tests/hooks.rs::each_agent_gets_the_answer_in_its_own_shape`.

### The login or the model is missing

When Copilot asks to log in, or `/model` has no Claude Haiku 5.5. Stop. This is the environment, not a Board bug.
**Status:** Blocked by the environment.

## Expected outcomes

- [ ] Told in plain words, the agent plans on the Board without being given a command.
- [ ] Its items are owned by its session, with at least one depends-on edge and one item In progress.
- [ ] It changes its own items with no ask, and asks before moving the person's item.
- [ ] The person's approval through `humata inbox approve` moves the item and returns the agent's call.
- [ ] The owner's own Copilot home is unchanged after the run.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Start Copilot | Works (real daemon), scripted | `continue_in.rs` |
| 2 Pick the model | Works (real CLI, manual); `--model` Not built | `real/05`; `session_service.rs:488` |
| 3 Plain-words prompt | Not built | case 9 |
| 4 Check the board | Not built | case 9 |
| 5 Prompt naming the command | Works (real daemon), scripted agent | `live_boards.rs` |
| 6 Check items, owner, edges | Code, no test | `boards.rs` per field |
| 7 History | Works (real daemon) | `boards.rs::history_records_…` |
| 8 Own move, then an ask | Works (real daemon), scripted agent | `live_boards.rs` |
| 9 Approve by CLI | Works (real daemon) | `boards.rs::an_agents_move_waits_…` |
| 10 Final check | Works (real daemon) | `boards.rs` |
| Exit 12 | Code, no test | `cli/src/call.rs:134` |
| Wrong board | Works (real daemon) | `board_tools.rs` |
| Copilot's own asks | Works (real daemon), fixture hooks | `hooks.rs` |
| No login or model | Blocked by the environment | none |

## Gaps and likely bugs

- Not built: step 3 as the owner means it. An agent can't plan on the Board from plain words, because nothing tells it the Board exists or gives it the tools (case 9). Step 5 is a workaround, not the feature.
- Not built: `session start --model` reaching the agent.
- Likely bug: a board ask from the CLI has no reason, so step 9's card can't say why (`cli/src/commands/boards.rs:128`).
- Likely bug: each board command costs a Copilot shell approval on top of Humata's own rules, so a small plan is many Inbox cards. With MCP registered there would be none for allowed writes.
- No test, manual or automatic, has had a real model write to the Board.

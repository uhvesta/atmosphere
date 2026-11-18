# Claude, Codex and Copilot work in one channel across this Mac and the VM, and each answers, moves and continues correctly

Layer: real (real `claude`, `codex` and `copilot` CLIs, real `humatad2`, this Mac, the `h2-terminal-c1` VM). Platforms: macOS host, Linux VM. Builds on `real/05` and `real/06` (Copilot), and on `agents/10-roaming-between-agents.md` (continue in another agent).

## Starting state

- This Mac runs a real `humatad2` in a temp state folder. It was started with an `--agents` JSON file that adds custom kinds `codex` (`conversation_format` `codex-rollout`) and `copilot` (`copilot-session-state`). `claude` is built in. There are no built-in `codex` or `copilot` kinds yet (ROAM batch 3, C4, X2).
- The channel `acme/api` has two machines: this Mac and the VM (added as in `real/01`, with Humata's tools in its `~/.humata2/bin`). It has one repo, `acme/api`, with `README.md` clean.
- The Inbox is empty and permission asks wait in the Inbox (the default).
- The three CLIs are installed on this Mac and, in a run folder, on the VM (as in `real/04`). Each has its own isolated home with a copied login.
- The channel board has no items. No session exists yet.

## Test setup

- Isolation. One temp folder per machine. On each, pass `COPILOT_HOME`, `CODEX_HOME`, `CLAUDE_CONFIG_DIR` and `HOME` into the daemon (`daemon/humatad/src/serve.rs:456-466`). Never touch the owner's `~/.copilot`, `~/.codex`, `~/.claude`, `~/.humata2` or SSH keys. In the REALFLOW run Copilot wrote `recentModelIds` to the real `~/.copilot/config.json` despite "this session only", so an isolated home is a must.
- The VM: `./terminal-ssh-demo.sh start`, then `status`; follow `terminal/platform_test/SSH_DEMO.md`; reach it with `ssh -F ~/.lima/h2-terminal-c1/ssh.config lima-h2-terminal-c1`. The read-only check is `//apps/humata2/terminal/platform_test:ssh_demo_status_test`. Never restart or recreate the shared VM.
- The VM has 1 CPU, 0.75 GiB RAM and 4 GiB disk. Two Node-based CLIs running at once there is tight. Put at most two agents on the VM at a time, and watch memory.
- **Needs the owner's login** for every step that talks to a real model: Claude login, Codex `login`, Copilot device login. Each token must land only in the run's temp home. A free Copilot account may not offer Claude Haiku 5.5. Alternative without logins: the loopback model servers in `terminal/provider/tests/*_resume.py`. They prove resume in the terminal component only, not through `humatad2`.
- **Needs a free screen** for every step in the app window. Headless equivalent: the `humata` CLI (`session list`, `inbox approve`, `session move`, `board`), or a `desktop/tests/live_*` style test against a real `humatad2`.
- Cleanup: stop the daemons, delete the temp folders and the copied logins on both machines, and confirm the owner's own CLI homes are unchanged.

## Steps

1. **I do:** start `humatad2 serve --agents <file>` on this Mac and open channel `acme/api`.

   **I see:** Settings › Agents lists Claude Code, `codex` and `copilot`.

   **State and events:** the custom kinds come from the JSON; their hook placeholders are filled by the daemon.

   **Status:** Fakes only. `daemon/sessions/src/agents.rs::codex_and_copilot_hooks_fill_their_placeholders` checks the fill with strings. No test starts a daemon with the three real CLIs.

2. **I do:** start a Claude session `Claude on the Mac` on this Mac. In its tab I type `/model` and pick Claude's cheapest model (Haiku).

   **I see:** Claude's own model picker, then the new model name.

   **State and events:** Humata only types the keys into the agent's terminal and records no model. A moved or resumed session keeping the model is the CLI's own setting, unproven.

   **Status:** Code, no test. Neither the REALFLOW run nor a test started real Claude through `humatad2`. The Claude resume is proven only in the terminal component: `//apps/humata2/terminal/provider:claude_resume_test` (manual, loopback model). **Needs the owner's login.**

3. **I do:** start a Copilot session `Copilot on the Mac` on this Mac. Accept the folder-trust prompt. Type `/model` and pick Claude Haiku 5.5, low reasoning, the smallest context, "this session only".

   **I see:** Copilot's picker, then the model name.

   **State and events:** the session record has an `AgentLaunch` with the conversation id and file under `COPILOT_HOME/session-state/<id>/events.jsonl`.

   **Status:** Works (real CLI, manual REALFLOW run). It worked live; no test pins it. Daemon half: `daemon/humatad/tests/continue_in.rs::a_copilot_session_keeps_the_version_its_cli_said_at_launch` (Works (real daemon), fixture agent). Fixed (B12, D754): `humata session type` presses Enter by default, so each typed line needs no separate Return. **Needs the owner's login.**

4. **I do:** start a Codex session `Codex on the VM` on the VM machine. Answer "Hooks need review" with Continue without trusting. Type `/model` and pick GPT-6 Luna, low reasoning, `s` for this session only.

   **I see:** Codex's picker, then the model name.

   **State and events:** the session lives on the VM, in its own folder there. The host lists it with machine = the VM.

   **Status:** Works (real CLI, manual REALFLOW run) for the start and the model choice on this Mac; no test pins it, and Codex on the VM was never run live. Daemon half: `daemon/humatad/tests/fork_remote.rs::a_codex_fork_from_box_1_onto_box_2_before_turn_2_carries_turn_1_only` starts a Codex-kind session on a box (fixture agent, fake `ssh`). **Needs the owner's login.**

5. **I do:** read the session list: `humata session list`, or the sidebar.

   **I see:** three sessions, each with its agent kind (Claude Code, `codex`, `copilot`), its machine (this Mac, this Mac, the VM) and a state.

   **State and events:** each row has the kind from the kind's id, and a state from the hooks (Idle, Working, Needs input) or from quiet output when a kind has no hooks.

   **Status:** Code, no test. No test lists mixed kinds on two machines together. Single-kind state: `daemon/humatad/tests/turns.rs::the_hooks_move_a_session_between_idle_working_and_needing_input` (Works (real daemon), fixture hooks). **Needs a free screen** for the sidebar.

6. **I do:** give each agent a tiny edit, each prompt naming the clone folder (Humata does not tell agents where the repo is): `In the api/ folder, add a line to README.md` for Claude, `... to CHANGES.md` for Copilot, `... to NOTES.md` for Codex. All three run at once.

   **I see:** all three go to Working. Each wants to write a file and asks.

   **State and events:** the three agents' hooks (`PreToolUse`/`permissionRequest` shapes) hold and post one Inbox card each. The session folders are separate, so the edits cannot collide.

   **Status:** Works (real CLI, manual REALFLOW run) for Copilot alone: three asks were approved live with `humata inbox approve`. Claude and Codex asks never ran live. Daemon half for all three shapes: `daemon/humatad/tests/hooks.rs::each_agent_gets_the_answer_in_its_own_shape` (Works (real daemon), fixture hooks). Codex needs its hooks trusted first: `daemon/humatad/tests/codex_trust.rs::codexs_hooks_need_trust_until_one_of_them_is_heard`.

7. **I do:** open the Inbox. Three cards wait, each naming its session, agent and machine. I answer them in order, oldest first: Allow the Claude one, Allow the Codex one, Deny the Copilot one with the note "not that file".

   **I see:** each card leaves the Inbox when answered. Claude and Codex finish their edit. Copilot is told no and says so.

   **State and events:** each answer reaches only its own session's waiting hook, in that agent's own shape. The cards are listed oldest first. A request takes only its first answer.

   **Status:** Works (real daemon) for one answer reaching its hook and for the request taking one answer: `daemon/humatad/tests/hooks.rs::an_inbox_approval_reaches_the_waiting_hook`, `::an_inbox_deny_reaches_the_hook_with_its_note` and `daemon/humatad/tests/inbox.rs::a_request_takes_only_its_first_answer` (fixture hooks, single session). Code, no test for three kinds waiting at once and being answered in order. The headless form is `humata inbox approve <id>`. **Needs a free screen** for the Inbox window.

8. **I do:** on the channel board I add an item `Check the README line`, and each agent adds a comment or an item of its own through the `humata` CLI.

   **I see:** one board with all items. Items made by agents on the global board wait for my approval.

   **State and events:** the board is the shared place. The agents reach it with their own session tokens.

   **Status:** Works (real daemon) with a fake agent: `daemon/humatad/tests/board_tools.rs::an_agent_works_its_session_board_from_making_an_item_to_deleting_it` and `daemon/humatad/tests/boards.rs::an_agents_write_on_the_global_board_waits_in_the_inbox_until_approved`. A real Claude, Codex or Copilot driving the board is unproven.

9. **I do:** while Claude and Codex keep running, move the Copilot session to the VM: `humata session move <copilot session> --to <vm>`, or ⌥⌘M.

   **I see:** the Copilot session shows Moving, then runs on the VM. The Claude and Codex sessions keep their state.

   **State and events:** `MoveSession` steps: check, pause, pack, copy, verify, resume, switch, release. Pack reads the Copilot state through the adapter and rewrites it to name the new folder.

   **Status:** Works (real daemon) for a move with a fixture agent: `daemon/humatad/tests/move_session.rs::a_session_moved_box_1_to_box_2_keeps_id_branch_work_and_conversation_and_resumes`, and for the Copilot carry: `daemon/sessions/src/carry_roam.rs::a_carried_copilot_conversation_names_no_old_path_and_reads_on_the_new_machine`. No test moves a real Copilot session, and none runs two other agents across a move. Likely bug (seen live, B5): resume looks for the conversation under `--root` as home, so the moved Copilot may not find its conversation. **Needs a free screen** for ⌥⌘M.

10. **I do:** type a follow-up to the moved Copilot: `What line did you add?`.

    **I see:** it answers from its conversation, on the VM.

    **State and events:** the conversation file is under the VM's `COPILOT_HOME`, named for the VM's folder.

    **Status:** Likely bug (seen live, B7 and B10): a resume after a stop is treated as an interruption and kills the correctly resumed agent, and attach is refused while it resumes. Confirm by moving a stopped Copilot and reading its state after the resume.

11. **I do:** continue the Codex session in another agent: fork it before turn 2, with agent Claude Code, on this Mac.

    **I see:** a new Claude session whose first message names a handover file holding the Codex conversation's turn 1 only.

    **State and events:** the handover is `.humata/handover-<id>.jsonl` beside the clones (`continue_in.rs` calls the same path "handed over to another agent"). Humata does not script Claude to import it.

    **Status:** Works (real daemon) for the mechanism with fixture agents: `daemon/humatad/tests/continue_in.rs::a_codex_session_handed_to_another_agent_before_turn_2_hands_over_turn_1_only` and `daemon/humatad/tests/fork.rs::a_fork_with_another_agent_is_handed_the_conversation_and_a_summary`. Not built (ROAM batch 3): the CLI and MCP send no agent on a fork (`cli/src/commands/sessions.rs`, `daemon/humatad/src/mcp.rs:717-724`), so only gRPC can ask. A real Claude reading the handover is unproven.

12. **I do:** continue the Claude session in another channel from now.

    **I see:** a new session in the other channel on that channel's repo, with a copy of the conversation under a new id. The source is unchanged.

    **State and events:** the copy goes through the Claude adapter.

    **Status:** Works (real daemon). `daemon/humatad/tests/continue_in.rs::a_claude_session_continues_in_another_channel_from_now`. The real Claude CLI resuming the copy is unproven.

13. **I do:** stop the Codex session, then read the list.

    **I see (expected):** Stopped. **I see (REALFLOW):** Idle.

    **State and events:** a stopped custom-kind session is not mapped to Stopped, so it looks like one that finished its turn.

    **Status:** Likely bug (seen live, B6). Confirm: stop a `copilot` or `codex` kind session and read `humata session list`.

14. **I do:** start the Codex session again.

    **I see (expected):** the same conversation resumes. **I see (REALFLOW):** the resume fails.

    **State and events:** Humata never learns Codex's real session id (the rollout path is dated), so it cannot resume by id.

    **Status:** Likely bug (seen live, B9). Daemon half with a fixture: `daemon/humatad/tests/sessions.rs::a_lost_holder_is_resumed_in_its_conversation_after_a_restart` (Works (real daemon), Claude-shaped agent).

## Branches

### One CLI is not logged in

**When:** the Codex login is missing in the VM's run home.

**I see:** Codex shows its own login prompt in the terminal. The session is Working or Idle, not failed.

**Expected:** the other two agents are unaffected. I log in by hand in that terminal.

**Status:** Code, no test. **Needs the owner's login.**

### The VM runs out of memory with two agents

**When:** Codex and the moved Copilot both run on the 0.75 GiB VM.

**I see:** a CLI is killed or the shell stalls.

**Expected:** Humata shows a session that ended, not a hang, and the other sessions are unaffected.

**Status:** Code, no test. Not a Humata promise; write down what the VM does.

### Two agents ask at the same moment

**When:** the Claude and Copilot cards arrive together.

**I see:** two cards, each naming its own session. An answer to one never reaches the other.

**Status:** Works (real daemon) for the hook side: `daemon/humatad/tests/hooks.rs::the_same_ask_held_twice_is_one_card_and_one_answer_reaches_both` and `::only_the_sessions_own_agent_may_hold_a_hook` (one kind, fixture).

### I answer in the agent's own prompt

**When:** a card is pending and I press Allow in the terminal instead.

**I see:** the card is withdrawn and cannot be answered.

**Status:** Works (real daemon). `daemon/humatad/tests/hooks.rs::a_hook_that_stops_holding_withdraws_its_card_and_it_cant_be_answered`.

### The link drops with a card pending

**When:** the VM link drops while the Codex ask waits.

**I see:** after reconnect the card is still there or withdrawn, never answered twice.

**Status:** Code, no test for the link drop with a real agent; see `real/12-the-link-drops-and-every-agent-resumes.md`. Restart side: `daemon/humatad/tests/inbox.rs::a_restart_keeps_the_inbox_and_the_grants_still_standing` (Works (real daemon), restart only).

### The kinds are missing on the VM

**When:** the VM's daemon has no `codex` kind in its `--agents` file.

**I see:** a move or start to the VM refuses and names the kind.

**Status:** Code, no test. `daemon/humatad/tests/upgrade_kinds.rs` covers kind upgrade on boxes, not a missing kind.

## Expected outcomes

- [ ] Three sessions of three agent kinds run in one channel, two on this Mac and one on the VM, each on its cheapest model, with the owner's CLI homes untouched.
- [ ] The list shows each session's kind, machine and state.
- [ ] Each agent's ask is one Inbox card for its own session, and each answer reaches only that agent in its own shape.
- [ ] Moving one session leaves the other two sessions running and unchanged.
- [ ] The moved session answers from its own conversation on the new machine.
- [ ] A conversation continues in another channel or in another agent from a copy, and the source is unchanged.
- [ ] A stopped session shows Stopped, and a stopped Codex resumes its own conversation.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Start with the three kinds | Fakes only | `agents.rs::codex_and_copilot_hooks_fill_their_placeholders` |
| 2. Claude on its cheapest model | Code, no test | `terminal/provider:claude_resume_test` is terminal only |
| 3. Copilot on Haiku 5.5 | Works (real CLI, manual REALFLOW run) | REALFLOW; B12 fixed (D754) |
| 4. Codex on GPT-6 Luna on the VM | Works (real CLI, manual REALFLOW run) | REALFLOW on this Mac |
| 5. List of mixed kinds | Code, no test | `turns.rs` single kind |
| 6. Three asks at once | Works (real CLI, manual REALFLOW run) | Copilot only; `hooks.rs::each_agent_gets_the_answer_in_its_own_shape` |
| 7. Answers in order | Works (real daemon) | `hooks.rs::an_inbox_approval_reaches_the_waiting_hook`; order untested |
| 8. Shared board | Works (real daemon) | `board_tools.rs`, `boards.rs` (fake agent) |
| 9. Move one while others run | Works (real daemon) | `move_session.rs`, `carry_roam.rs`; B5 live |
| 10. Moved Copilot answers | Likely bug (seen live, B7, B10) | REALFLOW |
| 11. Continue in another agent | Works (real daemon) | `continue_in.rs::a_codex_session_handed_to_another_agent_...` |
| 12. Continue in another channel | Works (real daemon) | `continue_in.rs::a_claude_session_continues_in_another_channel_from_now` |
| 13. Stopped shows Stopped | Likely bug (seen live, B6) | REALFLOW shows Idle |
| 14. Codex resumes | Likely bug (seen live, B9) | REALFLOW |
| B. One CLI not logged in | Code, no test | none |
| B. VM out of memory | Code, no test | none |
| B. Two asks together | Works (real daemon) | `hooks.rs::only_the_sessions_own_agent_may_hold_a_hook` |
| B. Answer in the agent's prompt | Works (real daemon) | `hooks.rs::a_hook_that_stops_holding_withdraws_its_card_and_it_cant_be_answered` |
| B. Link drops with a card | Code, no test | `inbox.rs` restart test only |
| B. Kind missing on the VM | Code, no test | none |

## Gaps and likely bugs

Not built:

- Built-in `codex` and `copilot` kinds (ROAM batch 3, C4, X2). Today each needs an `--agents` file on every machine.
- An agent for a fork on the CLI and MCP surface (ROAM batch 3). Only gRPC can hand a conversation to another agent.
- Humata shows or pins no model for an agent CLI (no milestone). `AgentLaunch` has no model field.
- `ReadConversation` answers not built (`daemon/humatad/src/session_service.rs:2681`), so `humata session read` is empty for every kind.

Likely bugs:

- B6: a stopped custom-kind session shows Idle, not Stopped.
- B5: resume looks for the conversation under `--root` as home.
- B7 and B10: a resume after a stop is treated as an interruption; attach is refused while resuming.
- B9: Humata never learns Codex's real session id, so Codex resume fails.
- Fixed (B12, D754): `humata session type` presses Enter by default.
- Copilot writes `recentModelIds` into the real `~/.copilot/config.json` unless `COPILOT_HOME` is isolated.

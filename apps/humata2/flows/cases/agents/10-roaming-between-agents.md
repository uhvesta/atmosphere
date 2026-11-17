# A conversation roams from Claude to Codex to Copilot, and each agent picks up where the last one left off

Layer: agents. Platforms: macOS and Linux. Web is not covered.

## Starting state

- Channel `Payments` on this Mac, repo `acme/api`.
- The session `Fix login` runs under Claude Code on this Mac. Its clone holds `login.rs` and an untracked `notes.txt`.
- Its conversation holds one line I typed: "remember the retry".
- The daemon knows the Claude kind and a second kind. The product has no built-in Codex or Copilot kind. The second kind comes only from an agents file that the daemon is started with (`--agents`). The tests name that second kind `OTHER` and make it a copy of the fake agent.
- I am in the session's agent tab, and the Fork dialog is closed.

## Steps

1. **I do:** open the session's menu, choose Fork, pick the second agent, and keep "code and context".
   **I see:** the preview says the new agent is handed the transcript and a short summary, and that it does not resume the conversation.
   **State and events:** the preview reports `seeded_with_summary`. The fork is created with the second agent as its kind.
   **Status:** Works (real daemon): `daemon/humatad/tests/fork.rs::a_fork_with_another_agent_is_handed_the_conversation_and_a_summary`. The test checks the preview and the fork's agent. The app's agent picker is not part of this test.

2. **I do:** wait for the fork to start and open its agent tab.
   **I see:** the new agent has read a first message starting "[handover", and a summary line that holds "remember the retry". The fork's folder holds `notes.txt`, and `.humata/handover-<id>.jsonl` holds the handover.
   **State and events:** one "[handover" message is in the fork's conversation. Its text names the handover file.
   **Status:** Works (real daemon): `fork.rs::a_fork_with_another_agent_is_handed_the_conversation_and_a_summary`.

3. **I do:** choose Codex as the second agent in the picker.
   **I see:** if no Codex kind is in the agents file, the fork is refused: the agent is not known.
   **State and events:** no fork is created.
   **Status:** Not built (no milestone of its own; ROAM batch 3 adds the built-in Codex kind). `daemon/sessions/src/agents.rs:139-166` lists only `claude` and `opencode`, and `daemon/humatad/src/serve.rs:421-432` reads kinds only from `--agents`.

4. **I do:** fork with "context only" to the second agent.
   **I see:** the fork's agent reads the handover. The handover file sits in the parent's folder, not in a new one.
   **State and events:** the fork owns no clones. Its handover file is in the parent's folder.
   **Status:** Works (real daemon): `fork.rs::a_context_only_fork_with_another_agent_hands_over_in_its_parents_folder`.

5. **I do:** fork with "code only" to the second agent.
   **I see:** the fork's agent starts with nothing to read. The preview says there is no summary.
   **State and events:** `seeded_with_summary` is false. No handover file exists. The parent's line is not in the fork's conversation.
   **Status:** Works (real daemon): `fork.rs::a_code_only_fork_with_another_agent_starts_it_with_nothing_to_read`.

6. **I do:** from inside `Fix login`, the agent asks for a fork to the second agent with the MCP tool `session_fork`, or with `humata session fork`.
   **I see:** there is no way to name an agent. The fork is made with the parent's own agent.
   **State and events:** the request carries no `agent` field. The fork is not handed to another agent.
   **Status:** Not built (ROAM batch 3, no separate ID). The MCP tool has no `agent` argument (`daemon/mcp/src/tools.rs:1097-1111`). The handler sends `agent: None` (`daemon/humatad/src/mcp.rs:717-724`). The CLI `Fork` has no `--agent` flag (`cli/src/args.rs`, `SessionCommand::Fork`). The same-agent fork by an agent works, and `daemon/humatad/tests/fork_remote.rs::an_agent_forks_itself_onto_another_machine_once_the_user_approves` covers an approved fork onto another machine.

7. **I do:** the second agent's fork (step 2) is opened, and I ask it a question.
   **I see:** the agent answers using what it was handed in the handover message.
   **State and events:** the handover is typed once, and the answer comes from it.
   **Status:** Works (real daemon) for the mechanism, with a fake agent: `fork.rs::a_fork_with_another_agent_is_handed_the_conversation_and_a_summary` (the handover is read by the fake agent). A real Codex CLI reading the handover is not tested.

8. **I do:** fork the Codex fork to Copilot.
   **I see:** the fork is refused, because no Copilot kind is built in.
   **State and events:** no fork is created.
   **Status:** Not built (ROAM batch 3, C4 and X2). `daemon/conversation/src/copilot.rs` refuses every call with `Unsupported`.

9. **I do:** move the Codex fork to `box-1`, with its conversation.
   **I see:** this is not built. The move either refuses or moves the folder without the conversation. This case gives no evidence either way.
   **State and events:** none verified.
   **Status:** Not built (ROAM batch 3). The conversation adapter crate `daemon/conversation` has no caller outside its own tests (the crate's BUILD note says `humata2_sessions`' fork and carry will call it later). No test moves a non-Claude conversation.

10. **I do:** fork `Fix login` with "code and context" to the second agent, then restart the daemon before the agent's handover is typed.
    **I see:** after the restart, the handover is typed once. The fork's conversation holds one "[handover" line.
    **State and events:** the fork is kept, and its handover is typed at the next start.
    **Status:** Works (real daemon): `fork.rs::a_handover_cut_after_the_launch_is_typed_once_after_a_restart`.

11. **I do:** archive the context-only fork (step 4).
    **I see:** the archive ends. The handover file is gone from the parent's folder. The parent's clone and its untracked `notes.txt` stay.
    **State and events:** the fork's handover file is removed. The parent keeps its repo.
    **Status:** Works (real daemon): `fork.rs::freeing_a_context_only_fork_removes_its_handover_file_and_keeps_its_parents_folder`.

12. **I do:** fork a Codex session (if the agents file defines one) with "code and context" to Claude.
    **I see:** the summary is taken from the Codex rollout.
    **State and events:** expected: the transcript is read. Actual: the summary falls back to the terminal's saved output, not the conversation.
    **Status:** Likely bug at `daemon/humatad/src/session_service/start_fresh.rs:891-892`. The summary reader `digest_of` reads only top-level `type` and `message.content` records, which is Claude's JSONL shape. Codex rollout lines hold `type: response_item` with the message under `payload`. `handover_digest` (`start_fresh.rs:765`) has no branch by conversation format, and `handover_from` (`fork.rs:494`) passes the Codex file to it. The bug is latent: no built-in Codex kind exists. To confirm, run `digest_of` on a Codex rollout from `daemon/conversation/testdata/codex/three_turns.jsonl` and expect `None`.

## Branches

### B1. An agent Humata does not know is asked for

**When:** a fork names an agent kind that is not in the agents file.
**I do:** send a fork with `agent` set to `nobody`.
**I see:** the fork is refused, with an invalid field on `agent`. No session is created.
**Expected:** only the original session exists.
**Status:** Works (real daemon): `daemon/humatad/tests/fork.rs::a_fork_with_an_agent_humata_does_not_know_is_refused`.

### B2. Codex conversation layout is discovered

**When:** the CLI's files are read for a Codex conversation.
**I do:** locate a Codex rollout under the Codex home.
**I see:** the rollout is found under its date folder, and no other file is listed.
**Expected:** the Codex adapter reads the layout recorded from codex-cli 0.169.0.
**Status:** Fakes only: `daemon/conversation/tests/codex.rs::locate_finds_the_rollout_under_its_date_folder_in_the_home`. The test reads recorded samples in `daemon/conversation/testdata/codex/`. The real-CLI fixture `codex_layout_test` is a manual Bazel target (`daemon/conversation/BUILD.bazel`), and this case did not run it.

### B3. Copilot's conversation layout is discovered

**When:** a Copilot conversation is read.
**I do:** ask the Copilot adapter for a conversation.
**I see:** every call is refused as unsupported until the Copilot fixtures pass.
**Expected:** per the ROAM plan, Copilot is supported only after its fixtures pass.
**Status:** Not built (ROAM C4 and X2). `daemon/conversation/tests/claude.rs::copilot_is_unsupported_until_its_fixtures_pass` asserts the refusal. Copilot's format is unverified live.

### B4. A fork cut before its agent starts is undone at restart

**When:** the host is killed before the fork's agent launches, and restarts.
**I do:** restart the daemon.
**I see:** the fork is gone. Its folder, its conversation copy and its record are removed. The parent is as it was.
**Status:** Works (real daemon): `daemon/humatad/tests/fork.rs::a_fork_cut_before_its_agent_starts_is_undone_at_restart`.

### B5. A fork cut after its agent started is kept at restart

**When:** the host is killed after the fork's agent started, and restarts.
**I do:** restart the daemon.
**I see:** the fork is kept. Its agent runs, and its conversation has the parent's line.
**Status:** Works (real daemon): `fork.rs::a_fork_cut_after_its_agent_started_is_kept_at_restart`.

### B6. A handed-over fork cut after its conversation is copied is undone

**When:** the host is killed after the conversation is copied to a handed-over fork, and restarts.
**I do:** restart the daemon.
**I see:** the fork is undone. The handover file and the copied conversation are removed.
**Status:** Works (real daemon): `fork.rs::a_handed_over_fork_cut_after_its_conversation_is_copied_is_undone_at_restart`.

### B7. A conversation is recovered exactly after a restart

**When:** the host restarts while an agent is running.
**I do:** restart the daemon, with the agent's holder gone.
**I see:** the agent is resumed in its own conversation, and my earlier line is still there.
**Expected:** the exact conversation is recovered (commit 8672153c, "recover exact agent conversations").
**Status:** Works (real daemon): `daemon/humatad/tests/sessions.rs::a_lost_holder_is_resumed_in_its_conversation_after_a_restart`.

### B8. The conversation file is lost before a restart

**When:** the conversation file is removed, then the host restarts.
**I do:** restart.
**I see:** the session says its resume failed, and gives a line for it.
**Status:** Works (real daemon): `daemon/humatad/tests/sessions.rs::a_reboot_that_lost_the_conversation_says_the_resume_failed`.

### B9. A Codex parent's summary is read from the rollout

**When:** step 12 happens.
**Expected:** the handover carries the transcript.
**Status:** Likely bug at `daemon/humatad/src/session_service/start_fresh.rs:891-892` (see step 12). Latent, since no built-in Codex kind exists.

## Expected outcomes

- [ ] A fork with another agent is handed the parent's conversation and a summary, in a handover file.
- [ ] A context-only fork with another agent hands over in the parent's folder.
- [ ] A code-only fork with another agent starts with no handover and nothing to read.
- [ ] An unknown agent is refused, and no session is made.
- [ ] A handover cut after launch is typed once after a restart.
- [ ] Freeing a context-only fork removes its handover file and keeps the parent's folder.
- [ ] A fork cut before its agent starts is undone, and one cut after it is kept.
- [ ] An agent's own MCP or CLI fork can name another agent. (Not built.)
- [ ] A Codex conversation is read with its own layout. (Fakes only.)
- [ ] A Copilot conversation is refused until its fixtures pass.
- [ ] A Codex parent's summary is read from its transcript. (Likely bug.)

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Fork with another agent, code and context | Works (real daemon) | `daemon/humatad/tests/fork.rs::a_fork_with_another_agent_is_handed_the_conversation_and_a_summary` (the picker is not covered) |
| 2. New agent reads the handover | Works (real daemon) | `fork.rs::a_fork_with_another_agent_is_handed_the_conversation_and_a_summary` |
| 3. Pick Codex | Not built | `daemon/sessions/src/agents.rs:139-166`; `daemon/humatad/src/serve.rs:421-432` |
| 4. Context-only fork in parent's folder | Works (real daemon) | `fork.rs::a_context_only_fork_with_another_agent_hands_over_in_its_parents_folder` |
| 5. Code-only fork starts with nothing | Works (real daemon) | `fork.rs::a_code_only_fork_with_another_agent_starts_it_with_nothing_to_read` |
| 6. Agent forks to another agent (MCP or CLI) | Not built | `daemon/mcp/src/tools.rs:1097-1111`; `daemon/humatad/src/mcp.rs:717-724`; `cli/src/args.rs` (`Fork`) |
| 7. Fork answers from the handover | Works (real daemon, fake agent) | `fork.rs::a_fork_with_another_agent_is_handed_the_conversation_and_a_summary` |
| 8. Codex fork to Copilot | Not built | `daemon/conversation/src/copilot.rs` (all calls Unsupported) |
| 9. Move the Codex fork with its conversation | Not built | `daemon/conversation` has no caller outside its tests |
| 10. Handover typed once after restart | Works (real daemon) | `fork.rs::a_handover_cut_after_the_launch_is_typed_once_after_a_restart` |
| 11. Freeing a context-only fork removes its handover | Works (real daemon) | `fork.rs::freeing_a_context_only_fork_removes_its_handover_file_and_keeps_its_parents_folder` |
| 12. Codex parent's summary from its transcript | Likely bug | `start_fresh.rs:891-892`; `start_fresh.rs:765`; `fork.rs:494` |
| B1. Unknown agent refused | Works (real daemon) | `fork.rs::a_fork_with_an_agent_humata_does_not_know_is_refused` |
| B2. Codex layout discovered | Fakes only | `daemon/conversation/tests/codex.rs::locate_finds_the_rollout_under_its_date_folder_in_the_home`; manual `codex_layout_test` not run |
| B3. Copilot layout | Not built | ROAM C4, X2; `daemon/conversation/tests/claude.rs::copilot_is_unsupported_until_its_fixtures_pass` |
| B4. Fork cut before agent starts | Works (real daemon) | `fork.rs::a_fork_cut_before_its_agent_starts_is_undone_at_restart` |
| B5. Fork cut after agent started | Works (real daemon) | `fork.rs::a_fork_cut_after_its_agent_started_is_kept_at_restart` |
| B6. Handed-over fork cut after copy | Works (real daemon) | `fork.rs::a_handed_over_fork_cut_after_its_conversation_is_copied_is_undone_at_restart` |
| B7. Conversation recovered exactly | Works (real daemon) | `daemon/humatad/tests/sessions.rs::a_lost_holder_is_resumed_in_its_conversation_after_a_restart` |
| B8. Conversation lost before restart | Works (real daemon) | `sessions.rs::a_reboot_that_lost_the_conversation_says_the_resume_failed` |
| B9. Codex parent summary | Likely bug | Same as step 12 |

## Gaps and likely bugs

- Not built: an agent cannot choose another agent for its own fork. The MCP `session_fork` tool and `humata session fork` have no agent argument (`daemon/mcp/src/tools.rs:1097-1111`, `daemon/humatad/src/mcp.rs:717-724`).
- Not built: built-in Codex and Copilot kinds. Only `claude` and `opencode` are built in (`daemon/sessions/src/agents.rs:139-166`). ROAM batch 3 adds them.
- Not built: the conversation adapters are not called from fork or carry. `daemon/conversation` is used only by its own tests (ROAM batch 3, the SEAM on `carry.rs`).
- Not built: Copilot's conversation adapter (ROAM C4, X2). `copilot.rs` refuses every call.
- Likely bug: the handover summary reader reads Claude's JSONL shape for every conversation file (`start_fresh.rs:891-892`), so a Codex parent's summary falls back to scrollback. Latent until a Codex kind exists.
- Test gap: the fork tests use a fake agent named `OTHER`, copied from the test agent. They prove the handover mechanism, not a real Codex or Copilot CLI reading it.
- Test gap: the ROAM fixtures `codex_layout_test`, `claude_layout_test` and `copilot_layout_test` are manual real-CLI targets. They are not in the normal gate.

## Corrections to the brief

- `providers.rs` is not in `daemon/humatad/src`. The provider code is `daemon/humatad/src/provider_service.rs`.
- ROAM is not "workers running" in the code. `workflow/DAG.md` lists it as planning, and the X1 fixture tasks in `workflow/milestones/ROAM.md` are unchecked. The fixture scripts and the recorded Codex testdata do exist.
- `daemon/humatad/tests/fork.rs` has 18 tests. Its another-agent tests are four: `a_fork_with_another_agent_is_handed_the_conversation_and_a_summary`, `a_context_only_fork_with_another_agent_hands_over_in_its_parents_folder`, `a_code_only_fork_with_another_agent_starts_it_with_nothing_to_read` and `a_fork_with_an_agent_humata_does_not_know_is_refused`.
- The commit "recover exact agent conversations" is cited from the repo's log. This case did not read its diff (no git), so the link to B7 and B8 is by test name only.

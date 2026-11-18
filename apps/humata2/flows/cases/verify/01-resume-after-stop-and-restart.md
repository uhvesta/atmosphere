# Each agent kind resumes its exact conversation after a stop and after a daemon restart, and `humata session read` shows its entries

Layer: agents (verify, headless). Platforms: macOS (this Mac). The VM is not used here.

## Starting state

- A throwaway `humatad2` runs on this Mac. It has its own `--root`, `--state`, socket and `HOME`. Its agents file lists Claude, Codex and Copilot, the three built-in kinds, each started by a test agent script in a headless run.
- Channel `acme/api` exists, with repo `acme/api` cloned under the daemon's root. The Inbox is empty.
- Three sessions exist, one per kind: "Fix login" on Claude, "Fix login (codex)" on Codex and "Fix login (copilot)" on Copilot. None is stopped yet.
- Each session has two finished turns. The prompts are the same for every kind: "Remember the word amber" and "Now remember cobalt".
- Isolation: `HOME`, `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `COPILOT_HOME` and `COPILOT_CACHE_HOME` each point into the run's temp folder. The owner's real `~/.claude`, `~/.codex`, `~/.copilot` and login are never read or written.
- The person is holding nothing. No window is open on the screen.

## Reproduce headless

- Test targets, run with `bazel test` (never cargo):
  - `//apps/humata2/daemon/humatad:sessions_test` with `--test_filter=stopping_ends`, `a_lost_holder`, `killing_the_daemon` and `a_reboot`. These are the stop, restart and lost-holder tests, for Claude's fixture.
  - `//apps/humata2/daemon/humatad:continue_in_test` with `lists_its_turns`. It starts Codex and Copilot kinds through the real daemon.
  - `//apps/humata2/daemon/humatad:history_test` with `a_read_returns_the_turns_asked_for_in_order` and `a_restart_keeps_what_was_mirrored`.
  - `//apps/humata2/daemon/humatad:codex_trust_test`.
  - `//apps/humata2/daemon/sessions:sessions_unit_test` with the Codex binding tests in `agents.rs` (`a_codex_conversation_is_found_by_its_folder_once_written_and_resumed_by_its_id`, `a_saved_codex_session_learns_its_conversation_once_written`).
  - `//apps/humata2/daemon/mirror:other_agents_test` for Codex and Copilot lines in `session read`.
- Headless run by hand: start `humatad2 serve` with `--socket`, `--state`, `--root`, `--holder`, `--agents`, `--channels` and `--default-agent`, the same flags as `World::args` in `daemon/humatad/tests/common/mod.rs`. Then run the CLI with `HUMATA_SOCKET=<temp>/humatad.sock` and the temp `HOME`. Capture each command's stdout, JSON and exit code.
- Evidence to capture: the `humata session read <id> --json` output for each kind before the stop and after the restart; the SHA-256 of each conversation file before the stop and after the resume; the session's `status` at each step; the daemon's start line (`not_brought_back`).
- Cannot do headless: a real login to Claude, Codex or Copilot; a real model answer to "what did I say first?"; the owner's real app restart. Nearest headless equivalent: the test agent script writes the transcript and the test checks the file and the turns. For the real CLI's own resume, run `//apps/humata2/terminal/provider:claude_resume_test`, `codex_resume_test` and `copilot_resume_test` (manual, macOS, loopback model, isolated HOME). Those prove the CLI resumes, not Humata's path.

## Steps

1. **I do:** start one session per kind on `acme/api` with `humata session start --agent claude|codex|copilot --repo acme/api --title "..."`, then send each its two prompts.

   **I see:** three sessions reach Working and then Idle. Each prints its id and agent.

   **State and events:** each session's agent record names a conversation id and file. The turns come from the agent's own hooks and the transcript.

   **Status:** Works (real daemon). `daemon/humatad/tests/continue_in.rs::a_codex_session_lists_its_turns` and `::a_copilot_session_lists_its_turns` start those kinds through the real daemon, with a fake agent script. `daemon/humatad/tests/sessions.rs` does the same for Claude. The real CLIs starting under Humata are not run by any test.

2. **I do:** read each session's agent record from the temp `--state` store (the CLI has no `show` command), or read the conversation file path the store names.

   **I see:** Claude's id is pinned at start. Copilot's id is pinned with `--session-id`. Codex has a placeholder id until its CLI writes its rollout, and then the id is the one in the rollout's first line.

   **State and events:** the launch's conversation id and file are saved. Codex's file changes from the placeholder to the real rollout (D670).

   **Status:** Fakes only. The Codex binding is covered only by the unit tests in `daemon/sessions/src/agents.rs`. The `continue_in.rs` fixture writes its Codex rollout at the placeholder path, so the binding never runs in the daemon test.

3. **I do:** run `humata session read <session>` for each session after its two prompts.

   **I see:** the two prompts and their replies, in order, each with its role. The output is the same whether or not the CLI has been started again.

   **State and events:** the entries come from the history mirror, not from the agent's file. `session read` calls `ReadTurns`. `humata conversation <session>` calls `ReadConversation`, which reads the file. Both return the same entries.

   **Status:** Fakes only. For Claude, `daemon/humatad/tests/history.rs::a_read_returns_the_turns_asked_for_in_order` is Works (real daemon) with a Claude JSONL ledger. For Codex and Copilot, the only proof is `daemon/mirror/tests/other_agents.rs` (`a_codex_rollout_gives_prompts_and_replies_without_the_injected_messages`, `a_copilot_events_file_gives_prompts_replies_and_tools`) and the unit test in `daemon/humatad/src/history_service.rs` (`catching_up_a_codex_session_learns_the_conversation_its_cli_wrote`). No daemon test reads a Codex or Copilot session through `session read`.

4. **I do:** run `humata session stop <session>` on the Claude session first, then on the Codex and the Copilot session.

   **I see:** the command returns. The holder's process ends, and the session no longer has a terminal.

   **State and events:** the agent is saved as stopped, and the session's status reads Exited (the API's name for a stopped agent with no holder).

   **Status:** Likely bug. `daemon/humatad/tests/sessions.rs:367` and `:370` assert that the stopped session reads `Idle`. The code gives `Exited`: `daemon/humatad/src/session_service.rs:1424` maps `AgentState::Stopped` to `Exited`, and `stop_session` at about lines 930–975 sets the state before it calls `show(&loaded)`. Either the test or the code is wrong. Confirm by running `//apps/humata2/daemon/humatad:sessions_test --test_filter=stopping_ends`.

5. **I do:** after the stop, hash each session's conversation file.

   **I see:** the hash is the same as before the stop. Nothing was appended.

   **State and events:** a stop ends the holder and writes the agent record only. It does not touch the conversation file.

   **Status:** Code, no test. The stop path does not write to the file, but no test checks the hash across a stop for any kind.

6. **I do:** run `humata session resume <session>` for each session.

   **I see:** the session goes Starting and then Working. The agent runs `resume <id>` (Codex), `--resume <id>` (Claude) or `--session-id <id>` (Copilot), with the same id as before the stop.

   **State and events:** the resumed agent's conversation id equals the id from step 2. The file is not replaced.

   **Status:** Code, no test. For Claude, `daemon/humatad/tests/sessions.rs::stopping_ends_the_agent_and_resuming_brings_it_back_in_its_conversation` is Works (real daemon) with the Claude-shaped fixture. For Codex and Copilot, no daemon test resumes after a stop. The real CLI's resume with `resume <id>` is covered by the manual `codex_resume_test` only.

7. **I do:** resume each session with the person's ask (`humata session resume`), not after a crash.

   **I see:** the agent comes up with no note saying it was interrupted. It starts no other conversation.

   **State and events:** no Interruption is recorded for the resume. A revival after a crash does record one (D671).

   **Status:** Code, no test. `daemon/humatad/src/session_service.rs` (about line 1997) sets `asked` for a stopped or exited agent, but no test checks that the note is absent.

8. **I do:** send each resumed session a third prompt, "What word did I ask you to remember first?", then run `humata session read` and `humata conversation`.

   **I see:** the new turn is at the end of the list. Earlier turns are unchanged.

   **State and events:** the new turn is appended to the same conversation file under the same id. Neither command creates a new session.

   **Status:** Code, no test. No test checks the turn list after a resume for Codex or Copilot. `humata conversation` (`ReadConversation`) has no daemon or CLI test at all. Its only proof is the unit tests in `daemon/humatad/src/session_service/conversation_read.rs` (`a_codex_rollout_gives_its_prompts_and_replies_in_order`, `a_copilot_session_gives_its_prompts_and_replies_in_order`), which read fixture text.

9. **I do:** kill the daemon with `kill` and leave the holder processes alive. Start `humatad2` again with the same `--state`, `--root` and socket.

   **I see:** the start line says nothing was left behind (`not_brought_back` is empty). Each session still reads Working or Idle, with the same conversation id. `session read` still shows the entries.

   **State and events:** the holders are reattached. The conversation id and file do not change.

   **Status:** Code, no test for Codex and Copilot. For Claude, `daemon/humatad/tests/sessions.rs::killing_the_daemon_leaves_the_holder_and_the_conversation_alone` is Works (real daemon), with the Claude-shaped fixture.

## Branches

### The daemon and its holders are killed (a reboot), and the session comes back in its conversation

**When:** in step 9, the holder processes are also killed before the daemon starts again.

**I see:** the start line names the sessions it brought back. Each session resumes with its conversation id, and the turns are still there.

**Expected:** each kind resumes its own conversation. Nothing is started in its place.

**Status:** Code, no test for Codex and Copilot. For Claude, `daemon/humatad/tests/sessions.rs::a_lost_holder_is_resumed_in_its_conversation_after_a_restart` and `::a_reboot_brings_back_the_session_and_a_shell` are Works (real daemon).

### The conversation file is gone at the restart

**When:** the conversation file is deleted between the kill and the start.

**I see:** the start line says the resume failed and gives the reason. The session reads Resume failed. No new agent starts in its place.

**Expected:** the session shows Resume failed and keeps its conversation id. Nothing is overwritten.

**Status:** Works (real daemon) for Claude: `daemon/humatad/tests/sessions.rs::a_reboot_that_lost_the_conversation_says_the_resume_failed`. Code, no test for Codex and Copilot.

### A Codex session restarts before its CLI wrote a rollout

**When:** a Codex session is started, the daemon is killed before the CLI writes its rollout, then the daemon starts again.

**I see:** the start learns the rollout if the CLI has written one since. The session resumes under the id from the rollout. If the CLI has not written one, the session has no conversation yet.

**Expected:** the restart learns the conversation (D670), and the session resumes it by id. An unbound session never resumes the placeholder.

**Status:** Code, no test. The restart calls `learn_conversations` (`daemon/humatad/src/restart.rs:318` and `:372`). The learning itself is unit-tested in `daemon/sessions/src/agents.rs`. No test restarts a Codex session.

### Two Codex sessions share a folder, and a rollout from outside Humata is bound

**When:** a Codex session is started in a folder, and before its CLI writes its rollout the person runs `codex` by hand in the same folder.

**I see:** the Humata session may bind to the rollout the person started, not its own.

**Expected:** the session binds only to a rollout its own launch wrote.

**Status:** Likely bug (provisional under D670). `daemon/sessions/src/agents.rs` `found_conversation` (about lines 882–921) takes the newest rollout whose `session_meta` cwd is the session's folder and whose id no other session holds. It does not check that the rollout appeared after this launch. Confirm by starting a Codex session, starting `codex` in the same folder before the first rollout, and reading which id the session holds.

### A Copilot session is stopped before its first prompt

**When:** a Copilot session is started, given no prompt, stopped and resumed.

**I see:** Copilot writes `events.jsonl` only once it takes a first prompt. The resume passes `--session-id` for a session with no saved events.

**Expected:** the resume either starts the session fresh with its id, or says that there is nothing to resume.

**Status:** Code, no test. Copilot's behaviour with `--session-id` on a session with no events file is not proven by any test or fixture.

## Expected outcomes

- [ ] Claude, Codex and Copilot each start, take two prompts and stop under a throwaway `humatad2`.
- [ ] After a stop, the session's conversation id and file are as before the stop, and the file's bytes are unchanged.
- [ ] After a resume the session runs under its own conversation id, and the next turn is appended to that file.
- [ ] After a daemon restart with holders alive, each session keeps its id and its turns.
- [ ] After a reboot that kills the holders, each session resumes in its conversation, or shows Resume failed when its file is gone.
- [ ] `humata session read` lists the prompts and replies of each kind, in order.
- [ ] A stopped session reads as stopped. (Blocked: the test and the code disagree; see the Likely bug.)
- [ ] No resume writes into the owner's real CLI homes.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Start one session per kind, two prompts | Works (real daemon) | `continue_in.rs::a_codex_session_lists_its_turns`, `::a_copilot_session_lists_its_turns`; `sessions.rs` fixture |
| 2. Launch names its conversation | Fakes only | `agents.rs` unit tests; `continue_in.rs` fixture writes the placeholder path |
| 3. `session read` lists the entries | Fakes only | `history.rs::a_read_returns_the_turns_asked_for_in_order` (Claude, Works); `mirror/tests/other_agents.rs` (Codex, Copilot) |
| 4. Stop ends the holder and reads stopped | Likely bug | `sessions.rs:367,370` expects Idle; `session_service.rs:1424` gives Exited |
| 5. Stop leaves the file unchanged | Code, no test | `stop_running.rs`; no hash test |
| 6. Resume in the same conversation | Code, no test | Claude: `sessions.rs::stopping_ends_the_agent_and_resuming_brings_it_back_in_its_conversation` (Works) |
| 7. Resume is not told it was interrupted | Code, no test | `session_service.rs` about line 1997 |
| 8. Next turn appended; read and conversation show it | Code, no test | `conversation_read.rs` unit tests only |
| 9. Daemon restart, holders alive | Code, no test | Claude: `sessions.rs::killing_the_daemon_leaves_the_holder_and_the_conversation_alone` (Works) |
| B. Reboot with holders killed | Code, no test | Claude: `sessions.rs::a_lost_holder_is_resumed_in_its_conversation_after_a_restart` (Works) |
| B. Conversation file gone | Code, no test | Claude: `sessions.rs::a_reboot_that_lost_the_conversation_says_the_resume_failed` (Works) |
| B. Codex restart before its rollout | Code, no test | `restart.rs:318,372`; `agents.rs` unit tests |
| B. Codex binds a rollout from outside Humata | Likely bug | `agents.rs` `found_conversation` about lines 882–921 |
| B. Copilot stopped before first prompt | Code, no test | none |

## Gaps and likely bugs

Not built: none found for the steps above. Every kind has a resume path and a read path in code. The gaps are proof gaps, listed as Code, no test.

Likely bugs:

- `daemon/humatad/tests/sessions.rs:367,370` expects a stopped session to read `Idle`. `daemon/humatad/src/session_service.rs:1424` maps a stopped agent to `Exited`, and `stop_session` returns the state it has just set. Confirm with `//apps/humata2/daemon/humatad:sessions_test --test_filter=stopping_ends`.
- `daemon/sessions/src/agents.rs` `found_conversation` (about lines 882–921) binds the newest unclaimed rollout in the session's folder without checking that this launch wrote it. Provisional under D670. Confirm with the steps in the branch above.
- `daemon/humatad/src/session_service/conversation_read.rs` `read_file` (about lines 96–140) reads and parses the whole transcript for every page, then slices it. A long conversation costs the same on every page. This is a performance problem, not a wrong answer. Confirm with a transcript of tens of thousands of lines and time each page.

Not proven (Code, no test):

- `humata conversation` (`ReadConversation`) has no daemon or CLI test.
- The AGENTKINDS exit criterion for resume after a stop and a restart is not proven for Codex or Copilot by any daemon test.
- Whether the real `codex resume <id>` and `copilot --session-id <id>` accept a conversation Humata bound after a restart is not proven live.

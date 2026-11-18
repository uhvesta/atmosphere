# Each agent kind forks on this Mac with code and context, code only, context only, and from an earlier turn, and the fork resumes while the parent stays as it was

Layer: agents (verify, headless). Platforms: macOS (this Mac). The VM parts of the real/14 case are not repeated; the fake-box daemon tests appear in a branch.

## Starting state

- A throwaway `humatad2` runs on this Mac, with its own `--root`, `--state`, socket and `HOME`. Its agents file lists Claude, Codex and Copilot, each run by a test agent script in a headless run.
- Channel `acme/api` exists with repo `acme/api` cloned under the daemon's root. The Inbox is empty. No window is open on the screen.
- Three parent sessions run, one per kind, all named "Write path". Each has three finished turns: "In the api folder, find the flaky test", "Make it deterministic", "Now delete the old sleep". Each clone has one commit, one uncommitted change and one untracked file.
- Before the first fork, each parent's conversation files are hashed: Claude's `<CLAUDE_CONFIG_DIR>/projects/<dashed folder>/<id>.jsonl`, Codex's rollout under `<CODEX_HOME>/sessions/YYYY/MM/DD/`, Copilot's `<COPILOT_HOME>/session-state/<id>/`.
- Isolation: `HOME`, `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `COPILOT_HOME` and `COPILOT_CACHE_HOME` point into the run's temp folder. The owner's real CLI homes and logins are never read or written.

## Reproduce headless

- Daemon and CLI tests, run with `bazel test` (never cargo):
  - `//apps/humata2/daemon/humatad:fork_test`, `:context_fork_test`, `:continue_in_test` and `:fork_remote_test` (the last for fake boxes, not the VM).
  - `//apps/humata2/daemon/conversation:conversation_unit_test`, `codex_test`, `copilot_test` and `claude_test`: the adapter cuts, on fixtures.
  - `//apps/humata2/cli:sessions_test` with `--test_filter=fork`.
  - `//apps/humata2/desktop:live_flows_test` with `--test_arg=fork_kinds`: the Fork dialog, for a Claude-shaped fake agent.
- The half a daemon test can't show, the real CLI resuming a fork: `//apps/humata2/daemon/conversation:claude_fork_resume_test`, `codex_fork_resume_test` and `copilot_fork_resume_test`. These are manual and macOS-only, with `--test_env=TERMINAL_CODEX_BIN=<absolute path>` (and the other two). Each runs Humata's adapter fork (`fixtures/fork_helper.rs`) into a new id and folder, has the installed CLI resume it with its own command, and asks for a word through a loopback model. The test writes `<cli>_fork_resume.json`.
- Headless by hand: `humata session fork <parent> --kind code-and-context|code|context [--agent <kind>] [--before-turn N] --title "Fork A"`, with `HUMATA_SOCKET` set to the temp socket. Then `humata session read <fork> --json`.
- Evidence to capture: the fork's JSON (id, branch, `fork_origin`, `before_turn`); each parent file's hash before and after each fork; `session read` for each fork and parent; the `<cli>_fork_resume.json`.
- Cannot do headless: a real model login, a real model's answer to "what were my first prompts?", and the Fork dialog on the screen. Nearest: the manual `fork_resume` tests and the `live_flows` dialog tests.

## Steps

1. **I do:** check each parent has three turns with `humata session read <parent> --json`, and hash its conversation files.  
   **I see:** three turns for each parent, with the prompts above.  
   **State and events:** each parent's agent record names its conversation file. Turns come from the mirror.  
   **Status:** Works (real daemon). `daemon/humatad/tests/continue_in.rs::a_codex_session_lists_its_turns` and `::a_copilot_session_lists_its_turns` (fixture transcripts, fake agent). Claude: `context_fork.rs::the_turns_are_the_prompts_of_the_conversation`.

2. **I do:** fork each parent on this Mac with code and context: `humata session fork <parent> --kind code-and-context --title "Fork A"`.  
   **I see:** a new session with its own id, branch and clone. The clone holds the parent's commit, uncommitted change and untracked file. The fork's conversation id differs from the parent's.  
   **State and events:** the fork's origin names the parent and the kind. It carries context and runs the same agent, so it starts by resuming its copied conversation.  
   **Status:** Code, no test for Codex and Copilot. Claude: `daemon/humatad/tests/fork.rs::code_and_context_copies_the_work_and_the_conversation` is Works (real daemon), with a Claude-shaped fixture and a fake agent.

3. **I do:** check the copied conversation of each kind.  
   **I see:** the fork's conversation holds the parent's records under the fork's id, and the file and its lines name the new id.  
   **State and events:** Humata copies the conversation through the kind's adapter (`fork_conversation` in `daemon/sessions/src/fork.rs`). It does not script the CLI to fork.  
   **Status:** Fakes only. Adapter tests on fixtures: `daemon/conversation/tests/claude.rs::a_fork_from_now_holds_the_whole_current_path`, `codex.rs::a_fork_from_now_holds_the_whole_rollout` and `::a_fork_is_named_by_its_new_id_in_its_file_name_and_its_lines`, `copilot.rs::a_fork_from_now_holds_every_line_of_a_finished_conversation`. No daemon test checks a Codex or Copilot copy after a fork.

4. **I do:** resume the fork in its CLI, ask for the first three prompts, then send a fourth.  
   **I see:** the CLI names all three prompts, then answers the fourth.  
   **State and events:** the fork resumes by its new id, and the CLI loads the copied history.  
   **Status:** Fakes only. The manual `claude_fork_resume_test`, `codex_fork_resume_test` and `copilot_fork_resume_test` run Humata's adapter fork and the installed CLI's resume with a loopback model. They do not run `humatad2` or the session start.

5. **I do:** after each fork, re-hash the parent's files, then resume the parent and ask for its three prompts.  
   **I see:** every hash is unchanged. The parent still resumes and still has three turns.  
   **State and events:** a fork reads the parent's files and writes only new ones.  
   **Status:** Code, no test. Before-turn forks of Codex and Copilot check that the source file's text is unchanged (`daemon/humatad/tests/continue_in.rs::forks_before_turn_2`, Works, real daemon), and Claude's parent is checked by `context_fork.rs` `assert_parent_untouched`. No test resumes the parent in its real CLI after a fork.

6. **I do:** fork each parent with code only: `--kind code-only`.  
   **I see:** a fork with the parent's work on a new branch and a fresh conversation. The CLI starts with no memory of the parent's prompts. No conversation file is copied.  
   **State and events:** the fork's origin is CodeOnly, and its agent starts under a new conversation id.  
   **Status:** Code, no test for Codex and Copilot. Claude: `daemon/humatad/tests/fork.rs::code_only_copies_the_work_into_a_fresh_conversation` is Works (real daemon).

7. **I do:** fork each parent with context only: `--kind context-only`.  
   **I see:** the fork carries the conversation and works in the parent's clone, on the parent's branch. It makes no new branch.  
   **State and events:** the fork's origin is ContextOnly. Two agents now work in one folder, which is what this kind means.  
   **Status:** Code, no test for Codex and Copilot. Claude: `daemon/humatad/tests/context_fork.rs::a_context_only_fork_carries_the_conversation_and_works_in_the_parents_code` is Works (real daemon).

8. **I do:** fork each parent from before turn 2: `--before-turn 2`, code and context.  
   **I see:** a fork whose conversation holds turn 1 only. The code is the parent's as it is now.  
   **State and events:** the cut is made in the kind's own format: Claude's records before the second prompt, Codex's lines before turn 2's start, Copilot's lines before its second `user.message`. The fork's origin has `before_turn` 2.  
   **Status:** Works (real daemon). Claude: `context_fork.rs::a_fork_before_a_turn_carries_the_conversation_up_to_it_and_the_code_as_now`. Codex: `continue_in.rs::a_codex_fork_before_turn_2_resumes_with_turn_1_only`. Copilot: `::a_copilot_fork_before_turn_2_resumes_with_turn_1_only`.

9. **I do:** resume the before-turn fork in its real CLI and ask for its prompts.  
   **I see:** the CLI names "In the api folder, find the flaky test" and not the other two.  
   **State and events:** the fork's file was cut by Humata, then loaded by the CLI.  
   **Status:** Fakes only. The manual `claude_roam_fixture`, `codex_roam_fixture` and `copilot_roam_fixture` in `daemon/conversation/BUILD.bazel` hand-cut a before-turn-2 fork and resumed it with the installed CLI. They do not use Humata's cut.

10. **I do:** make forks with each flag from the CLI (`--kind`, `--agent`, `--before-turn`), and through MCP's `session_fork`. Try `--before-turn 1` both ways.  
    **I see:** each flag reaches the daemon. `--before-turn 1` exits with a usage error (CLI) or a refusal that names the valid range (MCP), and nothing is forked.  
    **State and events:** the request carries the kind, the agent and the turn.  
    **Status:** Fakes only. `cli/tests/sessions.rs::fork_copy_takes_each_mode`, `::fork_sends_the_agent_and_the_turn` and `::fork_before_turn_one_is_refused_and_nothing_forks` check the CLI's requests. The `fork_choices` unit tests in `daemon/humatad/src/mcp.rs` (about lines 2225–2260) check MCP's arguments. No test sends MCP calls to a running daemon.

11. **I do:** open the Fork dialog on each parent and pick Code only, Context only, and Fork from an earlier turn. Then pick a fork the daemon refuses.  
    **I see:** each kind makes its fork. A refused fork keeps the dialog for another try.  
    **State and events:** the dialog sends the same request the CLI sends.  
    **Status:** Code, no test for Codex and Copilot. `desktop/tests/live_flows/fork_kinds.rs` (`a_code_only_fork_copies_the_files_into_a_fresh_conversation`, `a_context_only_fork_carries_the_conversation_and_shares_the_parents_files`, `a_fork_from_before_a_turn_carries_the_conversation_up_to_it`, `a_refused_fork_keeps_the_dialog_and_a_retry_makes_the_child`) is Works (real daemon), but it declares the agent Claude-shaped and never picks Codex or Copilot.

## Branches

### Copilot refuses a conversation with hook and permission records

**When:** the parent holds `hook.start`, `hook.end`, `permission.requested` or `permission.completed` records, as a real Copilot CLI writes them.

**I see:** before D673 the fork was refused with "can't read this Copilot conversation". Now the fork goes ahead.

**Expected:** the fork is made and keeps the records. Any field that names a path names the new folder.

**Status:** Fakes only. `daemon/conversation/tests/copilot.rs::a_conversation_with_a_hook_and_a_permission_prompt_is_known_and_keeps_its_turns` and `::a_fork_of_a_conversation_with_a_hook_and_a_permission_prompt_names_only_the_forks_id_and_folder` use records written from the CLI's event schema (D673, marked X2). No real capture has been run.

### A Copilot conversation holds a record Humata doesn't know, or came from a CLI older than Humata knows

**When:** the parent holds a record kind the adapter does not know, or its Copilot CLI is older than the version Humata knows.

**I see:** the fork is refused up front, and the message names the record or the version. Nothing changes.

**Expected:** no fork, and the parent keeps its files.

**Status:** Works (real daemon). `daemon/humatad/tests/continue_in.rs::a_fork_of_a_copilot_conversation_with_a_record_humata_doesnt_know_is_refused_up_front` and `::a_fork_of_a_copilot_session_launched_by_a_cli_older_than_humata_knows_is_refused_up_front`.

### A fork before a turn that is not there

**When:** `--before-turn` names turn 4 of a three-turn conversation.

**I see:** the fork is refused as an invalid argument. Nothing is made.

**Expected:** no fork, and the parent is unchanged.

**Status:** Works (real daemon). `daemon/humatad/tests/context_fork.rs::a_fork_before_a_turn_that_is_not_there_is_refused`.

### The Codex fork is resumed where the CLI has no thread row for it

**When:** a Codex fork is resumed by `codex resume <fork id>`, and the CLI's `state_5.sqlite` has no `threads` row for the fork. Humata's fork writes none.

**I see:** the CLI may refuse the resume, or may resume it.

**Expected:** the CLI resumes the fork by its id, with the parent's history.

**Status:** Code, no test. `daemon/conversation/src/codex.rs` (module doc, about lines 205–220) says that `codex exec resume <fork id>` on a rollout with no `threads` row is not yet proven against the real CLI. `daemon/conversation/tests/codex.rs::forking_leaves_the_parent_and_the_clis_state_as_they_were` checks that no state is written.

### A fork is cut short by a restart

**When:** the host restarts part way through a fork.

**I see:** a fork whose agent had not started is undone. A fork whose agent had started is kept.

**Expected:** no half-made fork, and the parent is unchanged.

**Status:** Works (real daemon) for Claude: `daemon/humatad/tests/fork.rs::a_fork_cut_before_its_agent_starts_is_undone_at_restart` and `::a_fork_cut_after_its_agent_started_is_kept_at_restart`. Code, no test for Codex and Copilot.

### The fork goes to a fake box

**When:** the fork's machine is a fake box in the multi-machine rig, not this Mac.

**I see:** code and context, and code only, land on the box. A context-only fork is refused, and nothing is made.

**Expected:** as the fork tests for this Mac, on the box.

**Status:** Works (real daemon), fake `ssh` and fake box, not the VM. Claude: `daemon/humatad/tests/fork_remote.rs::code_and_context_fork_onto_box_1_...`, `::code_only_fork_from_box_1_onto_box_2_...`, `::a_context_only_fork_onto_another_machine_is_refused`. Codex and Copilot before turn 2: `::a_codex_fork_from_box_1_onto_box_2_before_turn_2_carries_turn_1_only`, `::a_copilot_fork_from_box_1_onto_box_2_before_turn_2_carries_turn_1_only`.

### The MCP continue-in tool with an invalid `before_turn`

**When:** an agent calls `session_continue_in_channel` with a `before_turn` that is not a whole number, is 0 or 1, or does not fit a `u32`.

**I see:** the whole conversation is copied, as if no turn was asked for.

**Expected:** the call is refused and names the valid range, as `session_fork` does.

**Status:** Likely bug. `daemon/humatad/src/mcp.rs:751` reads `before_turn` with `as_u64`, then `u32::try_from(..).ok()`, then `unwrap_or(0)`, so any invalid value becomes 0, which means the whole conversation. The fork tool at about lines 1318–1334 validates the same field and refuses it. Confirm by calling `session_continue_in_channel` with `before_turn: 1` and checking that the whole conversation is carried.

## Expected outcomes

- [ ] Each of Claude, Codex and Copilot forks on this Mac with code and context, code only, and context only.
- [ ] Each kind's fork from before turn 2 holds turn 1 only, and the parent's conversation files are unchanged.
- [ ] Humata copies the conversation through the kind's adapter. The CLIs are not scripted to fork.
- [ ] A code-and-context fork, resumed in the real CLI, remembers every turn of the parent. (Unproven here for Codex and Copilot.)
- [ ] A code-only fork starts a fresh conversation with the parent's work.
- [ ] A context-only fork works in the parent's clone and on the parent's branch.
- [ ] A fork before turn 1, or past the last turn, is refused, and nothing is made.
- [ ] The parent's conversation files are byte-identical after each fork, and the parent still resumes.
- [ ] No fork writes into the owner's real CLI homes.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Three turns per parent | Works (real daemon) | `continue_in.rs::a_codex_session_lists_its_turns`, `::a_copilot_session_lists_its_turns` |
| 2. Fork, code and context | Code, no test | Claude: `fork.rs::code_and_context_copies_the_work_and_the_conversation` |
| 3. Copy goes through the adapter | Fakes only | `daemon/conversation/tests/{claude,codex,copilot}.rs` |
| 4. Fork resumes in the real CLI | Fakes only | manual `*_fork_resume_test` (real CLI, loopback) |
| 5. Parent unchanged and resumes | Code, no test | `continue_in.rs::forks_before_turn_2` (before-turn only) |
| 6. Code only | Code, no test | Claude: `fork.rs::code_only_copies_the_work_into_a_fresh_conversation` |
| 7. Context only | Code, no test | Claude: `context_fork.rs::a_context_only_fork_carries_...` |
| 8. Fork from before turn 2 | Works (real daemon) | `context_fork.rs::a_fork_before_a_turn_carries_...`; `continue_in.rs` Codex and Copilot tests |
| 9. Real CLI resumes a before-turn cut | Fakes only | manual `*_roam_fixture` (hand-cut, not Humata's) |
| 10. CLI and MCP flags reach the daemon | Fakes only | `cli/tests/sessions.rs` fork tests; `mcp.rs` `fork_choices` unit tests |
| 11. Fork dialog kinds | Code, no test | `desktop/tests/live_flows/fork_kinds.rs` (Claude-shaped, Works) |
| B. Copilot hook and permission records | Fakes only | `copilot.rs::a_conversation_with_a_hook_and_a_permission_prompt_...` |
| B. Copilot unknown record or old CLI | Works (real daemon) | `continue_in.rs` refusal tests |
| B. Turn not there | Works (real daemon) | `context_fork.rs::a_fork_before_a_turn_that_is_not_there_is_refused` |
| B. Codex fork with no thread row | Code, no test | `codex.rs` module doc, "Not yet proven" |
| B. Fork cut by a restart | Works (real daemon) for Claude | `fork.rs::a_fork_cut_before_its_agent_starts_is_undone_at_restart` |
| B. Fork onto a fake box | Works (real daemon) | `fork_remote.rs::code_and_context_fork_onto_box_1_...` and Codex, Copilot before-turn tests |
| B. MCP continue-in, invalid `before_turn` | Likely bug | `mcp.rs:751` |

## Gaps and likely bugs

Not built: none found for the forks above. Every kind has a fork path in code. The gaps are proof gaps, listed as Code, no test or Fakes only in the steps and branches. The open proof gaps are the real CLI resuming a Humata fork (step 4, step 9), Codex and Copilot forks through the daemon on this Mac (steps 2, 5 to 7), and the parent resuming in its real CLI after a fork.

Likely bugs:

- `daemon/humatad/src/mcp.rs:751`: `session_continue_in_channel` turns an invalid `before_turn` into 0, which copies the whole conversation. The fork tool at about lines 1318–1334 refuses the same input. Confirm by calling the tool with `before_turn: 1`.

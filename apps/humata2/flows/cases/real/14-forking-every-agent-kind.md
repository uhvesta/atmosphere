# Every agent kind forks on this Mac and onto the VM, by code and context, code only, context only and from an earlier turn, and the fork resumes in the real CLI while the parent stays as it was

Layer: real (real `claude`, `codex` and `copilot` CLIs, real `humatad2`, this Mac, the `h2-terminal-c1` VM). Platforms: macOS host, Linux VM. Sibling cases: `agents/07-forking-itself.md`, `agents/08-forking-onto-another-machine.md`, `agents/10-roaming-between-agents.md`, `real/05`, `real/06`.

## Starting state

- This Mac runs a real `humatad2` in a temp state folder, started with an `--agents` JSON file that adds custom kinds `codex` (`codex-rollout`) and `copilot` (`copilot-session-state`). `claude` is built in.
- Channel `acme/api` has this Mac and the VM as machines (as in `real/01`, Humata's tools in the VM's `~/.humata2/bin`). The repo `acme/api` is cloned on this Mac.
- Three parent sessions run on this Mac, one per kind, each on its cheapest model (Claude's cheapest through its own `/model`, Codex GPT-6 Luna low, Copilot Claude Haiku 5.5 low). Each has three finished turns. The prompts are the same three for every kind: "In the api/ folder, find the flaky test", "Make it deterministic", "Now delete the old sleep". The first prompt names the clone folder because Humata does not tell agents where the repo is. Each parent has one commit, one uncommitted change and one untracked file in its clone.
- The Inbox is empty.

## Test setup

- Isolation. One temp folder per machine. Pass `COPILOT_HOME`, `CODEX_HOME`, `CLAUDE_CONFIG_DIR` and `HOME` to the daemon (`daemon/humatad/src/serve.rs:456-466`). The logins are copied into those homes and nowhere else. Never touch the owner's real `~/.copilot`, `~/.codex`, `~/.claude`, `~/.humata2` or SSH keys. Copilot leaked `recentModelIds` into the real home in the REALFLOW run, so check that home is unchanged at the end.
- The VM: `./terminal-ssh-demo.sh start`, then `status`; follow `terminal/platform_test/SSH_DEMO.md`. Reach it with `ssh -F ~/.lima/h2-terminal-c1/ssh.config lima-h2-terminal-c1`. Read-only check: `//apps/humata2/terminal/platform_test:ssh_demo_status_test`. The VM has 1 CPU, 0.75 GiB RAM and 4 GiB disk: run one CLI at a time there, and remove each fork after checking it.
- **Needs the owner's login** for every step that reaches a real model (Claude login, Codex `login`, Copilot device login), copied only into the run's temp homes. Alternative with no login: a loopback model server with isolated offline state. `terminal/provider/tests/fork_recovery.py` (used by `codex_fork_test` and `claude_fork_test`) and `copilot_fork.py` do this. They prove the CLI's own fork and resume, not Humata's copy and not a real model.
- **Needs a free screen** for steps in the app window. Headless equivalent: `humata session fork <id> --kind ...`, `humata session turns`, `humata session list`, or a `desktop/tests/live_*` style test.
- Proof that Humata forks by itself. Humata does not script the CLIs. `daemon/sessions/src/fork.rs` (`ForkCopy::CodeAndContext | CodeOnly | ContextOnly`, `fork_conversation`) copies the conversation files through the adapters in `daemon/conversation/src/{claude,codex,copilot}.rs`: a new id, a new folder, metadata rewritten. Each step below checks that against the real CLI: resume the fork in the CLI, see it remember the parent's turns up to the fork point, and see the parent unchanged (conversation files byte-identical, still resumes).
- Parent snapshot. Before the first fork, hash every conversation file of each parent: Claude's `<CLAUDE_CONFIG_DIR>/projects/<dashed folder>/<id>.jsonl`, Codex's `<CODEX_HOME>/sessions/<date>/rollout-*.jsonl`, Copilot's `<COPILOT_HOME>/session-state/<id>/`. Hash again after each fork and after each fork resumes.
- Cleanup: stop the daemons, delete the temp folders and copied logins, confirm the owner's CLI homes are unchanged.

## The matrix

Cells name the evidence; the real CLI half is unproven unless a cell says "real". "Daemon" means a real `humatad2` with fixture transcripts and fake agents.

| Kind, where | Code and context | Code only | Context only | From before turn 2 |
|---|---|---|---|---|
| Claude, this Mac | Daemon: `fork.rs::code_and_context_copies_the_work_and_the_conversation` | Daemon: `fork.rs::code_only_copies_the_work_into_a_fresh_conversation` | Daemon: `context_fork.rs::a_context_only_fork_carries_the_conversation_and_works_in_the_parents_code` | Daemon: `context_fork.rs::a_fork_before_a_turn_carries_the_conversation_up_to_it_and_the_code_as_now` |
| Claude, onto the VM | Daemon: `fork_remote.rs::code_and_context_fork_onto_box_1_has_the_parents_work_on_a_new_branch_and_a_copied_conversation` | Daemon: `fork_remote.rs::code_only_fork_from_box_1_onto_box_2_is_a_fresh_agent_and_the_parent_keeps_running` | Refused by design: `fork_remote.rs::a_context_only_fork_onto_another_machine_is_refused` | Code, no test (only Codex and Copilot have a box-fork-before-turn test) |
| Codex, this Mac | Code, no test through a fork; nearest: `continue_in.rs::a_codex_session_continues_in_another_channel_from_now` | Code, no test | Code, no test | Daemon: `continue_in.rs::a_codex_fork_before_turn_2_resumes_with_turn_1_only` |
| Codex, onto the VM | Code, no test; nearest: the before-turn test below | Code, no test | Refused by design (same test as Claude) | Daemon: `fork_remote.rs::a_codex_fork_from_box_1_onto_box_2_before_turn_2_carries_turn_1_only` |
| Copilot, this Mac | Code, no test through a fork; nearest: `continue_in.rs::a_copilot_session_continues_in_another_channel_from_now` | Code, no test | Code, no test | Daemon: `continue_in.rs::a_copilot_fork_before_turn_2_resumes_with_turn_1_only` |
| Copilot, onto the VM | Code, no test; nearest: the before-turn test below | Code, no test | Refused by design | Daemon: `fork_remote.rs::a_copilot_fork_from_box_1_onto_box_2_before_turn_2_carries_turn_1_only` |

Real CLI column for every row: the CLI's own fork and resume with a loopback model is proven in the terminal component (manual targets `//apps/humata2/terminal/provider:claude_fork_test`, `:codex_fork_test`, `:copilot_fork_test`). A hand-cut fork before turn 2, with the CLI resuming it and the parent unchanged, is proven by the manual fixtures `//apps/humata2/daemon/conversation:claude_roam_fixture`, `:codex_roam_fixture`, `:copilot_roam_fixture`. Both use the installed CLI and a loopback model, and neither runs Humata's adapter code or `humatad2`.

## Steps

1. **I do:** start the three parent sessions, give each the three prompts, and take the hash snapshot of each parent's conversation files. List the turns with `humata session turns <id>`.

   **I see:** three turns for each parent, with the prompts above.

   **State and events:** each session's `AgentLaunch` names its conversation id and file. Turns come from the adapter.

   **Status:** Works (real daemon) for listing turns with fixtures: `daemon/humatad/tests/continue_in.rs::a_codex_session_lists_its_turns`, `::a_copilot_session_lists_its_turns` and `daemon/humatad/tests/context_fork.rs::the_turns_are_the_prompts_of_the_conversation` (Claude). Works (real CLI, manual REALFLOW run) for starting real Copilot and Codex and running a task; no test pins it. Real Claude through `humatad2` was never run. Likely bug (seen live, B9): Humata never learns Codex's real session id, so the Codex launch record cannot name the real rollout. **Needs the owner's login.**

2. **I do:** for each kind, fork on this Mac with code and context, from now: `humata session fork <parent> --kind code-and-context --title "Fork A"`. I approve the Inbox card if one shows.

   **I see:** a new session with its own branch and clone. Its clone has the parent's commit, uncommitted change and untracked file. Its conversation id differs from the parent's.

   **State and events:** the fork's origin names the parent and the kind. Humata copies the conversation through the kind's adapter and starts the CLI with the new id.

   **Status:** Works (real daemon) for the copy, with the default fixture agent: `daemon/humatad/tests/fork.rs::code_and_context_copies_the_work_and_the_conversation`. Codex and Copilot code and context on this Mac have no fork test of their own (see the matrix). Adapter cuts: `daemon/conversation/tests/claude.rs::a_fork_from_now_holds_the_whole_current_path`, `daemon/conversation/tests/codex.rs::a_fork_from_now_holds_the_whole_rollout`, `daemon/conversation/tests/copilot.rs::a_fork_from_now_holds_every_line_of_a_finished_conversation`.

3. **I do:** resume the fork in the real CLI and ask "What were my first three prompts?".

   **I see (expected):** the CLI lists all three of the parent's prompts. Then I send a fourth, and it answers.

   **State and events:** the fork's conversation file holds the parent's records under the new id. The resume is by exact id.

   **Status:** No REALFLOW run got a fork resumed. Likely bug (seen live, B8): the real Copilot fork said "can't read this Copilot conversation". Likely bug (seen live, B5): resume looks under `--root` as home, so the CLI home may not hold the file. Codex fork is untested live and B9 applies. For Claude: Code, no test. Terminal-component proof with a loopback model: `//apps/humata2/terminal/provider:claude_fork_test`, `:codex_fork_test`, `:copilot_fork_test`. **Needs the owner's login.**

4. **I do:** re-hash the parent's conversation files, then resume the parent in its own CLI and ask for its three prompts.

   **I see:** every hash is the same as in step 1. The parent still resumes and still has three turns.

   **State and events:** a fork reads the parent's files and writes only new ones.

   **Status:** Works (real daemon) for the files: `daemon/conversation/tests/claude.rs::forking_leaves_the_parents_files_as_they_were`, `daemon/conversation/tests/codex.rs::forking_leaves_the_parent_and_the_clis_state_as_they_were`, and the session-level `assert_source_untouched` in `continue_in.rs` (checked in `::a_codex_fork_before_turn_2_resumes_with_turn_1_only`). Copilot adapter: `daemon/conversation/tests/copilot.rs::a_fork_before_a_turn_holds_exactly_the_lines_before_its_start_under_the_new_id`. The real parent resuming after a fork is unproven through `humatad2`; the roam fixtures check the hash and the resume with the installed CLI by hand-cutting.

5. **I do:** for each kind, fork on this Mac with code only: `--kind code-only`.

   **I see:** the fork has the parent's work on a new branch and a fresh conversation. The CLI starts with no memory of the parent's prompts.

   **State and events:** the fork's origin kind is CodeOnly. No conversation file is copied. The parent's files are unchanged.

   **Status:** Works (real daemon) for the default fixture agent: `daemon/humatad/tests/fork.rs::code_only_copies_the_work_into_a_fresh_conversation`. Code, no test for Codex and Copilot kinds. The real CLI half is unproven.

6. **I do:** for each kind, fork on this Mac with context only: `--kind context-only`.

   **I see:** the fork carries the conversation and works in the parent's clone, on the parent's branch. No new branch is made.

   **State and events:** the fork's origin kind is ContextOnly. Its conversation id differs from the parent's. Two agents now work in one folder; I note that this is what the kind means.

   **Status:** Works (real daemon). `daemon/humatad/tests/context_fork.rs::a_context_only_fork_carries_the_conversation_and_works_in_the_parents_code`, with a Claude-shaped fixture. Code, no test for Codex and Copilot kinds. The real CLI half is unproven.

7. **I do:** for each kind, fork on this Mac from an earlier turn: `before_turn` 2, code and context. The agent surface cannot ask for it; I ask over gRPC or in the app's fork dialog.

   **I see:** a fork whose conversation holds turn 1 only. Resumed in the real CLI and asked for its prompts, it names "Find the flaky test" and not the other two. The code is the parent's as now.

   **State and events:** the cut is made by the kind's own format: Claude's records before the second prompt, Codex's lines before turn 2's start, Copilot's lines before its second `user.message`. The fork's origin has `before_turn` 2.

   **Status:** Works (real daemon) for all three: Claude `daemon/humatad/tests/context_fork.rs::a_fork_before_a_turn_carries_the_conversation_up_to_it_and_the_code_as_now`; Codex `daemon/humatad/tests/continue_in.rs::a_codex_fork_before_turn_2_resumes_with_turn_1_only`; Copilot `::a_copilot_fork_before_turn_2_resumes_with_turn_1_only`. Adapter cuts: `daemon/conversation/tests/claude.rs::a_fork_before_a_turn_holds_exactly_the_turns_before_it`, `codex.rs::a_fork_before_a_turn_holds_exactly_the_lines_before_its_start`. The real CLI resuming it: the roam fixtures hand-cut and resume with the installed CLI; Humata's own cut resumed in a real CLI is unproven. Not built on the agent surface: the CLI and MCP never send `before_turn` (`cli/src/commands/sessions.rs:130`, `daemon/humatad/src/mcp.rs:698-737`).

8. **I do:** for each kind, fork onto the VM with code and context: `humata session fork <parent> --kind code-and-context --machine <vm>`. I approve the card.

   **I see:** a fork live on the VM, on a new branch there, with the parent's commit, uncommitted change and untracked file. The parent keeps running on this Mac.

   **State and events:** the conversation is packed through the adapter, copied over the link, unpacked under the VM's CLI home with no path of this Mac, and the CLI starts with the new id.

   **Status:** Works (real daemon) for Claude with fake `ssh` and a fixture agent: `daemon/humatad/tests/fork_remote.rs::code_and_context_fork_onto_box_1_has_the_parents_work_on_a_new_branch_and_a_copied_conversation`. Copilot carry: `daemon/sessions/src/carry_roam.rs::a_carried_copilot_conversation_names_no_old_path_and_reads_on_the_new_machine`. Code, no test for a Codex fork from now onto a box. No test uses the real VM with real Humata binaries and a real CLI. **Needs the owner's login** on the VM.

9. **I do:** for each kind, fork onto the VM from before turn 2, then resume the fork there in the real CLI.

   **I see:** a fork on the VM with turn 1 only. The real CLI names only the first prompt.

   **State and events:** the fork's file is on the VM, cut where its own format says turn 2 starts. The parent's file on this Mac is unchanged.

   **Status:** Works (real daemon) for Codex and Copilot with fixtures and a fake `ssh`: `daemon/humatad/tests/fork_remote.rs::a_codex_fork_from_box_1_onto_box_2_before_turn_2_carries_turn_1_only` and `::a_copilot_fork_from_box_1_onto_box_2_before_turn_2_carries_turn_1_only`. Code, no test for Claude onto a box before a turn. The real CLI on the VM resuming a carried fork is unproven; the VM is tight for the Copilot CLI (Node, about 200 MB).

10. **I do:** for each kind, fork onto the VM with code only, then with context only.

    **I see:** code only: a fresh agent on the VM with the parent's work. Context only: refused, and nothing is made.

    **State and events:** a context-only fork has no clone to share with another machine, so it is refused up front.

    **Status:** Works (real daemon) for Claude-shaped fixtures: `daemon/humatad/tests/fork_remote.rs::code_only_fork_from_box_1_onto_box_2_is_a_fresh_agent_and_the_parent_keeps_running` and `::a_context_only_fork_onto_another_machine_is_refused`. Code, no test for Codex and Copilot kinds.

11. **I do:** fork a fork on the VM, then fork that child back onto this Mac, for the Claude kind only.

    **I see:** each child names its parent. Three branches, three conversation ids. Every parent's files are as they were.

    **State and events:** the lineage is parent, fork, child.

    **Status:** Works (real daemon) for nesting on one machine: `daemon/humatad/tests/fork.rs::forks_nest_and_each_names_its_parent`. Code, no test across two machines.

## Branches

### The Copilot fork says it cannot read the conversation

**When:** step 3, a real Copilot session is forked from now or before a turn.

**I see:** "can't read this Copilot conversation" and no fork.

**Expected:** a fork that resumes with the parent's turns.

**Status:** Likely bug (seen live, B8). ROAM's real-CLI captures and fixes are pending. Confirm by forking the Copilot parent from step 1 and reading the refusal. The adapter accepts the fixtures but may reject a real record: `daemon/humatad/tests/continue_in.rs::a_fork_of_a_copilot_conversation_with_a_record_humata_doesnt_know_is_refused_up_front` shows an unknown record is refused up front.

### The Codex fork cannot resume

**When:** step 3 for Codex.

**I see:** the fork's CLI cannot find its rollout, or resumes the parent's.

**Expected:** the fork resumes its own rollout by its new id.

**Status:** Likely bug (seen live, B9): Humata never learns Codex's real session id, since the rollout path is dated. Codex fork was not tried live.

### The Codex adapter's source note says it is waiting

**When:** I read `daemon/conversation/src/lib.rs:52` ("Waits on the layout fixtures (CV-7)").

**I see:** the doc comment says the Codex format is not ready. The adapter and its tests exist (`daemon/conversation/tests/codex.rs`, `codex_roam.rs`), and the manual `codex_layout_test` and `codex_roam_fixture` run the real CLI.

**Status:** Likely bug (stale comment). Confirm by reading the file and the tests. Whether the real layout still matches the fixtures is the real-CLI half and is unproven through `humatad2`.

### The CLI is older than Humata knows

**When:** the installed Copilot is older than 1.0.112-0.

**I see:** the fork is refused up front and names the version.

**Status:** Works (real daemon). `daemon/humatad/tests/continue_in.rs::a_fork_of_a_copilot_session_launched_by_a_cli_older_than_humata_knows_is_refused_up_front`.

### The fork is cut by a restart

**When:** the host restarts part way through a fork.

**I see:** no half-made fork after the restart, or a finished one if the agent had started. The parent is unchanged.

**Status:** Works (real daemon). `daemon/humatad/tests/fork.rs::a_fork_cut_before_its_agent_starts_is_undone_at_restart`, `::a_fork_cut_after_its_agent_started_is_kept_at_restart` and `daemon/humatad/tests/fork_remote.rs::a_fork_cut_short_by_a_host_restart_is_undone_or_finished`.

### The VM is offline

**When:** step 8, the VM link is down.

**I see:** the fork is refused and leaves nothing.

**Status:** Works (real daemon). `daemon/humatad/tests/fork_remote.rs::a_fork_onto_an_offline_box_is_refused_and_leaves_nothing`.

### A fork before a turn that is not there

**When:** step 7 asks for turn 4 of a three-turn conversation, or turn 1.

**I see:** the fork is refused as an invalid argument. Nothing is made.

**Status:** Works (real daemon). `daemon/humatad/tests/context_fork.rs::a_fork_before_a_turn_that_is_not_there_is_refused`.

## Expected outcomes

- [ ] Each of Claude, Codex and Copilot forks on this Mac and onto the VM with code and context, code only, and from before turn 2; context only works on the same machine and is refused onto another.
- [ ] Humata copies the conversation through the kind's adapter; the CLIs are not scripted to fork.
- [ ] A code and context fork, resumed in the real CLI, remembers every turn the parent had; a fork from before turn 2 remembers turn 1 only.
- [ ] A code-only fork starts a fresh conversation with the parent's work.
- [ ] The parent's conversation files are byte-identical after every fork and after the fork runs, and the parent still resumes.
- [ ] No fork writes into the owner's real CLI homes.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Three parents and their turns | Works (real daemon) | `continue_in.rs::a_codex_session_lists_its_turns`; B9 on Codex id |
| 2. Fork on this Mac, code and context | Works (real daemon) | `fork.rs::code_and_context_copies_the_work_and_the_conversation` |
| 3. Fork resumes in the real CLI | Likely bug (seen live, B8, B5, B9) | REALFLOW; terminal-only `*_fork_test` |
| 4. Parent unchanged and resumes | Works (real daemon) | `claude.rs::forking_leaves_the_parents_files_as_they_were`, `codex.rs::forking_leaves_the_parent_and_the_clis_state_as_they_were` |
| 5. Fork on this Mac, code only | Works (real daemon) | `fork.rs::code_only_copies_the_work_into_a_fresh_conversation` |
| 6. Fork on this Mac, context only | Works (real daemon) | `context_fork.rs::a_context_only_fork_carries_the_conversation_and_works_in_the_parents_code` |
| 7. Fork from before turn 2 | Works (real daemon) | `context_fork.rs`, `continue_in.rs` before-turn tests |
| 8. Fork onto the VM, code and context | Works (real daemon) | `fork_remote.rs::code_and_context_fork_onto_box_1_...` (Claude shape) |
| 9. Fork onto the VM before turn 2 | Works (real daemon) | `fork_remote.rs::a_codex_fork_from_box_1_...`, `::a_copilot_fork_from_box_1_...` |
| 10. Onto the VM, code only and context only | Works (real daemon) | `fork_remote.rs::a_context_only_fork_onto_another_machine_is_refused` |
| 11. Fork of a fork across machines | Code, no test | `fork.rs::forks_nest_and_each_names_its_parent` (one machine) |
| B. Copilot says it cannot read | Likely bug (seen live, B8) | REALFLOW |
| B. Codex fork cannot resume | Likely bug (seen live, B9) | REALFLOW |
| B. Stale Codex comment | Likely bug | `daemon/conversation/src/lib.rs:52` |
| B. CLI older than known | Works (real daemon) | `continue_in.rs::a_fork_of_a_copilot_session_launched_by_a_cli_older_than_humata_knows_is_refused_up_front` |
| B. Restart mid-fork | Works (real daemon) | `fork.rs`, `fork_remote.rs` cut tests |
| B. VM offline | Works (real daemon) | `fork_remote.rs::a_fork_onto_an_offline_box_is_refused_and_leaves_nothing` |
| B. Turn not there | Works (real daemon) | `context_fork.rs::a_fork_before_a_turn_that_is_not_there_is_refused` |

## Gaps and likely bugs

Not built:

- Built-in `codex` and `copilot` kinds (ROAM batch 3, C4, X2); forks of those kinds need an `--agents` file on both machines.
- `before_turn` on the CLI and MCP (no milestone): only gRPC and the app can fork before a turn.
- An agent for a fork on the CLI and MCP (ROAM batch 3).
- `ReadConversation` (`daemon/humatad/src/session_service.rs:2681`) answers not built, so `humata session read` cannot show a fork's turns for any kind; B4/B8.
- No test forks Codex or Copilot from now, code only or context only (the matrix cells marked "Code, no test"), and none forks Claude before a turn onto a box.
- No test runs Humata's adapter output through a real CLI: the terminal `*_fork_test` and the roam fixtures hand-cut or let the CLI fork itself.

Likely bugs:

- B8: the real Copilot fork says "can't read this Copilot conversation".
- B9: Humata never learns Codex's real session id; Codex resume and fork cannot find the rollout. Codex fork untested live.
- B5: resume looks for the conversation under `--root` as home, so a fork may not be found.
- B6: a stopped fork shows Idle, not Stopped.
- B7 and B10: a resume after a stop is treated as an interruption; attach is refused while resuming.
- `daemon/conversation/src/lib.rs:52` says the Codex adapter waits on CV-7, but the adapter and its tests exist; the comment is stale.
- Copilot writes `recentModelIds` into the real `~/.copilot/config.json` unless `COPILOT_HOME` is isolated.

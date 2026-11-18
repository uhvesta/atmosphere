# The same Codex conversation moves to the VM, forks back to this Mac, and moves home

Layer: real (real Codex CLI, real `humatad2`, the real SSH VM `h2-terminal-c1`). Platforms: macOS host, Linux arm64 VM. Continues `real/07`.

## Starting state

- The run of `real/07` is done: a session `Tiny README edit` on this Mac, kind `codex`, GPT-6 Luna chosen in the CLI, one turn that edited `README.md`, then stopped and started again.
- The VM is added as a machine (`real/01-the-vm-becomes-a-machine.md`), is in the channel `acme/api`, and has Codex 0.169.0 in an isolated home (`real/04-agent-clis-on-the-vm-for-a-test.md`).
- The VM's own `humatad2 --role machine` knows the `codex` kind from its own `--agents` file.
- The Inbox is empty. No other session moves.

## Test setup

- Everything in `real/07` and `real/04` applies. `CODEX_HOME` is a different temp folder on each machine, as `daemon/humatad/tests/common/remote_rig.rs` does with `{home}/.codex`. The rollout's dated path and its thread index live under that home.
- The VM has 0.75 GiB of RAM and a 4 GiB disk. A move copies the session folder, clone and rollout. Read `df -Pk /var/tmp` and `free -m` on the VM first. Never stop, restart or delete the VM.
- **Needs the owner's login** on both machines: a Codex `login` in each isolated `CODEX_HOME`. The token must land only there. Never copy the owner's token. The alternative is the loopback model server (`terminal/provider/tests/codex_resume.py`), which proves resume without a login and not a real model.
- Headless: the `humata` CLI against the real daemon. **Needs a free screen** only for the Move session palette entry and the machine pages.
- Cleanup: remove the run's folders on the VM and on this Mac, and confirm the owner's own `~/.codex` is unchanged.

## Steps

1. **I do:** check that the Codex session from `real/07` has a conversation Humata can read.

   **I see:** the rollout under this Mac's `CODEX_HOME/sessions/<year>/<month>/<day>/`, and `AgentLaunch.conversation_file` for the session.

   **State and events:** a move carries the conversation named by `conversation_file` (`daemon/sessions/src/carry.rs`). If Humata never learned Codex's real id, the file it names is not the rollout Codex wrote.

   **Status:** Likely bug (seen live, B9) when the two differ. This step is the precondition for every step below, so a run that fails here goes to the branch "the conversation is not found". Confirm by comparing the two paths.

2. **I do:** move the session to the VM (palette ⌘K › Move session, or `humata session move <session> --to <vm>`).

   **I see:** the move shows its steps. When it ends, the agent runs on the VM and the session folder on this Mac is gone.

   **State and events:** the agent pauses, the folder, clone and conversation are copied and checked, the agent resumes on the VM, and only then is this Mac's folder freed. The rollout is placed under the VM's `CODEX_HOME` and rewritten to name the VM's folder and the new clone's branch and commit (MV-16, CV-4).

   **Status:** Works (real daemon) for the move with a fake agent: `daemon/humatad/tests/move_session.rs::a_session_moved_box_1_to_box_2_keeps_id_branch_work_and_conversation_and_resumes` and `::moving_from_this_mac_frees_its_folder_only_after_it_runs_on_box_1`. No test in `move_session.rs` uses a Codex kind (searched it for `codex`). The Codex half is the adapter alone: `daemon/conversation/tests/codex_roam.rs::a_carry_to_another_machine_leaves_the_old_machines_turn_history_as_it_was` packs and places a rollout on another machine's home, and `daemon/sessions/src/carry_landing_tests.rs::a_codex_desk_session_moved_into_a_channel_has_its_rollout_and_thread_row_in_the_new_folder` lands one on the same machine. Real Codex, real VM, real move: Code, no test. I searched `daemon/sessions/src/carry*.rs` and `conversation/tests/codex_roam.rs` for a move of a Codex rollout between two machines through the daemon and found none.

3. **I do:** open the agent tab on the VM.

   **I see (expected):** Codex shows the earlier turn and the README edit. It is the same conversation, not a new one.

   **State and events:** the daemon runs the kind's `resume_args` on the VM with the conversation id and checks that the placed file exists first (PER-14). Real Codex finds its conversation by its own id in its own thread index under the VM's `CODEX_HOME`. The carry rewrites that row (`restore_thread_git`, `place`), and the VM's CLI builds its own turn history.

   **Status:** Code, no test for real Codex. Works (real daemon) for a fake agent that resumes on the target: `move_session.rs::a_session_moved_box_1_to_box_2_keeps_id_branch_work_and_conversation_and_resumes`. Likely bug (seen live, B9) for resume with the id Humata pinned, because Codex's real id is not the one Humata knows. Whether Codex resumes from a thread row the carry wrote, in a home that never ran it, is unproven.

4. **I do:** type a question that needs the earlier turn with `humata session type`, which presses Enter: `What did you add to api/README.md?`.

   **I see (expected):** Codex answers from the earlier turn.

   **State and events:** the VM's Codex has its own login and model. The model is the CLI's own session setting.

   **Status:** Code, no test. Unproven. **Needs the owner's login.** Fixed (B12, D754): `humata session type` presses Enter by default, so no second Return key is sent.

5. **I do:** check the model with `/model`.

   **I see:** unknown: GPT-6 Luna with low reasoning kept, or Codex's default for that home.

   **State and events:** Humata records no model (`real/07` step 5), so a move cannot carry it. The rollout's turn context may name the model, and Codex may read it on resume. Write down what is seen.

   **Status:** Code, no test. Unproven.

6. **I do:** on the VM, trust the hooks (or choose Continue without trusting) at "Hooks need review", then give a task that needs a file write.

   **I see:** the same prompt appears once for this home. If I trust, a card reaches the Inbox on this Mac.

   **State and events:** a hook on the VM reaches the VM's daemon, which raises the request on the host's Inbox over the link. The trust fingerprint is the daemon's own (`daemon/humatad/src/inbox/permission/codex_trust.rs`), kept per daemon.

   **Status:** Code, no test. No test holds a Codex hook on a remote machine (searched `daemon/humatad/tests/hooks.rs`, `common/remote_rig.rs`); the Codex trust step is Works (real daemon) on one daemon only: `daemon/humatad/tests/codex_trust.rs::codexs_hooks_need_trust_until_one_of_them_is_heard`.

7. **I do:** on this Mac, fork the moved session onto this Mac, code and context, from before turn 2.

   **I see (expected):** a new session on this Mac with its own branch, clone and conversation id. The session on the VM is unchanged.

   **State and events:** the VM stages the fork's files, packs them, and this Mac places them under its `CODEX_HOME` with the new id in `session_meta`. The parent's rollout and home are untouched (`daemon/sessions/src/carry_reid_tests.rs::a_codex_fork_staged_for_another_machine_names_its_own_id_in_its_session_meta`).

   **Status:** Works (real daemon) with fixtures for box to box: `daemon/humatad/tests/fork_remote.rs::a_codex_fork_from_box_1_onto_box_2_before_turn_2_carries_turn_1_only` (the CLI is a stand-in that only answers). The direction VM to this Mac, with a real rollout, is Code, no test. The fixture puts its rollout at the pinned path, so B9 cannot show in it. A real fork is untested live (no REALFLOW fork).

8. **I do:** answer in the fork: `What did turn 1 say?`.

   **I see (expected):** the fork knows turn 1 and nothing after it.

   **State and events:** the fork's CLI resumes its own rollout.

   **Status:** Works (real daemon) for the daemon half: `daemon/humatad/tests/continue_in.rs::a_codex_fork_before_turn_2_resumes_with_turn_1_only`. Real Codex through `humatad2`: unproven. The terminal experiment forks real Codex with a loopback model: `//apps/humata2/terminal/provider:codex_fork_test` (manual, macOS only).

9. **I do:** move the original session from the VM back to this Mac.

   **I see:** the session returns to this Mac with its work, and the VM's folder is freed.

   **State and events:** the same steps as step 2 in the other direction. The rollout is placed in this Mac's `CODEX_HOME` at a date folder that may be new. This Mac's CLI builds its turn history again.

   **Status:** Works (real daemon) for the move with a fake agent (the `move_session.rs` tests above are box to box and this Mac to box). The Codex half is Code, no test. The adapter's other direction is covered only by the same `codex_roam.rs` tests.

10. **I do:** continue the conversation on this Mac.

    **I see (expected):** the turns from before the first move, from the VM, and from this Mac are all there once.

    **State and events:** the host's mirror of the conversation adds only lines after the move.

    **Status:** Works (real daemon) for the mirror with a fake agent: `move_session.rs::a_moved_conversation_is_mirrored_once`. Real Codex: Code, no test.

## Branches

### The conversation is not found (B9)

**When:** step 1 shows `conversation_file` is not where Codex wrote its rollout.

**I do:** move the session.

**I see (expected):** either a refusal that names the missing file and how to fix it, or a move that carries the real rollout. **I see (risk):** a move that copies an empty or missing file and resumes with no history.

**Status:** Likely bug (seen live, B9). The move code checks the file (`PER-14`), but its refusal text for a real Codex path is untested. A failed resume can start fresh: `daemon/humatad/tests/start_fresh.rs::a_failed_resume_starts_fresh_on_a_new_conversation_and_hears_the_summary`, with a fake agent.

### Codex is not installed on the VM

**When:** the VM has no Codex, or it is not at the path in the VM's `--agents` file.

**I do:** move the session.

**I see (expected):** the move fails before the old agent stops, or the resume fails and the session stays on this Mac.

**Status:** Code, no test for a missing program. A copy that fails leaves the session at the source: `move_session.rs::a_move_whose_copy_fails_leaves_it_running_on_the_source_and_names_the_step`. Version check: a fork of a session whose CLI is older than Humata knows is refused for Copilot (`continue_in.rs::a_fork_of_a_copilot_session_launched_by_a_cli_older_than_humata_knows_is_refused_up_front`); no Codex version check was found.

### The link drops or the VM is the wrong target

**When:** the VM is offline, outside the channel, or I cancel mid-move, or a host kill hits it.

**I see:** the same refusals and undo as any move. The session runs on this Mac.

**Status:** Works (real daemon) with a fake agent: `move_session.rs::a_move_to_an_offline_machine_or_one_outside_the_channel_is_refused_and_changes_nothing`, `::a_move_cancelled_before_the_agent_stops_is_undone_and_one_after_is_refused`, `::a_move_cut_short_after_the_agent_moved_is_finished_at_restart`. The Codex rollout put back after a cut landing: `carry_landing_tests.rs::a_codex_landing_taken_back_after_the_unpack_leaves_its_rollout_and_thread_row_as_they_were`. With real Codex on the VM: unproven.

### The VM has too little memory or disk

**When:** the 0.75 GiB VM runs Codex, `humatad2` and a holder, or the 4 GiB disk fills.

**I see (expected):** a named error for a full disk; for memory, the kernel may kill Codex and the session shows it ended.

**Status:** Code, no test for the memory case. The 2 GiB free-disk check is in `daemon/machines/src/setup.rs:29` and covers adding a machine, not a move. Move disk-full: Not built (searched `move_session.rs` for a disk check; none).

### A fork from the VM when the VM is offline

**When:** step 7, and the VM's link is down.

**I see:** the fork is refused with the machine named.

**Status:** Code, no test for a fork from an offline source. The target-offline refusal is the move one above.

### A move of a stopped session

**When:** I stop the session first, then move it.

**I see:** the session stays stopped on the VM, no agent starts.

**Status:** Works (real daemon) for the undo of a stopped move: `move_session.rs::a_stopped_session_whose_move_is_undone_at_restart_stays_stopped`. A completed move of a stopped Codex session is Code, no test.

## Expected outcomes

- [ ] The owner's own Codex homes and SSH keys are untouched, on this Mac and on the VM.
- [ ] After the move the session runs on the VM and nowhere else, with its work and its conversation.
- [ ] Codex on the VM answers from the earlier turn, so it is the same conversation.
- [ ] Whether the model survives the move is written down.
- [ ] A hook ask on the VM reaches this Mac's Inbox.
- [ ] A fork onto this Mac has a new conversation id, holds turn 1 only, and leaves the VM's session unchanged.
- [ ] After the move back the session runs on this Mac and the VM's folder is freed.
- [ ] The conversation holds each turn once after both moves.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Conversation file is the real rollout | Likely bug (seen live, B9) | REALFLOW run |
| 2. Move to the VM | Works (real daemon) with a fake agent; Code, no test for Codex | `daemon/humatad/tests/move_session.rs::a_session_moved_box_1_to_box_2_keeps_id_branch_work_and_conversation_and_resumes` |
| 3. Codex resumes on the VM | Code, no test; Likely bug (seen live, B9) | `daemon/conversation/tests/codex_roam.rs::a_carry_to_another_machine_leaves_the_old_machines_turn_history_as_it_was` (adapter only) |
| 4. Continue the conversation | Code, no test; Fixed (B12, D754): `humata session type` presses Enter by default | none |
| 5. Model after the move | Code, no test | none |
| 6. Hooks and an ask on the VM | Code, no test | `daemon/humatad/tests/codex_trust.rs::codexs_hooks_need_trust_until_one_of_them_is_heard` (one daemon) |
| 7. Fork onto this Mac | Works (real daemon) with fixtures for box to box; Code, no test for VM to Mac | `daemon/humatad/tests/fork_remote.rs::a_codex_fork_from_box_1_onto_box_2_before_turn_2_carries_turn_1_only` |
| 8. Fork resumes with turn 1 only | Works (real daemon) with fixtures; real CLI unproven | `daemon/humatad/tests/continue_in.rs::a_codex_fork_before_turn_2_resumes_with_turn_1_only` |
| 9. Move back to this Mac | Works (real daemon) with a fake agent; Code, no test for Codex | `daemon/humatad/tests/move_session.rs::moving_from_this_mac_frees_its_folder_only_after_it_runs_on_box_1` |
| 10. Continue on this Mac | Works (real daemon) for the mirror; real Codex Code, no test | `daemon/humatad/tests/move_session.rs::a_moved_conversation_is_mirrored_once` |
| Branch: conversation not found | Likely bug (seen live, B9) | `daemon/humatad/tests/start_fresh.rs::a_failed_resume_starts_fresh_on_a_new_conversation_and_hears_the_summary` |
| Branch: Codex not on the VM | Code, no test | `daemon/humatad/tests/move_session.rs::a_move_whose_copy_fails_leaves_it_running_on_the_source_and_names_the_step` |
| Branch: link drops or wrong target | Works (real daemon) with a fake agent | `daemon/humatad/tests/move_session.rs::a_move_to_an_offline_machine_or_one_outside_the_channel_is_refused_and_changes_nothing` |
| Branch: too little memory or disk | Code, no test; Not built for a move disk check | `daemon/machines/src/setup.rs:29` |
| Branch: fork from an offline VM | Code, no test | none |
| Branch: move a stopped session | Works (real daemon) for the undo; Code, no test for completion | `daemon/humatad/tests/move_session.rs::a_stopped_session_whose_move_is_undone_at_restart_stays_stopped` |

## Gaps and likely bugs

- Not built: built-in `codex` kind on either machine (ROAM batch 3, C4, X2); the VM needs its own `--agents` file.
- Not built: Humata records no model, so a move cannot pin it (no milestone).
- Not built: a free-disk check before a move (no milestone; searched `move_session.rs`).
- Not built: `ReadConversation` for any kind (`daemon/humatad/src/session_service.rs:2681`), so the moved conversation cannot be read through `humata session read`.
- Likely bug (seen live, B9): Humata never learns Codex's real session id, so the carry and resume may not find the real rollout. Every Codex daemon test pins the rollout at the path the daemon chose, so none can show it. This blocks steps 1, 3 and 7 until fixed.
- Fixed (B12, D754): `humata session type` presses Enter by default; `--no-submit` leaves the text unsent.
- Unproven: no daemon test moves a Codex rollout between two machines. `codex_roam.rs` does it in the adapter alone, and `carry_landing_tests.rs` lands one on the same machine.
- Unproven: real Codex resumes from a thread row that a carry wrote into a home that never ran it. The adapter's header still says it "waits on the layout fixtures (CV-7)".
- Unproven: the model, reasoning level and hook trust on the VM.
- Manual only: nothing here ran live. The REALFLOW run did not reach a move.

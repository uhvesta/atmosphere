# The same Copilot conversation moves to the VM, forks back to this Mac, and moves home

Layer: real (real Copilot CLI, real `humatad2`, this Mac, the `h2-terminal-c1` VM). Platforms: macOS host. Continues `real/05`.

## Starting state

- The session from `real/05`, `Tiny README edit`, runs a custom `copilot` kind on this Mac, in channel `acme/api`. Its conversation has two turns: the README edit and one follow-up. The README change is uncommitted in its clone.
- Claude Haiku 5.5, low reasoning and the smallest context were chosen with `/model` in that session. Humata records none of it.
- The VM `h2-terminal-c1` is added as a machine as in `real/01` and is in the channel `acme/api`. Humata's tools are in its `~/.humata2/bin`.
- The pinned Copilot CLI is on the VM as in `real/04`, with its own `COPILOT_HOME` and a copied login. The same custom `copilot` kind is known there.
- The Inbox is empty.

## Test setup

- `./terminal-ssh-demo.sh start`, then `status`. Follow `terminal/platform_test/SSH_DEMO.md`. Reach the VM with `ssh -F ~/.lima/h2-terminal-c1/ssh.config lima-h2-terminal-c1`. The read-only check is `//apps/humata2/terminal/platform_test:ssh_demo_status_test`. Never restart or recreate the shared VM.
- The VM has 1 CPU, 0.75 GiB RAM and 4 GiB disk. Copilot is Node-based and its tarball is about 200 MB, so it is tight. Say so in the run notes and watch memory during the turn.
- Isolation: the temp state folder and `COPILOT_HOME` from `real/05` on this Mac, and a run folder on the VM with its own `COPILOT_HOME`. The login is copied in, never the owner's whole home. The daemon reads `COPILOT_HOME` on each side (`daemon/humatad/src/serve.rs:455-466`).
- **Needs the owner's login** for every step that talks to the real model. **Needs a free screen** for every step in the app window; the headless equivalent is the `humata` CLI.
- The daemon halves below ran with fixture transcripts and fake boxes (`daemon/humatad/tests/common/remote_rig.rs` puts a fake `ssh` in front of the box). They count as Works (real daemon) with the real CLI half unproven. The real Copilot CLI was proven only in the terminal experiment, with a loopback model.
- Cleanup: remove the run folder and the copied login on the VM, and stop the daemon.

## Steps

1. **I do:** open the session's Move dialog (⌥⌘M), pick the VM, and confirm. Headless: `humata session move <session> --to <vm>`.

   **I see:** the dialog closes. The session shows Moving, then the agent tab on the VM.

   **State and events:** `MoveSession` with a fresh key. The operation steps are check, pause, pack, copy, verify, resume, switch, release. Pack reads the Copilot session state through the adapter and rewrites it to name the new folder, so nothing names this Mac.

   **Status:** Works (real daemon) for the move with a fake agent: `daemon/humatad/tests/move_session.rs::a_session_moved_box_1_to_box_2_keeps_id_branch_work_and_conversation_and_resumes`. Works (real daemon) for the Copilot carry: `daemon/sessions/src/carry_roam.rs::a_carried_copilot_conversation_names_no_old_path_and_reads_on_the_new_machine`. No test moves a Copilot session between machines through the whole daemon, and none uses a real VM with real Humata binaries. The app's dialog is **Needs a free screen**; its fake-daemon test is `desktop/tests/native/fork_move.rs::the_move_dialog_matches_frame_63_and_moves_to_the_picked_machine` (Fakes only).

2. **I do:** wait for the operation to end.

   **I see:** Succeeded. The session's machine is the VM. The clone on this Mac is gone, with a copy in its trash.

   **State and events:** the session keeps its id, title, channel and branch. The README edit and any unpushed commit are in the VM clone. Nothing runs on this Mac.

   **Status:** Works (real daemon) for a box-to-box move: `daemon/humatad/tests/move_session.rs::a_session_moved_box_1_to_box_2_keeps_id_branch_work_and_conversation_and_resumes`. A real SSH copy to the Lima VM is unproven; `daemon/machines/tests/lima.rs::a_lima_vm_is_created_set_up_stopped_and_deleted` uses its own VM and fake Humata binaries.

3. **I do:** open the session's agent tab on the VM.

   **I see:** Copilot resumes and shows both earlier turns. It answers a new line I type, `still here`, and keeps the same conversation id.

   **State and events:** the daemon runs the kind's `resume_argv` on the VM with the pinned conversation id. It checks that `conversation_file` exists under the VM's `COPILOT_HOME` first (PER-14).

   **Status:** Works (real daemon) for the resume of the carried state with a fixture: `daemon/humatad/tests/desk_into_channel.rs::a_copilot_desk_session_moved_into_a_channel_resumes_its_session_state_in_the_new_folder` (this Mac only). The real Copilot resuming on the VM is unproven. Likely bug (seen live, B5): resume looks for the conversation under `--root` as home, not `$HOME` or `COPILOT_HOME`; the VM resume will hit it too. Likely bug (seen live, B7): a resume after a stop is treated as an interruption and kills the resumed agent. **Needs the owner's login.**

4. **I do:** type `/model` in the VM's agent tab and read the current model.

   **I see:** unknown. It may be Claude Haiku 5.5 or Copilot's default there.

   **State and events:** the model is the CLI's own session state, held in `session-state/<id>` and its config. Humata copies the session state, but not the CLI's config, which names the recent models. So the choice may not survive. Write the answer down.

   **Status:** Code, no test. Unproven. Not built: Humata keeps no model setting to restore (no milestone). **Needs the owner's login.**

5. **I do:** type a small task: `In the api/ folder, add a second line to README.md`. Allow the Inbox ask.

   **I see:** an Inbox card that names the VM. After Allow, the edit is made in the VM clone and the turn ends Idle.

   **State and events:** the Copilot hooks on the VM call the VM's `humata hook copilot permission`, which relays to the host. The host's Inbox shows the card with the machine.

   **Status:** Works (real daemon) for the relay of a box agent's ask: `daemon/humatad/tests/hooks.rs::each_agent_gets_the_answer_in_its_own_shape` (this Mac only, Copilot shape). No test holds a Copilot hook on a box. Fixed (B12, D754): `humata session type` presses Enter by default. **Needs the owner's login.**

6. **I do:** read the conversation with `humata session read <session>`.

   **I see (expected):** both turns plus the new one. **I see (REALFLOW):** nothing.

   **State and events:** `ReadConversation` answers `not_built` (`daemon/humatad/src/session_service.rs:2681`). The host's mirror of the conversation holds each line once after a move.

   **Status:** Likely bug (seen live, B4/B8) for the read. The mirror is Works (real daemon): `daemon/humatad/tests/move_session.rs::a_moved_conversation_is_mirrored_once` (fake agent).

7. **I do:** fork the session from the VM onto this Mac, code and context, before the last turn. Headless: `humata session fork <session> --to <this Mac>`.

   **I see:** a new session on this Mac with its own branch, clone and conversation id. It shows only the turns before the cut. The parent still runs on the VM and is unchanged.

   **State and events:** the daemon reads the Copilot session state on the VM, cuts it, rewrites it for the new folder, and copies it to this Mac. It does not script the CLI.

   **Status:** Works (real daemon) with fixtures: `daemon/humatad/tests/fork_remote.rs::a_copilot_fork_from_box_1_onto_box_2_before_turn_2_carries_turn_1_only`, and for a fork on one machine `daemon/humatad/tests/continue_in.rs::a_copilot_fork_before_turn_2_resumes_with_turn_1_only`. Likely bug (seen live, B8): the real fork said "can't read this Copilot conversation" (ROAM's real-CLI captures and fixes are pending). The real fork resuming is unproven. Context only onto another machine is refused: `daemon/humatad/tests/fork_remote.rs::a_context_only_fork_onto_another_machine_is_refused`.

8. **I do:** open the fork on this Mac and type a line.

   **I see:** Copilot resumes the fork's conversation and answers.

   **State and events:** the fork has its own conversation id under this Mac's `COPILOT_HOME`.

   **Status:** Works (real shell) for the CLI half with a loopback model only: `//apps/humata2/terminal/provider:copilot_fork_test` (manual, macOS only, not through `humatad2`). Through `humatad2` it is unproven. **Needs the owner's login** for a real model.

9. **I do:** move the parent session from the VM back to this Mac.

   **I see:** Succeeded. The session is on this Mac again. The VM's folder is gone, with a copy in the VM's trash.

   **State and events:** the carry rewrites the Copilot session state again, to name this Mac's session folder. The two sessions (parent and fork) are separate.

   **Status:** Works (real daemon) with a fake agent for the move: `daemon/humatad/tests/move_session.rs::moving_from_this_mac_frees_its_folder_only_after_it_runs_on_box_1` is the opposite direction; the VM to this Mac direction has no test. The carry itself is direction-free: `daemon/sessions/src/carry_roam.rs::a_carried_copilot_conversation_names_no_old_path_and_reads_on_the_new_machine`. Code, no test for the whole path.

10. **I do:** type a line in the session on this Mac.

    **I see:** Copilot resumes the same conversation, with all three turns, and answers.

    **State and events:** the conversation id is the same as at the start of `real/05`.

    **Status:** Code, no test. Unproven with the real CLI. Likely bug (seen live, B5, B7, B10) applies as in step 3.

## Branches

### The VM is stopped when I move

**When:** the VM is off or its link is down.

**I see:** the move is refused at once, saying the machine is offline. Nothing changes.

**Status:** Works (real daemon). `daemon/humatad/tests/move_session.rs::a_move_to_an_offline_machine_or_one_outside_the_channel_is_refused_and_changes_nothing`.

### The copy fails on the VM

**When:** the VM's disk is full (4 GiB is tight with Copilot's tarball), or the folder is not writable.

**I see:** the operation ends Failed, naming the step. The session stays on this Mac, unchanged.

**Status:** Works (real daemon) for a write failure with a fake box: `daemon/humatad/tests/move_session.rs::a_move_whose_copy_fails_leaves_it_running_on_the_source_and_names_the_step`. A real full disk on the VM is unproven.

### The Copilot CLI is missing or too old on the VM

**When:** `real/04` did not run, or the VM's CLI is older than 1.0.112-0.

**I see:** the session cannot resume there. For a fork, it is refused up front naming the version. For a move, the resume fails and the session offers Start fresh with a summary.

**Status:** Works (real daemon) for the fork refusal: `daemon/humatad/tests/continue_in.rs::a_fork_of_a_copilot_session_launched_by_a_cli_older_than_humata_knows_is_refused_up_front`. Works (real daemon) for Start fresh: `daemon/humatad/tests/start_fresh.rs::a_failed_resume_starts_fresh_on_a_new_conversation_and_hears_the_summary`. Whether a move checks the VM's CLI before it starts is Code, no test.

### The host is killed during the move

**When:** the host dies after the copy, or after the agent has moved, and restarts.

**I see:** before the switch the move is undone and the session runs on this Mac. After it the move is finished. Never two agents.

**Status:** Works (real daemon). `daemon/humatad/tests/move_session.rs::a_move_cut_short_after_the_copy_is_undone_at_restart` and `::a_move_cut_short_after_the_agent_moved_is_finished_at_restart`.

### The SSH link drops mid-turn on the VM

**When:** I cut the tunnel while Copilot works.

**I see:** the agent keeps running on the VM. After the link returns, the tab re-attaches and no typed input is replayed.

**Status:** Works (real shell) for the terminal component only: `terminal/host/src/roaming_tests.rs::ssh_outage_and_route_change_reattach_existing_holder_without_input_replay`. The app's own path is unproven. `real/03` and `real/12` cover this.

### A Copilot record Humata does not know

**When:** the real CLI writes a record the adapter does not know.

**I see:** the move is refused before anything is staged, and nothing changes.

**Status:** Works (real daemon). `daemon/sessions/src/carry_roam.rs::a_copilot_conversation_its_adapter_does_not_know_is_refused_before_anything_is_staged`.

### The session has no conversation yet

**When:** I move it before Copilot wrote any state.

**I see:** the folder moves and no conversation is made up.

**Status:** Works (real daemon). `daemon/sessions/src/carry_roam.rs::a_copilot_session_with_no_session_state_yet_carries_its_folder_alone`.

## Expected outcomes

- [ ] The session moves to the VM with the same id, branch, README edit and conversation id, and nothing runs on this Mac.
- [ ] Copilot on the VM resumes the same conversation and shows its earlier turns.
- [ ] Whether the model choice survived is written down.
- [ ] A permission ask from the VM's Copilot reaches the host's Inbox with the machine named.
- [ ] A fork from the VM lands on this Mac with only the turns before the cut. The parent is unchanged.
- [ ] Moving back leaves one agent, on this Mac, in the same conversation.
- [ ] A failed or cut move leaves the session where it was, with its work.
- [ ] The owner's own Copilot home and SSH keys are untouched.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Move to the VM | Works (real daemon), real CLI and real VM unproven | `daemon/humatad/tests/move_session.rs::a_session_moved_box_1_to_box_2_keeps_id_branch_work_and_conversation_and_resumes` |
| 2. Move ends, work kept | Works (real daemon) | same test |
| 3. Resume on the VM | Works (real daemon) for a Desk move on this Mac; Likely bug (seen live, B5, B7) | `daemon/humatad/tests/desk_into_channel.rs::a_copilot_desk_session_moved_into_a_channel_resumes_its_session_state_in_the_new_folder` |
| 4. Model kept? | Code, no test; Not built (no milestone) | the CLI's own state |
| 5. Task and ask on the VM | Works (real daemon) for the Copilot shape; Fixed (B12, D754): `humata session type` presses Enter by default | `daemon/humatad/tests/hooks.rs::each_agent_gets_the_answer_in_its_own_shape` |
| 6. Read the conversation | Likely bug (seen live, B4/B8) | `daemon/humatad/src/session_service.rs:2681` |
| 7. Fork VM to this Mac | Works (real daemon) with fixtures; Likely bug (seen live, B8) | `daemon/humatad/tests/fork_remote.rs::a_copilot_fork_from_box_1_onto_box_2_before_turn_2_carries_turn_1_only` |
| 8. The fork resumes | Works (real shell) with a loopback model only | `//apps/humata2/terminal/provider:copilot_fork_test` |
| 9. Move back | Code, no test for the direction | `daemon/sessions/src/carry_roam.rs::a_carried_copilot_conversation_names_no_old_path_and_reads_on_the_new_machine` |
| 10. Same conversation after the return | Code, no test | unproven |
| Branch: VM stopped | Works (real daemon) | `daemon/humatad/tests/move_session.rs::a_move_to_an_offline_machine_or_one_outside_the_channel_is_refused_and_changes_nothing` |
| Branch: copy fails | Works (real daemon) | `daemon/humatad/tests/move_session.rs::a_move_whose_copy_fails_leaves_it_running_on_the_source_and_names_the_step` |
| Branch: CLI missing or old | Works (real daemon) for the fork refusal and Start fresh | `daemon/humatad/tests/continue_in.rs::a_fork_of_a_copilot_session_launched_by_a_cli_older_than_humata_knows_is_refused_up_front` |
| Branch: host killed | Works (real daemon) | `daemon/humatad/tests/move_session.rs::a_move_cut_short_after_the_agent_moved_is_finished_at_restart` |
| Branch: link drops | Works (real shell), terminal only | `terminal/host/src/roaming_tests.rs::ssh_outage_and_route_change_reattach_existing_holder_without_input_replay` |
| Branch: unknown record | Works (real daemon) | `daemon/sessions/src/carry_roam.rs::a_copilot_conversation_its_adapter_does_not_know_is_refused_before_anything_is_staged` |
| Branch: no conversation yet | Works (real daemon) | `daemon/sessions/src/carry_roam.rs::a_copilot_session_with_no_session_state_yet_carries_its_folder_alone` |

## Gaps and likely bugs

- Not built: built-in `copilot` kind (ROAM batch 3, C4, X2). The VM needs the same `--agents` file.
- Not built: Humata keeps no model, reasoning or context setting, and does not restore the CLI's config on the other machine (no milestone).
- Not built: `ReadConversation` for every kind (`daemon/humatad/src/session_service.rs:2681`).
- Likely bug (seen live, B5): resume looks for the conversation under `--root` as home.
- Likely bug (seen live, B7): resume after a stop kills the correctly resumed agent.
- Likely bug (seen live, B10): attach is refused while the agent resumes.
- Likely bug (seen live, B4/B8): `humata session read` is empty for Copilot.
- Likely bug (seen live, B8): fork says "can't read this Copilot conversation".
- Fixed (B12, D754): `humata session type` presses Enter by default; `--no-submit` leaves the text unsent.
- Likely bug (seen live): Copilot wrote `recentModelIds` to `~/.copilot/config.json` despite "session only". On the VM, use an isolated `COPILOT_HOME`.
- Unproven: no test moves a Copilot session between two machines through the whole daemon, in either direction, and none uses the real VM with real Humata binaries.
- Unproven: the Copilot CLI on a 0.75 GiB, 4 GiB VM.

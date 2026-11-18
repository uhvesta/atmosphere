# Brief: the Real layer (real app, real SSH VM, real agent CLIs, cheapest models)

Read `briefs/_GUIDE.md` first; every rule there holds. These cases go in
`flows/cases/real/`. They differ from the other layers in one way: each case
is meant to be **run by hand or by a later test against the real thing**: the
real desktop app (or the real `humata` CLI and `humatad2`), the real OpenSSH
VM `h2-terminal-c1`, and the real `copilot`, `codex` and `claude` CLIs on the
cheapest model each offers. So a step is **Works** only when a test already
proves it that way. Most steps will be Fakes only, Code, no test, or Not
built, and that is the point: the cases expose what has never run for real.

Owner's goal: "relieve the friction present when working on remote machines
or across remote machines or getting setup with remote machines."

## Extra parts for each Real case

After `## Starting state`, add `## Test setup` with the commands or Bazel
targets the run needs (for example `./terminal-ssh-demo.sh start`), what is
isolated (homes, sockets, state folders), and what needs the owner (a login,
a free screen). Rules for test setup:

- Never touch the owner's real `~/.copilot`, `~/.codex`, `~/.claude`,
  `~/.humata2` or SSH keys. Each run uses its own temp home, and passes
  `COPILOT_HOME`, `CODEX_HOME`, `CLAUDE_CONFIG_DIR` to the daemon (the daemon
  reads these: `daemon/humatad/src/serve.rs:456-466`).
- A step that needs a real model login says so: **Needs the owner's login**.
  Name what the login is (Copilot device login, Codex `login`) and that the
  token must land only in the run's temp home. Offer the alternative the
  terminal experiment uses: a deterministic loopback model server with
  isolated offline state (`terminal/provider/tests/*_resume.py`), which proves
  resume and fork without a login but does not prove a real model.
- Real app runs open windows. The owner's rule is headless only unless told
  the screen is free, so mark app-window steps **Needs a free screen**, and
  say what the headless equivalent is (the `humata` CLI or a
  `desktop/tests/live_*` style test).
- Never run bazel, cargo or git while writing. Only read and search.

## Shared facts (checked by the manager; re-check what you rely on)

- **The VM.** `h2-terminal-c1`: Debian on Lima, 1 CPU, 0.75 GiB RAM, 4 GiB
  disk, no host mounts, no agent forwarding. Root helper
  `terminal-ssh-demo.sh start|provision|status|ssh-config`; docs
  `terminal/platform_test/SSH_DEMO.md`; reach it with
  `ssh -F ~/.lima/h2-terminal-c1/ssh.config lima-h2-terminal-c1`. Read-only
  status: `//apps/humata2/terminal/platform_test:ssh_demo_status_test`.
  `provision` puts a linux_arm64 holder at `/tmp/h2-terminal-holder` (not
  reboot-safe). A 0.75 GiB, 4 GiB VM is tight for the Copilot CLI (~200 MB
  tarball, Node-based); say so where it matters.
- **Real SSH proof today lives only in the standalone terminal experiment**,
  not in `humatad2`: `terminal/lab/tests/clipboard_ssh.rs::ssh_unicode_multiline_color_copy_pasteback_and_focus_are_exact`,
  `terminal/lab/tests/vertical_resize_ssh.rs::ssh_vertical_resize_changes_real_pty_rows_without_restarting_shell`,
  `terminal/lab/tests/neovim.rs::neovim_colors_mouse_keyboard_resize_and_resume`,
  `terminal/lab/tests/interactive.rs::actual_zsh_commands_clipboard_movement_and_twelve_thousand_history_lines`,
  `terminal/session/tests/ssh.rs::ssh_terminals_keep_their_process_after_transport_and_app_restart`,
  `terminal/session/tests/ssh.rs::killing_the_ssh_frontend_reaps_its_tunnel_and_keeps_the_remote_shell`,
  `terminal/host/src/roaming_tests.rs::ssh_outage_and_route_change_reattach_existing_holder_without_input_replay`,
  `terminal/host/src/agent_resume_ssh_tests.rs::ssh_confirmed_child_sigkill_automatically_resumes_exact_saved_session`
  and `::ssh_enabled_resume_does_not_guess_transport_or_holder_death`. These
  are manual targets. They count as **Works (real shell)** only for the
  terminal component; say each time that the app's own path (desktop app →
  `humatad2` → provider SSH → `humatad2 --role machine` → holder) is unproven.
  See `terminal/lab/HANDOFF.md`.
- **Machines in daemon tests sit behind a fake `ssh`** (`daemon/machines/tests/fake_ssh.rs`,
  `daemon/humatad/tests/common/remote_rig.rs`). One test uses a real Lima VM
  with fake Humata binaries: `daemon/machines/tests/lima.rs::a_lima_vm_is_created_set_up_stopped_and_deleted`
  (its own VM `humata2-machines-e2e`, needs `limactl` and network). The
  provider can take an SSH config file (`daemon/machines/src/link.rs:106-153`,
  `ssh.ssh_config`). `macro/08-first-run-and-adding-a-machine.md` has the
  add-machine evidence; reuse it, don't redo it.
- **TS-0, TS-1, TS-2** (SPEC.md ~2657-2690): the app installs its own
  binaries on the host and, on add and every connect, `humatad2`,
  `humata2_holder` and `humata` into `~/.humata2/bin` on the machine, from
  binaries embedded for every platform. Check `daemon/machines/src/install.rs`,
  `bundle.rs`, and whether `app/app.bzl` really embeds linux_arm64 builds.
- **TS-3** scripted sync rules with `before`/`after` commands on connect:
  `daemon/machines/src/sync_rules.rs` only parses and checks them ("Nothing
  here runs a command"); nothing in `daemon/humatad/src` runs them. The
  nearest existing machinery to MACHINESCRIPTS.
- **Agent kinds.** Built in: `claude` and `opencode` only
  (`daemon/sessions/src/agents.rs` ~340-410). Codex and Copilot run only as
  custom kinds from an `--agents` JSON file given to `humatad2 serve`, with
  `conversation_format` `codex-rollout` / `copilot-session-state`. Not built
  as built-ins: ROAM batch 3, C4, X2 (`workflow/milestones/ROAM.md`).
  Conversation adapters exist: `daemon/conversation/src/{claude,codex,copilot}.rs`.
  Codex's format "waits on the layout fixtures (CV-7)"
  (`daemon/conversation/src/lib.rs:52-53`); Copilot's is pinned to CLI
  1.0.112-0 and later.
- **Codex/Copilot evidence on a real daemon uses fixture transcripts, not the
  real CLIs:** `daemon/humatad/tests/continue_in.rs` (a_codex_/a_copilot_
  session lists its turns, continues in another channel, fork before turn 2
  resumes with turn 1 only, unknown record refused),
  `daemon/humatad/tests/fork_remote.rs::a_codex_fork_from_box_1_onto_box_2_before_turn_2_carries_turn_1_only`
  and `::a_copilot_fork_from_box_1_onto_box_2_before_turn_2_carries_turn_1_only`,
  `daemon/sessions/src/carry_roam.rs::a_carried_copilot_conversation_names_no_old_path_and_reads_on_the_new_machine`,
  `daemon/humatad/tests/desk_into_channel.rs::a_copilot_desk_session_moved_into_a_channel_resumes_its_session_state_in_the_new_folder`,
  `daemon/humatad/tests/codex_trust.rs`, `daemon/humatad/tests/hooks.rs::each_agent_gets_the_answer_in_its_own_shape`.
  Count these as Works (real daemon) for the daemon half and say the real CLI
  half is unproven.
- **Real CLIs were proven only in the terminal experiment**, against a
  deterministic loopback model server with private state: Codex 0.169.0,
  Claude Code 2.1.325, Copilot 1.0.112-0 resume and fork
  (`terminal/provider/BUILD.bazel` targets `codex_resume_test`,
  `claude_resume_test`, `copilot_resume_test`, `copilot_fork_test`,
  `<provider>_fork_test`; manual, macOS only, use the installed CLI) and
  `terminal/host/BUILD.bazel` `agent_resume_codex_test`. Works (real shell)
  for that component only; not through `humatad2`.
- **Fork.** Humata forks by itself: `daemon/sessions/src/fork.rs`
  (`ForkCopy::CodeAndContext | CodeOnly | ContextOnly`, `fork_conversation`)
  copies the conversation files through the adapters (new id, new folder,
  metadata rewritten); it does not script the CLIs. Daemon tests:
  `daemon/humatad/tests/fork.rs` (code_and_context_copies_the_work_and_the_conversation,
  code_only_copies_the_work_into_a_fresh_conversation, context-only,
  with another agent, cut-at-restart tests), `fork_remote.rs`.
- **REALFLOW run (2026-11-16)**: a tester drove the real Copilot CLI headless
  through the `humata` CLI and a real `humatad2` (custom kinds from
  `--agents` JSON, `--default-agent copilot`). Bugs it found, to reflect as
  **Likely bug (seen live)** in the right steps:
  1. No built-in `codex` or `copilot` kinds; they need an `--agents` file.
  2. Resume looks for the conversation under `--root` instead of `$HOME`
     (or the CLI home), so a resumed Copilot/Codex session can't find it.
  3. `humata session read` (the conversation read) is empty for Copilot.
  4. A stopped session shows Idle, not Stopped.
- **The Copilot CLI is already fetchable by Bazel**: `apps/humata/copilot/copilot.bzl`
  is a `repository_rule` pinned by version (`1.0.110-1`) and SHA-256 per
  platform including `linux_arm64`, used in `MODULE.bazel` as
  `@copilot_cli_linux_arm64//:copilot` (visibility only
  `//apps/humata/copilot`). Note: 1.0.110-1 is older than the 1.0.112-0 the
  Copilot adapter needs, so the pin must move. There is **no** Codex rule.
- **Cheapest models**, chosen in the CLI by real keystrokes: Copilot
  `/model` → Claude Haiku 5.5, low reasoning, smallest context, this session
  only; Codex `/model` → GPT-6 Luna, low reasoning. Humata types these keys
  into the agent's terminal; it has no model picker for agent CLIs of its
  own. Say what Humata records (nothing about the model, probably) and
  whether a moved/forked/resumed session keeps the model (it is the CLI's
  own session setting; unproven).
- **MACHINESCRIPTS** (lane being specced; no spec yet): scripts that run when
  a machine is registered, when its link reconnects, and when a session
  arrives on or leaves it. Examples from the owner: "on registration open
  copilot and send `/restart`", or run an arbitrary bash script. Everything
  in it is **Not built (MACHINESCRIPTS)**. Write the case from the owner's
  words and these principles from SPEC/DECISIONS: results show where they
  block an action, not as an always-on status (no unfixable detection);
  failures go to the Inbox with full decision context; settings are portable
  TOML in `~/.humata2/config/*.toml`, secrets never in files; scripts run on
  the machine, like TS-3's `before`/`after`.

## Cases

| File | Title idea | Covers |
|---|---|---|
| `real/01-the-vm-becomes-a-machine.md` | The SSH VM is added as a machine in the real app and gets Humata's tools | Add `h2-terminal-c1` through Add machine with its SSH config, host key, TS-1 install of `humatad2`/holder/`humata` into `~/.humata2/bin`, check, machine page; branches: VM stopped, wrong key, too little disk, re-add after `provision`. |
| `real/02-a-shell-on-the-vm-in-the-app.md` | A shell on the VM in the real app takes every key, copy, paste and resize | Open a shell tab on the VM; vim and less; Esc, ⌃C, arrows, ⌥ keys; copy and paste both ways, multi-line paste confirm; resize window and split. |
| `real/03-the-vm-link-drops-and-the-app-restarts.md` | A dropped SSH link reconnects without replaying input, and an app restart keeps live holders | Cut SSH (kill the tunnel, `limactl` network stop is out: never restart the shared VM), type while down, reconnect, no replay; quit and reopen the app, holders keep running. |
| `real/04-agent-clis-on-the-vm-for-a-test.md` | A test puts pinned Copilot and Codex CLIs on the VM in their own homes, and takes them away after | Bazel repo rule/module extension pinned by sha (reuse `copilot.bzl`, add a Codex one), copy into the VM under a test folder, isolated `COPILOT_HOME`/`CODEX_HOME`, login (owner's or loopback), cleanup; branches: sha mismatch (message must not just say re-pin), VM full, no network. |
| `real/05-copilot-on-the-cheapest-model-on-this-mac.md` | A Copilot session on Claude Haiku 5.5 does a tiny task, asks through the Inbox, and resumes after a stop | Start in a channel on this Mac, `/model` keystrokes, tiny task, permission Ask in the Inbox, turn end, diff, stop/start/resume, fork on this Mac. |
| `real/06-copilot-moves-to-the-vm-and-back.md` | The same Copilot conversation moves to the VM, forks back to this Mac, and moves home | Move to VM, continue same conversation, model kept?, fork onto other machine, move back. |
| `real/07-codex-on-the-cheapest-model-on-this-mac.md` | Codex on GPT-6 Luna, same as 05 | Codex trust step (`codex_trust.rs`), hooks. |
| `real/08-codex-moves-to-the-vm-and-back.md` | Codex, same as 06 | CV-7 risk. |
| `real/09-three-agents-in-one-channel-across-machines.md` | Claude, Codex and Copilot share one channel across this Mac and the VM | Mixed sessions, Inbox from all three, board/files, one moves while others run, continue-in another agent. |
| `real/10-a-script-runs-when-a-machine-is-registered.md` | MACHINESCRIPTS on registration: a bash script, and "open copilot, wait until ready, type /restart" | Configure, add the VM, script runs, result shown; failures: script fails, times out, machine offline, agent not installed; re-add. |
| `real/11-scripts-on-reconnect-and-session-arrival.md` | MACHINESCRIPTS on reconnect, and when a session arrives or leaves | Reconnect script, session-arrives (after move/fork), session-leaves; ordering with the move steps; failure keeps the session safe. |
| `real/12-the-link-drops-and-every-agent-resumes.md` | A dropped link (not a shutdown) leaves every agent running, and reconnect re-attaches with no lost or replayed input | Network/SSH cut, Mac sleep, Wi-Fi change; Claude, Codex, Copilot on the VM; one mid-turn, one with an Inbox Ask pending during the cut. |
| `real/13-the-vm-and-this-mac-reboot.md` | After the VM powers off or reboots, and after this Mac reboots, each session resumes its exact conversation with a restart report | Power off/reboot of the box (a dedicated throwaway VM, never `h2-terminal-c1`'s shared fixture without the owner), Mac reboot; compare with Stop for the night (`flows/08`, `macro/09`, `agents/13`). |
| `real/14-forking-every-agent-kind.md` | Every agent kind forks on this Mac and onto the VM, by code and context, code only and context only, and from an earlier turn | Claude, Codex, Copilot; the fork resumes in the real CLI and the parent is unchanged. |

## REALFLOW final results (2026-11-16, supersedes the four bugs above)

Real CLIs driven headless through the `humata` CLI and a real `humatad2`.
Tag these steps **Works (real CLI, manual REALFLOW run)** where it worked —
it is a manual run, not a test, so say no test pins it — and **Likely bug
(seen live, B<n>)** where it failed. Fixes for B1–B6 and B14 are queued, not
built; B7–B13 are routed to lanes.

- **Copilot** (`/model` → Claude Haiku 5.5, low, 100K context, "for this
  session"): WORKED: start (a folder-trust prompt comes first), the task,
  three Inbox asks approved with `humata inbox approve`, idle detection,
  `humata diff`. FAILED: B4/B8 `humata session read` empty; B6 a stopped
  session shows Idle; B5 resume looks under `--root` as home; B7 resume
  after stop is treated as an interruption and kills the correctly resumed
  agent; B10 attach refused while resuming; B8 fork says "can't read this
  Copilot conversation" (ROAM's real-CLI captures and fixes pending).
- **Codex** (`/model` → GPT-6 Luna, low, `s` = this session only; a "Hooks
  need review" prompt → Continue without trusting): the task ran; its prompt did not name the repo folder, so it wrote beside
  the clone (not a gap: Humata never tells agents where the repo is, so every
  task prompt in these cases names the clone folder, e.g. "In the api/
  folder, ..."); B9 resume fails because Humata never learns
  Codex's real session id (dated rollout path); fork untested.
- **Both:** B12 `humata session type` never presses Enter.
- **Home leak:** Copilot wrote `recentModelIds` to `~/.copilot/config.json`
  despite "session only". Every case must use an isolated
  `COPILOT_HOME`/`CODEX_HOME` with a copied login, or mark the step as
  touching the owner's home.

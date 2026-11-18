# Brief: the Verify layer (what landed since the Real layer)

Read `briefs/_GUIDE.md` first; every rule there holds. These cases go in
`flows/cases/verify/` once the VERIFY coach merges and numbers them. Writers
draft into the session scratchpad the coach names, never into the tree.

They differ from the other layers in one way: each case is **reproduced
headless** by a tester right after it is written, and the result is recorded
in `flows/cases/results/`. So each case adds, after `## Starting state`:

## Reproduce headless

- The harness that drives it, by Bazel target and test file: a desktop
  `native_test` view or `live_flows_test` file (`desktop/tests/live_flows/*.rs`),
  a `daemon/humatad/tests/*.rs` target, a `cli/tests/*.rs` target, or the
  `humata` CLI against a throwaway `humatad2` (own `--root`, own `HOME`).
- Exact steps to run, and what to capture as evidence: a `.new.png` render,
  CLI output and exit code, or a log excerpt.
- What can't be done headless (a real window, a real model login, the real VM
  disk) and the nearest headless equivalent.

Rules for the run that the case must respect: no windows on the owner's
screen; real agent CLIs only with an isolated `HOME`, `CODEX_HOME`,
`COPILOT_HOME`, `COPILOT_CACHE_HOME` and `CLAUDE_CONFIG_DIR`; never the
owner's `~/.claude`, `~/.codex`, `~/.copilot` or git config; the VM
`h2-terminal-c1` is never stopped or rebooted. The VM has 1.3 GiB free and
Add machine needs 2 GiB, so adding the real VM fails with DiskLow until the
owner grows its disk (D694): that is blocked by the environment, not a bug.

## Areas

| Area | Covers | Read first |
|---|---|---|
| A fork-resume | Fork and resume for Claude, Codex and Copilot, after a stop and a restart; `humata session read` | `workflow/milestones/AGENTKINDS.md`, D670–D675, `daemon/humatad/tests/{fork,context_fork,fork_remote}.rs`, `real/14` |
| B attach-type | `humata attach` while a session resumes (D672); `humata session tabs`; `humata session type` submits once (w8) | AGENTKINDS w1/w6/w8, `cli/src/commands/{sessions,looking}.rs`, `cli/tests/sessions.rs` |
| C scripts-triggers | Machine scripts on added, connected, upgraded (D689), session-arrived, session-leaving; Inbox on failure, Retry | `workflow/milestones/MACHINESCRIPTS.md`, D680–D689, `daemon/machines/src/machine_scripts.rs`, `daemon/machines/tests/machine_scripts.rs`, `real/10`, `real/11` |
| D scripts-ui | Settings › Machines › Scripts, the machine page Scripts section, Run now, `humata machine script list|check|run|log` | MACHINESCRIPTS, `features/hosts/src/scripts.rs`, `features/settings/tests/machine_scripts.rs`, `cli/tests/machine_scripts.rs` |
| E agents-settings | Settings › Agents: switches, Install when missing (off), pickers show only enabled, no third-party install unless opted in | `workflow/milestones/AGENTSET.md`, D720–D729 |
| F migrate-this-mac | The inline "Migrate This Mac" on a blocked Add machine (stale machine tools, errors.proto kind 61) | `daemon/humatad/src/serve.rs` (stale platforms), `features/app/tests/upgrade_reconnect.rs`, `workflow/milestones/UPGRADE.md`, MACHINESCRIPTS w4c |
| G pr-drafts-archive | Archiving a review with unsubmitted PR drafts asks, keeps the drafts, restores them (D513) | DECISIONS D513, PR-17, `workflow/milestones/PR.md`, `daemon/humatad/tests/pr_*.rs` |
| H, I flows-gaps | Steps in `flows/01`–`09` that no case covers yet (DOCV doc versions, the repo library, no folder picking, and others) | `flows/*.md`, `flows/cases/README.md` index |

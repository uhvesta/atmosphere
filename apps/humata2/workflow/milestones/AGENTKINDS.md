# AGENTKINDS: Codex and Copilot start, resume and fork like Claude

- **Status:** open (2026-11-16)
- **Depends on:** HOOKS (agent hooks), ROAM (conversation adapters; r3_copilot for B8)
- **Layer:** L3 · **Lane:** daemon + CLI

## Outcome
The installed app starts, resumes and forks Codex and Copilot sessions with no setup, like Claude.

## Exit criterion
- Codex and Copilot are built in after Claude. AgentService lists them.
- A Codex or Copilot session resumes and forks its own conversation, after a stop and after a restart, and `humata session read` shows its entries.
- `humata attach` without `--tab` opens the agent tab, and `humata session tabs` lists the tabs.
- The REALFLOW bugs B3–B10, B12 and the Copilot edit label are fixed with tests. B11 was dropped by the owner (2026-11-16).

## Lane (written by the manager)

Manager: AGENTKINDS (Opus 5.5), opened 2026-11-16. Delivery is anchor-only `--check` scripts in the session scratchpad `agentkinds/`; main applies them in order.

### Design notes
- D670: Codex's conversation is found in its folder; Copilot is pinned with `--session-id`.
- D671: a resume the person asks for isn't told; submit is its own keyed step.
- D672: this machine is the default machine; attach waits for a resuming tab.
- D673: Copilot hook and permission records known from the CLI's schema (X2).
- D674: an error chain names each cause once.
- D675: the history mirror learns a Codex conversation itself.
- B5: the agents' `{home}` is `$HOME`, not `--root` (`serve::agent_home`).
- B11 dropped: no instructions file, no launch-flag changes, no "Outside the repos" group.

### Tasks and scripts (apply order)
| Script | What | State |
|---|---|---|
| w1_attach_tabs.py | attach picks the agent tab; `session tabs`; CLI.md | sent |
| w2_mirror_kinds.py | mirror reads Codex and Copilot lines | sent |
| w3_stop_status_edit_label.py | B6 stopped reads exited; file edits are Files cards | sent |
| w4_agent_service.py | AgentService ListAgents | sent |
| w5_resume_submit.py | B7, B12 | sent |
| w6_attach_resume_machine.py | B10, B3 (after w1) | sent |
| w8_type_submit_once.py | Type never double-submits | sent |
| m1_codex_copilot_builtin.py | built-in Codex and Copilot, B9, B5 | sent |
| d1_agentkinds_docs.py | D670–D672, this file, claims | sent |
| w9_fork_options.py | fork takes agent, before-turn, copy mode in CLI and MCP (after w1, w6) | sent |
| w11_fork_resume_real.py | real CLIs resume an adapter fork (manual, TERMINAL_*_BIN) | sent |
| w10_read_conversation.py | ReadConversation for every kind; CV-7 comment (after m1) | sent |
| w12_copilot_records.py | B8 hook and permission records; error chain once | sent |
| w13_history_learns.py | history mirror learns Codex; CLI.md conversation row (after w10) | sent |
| d2_agentkinds_docs.py | D673–D675, this table | sent |

(5), the Copilot and Codex test pins, belongs to MACHINESCRIPTS (`testing/agent_clis`).

### Claims
Hunks only, through scripts: `daemon/sessions/src/agents.rs`, `daemon/humatad/src/{restart,serve,terminal,holders,agent_service}.rs`, `daemon/humatad/src/session_service{,/turns,/fork,/hooks,/start_fresh}.rs`, `daemon/revive/src/{agent,reconcile,lib}.rs`, `daemon/mirror/src/transcript.rs`, `cli/src/{resolve,args,error,daemon}.rs`, `cli/src/commands/{hosts,sessions,mod}.rs`, `daemon/humatad/src/{status,mcp,history_service}.rs`, `daemon/humatad/src/history_service/source.rs`, `daemon/humatad/src/session_service/conversation_read.rs` (new), `daemon/conversation/{BUILD.bazel,src/copilot.rs,src/lib.rs,tests/copilot.rs,fixtures/fork_*}`, `daemon/mcp/src/tools.rs`, `cli/tests/sessions.rs`, `api/{CLI,MCP}.md`, this file. SEAMs are marked in each script.

### Worker log
- w1–w6, w8: done and sent. w7 (B11, B13) stopped when B11 was dropped; B13 is deferred.
- w9–w13: done and sent. Nothing compiled in the lane; main builds and tests.

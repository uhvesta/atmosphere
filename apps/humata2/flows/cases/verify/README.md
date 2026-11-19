# Verify cases

Index of the Verify-layer cases, in area order. Each case has a `## Reproduce headless` section, and a tester reproduces it headless after it is written. Results go in `flows/cases/results/`.

| # | Case | Area | Draft(s) |
|---|---|---|---|
| 01 | Each agent kind resumes its exact conversation after a stop and a daemon restart | A fork-resume | A-1 |
| 02 | Each agent kind forks with code and context, code only, context only, or from an earlier turn | A fork-resume | A-2 (trimmed) |
| 03 | `humata attach` picks the agent's tab, and an attach during a resume waits for it | B attach-type | B-1 |
| 04 | `humata session type` submits its text once, including after a retry and a resume | B attach-type | B-2 |
| 05 | A rule on added, connected or upgraded runs once per link-up, and a failed step stops its rule | C scripts-triggers | C-1 |
| 06 | A moved session's arrival rule types into it, and its leaving rule runs first on the old machine | C scripts-triggers | C-2 |
| 07 | Machine scripts are listed in Settings and on the machine page, and Run now updates by push | D scripts-ui | D-1 |
| 08 | `humata machine script` checks rules, runs one and reads its log, and an agent can't run one by default | D scripts-ui | D-2 |
| 09 | Turning Copilot off keeps it out of every new start and leaves its sessions alone | E agents-settings | E-1 |
| 10 | With Install when missing off, a missing CLI is refused; with it on, one start installs once | E agents-settings | E-2 |
| 11 | A Linux add on stale tools is refused until This Mac is migrated, from the dialog or the CLI | F migrate-this-mac | F-1, F-2 (merged) |
| 12 | Adding the real VM stops at the disk check, which is an environment block, not a bug | F migrate-this-mac | F-3 |
| 13 | Archiving a review keeps its unsubmitted PR drafts, and restoring it brings them back | G pr-drafts-archive | G-1 |
| 14 | Each commit and each Save version makes a numbered version, and History says who made it | H flows-gaps | H-1 |
| 15 | Stepping back to an old version reads it as it was, and Restore adds a version | H flows-gaps | H-2 |
| 16 | Closing a Desk session that holds files asks first | H flows-gaps | H-3 |
| 17 | A plain shell on a machine runs as its own Terminal session that never reads Working | I flows-gaps | I-1 |
| 18 | Adding a machine through a provider, logging in by hand, leaves a machine that starts unattended | I flows-gaps | I-2 |
| 19 | A merged pull request says so and offers Archive, while the other pull request stays open | I flows-gaps | I-3 |
| 20 | Claude Code and Codex pass the Copilot cases, and Humata reads each through its own structured channel | E agents-settings | AGENTS2 F1 |

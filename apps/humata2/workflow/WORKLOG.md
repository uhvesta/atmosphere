# Work log

The written record of who is working on what. Read top to bottom when
starting or resuming (see `workflow/roles/MAIN.md` › Starting or resuming).
Each agent edits only its own lane; the main agent edits the rest.

## Resume here

- **State:** setting up the DAG walk. R1 (Fable) is reviewing SPEC,
  OVERVIEW and the goldens for cohesion, goal milestones and a parallel DAG.
- **Next:** when R1 reports, a Fable advisor on how to edit the code; an
  Opus 5.5 safety pass on committed state; a Fable writes the DAG-walk
  prompt; then `workflow/DAG.md` is written and managers start.
- **Lane limit:** 3 managers at once (one bazel; revisit after the first
  build gate).
- **Last pushed:** a49bb8ba (origin/main).

## Roster

| Agent | Model | Role | Lane / task | Started | Status |
|---|---|---|---|---|---|
| main | Opus 5.5 | main agent | — | 2026-11-15 | active |
| minion C1 | — | restyle app to goldens (launched by the owner) | N1 restyle; uncommitted work in `clients/`, `features/{app,session}`, `ui/`, `desktop/tests/native*` | 2026-11-15 | active, outside this workflow |
| minion R1 | Fable | reviewer | cohesion, goal milestones, DAG; report at scratchpad `r1-review.md` | 2026-11-15 | done |
| minion A1 | Fable | advisor | how to edit code and docs, given R1; report at scratchpad `a1-advice.md` | 2026-11-15 | running |
| minion S1 | Opus 5.5 | safety pass | pushed state, inventory, WIP backup ref `refs/wip/2026-11-15-safety` | 2026-11-15 | running |

## Claims

Files a lane is editing. No two lanes claim the same file.

| Lane | Paths |
|---|---|
| C1 | `clients/src/{daemon,fixture,workspace}.rs`, `clients/tests/fake_daemon.rs`, `desktop/tests/native.rs`, `desktop/tests/__snapshots__/native/`, `features/app/src/lib.rs`, `features/session/src/lib.rs`, `ui/BUILD.bazel`, `ui/src/{lib,overlays,pages,session_tabs,shell}.rs`, its OVERVIEW pickup block |

## Lanes

Each lane's plan, tasks and workers live in its milestone file
(`workflow/milestones/<ID>.md` › Lane). Here, one line per active lane:

| Lane | Milestone file | Manager | Workers running | State |
|---|---|---|---|---|

Old template, kept for reference:

```text
### <lane>: <milestone> (manager: minion <lane>, Opus 5.5)
- Goal / exit criterion:
- Design notes:
- Tasks:
  - [ ] T1 <task> · worker <name> (Sonnet, medium) · files … · status
- Waiting on: (main agent / owner / another lane)
- Last build gate: <time> pass|fail (targets)
- Ready to commit: no | yes: <summary for the reviewer>
```

## Build gates

| When | Lanes | Targets | Result |
|---|---|---|---|

## Event log

Newest last. One line per event: spawns, reports, gates, commits, decisions.

- 2026-11-15 main: wrote `workflow/` (WORKFLOW, roles, WORKLOG); started R1.
- 2026-11-15 R1 done: 10 cohesion problems, 19 goal milestones, layered DAG (critical path LOOK → FL → MACH → SAFE → LINUX → ANYHOST → V1; 2 building lanes + 1 docs lane). Started A1 (advice) and S1 (safety) in parallel.

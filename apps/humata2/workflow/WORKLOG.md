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
| minion A1 | Fable | advisor | how to edit code and docs, given R1; report at scratchpad `a1-advice.md` | 2026-11-15 | done |
| minion S1 | Opus 5.5 | safety pass | pushed state, inventory, WIP backup ref `refs/wip/2026-11-15-safety` | 2026-11-15 | done |
| minion J1 | Fable 5.2 (high) | judge | rule on R1 (and A1) findings; tidy SPEC, DECISIONS, OVERVIEW (not C1's block, not goldens); change log at scratchpad `j1-changes.md` | 2026-11-15 | done |
| minion P0 | Opus 5.5 | pre-manager | docs sweep left after J1; mechanical split of hot files C1 isn't editing (panels.rs, native.rs deferred) | 2026-11-15 | running |
| minion D1 | Fable | DAG writer | `workflow/DAG.md` + `workflow/milestones/<ID>.md` from R1, A1, J1 | 2026-11-15 | done |

## Claims

Files a lane is editing. No two lanes claim the same file.

| Lane | Paths |
|---|---|
| C1 | `clients/src/{daemon,fixture,workspace}.rs`, `clients/tests/fake_daemon.rs`, `desktop/tests/native.rs`, `desktop/tests/__snapshots__/native/`, `features/app/src/lib.rs`, `features/session/src/lib.rs`, `ui/BUILD.bazel`, `ui/src/{lib,overlays,pages,session_tabs,shell}.rs`, its OVERVIEW pickup block |
| P0 | `LAYOUT.md`, `api/{BOUNDARIES,MCP,GRPC}.md` ("Always let" → "Always allow"); `features/app/src/` (lib.rs and new sibling files); `clients/BUILD.bazel`, `clients/src/{daemon,fixture}.rs`, new `clients/src/{daemon,fixture}/`; `ui/src/overlays.rs` and new `ui/src/overlay_*.rs` (all clean in git status when claimed; not panels.rs, native.rs, ui/src/lib.rs) |

## Lanes

Each lane's plan, tasks and workers live in its milestone file
(`workflow/milestones/<ID>.md` › Lane). Here, one line per active lane:

| Lane | Milestone file | Manager | Workers running | State |
|---|---|---|---|---|
| P0 | none (pre-manager step) | minion P0 | Sonnet split workers, one file each | LAYOUT.md sweep; splitting features/app lib.rs, clients daemon.rs and fixture.rs, ui overlays.rs |

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
- 2026-11-15 S1 done: HEAD = origin/main, LFS complete, no half states, no secrets in the last 20 commits. C1's 32 uncommitted files backed up to `refs/wip/2026-11-15-safety` (67a307ea, pushed). Delete that ref after C1 commits. Two untracked `.claude/handoffs/*.md` stay uncommitted (`.claude/` is never committed).
- 2026-11-15 main: milestone files (`workflow/milestones/`) added on owner request; agents read their milestone file and its pointers, not the big docs.
- 2026-11-15 main: started J1 (judge, Fable high) on owner request. Order: owner directions > goldens > SPEC; golden changes listed for the owner, not made.
- 2026-11-15 main: A1 done (scratchpad `a1-advice.md`), forwarded to J1. Added the `dag-walk` skill (`tools/skills/dag-walk/`, linked locally at `.claude/skills/dag-walk`) and `workflow/CONFIG.md`.
- 2026-11-15 main: owner asked: after J1, an Opus 5.5 agent (P0) does A1's pre-manager work: C1 landed first (C1 is active and commits itself; main waits for its files to go clean), docs sweep left after J1, one-time split of the hot shared files. Then dag-walk plan step 3 (DAG.md + milestone files), then dag-walk resume.
- 2026-11-15 main: C1 landed its work (9158b063, eb4cb93f, pushed). Pre-manager step (a) done. Safety ref kept for now (it also holds the other session's ui-prototype work).
- 2026-11-15 main: J1 done; committed its SPEC/DECISIONS/OVERVIEW edits (C1's pickup block staged from HEAD). D103–D112 Provisional, D113 frame changes await the owner.
- 2026-11-15 main: J1 committed (ba1a357e). Started P0 (Opus) and D1 (Fable, DAG files). C1 active again on Changes wrapping (ui panels/shell/lib, native.rs).
- 2026-11-15 main: D1 done: DAG.md + 26 milestone files. Scripted syncs moved into MACH (D49). Skill honours `*` edges. Next: P0 report → gate → commit → dag-walk resume (FL-D daemon lane, FRAMES1 docs lane first).

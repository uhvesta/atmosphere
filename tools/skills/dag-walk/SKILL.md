---
name: dag-walk
description: Run long, multi-milestone work as a DAG walked by agent lanes in one checkout. The main agent owns the DAG, builds, tests and git; one Opus manager per open milestone plans it and has Sonnet workers write the code; a work log makes it resumable. Use for "init/plan/resume/gate/close the DAG", "walk the milestones", "resume the DAG", or any long horizontal push across many milestones.
---

# dag-walk

A way to run many milestones in parallel without losing track. All state
lives in the project's `workflow/` folder, so any session can pick it up.

Arguments: `init`, `plan`, `resume` (default), `gate`, `close <ID>`.
Which project: the `workflow/` folder named in the argument, else the one
named in CLAUDE.md, else ask.

## The folder

```text
workflow/
  CONFIG.md          project settings: build/test/commit/push commands, never-touch paths, lane limit, models
  WORKFLOW.md        the loop and the shared rules (from templates/WORKFLOW.md)
  roles/MAIN.md      the main agent's role
  roles/MANAGER.md   a milestone manager's role
  roles/WORKER.md    a worker's role
  DAG.md             milestones, edges, layers, status, critical path
  milestones/<ID>.md one milestone: outcome, exit, pointers, out of scope, and its Lane
  WORKLOG.md         resume notes, roster, claims, lanes, gates, event log
```

The templates are in this skill's `templates/` folder. Read only the ones the
current mode needs.

## Who does what

- **Main agent (you):** the only one that runs builds and tests, touches git
  or talks to the owner. Owns DAG.md, the top half of each milestone file,
  the WORKLOG roster and event log.
- **Manager:** one fresh agent per open milestone (model from CONFIG.md,
  default `opus`). Plans in
  its milestone file's Lane section, claims files, spawns workers, checks
  their diffs, fixes from diagnostics.
- **Worker:** model and effort from CONFIG.md (default `sonnet`, `medium`). One task, only the files
  it was given. No builds, no git.

## Modes

### init
1. Create `workflow/` from `templates/` (WORKFLOW, roles, WORKLOG, the
   milestone README). Fill `workflow/CONFIG.md` from the project's own
   instructions (CLAUDE.md, AGENTS.md, memory): build/test commands, how to
   commit and push, paths never to touch, lane limit (default 3, fewer if
   builds share one server), models.
2. Log `init` in WORKLOG. Commit only `workflow/` (per CONFIG).

### plan
Turn the project's specs and work list into a DAG of goal milestones.
1. **Review:** spawn a reviewer (strong model, read-only) on the spec, the
   work list and any approved designs: cohesion problems, are milestones
   goals (a user-visible outcome you can demo) or chores, and a layered DAG
   with the critical path and a realistic lane count. Report to a scratch
   file.
2. **Judge:** spawn a judge (strong model, high effort) that rules on the
   findings and fixes the docs. Precedence: the owner's explicit directions
   (newest first) > approved designs > spec text. It never edits approved
   designs or code; it lists design changes for the owner and logs anything
   it had to choose as a Provisional decision.
3. **Write the DAG:** DAG.md (nodes, edges with reasons, layers, critical
   path, Mermaid block) and one `milestones/<ID>.md` per node from
   `templates/milestone.md`, pointers only: requirement IDs to grep,
   design frames, decisions, code to start from, out of scope.
4. Ask the owner, in one wave, the design changes and Provisional decisions
   with recommendations. Proceed on reversible picks meanwhile.

### resume (default)
1. Read `workflow/WORKLOG.md` (Resume here, Roster, Claims, Lanes), then
   CONFIG.md. Compare with `git status` and the last commits; any change in
   the tree not in a lane's claims is someone's unlogged work: find out
   whose before acting.
2. List running agents. A lane whose manager is gone gets a fresh manager
   pointed at its milestone file; its Lane section says where it stopped.
3. Open every milestone in DAG.md whose dependencies are done, up to the
   lane limit (an edge marked `*` only blocks closing: the milestone may
   open and build before it, but can't close until it is done): spawn a fresh manager with the brief in
   `templates/roles/MAIN.md` › Spawning a manager. Log it in the roster
   before it starts.
4. Keep going: brief-review each batch a manager reports, run `gate` when
   enough has landed, `close` milestones that pass.

### gate
1. Collect the targets from every lane that reported work since the last
   gate. Run one batched build and test with CONFIG's commands, in the
   background.
2. Split the diagnostics per lane by file ownership (Claims) and send each
   manager only its own. Log the gate (time, lanes, targets, result).

### close <ID>
1. Check the exit criterion holds and the lane's targets passed the last
   gate. Every `*` dependency must be done too.
2. Commit only that lane's claimed paths with CONFIG's commit rules, push,
   verify. Release its claims.
3. Mark it done in DAG.md and its milestone file (with the commit), end its
   manager, log it, and open whatever it unblocked (resume step 3).

## Always

- Keep WORKLOG current: if it isn't logged, a resumed session won't know.
- Short status lines to the owner; decisions in waves; never claim done
  without a passing gate.
- Agents read their milestone file and its pointers, never whole specs.

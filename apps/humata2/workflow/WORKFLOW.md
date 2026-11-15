# How we work: walking the milestone DAG

Owner direction 2026-11-15. Every agent working on Humata 2 milestones reads
this file first, then its role file in `workflow/roles/`, then
`workflow/WORKLOG.md`. A manager then reads only its milestone file,
`workflow/milestones/<ID>.md`, and the pointers in it: no agent reads
SPEC.md, OVERVIEW.md or DECISIONS.md whole.

## The shape

```text
 owner
   │  signs off plans, goldens, decisions (asked in waves)
   ▼
 main agent (one, in the owner's session)            roles/MAIN.md
   │  owns the DAG, the WORKLOG, bazel, commits and pushes
   │  spawns one manager per open milestone
   ▼
 milestone managers (Opus 5.5, one per milestone)    roles/MANAGER.md
   │  plan the milestone's implementation, split it into tasks,
   │  claim files, brief and check workers, fix gaps from diagnostics
   ▼
 workers (Sonnet 5.5, medium effort)                 roles/WORKER.md
      write the code for one task in the files they were given
```

## The loop

1. **Open milestones.** The main agent reads the DAG (`workflow/DAG.md`) and
   starts a fresh manager for every milestone whose dependencies are done,
   up to the lane limit in WORKLOG.
2. **Plan.** The manager reads its milestone file and the SPEC IDs, frames
   and code it points to, writes its plan into the file's Lane section (tasks, files it
   claims, exit criterion), and spawns workers.
3. **Implement.** Workers write code and report back what they changed. The
   manager reads each worker's diff against the task and sends it back if it
   misses.
4. **Brief review.** When a manager reports a batch done, the main agent
   checks it did what the milestone asked (the right SPEC IDs, nothing
   outside its claim). Not a full code review.
5. **Build gate.** When enough work has landed across lanes, the main agent
   runs the batched bazel builds and tests (one run for everyone) and posts
   the diagnostics, split per lane, to the managers.
6. **Fix.** Managers hand the failures to workers (or fix small ones) and
   report again. Steps 4–6 repeat until the lane's tests pass.
7. **Commit.** The main agent commits each lane's files once its build and
   tests pass, with the lane's tag, updates OVERVIEW.md and GAPS_REVIEW.md
   in the same commit, and pushes.
8. **Close.** A milestone is done when its exit criterion holds and is
   committed. The main agent marks it in the DAG and WORKLOG, ends that
   manager, and goes back to step 1. New milestones always get a fresh
   manager.

## Rules everyone follows

- **One checkout, one bazel.** All work happens in
  `/Users/avestabarzegar/oss/atmosphere-khodr`. Only the main agent runs
  bazel. Never run cargo. Never `bazel clean` or `shutdown`.
- **Claims.** Before editing, a manager lists the files (or new directories)
  its lane will touch in WORKLOG. Two lanes never edit the same file at once.
  A file another lane claims is asked for through the main agent.
- **No git writes except by the main agent.** Managers and workers never
  stage, commit, stash, reset, checkout or push. They leave changes in the
  working tree inside their claim.
- **Never touch others' work.** Files outside your claim, `prototype/`,
  `ui-prototype/`, `.claude/`, and other agents' uncommitted changes are off
  limits.
- **Headless only.** No windows on the owner's screen; tests run headless.
- **Code rules:** repo `AGENTS.md` (explicit thiserror errors with sources,
  actionable messages, minimal visibility, behavioural tests only), no force
  unwraps, never name inspirations in code or docs.
- **Goldens** in `desktop/tests/__snapshots__/design/` change only with the
  owner's OK. The app is made to match them, not the other way round.
- **Decisions.** A choice the spec doesn't settle: pick the reversible
  option, log it Provisional in `DECISIONS.md` via the main agent, keep
  going. The main agent asks the owner in waves.
- **Record as you go.** Every agent keeps its WORKLOG lane current: what it
  is doing, its workers, what is done, what is blocked. If it isn't in the
  log, a resumed session won't know it happened.

## Files

| File | What it holds | Who writes it |
|---|---|---|
| `workflow/WORKFLOW.md` | this process | main agent, owner |
| `workflow/roles/*.md` | each role's brief | main agent, owner |
| `workflow/DAG.md` | milestones, edges, layers, status (one line each, links to the files) | main agent |
| `workflow/milestones/<ID>.md` | one milestone: outcome, exit, pointers into SPEC/frames/decisions/code, and its lane | main agent above "Lane"; its manager the Lane section |
| `workflow/WORKLOG.md` | live roster, lanes, claims, event log, resume steps | everyone, own lane only |
| `OVERVIEW.md` | the living work list | main agent at commit time |
| `GAPS_REVIEW.md` | one entry per commit, how to review it | main agent |

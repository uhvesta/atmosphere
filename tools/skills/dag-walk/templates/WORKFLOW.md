# How we work: walking the milestone DAG

Every agent working on this project's milestones reads this file first,
then `workflow/CONFIG.md`, its role file in `workflow/roles/`, then
`workflow/WORKLOG.md`. A manager then reads only its milestone file,
`workflow/milestones/<ID>.md`, and the pointers in it: no agent reads the
spec, the work list or the decision log whole.

## The shape

```text
 owner
   │  signs off plans, designs, decisions (asked in waves)
   ▼
 main agent (one, in the owner's session)            roles/MAIN.md
   │  owns the DAG, the WORKLOG, builds, commits and pushes
   │  spawns one manager per open milestone
   ▼
 milestone managers (one per milestone)              roles/MANAGER.md
   │  plan the milestone, split it into tasks,
   │  claim files, brief and check workers, fix gaps from diagnostics
   ▼
 workers                                             roles/WORKER.md
      write the code for one task in the files they were given
```

## The loop

1. **Open milestones.** The main agent reads `workflow/DAG.md` and starts a
   fresh manager for every milestone whose dependencies are done, up to the
   lane limit in CONFIG.md.
2. **Plan.** The manager reads its milestone file and what it points to,
   writes its plan into the file's Lane section (tasks, claims, exit
   criterion), and spawns workers.
3. **Implement.** Workers write code and report what they changed. The
   manager reads each diff against the task and sends misses back.
4. **Brief review.** When a manager reports a batch, the main agent checks it
   did what the milestone asked and stayed inside its claim. Not a full code
   review.
5. **Build gate.** When enough has landed across lanes, the main agent runs
   one batched build and test (CONFIG.md) and sends each manager only its own
   diagnostics.
6. **Fix.** Managers turn failures into tasks and report again. Steps 4–6
   repeat until the lane's targets pass.
7. **Commit.** The main agent commits each lane's claimed paths once they
   pass, following CONFIG's commit and push rules.
8. **Close.** A milestone is done when its exit criterion holds and it is
   committed. The main agent marks it in DAG.md and WORKLOG, ends that
   manager, and goes back to step 1. New milestones always get a fresh
   manager.

## Rules everyone follows

- **One checkout, one build.** All work happens in the checkout named in
  CONFIG.md. Only the main agent runs builds and tests.
- **Claims.** Before editing, a manager lists the files (or new directories)
  its lane will touch in WORKLOG. Two lanes never edit the same file at once.
  A file another lane claims is asked for through the main agent.
- **One writer per file.** Within a lane too: two workers never edit the
  same file at once.
- **Seam files** (CONFIG.md: shared state/action types, build roots, shared
  protocols, dependency pins) belong to the main agent. Managers ask for the
  exact lines they need.
- **Snapshots.** Workers never record snapshots or goldens. The main agent
  records; the manager reviews the images.
- **Workers can't compile.** Managers give them signatures or stubs and keep
  each task to one behaviour, so gate failures map back to one task.
- **Rollback.** A lane that breaks the gate for others is backed out of the
  batch (its files left in the tree, not committed) until it is fixed.
- **No git writes except by the main agent.** Managers and workers never
  stage, commit, stash, reset, checkout or push.
- **Never touch others' work.** Files outside your claim, CONFIG's
  never-touch paths, and other agents' uncommitted changes are off limits.
- **Code rules:** the repo's own instructions (CLAUDE.md, AGENTS.md) and any
  listed in CONFIG.md.
- **Approved designs** (CONFIG.md) change only with the owner's OK. The code
  is made to match them, not the other way round.
- **Decisions.** A choice the spec doesn't settle: pick the reversible
  option, have the main agent log it Provisional in the decision log, keep
  going. The main agent asks the owner in waves.
- **Record as you go.** Every agent keeps its lane current: what it is doing,
  its workers, what is done, what is blocked. If it isn't logged, a resumed
  session won't know it happened.

## Files

| File | What it holds | Who writes it |
|---|---|---|
| `workflow/CONFIG.md` | commands, paths, limits, models, docs | main agent, owner |
| `workflow/WORKFLOW.md` | this process | main agent, owner |
| `workflow/roles/*.md` | each role's brief | main agent, owner |
| `workflow/DAG.md` | milestones, edges, layers, status, critical path | main agent |
| `workflow/milestones/<ID>.md` | one milestone and its lane | main agent above "Lane"; its manager the Lane section |
| `workflow/WORKLOG.md` | resume notes, roster, claims, lanes, gates, event log | everyone, own lane only |

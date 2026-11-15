# Milestone files

One file per milestone of the DAG: `workflow/milestones/<ID>.md` (for
example `FL.md`). An agent working on a milestone reads its file and follows
its pointers; it does not read SPEC.md, OVERVIEW.md or DECISIONS.md whole.

## Template

```markdown
# <ID>: <goal-shaped name>

- **Status:** blocked | open | in progress (lane <lane>) | done (<commit>)
- **Depends on:** <IDs>, each with a one-line reason
- **Unblocks:** <IDs>

## Outcome
What the user can do when this is done, in one or two sentences.

## Exit criterion
The flow someone runs to judge it, and the tests that prove it.

## Read these (pointers, nothing else)
- SPEC: <IDs>, found with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md`
  (read each rule and the few lines around it; list the § for context)
- Frames: `desktop/tests/__snapshots__/design/NN-*.png` (which ones)
- Decisions: D<n> (grep `^## D<n>:` in DECISIONS.md)
- Code: crates and files to start from, and existing tests to follow
- API: `api/<file>` sections, if any

## Out of scope
What a neighbouring milestone owns, so lanes don't overlap.

## Lane (written by the manager)
- Design notes:
- Claims: <paths>
- Tasks:
  - [ ] T1 <task> · worker <name> (Sonnet, medium) · files … · status
- Waiting on:
- Build gates: <time> pass|fail (targets)
- Ready to commit: no | yes: <what changed, how a reviewer checks it>
```

The main agent writes everything above "Lane" when it opens the DAG and
keeps the status and edges current. The manager writes the Lane section and
nothing else in the file. When a milestone is done its file stays as the
record of what was built.

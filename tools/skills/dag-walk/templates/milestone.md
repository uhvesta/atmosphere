# <ID>: <goal-shaped name>

- **Status:** blocked | open | in progress (lane <lane>) | done (<commit>)
- **Depends on:** <IDs>, each with a one-line reason
- **Unblocks:** <IDs>

## Outcome
What the user can do when this is done, in one or two sentences.

## Exit criterion
The flow someone runs to judge it, and the tests that prove it.

## Read these (pointers, nothing else)
- Spec: requirement IDs to grep (read each rule and the lines around it)
- Designs: which approved frames or mockups
- Decisions: which decision-log entries
- Code: modules and files to start from, and existing tests to follow

## Out of scope
What a neighbouring milestone owns, so lanes don't overlap.

## Lane (written by the manager)
- Design notes:
- Claims: <paths>
- Tasks:
  - [ ] T1 <task> · worker <name> · files … · status
- Waiting on:
- Build gates: <time> pass|fail (targets)
- Ready to commit: no | yes: <what changed, how a reviewer checks it>

<!-- The main agent writes everything above "Lane" and keeps status and
edges current. The manager writes only the Lane section. A done file stays
as the record of what was built. -->

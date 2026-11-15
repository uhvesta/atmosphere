# Role: milestone manager

You own one milestone of the DAG from plan to passing tests. You plan it,
split it into tasks, and have workers write the code. Read
`workflow/WORKFLOW.md`, `workflow/CONFIG.md`, then your milestone file, then
only what it points to: spec rules by ID (grep them; don't read the spec
whole), the designs, the decisions and the code. Check `workflow/WORKLOG.md`
for other lanes' claims.

## Plan first

Before any worker starts, write into your milestone file's Lane section and
add your claims to the WORKLOG claims table:
- **Goal and exit criterion** (from the milestone file; ask the main agent
  if unclear).
- **Design notes:** what you will add or change and why, following the
  project's architecture (CONFIG.md).
- **Tasks:** small, independent units, each with its files, the behaviour it
  delivers, and the tests that prove it.
- **Claims:** every file or new directory your lane will touch. If another
  lane claims one, ask the main agent.

## Workers

Spawn each worker with the Agent tool, using the worker model and effort in
CONFIG.md. Give it: the task, the exact files it may edit, the behaviour and
tests expected, the requirement IDs, and "read workflow/roles/WORKER.md
first". Not the whole milestone file. Run workers in parallel only when their
files don't overlap. Log each worker (name, task, files, status) in your Lane
section.

If you cannot spawn agents, write the worker briefs into your lane and ask
the main agent to spawn them.

## Checking workers

Read every worker's diff against its task before reporting it. Send it back
if it misses the behaviour, edits outside its files, restates the
implementation in tests, or breaks the repo's code rules. Small fixes you may
make yourself.

## Reporting to the main agent

Report a batch when a coherent piece is in place: which tasks, which files,
which targets to build and test, and anything you need decided. Don't run
builds or tests; the main agent runs one batched gate and sends you your
diagnostics. Turn each failure into a fix task.

## Done

When the exit criterion holds and the main agent's gate passes for your
targets, set your Lane section to "ready to commit" with a summary a
reviewer can follow (what changed, how to check it). The main agent commits;
you then stop.

## Never

Run builds or tests; touch git; edit outside your claim; edit approved
designs or CONFIG's never-touch paths.

# Role: main agent

You run in the owner's session. You are the only agent that runs builds and
tests, touches git, or talks to the owner. Read `workflow/WORKFLOW.md` and
`workflow/CONFIG.md` first.

## You own

- `workflow/DAG.md` and the top half of each milestone file.
- The WORKLOG resume section, roster and event log.
- Builds, tests, commits, pushes, and edits to the docs CONFIG.md names as
  main-agent-only (work list, review log, decision log).

## Starting or resuming

1. Read `workflow/WORKLOG.md` top to bottom. Check `git status` and the last
   commits against it; anything in the tree not in a lane's claim is
   someone's unlogged work: find out whose before doing anything.
2. For each active lane, check its manager is still running (ListAgents). A
   lane whose manager is gone gets a fresh manager pointed at its milestone
   file; the Lane section says where it stopped.
3. Open milestones from DAG.md whose dependencies are done, up to the lane
   limit.

## Spawning a manager

Agent tool, the manager model from CONFIG.md, a short brief: the milestone
file path, its lane name, and "read workflow/WORKFLOW.md, workflow/CONFIG.md
and workflow/roles/MANAGER.md first". Name it `minion <lane>` and log it in
the roster before it starts.

## Brief review (each batch a manager reports)

A few minutes, not a full review:
- Did it build what the milestone asked (its requirement IDs, designs, exit
  criterion), and nothing it wasn't asked for?
- Are all changed files inside its claim?
- Are there tests for the behaviour, not restating the implementation?
Send misses back to the manager with the specific gap.

## Build gate

When two or more lanes have reported work, or one is ready to finish, run
one batched build and test in the background for every affected target with
CONFIG's commands. Split diagnostics per lane and send each manager only its
own. Log the run (time, targets, pass/fail per lane).

## Committing

Per lane, once its targets pass: stage only that lane's claimed paths and
commit with the lane tag, following CONFIG's commit and push rules. Verify
the push. Release the lane's claims.

## Talking to the owner

Short status lines when they haven't heard from you. Decisions go in waves
with recommendations. Never claim done without the build and tests passing.

# Role: main agent

You run in the owner's session. You are the only agent that runs bazel,
touches git, or talks to the owner. Read `workflow/WORKFLOW.md` first.

## You own

- `workflow/DAG.md` and the top half of each `workflow/milestones/<ID>.md`
  (outcome, exit, pointers, edges, status).
- The WORKLOG roster, the event log and the resume section.
- Builds, tests, commits, pushes, and `OVERVIEW.md` / `GAPS_REVIEW.md` /
  `DECISIONS.md` edits.

## Starting or resuming

1. Read `workflow/WORKLOG.md` top to bottom. Check `git status` and
   `git log --oneline -10` against it; anything in the tree not in a lane's
   claim is someone's unlogged work: find out whose before doing anything.
2. For each lane marked active, check whether its manager is still running
   (ListAgents). A lane whose manager is gone is resumed by a fresh manager
   pointed at its milestone file, whose
   Lane section says where it stopped.
3. Open milestones from DAG.md whose dependencies are done, up to the lane
   limit.

## Spawning a manager

Use the Agent tool with `model: "opus"` (Opus 5.5) and a short brief:
the milestone file path, its lane name, and "read workflow/WORKFLOW.md and
workflow/roles/MANAGER.md first". Name it
`minion <lane>` and log it in the roster before it starts.

## Brief review (each batch a manager reports)

Check, in a few minutes, not a full review:
- Did it build what the milestone asked (its SPEC IDs, frames, exit
  criterion), and nothing it wasn't asked for?
- Are all changed files inside its claim?
- Are there tests for the behaviour, not restating the implementation?
Send misses back to the manager with the specific gap.

## Build gate

When two or more lanes have reported work (or one lane is ready to finish),
run one batched bazel run in the background for every affected target:
`export PATH=$PWD/.tools/bin:$PATH`, then `bazel build` and `bazel test` on
the lanes' targets. Split the diagnostics per lane and send each manager only
its own. Log the run in WORKLOG (time, targets, pass/fail per lane).

## Committing

Per lane, once its targets pass: stage only that lane's claimed paths
(explicit paths; for shared docs with others' hunks, build the blob with
`git hash-object -w` and `git update-index --cacheinfo`), commit with the
lane tag ("Humata 2 (minion <lane>): …"), no AI attribution trailers, add
the GAPS_REVIEW entry and OVERVIEW ticks in the same commit, then the
hash-fill commit, push, verify with `git ls-remote origin main`. Release
the lane's claims in WORKLOG.

## Talking to the owner

Short status lines when they haven't heard from you. Decisions go in waves
with recommendations. Never claim done without the build and tests passing.

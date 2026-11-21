# Submitting work for local review

Request a local review **before** pushing to GitHub. Humata moves the To-do
to `Local review requested` and places a notice in the owner's `Reviews`
inbox (`Local · ready to push`) so they can read the diff, inspect layers,
and approve the push directly from the review screen.

## 1. Verify and inspect the local diff

Before requesting review: tests pass, commits are clean, and nothing
unrelated or secret is in the diff.

    humata diff <session> --json          # single-branch diff against main
    stack diff                            # layer-by-layer diff for a stack

`<session>` is yours, from `humata whoami`.

## 2. Draft the PR summary with a cheap subagent

Spawn a cheap/fast subagent (or run a lightweight pass) over the local diff
to draft a **declarative** PR summary in the `What / Why / Details` format:

- **What:** one declarative outcome statement describing what is now true
  (e.g. `"Refund retries back off exponentially"` — **not** imperative
  `"Add retry backoff"`).
- **Why:** the user or system problem solved, or the invariant preserved.
- **Details:** concise bullet list of stack layers (if stacked), key files
  changed, and verification results (tests/checks run).

Keep it short and scannable; the owner reads this at the top of the review
screen and can carry it into the GitHub PR description on push.

## 3. Call `todo_request_review`

Submit the drafted summary and layer metadata with the MCP tool
`todo_request_review`:

```json
{
  "key": "PAY-7",
  "summary": "Refund retries back off exponentially.",
  "summary_what": "Refund retries back off exponentially with jitter up to 60s.",
  "summary_why": "Prevents thundering-herd retries during payment gateway outages.",
  "summary_details": "- Layer 1 (`feat/retry-policy`): backoff schedule and jitter\n- Layer 2 (`feat/refund-worker`): wire policy into refund worker\n- Verified: `cargo test -p billing::refund` passes (42/42)",
  "layers": [
    {
      "repo": "owner/repo",
      "base_branch": "main",
      "head_branch": "feat/retry-policy",
      "title": "Refund retry backoff policy"
    }
  ]
}
```

- `key`: the To-do key when known. If omitted, Humata auto-attaches only on
  deterministic signals (branch stacked on an existing layer, or session
  already linked); otherwise it creates a new To-do.
- `summary_what`, `summary_why`, `summary_details`: structured sections for
  the review screen (`summary` is the one-line headline).
- `layers`: one entry per branch/PR layer (`repo`, `base_branch`,
  `head_branch`, `title`).

### Notification only — never blocks `Agent inbox`

`todo_request_review` is a **non-blocking notification**. It transitions the
To-do to `Local review requested` and posts a buttonless notice in the
`Reviews` inbox. It does **not** create a blocking permission prompt in
`Agent inbox`, which stays reserved for agents stopped waiting on an answer.

## 4. Owner approval boundary

- **Never push or open a GitHub PR** until the owner clicks **Looks good**
  (and pushes from the review screen) or explicitly approves a push via
  `session_push` / `humata session push` (`push.md`).
- **If the owner requests changes:** address the feedback locally, re-run
  verification, refresh the summary if the scope changed, and call
  `todo_request_review` again. Re-submitting marks
  `fixes_pushed_since_review` (`Fixes pushed` in the `Reviews` inbox) so the
  owner knows it is ready for another look.

## What needs the owner

| Action | Default rule |
| --- | --- |
| Read your session's diff (`humata diff`, `stack diff`) | Allowed |
| Draft PR summary and call `todo_request_review` | Allowed (notifies `Reviews` inbox; non-blocking) |
| Mark a layer `Looked at` / `Looks good` | The owner's alone |
| Push branches and open GitHub PRs | Owner action from Review screen, or approved via `push.md` |

## Failures

| What you see | What to do |
| --- | --- |
| empty diff or uncommitted changes | Commit or finish the work first; don't request review on an empty diff. |
| `todo_request_review` rejects unknown `key` | Verify the To-do key or omit `key` so Humata attaches deterministically or creates a new To-do. |
| `layers` branch mismatch | Re-check `stack status` or `humata diff <session> --json` and pass the exact `base_branch` and `head_branch`. |
| owner requests changes after review | Fix locally, re-verify, and call `todo_request_review` again; do not push directly to GitHub. |

## Not in the CLI

- Requesting local review with structured `What / Why / Details` summary and
  layers (MCP: `todo_request_review`).

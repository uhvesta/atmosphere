---
name: humata-pull-requests
description: Push a Humata session's work and open pull requests, split commits into a stack of pull requests with the stack CLI, start a pull request review session, and draft review comments for the owner to post. Use when the user says "push this", "open a PR", "put up a pull request", "stack these commits", "split this into stacked PRs", "restack", "update the PR after review", "review this PR", "review PR 123", "draft review comments" or "what changed in this session".
---

# Humata pull requests, reviews and stacks

You are in a Humata session and `humata` is on your `PATH`.

1. Read `shared/basics.md` once per conversation: who you are, how commands
   answer, and what to do when the owner must approve.
2. Then read the one file for the job:
   - `push.md`: see a session's changes, push its branches and open pull
     requests.
   - `stack.md`: turn commits into a chain of pull requests with `stack`.
   - `review.md`: start a pull request review, draft comments, and what
     the owner posts.

All paths are relative to this skill's folder. Run only commands those
files list. To arrange a change for the owner to read, use
`render-humata-review` and `order-humata-review` instead.

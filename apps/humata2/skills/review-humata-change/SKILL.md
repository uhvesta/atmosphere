---
name: review-humata-change
description: Actually review a change in Humata when the user asks for a review - read it (the whole stack, across repos, when it is one), find real problems, put each finding on its lines as an inline note in the review view, draft pull request comments when the change is a pull request the user is reviewing (never posted), and end with a short verdict. Use when the user says "review this", "review my PR", "review this stack", "do a code review", "find problems", "find bugs", "what's wrong with this", "leave comments" or "is this ready to merge". Not for only showing the change (render-humata-review) and not for a neutral walkthrough (order-humata-review).
---

# Review a change

The user asked for your judgement. Find what is wrong, put it on the lines
it is about, and say where the change stands.

1. Read `guide.md` and follow its steps.
2. It uses `shared/diff-spec.md` (how to say what is compared) and
   `shared/stacks.md` (how to find a stack and read it layer by layer).

Paths are relative to this skill's folder. You never post to GitHub: notes
are for the user, drafts wait for the user to post.

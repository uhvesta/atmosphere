---
name: render-humata-review
description: Prepare a change so the user can look at it and find their way through it, with no opinions - work out its layers as a stack (stack topics or chained pull requests, else one per commit, else a logical split of uncommitted work), across every repo it touches, and send that to Humata's review view to step through; or show one narrow diff they name ("just my unstaged changes", "since commit abc", "what GitHub will show", "since I last looked"). When asked, show it with notes - a reading order and short neutral notes that explain what the code does. Never judges. Use when the user says "show me", "show me what changed", "show the diff", "show me this PR", "let me look", "just look", "I just want to see the code", "what changed", "show the stack", "prepare this for me to look at", "show it with notes", "walk me through this", "explain this change" or "what order should I read this in". Not for finding problems or giving a verdict (review-humata-change).
---

# Prepare a change for the user to look at

Look only. The user wants to read the code themselves, layer by layer.

1. Read `guide.md` and follow it.
2. It uses `shared/stacks.md` (how to pick the layers and send them to the
   view) and `shared/diff-spec.md` (how to say what is compared).
3. If the user asked to be walked through it or for notes, also read
   `with-notes.md`: it adds a reading order and notes that explain.

Paths are relative to this skill's folder. Leave no notes unless the user
asked for them, and give no opinion unless the user then asks for a
review.

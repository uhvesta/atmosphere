# Brief micro-07: Large and image paste
Output: `apps/humata2/flows/cases/micro/07-large-and-image-paste.md`

Scenario: I paste a 3 MB log into a shell, then a screenshot into an agent session.

Steps: the view queues pending input up to MAX_EVENTS 1024 and MAX_BYTES 1 MiB (terminal/view/src/view.rs) — find what happens past the cap (dropped? refused? message?); an image paste is saved privately for the session and the agent gets a path; an image over the size limit is refused with a message; a retried image paste with the same key does not save twice.

Branches: paste while the link is slow (backpressure); paste in the composer instead (features/session queue/pastes — Humata 1 had Q milestones; check features/session/tests for paste); the daemon is gone during the paste.

Pointers: terminal/view/src/view.rs; daemon/humatad/tests/paste.rs (image saved privately, retry key, over limit refused); grep features/session/tests for paste; SPEC IDs via `grep -n 'paste' apps/humata2/SPEC.md | head`.

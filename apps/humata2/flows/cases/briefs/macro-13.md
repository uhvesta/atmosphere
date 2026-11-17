# Brief macro-13: Finding anything
Output: `apps/humata2/flows/cases/macro/13-finding-anything.md`

Scenario: I remember an agent said "the retry budget is 3" sometime last week. I search turns from the palette, find the conversation, open it at that turn, pin the session, then search for a file by name across sessions.

Steps: palette search modes; search turns / conversations (`humata sessions search`); results open at the turn; pins hold across restarts; file search (files_find).

Branches: the turn is on an offline machine; no results; a pinned item deleted; §26.22 steps 1, 3 and 4 owed (FIND milestone note in workflow/DAG.md — check what they are).

Pointers: features/find/tests/*.rs, features/app/tests/find.rs, find_conversation.rs, features/palette/tests/search.rs, desktop/tests/live_flows/find.rs (real daemon), daemon/humatad/tests/history*.rs, find_files.rs, marks.rs, pointers.rs; cli/src/commands/search.rs; workflow/milestones/FIND.md; SPEC §26.22.

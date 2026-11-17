# Brief agents-20: An agent on a box while the host is away
Output: `apps/humata2/flows/cases/agents/20-an-agent-on-a-box-while-the-host-is-away.md`

Scenario: the host Mac sleeps. An agent on box-1 keeps working: it reads and writes its own files, asks me a question, tries to push, edits the board, reads a channel list it read earlier, and waits on a request the host holds.

Steps (table rows in away_boundary.rs): own files read and written on the machine; a question is kept and journaled; a call the rules say never is denied; asks under wait held in the box's Inbox, under skip denied saying the user is away; a push under skip denied, under wait waits for the host then asked; a held call whose deadline passes is host_unreachable; reads answered from the copy marked with their age; a read never made while linked is host_unreachable; a start before the host's rules arrive is refused.

Branches: the link drops while the call waits (host_unreachable, retryable); the agent stops waiting (forwarded wait cancelled); on relink the journal replays and the first answer wins; an edit the host now refuses is kept as lost and the session says so.

Pointers: daemon/humatad/tests/away_boundary.rs, away_forward.rs, away_board_journal.rs, away_contain_copy.rs, away_inbox.rs, away_replay.rs, away_left_out.rs, away_policy_copy.rs, workflow/milestones/AWAY.md, SPEC AW-*.

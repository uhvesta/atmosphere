# Brief macro-14: The host is away while I work on a box
Output: `apps/humata2/flows/cases/macro/14-host-away-working-on-a-box.md`

Scenario: I SSH straight into box-1 from a borrowed laptop while my Mac (the host) sleeps. The agents on box-1 keep working. I log in with the approval passphrase, answer an ask in box-1's Inbox, and later the Mac wakes and everything syncs.

Steps: box-1 is "away" until the host opens its uplink; agents work on their own files; questions to me are kept and journaled; under the "wait" rule asks are held in the box's Inbox; I answer with the passphrase; when the link returns, answers reach the host in order and the first answer wins.

Branches: wrong passphrase (five tries lock it, survives restart); a push while away (denied under skip, waits under wait); a call the rules say never; reads answered from the copy marked with their age; an edit the host now refuses is kept as lost and the session says so; the Uplink carries nothing (OVERVIEW gap — reconcile with away_link tests).

Pointers: daemon/humatad/tests/away_*.rs (12 files, real daemons with fake links), cli/src (login with passphrase), cli/tests/waiting.rs (a passphrase is typed at the terminal; never read from a pipe), workflow/milestones/AWAY.md, SPEC AW-*.

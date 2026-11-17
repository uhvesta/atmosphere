# Brief macro-09: End of day, I stop everything and start next morning
Output: `apps/humata2/flows/cases/macro/09-end-of-day-stop-and-start.md`

Scenario: at 18:00 I choose "Stop all sessions…" for this Mac (and for box-1), confirm, see everything stopped; overnight the Mac reboots for an update; at 09:00 I press "Start sessions" and read the report.

Steps: the confirm sheet says what will stop; agents and shells end (a shell ignoring hangup is killed); an attached terminal is told to connect again; a pending Inbox request is withdrawn as stopped; no restart report on stop; after the reboot it stays stopped and starts nothing; Start brings sessions back like after a shutdown with a report naming the stop.

Branches: Stop pressed twice acts once; Start with nothing stopped is refused; a cut at each stop step (ends stopped with nothing lost); an agent tries to start the machine (refused); `humata stop` typed in a plain terminal with no login stops the machine; `humata start` with no login is refused; PR reviews stop following while stopped.

Pointers: daemon/humatad/tests/stop_sessions.rs, stop_access.rs, stop_box.rs, stop.rs, pr_stopped.rs; features/hosts/tests/stop.rs; ui/src (grep "Stop all sessions" and "Start sessions"); workflow/milestones/STOP.md (status open — reconcile with what exists).

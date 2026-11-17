# Brief macro-10: A reboot or power cut in the middle of work
Output: `apps/humata2/flows/cases/macro/10-reboot-or-power-cut-mid-work.md`

Scenario: the power goes out while a fork is copying, a move is halfway, a board edit is being saved, and the state store is writing its last good copy. Power returns, the Mac boots, I open the app.

Steps: each operation is undone or finished, never half; the fork cut before its agent starts is undone; one cut after start is kept; the move is undone or finished; the store opens whole (from the last good copy if damaged) and the start says so; the restart report lists each; "Couldn't resume" for a session whose conversation is gone, with Try again.

Branches: power cut on the host only, on the box only, on both; a cut while the last good copy is itself saved or restored; doctor skill (`humata doctor restarts`); `last_start` not pushed (OVERVIEW gap).

Pointers: daemon/humatad/tests/power_cut_host.rs, power_cut_machine.rs, power_cut_both.rs, restart.rs, restart_operations.rs, fork.rs (cut tests), move_session.rs (cut tests), checkpoint.rs, state_lock.rs, daemon/revive/tests/kill_matrix*; desktop/tests/live_flows.rs reboot tests; features/app/tests/restart.rs; ui/src/restart_report.rs, resume_failed.rs; workflow/milestones/SAFE.md.

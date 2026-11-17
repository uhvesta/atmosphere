# Brief micro-18: Terminals after a host reboot
Output: `apps/humata2/flows/cases/micro/18-terminals-after-a-host-reboot.md`

Scenario: my Mac reboots with three sessions running: an agent mid-turn, a shell, and an agent whose conversation file was deleted.

Steps: the daemon starts at login and brings sessions back; the agent session is running again in its conversation; the shell comes back (a new shell in the same folder? its old output?); a restart report says what came back and what did not; the lost conversation shows "Couldn't resume…" with Try again / start fresh with a summary.

Branches: one shell can't come back (one line in the report, others still do); the state file is damaged and comes back from its last good copy; I press Try again; I open `humata doctor restarts`.

Pointers: desktop/tests/live_flows.rs `after_a_reboot_the_session_is_running_again_in_its_conversation`, `a_session_whose_conversation_is_lost_cannot_resume`; daemon/humatad/tests/restart.rs (all five); features/app/tests/restart.rs (hearing a restart keeps it and tells the person; Try again resumes); ui/src/resume_failed.rs and ui/src/restart_report.rs (OVERVIEW says "Couldn't resume" is not shown in the app, PER-29; the files exist — check whether they are rendered and tested, and report the conflict); daemon/revive/tests/kill_matrix*; cli/src/commands/doctor.rs.

# Brief micro-19: Typing into an exited or read-only terminal
Output: `apps/humata2/flows/cases/micro/19-typing-into-an-exited-or-read-only-terminal.md`

Scenario: I type into four terminals that can't take input: a shell that exited, a session that is moving to box-1, a read-only attach, and a session on a stopped machine.

Steps for each: what I see (the tab's state line), what happens to my keys (refused, not queued), and how I get back to a writable terminal (new shell, wait for the move, ask to write, Start sessions).

Branches: the move finishes while I am typing; the move fails and the session stays on the source; I attach read-only and then ask for write.

Pointers: daemon/humatad/tests/terminal.rs (typing into an exited shell is refused; read-only attach; writers hear about each other), daemon/humatad/tests/move_session.rs `a_moving_session_s_terminal_is_read_only_until_it_resumes`, daemon/humatad/tests/stop_sessions.rs `while_stopped_agents_and_scripts_are_refused_and_nothing_can_be_typed_attached_or_started`, `a_terminal_attached_when_the_sessions_stop_is_told_to_connect_again`; ui/src/terminals.rs for what the app shows.

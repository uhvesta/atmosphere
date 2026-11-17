# Brief micro-08: Interrupt, end and suspend
Output: `apps/humata2/flows/cases/micro/08-interrupt-eof-suspend.md`

Scenario: I run `yes` and hit ⌃C; I press ⌃D on an empty line (shell exits) and on a non-empty line; I ⌃Z a job and `fg` it; I try ⌃\ and ⌃S/⌃Q.

Steps: ⌃C reaches the program on every platform, also on Mac where ⌘C is copy; output stops promptly even after a flood; ⌃D on an empty line exits the shell and the tab shows the shell ended (what does the tab show? check ui/src/terminals.rs and session_tabs.rs); typing afterwards is refused.

Branches: ⌃C with text selected on Linux (Ctrl+C vs Ctrl+Shift+C); ⌃C in the agent terminal mid-turn (interrupts the agent); a shell that ignores hangup on Stop all (daemon kills it); ⌃D in the last tab of a session.

Pointers: terminal/view/src/keys.rs unit test `ctrl_c_goes_to_the_program_on_every_platform`; terminal/vt/tests/input.rs `ctrl_c_sends_etx`; daemon/humatad/tests/terminal.rs (typing into an exited shell is refused); daemon/humatad/tests/stop_sessions.rs `a_shell_that_ignores_hangup_and_terminate_is_killed_by_the_stop`.

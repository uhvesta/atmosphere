# Brief micro-17: The terminal survives an app restart
Output: `apps/humata2/flows/cases/micro/17-terminal-survives-an-app-restart.md`

Scenario: a long build is printing in a shell tab. I quit the app with ⌘Q, wait a minute, reopen it.

Steps: the holder kept the PTY; the tab reopens on the same session; it shows what was printed while the app was gone; my scroll position or the bottom; typing waits until the replica is synced and then types once.

Branches: I type during catch-up (held or refused?); the daemon also restarted (tab outlives its daemon); a huge backlog (snapshot sent in pieces, compressed); the agent tab rather than a shell tab.

Pointers: clients/live/tests/term_restart.rs `a_shell_tab_catches_up_after_the_app_restarts_and_types_only_once_synced`, desktop/tests/live_flows.rs `after_the_app_restarts_the_session_tab_shows_what_was_printed_while_it_was_gone` (real daemon), daemon/humatad/tests/terminal.rs (reattach from cursor, a tab outlives its daemon, big snapshot in pieces, compression), terminal/client (replica).

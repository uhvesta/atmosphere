# Brief micro-05: History and reverse search
Output: `apps/humata2/flows/cases/micro/05-history-and-reverse-search.md`

Scenario: I press ↑ to recall the last command, ↓ back, ⌃R to search history, ⌃P/⌃N, then quit and reopen the app and ↑ again.

Steps: arrows encode as CSI A/B in normal cursor key mode and SS3 A/B in application mode; ⌃R is 0x12; history survives the app restart because the shell itself lives in the holder (the app is only a client).

Branches: ↑ inside a TUI in application cursor mode; history of 12,000 lines (lab test); ↑ while the session is moving (read-only); after a host reboot the shell is new so its in-memory history is whatever the shell saved.

Pointers: terminal/vt/tests/input.rs (arrows by cursor key mode); terminal/lab/tests/interactive.rs `actual_zsh_commands_clipboard_movement_and_twelve_thousand_history_lines`; clients/live/tests/term_restart.rs `a_shell_tab_catches_up_after_the_app_restarts_and_types_only_once_synced`; daemon/humatad/tests/restart.rs `a_shell_whose_holder_died_comes_back`.

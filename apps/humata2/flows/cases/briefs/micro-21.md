# Brief micro-21: The command palette
Output: `apps/humata2/flows/cases/micro/21-command-palette.md`

Scenario: with a terminal focused I press ⌘K (Ctrl+K on Linux), type "mov", pick Move, choose a machine inline, press Esc to step back; then paste a PR URL; then type "long" to reach a setting; then open a file result.

Steps: ⌘K opens the palette even with the terminal focused, and the keystrokes I type go to the palette, not the shell; ranking puts Move first for "mov"; Move asks inline for an online machine of the channel; Esc steps back one level, then closes; a pasted PR URL offers "Review this PR"; "long" opens Settings at Long lines; a file result opens in the current session's clone.

Branches: no online machine for Move; typing gibberish (empty state); palette while an overlay is open; ⌘K expected to clear the terminal (it does not).

Pointers: features/palette/tests/act.rs (the five tests), features/palette/tests/*.rs, desktop/tests/native/overlays.rs `palette_new_session_and_toast_reference_images`, `palette_file_results_open_the_current_sessions_clone`, desktop/tests/native/terminal.rs `an unfocused terminal gets no palette keystrokes`, desktop/tests/live_flows/find.rs `from_the_palette_a_session_moves_a_turn_is_found_a_pr_reviews_settings_open_and_a_pin_holds` (real daemon), ui/src/keys.rs.

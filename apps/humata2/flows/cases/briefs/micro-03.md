# Brief micro-03: Word moves with Option
Output: `apps/humata2/flows/cases/micro/03-option-word-moves.md`

Scenario: on macOS I hold Option to move and delete by word: ⌥← and ⌥→, ⌥B and ⌥F, ⌥D, ⌥⌫. I try it in plain zsh, plain bash and a zsh with a common plugin framework's key bindings.

Coach findings to verify:
- terminal/pane/src/host.rs: `option_as_alt_text` only applies when OptionAsAlt is on; default `OptionAsAlt::Off` (terminal/vt/src/terminal.rs ~line 223); no production code calls `set_option_as_alt` (only tests); no app setting exists. So ⌥B types ∫, ⌥F ƒ, ⌥D ∂. Likely bug / missing setting.
- ⌥←/⌥→ are encoded by the prebuilt encoder with the alt modifier, probably CSI 1;3D / CSI 1;3C. Plain bash and zsh do not bind these by default, so nothing visible may happen. Mark "Likely bug, verify" (no encoder source in the repo).
- ⌥⌫: say what the code does (search keys.rs and host.rs for backspace handling) or mark unknown.

Branches: Option-as-Alt "on" in a test (terminal/pane/tests/pane.rs "option types its character unless set to act as alt"; terminal/vt/tests/input.rs alt-letter ESC when option-as-alt) proves the mechanism works on fakes/vt only; Linux Alt+B/F (sends ESC b? check keys.rs non-Mac path); typing a real ∫ on purpose must still work when the setting is off; Settings has no switch (grep ui/src/settings* and features/settings for "option").

Pointers: terminal/pane/src/host.rs, terminal/vt/src/terminal.rs, terminal/vt/tests/input.rs, terminal/pane/tests/pane.rs. Status for the setting: Not built, no milestone unless TERM.md names it.

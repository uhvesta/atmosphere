# Brief micro-10: Scrollback and find
Output: `apps/humata2/flows/cases/micro/10-scrollback-and-find.md`

Scenario: a build prints 20,000 lines. I scroll up with the wheel and ⇧PgUp (Linux) / ⌘PgUp (Mac), stay put while output keeps streaming, then search with ⌘F for "error", step with ⌘G and ⇧⌘G, try a broken regex, and press ⌘K expecting "clear".

Steps: scroll keys (Mac: ⌘+PgUp/PgDn/Home/End; elsewhere Shift+…); the view does not jump while output streams; find bar shows "N of M" or "No matches"; a bad regex shows an error; Esc closes find.

Branches: ⌘K opens the command palette instead of clearing (ui/src/keys.rs binds cmd-k) — say this is a deliberate choice that differs from other Mac terminals; find in the agent terminal; scrollback after an app restart (kept? daemon replay); scrollback limit.

Pointers: terminal/view/src/keys.rs scroll_key, terminal/pane/src/pane.rs (find shortcuts), terminal/pane/tests/pane.rs (find bar, broken regex, view stays where scrolled), terminal/view/tests/view.rs (history keys, scrollbar, pixel scrolls), desktop/tests/native/terminal.rs (keeps its scrollback), terminal/lab/tests/interactive.rs (real ⌘PgUp/⌘Home/⌘End), daemon/humatad/tests/terminal.rs (replay, big snapshot in pieces).

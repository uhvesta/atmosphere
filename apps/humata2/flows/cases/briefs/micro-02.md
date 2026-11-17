# Brief micro-02: Mac line-editing shortcuts
Output: `apps/humata2/flows/cases/micro/02-mac-command-line-shortcuts.md`

Scenario: on macOS I press ⌘⌫ to delete back to the start of the line, then ⌘← and ⌘→ to jump to the line start and end, as I would in other Mac text fields and terminals.

Starting state: a shell tab focused, half-typed command `docker compose up --build web`.

Expected (user expectation): ⌘⌫ sends ^U (0x15), ⌘← sends ^A (0x01), ⌘→ sends ^E (0x05), or an equivalent the shell honours.

Coach finding to verify and write up as a Likely bug: in terminal/view/src/keys.rs, on Mac with platform (⌘) held and no ctrl/alt, only c, v and scroll keys (pageup, pagedown, home, end via scroll_key) map to intents; every other ⌘ key is `KeyIntent::Ignore`. The app (ui/src/keys.rs, desktop/src/main.rs) binds none of cmd-backspace, cmd-left, cmd-right. So they do nothing. Confirm by reading both files and say how a test would show it (send cmd-backspace, assert bytes ^U reach the PTY).

Branches: ⌘⌫ in the composer or a text field (works there? check gpui text input in ui/src); ⌘← in vim (should be Home-like or nothing); Linux equivalents (Ctrl+U works natively, no change); ⌘A (select all in the terminal or nothing?).

Pointers: terminal/view/src/keys.rs (Platform::Mac branch, scroll_key); ui/src/keys.rs `pub fn bind`; desktop/src/main.rs menus (Edit menu: Undo, Redo, Cut, Copy, Paste, Select All are OS actions). Status for the expected behaviour: "Not built, no milestone" plus the Likely bug entry. Check TERM milestone (workflow/milestones/TERM.md) for any key-mapping item.

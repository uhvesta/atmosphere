# Brief micro-06: Paste text
Output: `apps/humata2/flows/cases/micro/06-paste-text.md`

Scenario: I paste one line, then a five-line script, into a shell; then into vim; then text that contains the bracketed-paste end marker.

Steps: ⌘V (Mac) / Ctrl+Shift+V or Shift+Insert (Linux); one-line plain paste is sent as typed; a multi-line paste without bracketed paste waits for me to confirm (what do I see? read terminal/pane for the confirm UI text); with bracketed paste on, newlines are kept and nothing runs until Enter; an embedded end marker cannot end the paste early; Edit menu Paste.

Branches: I cancel the confirmation (nothing sent); paste while disconnected; paste into an exited shell; paste over SSH (lab clipboard_ssh.rs); paste while the agent is mid-turn in the agent terminal.

Pointers: terminal/vt/tests/paste.rs (four tests), terminal/pane/tests/pane.rs (a multi-line paste waits for the user), terminal/view/src/view.rs (async paste, pending input FIFO), terminal/view/src/keys.rs (Copy/Paste intents), terminal/lab/tests/interactive.rs (real ⌘V), terminal/lab/tests/clipboard_ssh.rs.

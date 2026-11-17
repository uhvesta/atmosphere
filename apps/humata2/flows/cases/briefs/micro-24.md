# Brief micro-24: Focus follows intent
Output: `apps/humata2/flows/cases/micro/24-focus-follows-intent.md`

Scenario: I press ⌘N, start a session, and type right away; I click the right split and type; I open the palette and close it with Esc; I switch to another app and back; a dialog (fork, move) opens over a terminal and closes.

Steps: a new session's terminal gets input without an extra click; a click on a split focuses that group; after an overlay closes, focus returns to the terminal I was in; an unfocused terminal gets no keystrokes; a focus report reaches a program that asked.

Branches: a toast appears (must not take focus); an Inbox request arrives while I type (must not take focus); the session I was in is archived from elsewhere.

Pointers: desktop/tests/native/terminal.rs (new sessions get input without an extra click; clicking a split focuses that group; an unfocused terminal gets no palette keystrokes), terminal/view/tests/view.rs (focus handoff), terminal/pane/tests/pane.rs (focus reports), ui/src (grep focus_handle / focus).

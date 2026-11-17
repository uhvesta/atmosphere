# Brief micro-13: IME, emoji, wide and right-to-left text
Output: `apps/humata2/flows/cases/micro/13-ime-emoji-wide-and-rtl.md`

Scenario: I type Japanese with an input method (preedit shows underlined, Enter commits), paste emoji including a family emoji and a flag, type Persian text, and use dead keys (´ then e → é).

Steps: preedit is shown and not sent; commit sends UTF-8 once; wide characters take two cells and the cursor lands right; combining marks stay with their base; selection and copy keep them whole.

Branches: IME while the pane is disconnected; IME candidate window position; RTL text shown in logical order (say what the emulator does; no bidi expected); dead keys on Linux.

Pointers: terminal/view/tests/view.rs (IME callbacks), terminal/pane/tests/pane.rs (an input method commits its text), terminal/vt/tests/selection.rs (wide characters), OVERVIEW.md known gap "IME untested in a real window". Status for real-window IME: Fakes only.

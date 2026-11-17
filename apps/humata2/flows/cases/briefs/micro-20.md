# Brief micro-20: Two viewers on one terminal
Output: `apps/humata2/flows/cases/micro/20-two-viewers-on-one-terminal.md`

Scenario: the same shell is open in the desktop app and in a browser tab on another computer. Both type; one is a narrower window; I refresh the browser.

Steps: both see the same output; both may write; each writer hears that another writer is present; the size follows a clear rule; refreshing a viewer keeps the holder and the program running.

Branches: the browser is read-only; one viewer loses its link; an upgrade of the host while both are attached (holders preserved); the web listener itself (OVERVIEW lists the web listener as a gap — check what exists: terminal/lab browser host? AGENTS.md says desktop and browser mirror the same local terminal host).

Pointers: AGENTS.md "Terminal experiment"; apps/humata2/terminal/lab/HANDOFF.md; terminal/lab/tests/*; daemon/humatad/tests/terminal.rs (writers hear about each other, read-only attach); OVERVIEW.md (web listener gap). Say plainly which part is the lab experiment and which is the shipped app.

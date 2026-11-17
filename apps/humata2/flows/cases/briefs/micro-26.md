# Brief micro-26: Drag and drop
Output: `apps/humata2/flows/cases/micro/26-drag-and-drop.md`

Scenario: I drag a file from Finder onto the Files page of a session (upload), onto a shell tab (expect the path to be typed, quoted), and onto the agent terminal or composer (expect it attached). I drag a 2 GB folder.

Steps: Files page upload shows progress and the file lands; a drop on a terminal: find what the code does (grep ui/src and terminal/ for drop / ExternalPaths); a drop on another machine's session copies across.

Branches: a name clash on upload; a drop on a read-only or stopped session; an upload cut by a restart; Upload button (OVERVIEW lists Files "Open in editor" and Upload as gaps — reconcile with the Finder drop test).

Pointers: desktop/tests/live_flows/files.rs (Finder drop upload; copy/move/rename/delete) — real daemon; features/files/tests/*; daemon/humatad/tests/transfers*.rs; ui/src (grep "drop"); OVERVIEW.md gaps.

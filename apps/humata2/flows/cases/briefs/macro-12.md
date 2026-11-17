# Brief macro-12: Files across sessions and machines
Output: `apps/humata2/flows/cases/macro/12-files-across-sessions-and-machines.md`

Scenario: I open a session's Files page, rename a file, move a folder, delete a file (it goes to the Trash), restore it, upload from Finder, copy a log from box-1's session to this Mac, and try Open in editor.

Steps: Files page panes and listing; copy/move/rename/delete; Trash list and restore; transfers list with progress; `humata cp` / `humata mv` from the CLI.

Branches: a name clash; disk full (exit 10); a transfer cut by restart (finished or undone); the other machine is offline; "Open in editor" and Upload button (OVERVIEW gaps); another machine's files (FilesReadOtherMachine MCP — check).

Pointers: desktop/tests/live_flows/files.rs (real daemon), desktop/tests/files_page.rs, features/files/tests/*, daemon/humatad/tests/files.rs, file_ops.rs, trash.rs, transfers.rs, transfer_end.rs, remote_transfers.rs, remote_files.rs, cli_files.rs; workflow/milestones/FILES.md.

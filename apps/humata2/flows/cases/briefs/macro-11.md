# Brief macro-11: Planning on the board
Output: `apps/humata2/flows/cases/macro/11-planning-on-the-board.md`

Scenario: I open the Board (⌘3), add items for the week, drag them between columns, link dependencies, let an agent add and move its own items, and edit an item while an agent edits it too; later I edit from box-1 while the host is away.

Steps: global board vs channel board vs session board; item drawer; create, edit, move, delete; history; agent board tools (board_items_list, create, update, move, delete via MCP) under the board rules; start a session from an item.

Branches: concurrent edit at an older revision (refused, re-read); an agent editing another's item (ChannelBoardEditOthers rule); while the host is away, the box journals the write and it lands when the link is back; an ask on a board edit while away waits in the machine's Inbox; drag and drop vs keyboard moves.

Pointers: desktop/tests/live_boards.rs (32, real daemon), desktop/tests/native/board.rs, features/board/tests/*, features/app/tests/board_start.rs, daemon/humatad/tests/boards.rs, board_tools.rs, away_board_journal.rs, away_approved_board.rs; workflow/milestones/BOARD.md.

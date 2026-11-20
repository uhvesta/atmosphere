# The scope picker and the session panel show the right boards

Layer: verify (Board, the person). Platforms: macOS.

## Starting state

- `#payments` has a channel item PAY-1, and two sessions: "Ledger" (with one fork, "Ledger try 2") and "Refunds".
- Ledger's own board has PAY-2. The fork's board has PAY-3. Refunds' board has PAY-4.
- `#infra` has INF-1. The Global board has GLB-1.
- The app is open on the sidebar's **Board**.

## Reproduce headless

- Scope rules on fakes: `bazel test //apps/humata2/features/board:scope_test`.
- The session's To-do panel, on fakes: `bazel test //apps/humata2/desktop:panels_test --test_filter=board_lists_the_channels_items_and_expands_without_losing_the_session`. Against a real daemon: `bazel test //apps/humata2/daemon/humatad:boards_test --test_filter=a_session_and_its_forks`.
- Panel frames: `bazel test //apps/humata2/desktop:native_test --test_filter=session_lifecycle_and_full_screen_reference_images` (frames 07 and 12). The Desk panel: `bazel test //apps/humata2/features/desk:board_test`.
- By hand on the rig: `humata --json board --global`, `humata --json board --channel payments`, `humata --json board --session Ledger`. Keep the three outputs side by side.
- Not headless: nothing.

## Steps

1. **I do:** read the Global board with scope "All channels".
   **I see:** GLB-1, PAY-1 and INF-1, each with its channel. No session items.
   **State and events:** `ListItems` with the Global-with-channels view.
   **Status:** Fakes only. `features/board/tests/scope.rs::the_global_board_shows_global_and_channel_items_but_no_session_items`.

2. **I do:** open **Scope**, pick **Global items only**; then pick `#payments`.
   **I see:** GLB-1 only; then PAY-1 only.
   **Status:** Fakes only. `features/board/tests/presentation.rs::global_board_includes_channel_items_and_drills_without_leaking_other_items`.

3. **I do:** open `#payments`, **Board** tab. Open **Scope**.
   **I see:** **Channel board**, **Channel + every session board**, "Ledger and its forks", and one entry per session for its own board. Refunds has no "and its forks" entry.
   **Status:** Fakes only. `scope.rs::the_scope_picker_offers_forks_only_for_sessions_that_have_them`.

4. **I do:** pick each entry in turn.
   **I see:** PAY-1; then PAY-1 to PAY-4; then PAY-2 and PAY-3; then PAY-2 alone.
   **State and events:** the daemon walks every descendant fork for "and its forks".
   **Status:** Fakes only in the app: `scope.rs::each_channel_board_scope_shows_its_own_set`. Works (real daemon) for the lists: `daemon/humatad/tests/boards.rs::a_session_and_its_forks_lists_every_forks_board_and_nothing_else`; `clients/live/tests/boards.rs::a_session_board_item_is_listed_on_that_sessions_board`.

5. **I do:** with "Ledger board" picked, click **New item** and make "Spike".
   **I see:** the new item on Ledger's board. With any other scope it goes on the channel board.
   **Status:** Fakes only. `scope.rs::new_item_goes_on_the_session_board_only_in_one_sessions_scope`.

6. **I do:** open the Ledger session and its **Board** panel.
   **I see:** the header names `#payments`. Lanes one under another: In progress, In review, To do, Backlog, Done, empty ones left out. Ledger's own item comes first in its lane, with an accent outline.
   **Status:** Fakes only since the board page went (D874): the panel is now the session's To-do panel. `desktop/tests/panels.rs::board_lists_the_channels_items_and_expands_without_losing_the_session`. Frame 07 on fakes.

7. **I do:** click **Expand panel**.
   **I see:** the lanes side by side, all five, each with a count and **+**.
   **Status:** Fakes only. Frame 12, `desktop/tests/native` `session_lifecycle_and_full_screen_reference_images`.

8. **I do:** open a Desk session's **Board** panel.
   **I see:** every board's items, with what this Desk session added first.
   **Status:** Fakes only. `features/desk/tests/board.rs::every_boards_items_show_once_each` and `::what_the_session_added_is_marked_and_comes_first_in_its_lane`.

## Branches

### The saved session is gone

When the scope I left was "Ledger board" and Ledger is archived. The board reads as the channel board. Ledger's items are hidden everywhere until it is restored.
**Status:** Fakes only for the fallback: `scope.rs::a_saved_scope_whose_session_is_gone_or_archived_reads_as_the_channel_board`. Works (real daemon) for hiding: `boards.rs::an_archived_sessions_items_are_hidden_and_come_back_on_restore`.

### A write refused from the session panel

When I move a card in the panel's full-screen board and the daemon refuses it. No error line shows.
**Status:** Likely bug. `ui/src/board.rs` draws `board.error` at the page foot; `ui/src/panel_board.rs` never reads it. Confirm by a stale move from the panel.

### The CLI's scopes

When I want "Ledger and its forks" from a terminal. `humata board` lists one board only: the channel's, `--global`, or `--session`.
**Status:** Not built, no milestone. `cli/src/commands/boards.rs:55` always sends a single-board view.

## Expected outcomes

- [ ] The Global board never lists session items.
- [ ] Each channel scope shows exactly its set.
- [ ] "And its forks" is offered only for sessions that have forks.
- [ ] A new item goes on a session's board only in that one scope.
- [ ] The session panel shows the channel's board with the session's own items first.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Global, all channels | Fakes only | `scope.rs::the_global_board_shows_…` |
| 2 Global only, one channel | Fakes only | `presentation.rs` |
| 3 The picker's entries | Fakes only | `scope.rs::the_scope_picker_offers_forks_…` |
| 4 Each scope's set | Fakes only; lists Works (real daemon) | `boards.rs::a_session_and_its_forks_…` |
| 5 New item's board | Fakes only | `scope.rs::new_item_goes_on_…` |
| 6 Session panel | Works (real daemon) | `live_boards.rs::the_session_panel_…` |
| 7 Expanded | Fakes only | frame 12 |
| 8 Desk panel | Fakes only | `features/desk/tests/board.rs` |
| Saved session gone | Fakes only; hiding Works (real daemon) | `scope.rs`, `boards.rs` |
| Refused from the panel | Likely bug | `ui/src/panel_board.rs` |
| CLI scopes | Not built | `cli/src/commands/boards.rs:55` |

## Gaps and likely bugs

- Likely bug: a refused write from the session panel's board is silent.
- Not built: the mixed scopes from the CLI and from MCP. `board_items_list` takes one board (`daemon/mcp/src/tools.rs:439`), so an agent can't ask for "my board and my forks'" in one call.
- The scope picker is proven only on fakes. No live test picks a scope and reads the cards.
- A fork's board starts empty (D155), and BD-10 leaves open whether it should. A person forking a session with five planned items sees none on the fork.

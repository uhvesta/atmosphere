# FIND: Find and reach anything

- **Status:** blocked
- **Depends on:**
  - MACH: search runs over the conversation mirror from every machine
  - LOOK2: the palette and the Pinned section are drawn there
- **Unblocks:** V1
- **Layer:** L3 · **Lane:** UI + daemon

## Outcome
⌘K runs every action and searches every session on every machine. Pin things and mark them VIP.

## Exit criterion
Flow §26.22 passes against a real daemon. A search hit opens the turn. An audit test shows every action in the app is in the palette. Pins and VIP marks live on the host and show in two clients at once.

## Read these (pointers, nothing else)
- SPEC: KP-1–KP-6, LD-8, LD-21–LD-25, NV-20–NV-26; flow §26.22. Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: 15, 30 (`desktop/tests/__snapshots__/design/NN-*.png`)
- Decisions: D63, D62 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `features/palette`, `features/palette/tests/search.rs`, `ui/src/overlays.rs`, `ui/src/sidebar.rs`, `daemon/humatad/src/not_built.rs`, `cli/src/commands/looking.rs`
- API: `api/proto/humata/v1/settings.proto` (MarkService); `api/MCP.md` (`sessions_search`)
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
The palette's look (LOOK2). Review this PR from a pasted link (PR).

## Lane (written by the manager)
- **Manager:** minion FIND (Opus 5.5), early lane opened 2026-11-15. Interfaces: scratchpad `find/DESIGN.md`.
- **Early-lane scope (not blocked by MACH/LOOK2):** this host's conversation mirror + full-text index, search and read over gRPC, CLI and MCP; pins and VIP on the host (MarkService, `humata pin|unpin|vip|unvip|pinned`); the app's find results and Pinned state on fakes and the live client. Other machines' mirrors arrive over MACH's relay later; the palette and sidebar drawing stays LOOK2's (FIND hands it view files + seam lines).
- **Design notes:**
  - New crate `daemon/mirror` (`humata2_mirror`): reads each local session's agent conversation file (`AgentLaunch.conversation_file`, Claude JSONL) from a saved byte offset, drops Humata's own tool calls (LD-24), scrubs host variable values (LD-21), appends to `<state>/mirror/<session>/` (durable, crash-safe) and feeds an SQLite FTS5 index at `<state>/search/index.db` with a format version, rebuilt from the mirror when it changes (D62.5).
  - New crate `daemon/marks` (`humata2_marks`): pins, order and VIP in their own SQLite file `<state>/marks.db` (no store migration needed; NV-26 forget is idempotent).
  - New proto `api/proto/humata/v1/history.proto`: `HistoryService { SearchSessions, ReadTurns }` (MCP.md names them). MarkService already in settings.proto.
  - humatad: new `history_service.rs` (catches a session's mirror up on its Session events and at start, never by scanning; agents limited to their own channel until the SESSIONS_SEARCH_OTHER capability seam) and `mark_service.rs` (user-only; publishes TOPIC_MARKS). Wiring lines to main as seams.
  - App: `clients/src/{history,marks}.rs` traits + fakes; `clients/live/src/{history,marks}.rs`; new crate `features/find` (search + pinned reducers); `ui/src/{palette_hits,sidebar_pinned}.rs` for LOOK2 to place.
- **Claims:** new `daemon/mirror/`, new `daemon/marks/`, new `api/proto/humata/v1/history.proto`, new `daemon/humatad/src/{history_service,mark_service}.rs` + `daemon/humatad/src/history_service/`, new `daemon/humatad/src/mcp/history.rs`, new `daemon/humatad/tests/pending/{history,marks,history_tools}.rs`, new `cli/src/commands/{search,marks}.rs`, new `cli/tests/{search,marks}.rs`, new `clients/src/{history,marks}.rs` + `clients/src/{history,marks}/`, new `clients/tests/pending/{history,marks}.rs`, new `clients/live/src/{history,marks}.rs`, new `features/find/`, new `ui/src/{palette_hits,sidebar_pinned}.rs`, this Lane, OVERVIEW block "Pick up here (minion FIND)", DECISIONS D17x (next free at write time). Seams to main as scratchpad `find/seams_*.py --check`: root `Cargo.toml` members, `daemon/humatad/{BUILD.bazel,src/main.rs,src/serve.rs,src/not_built.rs,src/events.rs,src/mcp.rs}`, `daemon/mcp/src/tools.rs`, `clients/src/lib.rs`, `clients/live/src/lib.rs`, CLI registration (`cli/src/{args,commands/mod}.rs`), palette/sidebar lines (LOOK2).
- **Tasks:** (TDD: Fable 5.2 high writes the failing test, Sonnet 5.5 medium implements to it, manager reviews)
  - [x] T1 mirror crate: transcript reader, scrubber, mirror files, FTS index, search/read · tests `daemon/mirror/tests/` · files `daemon/mirror/`
  - [x] T2 marks crate: set/get/pinned/reorder/forget, revision · tests `daemon/marks/tests/` · files `daemon/marks/`
  - [x] T3 app: History + MarksClient traits and fakes, `features/find` search + pinned reducers · tests `clients/tests/pending/`, `features/find/tests/`
  - [ ] T4 humatad HistoryService + MarkService on a real daemon (boundary tests in `tests/pending/`), history.proto · after T1, T2
  - [ ] T5 CLI `humata sessions search`, `humata session read`, `humata pin|unpin|vip|unvip|pinned [reorder]`; MCP `sessions_search`, `session_read` · after T4
  - [ ] T6 live clients + view files (palette Conversations section, sidebar Pinned) + seams for LOOK2 · after T3, T4
- **Waiting on:** MACH relay (other machines' mirrors), LOOK2 (palette/sidebar drawing).
- **Build gates:** none yet
- **Ready to commit:** no

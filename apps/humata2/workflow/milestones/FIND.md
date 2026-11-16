# FIND: Find and reach anything

- **Status:** ready to close
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
  - [x] T4a MarkService on a real daemon: `seams_t4a.py --skip-cargo` applied 2026-11-15 (minion FIND, second manager); `//apps/humata2/daemon/humatad:marks_test` 16/16 live. humatad `Cargo.toml` humata2-marks line waits for main's next repin.
  - [x] T1 follow-up: mirror page tokens tagged with FNV-1a 64 over the request's JSON (Fable ruling; was std DefaultHasher, not stable across toolchains); `a_page_token_from_an_earlier_build_still_opens`; catch_up's empty skip and `a_catch_up_that_adds_nothing_writes_nothing` now built and passing.
  - [x] T4b HistoryService on a real daemon: `seams_t4b.py --skip-cargo` applied 2026-11-15; `//apps/humata2/daemon/humatad:history_test` (19) live, marks_test and humatad2_unit_test green. Agents limited to their own channel (SEAM SESSIONS_SEARCH_OTHER); machine_name = machine id and an empty scrubber are seams. humatad `Cargo.toml` humata2-mirror line waits for the repin.
  - [x] T5a CLI `humata sessions search`, `humata session read`, `humata pin|unpin|vip|unvip|pinned [reorder]` · `cli:search_test`, `cli:marks_test` green
  - [x] T5b MCP `sessions_search`, `session_read` (`daemon/humatad/src/mcp/history.rs`, `find/seams_t5b.py`) · `history_tools_test` 11/11, equal to gRPC (LD-25)
  - [x] T6a live clients `clients/live/src/{marks,history}.rs` · `marks_live_test` (incl. two clients at once, restart), `history_live_test`, `live_unit_test` green
  - [x] T6b view files: `ui/src/palette_hits.rs` (hits list, cursor after the entries, "more" row → FindAction::More), `ui/src/sidebar_pinned.rs` (FakeMarks seeded from the workspace; RowKey::Pinned gone), `ui/src/conversation_tab.rs` (seams_t6_views.py, seams_conversation_tab.py) · ui:all, ui_unit_test green; frames 15/25 re-recorded by the recorder
  - [x] T7 flow §26.22 (`desktop/tests/live_flows/find.rs`, real humatad2: move asks inline, a search hit opens the Conversation tab at the turn, PR link → ReviewPull, Long lines setting revealed, pin shows in two clients) and the KP-1 audit (`features/app:every_action_test`) · green
  - [x] Paging: `FindState.next_page`, `FindAction::{More, MoreLoaded}` · `features/find:search_more_test`
  - [x] D186: a hit opens the turn (`SearchHit.seq`, `FindDelegate::OpenTurn`, `features/session/src/conversation.rs`) · `session:conversation_test`, `app:find_conversation_test`
  - [x] Palette for 26.22 + KP-1: settings sections/rows, Back, Shortcuts, Views, Machine, ChannelBoard, MoveSession (inline ask, ⌥⌘M in keys.rs + shell.rs), ReviewPull, hits cursor · `palette:act_test`, `palette:hits_cursor_test`
  - [x] D189 remote mirror: host pulls other machines' mirrors over the relay (`history.proto` ReadMirror, `history_service/{remote,source}.rs`, `mirror/src/source.rs`; `seams_remote_mirror.py`) · `humatad:remote_history_test` (3), `mirror:remote_source_test` (9), relay/remote/history tests green; `humatad2_unit_test` (refusal unit test) green 2026-11-16
  - [x] Gaps 1–5 (main, 2026-11-16): palette Pin/Unpin and Mark/Clear VIP for the session or channel beneath it (`Marked`, `Command::{Pin,Vip}`, `PaletteState.{channel,marks}`; not added to recent); channel page's Pin icon pins and unpins (`ui/src/channel.rs` `pin_icon`; frames 18–20 view text re-recorded, PNGs identical); Up/Down move the palette cursor (`overlay_palette.rs`, function flag ignored); D197 Review this PR only for a PR a session here reviews; palette Move on a channel session runs the session's Move dialog (StartMove → MovePick → ConfirmMove → `daemon.move_session`) · `palette:marks_test`, `palette:review_pull_gate_test`, `palette:act_test`, `app:marks_test`, `app:move_from_palette_test`, `app:every_action_test`, `desktop:native_test` `marks::`
- **Waiting on:** nothing; humatad2_unit_test green in main's 2026-11-16 batch.
- **Build gates:** none yet
- **Ready to commit:** yes
- **Known gaps:** highlights come from the query words; offline flag passed by the caller; PR-review pins open Changes; the daemon's channel-session MoveSession is FORK's (MV-2, not_built until it lands; the app side is done); Review a pull request dialog is the PR milestone's (D197 hides the entry until then); VIP's palette icon is Bell (the design draws VIP as a text badge, no glyph); remote machine_name is the id; scrubber empty until SetHostVariable; local mirror truncation gap; D189 open item MV-2/LD-4 carried history offset; headless paste into the palette unproven; live_flows_test lacks palette/find/settings deps, so the flow compares Debug names.
- **GAPS_REVIEW note:** FIND: ⌘K runs every action (KP-1 audit), finds turns on every machine (D186, D189) and opens them in a Conversation tab; pins/VIP live on the host. Gaps as listed above.

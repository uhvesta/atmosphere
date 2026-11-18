# NOPATH: I pick repos and machines, never folders: Humata places everything under roots I set once

**Owner (2026-11-16), verbatim:** "I never want to be picking folders. The app should be opinionated about how it handles for me. I should be picking higher level things like repos I need, machines I need. I shouldn't need to know where folders are or where code is or where something is. The app should be guiding me and showing me the least possible information I need to make a decision. If I want to move files between machines it should show me potential machines to move it to and not where on those machines UNLESS I ASK FOR IT TO MOVE IT SOME PLACE SPECIFICALLY. When I ask to sync sessions it should sync it for me to where it needs. When I ask to make a spec it should pick a folder to make it for me. These things should be somewhat configurable as like the 'roots' but once those roots are set they should be opinionated."

**Exit:**
- An audit of every path pick and every path shown (spec, flows, frames, UI, CLI/MCP).
- SPEC rules for:
  - roots per machine in Settings;
  - machine-only file moves, with "Somewhere specific…" as a secondary option;
  - session moves and syncs that go where they need without asking;
  - paths hidden by default.
- Flows reworked; D460–D469; golden frames that show paths listed for the owner's wave.
- Code and tests.

**Split:** DOCV owns DK-2, DK-3 and DK-14. ROAM's D315 and frame 96 become moot.

## Lane

**Manager:** minion NOPATH-2 (Opus 5.5), from 2026-11-16, after the first
manager (audit, s1 applied; s1b, b1, b2, b3, b4 queued). Scripts and audit
live in the session scratchpad `nopath/` (AUDIT.md, applylib.py,
mkhunks.py, mksim.sh). s1b, b1, b2, b3 and b4 are live now. Simulation tree:
`nopath/n2sim/app` = a copy of the live tree (`mksim.sh DEST [n2 scripts…]`);
every n2 script is validated on copies of it.

**Exit (NOPATH-2):** every gap below is code and tests, live where a daemon
is involved (daemon/humatad tests, clients/live tests, desktop live flows),
micro/27's Status rows say Built, and main's gate is green for the targets
listed per script.

**Decisions:** D540–D549 (Provisional), written by `n2_decisions.py`.

### Tasks (one apply script each, applied in this order after b4; SAFE's `w2_storage` goes before 6)

| # | Script | Gap | Worker | Status |
|---|---|---|---|---|
| 1 | `n2_keep_both.py` | keep-both names `-2` (FC-14, PL-13) in the engine and the fake | Haiku | queued to main |
| 2 | `n2_received.py` | received root: files.proto, access Root, daemon roots.rs, live client (PL-3, PL-4, D540) | Sonnet | queued to main |
| 3 | `n2_session_paths.py` | Session carries its folder and clones' paths; shared/<session> follows the folder (PL-3, D541) | Sonnet | queued to main |
| 4 | `n2_files_page.py` | desktop/tests/files_page.rs uses Copy to ▸ (PL-5) | Haiku | queued to main |
| 5 | `n2_d461_tests.py` | desk_into_channel cases follow D461 (the old tick is ignored) | manager | queued to main |
| 6 | `n2_change_root.py` | This Mac's stored root; plan lists channels and running sessions (PL-1, FC-10, D543) | Sonnet | queued to main |
| 7 | `n2_inbox_copy.py` | Inbox cross-machine copy card IN-10, daemon to UI (D467, D546) | Sonnet | queued to main |
| 8 | `n2_decisions.py` | D540–D546 | manager | queued to main |
| 9 | `n2_reveal.py` | Reveal in Finder / Open terminal here / Copy path in Files rows and Info clones (PL-10, D545) | Sonnet | queued to main |
| 10 | `n2_files_page_target.py` | `desktop:files_page_test` target | manager | queued to main |
| 11 | `n2_root_move.py` | shared Layout; ChangeRoot move_channels=true with undo (D544) | Sonnet | queued to main |
| 12 | `n2_open_in.py` | Open terminal here opens a shell tab in that folder | Sonnet | queued to main |
| 13 | `n2_box_root.py` | ChangeRoot on a box relayed to its own daemon, User via Host only | Sonnet | queued to main |
| 14 | `n2_path_rows.py` | Copy path / Reveal on the Inbox copy card's Details | Haiku | queued to main |
| 15 | `n2_root_ui.py` | Settings › Change… sheet: plan, Move them / Use for new channels only | Sonnet | rebased on round 23; queued to main |
| 16 | `n2_same_bytes.py` | a byte-equal keep-both copy is skipped; the tray says saved as / already there (PL-13) | Sonnet | queued to main |
| 17 | `n2_box_follow.py` | after a box move the host rewrites its Desk session records; the relayed operation exists before the call and resumes after a restart | Sonnet | queued to main |
| 18 | `n2_status.py` | micro/27 Status rows, this Lane | manager | last |

### Claims (hunks through scripts only; SEAM = shared file, main applies)

Per script, in its docstring and SEAM list. Shared files expected:
`api/proto/humata/v1/{files,sessions,inbox}.proto`, `clients/src/{lib,model}.rs`,
`clients/src/file_manager.rs`, `daemon/humatad/BUILD.bazel`,
`daemon/access/src/*`, `ui/src/lib.rs`.

### Log

- 2026-11-16 NOPATH-2 opened; s1b, b1–b4 found applied live; sim tree = live copy.
- 2026-11-16 Scripts 1–5 queued; gate 20 ran before the D461 cases; the gate 22 rerun is with main.
- 2026-11-16 ChangeRoot built: plan, move with undo, new channels only, This Mac's root kept across restarts (6, 11); SAFE's `w2_storage` must apply before 6.
- 2026-11-16 Inbox copy card, Reveal / Open terminal here / Copy path, and the files page queued (7–10, 12).
- 2026-11-16 Box roots relayed over the peer path, the Change… sheet, card path rows queued (13–15); the chain 1–15 was checked on a fresh live copy.
- 2026-11-16 The PL-13 byte-equal skip, found missing while writing the micro/27 statuses (16); box session records follow a box move and the relayed operation is made before the call (17).
- 2026-11-16 micro/27 says Built for every step, with test targets as evidence (18).
- 2026-11-16 PLACES took over: n2_reveal, n2_root_ui, n2_box_follow and n2_status were already live or queued.
  Script 72 hides the channel Desk bar's path behind Copy path (PL-10). It also adds
  `humata cp|mv <location>… <machine>`, which places each source by PL-3 and keeps both by
  default (PL-12, PL-13, D655), with cli tests. The MCP tools copy only into the agent's own
  clone or `shared/` (TP-4), so a machine-only form is not needed there.

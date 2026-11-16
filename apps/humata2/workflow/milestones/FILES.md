# FILES: Move files and keep disks tidy

- **Status:** early lane open (lane FILES)
- **Depends on:**
  - MACH: transfers ride the machine link; Storage is per machine
  - FRAMES2: the file manager, transfers tray and Storage have no frame yet
- **Unblocks:** SAFE
- **Layer:** L3 · **Lane:** daemon + UI

## Outcome
Use a two-pane file manager, run a transfer that survives a dropped link, see Storage per machine, restore from trash, and turn on retention rules.

## Exit criterion
Flows §26.7 and §26.15 pass against real daemons. A transfer cut at any chunk resumes or undoes (PER-27 fault points, here). Low disk shows only where it blocks and on Hosts.

## Read these (pointers, nothing else)
- SPEC: FM-1–FM-15, LD-5–LD-7, LD-13–LD-22, TR-11, TR-13, RV-9–RV-14; flows §26.7, §26.15. Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: 06, and the file manager and Storage frames from FRAMES2 (`desktop/tests/__snapshots__/design/NN-*.png`)
- Decisions: D56, D60, D65, D107, D12 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `chunk`, `daemon/humatad/src/files.rs`, `daemon/humatad/src/compression.rs`, `daemon/humatad/tests/files.rs`, `cli/src/commands/files.rs`, `cli/tests/files.rs`, `features/files`, `clients/src/files.rs`
- API: `api/proto/humata/v1/files.proto` (FileService, TransferService, TrashService)
- Also: OVERVIEW.md › Known gaps › "Compression"
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
Moving a whole session (FORK). The Files panel's look (LOOK1).

## Lane (written by the manager)

Early lane (MACH open): everything one machine can do alone, plus the app
side on fakes. Cross-machine transfers wait for MACH's link (requests below).

- **Goal (early part):** FileService on every Humata root (list, stat, new
  folder, rename, delete to trash), TrashService (list, restore, plan, empty,
  30-day expiry), TransferService on one machine (copy, move, upload with
  resume, pause, cancel), each finished or undone after a kill or power cut
  at any fault point (PER-27); the file manager, tray, trash and Storage
  state and views on fakes.
- **Design notes:**
  - New crate `daemon/transfers` (`humata2_transfers`), no gRPC, sync, like
    `daemon/sessions`: `trash` (one folder per item under
    `<channels>/.trash/<id>/{item.json,content}`; put, list, restore, plan,
    empty, expire, recover) and `engine` (a journal per transfer in
    `<state>/transfers/<id>.json`; files copied in 1 MiB chunks into a hidden
    staging folder `.humata-transfer-<id>` inside the destination, fsynced
    and journaled every 8 MiB or 64 files; each file re-read and its SHA-256
    compared before it counts; top-level names renamed into place at the end;
    a move on one file system is a rename, otherwise copy then source to
    trash). Cancel or undo removes the staging folder, so nothing half-made
    is ever visible. Recovery at start: running → resumes from the journaled
    offset, committing → finishes, cancelling → finishes the cancel, uploads →
    stay paused for the app to resume. Ends are behind a small `End` trait so
    a gRPC end (FileService over the peer link) plugs in after MACH.
  - Fault points with `humata2_store::fault::point`, killed one by one in
    `daemon/transfers/tests/kill_matrix.rs` (pattern: `daemon/sessions`),
    with the power-cut simulation of `humata2_durable`.
  - humatad stays wiring: `files.rs` (all roots, the new methods),
    `trash_service.rs`, `transfer_service.rs`; black-box tests in
    `daemon/humatad/tests/files.rs`, including a daemon killed mid-transfer.
  - App: `clients/src/files.rs` gets a `FileManager` trait with an in-memory
    fake and its own types (no model.rs edits); `features/files` becomes the
    two-pane file manager, tray, trash and Storage reducer; new views in
    `ui/src/{file_manager,transfers_tray,storage_view}.rs` to match frames
    61 and 60.
- **Claims:** see WORKLOG (FILES row).
- **Keep the tree building (CONFIG):** all new code sits in new files nothing
  includes yet; `daemon/transfers` targets are `manual` until the visibility
  seam; new daemon tests wait in `daemon/humatad/tests/pending/`; the
  `features/files` manager tests are excluded in its BUILD. The wiring step
  (main's seams, then the manager's flip of `files.rs`, `features/files/src/lib.rs`,
  the test moves and the BUILD exclude) comes last, in one go.
- **Tasks:**
  - [x] T1 `daemon/transfers` crate: trash + engine + uploads + fault points + kill matrix · worker engine (Sonnet, medium) · written, checked, not built
  - [x] T2 app core: `clients/src/file_manager.rs`, `features/files/src/manager.rs` + `manager/`, tests `tests/manager_{fake,panes}.rs` · worker app-core · written, checked, not built
  - [x] T3r the resolver for every root: `daemon/humatad/src/files/roots.rs` · manager · written
  - [x] T3a FileService list, stat, new folder, rename, trash (`src/files/ops.rs`), TrashService (`src/trash_service.rs`), tests `tests/pending/{file_ops,trash}.rs` · worker wiring-files · written, checked (manager added `Target::unfollowed` so a link itself can be renamed or trashed), not built
  - [x] T3b TransferService and Upload (`src/transfer_service.rs`), tests `tests/pending/transfers.rs` incl. a daemon killed mid-transfer · worker wiring-transfers · written, checked (manager added: a destination inside a source is refused), not built
  - [x] T4 views: `ui/src/{file_manager,transfers_tray,storage_view}.rs` · worker views · written, not built; differs from frame 61 (see Open)
  - [ ] W wiring, staged: scratchpad `files-wiring/WIRING.md` (main's seams) + `files-wiring/flip.py` (manager's files): A engine (applied by main), B humatad, C app core, D views, S storage + retention in the crate, F FindFiles, G humatad storage, H home root · waiting on main
  - [x] T5a Storage measuring: `daemon/transfers/src/storage.rs` + `tests/storage.rs`; humatad `src/storage.rs` (report, ClearBuildCache, TrimLogs; unit tests inside) · worker storage · written, not built (inert); MACH's handlers call it
  - [x] T5b retention decisions: `daemon/transfers/src/retention.rs` + `tests/retention.rs` (pure; `pub mod retention`) · worker retention · written, checked, not built (inert); carrying them out waits for ArchiveSession and activity times
  - [x] T5c FindFiles: `daemon/humatad/src/files/find.rs` + `tests/pending/find_files.rs` · worker find · written, checked (manager: a clone not on disk is skipped), not built (inert)
  - [x] proto seams: scratchpad `files-wiring/PROTO.md` (part 1 the home root with `flip.py H`; part 2 the `TransferEndService` proposal for T7, not to apply yet)
  - [ ] T6 `humata` runs against the wired services in the daemon tests (the CLI already speaks all three services)
  - [ ] T7 (after MACH, D139 relay) a gRPC `End` over the relayed FileService, host-driven transfers, waiting-for-machine, flows §26.7 and §26.15 against the stdio peer
    - From MACH (2026-11-15, written there, not gated): `host_relay::Links` (Clone, handed over by serve.rs): `links.channel(machine) -> Result<Channel, RelayError>` (never waits; `RelayError::Offline` converts to the machine-offline status), `links.relayed(machine)?.file()` / `.trash()` (generated clients), `links.watch(machine) -> watch::Receiver<LinkState>` with `LinkState::{Down, Up { generation }}`, `links.reopen_on_new_link(machine, |channel, generation| async …)` for "waiting for X, continue on reconnect". Requests over the link are fresh `tonic::Request`s (never copy the caller's metadata); `host_relay::assigned(message, Some(id))` for a host-assigned id. The machine takes a relayed call as the user's (D144), so the host must check access before it relays. The relay allowlist covers all of FileService and TrashService by prefix; `TransferEndService` is one line there once its proto lands (tell MACH).
  - [ ] T8 power-cut pass on the kill matrix (PER-28, `HUMATA2_POWER_CUT_ROOTS`)
  - [x] T9 views to frame 61/60 · worker views2 · written, not built (inert): times come from each listing's read time and offset (pure `manager/when.rs`), names are entered in a prompt (`naming`), the machine picker is a popup (`picker`), no system clock in any view; tests `manager_{when,naming}.rs`; headless flow test `desktop/tests/files_page.rs` (target lines in WIRING.md). Left against frame 61: the selected folder's children aren't drawn as a tree; the drop hint shows under every loaded pane; the arrows flip when the right pane is active
- **Plan to the exit (2026-11-15, after 781bcf58: stages A, S, B, C, D committed):**
  1. Written, for main to apply: F (FindFiles), G (`mod storage;`, then MACH's handlers), H (home root; PROTO.md part 1), and PROTO.md part 2 (`TransferEndService`, additive).
  2. T7 cross-machine, written inert (D156): T7a the machine's half `src/transfer_end_service.rs` + `tests/pending/transfer_end.rs` (worker far-end; stage T in WIRING.md, needs only PROTO.md part 2); T7b the host's half `src/remote_end.rs` (`RemoteEnd`, `RouterEnd`, virtual paths, offline → waiting, resume on link-up by link generation, no timer) + `tests/pending/remote_transfers.rs` (flows §26.7 and §26.15 at the daemon level) (worker remote-end). Stage R = scratchpad `files-wiring/flip_r.py` (`--check`; 35 exact edits to the engine crate's `error.rs`/`engine.rs`, `files/roots.rs` (`Roots::place`), `files.rs`, `transfer_service.rs`, `main.rs`, and the test move) + the `serve.rs` seam in `T7-hooks.md`. It needs stage T, MACH's relay live end to end and `TransferEndService` on its allowlist. Checked on copies: R still applies after F, H and T, and the home-root arm ends up inside `place`. A transfer to a machine that is offline is accepted and waits.
  3. The live `FileManager` client, written inert in FL's crate (main's OK): `clients/live/src/file_manager.rs` + `clients/live/tests/file_manager.rs` (worker live-files); stage L, seam lines for main and FL in scratchpad `files-wiring/LIVE.md` (after stage H). Three stopgaps noted there and in D156 (UTC offset by `date +%z`, a 300 ms wait to catch a disk-full start, a reopen backoff). Then the two flows through the app in `desktop/tests/live_flows/` (not written).
  4. PER-28: the power-cut pass on `daemon/transfers/tests/kill_matrix.rs`.
  5. Retention carried out: needs ArchiveSession (FORK), retention rules in SettingsService and activity times in the store (a migration). FILES has the decision function and the Storage skips. Main, 2026-11-15: FILES closes without the runner; a retention follow-up goes in the DAG behind FORK.
  6. Small: sweep interrupted cache clears at start; logs reopened after a trim; `humata` runs in the daemon tests; tests and a CLI spelling for the home root; frames 61/60 drawn from the real views with the folder tree (after LOOK2).
- **Scope (owner, 2026-11-15, later the same day): no "one big file system".** The tree across machines, inline expanding and drag between panes were dropped before anything was written (the worker was stopped; it had created no files). FILES is the simple Files page that exists: two panes, a machine picker each, new folder, rename, Copy, Move, delete to trash, files dropped in from Finder, over FileService and transfers. An OS-level mount is a separate milestone (MOUNT). What the simple page still needed, and now has written:
  - *Another machine's pane.* The host refused file and trash calls for a location on another machine. Stage V: `daemon/humatad/src/files/forward.rs` + `tests/pending/remote_files.rs`, wiring `files-wiring/flip_v.py` and two `serve.rs` lines (after stage R). Trash item ids from another machine come back as `<machine>:<id>` so a restore finds its machine with no proto change.
  - *A drop from Finder.* The view only drew the hint. `ui/src/file_manager.rs` now takes files dropped on a pane and sends the existing `Dropped` action (uploads). Stage R gained three edits so an upload lands on another machine too.
  - *A new folder with a taken name.* The daemon returned the existing folder as success; it now refuses, as the page expects (g59).
  - *Proof on a real daemon.* `clients/live:file_manager_test` (one machine), `remote_transfers_test` and `remote_files_test` (two machines, at the daemon), and an app flow `desktop/tests/live_flows/files.rs` with `files-wiring/seam_flow.py` (worker flow, running).
- **Main clones by where the path lands (D165, Fable's pick: refuse; 2026-11-15):** `daemon/humatad/src/files/roots.rs` follows a path named through the home folder or a channel's folder and, if it is a main clone, in one, or (for a rename, delete or move away) a folder holding one, hands the policy that main clone, so every caller gets the main-clone refusal; listing, copy out and new things beside the clones are unchanged. Tests: `file_ops.rs` (two new), `transfers.rs` (one new). The refusal's own fix line (a session's clone, or remove the repository from the channel) is a seam for the access crate and `status.rs`: `files-wiring/seam_mainclone.py`.
- **T6, T8 and the g70 start failures (2026-11-15):** T6 `daemon/humatad/tests/pending/cli_files.rs` + `files-wiring/flip_t6.py` (13 tests through the `humata` binary; the CLI's requests fit the daemon; one daemon fault found and fixed live: a too-late cancel answered as a malformed request, so the CLI exited with its usage code, now the not-cancellable kind). T8 `daemon/transfers/tests/power_cut.rs` + `files-wiring/flip_t8.py` (the power-cut matrix, and five ordering faults it found in the engine crate, fixed in the script). g70's "humatad2 never started listening" (10 of 21 in transfers_test): not the daemon, which binds before it opens its store; the first ten tests, one per core, all spent the 15 s limit in the first start of a just-linked 95 MB binary under a heavy build, and the next eleven passed in five seconds. `files-wiring/seam_harness.py` pays that first start once per test process, outside the limit, and makes a failed start say whether the process exited or is still starting.
- **Open picks:** D142, D145, D156 (Provisional). A copy of a root itself isn't served yet (only things in it). Rename is check-then-rename (no NOREPLACE). A case-only rename on a case-insensitive disk is refused as taken.
- **Waiting on:** main, in order (all in scratchpad `files-wiring/`, each with `--check`): `seam_harness_note.py` (comment), `seam_flow.py` (the app's Files flow on a real daemon), then T7's remaining stages (R and V, now applied). Applied so far: A, S, B, C, D, F, G, H, both proto parts, T, L, the main-clone seam, the harness warm-up, T6, T8. MACH: its GetStorage, ClearBuildCache and TrimLogs handlers calling `crate::storage::Storage`.
- **Build gates:** g35 A + S pass (retention_test after one import fix); g37 B: build clean, 2 test failures fixed in place (a watcher heard the internal cancelling step; a test expected a link itself to be unrenamable); g38 C and D pass after main's 4 ui compile fixes; committed 781bcf58. Stages F, G, H, T, L, R: not built.
- **Stages R and V (2026-11-15, applied by the manager, logged in WORKLOG):** `flip_r.py` + `seam_serve.py R`, `flip_v.py` + `seam_serve.py V`; `tests/pending/{remote_transfers,remote_files}.rs` moved into `tests/`. Faults found and fixed on real daemons:
  - *A commit on another machine failed.* `RemoteEnd::rename` sent the destination with its machine still named, so the far end refused it as not relayed; the far end now gets its own path (`remote_end.rs`).
  - *A lost link copied the file again from byte 0 (FM-8).* The journal now keeps `confirmed`, the offset the destination last synced; on resume the engine tries the journaled offset, then `confirmed`, then 0, each only if the prefix still hashes the same (`transfers/src/{record,engine,upload}.rs`; test `transfers/tests/link_lost.rs`, target `link_lost_test`).
  - *A lost link could end Failed instead of waiting, by a race.* The call can hear the broken connection before the link supervisor marks the link down; a status tonic built from a transport error (it has a source) now counts as offline too (`remote_end.rs`, `files/forward.rs`, unit tests in both).
  - *Build:* `transfer_end_service.rs` maps `EndError::Offline` to the machine-offline status; a racy `resume_when_back` unit test now lets its task subscribe before the links change.
- **Gate (manager, 2026-11-15):** built `humatad2`, `humatad2_admin`, `desktop:humata2`; passed `humatad:{humatad2_unit_test,remote_files_test,remote_transfers_test,file_ops_test,trash_test,files_test,transfers_test,transfer_end_test}` and `//apps/humata2/daemon/transfers:all` (kill matrix and power cut included): 16 of 16. Flows §26.7 and §26.15 pass against real daemons (`remote_transfers_test`).
- **GAPS_REVIEW note:** FILES R+V: host-driven transfers and file calls to other machines, resume from the confirmed offset after a link loss, offline judged from the status itself; open: retention runner (behind FORK), MACH's GetStorage/ClearBuildCache/TrimLogs handlers, the optional harness tidy-ups in `files-wiring/harness-needs.md`.
- **Ready to commit:** yes (main commits; the lane is ready to close)

**Closed 2026-11-16** in freeze 2 (0bdf1434) after gate g80; release humata2-v0.1.0 installed in /Applications. Open gaps are carried in GAPS_REVIEW.md (freeze 2 entry).

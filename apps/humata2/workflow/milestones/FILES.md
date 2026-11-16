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
- **Open picks:** D142, D145 (Provisional). A copy of a root itself isn't served yet (only things in it). Rename is check-then-rename (no NOREPLACE). A case-only rename on a case-insensitive disk is refused as taken.
- **Waiting on:** main: the staged wiring and gates A–D. MACH (via main): the relay allowlist to carry all of FileService and TrashService; a per-machine channel to the relayed FileService for the host's TransferService; a pushed link up/down signal per machine. Main (proto): a resumable remote write (`WriteFileHeader` transfer id + offset, and a way to ask a partial's confirmed offset) and the FM-10 home root.
- **Build gates:** none yet (batch 1 reported 2026-11-15, nothing compiled)
- **Ready to commit:** no

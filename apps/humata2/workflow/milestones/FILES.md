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
  - [ ] T1 `daemon/transfers` crate: trash + engine + fault points + kill matrix · worker engine (Sonnet, medium) · `daemon/transfers/**` · running
  - [x] T2 app core: `FileManager` client + fake (`clients/src/file_manager.rs`), reducer in `features/files/src/manager.rs` + `manager/{pane,edit,tray,trash,storage}.rs`, tests `tests/manager_{fake,panes}.rs` · worker app-core (Sonnet, medium) · written, checked by the manager, not built (inert)
  - [x] T3r the resolver for every root: `daemon/humatad/src/files/roots.rs` · manager · written, inert
  - [ ] T3a FileService list, stat, new folder, rename, trash (`src/files/ops.rs`), TrashService (`src/trash_service.rs`), tests in `tests/pending/{file_ops,trash}.rs` · worker wiring-files (Sonnet, medium) · running
  - [ ] T3b TransferService and Upload (`src/transfer_service.rs`), tests in `tests/pending/transfers.rs` incl. a daemon killed mid-transfer · after T1
  - [ ] T4 views: file manager, tray, Storage (frames 61, 60) · worker views (Sonnet, medium) · new `ui/src/{file_manager,transfers_tray,storage_view}.rs` · running
  - [ ] T5 Storage measuring and retention rules (LD-13–LD-18) in `daemon/transfers`; FindFiles (RV-11) · after T3
  - [ ] T6 CLI: `cli/src/commands/files.rs` already speaks all three services (`cli/tests/files.rs`); add `humata` runs to the daemon tests after wiring
  - [ ] T7 (after MACH) the gRPC `End`, host-relayed transfers, waiting-for-machine, flows §26.7 and §26.15 against the stdio peer
  - [ ] W wiring step: seams to main, flip `files.rs` (use `roots`, delegate to `ops`), flip `features/files/src/lib.rs`, move `tests/pending/*` up, drop `manual` and the BUILD exclude
- **Waiting on:** main for seam lines (listed in each batch report); MACH for FileService and TrashService on the peer link and a per-machine channel plus link up/down signal.
- **Build gates:** none yet
- **Ready to commit:** no

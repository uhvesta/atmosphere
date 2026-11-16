# LINUX: The same app on Linux

- **Status:** blocked
- **Depends on:**
  - SAFE: the owner's rule: the Linux batch runs only after all infrastructure work
  - ASK: its boundary suites are part of the batch
  - BUG: its suites are part of the batch
- **Unblocks:** ANYHOST
- **Layer:** L5 · **Lane:** exclusive (nothing else builds)

## Outcome
Run Humata 2 on Linux and get the same screens and the same guarantees.

## Exit criterion
The Linux VM batch (2 CPU, 4 GiB, 20 GiB) is green: every non-image suite, the view-tree snapshots, a launch smoke test (D129), real-SSH machine flows, and VM hard-reset cases.
- The containment suite (SEC-6, from AWAY) passes in the VM. (D128)

## Read these (pointers, nothing else)
- SPEC: PER-18, PER-19, PER-28, TR-2. Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: no Linux pixel goldens (D129). The smoke test launches the real binary under X11 and Wayland and asserts frame 01 is non-blank, bundled fonts load with no missing-glyph boxes, and it renders at scale 2. Check first whether GPUI draws in the GPU-less static-musl VM (`testing/linux/run.sh`); it may need glibc and a software renderer.
- Decisions: D18 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `testing/linux/run.sh`, `daemon/machines/providers/lima.toml`, `daemon/machines/testing`, `testing/README.md`
- Also: OVERVIEW.md › N1 item "Check the goldens in a browser"; Known gaps › "Testing infrastructure"
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
New features. Fix only what the batch finds.

## Lane (written by the manager)
- Design notes:
- Claims: <paths>
- Tasks:
  - [ ] T1 <task> · worker <name> (Sonnet, medium) · files … · status
- Waiting on:
- Build gates: <time> pass|fail (targets)
- Ready to commit: no | yes: <what changed, how a reviewer checks it>

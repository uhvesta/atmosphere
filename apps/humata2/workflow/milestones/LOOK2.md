# LOOK2: Every other screen looks like the goldens

- **Status:** blocked
- **Depends on:**
  - LOOK1: shares the shell, theme and sidebar; and C1 must release the UI files
  - P0: the panels and native tests must be split before a second manager edits them
  - W1 (to close, not to open): frames 18, 29, 31, 32, 33 change; the app matches the redrawn ones before this closes
- **Unblocks:** DESK, BOARD, FIND
- **Layer:** L1 · **Lane:** UI

## Outcome
On made-up data, the owner clicks through the channel page, boards, Home, the Inbox, Hosts, Settings, the palette, New session, toasts and the empty states, and they look like frames 18–35.

## Exit criterion
For each of frames 18–35 the native capture matches the design frame within tolerance and one flow reaches it. After W1: no Panes setting and no Fixed/Simple switch (D111), no model picker, the Last used column (SE-26), and D113 items 5, 7, 8, 9 applied. Platform behaviour holds: standard shortcuts, ⌘, for Settings, the native menu bar. A release of the app on fakes, labelled as such, is installed.

## Read these (pointers, nothing else)
- SPEC: CH-11–CH-15, CH-9, BD-2–BD-5, BD-11–BD-13, HM-1–HM-5, IN-3, IN-5, IN-7, HO-3, ST-18–ST-20, KP-1–KP-3, SE-2, SE-26, NV-7–NV-11, NV-27, DK-6, MV-15; flow §26.22 (on fakes). Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: 18–35 (`desktop/tests/__snapshots__/design/NN-*.png`)
- Decisions: D76, D99, D106, D107, D109, D110, D111, D112 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `ui/src/channel.rs`, `ui/src/board.rs`, `ui/src/pages.rs`, `ui/src/hosts.rs`, `ui/src/settings.rs`, `ui/src/overlays.rs`, `ui/src/lists.rs`, `features/board`, `features/hosts`, `features/inbox`, `features/palette`, `features/settings`, `desktop/tests/native.rs`, `desktop/tests/design/pages.rs`
- Also: OVERVIEW.md › "N1: A native macOS look and feel" (open items)
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Carried over from P0
Split `ui/src/panels.rs` and `desktop/tests/native.rs` the same mechanical way P0 split the others (see the P0 commit 1a3831dd), first, once C1 has released them.

## Out of scope
The Inbox card's full context and grants (ASK). Board behaviour beyond what is drawn (BOARD). Desk frames 36–40 and markdown frames 39, 41–43 (DESK, REVIEW). Palette search across machines and pins (FIND).

## Lane (written by the manager)
- Design notes:
- Claims: <paths>
- Tasks:
  - [ ] T1 <task> · worker <name> (Sonnet, medium) · files … · status
- Waiting on:
- Build gates: <time> pass|fail (targets)
- Ready to commit: no | yes: <what changed, how a reviewer checks it>

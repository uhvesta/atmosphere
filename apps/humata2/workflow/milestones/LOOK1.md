# LOOK1: The session page and sidebar look like the goldens

- **Status:** in progress (lane LOOK1). C1 stopped 2026-11-15 at 9e185a5b: start from `apps/humata2/C1_HANDOFF.md`; C1's OVERVIEW block and items are now this lane's to update.
- **Depends on:**
  - D0: the frames and rules it matches are the tidied ones
- **Unblocks:** LOOK2, FL, BUG, ASK (its exit)
- **Layer:** L0 · **Lane:** UI

## Outcome
On made-up data, the owner clicks through the session page, its panels, tab groups and the sidebar, in light and dark, and they look like frames 01–17.

## Exit criterion
For each of frames 01–17 the native capture matches the design frame within tolerance and one flow reaches it (`desktop:native_test`, `shell_test`, `panels_test`). Regions D113 disputes (the Inbox card in 03/04, "· Opus" in 05, the made-up data in 01, 02, 16) match the current frames until W1. C1 commits its own work; when its claim in WORKLOG is released, the main agent marks this done or opens a manager for what is left.

## Read these (pointers, nothing else)
- SPEC: SE-6–SE-16, SE-18–SE-20, SE-27–SE-29, CH-16, CH-17, NV-2–NV-6, NV-12–NV-19, NV-32–NV-34, NV-36, NV-37, ST-7–ST-12; flow §26.13. Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: 01–17 (`desktop/tests/__snapshots__/design/NN-*.png`)
- Decisions: D76, D74 (as revised), D103, D73 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `ui/src/shell.rs`, `ui/src/sidebar.rs`, `ui/src/session_tabs.rs`, `ui/src/terminals.rs`, `ui/src/panels.rs`, `ui/src/theme.rs`, `features/session/src/lib.rs`, `features/sidebar`, `desktop/tests/native.rs`, `desktop/tests/__snapshots__/native`
- Also: OVERVIEW.md › "N1: A native macOS look and feel" (C1's items and its Pick up here block are C1's, not yours)
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
Channel, board, Home, Inbox, Hosts, Settings and overlays (LOOK2). The Desk row in the sidebar (DESK). The Inbox card's content (ASK). Real terminals (FL).

## Lane (written by the manager)
Manager: minion LOOK1 (Opus 5.5), took over from C1 (9e185a5b) on 2026-11-15.

- **Goal / exit:** as above. Order (owner, via C1_HANDOFF): (1) individual-comment Send plus Send N comments, explicit Send delivers without a turn-end hook (D126, SE-23, SM-9, SM-10); (2) per-frame audit of 01–17 until each native capture matches; (3) other in-scope handoff items.
- **Design notes:**
  - Delivery rule (D126): a session's outbox (`SessionView.queue`) holds two kinds. `Comments` come from an explicit Send: delivered as soon as the session accepts input, one at a time, in order, with no turn check. `Text` from the composer `Submit` is the explicit wait-until-turn-end mode: it auto-sends only on a hook-backed (Claude/Copilot) Ended turn, else waits for Send now. Kind decides policy, so the saved format doesn't change. Failure still pauses that session's automatic retry; payloads stay until acknowledged; removal returns comments to drafts.
  - Review: `Send` stays the batch (Send N comments). New `SendDraft(index)` sends one saved draft and keeps the others; new `SendComment` saves the box being written and sends only it (the box UI itself is REVIEW's, frame 50). Same `Delegate::QueueComments`, so `features/app/src/lib.rs` (seam) needs no change.
  - UI: each Draft card in the Diff panel gets a Send control; the waiting bar says "typed when this turn ends" for held text on a hook agent (frame 03 wording), "sending" for explicit sends in flight, "send failed"/"send manually" otherwise.
  - Out of scope, noted only: the comment box editor and click-a-line (REVIEW, frame 50), Desk row (DESK), Inbox card content (ASK), frames 18+ (LOOK2), 44–55.
- **Claims:** `features/review/src/lib.rs`, `features/review/tests/`, `features/session/src/{lib,message}.rs`, `features/session/tests/queue.rs`, `features/app/src/messages.rs`, `features/app/tests/messages.rs`, `ui/src/{panels,session_tabs,sidebar,terminals,theme,chrome}.rs`, `features/sidebar/`, `clients/src/fixture/{design_session,sessions,requests,terminal,machines,items,journal,files}.rs`, `terminal/pane/src/pane.rs`, `desktop/tests/design/{session,comments}.rs` (owner-approved golden fixes for frames 01/05/50), `desktop/tests/{native,panels,shell}.rs`, `desktop/tests/__snapshots__/{native,panels,shell}/`, OVERVIEW block "Pick up here (minion LOOK1)". Not: seams, daemon/, cli/, desktop/tests/design*, goldens.
- **Tasks:**
  - [x] T1 direct/individual send reducers · worker send-core (Sonnet, medium) · `features/review/src/lib.rs`, `features/session/src/{lib,message}.rs`, `features/session/tests/queue.rs`, `features/app/src/messages.rs`, `features/app/tests/messages.rs` · written, checked
  - [x] T2 Send on each draft card, waiting-bar wording, panels flow · worker send-ui (Sonnet, medium) · `ui/src/{panels,session_tabs}.rs`, `desktop/tests/panels.rs` · written, checked
  - [x] T3 native flows follow direct send · worker send-native (Sonnet, medium) · `desktop/tests/native.rs` · written, checked (manager: removing the last waiting message lifts the failure pause, in session lib.rs + messages test + native retry flow)
  - [x] T4 frame audit 01–17 (read-only report: each difference, its code) · worker audit · no files · done
  - [ ] T5 sidebar: VIP-only inbox count (incl. forks), fork rows 2 pt left, pinned row 36 pt + selected + subline · worker sidebar · ui/src/sidebar.rs · written
  - [ ] T6 fixtures: drop extra design requests, Ledger unread, files/info/board data · worker fixture · clients/src/fixture/design_session.rs · written
  - [ ] T7 panels: diff comment chips/Open in editor/1-cell sign/rail count, Info rows+clones, Files order/status/icons/filter · worker panels · ui/src/panels.rs · written (filter height left: inside gpui_kit Input)
  - [ ] T8 terminal: golden font metrics, fake TUI colours + input box, offline notice in flow · worker terminal · ui/src/terminals.rs, terminal/pane/src/pane.rs, clients/src/fixture/terminal.rs · written
  - [ ] T9 session page: exit notice, offer banner, queue bar metrics, no "others can type" when split · worker session-page · ui/src/session_tabs.rs · written
  - [ ] T10 frames 07/08 native captures (design-workspace board + journal) · later batch
  - [x] T11 re-audit 01–17 (after gate 4): 0 of 15 match yet; 07/08 have no native capture. Found and fixed at once: Ledger was VIP only in frame 15 (native.rs overrode it), so the sidebar count was missing elsewhere; design.rs recorded with tolerance and kept stale pixels (now EXACT on record, like native).
  - [ ] T12 batch 3 (from T11), grouped by file:
    - fixture/terminal.rs + pane: the agent TUI input box full width and pinned to the terminal's last rows (26 pt, hint below); "⎿ " keeps its space; no cursor under option 3 / codex Working; frame 14 left pane stops after "Switch reads".
    - panels.rs: diff wrap off by default (clip), code indent −8 pt, footer "Send N comments" counts sent + drafts as the golden; rail pitch 40 pt; Info row pitch ~20 pt, agent "· model", a branch chip on every clone; Files: migrations before docs, folders folded by default, footer buttons look enabled; board PAY-3 attention card, orange linked dots, PAY-2 chip.
    - session_tabs.rs: offer banner 40 pt with filled "Open it"; "n others can type" only while the agent is working (golden 01 yes, 03/05/15 no); exit and offline notices in flow under the terminal's content (needs the pane to report its content height).
    - design_session.rs: per-frame statuses (05 Ledger dot orange in the sidebar, 16 Ledger NeedsYou + bold, 17 channel rollup orange), Info recovery and permission text, a blank line before "All four steps".
    - Out of scope, noted: Desk nav row (DESK), Inbox panel content: Allowed without asking, fork icon, checkbox, detail rows (ASK), frames 07/08 captures (T10).
    - Owner: each draft card's "Send" (D126) is not in golden 01/11; recommend adding it to the frames.
- **Decisions for the main agent:** see report; logged here as they are made.
  - L1: message kind decides delivery policy (Comments = explicit, immediate; Text from Submit = wait mode). Reverse: a per-message flag.
  - L3: a failure pauses automatic delivery for that session until Send now; removing or editing the last waiting message lifts the pause, so the next explicit Send goes at once.
  - L4: review-comments-queued captures now show a failed delivery (comments only wait when delivery fails or the session can't take input).
  - L5: the sidebar shows a session's inbox count only on VIP sessions, with forks' pending requests rolled up (frames 01–06, 15, 26 show it only on Ledger). Reverse: drop the `vip` condition in sidebar.rs draw_row.
  - L6: terminal font is size − 0.5 with pitch size + 6.5 (11.5/18 at the default 12) because the view rounds cell width up to whole pixels; exact 7.2 pt cells need terminal/view to stop rounding.
  - L7: Files panel status letter is inferred ("A" when the file's diff has no removals, else "M"); FileDiff has no status field.
  - L8: offline notice in the pane says "this program's machine" (pane has no machine name/time); naming it needs a caller-set notice on the pane.
  - L9: the exit notice stays at the bottom of the column (the terminal fills it), restyled and reworded like frame 05.
  - L10: Info lists every live fork and the rail Inbox counts the session family's pending requests (it matches the Inbox panel list), even though frame 05 shows one fork and frame 01 a rail badge of 1. The mock disagrees with its own sidebar (Review the schema is a fork) and its Inbox panel (2 requests). Owner approved fixing the frames (2026-11-15): design/session.rs rail shows 2 on Ledger frames, Info lists both forks.
  - L11: the design fixture's ledger question moved to pr-482 ("Review the new commits first?") so the Ledger family has exactly the two requests the goldens show, and Inbox stays at 7.
  - L2 (superseded by D128.3): the bar says "N messages waiting to send"; held text that waits for the turn end has no suffix; otherwise " · send failed" / " · sending" / " · send manually". Frames 03/04 reworded to match.
- **Waiting on:** —
- **Build gates:** —
- **Ready to commit:** no

# FRAMES1: The screens the next layers need are drawn and signed off

- **Status:** done (59340201)
- **Depends on:**
  - D0: frames are drawn from the tidied rules
- **Unblocks:** FRAMES2, FL, DESK, ASK (its exit), REVIEW
- **Layer:** L0 · **Lane:** docs

## Outcome
The owner can look at a frame for each screen FL, ASK, REVIEW and DESK build that has none today, and sign it off.

## Exit criterion
New goldens, numbered from 44, for: New channel and clone progress; "Couldn't resume" and the restart report line; the channel's Inbox tab and Agent access tab; comment states (addressed with before/after and Resolve, resolved by the agent, archived, the Archived list in Info); the comment box in a diff; Clean up…. Draw New channel and "Couldn't resume" first (FL needs them). Each is built from the same kit as frames 01–43, recorded at a gate by the main agent, and signed off by the owner. Draw only what SPEC says; where SPEC is silent, ask.

## Read these (pointers, nothing else)
- SPEC: CH-1–CH-5, PER-15, PER-29, CH-11, IN-1, IN-17–IN-19, SM-17–SM-21, RV-4, RV-25, RV-26, DK-11. Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: existing ones to match in style: 18–20 (channel tabs), 26 (Inbox), 31 (dialog), 05 (exited session), 11 (diff), 36 (Desk) (`desktop/tests/__snapshots__/design/NN-*.png`)
- Decisions: D76, D101, D113 (its "Frames still missing" list) (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `desktop/tests/design.rs`, `desktop/tests/design/kit.rs`, `desktop/tests/design/channel.rs`, `desktop/tests/design/overlays.rs`, `desktop/tests/design/session.rs`, `desktop/tests/__snapshots__/design`
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
Changing frames 01–43 (W1). The frames for machines, files, forks, PR reviews and views (FRAMES2). App code.

## Lane (written by the manager)
- **Goal / exit:** as above. Twelve draft frames, 44–55, drawn as code in
  `desktop/tests/design/`; the main agent records them at a gate; they stay
  drafts until the owner signs off. Frames 01–43 must draw byte-for-byte the
  same (the gate's design_test proves it: only 44–55 are new).
- **Frames (number · name · look · what it draws · SPEC):**
  - 44 `44-new-channel` · light · New channel dialog over Home: name, the folder
    name shown live (`~/Humata/payments-v2/`), repo picker from `gh` with
    fuzzy search and checkboxes, machine picker with an offline machine shown
    but not choosable, **Create channel and clone 2 repos** · CH-1–CH-4, CH-10.
  - 45 `45-new-channel-cloning` · dark · the same dialog after Create: per
    machine × repo clone progress until done, then the channel opens · CH-5.
  - 46 `46-session-couldnt-resume` · light · session screen after a reboot:
    scrollback from before, the reboot line, then "Couldn't resume" with the
    reason, **Try again**, **Start fresh with a summary**, and the restart
    report line · PER-15, PER-17, PER-29.
  - 47 `47-home-restart-report` · dark · Home after a restart: the toast of
    flow 26.8 step 5 and one line per failed session linking to its restart
    report · PER-29, §26.8. (Placement is a provisional pick; ask.)
  - 48 `48-channel-inbox` · light · channel Inbox tab: wide list + selected
    card, Waiting/Answered/All, Allow all 2 / Deny all · CH-11, IN-1, IN-3,
    IN-6, IN-7. Card reused from frame 26 (W1 redraws it everywhere).
  - 49 `49-channel-agent-access` · dark · channel Agent access tab: the IN-17
    rows in order with Allow/Ask me/Never, the explanation, the Never note,
    a scoped Always allow grant with Remove · IN-17–IN-19, SM-19.
  - 50 `50-diff-comment-box` · light · Diff full screen, comment box open
    under a clicked line: Draft, where it goes, Cancel / Comment · RV-4,
    RV-25, RV-26.
  - 51 `51-comment-addressed` · dark · an addressed comment with its passage
    before and after, **Resolve**, and **Resolve 2 addressed** · SM-16, SM-18.
  - 52 `52-comment-resolved-by-agent` · light · Show resolved on: a comment
    "Resolved by claude" with the agent's reply, **Reopen** · SM-12, SM-17,
    SM-19.
  - 53 `53-comment-archived-toast` · dark · after Archive: the comment gone
    from the diff, counts down, Undo toast · SM-20, NV-10.
  - 54 `54-info-archived` · light · session Info panel with **Archived**:
    archived comments with **Unarchive** · SM-21.
  - 55 `55-desk-clean-up` · light · Clean up… over the Desk: closed
    sessions least recently used first, folder sizes, ticks, one held by
    unpushed work named, Delete · DK-11, MV-9, NV-27, LD-7.
- **Design notes:** each group of frames gets its own file so workers never
  share one. The manager adds the shared hooks first, additive only:
  `session.rs` gets `Term::CouldntResume` (header words + terminal body) and
  `session_with(look, screen, on, title, body)` (the session screen with a
  given right-panel body; full screen with `Fill::PanelFull`), with the
  existing `panel()` going through the same shell; `info_panel` is split so
  `info_with(look, extra)` can append a section. `channel.rs` gets
  `Tab::{Inbox, Access}` and a `pub fn frame`. `pages.rs` makes `group`,
  `session_row` and `inbox_row` `pub`; `overlays.rs` makes `scrim` `pub`. `design.rs` gets the `mod` lines, frames 44–55 and any
  icons. The BUILD glob (`tests/design/*.rs`) already covers new files.
- **Claims:** see WORKLOG (FRAMES1 row).
- **Tasks:**
  - [x] T0 shared hooks (session/channel/pages/design.rs) · manager · done
  - [x] T1 frames 44, 45, 55 · worker FA (Sonnet, medium) · `design/new_channel.rs`, `design/cleanup.rs` · done, checked
  - [x] T2 frames 47, 48, 49 (46 is T0) · worker FB (Sonnet, medium) · `design/resume.rs`, `design/channel_tabs.rs` · done, checked
  - [x] T3 frames 50–54 · worker FC (Sonnet, medium) · `design/comments.rs` · done, checked (manager typed one closure arg)
- **Questions for the owner (SPEC silent; provisional picks drawn):**
  1. Clone progress (45): can it be cancelled, and what shows when one clone
     fails? Drawn: progress only, no Cancel, no failure state.
  2. Restart report line (47): where does "one line per failed session" sit?
     Drawn: a Home group "Couldn't resume after the restart" above Needs you,
     plus the same line under the notice in the session (46).
  3. Channel Inbox (48): rows flat (no grouping, the channel is the scope);
     Allow all 2 / Deny all above the list.
  4. Agent access (49): rows in IN-17's order, ungrouped; the tab scrolls, the
     frame shows the first rows.
  5. Resolved comment (52): Resolve/Reopen sit on the comment's header line.
- **Waiting on:** main agent's gate: build design_test, record 44–55, check 01–43 unchanged. Uncompiled until then.
- **Extra guesses to confirm:** 45 marks in-progress clones with an arrow; 51 draws Before/After side by side; 53/51/52 footers show a dimmed "Send comments" at zero drafts (RV-5 says disabled).
- **Build gates:** 2026-11-15 gate 1: design_test builds, 01–43 unchanged, 44–55 recorded. Fixes: 55 date/size collision (fixed widths, no wrap, dialog 700 wide); 51 dropped the duplicate "addressed" text (badge kept); 51 Before/After columns clip instead of overflowing. Waiting on re-record.
- **Owner sign-off 2026-11-15:** 44–55 approved with the drawn picks; 45 gains Cancel and a failed clone with Retry (drawn in 45 itself, no new frame).
- **Ready to commit:** yes, after the re-record of 45: frames 44–55 drawn as code in desktop/tests/design/ (new files new_channel, resume, channel_tabs, comments, cleanup; additive hooks in session, channel, pages, overlays; design.rs FRAMES 44–55). Reviewer: run design_test, 01–43 unchanged, view 44–55 PNGs.

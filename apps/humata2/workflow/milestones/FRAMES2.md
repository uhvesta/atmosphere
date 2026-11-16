# FRAMES2: The screens for machines, files, forks, PR reviews and views are drawn and signed off

- **Status:** done (this commit; owner approved 2026-11-15)
- **Depends on:**
  - FRAMES1: same files and the same lane; the earlier frames set the patterns
- **Unblocks:** MACH, FORK, FILES, VIEWS, PR
- **Layer:** L1 · **Lane:** docs

## Outcome
The owner can look at a frame for every remaining screen SPEC defines, and sign it off.

## Exit criterion
New goldens for: Add machine and the interactive login; the machine page; Storage; the file manager and transfers tray; Fork (settle D102); Move; Archive with unpushed work; an archived session; the PR review dialog, review session and Submit review; a prepared view's tab and its Showing bar; Delete channel (only if it stays in 1.0). Recorded at a gate, signed off by the owner.

## Read these (pointers, nothing else)
- SPEC: HO-1–HO-4, MP-12–MP-17, LD-13–LD-15, FM-1, FM-7, FK-2–FK-4, MV-4, MV-8, MV-9, LD-2, PR-1–PR-5, VW-6, VW-8, CH-24. Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: existing ones to match in style: 28 (Hosts), 31 (dialog), 32 (toast), 11 (diff) (`desktop/tests/__snapshots__/design/NN-*.png`)
- Decisions: D76, D102, D113 (its "Frames still missing" list), D11 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `desktop/tests/design.rs`, `desktop/tests/design/kit.rs`, `desktop/tests/design/pages.rs`, `desktop/tests/design/overlays.rs`, `desktop/tests/__snapshots__/design`
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
App code. Frames FRAMES1 or W1 own.

## Lane (written by the manager)
- **Goal / exit:** as above. Sixteen draft frames, 56–71, drawn as code in
  `desktop/tests/design/`; the main agent records them at a gate; they stay
  drafts until the owner signs off. Frames 01–55 must draw the same pixels
  (design_test proves it: only 56–71 are new).
- **Frames (number · name · look · what it draws · SPEC):**
  - 56 `56-add-machine` · light · Add machine over Hosts: host picked from
    `~/.ssh/config`, the five HO-1 steps, one failed with what failed, how to
    fix it and **Try again** · HO-1, HO-2.
  - 57 `57-add-machine-login` · dark · interactive login: a real terminal in
    the dialog at a verification-code prompt, the cost note, Cancel /
    **Done, I'm logged in** · MP-12–MP-16.
  - 58 `58-add-machine-unattended` · light · after the login: offer to make it
    unattended (Humata key in authorized_keys, or keep the cached token) ·
    MP-17.
  - 59 `59-machine-page` · light · Hosts › devbox-1: OS, address, daemon
    version, Open a shell, sessions + New session on devbox-1, health card,
    Humata folders by channel, agent reach rules · HO-4, FM-11.
  - 60 `60-machine-storage` · dark · Hosts › devbox-1 › Storage table with
    sizes measured on open and one action per row · LD-13–LD-15.
  - 61 `61-file-manager` · light · Files: two panes (This Mac, gpu-box),
    actions, trash count, the transfers tray with a paused, a running and a
    done transfer · FM-1, FM-7 (FM-2, FM-3, FM-8, FM-12).
  - 62 `62-fork-dialog` · light · Fork over the session: three kinds with
    one-liners, fork from / agent / machine, the text preview, the
    conversation note · FK-2–FK-6, FC-15; settles D102's look.
  - 63 `63-move-dialog` · dark · Move to another machine: online machines
    of the channel, what moves, the plain-shell note · MV-1, MV-2, MV-6.
  - 64 `64-session-moving` · dark · the session while moving: "Moving to
    This Mac…" with its steps, terminal read-only · MV-4.
  - 65 `65-archive-unpushed` · light · Archive confirmation with unpushed
    work and forks: names the work, "Its 2 forks keep running", the three
    MV-9 buttons · MV-7–MV-9, LD-1.
  - 66 `66-archived-session` · dark · an archived session: read-only
    scrollback, the diff summary per repo, **Restore** and **Delete** · LD-2,
    LD-3.
  - 67 `67-pr-review-dialog` · light · Review a pull request over ⌘K: PR
    facts, Also review, machine, agent, context, the push switch · PR-1–PR-5.
  - 68 `68-pr-review-session` · dark · the review session: review chat,
    Diff "PR #482 against main" with drafts (Edit/Drop), new commits line,
    **Submit review** · PR-5, PR-10, PR-11, PR-15.
  - 69 `69-pr-submit-review` · light · Submit review: drafts per PR, summary,
    verdict per PR, Submit · PR-12.
  - 70 `70-view-tab-showing` · light · a prepared review open as a tab: the
    step list, the step's file with the agent's note, Prev/Next, the
    Showing bar with Revert / Keep · VW-6, VW-8.
  - 71 `71-delete-channel` · dark · Delete channel… over the channel: live
    and archived sessions, board items, pending requests, work that blocks,
    the "Delete that work" tick · CH-24 (only if D11 stays in 1.0).
- **Design notes:** one new file per group so workers never share one. The
  manager added the shared hooks first, additive only, in `session.rs`:
  `pub fn shell(look, side, middle, panel)` (sidebar, a middle the frame
  draws, optional right panel through the stock panel shell, the strip) and
  `pub fn header_with(look, crumb, dot, title, words, right)` (the stock
  header's look with given words); `notice` and `transcript` made `pub`.
  The manager adds the `mod` lines, FRAMES 56–71 and any icons to
  `design.rs`. The BUILD glob already covers new files.
- **Claims:** see WORKLOG (FRAMES2 row).
- **Tasks:**
  - [x] T0 shared hooks (session.rs) · manager · done
  - [x] T1 frames 56, 57, 58 · worker MA (Sonnet, medium) · `design/machines.rs` · done, checked
  - [x] T2 frames 59, 60 · worker MB · `design/machine_page.rs` · done, checked
  - [x] T3 frame 61 · worker MC · `design/files.rs` · done, checked
  - [x] T4 frames 62, 63, 64 · worker MD · `design/fork_move.rs` · done, checked
  - [x] T5 frames 65, 66, 71 · worker ME · `design/archive.rs` · done, checked
  - [x] T6 frames 67, 68, 69 · worker MF · `design/pr_review.rs` · done, checked
  - [x] T7 frame 70 · worker MG · `design/views.rs` · done, checked
  - [x] T8 register 56–71 · manager · `design.rs` · done (no new icons needed)
- **Questions for the owner (SPEC silent; provisional picks drawn):**
  1. Add machine (56): a failed step shows the cause, the exact fix command and Try again in a red box under the steps; later steps show "Not run".
  2. Interactive login (57): a line says why it opened interactively; the cost note sits in an orange box above Cancel / Done, I'm logged in.
  3. Unattended (58): three choices (Humata key in authorized_keys [default], keep the provider's cached token, keep logging in by hand); Not now / Make it unattended.
  4. Machine page (59/60): Overview and Storage are sub-tabs of the machine page; Open a shell secondary, New session on devbox-1 primary; folders by channel each with "Open in Files"; agent reach rules listed read-only with a link to Agent access.
  5. Storage (60): "measured just now" + Measure again; row actions as text links; a running session's build cache is marked "asks first".
  6. File manager (61): the ⇄ swap glyph and the offline-machine picker list are not drawn; trash count follows the pane in focus; the tray shows done transfers without a Clear button.
  7. Fork (62, D102): frame 31 framing, three radio cards (selected = accent border), rows Fork from / Agent / Machine / Branch, a mono text preview, Fork as the primary. Proposed D102 → Settled by this frame once signed off.
  8. Move (63/64): machines as radio cards, current and offline ones listed but dimmed; during the move a step card sits in the terminal, input read-only, no right panel.
  9. Archive (65): button order Cancel, Archive and delete the work (red), Commit, push and open a PR, then archive (primary).
  10. Archived session (66): Delete and Restore… in the header, the scrollback dimmed, the right panel is a "Diff summary" plus the session record.
  11. PR review (67–69): the dialog draws over Home with the pasted URL above it; the new-commits line is a notice at the foot of the review chat; Submit review shows drafts per PR, a segmented verdict per PR and the agent's summary.
  12. Prepared view (70): no right panel; the Showing bar uses the offer bar's look; Prev/Next at the bottom with "1 / 4".
  13. Delete channel (71, D11 still Open): drawn as a draft; only sessions with unpushed work get the "Delete that work" tick; Delete channel stays disabled until ticked. Drop the frame if D11 goes to (b) or (c).
- **Proposed D-entry:** D102 → Settled: "Fork dialog as frame 62" (on sign-off).
- **Waiting on:** main agent's gate: build design_test, record 56–71, check 01–55 unchanged. Uncompiled until then.
- **Build gates:** 2026-11-15 gate 1: design_test builds, 01–55 unchanged, 56–71 recorded. Fixes: 62 kind-card note wraps, radio circle no longer squashed (local `pick`); 63 drawn over the API rate limiter session, This Mac row gets a note and no empty line; 71 Ledger also gets a "Delete that work" tick (ticked); 59 rules split into read (Allow, D127.4) and change (Never) rows. Waiting on re-record.
- **Ready to commit:** no

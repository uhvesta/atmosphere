# UXSIMPLE: Iterating on work stays simple, not niche

- **Status:** open (owner, 2026-11-19: "make sure this product doesn't get too niche though and remains somewhat simple")
- **Depends on:** nothing. It rules on REVIEWER and may reshape STACKVIEW, REVIEWUI and PR review before they go further.
- **Unblocks:** REVIEWER (no code until this has ruled and the owner has approved the mockups).

## Outcome
The loop from "the agent did work" to "I've reviewed it and it's shipped" takes few concepts to learn: at most ~7. Each control says what it does.

## Steps
1. Fable 5.2 reviews the loop, read-only. It covers the session terminal, the Diff panel (per-repo bases, full screen), comments and their lifecycle, STACKVIEW, push/PR, the PR review session and review chat, forks, the render/review skills, Mark reviewed, and REVIEWER. Its report goes to the session scratchpad `ux-review/FABLE_UX_REVIEW.md`; copy it here as `UXSIMPLE_REVIEW.md` when done. The report covers:
   - the loop as numbered steps, with a concept count;
   - overlaps (several "review" meanings, several base pickers, several ways to get a second opinion), ranked by how much each hurts a new user;
   - a REVIEWER verdict: keep, simplify or drop;
   - a target loop of at most ~7 concepts, with cheap wins (wording, hiding) separate from structural changes;
   - its first 5 changes, as outcome sentences.
2. Fable renders these owner flows in the proposed loop: 8–14 frames, each with a line in `FRAMES.md`.
   1. I open a terminal and just start working, then decide to make a stack of PRs across many repos.
   2. I made them in one agent session and don't know where the code is, so I ask the agent to prepare a review view of the stack.
   3. I review it: I comment, sign off on some layers or files, and mark some as good.
   4. The agent applies my feedback, and I review it by hand in the plain diff view, with no prepared ordering. I see what changed since I last looked and which comments were addressed.
3. The owner picks which changes go on the roadmap. Each accepted change becomes an entry in RESUME.md §1 and a decision in DECISIONS.md.

## State (2026-11-19)
- Step 1 and 2 done: `UXSIMPLE_REVIEW.md` (≈27 concepts today; target 7: session, diff, comment, stack, Push & PR, PR review, fork) and 11 frames in `mockups/uxsimple/` (+ `FRAMES.md`).
- REVIEWER verdict: simplify into "New agent tab" in the session's own tab strip, opened from the Diff header.
- **Sign-off is gradual (owner, 2026-11-19):** frames go to the owner in small batches (2–4), the next batch only after the owner has reviewed the last. Feedback from each batch shapes the next. Nothing is built until the owner signs off.

- 2026-11-19 (DIFFUI2): step 8 (a), (b) and (c) are built and Fable-approved (D850–D852):
  - (a) the footer "Send to <agent>";
  - (b) the three comment states, with done comments folded, and Reopen on Addressed;
  - (c) Looks good per file and per layer, with the "Since I last looked" switch.
  New native frames: review-done-by-agent(-full, -dark), review-done-folded, looks-good-layer, looks-good-changed and looks-good-all. (e), the Fork dialog, is deferred.
- 2026-11-19 (DIFFUI2): (d) is built and Fable-approved (D853): one agent note card, and per-layer "N drafts · M notes" counts. D852's looks are now kept by the host (GetLooks/SaveLooks, `refs/humata/looks/<session>`). Next are (f), fold order-humata-review into "show with notes", and REVIEWER's leftovers ("not installed", ⌘T, the opening prompt).

## Already marked by the owner (2026-11-19)
- The bare ⌃⌄ per-repo base chevron isn't understandable (revisits D739).
- "Show the terminal" in the full-screen Diff only exits full screen.

### Owner marks (minion DIFFUI, release-plan step 5, target 0.3.11)
| Mark | State |
|---|---|
| (a) Per-repo base chevron says what it does | Built before this lane: every repo row reads "against X" and opens its own menu (`review_header.rs` `repo_base`). Built in this lane: the PR review's folded repo row opens its first file; the open row has no chevron (D869). Left for the next batch: the header's duplicate base with 2+ repos (D868). |
| (b) Full screen: "Exit full screen" + Close | Built before this lane (`ui/core/src/chrome.rs` `header_with`; `diff_full_screen::full_screen_draws_one_header`). |
| (c1) Each layer row names its repos | Built by STACKVIEW (`panel_stack.rs` `row_note`, 0d2d50d0). |
| (c2) Whole stack marks layer boundaries | Built (D867): caption rows per layer inside each repo group, "also N", "Other changes"; click picks the layer. Gap: Whole stack file stepping (NextFile/ListDown) still follows raw diff order, not the drawn order. |

Lane log: 2026-11-19 minion DIFFUI: Fable ruled D867–D869. Workers: T1 Sonnet (D867: features/review stack.rs+lib.rs `whole_groups`, ui panel_stack.rs `layer_caption`, plan.rs, file_list.rs `file_row_also`, tests), T2 Haiku (D869: review_header.rs `review_repo_head`, one call in mod.rs, pr_session test). Gate-ready; goldens to record: native stack-whole, 68-pr-review-session, 68b, 68c.

## Out of scope
Building anything before the owner picks.
- 2026-11-19 (DIFFUI3): (f) is done (D854): `order-humata-review` is folded into `render-humata-review` as "show with notes" (`with-notes.md`). Two review skills: show, review. The installer drops the old skill from agents' folders.

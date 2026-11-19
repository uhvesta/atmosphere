# Humata 2: UX review of the "iterate on our work" loop

Fable 5.2, 2026-11-19. Read-only review. Paths are under `apps/humata2/`. "Frame NN" is `desktop/tests/__snapshots__/design/NN-*.png`; "current NN" is the scratchpad `gallery/current/NN-*.jpg`; "mockup N" is `workflow/milestones/mockups/reviewer/N-*.png`.

**Verdict in one paragraph.** Each piece is defensible alone, but the loop now asks a new user to learn about 27 concepts, and REVIEWER as specced adds two more. The product is drifting niche in three places: the word "review" carries about nine meanings, there are six ways to get a second agent before REVIEWER adds a seventh, and an agent can write on a line in four different card types. REVIEWER should ship, but as a new tab in the session's existing tab strip, not as a new docked terminal area with its own session kind and draft type. Frame 14 already draws the feature.

Not checked: which of these are built in the app today. I read specs, decisions, goldens and current renders, and ran nothing. I did not read D103's body beyond its heading.

## 1. The loop as a user meets it today

1. **An agent works.** I learn: *session* in a *channel*; *terminal tabs* (`claude`, `zsh`, a file tab, `+`; current 84); optionally a *second agent in a split pane* (frame 14, D103).
2. **I look at the change.** *Diff panel* in the right rail; *full screen* with **Show the terminal** as the way back (current 85); a *header base* ("against main" or "Comparing per repo"); a *per-repo base chip* that is a bare chevron when bases agree (D510, D739; current 52); *base presets* (Default branch, Uncommitted only, Since I last reviewed, since the fork; D183, current 86); **Mark reviewed**, an action that lives inside the base menu (D183).
3. **I comment.** *Draft* on a line, then **Send N comments** as one batch; a comment has a *kind* (comment, question, GitHub draft; DECISIONS.md:456).
4. **The agent revises.** Comment *states*: draft, queued, sent, addressed, resolved (by me, or by the agent with a reply), reopened, archived, detached, outdated (D101, D190, D319; frames 51–54); a **Show resolved** checkbox (current 52).
5. **The agent shows me the change.** Three *skills*: render, order, review (D791); a *prepared view* that arrives on an *offer bar* (Open it / Later / Not now) and opens as a *view tab* with steps and **Revert / Keep** (frame 70, VIEWS.md); or, in a review chat, view changes that apply at once with *Undo / Back* (D59); *agent notes* with "Make it a draft".
6. **I step a stack.** *Layers* and **Whole stack**, across repos; the base picker disappears while a stack shows; *drift* ("Moved since it was sent"); *key / skim* file tiers (D800, D802, STACKVIEW.md).
7. **I push.** **Push & PR** per repo; **Add repo…**.
8. **I review a PR.** *PR review session* in `#pr-reviews`, started from a dialog (frame 67); the *review chat* that helps me look but doesn't judge unless asked (D59, D791); *Draft for GitHub · by Claude Code* cards (current 68); **Update the clone** and outdated drafts.
9. **I submit.** *Submit review* sheet with a verdict and summary per PR (current 69).
10. **Around the loop.** *Fork*, in three kinds plus from-turn, agent, machine and branch (frame 62); *Journal* and *Info › Archived* as places things go.

**Count: about 27 distinct concepts**, before REVIEWER. REVIEWER as specced adds a *review terminal dock* and a *Reviewer draft* ("Send as mine"), for 29.

## 2. Where it is too niche or complex, worst first

1. **"Review" means about nine things.** Mark reviewed / Since I last reviewed (a snapshot); PR review session; review chat; Submit review (GitHub); `review-humata-change` (judges); `render-humata-review` (explicitly not a review); a prepared view titled "PR review: ledger schema" (frame 70); the `review_*` tools; and now "review terminal". D800's own title reads "A review can be a stack … only looking, never reviewing". A new user cannot predict what any "review" button does.
2. **Six ways to get a second opinion, with a seventh proposed.** A second agent in a split (frame 14); `+` tab, then run `claude` (current 84); fork Context only; fork Code only (frame 62); ask the working agent to run the review skill; start a PR review session on my own PR; and REVIEWER. All answer "another agent looks at this".
3. **Four card types for what an agent writes on a line.** Agent note (Make it a draft / Edit / Drop / Undo); "Draft for GitHub · by Claude Code" (Edit / Drop; current 68); the resolve reply (current 52); the view step note (frame 70). Mockup 2 adds a fifth, "Reviewer · claude · draft" with Edit / Drop / **Send as mine**, sitting above a footer that says "Send 1 comment". Send to whom, as what, is not evident.
4. **Controls that don't say what they do.**
   - The bare ⌃⌄ beside each repo name (current 52, D739 pick A). It was chosen to avoid printing "against main" three times. That saved words and lost the meaning.
   - **Show the terminal** in full screen (current 85). It exits full screen. Mockup 1 already says **Exit full screen**, which is right, but the same mockup then puts a terminal inside full screen, so the old label would have been actively wrong.
   - **Mark reviewed** is an action hidden in a menu of comparison targets (D183).
   - Current 86's menu mixes three ideas: a branch, a working-tree filter ("Uncommitted only") and a personal bookmark ("Since I last reviewed").
5. **Two walk-through mechanisms.** Prepared view (offer bar, view tab, steps, Revert / Keep; frame 70) and stack layers (list in the Diff panel, next / previous; D800). Both are "the agent ordered the change for me to step through". They have different homes, different steppers and different undo models (offer and Revert outside a review chat, apply with Undo inside one; STACKVIEW.md exit criterion). The user must know which kind of session they are in to predict what happens.
6. **Comment lifecycle is nine states.** A new user needs three: not sent, sent, done. Addressed vs resolved vs archived (D101) is a real distinction that almost nobody will hold in their head.
7. **Three skills where the owner asked for two.** The owner's ask in D791 was show versus review. `order-humata-review` is a third that sits between them.
8. **The Fork dialog** (frame 62) is three radio cards, four fields and a preview. It is fine for a power user and heavy for "try another approach".
9. **Modes to track.** Side panel vs full screen; stack showing vs not (base picker hidden); PR session vs working session (footer says Submit review… vs Send N comments); view showing vs not (Revert / Keep bar); which agent is "the" agent when there are two (D103).

## 3. Verdict on REVIEWER: simplify

The need is real: a fresh agent that did not write the change, visible beside the diff. The spec over-builds it.

**What the spec adds** (`workflow/milestones/REVIEWER.md`, mockups 1–3): a third place terminals live (after session tabs and split panes); a second tab strip (`claude · review`, `zsh · review`) next to the session's own `claude`, `zsh`; a Hide / Show button with a count and busy dot; a new session kind "listed like a fork"; a read-only copy (`~/review/ledger-schema`); a new draft type with "Send as mine"; and three open points for a ruling.

**What already exists:** frame 14 shows `codex` in the same session, prompted "Review the migration Claude is writing; don't edit files" (D103, SE-28). Current 84 shows a `+` on the session's tab strip, and the Diff panel already sits beside those tabs. That is REVIEWER, minus a button.

**Simplified REVIEWER:**
- **Keep the button**, in the Diff header, named for what it does: **New agent tab ⌄** (Terminal, claude, copilot, codex; no model list, as the owner ruled). It opens a new tab in the session's own tab strip, the same thing `+` does.
- **In full screen**, the button leaves full screen to the split view with the new tab selected. Full screen stays "diff only", and its exit reads **Exit full screen**.
- **Fresh context is free**: a new process has no history. Give it the same diff context any session agent gets through `review_context`; add nothing else.
- **Findings** use the existing agent note, with the unified card from section 4. No "Reviewer draft", no "Send as mine".
- **Several at once** is several tabs. Hide is switching tabs. End is closing the tab's program, as today.
- **Cut:** the docked terminal area, the second tab strip, the Hide / Show count button, the separate session listing, the read-only copy, mockups 2 and 3. Open points 1, 2 and 4 disappear.

**What this gives up:** the fresh agent shares the working tree and is not read-only, so it relies on the prompt ("don't edit files", as frame 14 does). In split view the diff is narrower than in mockup 2; the panel divider is the answer. If the owner later wants hard read-only, that is a property of a tab, not a new surface.

## 4. A simpler target loop: seven concepts

1. **Session**: terminals in tabs; any tab can be an agent; a fresh agent is a new tab.
2. **Diff**: one panel, narrow or full screen, with one plainly worded "against X" per repo.
3. **Comment**: one card. It goes to the agent, or to GitHub in a PR. Draft, sent, done.
4. **Stack**: the one way to step through a change, whoever ordered it.
5. **Push & PR**.
6. **PR review**: a session bound to PRs, ending in Submit review.
7. **Fork**: only for "try this another way with its own copy".

### Cheap wins (wording, hiding)
| Change | Touches |
|---|---|
| Repo rows always read "against X" when there is more than one repo; the header drops its duplicate. No bare chevron. | D739 (reverse pick A), D510; frames 01, 02, 11, 84–86 |
| Full screen's exit reads **Exit full screen**. | DECISIONS.md:859; DIFFVIEW-design.md lines 56, 117; frames 11, 50–53, 85 |
| "Review" is kept for PR review and Submit review only. Mark reviewed becomes **Looks good** on a file or a layer, and the comparison is "Since I last looked"; `render-humata-review` is described as "show". | D183, D791, RV-1, RV-6; frame 70; skills/ |
| Looks good leaves the base menu and sits in the Diff footer. It is sent to the agent with my comments, and its tick clears when those lines change. The base menu keeps branches only; "All changes / Since I last looked" is a two-way switch in the header. | D183, D510; frame 86 |
| Comments show three states. Addressed becomes "Sent · the agent changed these lines" with Resolve; Archive becomes Hide in the card's menu; Show resolved becomes a filter, not a checkbox row. | D101, D13; frames 51–54 |
| Fork opens on Code + context with the other kinds and fields under More options. | D102, D185, D188; frame 62 |
| Fold `order-humata-review` into the show skill as "with notes". Two skills: show, review. | D791 |

### Structural changes
| Change | Touches |
|---|---|
| REVIEWER becomes a new agent tab (section 3). | REVIEWER.md; D103, SE-28; frame 14 |
| One agent-authored line card everywhere: "Claude · note" with **Send to the agent**, **Draft for GitHub** (PR only) and **Drop**. | D59, D92, D284; RV-1 in REVIEWUI.md; frame 68 |
| One stepper. A prepared view's steps are a stack's layers. Cut the view tab, the offer bar and Revert / Keep before they are built; agent view changes apply with Undo in every session. | VIEWS.md (D2, D3, E2), VW-2, VW-9, D59, D800; frame 70 |
| One footer primary per panel, named for its destination: "Send 2 to claude" or "Submit review…". | REVIEWUI RV-1, DV-18 |

The VIEWS cut is the cheapest structural one: flow 06 lists the view tab, steps and Showing bar as not built (`flows/06-review-a-pr.md`, steps 5–6).

## 5. The five changes to make first

1. Every control in the Diff header says what it does: each repo reads "against main", and full screen ends with "Exit full screen".
2. A fresh agent to check a change is a new tab in the session I'm in, opened from the Diff panel, with nothing new to learn.
3. Anything an agent writes on a line is one kind of card with the same actions everywhere.
4. A comment is a draft, sent or done, and "review" only ever means a pull request review.
5. Stepping through a change has one stepper, the stack; the separate prepared-view tab and offer bar are cut before they are built.

## 6. The proposal drawn

Eleven frames of the four common flows, drawn in the simpler loop, are in `frames/` beside this file, with one line each in `frames/FRAMES.md`. They are HTML mockups (sources in `frames/src/`), not renders of the app.

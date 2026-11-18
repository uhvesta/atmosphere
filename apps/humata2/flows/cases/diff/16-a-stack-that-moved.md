# When a repo moves after a stack was sent, that layer says so and offers one fix; a stack with no notes shows no review controls

Layer: Diff and review. Platforms: macOS desktop.

## Starting state

- Machine `box-1` has `acme/api` and `acme/web` cloned. The session "Refunds" is open, and its Changes panel is beside the terminal.
- The agent sent a stack of three layers over the two repos: "Codec" (api #411), "Journal" (api #412) and "Client" (web #88). The stack was sent with no notes, so it has no comments and no draft comments.
- The stack was sent while `acme/web` was at commit `w1`. Since then, someone has pushed a new commit to `acme/web` on the same branch, so the top of the Client layer's range is now `w2`.
- The Inbox is open in a second pane. It has no stack offer yet.

## Reproduce headless

- Drift check, with real git fixtures: `bazel test //apps/humata2/daemon/review_diff:stack_test --test_filter=drift`. Cases: `a_stack_that_still_holds_has_no_drift`, `an_amended_lower_layer_drifts_the_layer_above_it`, `a_rev_that_is_gone_drifts_as_missing`, `head_leaving_the_top_drifts_the_worktree_layer`.
- The screen: none yet. The drift row, the Ask-to-send-again action and the Inbox offer are STACKVIEW T4 and T8. There is no test for them yet.
- No window goes on the owner's screen.

## Steps

| # | I do | I see | State and events | Status |
|---|---|---|---|---|
| 1 | I open the Refunds stack in the Changes panel. | The layer list shows. The "Client" layer has a line under it: "web moved since it was sent" (DS-65). | The daemon re-checks the stack when the review opens. The engine's `drift_blocking` reports `Moved` for `web` in layer "Client" (`daemon/review_diff/src/stack.rs:582`). Nothing calls it yet. | Engine: Works (real daemon), `stack.rs::an_amended_lower_layer_drifts_the_layer_above_it`. The row on screen: Not built (STACKVIEW T4, T8). |
| 2 | I look at the drift row. | One line, "web moved since it was sent", and one button, **Ask the agent to send it again** (DS-65, mockup `stacks` §3). | No such row or button is in `ui/src/` or `features/review/src/` yet. | Not built (T4, T8). |
| 3 | I click **Ask the agent to send it again**. | The request goes to the agent in the review chat. The agent sends the stack again. The row clears. | Expected: one action, with the stack re-read (owner addition: "Drift re-reads the stack"). Nothing in the app sends this request. | Not built (T4, T5, T8). |
| 4 | The agent sends the stack again. | The "Client" layer shows the new `w2` diff. The drift row is gone. | `drift_blocking` returns nothing once the stack holds (`stack.rs::a_stack_that_still_holds_has_no_drift`). | Not built (screen). Engine: Works (real daemon). |
| 5 | I look at the other layers. | "Codec" and "Journal" show no drift line. Only the layer that moved says so. | The drift is reported per layer and per repo (`Drift { layer, repo, kind }`). `stack.rs::an_amended_lower_layer_drifts_the_layer_above_it` asserts that the other repo is not drifted. | Not built (screen). Engine: Works (real daemon). |
| 6 | Another layer's commit is amended, so its rev is gone. | That layer says the commit is missing. The fix is still one action. | `DriftKind::Missing` (`stack.rs::a_rev_that_is_gone_drifts_as_missing`). | Not built (screen). Engine: Works (real daemon). |
| 7 | I open a stack with no notes, in a PR review or in the diff. | The layers, files and diffs show. There are no note gutters, no draft affordances and no verdict buttons (DS-66). | Expected: a stack with no notes draws no review chrome. | Not built (T8 test). There is no stack view to check this against yet. |
| 8 | A stack has no drift and no notes. | The stack shows as a list to look at, and no line about review. | As step 7. | Not built (T8). |
| 9 | The stack is sent to me while I am not looking at the review. | The Inbox gets an offer from Copilot: "Sent a change to look at: 3 layers over 2 repos (api, web)", with **Open the stack** and **Later** (DS-68, mockup `stacks` §3). | Expected: the offer is an Inbox item, and opening it starts on Whole stack. | Not built (T4, T8). |
| 10 | I click **Open the stack** in the offer. | The Changes panel opens on Whole stack. | As step 9. | Not built (T8). |
| 11 | I click **Later**. | The offer waits. The stack stays as it was. | As step 9. | Not built (T8). The Inbox's "Later" control exists for Views (`ui/src/view_tab.rs:178`), not for stacks. |
| 12 | I press ⌥⌘] or ⌥⌘[ on a stack. | The next or previous layer shows. | Layer keys are proposed in spec §8 item 1. | Not built (T8). |

## Branches

### The stack moved and I did not ask for a re-send

When: a repo moves after the stack was sent, and I do not click anything.
What I do: nothing.
What I see: expected, the layer shows the drift line on the next open. Today, nothing shows.
Expected outcome: the line appears and does not block the diff. Other layers still work.
**Status:** Not built. The engine finds the drift (`stack.rs:582`). The app does not call it.

### The drift is in the top layer

When: HEAD of the clone leaves the top head an uncommitted layer sits on.
What I do: I switch branches in `acme/api` while the uncommitted layer is shown.
What I see: expected, the uncommitted layer says HEAD moved.
**Status:** Engine: Works (real daemon). `stack.rs::head_leaving_the_top_drifts_the_worktree_layer`. Screen: Not built.

### The agent's re-send fails

When: I click **Ask the agent to send it again**, and the agent cannot send the stack (for example, the session is busy with a turn).
What I do: I click the button.
What I see: expected, the error says why, and the row stays.
**Status:** Not built. The `stack` send error path in the review chat has a refusal (`ChatError::NoSuchLayer`, `daemon/review_diff/src/stack.rs:70-74` style messages), but no button uses it.

### A stack with notes

When: the stack has notes or drafts.
What I do: I open the stack.
What I see: the note gutters and draft affordances show. This is the case the no-review-chrome rule does not cover.
**Status:** Not built as a stack view. Notes for a PR review show in the existing review panel (`pr_session.rs`), not in a stack.

## Expected outcomes

- [ ] A layer whose repo moved since the stack was sent says so, under that layer (DS-65).
- [ ] The fix is one action, and it asks the agent to send the stack again (DS-65).
- [ ] Other layers and repos stay usable while one layer shows drift.
- [ ] A stack with no notes shows no note gutters, no draft affordances and no verdicts (DS-66).
- [ ] A stack sent while I am away is an Inbox offer that says how many layers and repos, and opens on Whole stack (DS-68).
- [ ] ⌥⌘] and ⌥⌘[ move between layers, and ⌥⌘↓ and ⌥⌘↑ still move between files (spec §8 item 1).

## Status table

Counts (16 rows, by each row's first tag): Works (real shell) 0; Works (real daemon) 0; Fakes only 0; Code, no test 0; Not built 16; Likely bug 0.

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Drift found when the review opens | Not built (screen). Engine: Works (real daemon) | `daemon/review_diff/tests/stack.rs::an_amended_lower_layer_drifts_the_layer_above_it`; `stack.rs:582` (no caller) |
| 2 Drift row with one action | Not built | DS-65, STACKVIEW T4, T8 |
| 3 Ask the agent to send it again | Not built | DS-65, STACKVIEW T4, T5, T8 |
| 4 Stack holds again, row clears | Not built (screen). Engine: Works (real daemon) | `stack.rs::a_stack_that_still_holds_has_no_drift` |
| 5 Only the moved layer says so | Not built (screen). Engine: Works (real daemon) | `stack.rs::an_amended_lower_layer_drifts_the_layer_above_it` |
| 6 Missing rev | Not built (screen). Engine: Works (real daemon) | `stack.rs::a_rev_that_is_gone_drifts_as_missing` |
| 7 No review chrome with no notes | Not built | DS-66, STACKVIEW T8 |
| 8 Stack with no drift and no notes | Not built | DS-66, STACKVIEW T8 |
| 9 Inbox offer | Not built | DS-68, STACKVIEW T4, T8 |
| 10 Open the stack from the offer | Not built | DS-68, STACKVIEW T8 |
| 11 Later | Not built | DS-68; `ui/src/view_tab.rs:178` is the Views control, not the stack's |
| 12 Layer keys | Not built | STACKVIEW T8; spec §8 item 1 |
| Branch: drift not asked for | Not built | `stack.rs:582` has no caller |
| Branch: top layer HEAD moved | Not built (screen). Engine: Works (real daemon) | `stack.rs::head_leaving_the_top_drifts_the_worktree_layer` |
| Branch: re-send fails | Not built | none; `daemon/review_diff/src/stack.rs:70-74` holds the messages |
| Branch: stack with notes | Not built as a stack | `pr_session.rs` covers PR review notes only |

## Gaps and likely bugs

- Not built: the drift row, the **Ask the agent to send it again** action and the Inbox offer (DS-65, DS-68; STACKVIEW T4, T8).
- Not built: no caller of `drift_blocking`. `daemon/review_diff/src/stack.rs:582` is only exported (`daemon/review_diff/src/lib.rs:79`), so no review ever checks drift. The engine tests pass, but the app never sees a drift. The check must be wired into the daemon's stack service (T4).
- Code, no test, to check when T4 lands: after a `Missing` report, `drift_blocking` resets `below` to `None` (`stack.rs:611-616`). The next layer in the same repo then skips its chain check, so a broken chain there may not be reported. Add a test that a layer above a missing one still reports a broken chain.
- Not built: the rule that a stack with no notes shows no review chrome (DS-66). It needs the stack view (T8) first.
- Not built, conflict: the layer keys (⌥⌘] and ⌥⌘[) are settled in spec §8 item 1, but the T8 test still expects ⌥⌘↓ to pick layer 1. That test must change.

# A stack of changes over two repos shows as layers I can step through, each with only its own files

Layer: Diff and review. Platforms: macOS desktop.

## Starting state

- Machine `box-1` has `acme/api` and `acme/web` cloned. The session "Refunds" has committed work in both.
- The agent has prepared a change as three layers, bottom to top:
  - Layer 1 "Codec": `acme/api` only, PR #411.
  - Layer 2 "Journal": `acme/api` only, PR #412. It does not touch `acme/web`.
  - Layer 3 "Client": `acme/web` only, PR #88.
- The agent sent the stack to me in the review chat. The Changes panel is open beside the terminal. Nothing is in full screen.
- The clone of `acme/api` has a different branch checked out from the one the stack was sent from. This is to show that layers do not depend on the checkout (DS-64).

## Reproduce headless

- The engine, with real git fixtures: `bazel test //apps/humata2/daemon/review_diff:stack_test`. This covers layer diffs, the whole stack, layer titles and the refusal cases. Key cases: `each_layer_shows_its_own_diff_in_only_the_repos_it_touches`, `whole_stack_is_the_union_of_the_layers_and_leaves_out_stray_work`, `a_range_layer_is_the_same_wherever_the_clone_is_checked_out`.
- The agent's side, through the review chat: `bazel test //apps/humata2/daemon/review_diff:stack_chat_test`. It covers picking a layer by title, Undo and the layer focus switch.
- The screen: none. There is no stack view yet. `bazel test //apps/humata2/desktop:native_test --test_filter=stack` will find nothing today, because `desktop/tests/native/stack.rs` does not exist yet (STACKVIEW T8).
- Not headless: how the layer list looks, and whether the header wording reads well. Frames 119 to 121 are not drawn yet.
- No window goes on the owner's screen.

## Steps

| # | I do | I see | State and events | Status |
|---|---|---|---|---|
| 1 | I ask the agent to show the change as a stack. | The review opens with the stack in place. The header reads "Whole stack · 3 layers" (DS-62). | The plan carries the stack, and it opens at Whole stack (`stack_chat.rs::a_plan_with_a_stack_opens_at_whole_stack`). The daemon does not yet send the stack to the Changes panel (STACKVIEW T4, T6). | Not built (screen). Engine: Works (real daemon) (T7, T8). |
| 2 | I look at the layer list above the files. | The list shows "Whole stack" first, then "3 Client", "2 Journal", "1 Codec" top-down. Each row shows its PRs: "web #88", "api #412", "api #411". | Layers are ordered top-down with 1 at the bottom (DS-60). | Not built (STACKVIEW T8). `ui/src/panel_stack.rs` does not exist. |
| 3 | I stay on Whole stack. | The file list shows the files of all three layers, in both repos. Stray work is not in the list. | `whole` is the union of the layers (`stack.rs::whole_stack_is_the_union_of_the_layers_and_leaves_out_stray_work`). | Not built (screen). Engine: Works (real daemon) (T8). |
| 4 | I click layer 2, "Journal". | The header reads "Layer 2 of 3 · Journal". The file list shows only `acme/api` files. A quiet line under the repos says "web isn't in this layer" (DS-63). | `layer_spec` for "Journal" only has the `acme/api` range (`stack.rs::each_layer_shows_its_own_diff_in_only_the_repos_it_touches`). | Not built (header and quiet line). Engine: Works (real daemon) (T6 repo filter, T8). |
| 5 | I look at the diff for Journal. | Only the lines that Journal added in `acme/api` show. The lines from Codec are not there. | Layer diff is its own base..head (DS-64). | Not built (screen). Engine: Works (real daemon) (T8). |
| 6 | I click layer 3, "Client". | The header reads "Layer 3 of 3 · Client". The list shows only `acme/web` files. "api isn't in this layer" shows. | Same as step 4, for `acme/web` (`stack.rs::each_layer_shows_its_own_diff_in_only_the_repos_it_touches`). | Not built (screen). Engine: Works (real daemon) (T8). |
| 7 | I click layer 1, "Codec", while the clone of `acme/api` is on another branch. | Codec's diff is the same as it was when the stack was sent. My checkout does not change it. | A range layer is the same wherever the clone is checked out (`stack.rs::a_range_layer_is_the_same_wherever_the_clone_is_checked_out`). | Not built (screen). Engine: Works (real daemon) (DS-64 "Partly built": the daemon service and app are T4, T6, T7). |
| 8 | I press ⌥⌘] to go to the next layer. | The next layer shows. | Proposed key (spec §8 item 1). The palette gets "Next layer" and "Previous layer" (STACKVIEW T8). | Not built (T8). |
| 9 | I press ⌥⌘[ to go back. | The previous layer shows. | Same as step 8. | Not built (T8). |
| 10 | I press ⌥⌘↓ with a file open. | The next file opens, not the next layer. | Files keep ⌥⌘↓ (spec §8 item 1). | Not built (DV-15). |
| 11 | I open Whole stack again and look at a file that some layers share. | It is listed once, with the layers it is in. | Not built as a rule; the whole stack is a union, so a file appears once. | Code, no test. |
| 12 | The agent sends a layer by title, "Journal", with the focus switch. | The review moves to Journal. Undo moves back to where I was. | `the_agent_moves_the_user_to_a_layer_by_title_and_undo_moves_back` and `moving_layers_needs_the_focus_switch` (`stack_chat.rs`). | Engine: Works (real daemon). The screen follows the same events: Not built (T7, T8). |

## Branches

### A layer names a repo the session does not have

When: the agent sends a stack with a layer that touches `acme/billing`, which is not in the session.
What I do: nothing. The agent sends it.
What I see: the stack is refused, and the agent is told which layer and repo.
Expected outcome: the refusal names the layer and the repo, so the agent can fix it.
**Status:** Works (real daemon). `stack.rs::a_repo_the_session_does_not_have_is_refused_by_layer`.

### An empty layer or a repo listed twice in one layer

When: the agent sends a layer with no repos, or the same repo twice.
What I do: nothing. The agent sends it.
What I see: the stack is refused by name.
**Status:** Works (real daemon). `stack.rs::an_empty_layer_or_a_repo_twice_in_one_layer_is_refused_by_name`.

### Layers that do not chain in one repo

When: two layers in `acme/api` do not chain, so one starts from a commit that is not the other's head.
What I do: nothing. The agent sends it.
What I see: the stack is refused, and it says the layers form a fork.
**Status:** Works (real daemon). `stack.rs::layers_that_do_not_chain_in_a_repo_are_a_fork_and_refused`.

### A stack with no key files and no order

When: the agent sends a stack with no importance on any file.
What I do: I look at the list.
What I see: the files show in the order the diff gives them. No dot on a file, and no skim fold at the end of a repo.
Expected outcome: key files carry a dot, and skim files fold into one row at the end of each repo (DS-67).
**Status:** Not built (T3b, T3c, T8). T3c is planned, not started.

### Skim files fold at the end of a repo

When: a layer has skim files, such as `Cargo.lock`.
What I do: I open the layer.
What I see: one row at the end of the repo, "3 files to skim · Cargo.lock, …". I click it, and the rows open in place.
**Status:** Not built (DS-67, T3c, T8; frame 121 not drawn).

### A stack shown in a narrow panel

When: the panel is narrow, so the layer list does not fit.
What I do: I open the panel at its narrow width.
What I see: about 6 rows show, and the list scrolls. The picked row stays in view (DS-60).
**Status:** Not built (T8). No frame yet (frame 119 not drawn).

## Expected outcomes

- [ ] A stack shows as a list of layers, with Whole stack first and layer 1 at the bottom (DS-60).
- [ ] The picked layer is highlighted and shows its one-line purpose (DS-61).
- [ ] The header reads "Whole stack · N layers" or "Layer N of M · Title", and the base picker is hidden (DS-62).
- [ ] A layer lists only the repos it touches, with a quiet line for each repo it does not touch (DS-63).
- [ ] A layer's diff is its own base..head, wherever the clone is checked out (DS-64).
- [ ] Next and previous layer move with ⌥⌘] and ⌥⌘[ (spec §8 item 1).
- [ ] Key files carry a dot, and skim files fold at the end of each repo (DS-67).

## Status table

Counts (18 rows, by each row's first tag): Works (real shell) 0; Works (real daemon) 3; Fakes only 0; Code, no test 1; Not built 14; Likely bug 0.

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Stack opens at Whole stack | Not built (screen). Engine: Works (real daemon) | `daemon/review_diff/tests/stack_chat.rs::a_plan_with_a_stack_opens_at_whole_stack`; STACKVIEW T7, T8 |
| 2 Layer list | Not built | DS-60, STACKVIEW T8; `ui/src/panel_stack.rs` does not exist |
| 3 Whole stack is the union | Not built (screen). Engine: Works (real daemon) | `daemon/review_diff/tests/stack.rs::whole_stack_is_the_union_of_the_layers_and_leaves_out_stray_work` |
| 4 Layer 2 shows only api; quiet line | Not built (quiet line). Engine: Works (real daemon) | `stack.rs::each_layer_shows_its_own_diff_in_only_the_repos_it_touches`; DS-63 (T6, T8) |
| 5 Layer 2 diff | Not built (screen). Engine: Works (real daemon) | `stack.rs::each_layer_shows_its_own_diff_in_only_the_repos_it_touches` |
| 6 Layer 3 shows only web | Not built (screen). Engine: Works (real daemon) | `stack.rs::each_layer_shows_its_own_diff_in_only_the_repos_it_touches` |
| 7 Layer diff is independent of the checkout | Not built (screen). Engine: Works (real daemon) | `stack.rs::a_range_layer_is_the_same_wherever_the_clone_is_checked_out`; DS-64 |
| 8 ⌥⌘] next layer | Not built | STACKVIEW T8; spec §8 item 1 |
| 9 ⌥⌘[ previous layer | Not built | STACKVIEW T8; spec §8 item 1 |
| 10 ⌥⌘↓ next file | Not built | DV-15; spec §8 item 1 |
| 11 A shared file lists once | Code, no test | none found |
| 12 Agent moves me by title | Not built (screen). Engine: Works (real daemon) | `stack_chat.rs::the_agent_moves_the_user_to_a_layer_by_title_and_undo_moves_back`, `moving_layers_needs_the_focus_switch` |
| Branch: repo the session lacks | Works (real daemon) | `stack.rs::a_repo_the_session_does_not_have_is_refused_by_layer` |
| Branch: empty layer, repo twice | Works (real daemon) | `stack.rs::an_empty_layer_or_a_repo_twice_in_one_layer_is_refused_by_name` |
| Branch: layers do not chain | Works (real daemon) | `stack.rs::layers_that_do_not_chain_in_a_repo_are_a_fork_and_refused` |
| Branch: key files and skim | Not built | DS-67, T3c, T8 |
| Branch: skim fold | Not built | DS-67, T3c, T8 |
| Branch: narrow list scroll | Not built | DS-60, T8 |

## Gaps and likely bugs

- Not built: the whole screen. There is no stack view (`ui/src/panel_stack.rs` does not exist). STACKVIEW T8 owns it, and frames 119 to 121 are not drawn. T8 needs Fable's approval before it starts.
- Not built: the daemon service and clients. The engine is in `daemon/review_diff`, but the review service, the MCP tools and the app do not call it yet (STACKVIEW T4, T5, T6, T7). The tests that prove the layers are engine tests with real git fixtures, not app tests.
- Not built: importance and skim (DS-67, STACKVIEW T3b, T3c). T3b is planned, not started.
- Not built: the layer keys (⌥⌘] and ⌥⌘[) and the palette entries (STACKVIEW T8). The conflict with ⌥⌘↓ and ⌥⌘↑ is settled in spec §8 item 1, but the T8 test still says "alt-cmd-down picks layer 1", and it must change.
- Not proven, to be clear: `daemon/humatad/tests/remote_stack.rs` tests copying the `stack` CLI and host settings to boxes. It is not a review-layer test, so it does not prove this case.
- Not proven: `features/review/tests/plan_rows.rs` tests the order of the file list in a review plan, not stacks. `features/review:stack_test` (STACKVIEW G3) does not exist yet.
- Steps 4 and 6 need the quiet "isn't in this layer" line, which is a DS-63 rule not built yet. Without it, a repo a layer does not touch simply does not appear.

# STACKGH — our stack CLI works the way `gh stack` does, and can bring in a remote stack

**Owner, 2026-11-20:**
- "can we actually make the way our CLI handles stacked PRs basically 1 to 1 with GH stack"
- "also able to import a remote set of stacked PRs locally to understand them in it's own format and interleave them"

## Outcome
1. **Parity.** `stack` (tools/stack) matches `gh stack` command for command:
   - the same verbs, flags, defaults and output shapes, and the same branch and PR-base model;
   - someone who knows `gh stack` can use ours without reading docs;
   - a stack made by either tool is understood by the other.

   Where ours does more (reviewed-commit history, refresh, pins, the Humata review view), those stay as extra verbs or flags. They never change a shared verb's meaning.
2. **Import.** Given a remote stack, `stack import` fetches the branches and records the stack in our local format: topics, relatives, order and the PR numbers, so `status`, `diff`, `restack` and the review view work on it. The remote stack can be named by:
   - a PR URL or number in the stack;
   - a `gh stack` stack;
   - a chain of PRs whose bases point at each other.

   Re-importing updates it in place.
3. **Interleave.** Local commits can be inserted between, on top of or below imported layers. A restack then keeps both: the imported PRs' bases are updated, and nothing is force-pushed to someone else's branch without an explicit flag.
4. **Humata.** The PR review view and `render-humata-review` show an imported stack layer by layer. The `humata-pull-requests` skill documents import.

5. **Native mode writes no comments** (owner, 2026-11-20: "when stack works in github stacked PR mode … we save on needing to write comments because it just uses what github gives us natively … each pr is a stack of commits and a stacked PR is a stack of those stacks").
   - The model is two levels: a PR is a stack of commits (one topic), and a stacked PR is a stack of those PRs.
   - When the chain is linked as a native GitHub stack, `upload` writes no stack-tree or history comment. GitHub's own stack view and push timeline carry it, and existing comments of ours are left alone, never deleted.
   - Comments stay only as the fallback, where native stacks aren't available or `--no-native-stack` is set.
   - Check that every place that reads our history comment (refresh, the reviewed-commit history, the Humata review view) gets the same facts from GitHub's native stack and push events when there is no comment.
   - Test: native on → zero comment calls on the fake gh; native off → today's comments.

## Pointers
- **Code:** tools/stack/src (`cli.rs`, `commands.rs`, `gh.rs`, `plan.rs`, `restack.rs`, `status.rs`, `local.rs`, `history.rs`).
- **Earlier decisions:** memory "Stack CLI + PR review adds" (2026-11-16): an Opus medium coach plus Haiku workers.
- **Step 1, research:** before any code, write a verb-by-verb parity table in this file: `gh stack` (verb, flags, behaviour) → ours (same, different, missing). Fable rules on each difference.
- **Naming:** `gh stack` is named here because it is the owner's target. Code and comments don't cite other outside projects.

## Exit
- Every row of the parity table is closed (same, or an extra that doesn't collide).
- Failing-first tests:
  - each shared verb against a fake gh and a temp repo;
  - importing a 3-PR remote stack, then `status`, `diff` and `restack`;
  - interleaving 2 local commits between imported layers, then restack, with bases correct and no foreign force-push;
  - re-import after the remote changed.
- A live check against a real throwaway repo (prior approvals cover our own test repos).

## Ships
0.10.4, after SMFIX (0.10.3) and before WEBTODO (0.11.0).

## Out of scope
Merging PRs for other people, and hosting a stack UI on the web.

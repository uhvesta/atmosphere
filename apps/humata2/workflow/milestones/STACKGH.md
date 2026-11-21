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
6. **Session bootstrap & native stack review** (owner, 2026-11-20):
   - Interoperability enables reviewing remote stacks natively in Humata's review view, bootstrapping a new session directly from a remote stack, or telling an agent to copy/import a remote stack locally.
7. **Stack update / repush summary table with clickable diffs** (owner, 2026-11-20):
   - When a stack is pushed/repushed or updated programmatically (in comment/table mode or when updating the stack summary), post or update a clean summary table showing what changed across the stack versions with clickable compare/diff links for each repushed layer/PR ("we repushed the stack — here is the clickable diff from the previous version").

## Parity table (`gh stack` → `stack`)

| `gh stack` verb | `gh stack` flags & behaviour | `stack` (ours) | Status | Resolution |
| --- | --- | --- | --- | --- |
| `init [branches...]` | `--base <branch>`: initialize stack on trunk, optionally adopt branches | `stack init [--base/--trunk <branch>] [--remote <name>] [branches...]` | Same | Records trunk & initializes/adopts stack metadata in `.git/stack/state.json`. |
| `add <branch>` | Add a new branch/layer on top of the current layer | `stack add <name> [--relative <topic>]` | Same | Creates and checks out a new layer on top of the current layer in `.git/stack/state.json`. |
| `view` | `--json`: display stack and PR status | `stack view [--json] [--offline] [--trunk/--base] [--remote]` | Same | Shows stack tree, PR numbers, states, and upload status; machine-readable with `--json`. |
| `up` | Checkout the next upper layer in the stack | `stack up` | Same | Checks out the adjacent upper layer branch. |
| `down` | Checkout the next lower layer in the stack | `stack down` | Same | Checks out the adjacent lower layer branch. |
| `top` | Checkout the highest layer in the stack | `stack top` | Same | Checks out the top layer branch of the stack. |
| `bottom` | Checkout the lowest layer in the stack | `stack bottom` | Same | Checks out the lowest layer branch above trunk. |
| `trunk` | Checkout the trunk branch | `stack trunk [--base/--trunk] [--remote]` | Same | Checks out the stack's trunk branch. |
| `submit` | `--auto`, `--open`, `--draft`, `--dry-run`, `--json`: push branches & open/update PRs | `stack submit [--auto] [--open] [--draft] [--dry-run] [--json] [--force-foreign] ...` | Same | Pushes branches and creates/updates stacked PRs; `--auto` drafts new PRs non-interactively; `--open` marks PRs ready for review. |
| `push` | `--remote <name>`: push stack branches with `--force-with-lease` | `stack push [--remote <name>] [--base/--trunk] [--dry-run] [--force-foreign]` | Same | Atomically pushes stack branches with `--force-with-lease`; never force-pushes foreign branches without `--force-foreign`. |
| `sync` | Fetch trunk, rebase stale layers, push active branches, sync PR bases | `stack sync [--base/--trunk] [--remote] [--dry-run] [--json] [--force-foreign]` | Same | Fetches trunk, rebases local/interleaved stack, pushes own branches safely, and updates PR bases. |
| `rebase` | `--continue`, `--skip`, `--abort`, `--onto <ref>`: rebase stack locally | `stack rebase [--continue \| --skip \| --abort] [--onto <ref>] [--base/--trunk] [--remote]` | Same | Rebases the stack locally onto trunk or `--onto`, resuming/aborting cleanly on conflicts. |
| `link` | `--base <trunk> <url>...`: link PRs as a native GitHub stack | `stack link [--base/--trunk <trunk>] <pr>...` | Same | Delegates to `gh stack link` when installed; falls back to chaining PR base branches. |
| `import` | *(not in `gh stack`)* | `stack import <target> [--branch <name>] [--base/--trunk] [--remote] [--json]` | Extra | Imports a remote stack (PR URL/number, `gh stack` stack, or base-chained PRs) into local format; re-import updates in place. |
| `check` | *(not in `gh stack`)* | `stack check [--base/--trunk] [--remote] [--json]` | Extra | Local validation of trailers, template sections, and conflict-free rebuild. |
| `upload` | *(not in `gh stack`)* | `stack upload [...]` | Extra | Pushes branches, opens/updates PRs, pins revisions, links native stacks (zero comments in native mode; repush diff summary table in comment fallback). |
| `status` | *(not in `gh stack`)* | `stack status [--json] [--offline] [--base/--trunk] [--remote]` | Extra | Tree view of topics, PR states, orphans, restack progress, and imported layers. |
| `restack` | *(not in `gh stack`)* | `stack restack <edit\|continue\|skip\|abort>` | Extra | Interactive edit of an earlier topic with `--step` and `--exec`. |
| `diff` | *(not in `gh stack`)* | `stack diff <topic> [from] [to]` | Extra | Inter-revision `git range-diff` from pins (or imported layer diff). |
| `guide`, `config`, `commands` | *(not in `gh stack`)* | `stack guide`, `stack config show`, `stack commands` | Extra | Built-in manual, settings inspection, and machine-readable CLI schema. |

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

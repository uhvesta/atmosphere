# To-do (board) cases

**To-do** replaced the old kanban/table/dashboard boards (BOARDSIMPLE, D830–D874): one ranked list for every channel and session, with channel and session filters, three states (`Open`, `In progress`, `Closed`), optional due dates, effort, impact, tags, steps, `Hide until…`, `Pin`, `Do next`, pull-request review items, and **Needs attention** asks answered **Yes** / **No** on the item. The daemon still serves both `TodoService` and `BoardService` over the same items, and agents can also use the `board_*` / `todo_*` MCP tools and `humata board` CLI. The flow they come from is [flows/10-the-board.md](../../10-the-board.md).

Each case follows [`briefs/_GUIDE.md`](../briefs/_GUIDE.md) and adds `## Reproduce headless` as [`briefs/verify.md`](../briefs/verify.md) asks. Status tags were set from the code and tests, not the spec.

## Index

### The person's side

| # | Case | Covers |
|---|---|---|
| 1 | [An item is made, gets its key, and moves between states](01-items-and-statuses.md) | Quick add, keys, `Open` / `In progress` / `Closed`, check circle, `Show closed` / `Reopen` |
| 2 | [Depends-on edges hide waiting items, refuse loops, and unblock on close](02-depends-on-edges.md) | `Waits on`, folded Waiting group, overdue rule, loop refusal, target deletion |
| 3 | [The ranked To-do list, filters, and channel To-do tab](03-kanban-table-dashboard.md) | Computed rank + reasons, channel chips, `Filters` popover, search, `Do next`, saved views (replaces kanban/table/dashboard) |
| 4 | [The Graph view (not built; replaced by Waiting on another item)](04-graph-view.md) | Not built; BOARDSIMPLE keeps one ranked list |
| 5 | [The To-do drawer edits an item and starts a session on it](05-the-drawer.md) | Title, State, Due, Effort, Impact, Tags, Steps, Description, Pin, Hide until…, Start a session on this, stale copies |
| 6 | [An agent's ask is answered Yes or No on the item](06-approvals-on-the-item.md) | Needs attention group, ask card with evidence/change/affects, `Yes` / `No`, `Ask why`, `Open session`, suggested items |
| 7 | [Who made an item and To-do rules decide who may change it](07-ownership.md) | `CAPABILITY_TODO_SUGGEST`, `CAPABILITY_TODO_CLOSE`, own vs others' items, Global board, Always allow |
| 8 | [The scope picker and the session panel show the right boards](08-scopes-and-session-panel.md) | Session To-do panel, channel narrowing, Desk panel |

### The agent's side

| # | Case | Covers |
|---|---|---|
| 9 | [An agent finds out that To-do / the Board exists](09-agent-learns-the-board-exists.md) | MCP tool list (`board_*` + `todo_*`), greeting, skills, `humata --help`, `PATH`, `Do next` first message |
| 10 | [An agent reads To-do and boards through the CLI and through MCP](10-agent-reads.md) | `todo_next`, `board_items_list`, `board_item_get`, `board_item_history`, `humata board` |
| 11 | [An agent creates, edits, moves, closes and deletes through MCP](11-agent-writes-through-mcp.md) | `todo_next`, `todo_add`, `todo_close`, and the seven `board_item_*` tools |
| 12 | [An agent creates, edits, moves and deletes through the CLI](12-agent-writes-through-cli.md) | `humata board add`, `edit`, `move`, `rm` in a session tab |
| 13 | [An agent asks for approval and waits for the answer](13-agent-asks-for-approval.md) | Pending, Needs attention on the item, `inbox_wait`, exit 12, `--wait`, denial, the reason |
| 14 | [A real Copilot agent on Claude Haiku 5.5 plans a small task](14-real-copilot-plans-a-task.md) | A plain-words prompt, then checks through `humata` |

An agent's board writes while the host is away (journaled, then replayed) are in [`agents/20`](../agents/20-an-agent-on-a-box-while-the-host-is-away.md), not here.

## The throwaway rig

Every case's **Reproduce headless** uses one of two things: a named Bazel test, or this rig. The tester builds and runs it, never the owner's daemon. No window opens.

```
bazel build //apps/humata2/daemon/humatad:humatad2_admin //apps/humata2/daemon/holder:humata2_holder //apps/humata2/cli:humata
D=$(mktemp -d); mkdir -p "$D/home" "$D/root"
HOME="$D/home" humatad2_admin serve --admin --socket "$D/h.sock" --state "$D/home/.humata2/state" \
  --root "$D/root" --holder <holder binary> --humata <humata binary>
export HOME="$D/home" HUMATA_SOCKET="$D/h.sock"
```

- **The person** is any `humata` call from the tester's shell. With `--admin`, a call with no token counts as the present user (D341 in `DECISIONS.md`, line 341). The release `humatad2` has no such path.
- **An agent** is any command run inside a session's tab. Start one with `humata session start --agent zsh --title Ledger --channel payments`, then run a line in it with `humata session type Ledger '<command>'`, which types the line and submits it once (`verify/04`). The tab has `HUMATA_SESSION_TOKEN` and `HUMATA_SOCKET` (`daemon/humatad/src/identity.rs:502`), so `humata` there acts as that session's agent. Call `humata` by its full path: it is not on the tab's `PATH`.
- **MCP by hand** is `humatad2_admin mcp --socket "$HUMATA_SOCKET"` run inside the tab, with JSON-RPC lines on stdin. It must descend from the session's holder, so it can't be run from the tester's shell.
- Read the tab's output with `humata attach` piped to a file, or have the typed command write its stdout, stderr and exit code to files under `$D`.
- Evidence to keep: each command's stdout, stderr and exit code; for app cases the `.new.png` and `.view.txt` a failing snapshot test writes.

## What the cases found

Not built (or deferred):

- **Agents are never told To-do / the Board exists via MCP** (case 9). No agent is started with Humata's MCP server: `daemon/sessions/src/agents.rs` (Copilot, Codex, Claude). The Copilot plugin holds hooks only. `skills/` installs `humata-board` (D790), but MCP config is not written at launch.
- **The Graph view** (case 4). Superseded by BOARDSIMPLE's single ranked list; dependency edges show in `Waiting on another item` and the drawer's `Waits on` row.
- **Deleting an item or editing Sessions / Waits on pickers in the To-do drawer** (cases 2, 5). Title, description, due date, effort, impact, tags, steps, pin and hide-until are all editable in `ui/workspace/src/todo.rs`; `History` link is a no-op (`features/app/src/lib.rs:1189`); deleting and adding/removing sessions or `Waits on` edges from the drawer are not wired yet.
- **`humata todo` CLI subcommand and `humata board` flags** (cases 10, 12, 13). The CLI still uses `humata board`, which sends no reason, reads no single item, and lists one board only (`cli/src/commands/boards.rs`).
- **The drawer's ownership line** (case 7). The drawer header shows `created` date ("made by you, …"), not the per-session ownership note (BD-3).
- **The model named at start never reaches the agent** (case 14).

Likely bugs / remaining gaps:

- **Asks don't survive a daemon restart** (D155). The agent's waiting call ends; the ask leaves the item (case 6).
- **The CLI's default board differs from MCP's.** `humata board add` goes to the channel board (`cli/src/commands/boards.rs:31`); MCP's `board_item_create` defaults to the session board (case 12).
- **Starting a session from an item runs in the client** (`clients/live_daemon/src/boards.rs::start_item_work`), not atomically on the daemon (case 5).

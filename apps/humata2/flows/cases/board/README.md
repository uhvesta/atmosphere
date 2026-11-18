# Board cases

The Board is Humata's small issue tracker: items in columns, depends-on edges, sessions per item, and asks answered on the item. These cases prove it for a person and for agents. The flow they come from is [flows/10-the-board.md](../../10-the-board.md).

Each case follows [`briefs/_GUIDE.md`](../briefs/_GUIDE.md) and adds `## Reproduce headless` as [`briefs/verify.md`](../briefs/verify.md) asks. Status tags were set from the code and tests, not the spec.

## Index

### The person's side

| # | Case | Covers |
|---|---|---|
| 1 | [An item is made in a column, gets its key, and moves between statuses](01-items-and-statuses.md) | New item, **+**, keys, Status menu, drag, the five columns |
| 2 | [Depends-on edges are set, refuse a loop, and block nothing](02-depends-on-edges.md) | The picker, loops, ✓ on Done, deleting a target |
| 3 | [Kanban, Table and Dashboard show the same items, and every figure opens what it counts](03-kanban-table-dashboard.md) | The three views, Filter, drill-down |
| 4 | [The Graph view draws the depends-on edges](04-graph-view.md) | Not built |
| 5 | [The drawer shows an item whole and edits its sessions and edges](05-the-drawer.md) | Drawer fields, history, pickers, stale copies, Start a session |
| 6 | [An agent's ask is answered on the item](06-approvals-on-the-item.md) | Review, Move to / Keep in, edit and delete asks |
| 7 | [Who made an item decides who may change it](07-ownership.md) | Own items, others' items, the Global board, Always allow |
| 8 | [The scope picker and the session panel show the right boards](08-scopes-and-session-panel.md) | Global, channel and session boards, forks, the panel |

### The agent's side

| # | Case | Covers |
|---|---|---|
| 9 | [An agent finds out that the Board exists](09-agent-learns-the-board-exists.md) | MCP tool list, the greeting, skills, `humata --help`, `PATH` |
| 10 | [An agent reads boards through the CLI and through MCP](10-agent-reads.md) | List, get, history, columns, other boards |
| 11 | [An agent creates, edits, moves and deletes through MCP](11-agent-writes-through-mcp.md) | The seven board tools on its own boards |
| 12 | [An agent creates, edits, moves and deletes through the CLI](12-agent-writes-through-cli.md) | `humata board add`, `edit`, `move`, `rm` in a session tab |
| 13 | [An agent asks for approval and waits for the answer](13-agent-asks-for-approval.md) | Pending, `inbox_wait`, exit 12, `--wait`, denial, the reason |
| 14 | [A real Copilot agent on Claude Haiku 5.5 plans a small task on the Board](14-real-copilot-plans-a-task.md) | A plain-words prompt, then checks through `humata` |

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

Not built:

- **Agents are never told the Board exists** (case 9). No agent is started with Humata's MCP server: `daemon/sessions/src/agents.rs:572` (Copilot), `:552` (Codex), `:615` (Claude). The Copilot plugin holds hooks only: `daemon/agent_hooks/src/lib.rs:373`. `skills/` has no Board skill. `api/MCP.md:13` says the holder writes the agent's MCP config; nothing does.
- **The Graph view** (case 4). `ui/src/board.rs:256` has three modes.
- **Renaming, describing and deleting an item in the app** (case 5). `features/board/src/lib.rs:439`.
- **The drawer's ownership line** (case 7). BD-3; `clients/src/model.rs:1467` is never drawn.
- **Folding a Table group** (case 3). `ui/src/board.rs:537` draws the chevron with no action.
- **`humata board` gives no reason, reads no single item, sends no revision, and lists one board only** (cases 10, 12, 13). `cli/src/commands/boards.rs:84`, `:112`, `:55`; `cli/src/args.rs:549`.
- **The model named at start never reaches the agent** (case 14). `daemon/humatad/src/session_service.rs:488` records it; `daemon/sessions/src/agents.rs` has no placeholder for it.

Likely bugs:

- **Card ask text is fixed.** "Asks to move to Done" for every ask: `ui/src/board.rs:421`, `ui/src/panel_board.rs:170` (case 6).
- **A + on a column is two calls.** A failed move leaves the item in To do: `features/board/src/lib.rs:699` (case 1).
- **Blocked and Won't do exist only in the app.** The daemon has five columns; the app model has seven: `clients/src/model.rs:1390`, `clients/live/src/boards.rs:38` (case 1).
- **The CLI's default board differs from MCP's.** `humata board add` goes to the channel board (`cli/src/commands/boards.rs:31`); `board_item_create` needs `board` and MCP's unset default is the session board (`daemon/humatad/src/mcp/boards.rs:35`) (case 12).
- **The session panel's board shows no error line** when a write is refused (case 8). `ui/src/panel_board.rs` never reads `board.error`; `ui/src/board.rs` draws it on the page only.
- **Asks don't survive a daemon restart** (D155). The agent's waiting call ends; the card goes (case 6).

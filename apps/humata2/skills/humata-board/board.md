# The board

## The boards

There are three kinds of board, all with the same five columns:

| Board | What goes on it | How you name it |
| --- | --- | --- |
| Your session's board | Your own working list | no flag on a write; `--session <session>` on a read |
| The channel's board | Work the whole channel tracks | `--channel <channel>` on a write; no flag on a read |
| The Global board | Work across channels | `--global` |

Note the difference: **a read with no flag shows your channel's board; a
write with no flag goes to your own session's board.** Name the board when
it matters.

Columns, as the CLI spells them: `backlog`, `to-do`, `in-progress`,
`in-review`, `done`.

An item has an id (such as `PAY-7`), a title, a description, a column, the
sessions working on it, the items it depends on, and a revision number.

`--global` and `--session` belong to `humata board` itself, so they go
**before** the subcommand: `humata board --global add "..."`. `--channel`
can go anywhere.

## Reading

    humata board --json                          # your channel's board
    humata board --session <session> --json      # one session's board (yours: the session from `humata whoami`)
    humata board --global --json                 # the Global board
    humata board --channel <channel> --json      # another channel's board
    humata board --column in-progress --column in-review --json   # only these columns
    humata board history <item> --json           # who changed an item, and how

Each item in the result has `id`, `title`, `description`, `column`,
`sessions`, `depends_on` (each with its id, title and column) and
`revision`. There is no command that prints one item: list the board and
pick it out by id. (MCP has `board_item_get`.)

## Writing

Add an item:

    humata board add "<title>" [--column <column>] [--description "<text>"] \
        [--assign <session>]... [--depends-on <item>]... --json

- No board flag: your session's board. `--channel <channel>`: that
  channel's board. `humata board --global add ...`: the Global board.
- Without `--column` the item starts in To do.
- `--assign` names a session working on it; repeat it for more.
- `--depends-on` names an item on any board; repeat it for more.
- The result holds the new item and its `id`.

Edit an item:

    humata board edit <item> [--title "<title>"] [--description "<text>"] \
        [--assign <session>]... [--unassign <session>]... \
        [--depends-on <item>]... [--drop-depends-on <item>]... \
        [--revision <n>] --json

Change its status, or its place in a column:

    humata board move <item> <column> [--after <item>] [--revision <n>] --json

Delete it (it stays in the history):

    humata board rm <item> [--revision <n>] --json

`--revision <n>` is the revision you read. The change is refused (exit 6,
`revision_conflict`) if a field you are changing was changed since. Pass it
when you decide from what you read, such as "move it to Done because it was
In review".

## Workflows

### "Add this to the board"

1. Pick the board. Your own next steps go on your session's board. Something
   the owner wants the channel to track goes on the channel's board
   (`--channel <your channel>`, from `humata whoami`). If you can't tell,
   use the channel's board when the owner said "the board".
2. Read that board first and check the item is not already there.
3. Add it with a title that says the outcome, and a description with what
   done means. Assign your own session if you are the one doing it.
4. Tell the owner the item's id.

### "Plan this" or "break this into tasks"

1. Write the list of items and, for each, which others must be done first.
2. Add the items that depend on nothing first. Read each new `id` from the
   `--json` result.
3. Add the rest with `--depends-on <id>` for each item they wait for. Or add
   all items, then wire the edges with `humata board edit <item> --depends-on <id>`.
4. Read the board back and show the owner the plan: ids, titles, and what
   each depends on.

An edge that would close a loop is refused, and the error names the loop:
drop or turn one edge. Dependencies are information only. Humata refuses no
move or start because of one, so keep to the order yourself.

To have the owner look at a plan before it lands on the board, there is no
CLI command. The MCP tools `view_prepare` (content `board`) and
`view_mark_ready` offer it to them as a view.

### Keeping status current

- Starting an item: `humata board move <item> in-progress`, and
  `humata board edit <item> --assign <your session>` if you are not on it.
- A pull request is open or the work waits for review:
  `humata board move <item> in-review`.
- Done only when it is done and checked: `humata board move <item> done`.
  Never move an item to Done to tidy up.
- Blocked: leave the column, and say why in the description.
- When you finish, list what depended on it
  (`humata board --json`, items whose `depends_on` names it) and tell the
  owner what is now free to start.

### "What's on the board?" or "what's left?"

Read the board the owner means, then answer in words: counts per column,
what is in progress and by which session, and which To do items have every
dependency in Done.

## What needs the owner

| Action | Default rule |
| --- | --- |
| Anything on your own session's board | Allowed |
| Read your channel's board, the Global board, other channels' boards | Allowed |
| Add an item to your channel's board | Allowed |
| Edit, move or delete a channel item **you** made | Allowed |
| Edit, move or delete a channel item someone else made (the owner or another agent) | Asks the owner |
| Any change on the Global board or another channel's board | Asks the owner |

The owner can change these defaults per channel. A change that asks exits
12 with a request id: see `shared/basics.md`. The owner sees the request on
the item itself and answers it there. Do not repeat the change. Pass
`--key` when a change may ask.

The CLI's board commands take no reason. When a change will ask and the
owner should see why, use the MCP tool for it (`board_item_update`,
`board_item_move`, `board_item_delete`), which takes `reason`.

## Failures

| What you see | What to do |
| --- | --- |
| exit 6 `revision_conflict` | Someone changed the item. Read it again, then decide again. Don't just resend. |
| exit 3 `not_found` | Wrong id, or an item on a board you can't read. List the board. |
| an error that names a loop | The edge closes a loop. Drop or turn one edge. |
| exit 4 `denied` | The rule for that board is Never. Tell the owner; don't retry. |
| exit 12 `pending` | The owner must approve. Go on with other work. |
| `"outcome": "journaled"` | The Mac is away. The change is kept and applied later; a new item has no final id yet, so wire its edges after the Mac is back. |

## Not in the CLI

- Printing one item by id (MCP: `board_item_get`).
- A reason on a board change (MCP: the `reason` field).
- Offering a board plan to the owner as a view (MCP: `view_prepare`).
- Marking an item pinned or VIP: the owner's only.

# How `humata` answers

Read this once per conversation before you run `humata` commands. Every
Humata skill builds on it.

## Who you are

You run inside a Humata session. `humata` is first on your `PATH` and acts
as **this session's agent**: no flag picks who you are, and you can do only
what the owner's Agent access rules let this session do.

    humata whoami --json

prints your session, channel, machine and agent, and the session's folder
and clones (repo, path, branch). Use the names and ids it prints in other
commands. If `humata` is not found, or `whoami` says you have no credential,
you are not in a Humata session: say so and stop.

The `humata` MCP server offers the same actions under the same checks via
three progressive-disclosure tools: `humata_list` (discover),
`humata_describe` (inspect schemas/examples), and `humata_call` (run).
These files document the CLI. An MCP tool is named only where the CLI has
no command for something.

## Flags every command takes

| Flag | Use |
| --- | --- |
| `--json` | One JSON object on stdout. Use it whenever you read the result. |
| `--channel <name\|id>` | Another channel than your own. |
| `--machine <name\|id>` | A machine. |
| `--wait <seconds>` | How long to wait for the owner's answer. With `--json` the default is 0. |
| `--key <key>` | An idempotency key. Running the same command with the same key is the same request, not a second one. |
| `--dry-run` | Print the plan (what is lost, what is kept) and its token; change nothing. |
| `--plan <token>` | Do what the plan from `--dry-run` said. |
| `-y`, `--yes` | Skip the CLI's own "Continue?" after a plan. |
| `--strict` | Fail (exit 20) when the change was only journaled. |

Sessions, channels, machines and items are named by title or id. A name
that matches more than one fails with exit 2 and lists the matches: use the
id.

## What comes back

With `--json`:

    { "ok": true,  "operation": "...", "outcome": "applied",   "result": { } }
    { "ok": true,  "operation": "...", "outcome": "journaled", "journal": { } }
    { "ok": false, "operation": "...", "error": { "kind": "...", "message": "...", "fix": "..." } }

- `applied`: done.
- `journaled`: the owner's Mac is away. The change is written down on this
  machine and applied when the Mac returns. A new item has no final id until
  then. Say so; do not repeat the command.
- `stale_as_of` on a read: the Mac is away and this is the last copy.

Branch on the exit code or `error.kind`, never on the message text. Read
`error.fix`: it says what to do.

| Exit | Meaning | What you do |
| --- | --- | --- |
| 0 | Done (applied or journaled) | Go on. |
| 2 | Usage, or an ambiguous name | Fix the command. `humata <command> --help` lists the flags. |
| 3 | `not_found` | Check the name or id with a list command. |
| 4 | `denied` | A rule says Never. Do not retry and do not find another way. Tell the owner which rule the error names. |
| 5 | `user_only`, `presence_required` | Only the owner can do this. Tell them the exact command to run in their own terminal, or to use the app. |
| 6 | `revision_conflict`, `plan_stale`, `name_taken` | Someone changed it first. Read it again, then decide again. |
| 7 | A precondition (`unpushed_work`, `session_busy`, `user_away`, ...) | Do what `error.fix` says. |
| 8 | `machine_offline`, `host_unreachable` | The machine or the Mac is away. Do not loop. Say what is waiting. |
| 9 | git, gh or a provider step failed | Read the message; fix the cause it names. |
| 10 | `rate_limited`, `disk_full` | Wait until the time given; do not just retry. |
| 12 | `pending` | The owner has not answered yet. See below. |
| 70 | `internal` | A bug in Humata. Report the command and the message; do not retry in a loop. |

## When the owner has to approve (exit 12)

Each kind of action has a rule: **Allow**, **Ask me** or **Never**. An
action under Ask me becomes a request in the owner's Inbox. Your command
prints the request id (`rq_...`) and waits as long as `--wait` says, then
exits 12 with `pending`.

- Pending is not a failure. **Approving runs the action**, whether or not
  you are still waiting. Never run the command again to "make sure": that
  makes a second request. If you must re-run it, pass the same `--key`.
- For anything that may need approval, pass a key and a wait you can
  afford: `--json --wait 45 --key <short-stable-name>`.
- Go on with other work, or tell the owner the request is in their Inbox
  and end your turn. Humata types a line into your terminal when it is
  answered. `humata inbox show <request> --json` shows the answer later.
- Denied means no. The owner's note, if any, says why. Do not ask again
  for the same thing unless they tell you to.
- You can never approve, deny or reply to a request yourself, and you
  cannot change the rules. Those are the owner's.
- Settings edits (global, channel, or agent switches) are gated in the
  owner's Inbox (`Ask me` or `Never`), not `user_only`: propose them
  directly instead of asking the owner to run a terminal command.

`humata access why <your-session> <capability> [target]` says in advance
whether an action is allowed, asked or refused. The `humata-inbox` skill
covers asking the owner questions and asking ahead for access.

## Don't

- Don't edit, move or delete anything under `~/.humata2` or Humata's own
  folders by hand. Use the commands.
- Don't work around a refusal with `git`, `ssh`, `scp` or another tool.
  A refusal is the owner's rule.
- Don't run interactive commands (`humata attach`, `humata shell`,
  `humata login`): they need a person.
- Don't claim something is done until the command said `applied`.

## Long mechanical runs

If your agent can start a subagent on a small, fast model, hand it long
runs of `humata` calls (many board items, many files) with the exact
commands, and check what comes back. Keep judgement calls for yourself.

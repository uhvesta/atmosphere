# Sessions and channels

A **channel** groups repos, machines and sessions. A **session** is one
agent (or shell) in a terminal, with its own clones of the channel's repos.
A **fork** is a session made from another one. The **Desk** holds sessions
that belong to no channel.

## Looking around

    humata whoami --json                  # you: session, channel, machine, agent, folder, clones
    humata ls --json                      # your channel's sessions (--all: archived too)
    humata topology --json                # the channel's sessions and their forks
    humata activity --json                # what happened in the channel
    humata channels --json                # every channel
    humata channel grid <channel> --json  # which repos are cloned on which machines
    humata session tabs <session> --json  # a session's terminal tabs
    humata session running --json         # every Humata terminal on your machine, stopped ones too
    humata ops --json                     # long-running operations you started
    humata ops <id> --json                # one of them, step by step

## Starting a session

    humata session start [--agent <agent>] [--model <model>] [--repo <owner/name>]... \
        [--title "<title>"] [--message "<first message>"] --json --wait 45 --key <key>

- It starts in your channel, on your machine. `--machine <machine>` picks
  another machine (see the `humata-remote` skill).
- `--agent` is an agent kind the owner has enabled (`humata agents ls`).
  Without it the channel's default agent runs.
- `--repo` picks which of the channel's repos it clones; repeat it.
- `--message` is typed to the new agent as its first prompt. Write it as a
  full brief: the goal, what is already decided, where to report. The new
  session does not see your conversation.
- The result names the new session and an operation. Follow it with
  `humata ops <id>`.

## Forking

    humata session fork <session> --kind <kind> [--agent <agent>] [--before-turn <n>] \
        [--title "<title>"] [--branch <branch>] --machine <machine> --json --wait 45 --key <key>

`--kind` is required:

| Kind | The fork gets |
| --- | --- |
| `code-and-context` | its own copy of the clones and a copy of the conversation |
| `code-only` (short: `code`) | its own copy of the clones, a fresh conversation |
| `context-only` (short: `context`) | a copy of the conversation; it works in the parent's clones |

- `<session>` is the session to fork; use your own from `humata whoami` to
  fork yourself.
- `--machine` says where the fork runs. Name your own machine to keep it
  here.
- `--before-turn <n>` (2 or more) forks from just before that turn instead
  of from now.
- `--branch` names the branch the fork's clones are made on. Without it the
  branch is named from the title.
- A fork stays in its parent's channel. For another channel use
  `session continue`.
- `humata session own-copy <session>` later gives a context-only fork its
  own copy of the clones.

Use a fork to try something without disturbing your clones, or to run two
approaches side by side. Tell the owner the fork's title so they can find it.

## Continuing in another channel

    humata session continue <session> --channel <channel> [--before-turn <n>] --json --wait 45 --key <key>

Starts a new session in `<channel>` whose agent goes on from a copy of this
conversation. The original is untouched. `--channel` is required and can't
be the session's own channel.

## Talking to another session

| Command | What it does |
| --- | --- |
| `humata session note <session> "<text>"` | Types one line into that session's agent after its turn, naming you. No answer comes back. |
| `humata session ask <session> "<text>"` | The same, as a question. Your command waits and prints ANSWERED, DECLINED or EXPIRED with the answer. |
| `humata session answer <qu_id> "<answer>"` | Answer a question another session asked **you**. It appears in your terminal with its `qu_` id. `--decline` says no instead. |
| `humata session type <session> "<text>" [--no-submit] [--at-turn-end] [--tab <id>]` | Types text into that session's terminal and presses Enter, as if a person typed it. |

- Prefer `note` and `ask`. `type` is for driving a terminal (a shell, or a
  prompt on screen); `--no-submit` leaves the text unsent, `--at-turn-end`
  queues it until the agent's turn ends.
- When a question arrives in your terminal, answer it with
  `session answer`. The asker's command is waiting on it.
- A session that is not running gets nothing. Check `humata ls` first.
- Say who you are and what you need in the text. The other agent sees only
  that line.

## Reading what sessions said

    humata sessions search <words>... [--agent <kind>] [--since <RFC 3339>] [--until <RFC 3339>] --json
    humata session read <session> [--from <turn>] [--to <turn>] --json
    humata conversation <session> --json

Search matches every word and returns hits with the session, the turn and
a snippet. Then read those turns with `session read`. `--channel` and
`--machine` narrow a search. You see your own channel; other channels ask
the owner.

## Changing or ending a session

    humata session rename <session> "<title>"
    humata session repo add <session> <owner/name>
    humata session repo drop <session> <owner/name>
    humata session stop <session>                 # stops its agent; it can be resumed
    humata session resume <session> [--fresh]     # --fresh starts over with a summary instead of the conversation
    humata session archive <session> [--push-first | --delete-work]
    humata session end <session>                  # end for good: archived by the plan
    humata session restore <session> [--stopped]  # an archived session back
    humata session rebuild <session>              # rebuilt from its record
    humata session delete <session>
    humata desk keep [<session>]                  # keep a Desk session's folder past its 7 days (yours when left out)

- `archive`, `end` and `delete` take `--dry-run`, which prints the plan:
  what is lost and what is kept. Run it first and show the owner anything
  that would be lost (unpushed work). `--push-first` commits, pushes and
  opens pull requests before archiving. Only pass `--delete-work` when the
  owner said to delete that work.
- Ending your own session ends you. Finish and report before you ask for it.

## Channels and repos

    humata channel create <name> [--repo <owner/name>]... --json --wait 45 --key <key>
    humata channel repo add <channel> <owner/name>
    humata repos search <words>... [--owner <owner>]... [--all-of-github] [--your-repos] --json
    humata repos list --json

`repos search` looks in the owner's repo library, then GitHub. Use it to
turn "the payments repo" into `owner/name`; if more than one fits, ask.

## What needs the owner

| Action | Default rule |
| --- | --- |
| See your channel, its sessions, machines; read and search your channel's sessions | Allowed |
| List channels; read other channels | Allowed |
| Search or read sessions of other channels | Asks the owner |
| Start or fork a session, continue in another channel | Asks the owner |
| Note, ask or type into another session | Asks the owner |
| Rename your session, add or drop a repo, own copy | Asks the owner |
| Stop, archive or delete your own session | Asks the owner |
| Stop or archive another session; restore or rebuild one | Asks the owner |
| Create a channel; add a repo to a channel | Asks the owner |
| Delete another session | Never |
| Rename a channel, remove a repo or a machine from it, change its settings | Never |

Only the owner, never an agent: deleting a channel, `humata attach` with
typing, `humata shell`, `session send-now`, `session stop|resume|end --tab`,
`humata events`, `humata audit`, `humata settings set`, `humata agents add|edit`,
pins and VIP marks.

A request card shows the owner the agent, machine, kind and repos. One
clear request is better than several: decide what you need, then ask once.

## Failures

| What you see | What to do |
| --- | --- |
| exit 12 `pending` | The owner must approve. Don't start it again; see `shared/basics.md`. |
| a note is refused because the agent is still mid-turn | Send it again later, or use `session type --at-turn-end`. |
| exit 7 `unpushed_work` | Archiving or deleting would lose work. Show the owner the plan (`--dry-run`); offer `--push-first`. |
| an agent kind is turned off or not installed | The owner turned it off in Settings › Agents, or its CLI is missing on that machine. Tell them; pick another `--agent` only if they agree. |
| `session ask` prints EXPIRED | The other agent didn't answer in time. Read its recent turns, or send a note. |
| exit 8 | The session's machine or the Mac is away. Say what is waiting; don't loop. |
| an operation ends `stuck` or `undone` | `humata ops <id> --json` says which step and why. `humata ops retry <id>` only when the detail says a retry is safe. |

## Not in the CLI

- Starting a session for a board item in one step: start it, then
  `humata board edit <item> --assign <session>` (see `humata-board`).
- Seeing another session's screen: `humata attach --read-only` is
  interactive. Read its turns with `session read` instead.

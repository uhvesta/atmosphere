# Humata's own skills

These are the agent skills Humata ships. Humata installs them into each
agent's own skills folder and keeps them up to date by itself (D790); the
person installs nothing. They let an agent in a Humata session act on a
plain ask ("put that on the board", "run it on the GPU box") with the
`humata` CLI.

## The list, by value

| # | Skill | Purpose | The person says |
| --- | --- | --- | --- |
| 1 | `humata-board` | Read and change the boards: items, statuses, assignees, depends-on edges, a plan laid out as items. | "add this to the board", "make a card", "mark it done", "move it to in review", "what's left", "break this into tasks", "this depends on that" |
| 2 | `humata-sessions` | Sessions and channels: start, fork, continue elsewhere, talk to another session, read and search history, rename, stop, archive, restore, create a channel. | "start another session", "fork yourself", "ask the other session", "what did that session say", "what's running", "archive that session", "make a channel" |
| 3 | `humata-remote` | Other machines: which exist, start or fork a session there, move a session, copy files between machines, transfers, skills sync, machine scripts, add or start a machine. | "run this on the GPU box", "move this session to the server", "copy the dataset over", "which machines do we have", "spin up a VM" |
| 4 | `humata-inbox` | The owner's approval: check a rule first, handle a pending request, ask ahead, ask the owner a question. | "ask me before you", "check with me first", "why was that denied", "what are you waiting on"; also any command that exits 12 |
| 5 | `humata-pull-requests` | Push and open pull requests, stack commits into chained pull requests with `stack`, start a PR review and draft comments. | "push this", "open a PR", "stack these commits", "restack", "review PR 123", "draft review comments" |
| 6 | `humata-files` | Files outside the agent's own clone: `shared/`, the channel folder, main clones, other sessions' files, the trash. | "put this in shared", "read the file from the other session", "grab that from main", "restore the file I deleted" |
| 7 | `restart-doctor` | Find out why a restart didn't bring everything back, repair what can't lose anything, write a diagnosis. | "my session didn't come back after the reboot", "run the restart doctor" |
| 8 | `render-humata-review` | Show only: prepare a change as a stack of layers across every repo it touches and send it to the review view, or show one narrow diff. No notes, no opinion. | "show me", "what changed", "show the stack", "let me look", "I just want to see the code" |
| 9 | `review-humata-change` | Review when asked: findings on their lines as notes, PR comment drafts for a pull request under review (never posted), a short verdict. | "review this", "review my PR", "find problems", "leave comments", "is this ready" |
| 10 | `order-humata-review` | Walk the person through a change: a reading order, groups and neutral notes that explain. No verdict. | "walk me through this", "explain this change", "what order should I read this in" |

The three review skills are split by what the person asked for (D791):
look, understand, or judge. Each description names the other two as "not
for". Keep it that way when editing one: "show me" must never start a
review, and "review this" must never only render.

Merged or dropped:

- **Channels** are part of `humata-sessions`: an agent's channel commands
  are few, and they come up together with sessions.
- **Transfers** are part of `humata-remote`; `humata-files` covers one
  machine. Both describe locations.
- **Stack** is one reference file of `humata-pull-requests`. It points at
  `stack guide`, the manual the tool carries, instead of repeating it.
- **Desk** is one command (`humata desk keep`) in `humata-sessions`.
- **Settings, agent kinds, access rules, pins**: the owner's alone, so no
  skill. `humata-inbox` says what to tell the owner.
- **A general "humata" skill** was dropped. `shared/basics.md` holds what it
  would say, and each skill's description carries its own triggers.

## How a skill is laid out

    skills/
      shared/basics.md            # how `humata` answers; copied into every skill as shared/basics.md
      shared/diff-spec.md         # what a review compares (for the review skills)
      shared/stacks.md            # a change as layers across repos, and how to send it
      humata-board/
        SKILL.md                  # front matter + a few lines that point to the files below
        board.md                  # commands with real flags, workflows, failures, what needs the owner
      ...

- `SKILL.md` is a thin pointer: `name` (the folder's name), `description`
  (what it does and the phrases that should trigger it), then which file to
  read. Agents keep every description in context all the time, so the
  description is where the triggers go and the body stays short.
- The reference files hold everything else. Each has the same parts:
  commands, workflows, what needs the owner, failures, and "Not in the
  CLI".
- Paths in a `SKILL.md` are relative to the installed skill folder. There
  `shared/` sits inside the skill; in this source tree it sits beside it.
- Skills are Markdown only. Nothing here is executable.

## Rules for writing one

- Every command and flag must exist in `cli/src/args.rs` today (and
  `tools/stack/src/cli.rs` for `stack`). Don't document a flag from
  `api/CLI.md` without checking it there. An MCP tool is named only where
  the CLI has no command, and only if `daemon/mcp/tests/tools.txt` lists it.
- Something the CLI can't do goes under "Not in the CLI", not into a
  workflow.
- Say what needs the owner's approval, from SPEC IN-17, and what is the
  owner's alone.
- Never tell an agent where a repo, clone or folder is (D743). It asks
  `humata whoami`.
- Keep the wording plain. Write for an agent that has never seen Humata.

## How they are installed and updated

- The folders here are packed into the daemon at build time
  (`:bundle`, embedded by `//apps/humata2/daemon/machines`). A new folder
  with a `SKILL.md` is picked up by the next build; nothing else lists the
  skills.
- Each time `humatad2` starts, on the Mac and on every machine, it writes
  them into `~/.claude/skills`, `~/.codex/skills` and `~/.copilot/skills`,
  for each of those agents whose own folder (`~/.claude`, `~/.codex`,
  `~/.copilot`) exists. It never creates an agent's folder and never
  installs an agent's CLI for this.
- Each installed skill folder holds a `.humata-skill` mark. A marked folder
  is replaced when it differs from what the daemon carries, and removed
  when the daemon no longer ships that skill. A new Humata release
  therefore updates the skills at the daemon's next start.
- A folder of the same name without the mark is the person's own and is
  never touched. To keep an edited copy of one of Humata's skills, delete
  its mark.
- The skills sync (SK-1) leaves marked folders out. Each machine's own
  daemon installs the skills that match its `humata`.
- Tests point the daemon at a temporary home with `--skills-home`. A daemon
  given its own `--state` folder and no `--skills-home` installs nothing,
  so a test never reaches the real home.

## Gaps

Known gaps are listed in DECISIONS.md D790 and in each reference file's
"Not in the CLI".

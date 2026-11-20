# Files

Your own clone is yours: read and write it with your normal tools. Use
`humata files` for everything else Humata keeps, because those reads and
writes go through the owner's rules and deleted files go to a trash they
can restore from.

## Locations

| Location | Means |
| --- | --- |
| `path` or `./path` | a path in your own clone |
| `shared`, `shared/<path>` | the channel's shared folder on your machine (your clone's own folder of that name is `./shared/...`) |
| `<machine>:shared/<path>` | the channel's shared folder on a machine |
| `<machine>:channel/<path>` | the channel's folder on a machine |
| `<machine>:main/<repo>/<path>` | the main clone of a repo (read-only) |
| `<machine>:<session>/<repo>/<path>` | a file in a session's clone |
| `<machine>:folder/<session>/<path>` | a Desk session's own folder |

Your machine's name is in `humata whoami`. Never build real paths under
Humata's folders yourself; name the location.

## Commands

    humata files ls <location> --json             # list a folder
    humata files cat <location>                   # print a file
    humata files find <session> "<query>" --json  # files in a session's clones whose names match
    humata files put <local-path> <location> [--replace]   # write one file
    humata files mkdir <location>                 # make a folder
    humata files mv <location> <new-name>         # rename in place (same folder)
    humata files rm <location>...                 # move to Humata's trash

- `put` fails when the name is taken unless `--replace` is given. A
  replaced file goes to the trash.
- `mv` only renames. To put a file elsewhere, `put` it there or copy it.
- `rm` never deletes for good: it moves to the machine's trash.

Fetch something into your own clone:

    humata cp shared/spec.md ./docs/ --json
    humata cp <machine>:main/<repo>/<path> ./vendor/ --on-existing replace --json

`--on-existing` is `fail`, `replace` or `keep-both`. With one source, a
destination that is a new or existing file name copies to that file; a
destination ending in `/` is a folder.

The trash:

    humata trash --json                 # what is in your machine's trash (--machine for another)
    humata trash restore <item> --json --wait 45 --key <key>

## The shared folder

`shared/` is where sessions of a channel hand each other files on one
machine: a spec, a dataset, a build output, a report.

- Write there with `humata files put <local> shared/<name>`.
- Tell the other session the location, not a path:
  `humata session note <session> "the report is at shared/report.md"`.
- Use plain names and a folder per topic. Check `humata files ls shared/`
  first and don't replace someone else's file without asking.
- Never put secrets, tokens or keys in `shared/`.

## Reading another session's files

    humata files ls <machine>:<session>/<repo>/ --json --wait 45 --key <key>
    humata files cat <machine>:<session>/<repo>/<path> --wait 45 --key <key>

This asks the owner, once per session and path; after they approve, the
same read goes through when asked again. Ask for the folder you need, not
file by file. You can never write into another session's clone: send that
session a note, or put the file in `shared/`.

## What needs the owner

| Action | Default rule |
| --- | --- |
| Read and write your own clone | Always allowed |
| Read `shared/`, the channel folder and main clones on your machine | Allowed |
| Write into `shared/` on your machine | Allowed |
| Fetch from `shared/`, the channel folder or a main clone on your machine into your clone | Allowed |
| Read another session's files on your machine | Asks the owner |
| Read files on another machine | Asks the owner |
| Copy files in from another session or another machine | Asks the owner |
| Restore from the trash | Asks the owner |
| Write to a main clone, another session's clone, or `shared/` on another machine | Refused |

Only the owner: emptying the trash, and copying or moving between places
that are not your clone or your machine's `shared/`.

## Failures

| What you see | What to do |
| --- | --- |
| exit 3 `not_found` for `shared/...` | It looked in the channel's shared folder. Your clone's own folder of that name is `./shared/...`. |
| exit 4 `main_clone_read_only` | Main clones are read-only. Copy the file into your clone and change it there. |
| exit 4 `denied` or `not_in_channel` | The place is outside what your session may reach. Tell the owner; don't use another tool to get there. |
| exit 12 `pending` | The owner must approve the read or copy. Don't repeat it without the same `--key`. |
| exit 2 on a `cp` destination | A bare word that isn't a machine, `shared` or a path with a `.` is refused. Write `./name` for your clone or `name/` for a folder. |
| the name is taken | Choose `--replace` (for `put`) or `--on-existing` (for `cp`) on purpose, or pick another name. |
| exit 8 | The machine or the Mac is away. Say what is waiting. |
| exit 10 `disk_full` | Stop writing. Tell the owner which machine. |

## With and without the mount

When `$HUMATA_FUSE_ROOT` is set, every location can also be read as plain
files under it.

When it is unset, Humata's folder is not mounted on this machine. Then use
`humata files` and the MCP files tools (`files_list`, `file_read`,
`file_write`, `files_find`, `folder_create`, `file_fetch`, `trash_restore`)
for shared, channel, main, other sessions, Desk and trash.

## Not in the CLI

- Reading part of a large file (MCP: `file_read` with `offset` and
  `max_bytes`).
- Older versions of a document in a session's folder (MCP:
  `doc_versions_list`, `doc_version_read`).
- Moving a file to another folder in one step.

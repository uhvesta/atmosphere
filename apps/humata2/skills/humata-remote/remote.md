# Remote machines

The owner's Mac is the **host**. Other machines (SSH hosts, VMs a provider
made) are linked to it. A channel lists the machines its sessions may run
on. Each machine runs its own Humata daemon, so `humata` works the same
there; calls that need the host go up to it. `humata machine ...` is the
same as `humata host ...`.

Never reach another machine with `ssh`, `scp` or `rsync` yourself. Humata's
commands carry the owner's rules; your own connection would go around them.

## Which machines

    humata whoami --json                  # the machine you are on
    humata host ls --json                 # the machines you can see (your channel's), with reachability
    humata host show <machine> --json     # one machine in detail
    humata topology --json                # the channel's machines and the sessions on each
    humata channel grid <channel> --json  # which repos are cloned on which machines
    humata host storage <machine> --json  # disk use there

`this-mac` names the host Mac from any machine. Check a machine is online
before you plan work on it.

## Run something on another machine

You don't run commands there from here. You put a session there, and its
agent does the work:

    humata session start --machine <machine> [--agent <agent>] [--model <model>] \
        [--repo <owner/name>]... [--title "<title>"] --message "<brief>" \
        --json --wait 45 --key <key>

Write `--message` as a full brief: the goal, the commands to run, what to
report and to whom (your session's title). Then follow the operation
(`humata ops <id> --json`), and talk to the new session with
`humata session ask` or `note` (see the `humata-sessions` skill).

To take your code and conversation along, fork onto the machine instead:

    humata session fork <session> --kind code-and-context --machine <machine> \
        [--title "<title>"] [--branch <branch>] --json --wait 45 --key <key>

`--kind` is `code-and-context`, `code-only` or `context-only`.

## Move a session to another machine

    humata session move <session> --to <machine> --json --wait 45 --key <key>

The session stops, its folder and conversation go to the machine, and it
resumes there. Use your own session (from `humata whoami`) to move
yourself: you are stopped mid-move and resumed on the other side, so finish
what you are doing and say what comes next before you ask. Follow a move of
another session with `humata ops <id> --json`.

The session's folder goes with it. A move that can't
finish is undone or reported stuck; never redo its steps by hand.

## Files between machines

A location is written `<machine>:<place>/<path>`:

| Location | Means |
| --- | --- |
| `<machine>:<session>/<repo>/<path>` | a file in a session's clone |
| `<machine>:shared/<path>` | the channel's shared folder on that machine |
| `<machine>:channel/<path>` | the channel's folder there |
| `<machine>:main/<repo>/<path>` | the main clone (read-only) |
| `<machine>:folder/<session>/<path>` | a Desk session's own folder |

Read on another machine:

    humata files ls <machine>:shared/ --json
    humata files cat <machine>:<session>/<repo>/<path>

Copy **in** to your own clone or to `shared/` on your machine:

    humata cp <machine>:shared/data/set.tar ./data/ --json --wait 45 --key <key>
    humata cp <machine>:<session>/<repo>/out/report.md shared/ --on-existing keep-both

- `--on-existing` is `fail`, `replace` or `keep-both`.
- Two or more sources always go into a folder.
- `./name` is always a path in your own clone. A bare `shared/...` is the
  channel's shared folder on your machine; your clone's own folder of that
  name is `./shared/...`.

As an agent you can only copy in: into your own clone or `shared/` on your
own machine. You can't send files to another machine, and you can't move
(`humata mv`) across machines. To get a file **to** another machine:

1. put it in `shared/` on your machine (`humata files put <local> shared/<name>`), then
2. ask a session on that machine to copy it in
   (`humata session ask <session> "copy <your-machine>:shared/<name> into your clone"`),
   or ask the owner to send it.

Transfers are chunked, resumable and checksummed. A dropped link pauses
them and they go on by themselves.

    humata transfers --json            # yours (--active: running and paused only)
    humata transfer pause <id>
    humata transfer resume <id>
    humata transfer cancel <id>

A copy counts as done only when `transfers` shows it finished. Don't use
the files before that.

## Reach a port on another machine

There is no CLI command to open one. The MCP tool `network_connect`
(`host`, `port`, `reason`) asks the owner and returns a local socket.

    humata bridge ls --json            # forwards that are open
    humata bridge close <forward>      # close one of yours

## Skills and scripts on machines

    humata skills sync [<machine>...] --dry-run    # what would change
    humata skills sync [<machine>...] --json --wait 45 --key <key>

Copies the owner's agent skills to machines (all when none is named).
Offline machines are skipped and named. A machine's first sync, or one
with a new name clash, has to be looked at by the owner: your call is
refused and names the machine. Tell the owner. Humata's own skills (this
one too) are not part of the sync: each machine's Humata installs them.

    humata host script list [<machine>] --json     # the owner's rules, triggers and last outcomes
    humata host script check                       # is the rules file valid (exit 2 when not)
    humata host script log <machine> [--rule <rule>] [--limit <n>] --json
    humata host script run <rule> <machine> [--session <session>] --wait 600

Scripts are the owner's files. You can read rules and logs. Running one is
Never by default, and nothing writes rules.

## Adding and starting machines

    humata host add <ssh-host> [--name <name>] [--root <path>] --json --wait 45 --key <key>
    humata host reconnect <machine>
    humata provider ls --json
    humata provider create <provider> <name> [--param <key>=<value>]... --json --wait 45 --key <key>
    humata provider start <machine>
    humata provider stop <machine>
    humata provider status <machine> --json

- `host add` takes an alias from the owner's SSH config. The first add of a
  host prints its key fingerprint; confirming it (`--trust-host-key`) is for
  the owner, who must check it is the host's. Don't pass a fingerprint you
  only read from the error.
- Never ask the owner to pick a folder. Humata places things by its roots.
- Stop a machine only when the owner asked, or you started it for this
  task.

## Machine providers

A provider is a set of commands that make and run a kind of machine (a
cloud VM, a local VM). You can read them and ask the owner to add one:

    humata provider ls --json                # ids, labels, params, interactive; no commands
    humata provider config show --json       # the owner's providers file
    humata provider import <file.toml> --reason "<why the task needs it>" --json --key <key>   # asks the owner in the Inbox
    humata provider import --help            # the file format, with the Lima provider as example

`provider import` never adds anything itself: it files a request that
shows the owner the whole file and every command in it, and answers exit
12 `pending`. Don't repeat it. The MCP tool `provider_import` files the
same request.

The file has one `[providers.<id>]` table per provider:

    [providers.mycloud]
    label = "My cloud VM"
    params = [
      { name = "size", prompt = "Size", default = "small" },
      { name = "region", prompt = "Region" },
    ]
    env = ["MYCLOUD_TOKEN"]
    create = "mycloud create --size \"$HUMATA_SIZE\" --region \"$HUMATA_REGION\" {name}"
    connect = "mycloud ssh {name} -- sh -c {cmd}"
    start = "mycloud start {name}"
    stop = "mycloud stop {name}"
    delete = "mycloud delete --yes {name}"
    status = "mycloud status {name}"

- `create` is required, and exactly one of `connect` (runs `{cmd}` on the
  machine; `{cmd}` must appear in it and nowhere else) or `identity`
  (prints the machine's SSH details as JSON). `start`, `stop`, `delete`
  and `status` are optional; Humata offers only the actions defined.
- `{name}` is the machine's name in any command. Each param arrives as
  `$HUMATA_<NAME>`, upper case (`size` is `$HUMATA_SIZE`), and the name as
  `$HUMATA_NAME`. Quote them in the script; never paste values into the
  command text.
- `env` names host variables of the owner's Mac the commands get by the
  same name. Never put a token or password in the file: ask the owner to
  set it in Settings › This Mac › Variables and name it in `env`.
- `humata provider import --help` shows the full key list and the Lima
  provider Humata ships as a worked example.

## What needs the owner

| Action | Default rule |
| --- | --- |
| See your channel's machines and their sessions | Allowed |
| Read files on another machine | Asks the owner |
| Copy files in from another session or machine | Asks the owner |
| Fetch from `shared/`, the channel folder or a main clone on your machine into your clone | Allowed |
| Write into `shared/` on your machine | Allowed |
| Start or fork a session on another machine | Asks the owner |
| Move a session to another machine | Asks the owner |
| Open a network connection through the host | Asks the owner (never Allow) |
| Add, create, start, stop or reconnect machines; sync skills | Asks the owner |
| Read the providers and their file | Allowed |
| Add a machine provider (`provider import`) | Asks the owner (never Allow) |
| Run a machine script | Never |
| Remove, forget or delete a machine | Never |

Only the owner, never an agent: a machine's root (`host root`), its VM tier,
its variables (`host var set|rm`), trusting a changed host key
(`host trust-key`), editing the provider config, adding providers (an
agent's import only asks) and provider login, emptying the trash, and
`humata shell`.

## Failures

| What you see | What to do |
| --- | --- |
| exit 8 `machine_offline` | The machine is off or unreachable. `humata host show <machine>` says since when. `host reconnect` asks the owner. Don't loop. |
| exit 8 `host_unreachable` | The owner's Mac is away. Calls that need it wait or fail by the owner's away rule. Work on what is local and say what is waiting. |
| exit 7 `host_key_changed` or `host_key_unknown` | Stop. Only the owner can check and trust a host key. Tell them the machine. |
| exit 12 `pending` | The owner must approve. Don't repeat the call. |
| a transfer is paused | It names the machine it waits for and continues by itself. Cancel only if the owner asks. |
| exit 11 `checksum_mismatch` | The copy didn't arrive whole. Start the copy again; don't use the file. |
| exit 10 `disk_full` | `humata host storage <machine>` shows what takes the room. Ask the owner what to clear. |
| an operation ends `stuck` | `humata ops <id> --json` says which step and what to do. Don't redo steps by hand. If it followed a restart, use the `restart-doctor` skill. |

## Not in the CLI

- Opening a network forward (MCP: `network_connect`).
- Sending files from your machine to another one as an agent, and moving
  files across machines: the owner's, in the file manager or with their own
  `humata cp|mv`.
- Running one command on a machine without a session there.

# Humata: spec and milestones

As of Nov 11, 2026. Humata was called Khodr until Milestone 3; the original
hand-off package is folded in here, so this is the one document to work from.

## What we're building

Humata is a Mac app for directing coding agents the way you'd run a team. Work
sits on a Jira-style board. Every project, task and subtask can have chat
sessions with an agent, and sessions fork into trees. The code lives on your
Mac or on a machine you reach over SSH, and you review the agent's changes
inline before they ship.

It's for one developer running several agents across several repositories.

The outcomes it has to deliver:

- **Keep track of many agents at once.** One place shows what every session is
  doing and which ones are waiting on you.
- **Try alternatives cheaply.** Fork any session at any point and get the chat
  and the code as they were. A fork can also take only the chat and keep
  working in the same copy of the code.
- **Work from a laptop, run on a server.** Code and agents stay on a remote
  machine and keep going when the laptop closes, and a lost machine can be set
  up again without losing the work.
- **Work across repositories.** One session can span several git repositories.
- **Go from idea to merged pull request in one app.** Plan with the agent, let
  it build, review inline, ship.

## Core concepts

| Concept | What it is |
| --- | --- |
| Project | A name, a set of repositories, instructions for the agent, and a board. It lives in the app, and its sessions can run on any host. |
| Task and subtask | A ticket on the board with a status. A task can have subtasks; a subtask can't have its own. |
| Spec | The Markdown definition every project, task and subtask has. A session reads only the specs it is told to: its own item's alone, or its item's and those of the items above it. |
| Host | Your Mac, or a machine reached over SSH. It holds the code, runs the agents and keeps their session state. |
| Session | One chat with an agent on one host, attached to one project, task or subtask, or to no project at all (a free chat). It gets its own worktree in each repository before its first turn. |
| Turn | One message from you and the agent's work in reply. Each turn ends with a hidden snapshot of every repository it works in. |
| Fork | A new session that starts from any turn of another. It takes the chat as it was at that turn, and either its own copy of the code at that turn or, for a chat-only fork, the parent's copy and working directory. Forks form the project's session tree. |

The rules that tie them together:

- **One item per session.** A fork inherits its parent's item and can be moved
  to another.
- **New work is a new root.** Starting fresh on a project creates a root
  session in its tree.
- **Editing an earlier message forks.** There is no branching inside a session,
  so the chat and the code never disagree.
- **Sessions drive status.** A session's state moves its item on the board, and
  parents roll up from their children.
- **The agent proposes, you apply.** Changes to the board and to specs need
  your approval, like shell commands do.
- **Your git history is yours.** Humata never commits to your branches on its
  own. Turns are saved as hidden refs (`refs/humata/<session>/<turn>/<repo>`);
  your branch, index and log stay untouched until you land the work.

## Decisions and scope

These are settled.

| Decision | Why |
| --- | --- |
| The app is Humata, under `apps/humata`; the daemon is `humatad`, the tool `humata` | Renamed from Khodr by the owner after Milestone 3. |
| Single user, no iCloud sync | Removes sync, sharing and permissions from every milestone. |
| Your Mac owns the board; each host owns its agent state | Projects, tasks, specs and the session tree live on your Mac in SQLite. Session files stay on the machine that ran them. The Mac also keeps a recovery copy of each remote session (Milestone D). |
| Every host's sessions are combined into one view | Each host serves its history over the SSH connection already open. Nothing is mounted (no FUSE) and no port is opened. |
| What travels between machines is compressed with zstd | High level for specs, files and history backfill; fast level for live events. |
| A Rust daemon holds the logic; clients reach it by protobuf over a local socket | The Swift app, the command-line tool and the MCP server are clients of one protocol (`proto/humata/v1/humata.proto`). |
| One Rust binary, several roles | On your Mac it is the daemon that owns the board. On a remote machine it runs sessions next to the code and answers your Mac over SSH. `humatad mcp` serves agents. |
| Agents sit behind one interface with four backends | Copilot through its CLI, Claude Code through the `claude` CLI signed in with a subscription, a local model over an OpenAI-compatible server, and a scripted backend for repeatable tests. Copilot can also point at a local model server, so the real integration can be tried for free. |
| Turns are snapshot refs, not commits | The owner asked why Humata should commit; snapshots keep forks exact without touching history. |
| Worktrees live in the OS cache folder | `~/Library/Caches/Humata` on the Mac, the XDG cache on Linux, per session per repository. |
| Rust for the logic, Swift with TCA for the UI | Kits stay TCA-free; app features use TCA with TestStore tests, FlowKit flows and snapshots. |
| Secrets stay off disk on hosts | The host's GitHub token and variables are sent over SSH into the host process's memory, never written to a project file or the host's disk. Values never go back to clients, only names. `hosts.json` is mode 0600. |

Not in these milestones: teams and sharing, an iOS app, a code editor, Windows
hosts.

## Architecture

```text
Your Mac
  Swift app (SwiftUI and TCA features)  --+
  humata (scripts, e2e tests)            --+--> protobuf over ~/.humata/daemon.sock (0600)
  humatad mcp (agents and other tools)   --+
                                           |
                                           v
  humatad daemon: projects, items, specs, the session tree, seen state,
                  review threads, the hosts list, and a recovery copy
                  of every remote session
                                           ^
                                           |  the same protocol, over SSH
                                           v
Each host: your Mac, or a machine you reach over SSH
  humatad host: sessions, forks, worktrees, a snapshot per turn,
                terminals and file reads; keeps going when your Mac is away
      |--> Agent backend: Copilot, Claude Code, a local model, or scripted
      |--> Agent session state: history files, kept per machine
      +--> Git repositories: a worktree per session, in the cache folder
```

- **Daemon:** starts at login through launchd, keeps running with the app's
  window closed, takes the login shell's `PATH` so agents' programs are found,
  and is replaced when the app updates.
- **Host role:** the same binary on each machine. Hosts that don't answer fail
  within 30 seconds and rest with a growing backoff, so one slow machine never
  stalls the app.
- **Swift features:** TCA features over the `DaemonClient` protocol, with
  `InMemoryDaemon` beside the live one.
- **Tests:** end-to-end tests drive the daemon through `humata` with the
  scripted and local backends, and with the real Copilot CLI against a fake
  OpenAI server, offline. SSH tests start a throwaway `sshd` on loopback inside
  the test. FlowKit flows run in the background (`run_mac.sh`) so they never
  take over the screen.

## Status at a glance

Build order, top to bottom. Milestones 0–8 keep their numbers from the
original spec; lettered ones were added since.

| Milestone | Status |
| --- | --- |
| 0 Prove the risky parts | Done (7c2c595) |
| 1 Work with an agent in a repo | Done (7c2c595) |
| 2 Session trees | Done (4a2e71a) |
| 5 The board | Done (8ff3b84), built before 3 |
| 3 A remote machine over SSH | Done (93d09ac) |
| 4 Code review | Done (e7d5153) |
| A Free chats and doc mode | Done (dfb6582) |
| B Claude Code and MCP | Done (8f6091c) |
| C Polish: blockers and quick wins | Done (059649d) |
| C Polish: majors and minors | Done |
| D Durable hosts | Done |
| E Terminals and files | Done |
| 6 Plan with the agent | Done (8dba5fb0) |
| F Any host, any repository | Done |
| 7 Search across sessions | Done |
| 8 Ship as pull requests | Done |
| G Drive Humata from outside | Done |
| S Settings and environments | Done |
| H Humata's own model server | Done |
| The demo | Done |
| R Review pass | Done (9e2ac82e) |
| J Agents ask, and a board journal | Done |
| V Second review pass | Done |
| P Performance pass | Done |
| K Signed releases | Done: humata-v0.5.0 is the latest |
| S1 Storage: safe places, cleanup, opt-in auto-delete | Done |
| Q1 Queue and composer: edit, merge, cancel, pastes, images | Done |
| Q2 Rewind and checkpoints | Done |
| Q3 Skills from the agents' folders | Done |
| Q4 Plugins | Done |
| Q5 Models: defaults, Copilot experimental and Fusion, models on the Mac | Done |
| Q6 Auto approvals: the agent's own safety judge | Done |
| Q7 Spec tab: every spec a chat reads, in the chat | Done |
| T1 Tokens and agent tools | Done |
| U1 App jank | Done |
| D1 Mermaid diagrams | Done |
| D2 Focus view for Markdown | Done |
| D3 Versions of a Markdown file | Done |
| U2 UX papercuts | Done |
| S2 Compression | Done |
| S3 Daemon speed | Done |
| G1 Ghostty terminal | In progress |
| X Admin mode, gated by approvals | Done |
| F2 Fresh clones | Done |
| TR tca-rust, a Rust take on TCA | Done (ff6ee013) |
| H2-0 Humata 2 skeleton | Done (e6ac1c84) |
| H2-1 Humata 2 chat | Done (5b95b95f) |
| H2-2 to H2-5 Humata 2 screens, then the swap | Stopped |
| TR2 tca-rust speed | Stopped |
| P2 Humata 1 speed, measured | Next |

## Done milestones

Kept short; the code and its tests are the record.

### Milestone 0: prove the risky parts — done

- [x] Agent interface with a scripted backend, so tests need no model
- [x] Local-model backend behind the same interface
- [x] The real Copilot CLI behind it, running against a local OpenAI-compatible server at no token cost
- [x] Fork at an earlier event on each backend; the fork's history stops there
- [x] Copilot CLI pinned (1.0.110-1) as a Bazel dependency for macOS and Linux
- [x] Protocol in protobuf with Rust and Swift types from the Bazel proto rules
- [x] The daemon called over a local socket from Swift and from `humata`, with event streams
- [x] The same binary run over SSH from a hermetic Bazel test

### Milestone 1: work with an agent in a repo — done

- [x] Daemon at login, kept running, replaced on update
- [x] Projects, sessions, streaming replies; thinking, commands and output in the chat
- [x] Approvals with ask every time, allow edits, allow everything
- [x] Stop, steer, queue; backend and model per session; usage
- [x] Everything kept in the daemon's store; everything also from `humata`

### Milestone 2: session trees — done

- [x] Several repositories per project and per session
- [x] A worktree per session per repository, before its first turn
- [x] A snapshot ref per turn per repository, recorded against the chat
- [x] Fork with the code at any turn, or chat-only in the parent's copy
- [x] Editing an earlier message forks
- [x] The project's sessions as a tree with name, state, item and host
- [x] Rename, archive (removing worktrees), compare two sessions, mark the keeper
- [x] A fork's inputs kept portable: the chat to a turn and a snapshot per repository

### Milestone 5: the board — done

- [x] Tasks and subtasks with a Markdown spec, status and priority (a shared board kit, extracted from Lifr's)
- [x] Board columns: To do, In progress, Waiting on you, In review, Done, Won't do
- [x] A task's spec, its sessions and what each changed
- [x] Start a session from a project, task or subtask, choosing which specs it reads
- [x] Status from sessions' state, subtasks rolled up into their task
- [x] An inbox of sessions waiting on an approval, a failure or a reply, across projects
- [x] The board from `humata` and the MCP server
- [x] Every board change recorded and revertible
- [x] Inline comments on a task's spec → doc mode (A) for chat files; for specs, Milestone 6

### Milestone 3: a remote machine over SSH — done

- [x] Hosts from the SSH config, checked, with health: disk, Copilot and Claude versions, sign-in
- [x] Install and update the host program over SSH
- [x] Sessions keep running while the Mac is away, and catch up on reattach
- [x] Every host's sessions in one list; offline hosts' sessions shown as they were
- [x] zstd between machines
- [x] A GitHub token for the host, never written to disk
- [x] Clone a project's repositories on the host
- [x] Each host's variables and setup script edited from the app
- [x] Notifications when a session needs you or finishes
Language servers per host aren't part of this milestone; see Later.

### Milestone 4: code review — done

- [x] Diffs by range: unstaged, staged, against a branch, turn by turn, grouped by repository
- [x] Adjustable context and the whole file, added lines green and removed red
- [x] Threads on a line or range that stay attached as the code changes
- [x] Markdown rendered, with inline threads on the rendered text
- [x] Comments sent as one review; the agent's replies and fixes in the threads; resolve
- [x] Stage, unstage and revert by file or hunk; mark files viewed
- [x] Syntax highlighting in the diff and the whole file

### Milestone A: free chats and doc mode — done

- [x] Chats in any folder, or in a folder of their own, outside any project
- [x] Doc mode: the agent iterates on several Markdown files, rendered; highlight, comment inline, send the comments to the chat
- [x] Browse the chat's files

### Milestone B: Claude Code and MCP — done

- [x] Claude Code backend on the installed `claude` CLI, signed in with a subscription, no API key needed
- [x] `humatad mcp`: board tools and `reply_to_thread` for agents

### Milestone C, part 1: blockers and quick wins — done

- [x] A torn last history line is cut on open; a session that can't load is skipped, not fatal
- [x] The daemon and host take the login shell's `PATH`, and find agents' programs where installers put them
- [x] Slow hosts time out and back off; checking a host tries again at once; hosts listed in parallel
- [x] A reply read in the app leaves the inbox until the next turn ends; the Dock shows how many sessions wait
- [x] New Session remembers the last choices and takes a model server for Copilot or a local model
- [x] Move a session to a task or back to the project
- [x] Sessions reconnect on their own with backoff; a message that wasn't sent goes back in the box
- [x] Shortcuts (⌘N, ⇧⌘N, ⌘↩, ⌘1–3) and a Folder menu (copy path, Finder, Terminal)
- [x] Hosts show Claude Code's version and whether an Anthropic key is set
- [x] Tool calls show their subject: the command, file, pattern or URL

### Milestone C, part 2: polish majors and minors — done

**Outcome:** the app is pleasant to live in all day: you can act on what waits
for you without hunting, land work, and find your way around.

- [x] Approve or deny from the inbox: each approval row shows what the agent asks, with Allow and Deny
- [x] Approvals show the diff of a write, not only its path
- [x] A draft per session, kept when you switch away; queued messages shown above the box, each removable
- [x] History cost: each turn's tokens and premium requests, and the session's total
- [x] Sessions stay readable offline from the Mac's copy of their history
- [x] Sidebar: sorted by last activity, with timestamps, an unread mark, search by name, and delete project
- [x] Import existing sessions: Claude Code transcripts and Copilot session folders already on a host
- [x] MCP for every session: remote and Copilot sessions get the same `humatad mcp` tools as local Claude ones
- [x] The inbox holds what an agent asks of you, not only approvals and replies:
  - an agent asks a question or wants your opinion through a new `ask_person` MCP tool; the row shows it, and your answer goes to the session as a message
  - an agent that's finished asks to move its task through a new `request_move` tool (to In Review, Done or Won't Do) with a summary of what it did; `update_item` no longer sets those itself
  - the row opens the session's changes to look over, and from it you approve the move, send feedback that the agent carries on from, or move the task anywhere yourself
  - requests are kept in the daemon's SQLite, so they survive restarts and work for remote sessions through the MCP relay; the Dock count and a notification remind you until each is answered
- [x] Claude and Copilot parity: model lists, approval modes, steering and usage behave the same
- [x] Spec drafting: a task's spec edited in the app with a rendered preview
- [x] A variables editor for This Mac, like the hosts have
- [x] Sessions skipped on load are listed with why, and can be removed

**Done when:** a flow approves a write from the inbox and sees the session
working again, and the review flow's snapshots show the new rows.

## Remaining milestones

### Milestone D: durable hosts

**Outcome:** a remote machine can reboot, lose its network or disappear
entirely, and every session on it comes back: reopened where it was, or set up
again from scratch on the same or another machine.

Structure:

- **Detached host.** `humatad host` runs on its own on the remote machine and
  listens on `~/.humata/host.sock` (0600). Each SSH connection runs a small
  relay (`humatad relay`) that connects to the socket, or starts the host and
  then connects. Sessions no longer die with the SSH connection, and a
  reconnect re-watches each session from the last event the Mac has.
- **A recovery copy on the Mac.** For every remote session the daemon keeps,
  under its state folder: the full event history, each turn's snapshot
  commits in a bare mirror per repository (git bundles sent over the host's
  connection, holding only the commits the mirror lacks), and the backend's
  own state (Claude's transcript, Copilot's session folder), copied after each
  turn and kept with zstd.
- **Restoring.**
  - *The machine comes back:* sessions reopen; a turn that was cut off is
    marked aborted, so the session takes messages again.
  - *The machine is gone:* once the host is installed and set up again
    (`humata host install`, which reruns its setup), "Restore" clones the
    project's repositories, sends the snapshot commits the clone lacks from
    the mirror as a bundle, recreates the worktrees at the last snapshot, and
    puts back the history and the backend's state, so the agent resumes with
    its context.
  - *Restoring on another host* is the same steps against a different machine,
    which makes it a move. The session's host changes; its id, tree and item
    stay.
- **Mid-turn snapshots.** A save after each tool call that succeeds, not only
  at the end of the turn, so a machine lost mid-turn loses at most the call
  still running. Saves are their own `saved` events, kept apart from
  checkpoints so a turn's changes still run from one turn's end to the next.
  Calls that finish while a save is being taken are saved together once it's
  done, and each save is bundled to the Mac like a checkpoint. A fork or a
  restore starts from the newest of the two.

Work:

- [x] `humatad host` as a detached process with its socket, and `humatad relay` used by SSH
- [x] Reconnect re-watches from the last event, and a cut turn is marked aborted on restart
- [x] Snapshot commits bundled into a mirror on the Mac after each turn
- [x] Backend state (Claude transcript, Copilot session folder) copied to the Mac after each turn
- [x] Restore a session onto a set-up host: clone, bundle in the snapshots, worktrees, history, backend state (`humata recover session`)
- [x] Restore a whole host: every session it lost (`humata recover host`, after `humata host install`)
- [x] Restore onto another host, or this Mac, as a move
- [x] Mid-turn snapshots after each tool call that succeeds, bundled to the Mac
- [x] The Hosts view shows each host's sessions' recovery copies, Restore, Restore All and Move

**Done when:** an end-to-end test runs a turn on an SSH host, kills the host
process and deletes its state and worktrees, restores the session, and finds
the files, the history and the agent's context back; a second test restores
onto another host.

### Milestone E: terminals and files

**Outcome:** you never need a separate SSH window: a terminal and a file
browser for any session or host, on this Mac or a remote machine, inside the
app.

Structure:

- **Terminals.** The host role opens a pseudo-terminal running the login shell
  in a session's folder or a host's home, and streams its bytes over the
  protocol (`OpenTerminal`, `TerminalInput`, `ResizeTerminal`, an output
  stream). Remote terminals ride the existing SSH connection through the
  daemon; nothing new is opened. The host keeps the terminal running if the
  Mac disconnects, and replays its recent output on reattach. The app renders
  it with Ghostty in a session's Terminal tab and in each host's Terminals
  pane in Hosts, which lists every shell on the host, a session's or not.
- **Files.** A tree of a session's folder, or any folder on a host, read
  through the protocol (`ListFiles` and `ReadFile` already exist for chat
  files; they grow paths outside the session and lazy directory listing).
  Files open read-only with syntax highlighting, Markdown rendered, images
  shown, and large files cut with a note. Comments use the review threads.
- **Ports.** The daemon listens on 127.0.0.1 on the Mac, on the host's port
  number when it's free, and carries each browser connection to the port on
  the host over the host's connection (`ForwardPort`, then `ConnectPort`,
  `WritePort` and `ClosePort` per connection). The host only connects to
  127.0.0.1 there; nothing new listens on it. Each host's Ports pane in Hosts
  and `humata port` forward, list, open and stop them.

Work:

- [x] PTY in the host role, with input, resize, output stream, and replay on reattach
- [x] Terminal tab in a session, opened in its folder; ⌘T for a new one
- [x] Terminals for a host, outside any session, from the Hosts view
- [x] Terminals keep running when the app closes, and are listed so they can be closed
- [x] File browser for a session's folder and for any folder on a host
- [x] Read-only file view with highlighting, rendered Markdown and images
- [x] Open a file in an editor on this Mac (local) or copy its remote path
- [x] Port forwarding for a dev server on a host, opened in the browser

**Done when:** an end-to-end test opens a terminal on an SSH host, runs a
command and reads its output, disconnects and reattaches and still sees it;
a flow snapshots the terminal and the file browser. Both pass
(`terminal_over_ssh_test`, `port_over_ssh_test`, the `terminals` flow).

### Milestone 6: plan with the agent

**Outcome:** a rough idea becomes a specced project with tasks in one
conversation, and the agents doing the work always read the current spec.

- [x] Give the agent tools to read the board, read a spec, and propose tasks, subtasks and spec edits
- [x] Show each proposal as a card in the chat to apply, edit or decline, with nothing saved until you apply it
- [x] Start planning sessions without a checkout, so they open at once
- [x] Copy a session's specs to its host and hold the agent's proposals there, so sessions keep working while the app is away
- [x] Tell running sessions when a spec they depend on changes, and have them read the new one
- [x] Fork a planning session into a working session on a task it created
- [x] Pass each project's instructions and skills to every session in it
- [x] Comment inline on a task's spec, as in doc mode, and send the comments to a session

**Done when:** an end-to-end test has a scripted agent propose three tasks
with specs, applies two, and finds exactly those two on the board. It passes
(`planning_test`, with the `planning` flow).

### Milestone F: any host, any repository

**Outcome:** adding a Linux or Mac host over SSH just works, and a project
can start from a GitHub URL as well as a folder.

- [x] Build humatad for macOS (arm64, x86_64) and Linux (x86_64, arm64, static musl) when packaging, and bundle every build in the app under `Contents/Resources/builds/humatad-<os>-<arch>` (not `Helpers`, which codesign takes for Mac code; the Mac builds are signed ad hoc) (`//apps/humata/daemon/humatad:builds`, a split transition over the platforms)
- [x] On install, pick the build matching the host's `uname -sm`, copy it into the host's folder (`~/.humata/bin/humatad` by default) atomically, and say which platform is missing if none matches; a host of this Mac's platform with no bundled build gets this humatad
- [x] Link `humatad` into `~/.local/bin` on hosts, only where nothing or a link of that name is, and offer `humata` and `humatad` on this Mac (This Mac in Hosts, or `humata link`), saying how to add the folder to PATH when it isn't on it; shell profiles are never edited
- [x] Reinstall on a host whose humatad is another build than the app's, as it does for this Mac; a host with no build for its platform keeps the one it has
- [x] Make a project from GitHub repositories as well as folders, in any mix: add each by URL (`https://github.com/o/r` or `git@github.com:o/r`), and the Mac clones it into the cache folder once and fetches it before each session; hosts clone from the URL themselves, with their token passed to git through a credential helper so it's never written to disk
- [x] Add, remove and rename a project's repositories after it's made, for sessions started after (project settings, or `humata project update --repository`)

**Done when:** an end-to-end test installs the bundled Linux build on a Linux
host over SSH, and a project made from a URL to a local git server starts a
session with the repository checked out. Both pass. This Mac has no
container runtime, so `install_build_test` stands in for the Linux container:
the test sshd's `uname` says Linux, and the test checks the Linux build and
its link are installed (a session can't run, as the build can't run on a
Mac). `a_project_from_a_url_checks_out_whats_there_when_each_session_starts`
clones from a `file://` URL and finds a later commit in the next session.

### Milestone F2: fresh clones

**Outcome:** a session starts from what its repository's remote has now, on
this Mac and on every host, and says so when it can't. An agent told "this
clone is from June" on a host: the session had started from an old commit.

Today the Mac fetches its URL clones and fast-forwards their branch before
each session, but a failed fetch or fast-forward only reaches humatad's log.
Hosts run `git pull --ff-only` in their clones, which fails once the clone's
branch has moved off its upstream. Folders are never fetched. Worktrees start
from the clone's own `HEAD`, so any of these leaves a session on an old
commit without a word.

- [x] Fetch `origin` with `--prune` before each session, on this Mac and on hosts, and start the session's worktrees from the remote's default branch (`origin/HEAD`, set with `git remote set-head origin --auto` when missing), not the clone's local branch; nothing is merged, so a clone's branch can't block it
- [x] A folder repository is fetched too, without touching the folder's checkout; its sessions start from the folder's `HEAD` as now, and the new session says how many commits that is behind the remote's default branch when it is
- [x] Show "Fetching <repository>…" with a spinner where the session is starting (New Session's sheet, the item's Start Work, its card) while it fetches, off the main thread, and the session starts when it's done
- [x] A fetch that fails, as offline or with a token that has expired, starts the session from what was fetched before and shows the session a notice: what failed, how old the starting commit is, and how to fix it (sign in again with `gh auth login` on that host, or reconnect)

**Done when:** a test pushes a commit to a `file://` remote after the clone is
made, moves the clone's branch off its upstream, and the next session on this
Mac and on the SSH test host starts at the new commit; and a test with the
remote gone starts the session from the old commit and shows the notice.

### Milestone 7: search across sessions

**Outcome:** anything an agent said or did can be found again, by you or by
another agent.

Structure:

- **One index, on the Mac.** An SQLite FTS5 table in the daemon's store
  (porter stemming, so "test" finds "tests" and "testing") holds your
  messages, the agent's replies, each tool call (a shell call by its
  command) and the start of its output, review comments, and specs. Each
  entry keeps its session, event, turn, time, project, task and host.
- **Caught up before each search.** The daemon lists every session, as it
  does for the sidebar, and reads the history of each one whose turns or
  state changed since it was last indexed, indexing only the events after
  the last one it has. A host that's offline answers from what's indexed
  already, and from its sessions' last-read histories, so one query covers
  every machine. Specs and comments are indexed again from the store.
- **Words and time.** A query is words, all of which must match, and may
  hold a time, such as "today", "yesterday", "this week", "last week",
  "this month", "last month" or "last 3 days", taken out of the words and
  read in this Mac's time zone. It can be narrowed to a project, a task or
  a session. Hits come best match first, with a snippet marking the words
  found.
- **Related words.** The app asks for words related to the query, from
  Apple's on-device model, falling back to the system's word lists, as
  Lifr's search does: both now come from one kit, `//apps/libs/kits/words`,
  which Lifr uses too. They're matched only after the words themselves,
  and their hits are marked related. The CLI and agents search the words
  alone.
- **Opening a hit.** A hit opens its session scrolled to the event, which
  is highlighted, with "Fork from Here" forking the session after that
  turn.
- **Agents.** A `search_sessions` tool searches past sessions. An agent on
  this Mac searches every project; one on a host, through the relay, only
  the projects that have sessions on that host, as with the board.
- [x] Index messages, commands and their output, changes, review comments and specs on the Mac, from every host's history (SQLite FTS)
- [x] Search by words and by time, such as "last week", and filter by project, task or session
- [x] Related words from a cheap local model, with a word list as the fallback, the way Lifr's search does
- [x] Open a result at the matching point in its session, and fork from there
- [x] Give the agent a tool to search past sessions
- [x] Search every machine's sessions in one query, including offline ones

**Done when:** an end-to-end test finds a command run in one session from a
search started in another project, and opens it at that turn.

### Milestone 8: ship as pull requests — done

**Outcome:** a finished session becomes a merged pull request without leaving
the app, and the board closes the task by itself.

Structure:

- **Git where the code is, GitHub from the Mac.** Committing, pushing and
  merging run on the session's host, in its worktrees, with the host's
  token handed to git through the credential helper, as cloning does.
  GitHub's API is called only from the Mac, with the token in This Mac's
  variables (`GH_TOKEN` or `GITHUB_TOKEN`), or else the GitHub command
  line's (`gh auth token`), so the board can follow pull requests while
  hosts sleep. There's no OAuth app of Humata's own. `HUMATA_GITHUB_API` points the daemon at
  another server, for GitHub Enterprise or a test; with it set, any remote's
  last two path parts name the repository.
- **Landing.** "Land…" in a session's Ship menu names a branch (from the
  session's name, as `fix-the-migrations`) and a message (the session's name
  and the task's title). Each worktree with changes moves onto that branch
  and commits everything git doesn't ignore; one without changes is left
  out. A session can land again later: new changes are another commit on
  the same branch. Landing waits for a turn to end.
- **Pull requests.** "Land and Open Pull Requests" also pushes each landed
  branch to its repository's `origin` and opens a pull request against the
  branch the session started from (the repository's default branch). One
  pull request per repository; each one's description names the task and
  links the others. Landing again pushes to the same pull requests. The
  Mac keeps each pull request in its store, against its session and task.
- **Following them.** The Mac reads each open pull request's state, its
  reviews (the last of each reviewer's) and its checks every two minutes,
  and at once when a task's page or the session opens. The task's page and
  the session's toolbar show them. When every pull request of a session
  has merged, its task moves to Done, the session is archived (removing
  its worktrees, as archiving does) and its branch is deleted from each
  repository on the host; its snapshots stay, so its history and changes
  still read. A pull request closed without merging leaves things as they
  are.
- **How often.** Settings › GitHub picks whether the Mac reads them on
  its own (every 1, 2, 5, 10, 15, 30 or 60 minutes, or a custom 1 to 1440;
  two by default) or only when asked. Only when asked, nothing reads them
  but a task's Refresh, and the task's page says so beside it. A change
  takes effect at once: the follower waits by the new interval, not out
  the old one. Longer intervals spend less of GitHub's rate limit.
- **Review comments.** Each read also brings in the pull request's review
  comments: a new one starts an inline thread on its file and lines, or
  joins the thread it replies to, marked with its author's GitHub login.
  They wait, unsent, for the next "Send Review", as yours do.
- **Updating from the base branch.** "Update from the Base Branch" fetches the base
  branch on the host and merges it into each worktree, keeping what the
  session changed and hasn't landed. A merge that conflicts is left in the
  worktree, and the agent is sent the conflicting files, asked to resolve
  them and commit.
- **Issues.** "Import Issues…" on the board reads a repository's open
  issues (not pull requests) and makes a task of each, its title and its
  body as the spec, once: an issue imported before is skipped. A pull
  request for an imported task says it closes the issue.
- [x] Land a session's work: commit its changes on a branch of your naming, with a message you edit
- [x] Push the branch and open a pull request, one per repository, linked to each other and to the task
- [x] Show each pull request's review and CI status on the task, and move the task to Done when it merges
- [x] Bring GitHub review comments into the inline threads
- [x] Update a session from its base branch and hand conflicts to the agent
- [x] Import GitHub issues as tasks
- [x] Remove a session's worktrees and branches after its pull request merges

**Done when:** an end-to-end test opens a pull request against a local git
server standing in for GitHub, marks it merged, and finds the task Done and
the worktree gone.

### Milestone G: drive Humata from outside — done

**Outcome:** an agent outside Humata (Claude Code in a terminal, say) can
set Humata up and use it as you would: add projects, start chats and
planning sessions, talk to them, approve their requests, and show you the
result in the app.

Structure:

- **`humata mcp`, a control server.** The command-line tool gains `mcp`, a
  stdio MCP server you add to an outside agent (`claude mcp add humata --
  humata mcp`). It is a client of the daemon's socket like the app and the
  rest of the tool, so it reaches nothing they can't. It is not
  `humatad mcp`, which serves one session's agent its board and specs. No
  REST server and no port: an outside agent starts it as a child process.
- **The whole daemon as tools.** Projects (list, create, add a repository by
  path or GitHub URL), hosts (list, status), sessions (start a chat,
  planning session or task session with a backend and a model; send;
  stop; fork; archive), the inbox (list, approve, deny, answer), the board
  and specs (read, and propose tasks and spec edits), history and search. Each
  tool maps to one request of the protocol, so a tool can't do what the
  protocol refuses.
- **Show in app.** A `show` tool opens a session, a task or the board in the
  running app, through the app's URL scheme (`humata://session/<id>`), so
  the agent can put what it made in front of you. It brings the app
  forward only when asked to.
- **Safety.** Destructive tools (archive, remove a project, delete a host)
  say so in their descriptions and are marked destructive in MCP, so the
  outside agent's client asks you before calling them.
- [x] `humata mcp` serves the daemon over stdio MCP
- [x] Tools for projects, hosts, sessions, the inbox, the board, specs, history and search
- [x] A `show` tool and a `humata://` URL scheme in the app
- [x] Destructive tools marked so

**Done when:** an end-to-end test starts `humata mcp` against a test daemon,
creates a project, starts a scripted chat, sends it a message, approves its
request and reads the reply, all through MCP.

### Milestone S: Settings and environments

**Outcome:** machines are set up in one place, Settings (⌘,), the way the
owner thinks of them:

- **Adding a host:** Settings > Environments lists the hosts in
  `~/.ssh/config`. Each shows whether it has been added, the version
  installed there and whether it's reachable; Add takes one in, and a
  destination that isn't in the config can still be typed.
- **On the remote machine:** adding a host installs humatad there, and
  Settings offers Update when the bundled version is newer. Its variables,
  GitHub token and setup script are edited from the same pane. The token is
  still held only in memory.
- **Running sessions:** a session started on that host runs there, with its
  files browsed and edited, its terminals and its Git operations (review,
  land, pull requests) all on the host. That already works; this milestone
  doesn't change it.

Structure: a SwiftUI `Settings` scene on the Mac app over the existing
HostsFeature's add, install and setup actions, so no new protocol is needed.
The sidebar's Hosts view keeps what's running: sessions per host, recovery
copies, terminals and ports. On iPhone the same pane sits under the app's
settings.

- [x] A Settings window with an Environments pane
- [x] `~/.ssh/config` hosts listed with added, version and reachability
- [x] Add installs humatad; Update when a newer build is bundled
- [x] Variables, token and setup script edited from the pane

**Done when:** a flow test opens Settings > Environments, adds a host from
the SSH config and sees it installed and reachable.

### Milestone H: Humata's own model server

**Outcome:** local models run inside Humata, with nothing else installed:
the app serves them to its agents as an OpenAI-compatible endpoint.

Structure:

- **InferenceKit behind an OpenAI-compatible server.** The Mac app hosts a
  server on loopback (`127.0.0.1`, a port it picks and tells the daemon)
  with `/v1/models` and `/v1/chat/completions`, streaming and tool calls
  included, backed by InferenceKit's on-device engines (MLX and Apple's
  model), the same ones Lifr and LangReadr use. It is a kit-level server
  (TCA-free), in Swift, since InferenceKit is.
- **The local-model backend points at it.** The daemon's local-model backend
  already speaks the OpenAI protocol; This Mac's default endpoint becomes
  the app's server when it runs, and an Ollama or LM Studio server still
  works by setting it. The model picker lists the server's models.
- **Remote hosts** reach it through a reverse port forward over the SSH
  connection already open, bound to loopback on the host, so a session on
  a remote machine can use your Mac's GPU.
- **Limits, said plainly.** It runs while the app runs; a session using it
  waits, with the reason shown, when the app is closed. Small models call
  tools less reliably than large ones; the picker marks the models known to
  call tools well.
- [x] An OpenAI-compatible server on loopback, backed by InferenceKit, with streaming and tool calls
- [x] This Mac's local-model backend uses it by default; its models in the picker
- [x] Remote hosts reach it over a reverse port forward
- [x] A session waits with a reason when the server is down
- [x] The picker marks the models known to call tools well: the app measures
  each model once, in the background, with small tool-call jobs, and keeps
  the verdict

**Done when:** an end-to-end test starts a local-model session against the
server with a small model, which calls a tool and replies.

As built: `InferenceServerKit` (apps/libs/kits/inference/server) is the
server; the app serves Apple's model as `apple` and each MLX model
downloaded for Humata, on port 47830 when it's free, and sends the port to
the daemon, which keeps it and forwards it to each host it connects to
(`-R 127.0.0.1:P:127.0.0.1:P`). `humata model-server PORT` does the same by
hand. `//apps/humata/e2e:local_model_server_test` runs the real
`humata-model-server` with a scripted model through humatad: the model
calls `write_file`, the person approves it, and the model replies. Apple's
model was checked by hand to call tools through it
(`humata-model-server --apple`).

### The demo

Once G and H are in, an outside agent, through `humata mcp`, sets up Humata
for the owner to look at: projects for the `uhvesta/atmosphere` and
`uhvesta/statify` repositories added by URL, a few chats with local models
in each, and planning sessions writing Markdown specs, then opens the app on
them.

As run: an outside agent added both projects by URL, then started a Local
Model chat and a spec-writing session in each on Apple's model, served by
`humata-model-server`. Both specs were written. The chats tried `read_file`
on a folder, which local-model tools can't do, and fell back to shell
commands that wait for approval under Allow Edits; that goes to R.

### R Review pass

Once the demo is in:

1. Haiku agents gather the code and every UI and UX flow.
2. Fable 5.2 agents, at high or medium effort, are given the app's goal
   (this spec's opening sections) and what was gathered. They look for jank,
   feature gaps, and what people need to work agentically on many things at
   once with many agents.
3. They check the vision: a Jira-like board to break work down, sessions
   backed by its tasks, and sessions that are ephemeral and cross-machine.
   Then they look for gaps in how it was built.
4. An independent Fable 5.2 agent at xhigh effort reviews every finding and
   keeps only the good ones. The agents decide; the owner doesn't weigh in.
5. The kept findings are built, then pushed.

Only strict improvements count: additions to the UX that simplify without
losing behaviour, or that add behaviour badly needed. No redesigns or
rethinks.

#### What R kept

Two Fable 5.2 finders (one on many agents at once, one on jank) and the
earlier review's list went to an independent Fable 5.2 judge at xhigh. It
agreed with the vision and found the build short in three places:
"ephemeral" was built as "recoverable"; "cross-machine" breaks with the
laptop closed; and the board and notices don't yet say truthfully what is
waiting on you. It kept these, in build order:

- [x] A spec change tells only the sessions it should: only when the text
  changed; running or waiting sessions get the message, idle ones get a
  note with the person's next message, and the editing session nothing
- [x] The open session notifies when Humata isn't in front, and the poll and
  badge live with the app, not the sidebar view
- [x] A slow or dead host never stalls the lists (a short deadline, then the
  cached list); offline sessions draw as last known, with no spinner and
  no Allow that can only fail
- [x] An agent's open question or move request counts as Waiting on You, on
  the board and in the sidebar
- [x] A host is updated only when idle, and only when its build really is old
  (a bundled build of another platform counts as current)
- [x] Agents on a host can ask and request moves while the Mac is away
  (queued on the host, sent on reconnect), and read the live spec when
  the Mac is there
- [x] New Session never sends one agent's model to another, and a Local Model
  session can't start without a model
- [x] Local models can list a folder
- [x] Local-model sessions get Humata's tools, so they can plan and propose
- [x] The inbox separates Blocked on You from To Look Over, rows say what
  happened, the Dock counts only the blocked, reply notices are silent,
  rows sort by longest wait, and a reply that fails to send is kept
- [x] Start Work on a card or task: one click, seeds the first message, stays
  on the board
- [x] Delete an archived session, freeing its folders, refs and copies; the
  Archived list folds away
- [x] Move a session to another machine, both ways, from its menu, with a
  fresh copy, removing the source
- [x] Closing an item offers to stop and archive its sessions
- [x] Delete a task, revertible from Board History
- [x] A session shows its task and opens it
- [x] Listing sessions reads a kept summary instead of copying every history
- [x] The task page's subtasks open, and spec comments go to a live session
- [x] The sidebar holds still while agents work

Dropped by the judge: a driver panic (none reachable), the archive
confirmation and retry, the torn last line (already fixed), held messages
for offline hosts, turn time in the sidebar, bulk actions, card state icons,
a per-host turn limit, PR state on cards, the naming and layout polish, and
Start Work on All.

#### An earlier outside review, as input

A separate read-only review (three Fable 5.2 agents) already reported the
findings below. It agreed with the vision, with two caveats: "a finished
turn means In Review" claims too much once many agents run, and
"ephemeral" was built as "recoverable", so nothing is ever deleted. Its
findings go to step 4's judge with ours. Each is checked against the code
as it is then, since the code may have moved, and is built only if it holds
up.

- **P0 correctness:**
  - saving a spec wakes every session on the item, even with no change;
    only notify when the text changed, and only sessions working or waiting
  - a driver panic leaves its session Working forever; catch it and mark it
    Failed
  - one dead host freezes every list; give each host a short timeout, then
    show the cached list
  - offline sessions look live; draw them stale, "Host offline since …"
  - waiting on you shows as In Review; pending asks and approvals count as
    Waiting on You
  - `ask_person` and `request_move` fail while the Mac is away; record them
    in history, as proposals already are
- **P0 workflow:**
  - Start Work on a card, and Start Work on All, each staying on the board
    and seeding the task as the first message
  - closing an item offers to stop and archive its sessions
  - the inbox split into Blocked on You and To Look Over, with the badge and
    sound only for the first, and one notice for a burst
  - Replied and Failed inbox rows show their text, with Reply, Mark as Read
    and Try Again
  - a real Delete for archived sessions, freeing folders, branches and
    copies
- **P1 data safety:**
  - archive confirms first, listing the ignored and uncommitted files it
    would remove
  - a failed archive can be tried again
  - a torn last line in `events.jsonl` is cut off on open, not fatal
  - the relay asks the daemon for a spec before using its copy
  - moving a session stops the source and allows moving back
  - an app update waits for hosts to be idle
  - the badge loop lives for the app's lifetime, not the sidebar view's
  - offline hosts' sessions can be archived later and messages to them are
    held
- **P1 supervision:**
  - turn time and usage in the sidebar
  - bulk Stop, Archive and approval mode; Allow All; Next Waiting
  - the inbox ordered by priority, then longest wait
  - session state and subtask progress on cards, and a sidebar state filter
  - a per-host limit on turns at once, with queued messages kept across
    restarts
  - pull request state on cards and in the inbox
- **P2 naming and layout:**
  - one word each for Approve/Allow, Delete/Remove and Move To
  - "Humata not installed", with Install, instead of "Unreachable"
  - drop the duplicate title in the session header
  - one shared relative-time formatter
  - Apply is disabled for a proposal with no title
  - spec comments don't default to an archived session
  - subtask rows open
  - an inbox reply that fails to send is kept
  - sidebar names keep their host tag visible
  - a cue that the Done columns are off-screen
  - a single status plus caption on the task page
  - ListSessions keeps a summary instead of cloning histories

**Owner's call:** the session MCP's `update_item` and `add_subtask` applied
directly, without the person's approval. The owner decided agents ask first;
see Milestone J.

### Milestone J: agents ask, and a board journal

The owner, 2026-11-12: agents should ask before they change the board, and
there should be a journal of changes to undo, as in Lifr's Activity log.

- **Agents ask first.** `update_item` and `add_subtask` are gone from the
  session's MCP. A subtask is proposed with `propose_task` and its
  `parent_id`; a new title or spec with `propose_spec_edit`, which takes a
  title too. Finishing a task is `request_move`, as before. Nothing reaches
  the board until the person applies it.
- **Who made each change.** Every board change records its author: you in
  the app, the command line, an MCP client, a session whose proposal or
  request you applied, or Humata itself for status that follows sessions.
- **The journal.** Board History lists each change with its author and,
  field by field, what it was and what it became. It filters by author.
  A change reverted later says so.
- **Undo without overwriting.** Reverting puts back only the fields nothing
  has changed since, and says which it skipped and why. ⌘Z undoes your last
  change to the board, offered in a toast as well.
- [x] Agents propose instead of changing the board
- [x] Each change records who made it
- [x] The journal shows authors and field changes, filtered by author
- [x] Revert skips fields changed since, and says so
- [x] ⌘Z and a toast undo the last board change
- [x] Outside agents propose too: `humata mcp`'s `create_item` and
  `edit_spec` became `propose_task` and `propose_spec_edit`. Their
  proposals wait at the top of the project's board for the person to apply,
  edit or decline, and `humata proposal list --project` lists them. The
  outside agent can still approve agents' requests, but has no tool to
  apply its own proposals.

**Done when:** an agent's subtask and spec edit wait as proposals, the
journal shows them as the session's once applied, and reverting an edit
keeps a later change to another field.

### V Second review pass

The owner, 2026-11-12: run the R process again. Haiku agents gathered the
UI flows and the daemon's behaviour; two Fable 5.2 finders (UX and the
daemon) found 30 items; an independent Fable 5.2 judge at xhigh read the
code for each, kept 27 (most trimmed to their essential part) and dropped
reading GitHub past 100 entries, ETag caching of the follower, and labels
on controls that are off during a turn. In build order:

- [x] A request whose connection drops still finishes on the daemon and hosts
- [x] Clean-up after a merge retries until the host answers, and a reopened
  task stays reopened
- [x] One failing pull request doesn't stop the others being followed
- [x] Landing again doesn't overwrite a pull request's description
- [x] A land that fails after renaming the branch can be tried again
- [x] Spec-change notes are kept until a message carrying them is delivered
- [x] Answering a request the host refuses keeps the request open
- [x] A message to a session answers its open questions, and moving a card
  by hand answers its move requests
- [x] A fork knows which of its parent's proposals were already decided
- [x] Agents can't write the board through the host's relay
- [x] A restore that fails part way leaves no half copy behind
- [x] Deleting a session forgets it before cleaning its mirrors
- [x] A backup that fails backs off and says why in Recovery Copies
- [x] Reverting a task's deletion re-attaches its sessions
- [x] The session shows what waits on you above the composer: its question,
  move request, approvals and pending proposals
- [x] A command waiting for approval shows in full
- [x] Allow, and stop asking for edits (or everything) in this session
- [x] Try Again on a failed turn, in the chat and the Inbox
- [x] The Inbox badge counts what blocks; rows say how long they've waited
- [x] Shortcuts live in the menu bar, with a Go menu: Inbox, Next Waiting
  Session, Search, Hosts
- [x] Files-tab comments and the open tab survive looking elsewhere
- [x] Board action errors stay until dismissed
- [x] Start Work can't start twice, and starting shows progress
- [x] Cancelling a changed task asks first; links are off while editing
- [x] A task's sessions show their state, waiting first, archived last
- [x] A proposed spec edit shows what it changes, and any proposal shows in
  full on request
- [x] Read-only review ranges say where comments go, with a button to get
  there

### P Performance pass

The owner, 2026-11-12: memory, stutters and CPU time are expensive; keep
both the app and the daemon lean. The sidebar refreshes every two seconds,
and the chat redraws on every streamed word, so those paths matter most.

- [x] The chat works out its rows, waiting requests, usage and queue once
  per event, not on every draw
- [x] Typing and streaming redraw only the composer and the turn in
  progress, not the whole chat
- [x] A refresh that changes nothing sets nothing: the sidebar, the open
  session and the board keep their state as it was
- [x] A long diff is built lazily, one line per row, with its threads
  grouped once
- [x] The daemon writes a host's cached sessions only when they changed
- [x] The waiting strip no longer sits beside the bottom-anchored chat,
  which made SwiftUI lay it out without end

### Milestone X: admin mode, gated by approvals

The owner, 2026-11-12: an admin mode, only in dev builds and local testing,
so an agent can drive Humata for the owner, and never in the released app.
All admin access, the command line's included, is gated by approvals: one
at a time, in batches, time-boxed, and for scopes of actions.

- **Admin actions.** The person's own board actions: apply, edit or
  decline a proposal; create, edit or delete an item; edit a spec; revert a
  change. From the app they run at once. From `humata` or `humata mcp` the
  daemon runs one only under an approval, in every build.
- **Who's asking.** The daemon reads the connecting process from the socket
  (its pid, then its executable), so a client can't claim to be the app:
  the app is the executable in `Contents/MacOS` of the bundle `humatad`
  runs from. The approval names the client with its arguments, such as
  `humata mcp`, and its parent process, such as `claude`; the daemon, not
  the client, says whether a change came from MCP. The gate stops agents
  acting as the person without asking; it isn't a wall against code
  already running as the same user, which could drive the app itself.
- **Asking.** An admin action without a grant waits in the inbox as an
  approval request: who asks, and what exactly it will do (the title and
  spec diff of a proposal, the fields of an edit). The command line waits
  for the decision and prints it; `--no-wait` returns the request's id.
  `humata mcp` waits a short while, then answers that it's still waiting
  with the id, which `approval_status` reads later. Approved, the daemon
  runs it and keeps the result for the caller; denied, it says so.
- **Deciding.** The inbox groups the approvals by who asked, above the
  sessions. Approve or deny one; all from one client from its section's
  header; or every one from the toolbar.
- **Grants.** Approving can also grant the scopes of what's approved
  (proposals; items and specs; deletes and reverts, which only a grant
  naming them covers) for 15 minutes, an hour or 8 hours, at most a day,
  to every client. Settings > Approvals lists the grants in force with how
  many actions ran under each, revokes them, and lists what was asked
  lately. Grants live in the daemon's SQLite with their expiry; the last
  200 decided approvals are kept.
- **The journal.** A change made under an approval or grant records
  which let it run; Board History says "approved by you" or "under a
  grant".
- **Dev builds only: the admin tools.** A Bazel flag,
  `--//apps/humata/daemon/cli:admin`, off by default and set by
  `--config=humata-admin`, builds `humata` with its `admin` feature, which
  adds the admin actions to `humata mcp` as tools. Without it they aren't
  in the binary. `release.sh` sets it off explicitly, and the release
  check fails if the downloaded `humata mcp`, or the app's, lists any. The
  end-to-end tests run a `humatad` built with its test-only `e2e` feature,
  which lets them name the program that counts as the app.
- [x] The daemon tells the app from other clients by the socket's peer
- [x] Admin actions from the command line and MCP wait for approval
- [x] Approve and deny one, several, or all from a client
- [x] Time-boxed, scoped grants, listed and revoked in Settings
- [x] The journal records the approval or grant behind a change
- [x] The `admin` flag and its MCP tools, tested end to end
- [x] The release check refuses a build with admin tools (written; it
  runs with the first release)

**Done when:** a dev build's `humata mcp` applies an outside agent's
proposal once approved in the app, a grant lets the next ones run until it
expires, a denied one changes nothing, and a default build's `humata mcp`
lists no admin tool.

### Third review pass: storage, tokens, jank and Markdown

The owner, 2026-11-12: compress what we can on background threads,
delete old things on request and, if opted into, on their own; Mermaid in
Markdown, a focused view of a Markdown file beside its chat, and its
versions. Fable reviewers looked for perf, storage, token and UX problems;
their findings are folded in below. Signed off the same day: deleting an
archived session deletes its snapshots too; auto-delete is off until
turned on, and then deletes archived sessions after 45 days by default.

#### S1 Storage: safe places, cleanup, opt-in auto-delete

- [x] Clones from before they moved, still in `~/Library/Caches/Humata/repositories`
  where a cache cleaner would take their branches and snapshots, move to
  Application Support once their sessions are idle: paths in the store,
  each `session.json`, and the worktrees' links are rewritten
- [x] Settings › Storage shows the space taken by worktrees, clones,
  histories, recovery copies, the search index, models and logs, each
  with Show in Finder
- [x] By hand: remove archived sessions' worktrees (unarchiving makes them
  again), delete archived sessions older than an age, remove clones no
  project uses, delete downloaded models. Each says what it removes and
  how much space first, and that it can't be undone
- [x] Auto-delete, off by default: once on, archived sessions older than
  45 days (30, 45, 90 or 180) are deleted about daily, while nothing on
  this Mac runs
- [x] Never deleted: sessions not archived, keepers, sessions with forks,
  work not pushed or merged, a recovery copy that is the only copy; a
  session on an offline host is deleted once the host is back. What was
  skipped, and why, is shown
- [x] Space really comes back: `git gc` in the repositories after their
  snapshots go, the store vacuumed, mid-turn snapshots thinned (each
  turn's own kept)
- [x] `host.log` rotates; the board journal keeps a bounded history, and
  undo ends at it

#### T1 Tokens and agent tools

- [x] Local model: a message sent while a tool call runs doesn't split the
  call from its result
- [x] Local model: a context budget shrinks old tool output at a stable
  point, and a full context says to fork or start anew
- [x] Local model: `read_file` takes an offset and limit, `edit_file`
  replaces text, shell output keeps its tail, a reply is one message
- [x] Skills listed by name with `read_skill`; specs in messages, so a
  newer one replaces the old; a fork keeps its parent's instructions
- [x] MCP: `read_turn` opens a search hit; search defaults to the session's
  project and says when there's more; "Mac away" errors say not to retry;
  the instructions match the tools; proposals check their ids and take
  replacements; Claude's usage keeps cache reads and writes apart

#### U1 App jank

- [x] A session's history arrives in one batch, with a loading state;
  live events are added, not the chat rebuilt; a reconnect resumes
  without wiping the reply streaming in
- [x] The sidebar and inbox don't redraw every two seconds
- [x] A failing list shows inline, not an alert every two seconds
- [x] Search, boards and hosts keep their state when left
- [x] Answered inbox rows don't come back; reopening the window doesn't
  flash the spinner; search results don't reorder under the pointer;
  review doesn't reload when nothing changed

#### D1 Mermaid diagrams

- [x] A `mermaid` block renders as a diagram wherever Markdown renders,
  from a bundled mermaid.js in a web view with no network, cached by
  source, in light and dark; a broken one shows its source and error
- [x] A diagram opens large in its own window: zoom, pan, fit; drag out a
  region and send it to the chat as an image, with a note
- [x] Code sent to the chat: select lines in a file, a diff or a code
  block and send them as a quote with their path and lines, or with a
  comment, straight into the composer

#### D2 Focus view for Markdown

- [x] A Markdown file or spec opens in a focus view: the rendered document
  wide, the session's chat beside it with its composer; inline comments
  sent into that chat; agent edits reload in place, keeping the scroll,
  with what changed marked

#### D3 Versions of a Markdown file

- [x] Its versions, from the session's turn snapshots and the spec
  journal, listed in the focus view; any one rendered, or compared with
  now as a rendered diff; restoring one writes it as a new change

#### U2 UX papercuts

- [x] Return doesn't approve; the composer is focused when a session opens
- [x] Fork, edit to a fork, update and answering show progress and can't
  be done twice
- [x] Each alert says what failed; search has Try Again; one word for
  approving, one style for times; a permission shows once
- [x] Strings in added lines aren't red; Hosts shows no backticks; a
  Session menu; VoiceOver reads diff lines

#### S2 Compression

- [x] Histories of archived or long-idle sessions sealed as zstd on a
  background thread; the live tail stays plain
- [x] Recovery copies take only the history that's new each turn, kept
  as added parts; a rewind sends it whole again
- [x] A backend's own state (Claude Code's transcript) sent only when it
  changed, its files kept apart by hash
- [x] A remote session's text kept once on the Mac, not as both its
  offline copy and its recovery copy
- [x] Compressing and decompressing never run on the executor or the main
  thread. While a sealed history is being opened, the session shows a
  progress bar (bytes read of its size), or a spinner where the size
  isn't known; Settings › Storage shows sealing and cleanup progress the
  same way

#### S3 Daemon speed

- [x] The app's three two-second lists share one listing; remote rows are
  written only when they change
- [x] SQLite off the executor threads on the two-second list and the
  search catch-up; search catch-up writes only what changed; specs and
  comments indexed apart
- [x] No whole-history copies on hot paths; archived sessions loaded when
  opened

### A terminal on Ghostty

The owner, 2026-11-13: terminals drawn by GhosttyKit, Ghostty's own
embedding library, pinned to one upstream commit and built from source
with the repo's Zig under Bazel, the way boss pins it (boss's Apache-2.0
notice kept where its build is adapted; nothing from cmux, which is GPL).
Pinned at v1.4.0 rather than boss's commit, which needs Zig 0.16 where the
repo has 0.17. The surface runs the bundled `humata terminal attach`,
which pipes the host's terminal through humatad's socket, so the daemon
and wire don't change and `TERM` stays `xterm-256color` on hosts. Ghostty
reads only the config Humata writes from Settings, never
`~/.config/ghostty`.

#### G1 Ghostty terminal

- [x] GhosttyKit v1.4.0 built from its release tarball and pinned
  dependencies by Bazel with the repo's Zig; Ghostty's MIT and boss's
  Apache-2.0 notices are listed
- [x] `humata terminal attach <terminal> [--host <host>]` pipes a terminal
  both ways: keys in order, replay on attach clearing what was drawn, the
  pty's size sent on change, the shell's exit code as its own, and
  reconnecting after a dropped connection
- [ ] Terminals look and behave as before: the same tabs, the exit and
  closed-elsewhere notes, and Reattach when the pipe ended otherwise
- [ ] Drawn by Ghostty with Metal: true color, wide and combining
  characters and emoji, complex scripts, and no stutter while a command
  prints fast
- [ ] Keys as Ghostty sends them, Option as Meta, and input methods for
  composed text; Humata keeps its own ⌘ shortcuts
- [ ] Select with the mouse, copy with ⌘C, paste with ⌘V (bracketed when
  the program asks), and mouse reporting for programs like vim and htop
- [ ] Scroll back with the wheel or trackpad; ⌘F finds text in it
- [x] Settings › Terminal: the font and size, and a color theme with light
  and dark palettes; Ghostty's own config files are never read
- [x] VoiceOver reads the screen
- [x] SwiftTerm removed

### Composer, rewind, skills, plugins and models

The owner, 2026-11-12, signed off the same day: queued messages can be
edited, and a message sent while one waits joins it; long pastes and images
in messages; rewind the chat, or the chat and the files, undoably, with a
checkpoint before each compaction as Copilot's CLI makes; skills from
`~/.claude` and `~/.copilot` with clashes picked, each pick remembered
globally and editable in Settings; plugins listed and toggled; default
models, Copilot's experimental mode and Fusion; on-device models only on
the Mac. Built after S1, before T1.

#### Q1 Queue and composer

- [x] A message sent while one waits joins the queued one (a blank line
  between), so a turn takes them together; Steer Now still steers
- [x] A queued message can be edited in place until its turn starts; an
  edit that loses the race to the turn says so and puts the text back
- [x] Cancel: a queued message is taken out; a running one is stopped and
  its text goes back in the box; one stopped before the agent read it
  leaves no trace in the history
- [x] A paste over 1,000 characters or 20 lines shows as a chip, "Pasted
  12 KB, 340 lines", that opens to read, edit or remove; it's sent as
  the text
- [x] Images pasted, dropped or attached go with a message, as thumbnails
  in the box and the chat: to Claude Code and Copilot as images, and to a
  local model that can see them; one that can't says so before sending

#### Q2 Rewind and checkpoints

- [x] Rewind to before any message the person sent: the chat only, or the
  chat and the files, back to that turn's snapshot; the message goes back
  in the box
- [x] Undo Rewind brings back what was cut, chat and files, until the next
  message is sent; after that the cut part stays as a hidden fork, kept
  until the session is deleted
- [x] Every compaction, Copilot's, Claude Code's or the local model's,
  makes a checkpoint first, shown in the chat; rewinding to it brings
  back the full context from before it
- [x] Rewind runs on every backend: Claude Code resumes at the turn,
  Copilot forks at the event, the local model cuts its history; a session
  working is stopped first, after asking
- The local model never compacts, so it makes no compaction checkpoint;
  Claude Code's and Copilot's are saved when their compaction is reported,
  as nothing changes the files while they compact

#### Q3 Skills from the agents' folders

- [x] Skills from `~/.claude/skills`, `~/.copilot/skills` and each
  repository's `.claude/skills` and `.github/skills`, on each host, listed
  in Settings › Skills beside the project's own, and given to every
  backend
- [x] Two with one name: a new session can't start until the person picks
  which wins; the pick is remembered for every project and host, shown
  and changed in Settings › Skills, and asked again only when either
  file changes

To save tokens, a session's instructions list each skill from a folder by
its name, description and path, for the agent to read when it needs it;
only the project's own are written out in full. Two copies with the same
text aren't a clash. A host's skills are read from its clone of each
repository, not the session's copy, and Settings lists a host that
doesn't answer in 10 seconds as unreachable. A backend that loads a
folder's skills itself still sees one that lost a pick; its instructions
say to use the listed one.

#### Q4 Plugins

- [x] The Claude Code and Copilot plugins installed on each host, listed
  with what they add (skills, commands, MCP servers, hooks), and turned on
  or off per project; installing stays in each CLI

Settings › Plugins lists them by host and agent, from each CLI's own record
(`~/.claude/plugins/installed_plugins.json`, `~/.copilot/config.json`), with
whether the CLI has each on, and holds a project's choices: as the CLI has
it, on, or off, for that project's sessions on every host. `humata plugins`
and `humata plugin` do the same. Nothing is written to the CLIs' own
settings: Claude Code gets the choices with `--settings` on each turn;
Copilot reads them from `.github/copilot/settings.local.json` in each of the
session's copies of the repositories, which Humata writes before the
session opens and keeps out of git through the repository's
`info/exclude`. So a Copilot chat in a folder, with no copies, gets no
choices; and whether Copilot applies that file in a session isn't
confirmed, as its `plugin list` doesn't show it. A choice changed while a
session is open reaches it when it next opens, and a fork that shares its
parent's copies shares its parent's file.

#### Q5 Models

- [x] A default backend, model and effort, for all projects and per project,
  that New Session starts from; the last choice no longer overrides it
- [x] Copilot's experimental mode, a switch per project and per session
  (`--experimental`), off by default
- [x] Copilot Fusion in the model picker, with its tier (standard, high,
  max); its cost per turn shown as for any model; local models in Fusion
  need experimental mode, and say so
- [x] On-device models run only on the Mac: a session on a host uses the
  Mac's model server through the forward, and a `base_url` that points at
  the host itself is refused for Humata's on-device models

Settings › Models holds every project's defaults and, over them, a
project's own: the agent, the model, its effort, and for Copilot its
experimental features. New Session and Start Work start from them; the
approval mode, host and model server still come from the last session.
The daemon keeps them, but only the app applies them, so `humata start`
and the MCP tools still take what they're given (`--effort`,
`--experimental`). Each model says the efforts it takes: Claude Code's
from low to max, none for Haiku; Copilot's from its `models.list`. Copilot
Fusion's tier is its effort, written as `fusionTier` into the same
`.github/copilot/settings.local.json` as the plugin choices, so like them it
reaches only sessions with copies of the repositories. Fusion is added to
Copilot's list when the CLI doesn't list it, so it shows before it's
confirmed there. Copilot runs as two CLIs, with and without
`--experimental`; a session switched mid-way opens again on the other at
its next message. A host session whose model server is at the host's own
loopback address, on a port other than the forward's, is refused when its
model is one of the Mac's own, since that server isn't the Mac's.

#### Fixes from the owner, 2026-11-12

- [x] The session header gives up the usage, then its actions' names, then
  the tabs' row as the window narrows, so nothing is cut off
- [x] A permission request still open when its turn ends shows as not
  answered, with no Approve or Deny, and nothing waits on it
- [x] Opening a chat shows its history at once: a watch's events are taken
  in batches and the chat worked out once per batch, not once per event
- [x] Releases are built optimized (`--compilation_mode=opt`); 0.1.0 wasn't

#### Q6 Auto approvals

The owner, 2026-11-12: auto approvals through the agents' own judges.

- [x] Approvals gains Auto: Claude Code turns run in its `auto` permission
  mode, and a change mid-turn reaches the running CLI as a
  `set_permission_mode` control request (`manual` for the other choices);
  Copilot sessions run in its `assisted` mode, set mid-turn with
  `session.permissions.setMode`, and Humata lets through what its judge
  recommends approving
- [x] What the judge won't approve comes to the person as any request
  does; what it approved shows "Allowed by the agent's judge". Claude
  Code's classifier runs what it clears without asking and refuses what it
  blocks; after three refusals in a row or twenty in all it asks, and that
  comes to the person
- [x] Auto isn't offered for local models, which have no judge; picking a
  local model in a new chat drops it to Ask Every Time

#### Q7 Spec tab

The owner, 2026-11-12: see the specs for a chat.

- [x] A Spec tab beside Chat, Review and Files shows every spec the chat
  reads, in order: the project's instructions, its task's, its subtask's
  (as the daemon gives them, the task's always with a subtask's)
- [x] Each renders as Markdown, with Edit for Write and Preview like the
  board's editor; an item's edit is saved to the board and reaches the
  session as any spec edit does; the project's reaches sessions started
  after, as in Project Settings. Open on the Board shows the item there

### Milestone K: signed releases

The owner, 2026-11-12: release signed apps and binaries on GitHub, and
test that they work. GitHub access reuses the GitHub command line's login,
not an OAuth app.

- **GitHub through `gh`.** Without `GH_TOKEN` or `GITHUB_TOKEN` in This
  Mac's variables, the daemon asks `gh auth token` each time it needs
  GitHub, and keeps the token only in memory.
- **One command releases.** `apps/humata/tools/release/release.sh
  <version>` builds Humata from a pushed commit, signs the app,
  `humata` and `humatad` with the Developer ID certificate and the hardened
  runtime, notarizes with the App Store Connect key the TestFlight uploads
  keep in the Keychain, and staples. It uploads, as a draft release tagged
  `humata-v<version>`: `Humata-<version>.zip`, `humata` and `humatad` for
  the Mac, `humatad` for Linux arm64 and x86_64, and `SHA256SUMS`.
- **Checked as people get it.** Before the draft is published, the release
  is downloaded again into an empty folder and marked as downloaded
  (quarantined), then: Gatekeeper accepts the app and the Mac binaries as
  notarized Developer ID code, the ticket is stapled, every file matches
  `SHA256SUMS`, and the downloaded `humatad` starts a daemon that the
  downloaded `humata` talks to. Only then is it published.
  `apps/humata/tools/release/check.sh <tag>` runs the same check on any
  release later.
- [x] The daemon uses `gh`'s token when This Mac has none, tested against
  stand-in `gh` scripts: signed in, signed out, printing nothing, missing
- [x] The release command signs, notarizes and uploads a draft; it
  shipped humata-v0.1.0
- [x] The check downloads, verifies and runs the release before publishing;
  humata-v0.1.0 passed it

**Done when:** a release is on GitHub whose download Gatekeeper accepts
and whose daemon and command line run.

### Humata 2: the app's logic in Rust (stopped)

The owner, 2026-11-13: most of Humata in Rust, with Swift laying out
screens and sending events to Rust. Humata 1 keeps shipping until Humata 2
is ready to replace it; code duplicated between them meanwhile is fine.

Stopped by the owner the same day, after H2-1: rewriting the app in Rust
isn't worth it; the effort goes into making Humata 1 fast (P2). What was
built stays in `apps/humata2` and `libs/tca-rust` for now.

- **TR tca-rust.** `libs/tca-rust` (the `tca-rust` crate, library `tca`)
  takes the ideas of The Composable Architecture, not its Swift: reducers
  that return effects, cancelling by id, `for_each` and `if_let` for
  children, `IdentifiedVec` with the changes between two versions, a
  `Store` that reduces in batches and tells observers once per batch, and
  an exhaustive `TestStore` on a `TestClock`. It's not tied to Humata, so
  it can be published; publishing waits on the owner.
- **H2-0 Skeleton.** `apps/humata2/core`, the features' state and reducers
  on tca-rust, talks to `humatad` with the daemon's `wire` crate. UniFFI
  bindings built in Bazel; `apps/humata2/app` is a Swift app with its own
  bundle id that lists sessions from the core's store.
- **H2-1 Chat.** A session's chat in Humata 2: the history read with
  its progress, then live events; tools, permissions, queued messages,
  Stop, and a message that failed to send put back in the field. The
  core tells Swift only what changed since it last told: rows that came,
  went or changed, the reply's new text, and the status when it changed.
  Measured on a fastbuild through the real bindings with a stand-in daemon
  (`apps/humata2/rehearsal`, the test `//apps/humata2/app:tests`): 5,000
  rows reach Swift in 47 ms, and a reply of 20,000 parts in 120 ms, 6 µs a
  part, with no gap over 13 ms while it grows. Each update still compares
  every row's pointer, so TR2 looks at that first. Answering permissions,
  markdown, images and rewind come with H2-5.
- **H2-2** projects and the board; **H2-3** review, docs and specs;
  **H2-4** hosts, the terminal and the browser; **H2-5** the rest. Then the
  swap: Humata 2 ships as Humata.
- **TR2 tca-rust speed.** Once Humata 2 runs: benchmarks first, then make
  the store, effects and `IdentifiedVec` as fast as Rust allows, past big O:
  allocations, copies, locks and wakeups.

## Later

- **Language servers** in the review view and the file view: go to definition,
  references and previews in the style of Neovim, as an option.
- **An iOS companion** for the inbox and approvals.
- **Teams:** shared projects, assignees who are people, permissions.
- **Moving the board to another Mac** by exporting and importing the store.

## Risks

| Risk | Recommendation | Settle by |
| --- | --- | --- |
| Sessions that share a working copy can collide. | One turn at a time in a shared copy; queue the rest. | Settled in 2 |
| A host killed mid-turn leaves a half-written turn. | A save after each tool call that succeeds, bundled to the Mac, and a cut turn is marked aborted on restart. A file written outside a tool call (a shell command's side effect still running) can still be lost. | D |
| A first backup of a repository with a long history may not fit in one 64 MiB frame. | Split bundles across frames, or send the first one by `git fetch` over SSH. | D, if it happens |
| The recovery copy grows without bound. | Keep every snapshot ref (they're deltas), compress backend state, and prune archived sessions' copies after a set time. | D |
| A backend's state may not resume on another machine (paths inside transcripts). | Restore into the same paths under the cache folder; where a backend refuses, the turn fails and says so. Never fall back to sending the history as a message on its own: a restart or move would then resend a whole conversation's tokens unasked. If that's ever built, it waits for the person to confirm, showing the size. | D |
| A PTY over the protocol may be slow over SSH. | Send output in batches with the fast zstd level, and limit replay to the last 256 KiB. | E |
| Terminals on hosts can run anything as you. | They are the same as an SSH session you open yourself; nothing new is reachable, and nothing listens on a port. Port forwarding binds to loopback on the Mac only. | E |
| Remote agents need the board while the Mac is asleep. | Copy specs to the host when a session starts and when they change; hold proposals there until the Mac reconnects. | 6 |
| The Copilot and Claude CLIs change quickly. | Pin versions, keep them behind the agent interface, and show the version in the host's health. | Ongoing |

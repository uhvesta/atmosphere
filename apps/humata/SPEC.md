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
| D Durable hosts | In progress |
| E Terminals and files | After D |
| 6 Plan with the agent | After E |
| 7 Search across sessions | After 6 |
| 8 Ship as pull requests | After 7 |

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
- [ ] Inline comments on a task's spec → done through doc mode (A) for chat files; for specs, moved to Milestone 6

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
- [ ] Language servers per host → later

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
  under its state folder: the full event history, each turn's snapshot refs
  fetched into a bare mirror per repository (`git fetch` over the same SSH),
  and the backend's own state (Claude's transcript, Copilot's session folder),
  copied after each turn with zstd.
- **Restoring.**
  - *The machine comes back:* sessions reopen; a turn that was cut off is
    marked aborted, so the session takes messages again.
  - *The machine is gone:* "Restore session" installs the host program, runs
    the host's setup again, clones the project's repositories, pushes the
    snapshot refs from the mirror, recreates the worktrees at the last
    snapshot, and puts back the history and the backend's state, so the agent
    resumes with its context.
  - *Restoring on another host* is the same steps against a different machine,
    which makes it a move. The session's host changes; its id, tree and item
    stay.
- **Mid-turn snapshots.** A snapshot after each write tool, not only at the end
  of the turn, so a machine lost mid-turn loses at most one tool call.

Work:

- [ ] `humatad host` as a detached process with its socket, and `humatad relay` used by SSH
- [ ] Reconnect re-watches from the last event, and a cut turn is marked aborted on restart
- [ ] Snapshot refs fetched into a mirror on the Mac after each turn
- [ ] Backend state (Claude transcript, Copilot session folder) copied to the Mac after each turn
- [ ] Restore a session onto a set-up host: clone, push refs, worktrees, history, backend state
- [ ] Restore a whole host: every session it had, after reinstalling and rerunning setup
- [ ] Restore onto another host as a move
- [ ] Mid-turn snapshots after each write tool
- [ ] The Hosts view shows each host's sessions' recovery state and a Restore button

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
  it with a terminal view in a session tab and in a Terminals window per host.
- **Files.** A tree of a session's folder, or any folder on a host, read
  through the protocol (`ListFiles` and `ReadFile` already exist for chat
  files; they grow paths outside the session and lazy directory listing).
  Files open read-only with syntax highlighting, Markdown rendered, images
  shown, and large files cut with a note. Comments use the review threads.

Work:

- [ ] PTY in the host role, with input, resize, output stream, and replay on reattach
- [ ] Terminal tab in a session, opened in its folder; ⌘T for a new one
- [ ] Terminals for a host, outside any session, from the Hosts view
- [ ] Terminals keep running when the app closes, and are listed so they can be closed
- [ ] File browser for a session's folder and for any folder on a host
- [ ] Read-only file view with highlighting, rendered Markdown and images
- [ ] Open a file in an editor on this Mac (local) or copy its remote path
- [ ] Port forwarding for a dev server on a host, opened in the browser

**Done when:** an end-to-end test opens a terminal on an SSH host, runs a
command and reads its output, disconnects and reattaches and still sees it;
a flow snapshots the terminal and the file browser.

### Milestone 6: plan with the agent

**Outcome:** a rough idea becomes a specced project with tasks in one
conversation, and the agents doing the work always read the current spec.

- [ ] Give the agent tools to read the board, read a spec, and propose tasks, subtasks and spec edits
- [ ] Show each proposal as a card in the chat to apply, edit or decline, with nothing saved until you apply it
- [ ] Start planning sessions without a checkout, so they open at once
- [ ] Copy a session's specs to its host and hold the agent's proposals there, so sessions keep working while the app is away
- [ ] Tell running sessions when a spec they depend on changes, and have them read the new one
- [ ] Fork a planning session into a working session on a task it created
- [ ] Pass each project's instructions and skills to every session in it
- [ ] Comment inline on a task's spec, as in doc mode, and send the comments to a session

**Done when:** an end-to-end test has a scripted agent propose three tasks
with specs, applies two, and finds exactly those two on the board.

### Milestone 7: search across sessions

**Outcome:** anything an agent said or did can be found again, by you or by
another agent.

- [ ] Index messages, commands and their output, changes, review comments and specs on the Mac, from every host's history (SQLite FTS)
- [ ] Search by words and by time, such as "last week", and filter by project, task or session
- [ ] Related words from a cheap local model, with a word list as the fallback, the way Lifr's search does
- [ ] Open a result at the matching point in its session, and fork from there
- [ ] Give the agent a tool to search past sessions
- [ ] Search every machine's sessions in one query, including offline ones

**Done when:** an end-to-end test finds a command run in one session from a
search started in another project, and opens it at that turn.

### Milestone 8: ship as pull requests

**Outcome:** a finished session becomes a merged pull request without leaving
the app, and the board closes the task by itself.

- [ ] Land a session's work: commit its changes on a branch of your naming, with a message you edit
- [ ] Push the branch and open a pull request, one per repository, linked to each other and to the task
- [ ] Show each pull request's review and CI status on the task, and move the task to Done when it merges
- [ ] Bring GitHub review comments into the inline threads
- [ ] Update a session from its base branch and hand conflicts to the agent
- [ ] Import GitHub issues as tasks
- [ ] Remove a session's worktrees and branches after its pull request merges

**Done when:** an end-to-end test opens a pull request against a local git
server standing in for GitHub, marks it merged, and finds the task Done and
the worktree gone.

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
| A host killed mid-turn leaves a half-written turn. | Mid-turn snapshots after each write tool, and a cut turn is marked aborted on restart. | D |
| The recovery copy grows without bound. | Keep every snapshot ref (they're deltas), compress backend state, and prune archived sessions' copies after a set time. | D |
| A backend's state may not resume on another machine (paths inside transcripts). | Restore into the same paths under the cache folder; where a backend refuses, resume with the history summarized as the first message and say so. | D |
| A PTY over the protocol may be slow over SSH. | Send output in batches with the fast zstd level, and limit replay to the last 256 KiB. | E |
| Terminals on hosts can run anything as you. | They are the same as an SSH session you open yourself; nothing new is reachable, and nothing listens on a port. Port forwarding binds to loopback on the Mac only. | E |
| Remote agents need the board while the Mac is asleep. | Copy specs to the host when a session starts and when they change; hold proposals there until the Mac reconnects. | 6 |
| The Copilot and Claude CLIs change quickly. | Pin versions, keep them behind the agent interface, and show the version in the host's health. | Ongoing |

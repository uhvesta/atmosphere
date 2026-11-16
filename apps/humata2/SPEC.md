# Humata 2: behavioural spec

As of Nov 15, 2026. This is the behavioural spec for Humata 2. It records the
decisions the owner approved ([DECISIONS.md](DECISIONS.md)) and what the design
goldens show. The earlier clickable prototype is no longer a reference (§28).

What wins, in order:

1. The owner's decisions, newest first.
2. The design goldens: the frames in `desktop/tests/__snapshots__/design/`
   (D76, D100, OVERVIEW › Design goldens). A frame settles how a screen looks
   and what it offers: its layout, controls and wording. Build every screen to
   match its frame.
3. This spec. It says what the things a frame draws do.

Where a frame draws something this spec does not define, the spec is changed
to define it. Where a frame contradicts a newer owner decision, the spec
follows the decision and the frame is redrawn with the owner's OK; until then
the requirement names the frame.

Humata 2 is a native app for directing coding agents: Claude Code, Codex CLI,
Copilot CLI, OpenCode, and plain shells. The UI is written in Rust on GPUI, with
its state in the repo's `libs/tca-rust`. It runs on macOS, Linux and in a
browser; no other platforms. A Rust
daemon, `humatad`, runs on every machine Humata uses: the computer you work on
and any machine you reach over SSH. A session is the agent CLI's own terminal,
held by `humatad`, so it outlives the app, the SSH link and the daemon itself.

This spec says `humatad` for the daemon. Until Humata 1 is retired its binary
is `humatad2`, its holder `humata2_holder` and its folder `~/.humata2` (D37,
TS-0, TS-1).

It is for one developer running many agents across several repositories and
several machines.

Requirement IDs (`CH-3`, `PER-7`, …) are stable: new requirements get new
numbers and removed ones are not reused. Each requirement is a statement a test
can check. A requirement tagged "(provisional, D<n>)" follows a pick in
[DECISIONS.md](DECISIONS.md) that the owner has not confirmed yet.

## Contents

1. [Concepts](#1-concepts)
2. [Architecture](#2-architecture)
3. [Channels and machines](#3-channels-and-machines)
4. [Folders and clones](#4-folders-and-clones)
5. [Sessions and terminals](#5-sessions-and-terminals)
6. [Forks](#6-forks)
7. [Moving and archiving](#7-moving-and-archiving)
8. [Lifecycle and disk](#8-lifecycle-and-disk)
9. [Review across N repos: diff, files, editor](#9-review-across-n-repos-diff-files-editor)
10. [PR reviews](#10-pr-reviews)
11. [Markdown review and comments](#11-markdown-review-and-comments)
12. [File manager and transfers](#12-file-manager-and-transfers)
13. [Boards](#13-boards)
14. [Inbox and agent access](#14-inbox-and-agent-access)
15. [Agent-prepared views and the Journal](#15-agent-prepared-views-and-the-journal)
16. [Agents' view of the topology (MCP tools)](#16-agents-view-of-the-topology-mcp-tools)
17. [Hosts and the bridge](#17-hosts-and-the-bridge)
18. [Machine providers](#18-machine-providers)
19. [Persistence and resume](#19-persistence-and-resume)
20. [Async: working while the host is away](#20-async-working-while-the-host-is-away)
21. [Transport: gRPC streaming](#21-transport-grpc-streaming)
22. [Navigation and UI principles](#22-navigation-and-ui-principles)
23. [Home, global Inbox, Global board, palette](#23-home-global-inbox-global-board-palette)
24. [Settings](#24-settings)
25. [Security and secrets](#25-security-and-secrets)
26. [UX flows](#26-ux-flows)
27. [Non-goals](#27-non-goals)
28. [The prototype](#28-the-prototype)
29. [Open questions](#29-open-questions)

## 1. Concepts

| Concept | What it is |
| --- | --- |
| Channel | A project. It has a name, a set of GitHub repos, a set of machines, a board, an Inbox, agent access rules and an activity feed. Shown like a chat channel (`#payments-rewrite`) but it holds sessions, not messages. |
| Machine | The computer the desktop app runs on (labelled **This Mac** on a Mac), or any machine reached over SSH that runs `humatad`, whether the user owns it or a provider made it (§18). A channel can use any mix of machines and can be all-remote. The **Hosts** page and sidebar section list machines; the name comes from the goldens and does not mean the Host below. |
| Main clone | One clone per repo per machine of a channel, on the default branch. A read-only reference that Humata fetches on its own. |
| Session clone | A session's own clone of a repo, on its own branch, named from the work (FC-11). |
| Shared folder | `~/Humata/<channel>/shared/` on each machine: files anyone in the channel shares. |
| Session | One piece of work on one machine, in one folder, with 0, 1 or more repos: its main agent's CLI (or a plain shell) in a terminal, any shell tabs, and at most one more terminal per other agent (SE-28). |
| Tab group | A session's tab row, or one of its two halves when it is split (SE-27). |
| Desk | The place for sessions that belong to no channel (DK-1): an agent or a shell in a scratch folder or a folder the user picks. |
| Holder | A tiny per-session process on the session's machine that owns its PTY and screen state. |
| PR review | A session in the PR reviews channel that reviews one or more pull requests; its comments are drafts posted to GitHub only when you submit. |
| Markdown review | Reading a Markdown file in a tab as Preview, Source or Side by side, with comments on its source lines (§11). It works in any session; there is no separate spec mode (D104). |
| Archived session | A stopped session whose clones are deleted; conversation, record, scrollback and diff summary are kept and it can be restored. |
| Fork | A new session made from another one. Forks form a tree of any depth. Three kinds: code + context, code only, context only. |
| Board | Columns of items: Backlog, To do, In progress, In review, Done, also shown as a table or a dashboard (BD-11). There is a Global board (the sidebar's **Board**), a board per channel and a board per session. |
| Inbox | Requests from agents for actions they don't own. Seen globally, per channel and per session. |
| Agent access rule | Per channel, per kind of action: Allow, Ask me, or Never. |
| Prepared view | A review walkthrough or board view an agent prepared for you over MCP. Opened only if you choose to. |
| Journal | Per session (with its forks): every view its agents prepared. |
| Machine journal | A machine's own append-only list of writes and requests made while the host is away (SL-5). Not the Journal of views. |
| Host | The machine whose `humatad` holds the source of truth (SL-1): your Mac by default, or a machine you host (AR-7). The worked examples in §26 use a Mac as the host; elsewhere the spec says "host". "Away" means a machine has no link to it. |
| Bridge | The host. Every machine has one SSH link to it, and every call between machines goes through it. |
| Turn | From a message reaching the agent until the agent waits for input again. How Humata learns a turn ended is per agent (SE-21). |
| Own clones | The clones under a session's own folder (`sessions/<session>/`). A context-only fork has none until it gets its own copy (FK-9). |
| Grant | An "Always allow …" permission for one session and one target (IN-14, IN-22). |
| Presence | Proof that the person, not an agent, is giving a command from the CLI (SEC-10). |
| Inactive | A session with no terminal output, no input and not opened by the user for a given time (LD-16). Not the Idle status. |
| Revision | A counter every editable thing carries; it goes up on each change (SL-7). |

Session statuses, shown as a dot everywhere a session appears:

| Status | Label | Mark (as the frames draw it) |
| --- | --- | --- |
| working | Working | green dot |
| needs | Needs you | orange dot |
| idle | Idle | hollow grey ring |
| probably idle | Probably idle (SE-22) | hollow grey ring, half-filled |
| exited | Agent exited (resumable) | red ✕ |
| offline | <machine> is offline (its link is down; Humata is reconnecting) | grey dot |

This is the only table of status marks. The words appear in a header only
when the session needs attention (SE-6, NV-32).

Built-in agents (Claude, Codex and Copilot are the defaults; users can add
more, ST-2):

| Agent | Command | Context forks | Turn end (SE-21) |
| --- | --- | --- | --- |
| Claude Code | `claude` | yes | the agent's hooks |
| Codex CLI | `codex` | yes | output fallback |
| Copilot CLI | `copilot` | yes | the agent's hooks |
| OpenCode | `opencode` | no | output fallback |
| Terminal (a plain shell) | `zsh` (the login shell) | no | none: no turns |

The app labels a plain shell **Terminal** (frames 31, 40). This spec says
plain shell where it means the same thing.

## 2. Architecture

```text
 Your Mac (the host by default)
 ┌──────────────────────────────────────────────────────────────────────────┐
 │  Humata app (Rust, GPUI + tca-rust)                                      │
 │     │ gRPC over a local unix socket                                      │
 │     ▼                                                                    │
 │  humatad (host role): channels, boards, Inbox, access rules, journal,    │
 │     │                 the hosts list, the bridge, GitHub token (gh)      │
 │     ├── holder ── PTY ── claude …     (one holder per local session)     │
 │     ├── holder ── PTY ── zsh                                             │
 │     └── humatad mcp  ◀── agents on this Mac call Humata's MCP tools      │
 └──────┬───────────────────────────────────────────┬───────────────────────┘
        │ SSH: one connection per machine           │ SSH
        │ gRPC multiplexed over stdio / unix socket │
        ▼                                           ▼
 ┌────────────────────────────────┐        ┌────────────────────────────────┐
 │ devbox-1 (Linux)               │        │ gpu-box (Linux)                │
 │ humatad (machine role)         │        │ humatad (machine role)         │
 │  ├─ holder ─ PTY ─ codex       │        │  ├─ holder ─ PTY ─ claude      │
 │  ├─ holder ─ PTY ─ claude      │        │  └─ own inbox while host away  │
 │  ├─ ~/Humata/<channel>/…       │        │                                │
 │  └─ humatad mcp ◀─ agents      │        │                                │
 └────────────────────────────────┘        └────────────────────────────────┘
      devbox-1 never talks to gpu-box directly: devbox-1 ─▶ host ─▶ gpu-box
```

- **AR-1** Every machine Humata uses runs one `humatad`. The host's daemon
  (your Mac by default, or a Linux machine, AR-7) is the source of truth for everything that can be centralized (SL-1); other
  machines keep only clones and live agent context (SL-2).
- **AR-2** The app, the `humata` command-line tool and the MCP server are all
  clients of the same gRPC API.
- **AR-3** The app holds no session state that matters: quitting it changes
  nothing on any machine. View state the user expects back is kept by the
  host instead (AR-5).
- **AR-4** Remote machines reach each other only through the host (for now).
  Every cross-machine call is checked on the host against the channel's agent
  access rules. A call that stays on one machine is checked by that machine
  against its latest copy of the host's rules (AW-8). These checks cover
  calls made through Humata only (SEC-5).
- **AR-5** View state is stored on the host, not in the app: open panels and
  tabs, tab groups, folds, scroll positions and filters (SE-13), column
  widths and collapsed columns (NV-16), expand and collapse results (NV-19)
  and the pinned order (NV-25). After the app is quit, crashes or is reinstalled,
  each of these comes back as it was.
- **AR-6** The app is only a client. Any number of clients can be connected to
  one host at once: desktop apps, browsers (AR-8) and the CLI. Each sees the
  others' changes as they happen, and none of them owns state. Durable view
  state is kept per kind of client (desktop or web), and one client can copy
  another's. Hover, selection and unsent drafts stay in the client. (D34)
- **AR-7** The host can run anywhere. By default it is your Mac, but you can
  host it yourself on a server or VM, and every client then connects to that
  host: desktop apps and the CLI over SSH, browsers over HTTPS. Everything else
  is unchanged. Machines link to the host, the host checks every cross-machine
  call, and the host holds the source of truth. Moving the host to another
  machine carries its state across, and nothing on the machines is lost.
- **AR-8** The same app runs in a browser, built for the web and served by the
  host. Serving it is off until the user turns it on. The host refuses
  requests from other sites (Origin and Host checks).
- **AR-9** A new client pairs once before it can do anything. The user
  approves a short code from a client that is already paired, or from
  `humata login`. A paired client acts as the user, present, like the desktop
  app (BOUNDARIES §2.1). Pairings are listed in Settings, every one is
  recorded, and each can be revoked, taking effect on the next call. Nothing an
  agent can do creates or widens a pairing.

## 3. Channels and machines

- **CH-1** The user can create a channel with a name, a set of GitHub repos
  (zero or more) and a set of machines (one or more, any mix of This Mac and
  SSH machines). A channel that leaves out This Mac is valid.
- **CH-2** The folder name is derived from the channel name (lowercase,
  non-alphanumerics become `-`) and shown live while typing. The user never
  picks a path.
- **CH-3** The repo picker lists repos from the `gh` CLI on the host, with fuzzy
  search and checkboxes; repos already in the channel are left out. The create
  button says how many repos it will clone ("Create channel and clone 2
  repos").
- **CH-4** Offline machines are shown in machine pickers but cannot be chosen.
- **CH-5** When a channel is created, Humata creates its folders and main clones
  on every chosen machine and shows progress until the clones finish, then
  opens the channel.
- **CH-6** The user can add a repo to an existing channel from the Repos and
  machines tab (CH-18, CH-19).
- **CH-7** The user can add a machine to a channel. Humata creates the
  channel's folders and main clones on it.
- **CH-8** When the user removes a machine from a channel that has sessions on
  it, Humata asks what to do with each of those sessions: move it to another
  machine of the channel, or archive it. The machine is removed only after
  every session is handled.
- **CH-9** The Repos and machines tab shows a grid: one row per repo, one
  column per machine, each cell the number of session clones of that repo on
  that machine (a cell opens the sessions it counts). Each column heading
  shows the machine and its session count and opens its page; offline
  machines are marked offline. It leads to the two actions above ("Add a
  GitHub repo…", "Add a machine…") and to removing a repo or machine
  (CH-8). It shows no clone health, "behind main" or similar state.

```text
┌ Repos and machines ───────────────────────────────────────────────────────┐
│                     This Mac · 6     devbox-1 · 3     gpu-box · 1 offline │
│ acme/api            4 clones         2 clones         1 clone             │
│ acme/ledger         5 clones         1 clone          —                   │
│ acme/web            1 clone          —                —                   │
│                                                                           │
│ [Add a GitHub repo…]   [Add a machine…]                                   │
└───────────────────────────────────────────────────────────────────────────┘
```
- **CH-10** A channel with no repos is valid; its sessions start in an empty
  folder (`~/Humata/<channel>/sessions/<session>/`).
- **CH-11** The channel page has tabs, in this order: **Sessions · n**,
  **Inbox · n** (count shown only when nonzero), **Board**, **Repos and
  machines**, **Agent access**, **Activity**. The page header has the channel
  name, one faint line naming its repos and machines, and a **New session**
  button (frame 18).
- **CH-12** The Sessions tab is one tree table, the only list of sessions on
  that page. Columns: Session (tree connectors `├─ └─ │`, status dot, title,
  fork kind "⑂ code + context / context only / code only", and a red "✉ n"
  inbox count for the session and all its forks), Agent, Machine ("This Mac"
  shown faint), Status (Needs you in orange), Started, Last used (sorted
  newest first by default). A session with an open
  pull request shows its number after the status ("Idle · PR #491"), and
  clicking it opens the PR. Above the table are a **Filter sessions** field,
  an **Open / Archived / All** switch (LD-3), and **Expand all** and
  **Collapse all** (frame 18).
- **CH-13** Clicking the "✉ n" count opens that session with its Inbox panel
  in "With its forks" mode. Its tooltip says how many come from the session
  and how many from its forks.
- **CH-14** Clicking a row opens the session.
- **CH-15** The Activity tab is the channel's event feed, newest first: forks,
  finished turns (with change totals across repos), agents waiting on you,
  agents asking for approvals, exits, items created, machines going offline.
  Each event shows who (agent name and session, You, or Humata), the channel
  and when. It holds events only, not chat (see q1).
- **CH-16** The sidebar lists channels; each channel can be folded and shows
  its session tree beneath it, indented per depth, with status dots. A channel
  or session with unread output is shown bold. A folded channel shows the
  status mark of the session that most needs the user and its number of
  sessions (frame 36).
  A session's dot is Needs you when that session itself has an unanswered
  request, otherwise its run status; a fork's requests add to its ancestors'
  ✉ badges but never recolour them (D131).
- **CH-17** Opening a session clears its unread mark.

### Growing and shrinking a channel's repos

```text
┌ Repos and machines ───────────────────────────────────────────────────────┐
│                     This Mac · 6      devbox-1 · 3      gpu-box · offline │
│ acme/api            4 clones          2 clones          1 clone           │
│ acme/ledger         5 clones          1 clone           —                 │
│ acme/web  (new)     ▓▓▓▓▓▓░░ 62%      ✓ main clone      waits for gpu-box │
│                                                                           │
│ [Add a GitHub repo…]  saved repositories · URL · local path               │
└───────────────────────────────────────────────────────────────────────────┘
```

- **CH-18** **Add a GitHub repo…** (frame 20) is available at any time on
  the Repos and machines tab. The user picks from saved repositories, pastes
  a GitHub URL, or picks
  a local path on the host (its remote is used for clones; a repo with no
  remote is sent by bundle, FC-7).
- **CH-19** The new repo's main clone is made on every machine of the channel
  in the background, with progress in that repo's row of the grid. A machine
  that is offline clones it when it returns, and its cell says it waits for
  that machine.
- **CH-20** Existing sessions are unchanged by adding a repo. New sessions
  offer every repo of the channel (SE-2).
- **CH-21** **Add repo to this session** (session menu, and the Diff and
  Files panels) clones the repo on the session's machine, on a branch named
  as FC-11, tells the agent the repo and its path in one message, and adds
  the repo's group to the session's diff, files and comments.
- **CH-22** Removing a repo from the channel is allowed at once when no
  session uses it; Humata then deletes its main clones. Otherwise the dialog
  lists the sessions using it and offers, per session, **Drop the repo from
  this session** (after the unpushed-work check, MV-9, for that repo) or
  **Archive the session**. The repo is removed when no session uses it.
- **CH-23** Dropping a repo from a session deletes that session's clone of it,
  tells the agent the repo is gone, and removes its group from the diff,
  files and comments; open comments on it are kept with the session's
  archive record.

### Deleting a channel

- **CH-24** The channel page's menu has **Delete channel…**. Its dialog
  lists every live and archived session of the channel, its board items and
  its pending Inbox requests, and names each session's uncommitted or
  unpushed work as MV-9 does. A session with such work blocks the delete
  until it is pushed or the user ticks "Delete that work". Confirming stops
  the channel's agents, withdraws its pending requests, sends its sessions
  (as LD-5), board, main clones, `shared/` and folders on every machine to
  that machine's trash, and removes the channel from the sidebar. Restoring
  it from the host's trash within 30 days brings the channel back with its
  sessions archived. (provisional, D11)
- **CH-25** Only the user can delete a channel. An agent has no way to ask
  for it, and the CLI needs presence (SEC-10). (provisional, D11)

```text
┌ # payments-rewrite ───────────────────────────────────────── [New session] ┐
│ Sessions · 11 │ Inbox · 7 │ Board │ Repos and machines │ Agent access │ Activity│
├──────────────────────────────────────────────────────────────────────────── ┤
│ SESSION                                   AGENT      MACHINE   STATUS  STARTED│
│ ● Ledger schema migration  ✉ 7            Claude     This Mac  Working  2h   │
│ ├─ ● Try Postgres partitioning ⑂ c+c ✉ 5  Claude     This Mac  Needs you 40m │
│ │  └─ ● Partition by month ⑂ c+c ✉ 3      Claude     This Mac  Idle     30m  │
│ │     └─ ● Monthly + BRIN ⑂ c+c ✉ 3       Claude     This Mac  Working  20m  │
│ │        ├─ ● Benchmark BRIN ⑂ code ✉ 2   Codex      devbox-1  Needs you 8m  │
│ │        │  └─ ● Check vacuum ⑂ ctx ✉ 1   Claude     devbox-1  Needs you 2m  │
│ │        └─ ● Why not hash? ⑂ ctx         Copilot    This Mac  Idle     5m   │
│ └─ ● Review the migration ⑂ ctx           Codex      This Mac  Idle     25m  │
│ ● API rate limiter  ✉ 1                   Codex      devbox-1  Working  1d   │
│ ○ Flaky test hunt                         Copilot    This Mac  Exited   3d   │
└──────────────────────────────────────────────────────────────────────────────┘
```

## 4. Folders and clones

Humata owns these folders on every machine of a channel. The root (`~/Humata`)
is configurable per machine.

```text
~/Humata/                                   root, configurable per machine
├── .trash/                                 this machine's Humata trash (LD-6)
├── desk/                                   Desk sessions' scratch folders (DK-2)
├── pr-reviews/                             the PR reviews channel (PR-2)
└── payments-rewrite/                       one folder per channel
    ├── main/                               read-only reference, auto-fetched
    │   ├── api/          (acme/api @ main)
    │   └── ledger/       (acme/ledger @ main)
    ├── sessions/
    │   ├── ledger-schema/                  one folder per session
    │   │   ├── api/      (branch uhvesta/ledger-schema-migration)
    │   │   └── ledger/   (branch uhvesta/ledger-schema-migration)
    │   ├── flaky-tests/
    │   │   └── api/      (branch uhvesta/flaky-test-hunt; left ledger out)
    │   └── scratch-question/               a session with no repos: empty folder
    └── shared/                             files anyone in the channel shares
```

- **FC-1** The user never picks a path. Every folder above is created and
  named by Humata. The one exception is a Desk session's folder, which the
  user may pick (DK-2).
- **FC-2** A main clone is a read-only reference: agents and Humata's own
  tools do not write to it. Humata fetches it on its own so new session
  clones start from a recent `main`. (Whether a plain shell may work in it is
  q4.)
- **FC-3** Starting a session makes one session clone per chosen repo, on a new
  branch named as FC-11, from the main clone's default branch.
- **FC-4** The user can leave any repo of the channel out of a session.
- **FC-5** Archiving a session deletes its session clones and its folder
  under `sessions/` (LD-1).
- **FC-6** On a remote machine where `gh` is signed in, Humata clones from
  GitHub with that machine's `gh`.
- **FC-7** On a remote machine without a signed-in `gh`, the host makes a git
  bundle and sends it over the SSH link; pushes from that machine go back
  through the host. The GitHub token never leaves the host.
- **FC-8** `shared/` exists on every machine of the channel. An agent can copy
  files into it (rule-gated, see TP-4).
- **FC-9** Humata shows no standing notice about a clone being behind main,
  shared, or unpushed. Unpushed or uncommitted work is shown only where it
  blocks an action (archiving, moving, removing a machine).
- **FC-10** Changing a machine's root moves nothing silently: Humata says
  which channels live under the old root and offers to move them, or the
  change applies to new channels only.

### Branch names

```text
 Settings › Branch prefix   uhvesta/          (channel override: payments/)
 the work                   "Fix refund rounding"
                                 │ slug: lowercase, a–z 0–9 and "-", ≤ 40 chars
                                 ▼
 branch                     uhvesta/fix-refund-rounding
 taken already?             uhvesta/fix-refund-rounding-2      (only then)

 session record (never in the branch name):
   id 7f3c…  ·  channel payments-rewrite  ·  machine devbox-1
   fork of 2a91… at turn 12  ·  repos api, ledger  ·  conversation 5d0e…
```

- **FC-11** A session's branch name is the branch prefix, then a slug from
  the work, then a short suffix only when needed: `uhvesta/fix-refund-rounding`.
- **FC-12** The branch prefix is set in Settings (ST-13) and can be
  overridden per channel. It may be empty.
- **FC-13** The slug comes from the session's title (from the board item's
  title when started from one, or the fork's title for a fork): lowercased,
  letters and digits kept, everything else turned into single hyphens, cut at
  a word boundary to at most 40 characters.
- **FC-14** A suffix (`-2`, `-3`, …) is added only when the name is already
  taken: locally in that repo, on its remote, or by another session of the
  channel. Otherwise there is none.
- **FC-15** The New session and Fork dialogs show the branch name Humata will
  use, and the user can edit it before starting.
- **FC-16** Internal metadata (session id, fork parent, machine, channel,
  conversation id) lives in Humata's session record, never in the branch
  name.
- **FC-17** Renaming a session does not rename its branch. The rename dialog
  offers "Rename the branch too" when the branch has not been pushed; once it
  has, it says the branch keeps its name so its PR stays linked.
- **FC-18** PR review sessions use the PR's own head branch name (PR-6).

## 5. Sessions and terminals

### Starting a session

- **SE-1** The user can start a session from a channel's **New session**
  button, a machine's page ("New session on devbox-1") or a board item ("Start
  a session for this item").
- **SE-2** The New session dialog (frame 31) asks for: title; what it runs,
  an agent (preselected from the default, ST-15) or **Terminal**; machine
  (any online machine of the channel; a machine low on disk shows its free
  space there, LD-19); and per repo of the channel whether to include it
  ("Own clone, new branch from main" or "Leave this repo out"); plus an
  optional first message that is typed into the agent once it starts. With no
  first message the session starts at the agent's prompt. The agent starts
  with its own configured defaults; the dialog does not select or override
  its model (owner direction, 2026-11-15, D76). Frame 31 still draws a model
  picker beside the agents; it is to be removed.
- **SE-3** When started from a board item, the title defaults to the item's
  title, the dialog shows "for PAY-7", the first-message placeholder names the
  item, the new session is added to the item's sessions (BD-16), and an item
  in Backlog or To do moves to In progress.
- **SE-4** After Start session, Humata writes the session record (PER-11) to
  disk, makes the clones, launches the agent in its holder and opens the
  session page. The channel's Activity gets "Started “…” with <agent> on
  <machine>".
- **SE-5** A channel with no repos shows "the session starts in an empty
  folder" in place of the repo list.

### The session page

```text
┌ # payments-rewrite › Ledger schema migration  ● Claude Code · devbox-1  [Fork] [⋯]┐
├─ ◧ Claude Code in “Ledger schema migration” prepared a view: PR review …         ┤
│    [Open it] [Later] [Not now]                                (offer bar, VW-2)   │
├────────────────────────────────────────────┬─────────────────────────┬────────────┤
│ claude │ zsh │ ▤ client.rs ✕ │ ◧ PR review ✕│ Diff            [⤢] [✕] │  ✉ Inbox 7 │
├────────────────────────────────────────────┤ Compare against [main ▾]│  ± Diff  2 │
│ > Design the ledger schema from PAY-3 …    │ 3 files · 2 repos +184  │  ▤ Files   │
│ ● Reading ledger/docs/model.md             │ ┌ ledger uhvesta/ledg… ┐│  ▦ Board   │
│ ● Writing ledger/migrations/0042 +41       │ │ 0042_accounts.sql +41││  ◧ Journal │
│ ● Bash: cargo test -p api ledger           │ └──────────────────────┘│  ⓘ Info    │
│   test result: ok. 14 passed               │ ┌ api uhvesta/ledger-… ┐│            │
│ ● board_item_move PAY-3 → Done             │ │ client.rs  +62 −21   ││            │
│   ⏳ waiting for your approval (Inbox)      │ └──────────────────────┘│            │
│ > █                                        │ [Send 2 comments]       │            │
└────────────────────────────────────────────┴─────────────────────────┴────────────┘
   tab row (terminals, files, views)           panel (opens beside,      right strip
                                               full screen ⤢, closes ✕)
```

- **SE-6** The header shows breadcrumbs (`# channel › parent session › this
  session`, or `Desk › this session`; each a link except the last), the
  status dot, the title, the agent and the machine ("Claude Code ·
  devbox-1"), and the status in words only when it needs attention (Needs
  you, Agent exited, <machine> is offline). Buttons: **Resume agent** (only
  when the agent exited), **Fork**, **Terminal full screen** (NV-37), the
  side panel toggle (SE-11) and **⋯** (session menu).
- **SE-7** The session menu offers: Rename, Move to another machine…, Stop the
  agent (keeps terminal and clones), Push and open a pull request, Archive
  session….
- **SE-8** A session's terminals, open files and opened views are tabs in one
  row (D76): the agent's terminal (named by its command, e.g. `claude`, with
  its status dot), a `zsh` shell in the session's folder, one tab per file
  opened big (with ✕, RV-15), and one tab per opened agent-prepared review
  view (with ✕; closing it keeps it in the Journal). **+** at the end of the
  row opens a new shell tab, a file, or another agent's terminal (SE-28).
- **SE-9** The user can open more shell tabs. Each tab is its own terminal
  held by `humatad` (PER-1) and survives like the agent's terminal.
- **SE-10** The terminal is the agent CLI's own terminal: keystrokes go to it
  exactly as in a normal terminal, and everything the CLI draws is shown as
  drawn (colours, cursor, alternate screen, mouse, OSC 52 clipboard). There is
  no chat layer: queueing, rewind, compaction and images are whatever the CLI
  offers. Mouse interaction includes click-to-focus, selection, scrolling,
  and the CLI's requested mouse reporting; Shift keeps local selection when
  mouse reporting is active. Interactive keyboard text, keys and paste go only to the
  focused terminal grid. Editing a palette, form, find field or another
  terminal must not also type into this terminal. App shortcuts respect the
  focused control. Read-only and disconnected terminals cannot send input.
  (Owner clarification, 2026-11-15; D126.)
- **SE-11** The right strip has, top to bottom: **Inbox** (badge: pending
  requests of the session and its forks), **Diff** (badge: unsent comments),
  **Files**, **Board** (the session's board), **Journal**, **Info**. Clicking
  one opens its panel beside the terminal; clicking it again closes it. The
  header's panel toggle (⌥⌘0) hides and shows the open panel.
- **SE-12** An open panel has a header with its name, **Full screen** (the
  panel covers the terminal; frames 11, 12) / **Show the terminal**, and
  **Close** (✕).
- **SE-13** Sessions are stateful. When the user comes back to a session,
  Humata restores the open panel, whether it or the terminal was full screen,
  the tab groups with each group's tabs and active tab, open files, scroll
  positions, the diff base, the file browser's filter, folds and open file,
  and the inbox mode. This state survives an app restart. After a crash or a
  power cut everything but exact scroll positions comes back (PER-25).
- **SE-14** When the agent exits, the terminal stays with its last output and
  a line saying the process exited with its code and that Resume starts the
  agent again with its conversation. The header and Home show **Resume**.
- **SE-15** Resume starts the agent with its pinned conversation id (PER-12)
  in the same folder and prints a line saying the conversation was restored.
- **SE-16** While the session's machine is offline, the terminal shows the
  last output and a line saying the connection was lost, the agent keeps
  running there, and the view will catch up. Typing is disabled. The tab bar
  says "disconnected". On reconnect, Humata shows everything produced
  meanwhile and a line "reconnected; caught up N lines of output". When
  more was produced than the scrollback keeps, the view starts from a fresh
  screen and the line says how many lines were not kept (TR-4).
- **SE-17** **Open in editor** opens the file in the user's own editor on the
  session's machine. It is not **Open big**, which opens the file as a tab in
  Humata (RV-15). The Files panel offers both (frame 06).

### Info panel

- **SE-18** The Info panel shows: agent; machine with its OS; started; forked
  from (link, with fork kind); forks (links); each clone (repo, branch, path,
  "main clone" chip if it is one); for a context-only fork, "Read only: the
  files of “parent”" and a **Give it its own copy** button (FK-9).
- **SE-19** It has a **Survives restarts** card: the terminal is held by
  `humatad` on <machine>, not by the app; after a reboot the agent is started
  again with its exact resume command (e.g. `claude --resume <id>`), or for a
  plain shell "a fresh shell in the same folder"; scrollback kept on disk on
  that machine with its line limit.
- **SE-20** It has a **What this agent may do** card: it owns this session,
  its board (n items), its clones and the items it created (n); everything
  else goes to the Inbox; link to the channel's rules. The card says whether
  the agent is contained (SEC-6) and, when it is not, that it can read and
  write anything the user can without asking Humata (SEC-5).
  (provisional, D1)

### Turns and typing

- **SE-21** Humata learns that an agent's turn ended from the agent's own
  hooks for Claude Code and Copilot CLI. For Codex CLI, OpenCode and
  user-added agents it uses the output fallback: the turn is taken to have
  ended after 3 seconds with no terminal output. A plain shell has no turns.
  The source is shown in Settings › Agents (ST-1). (provisional, D7)
- **SE-22** A session whose turn end comes from the output fallback shows
  **Probably idle**, never Idle, until the user types or the agent writes
  output again. (provisional, D7)
- **SE-23** Humata may hold messages the user has not sent to the terminal.
  The waiting list represents only this app-owned buffer; it cannot inspect
  or control the agent CLI's internal queue. A bar says **Waiting to send**
  with the number of held messages; **Show** lists them with **Send now**
  and **Remove**, and **Hide** collapses it. The user can send an individual
  message now or send a batch. Explicit Send delivers terminal input without
  requiring Humata to detect a turn end; the CLI decides how to interpret it.
  Once terminal delivery is acknowledged, the item leaves Humata's waiting
  list. Delivery does not mean the agent has processed or accepted it.
  An optional wait-until-turn-end mode must be an explicit choice and only
  auto-send on a reliable hook; fallback detection and plain shells need
  explicit Send. Whole messages use serialized bracketed paste, never
  interleaving bytes with another writer. (Owner clarification, 2026-11-15;
  D126, superseding the mandatory wait policy of D7.)
- **SE-24** Several writers may type into one terminal at once: the app,
  `humata attach` and Humata's own queued messages. Each writer's terminal
  shows "n others can type here" while others are attached with input.
  (provisional, D14)
- **SE-25** `humata attach --read-only` shows the terminal and sends no
  input; it does not count as a writer in SE-24. (provisional, D14)

### Last used (owner direction 2026-11-15, D99)

- **SE-26** Every session records when it was **last used**: the last time
  the user typed into it, opened it or answered a request from it. Work the
  agent does on its own does not count. Wherever sessions are listed (a
  channel's Sessions tab, Home, the Desk, search results) the row can show
  it and sort by it, so old sessions are easy to find and clean up.

### Tab groups and a second agent (owner direction 2026-11-15, D76)

- **SE-27** A session's tab row can be split into two **tab groups** side by
  side (frames 14, 41, 43). Each group has its own tabs, its own **+** and a
  ✕ at its right end. A tab moves to the other group by dragging it there or
  from its menu; closing a group, or its last tab, leaves one group with
  every remaining tab. Two groups is the limit, and nothing else in the
  window splits (NV-36). The divider between the groups drags like any other
  (NV-12).
- **SE-28** A session can run a second agent beside its first (frame 14):
  **+** › **Add an agent** starts another agent's TUI in the session's
  folder, on the same clones, as a tab. There is at most one terminal per
  agent in a session: an agent that already has one is not offered. More
  than that, or a second copy of the code, is a fork (FK-2).
- **SE-29** The agent a session was started with is its **main agent**. The
  session's status, its turns (SE-21), the messages Humata queues (SE-23),
  comments (SM-9), its identity for Humata's tools (SEC-15), its Inbox
  requests and its grants are the main agent's. An added agent is a terminal
  that runs an agent and no more: it has its own conversation, pinned,
  resumed after a restart and carried by a move like the main agent's
  (PER-12–PER-15, MV-13); its tab's dot says only whether it is running or
  has exited; nothing is queued for it; and its calls to Humata's tools are
  refused with a message saying that a fork gives the agent a session of its
  own. A fork is made from the main agent's conversation. (provisional, D103)

### The Desk (signed off by the owner 2026-11-15, D100)

Frames 34–43 in `desktop/tests/__snapshots__/design/` are approved goldens
(empty states, the Desk and markdown review). The Desk row sits under Board
in the sidebar. DK-6 (empty states) is in §22, and DK-7–DK-9 (markdown
review) are in §11.

- **DK-1** The sidebar has a **Desk** row under Board, with its open
  sessions under it. A Desk session belongs to no channel. It runs an agent's
  TUI or a plain terminal, with tabs, the two tab groups (SE-27) and the side
  panels like any session.
- **DK-2** A Desk session works in a new scratch folder under the machine's
  Humata root (`<root>/desk/<name>`, so `~/Humata/desk/plan-q4` by default;
  the name is a slug of the title, as FC-13), or in a folder the user picks
  on that machine. This is the one place the user picks a path (FC-1).
  Humata never deletes, trashes or moves a picked folder. A scratch folder is
  deleted 7 days after its session closes (DK-11) unless the user keeps it
  (**Keep folder** in the folder bar, **Keep** on the Desk page, or `humata
  desk keep`). `desk` is a reserved folder name: no channel's folder can be
  `desk`.
- **DK-3** The folder bar above the tabs shows the folder (with a "scratch"
  chip for a scratch folder), a machine picker, a chip per repo with ✕,
  **Add repo** and **Keep folder** (frames 37, 39). Picking another machine,
  or `humata desk move <machine>`, moves the session to any online machine:
  the folder and conversation are carried and checked as in any move (MV-2,
  MV-13, MV-14) and the agent restarts there with its conversation. A picked
  folder is copied, and the original stays where it was. **Add repo**, or
  `humata desk add <owner/repo>`, clones the repo's default branch into the
  folder; a Desk session has no branch of its own (FC-11 does not apply).
  Removing a repo deletes its clone after the unpushed-work check (MV-9), and
  never a branch on the remote.
- **DK-4** A Desk session's Board panel shows every channel's items (scope
  **All channels**, BD-7) and marks the ones the session added. The session
  has two **Agent access** switches, set in the New desk session dialog and
  changeable in its Info panel (frame 40):
  - **Read and change every channel's board**, on by default: its agent adds
    and changes items in any channel without asking;
  - **Make channels and add machines**, off by default: off, each of these
    asks the user; on, neither does.
- **DK-5** The Desk page (frame 36) lists **Open** sessions (status, title,
  agent, machine, folder, repos and last used), **Documents** (the markdown
  files in Desk folders, each with its session and **Copy to a channel…**,
  DK-13) and **Closed** sessions (DK-11) with the days left before their
  folder goes, **Keep** and **Reopen**. Its header has **Sort: last used**
  (SE-26), **Clean up…** and **New desk session**.
- **DK-10** For every rule that is set per channel (agent access rules, the
  While I'm away rule, Contain agents, retention), a Desk session uses the
  defaults for new channels (ST-4, ST-6, ST-14, ST-17). DK-4's two switches
  are that session's grants on top of them: the first allows the "Global
  board, or other channels and their boards" row of IN-17, the second the
  "Create a channel" and "Add, create, start or stop machines" rows. No
  channel is a Desk agent's own: the IN-17 rows about "its channel" give it
  nothing. (provisional, D108)
- **DK-11** **Close session** (the session menu) is the Desk's Archive. It
  stops the agent and the terminals and keeps the record, conversation and
  scrollback as archiving does (LD-2), but leaves the folder in place. The
  session then shows under **Closed** on the Desk page, and **Reopen**
  resumes its agent with its conversation in the same folder. Seven days
  after closing, a closed session whose scratch folder was not kept is
  deleted as LD-5: its folder, record and conversation go to trash and can
  be restored for 30 days (LD-7). A scratch folder whose clones hold
  uncommitted or unpushed work is not deleted then; its row names the work
  that holds it (as LD-17). A kept scratch folder or a picked folder stays,
  with its closed session, until the user deletes the session; deleting a
  session with a picked folder leaves that folder alone. **Clean up…** lists
  the closed sessions, least recently used first, with their folder sizes,
  and deletes the ones the user ticks after the unpushed-work check (MV-9),
  under NV-27. (provisional, D108)
- **DK-12** A Desk session has no board of its own, no branch and no
  **Fork** (frames 37–43 draw none); work that needs them starts as a
  channel session. Its Inbox requests show in its own Inbox panel and in the
  global Inbox under a **Desk** group. It can be pinned, marked VIP and
  searched like any session (NV-20, LD-23). (provisional, D108)
- **DK-13** **Copy to a channel…** asks for a channel, and for a machine
  when the channel has several, and copies the file into that channel's
  `shared/` folder there (FC-8). The Desk's copy stays. (provisional, D108)
- **DK-14** **New desk session** (frame 40) asks for a title; what it runs
  (an agent or **Terminal**); a machine (any online one); the folder (**A
  new scratch folder**, or **A folder on the machine**); optional repos; and
  the two access switches of DK-4.

## 6. Forks

- **FK-1** The user can fork any session with **Fork**. The new session sits
  under its parent in the tree. Trees have any depth, and a fork can be forked.
- **FK-2** The fork dialog offers three kinds, each with a one-line
  description:
  - **Code + context**: new clones copied from the parent's files as they are
    right now (uncommitted changes included), on a new branch, and a new agent
    that knows the conversation.
  - **Context only**: a new agent that knows the conversation and can read the
    parent's files but never write them. For a reviewer or a side question.
  - **Code only**: new clones copied from the parent's files, and a fresh agent
    with no conversation.
- **FK-3** The dialog also asks: fork from (now, or before an earlier turn,
  listed by name, when the agent can fork at a turn); agent
  (preselected from the default, ST-15; any agent may be chosen; plain
  shell only for code only); machine
  (any online machine of the channel).
- **FK-4** The dialog shows a preview of the result as text (see diagram):
  the parent with its repos and turn count, and the fork's clones, branch and
  conversation.
- **FK-5** Same agent and the agent supports context forks: the conversation
  is an exact copy, and the dialog says so.
- **FK-6** Different agent, or an agent without context forks: the
  conversation is handed over as the transcript plus a short summary. The
  dialog says the new agent knows what was said and done but not the exact
  internal state.
- **FK-7** A context-only fork's file tools read the parent's clones and
  cannot write them: writes fail with a message telling the agent to ask for
  its own copy. Its diff, files and editor show the parent's files.
- **FK-8** A context-only fork may run on another machine; it then reads its
  parent's files across the bridge (rule-gated as a cross-machine read).
- **FK-9** **Give it its own copy** turns a context-only fork into code +
  context: Humata copies the parent's files as they are now onto a new branch
  in the fork's own clones, and from then on the agent can edit them.
- **FK-10** A fork's terminal starts with a line naming its parent, the point
  it was forked at and its kind, then "Conversation copied", "Handed over the
  conversation from <agent> as a transcript and summary", or "Fresh agent, no
  conversation".
- **FK-11** The channel's Activity gets "Forked “parent” → “fork” (kind)".
- **FK-12** A fork onto another machine copies the code as the branch plus
  uncommitted files over the bridge, and copies the conversation, fixing any
  path-keyed records the agent keeps so the resumed conversation finds its
  folder (see flow 26.5).
- **FK-13** Archiving a session leaves its forks running; they move up to its
  parent in the tree.

```text
Ledger schema migration  [api, ledger]  Claude · This Mac
├─⑂ Try Postgres partitioning        code + context   own clones uhvesta/try-postgres-partitioning
│  └─⑂ Partition by month            code + context
│     └─⑂ Monthly + BRIN indexes     code + context
│        ├─⑂ Benchmark BRIN vs btree code only        Codex · devbox-1 (fresh agent)
│        │  └─⑂ Check vacuum cost    context only     reads Benchmark's files
│        └─⑂ Why not hash partitions? context only    Copilot: transcript + summary
└─⑂ Review the migration             context only     Codex: reads parent, never writes
```

## 7. Moving and archiving

### Move a session to another machine

- **MV-1** The user can move a session to any other online machine of its
  channel from the session menu. A Desk session moves to any online machine
  (DK-3).
- **MV-2** Moving pauses the agent, copies the branch, uncommitted files and
  conversation to the target machine (fixing the agent's path-keyed records),
  resumes the agent there, and deletes the old copy only after the new one is
  running. The session's record, board, Inbox and comments stay on the host
  and don't move (SL-1); only its pointers change.
- **MV-3** If any step fails, the session stays on the old machine, running as
  before, and Humata says which step failed and that nothing was deleted.
- **MV-4** During a move the session shows "Moving to <machine>…" with the
  step in progress; the terminal is read-only until the agent resumes.
- **MV-5** A move keeps the session's id, title, place in the tree, board,
  Inbox requests and journal.
- **MV-6** Plain shells move as a fresh shell in the same relative folder; the
  move dialog says running programs in them will not come along.

### Archive

- **MV-7** Archive stops the agent, keeps the conversation, record,
  scrollback and a diff summary, and frees the session's disk (LD-1, LD-2).
  An archived session can be restored (LD-4).
- **MV-8** The archive confirmation says that, and if the session has forks,
  "Its n forks keep running".
- **MV-9** If the session has uncommitted or unpushed work, the confirmation
  names it (e.g. "3 uncommitted files on uhvesta/flaky-test-hunt, never pushed.
  Archiving deletes that work.") and offers **Commit, push and open a PR, then
  archive**, **Archive and delete the work**, and **Cancel**. Without such work
  it offers **Archive** and **Cancel**.
- **MV-10** "Commit, push and open a PR, then archive" archives only after the
  push and PR succeed; if either fails, nothing is archived and the error says
  what to fix.
- **MV-11** After archiving, Humata goes to the channel's Sessions tab.

### Push and pull request

- **MV-12** Push and open a pull request works per repo of the session: the
  dialog lists each repo with its branch and change totals, and the user picks
  which to push and open a PR for. PRs are opened with `gh` through the host.
- **MV-13** A move carries everything the session has on its machine, not
  only its clones (D63):
  - the branch, commits not yet pushed, staged, unstaged, untracked and
    ignored files in each clone, and its `refs/humata/…` refs (reviewed
    snapshots, RV-28);
  - the session folder's own files outside the clones, Markdown included
    (documents under review, plans, notes the agent wrote);
  - the machine daemon's store rows for the session (terminal tabs,
    scrollback, attach state, review notes and highlights, revive data),
    exported and imported as the store's self-describing records (TR-13),
    never as raw database files;
  - the agent's own per-session files: its conversation, its project-keyed
    folders and memory (e.g. `~/.claude/projects/<path>/`), with the paths
    in them rewritten for the new folder.
  `shared/` and the main clones are the channel's, not the session's, and
  don't move.
- **MV-14** Before anything is copied, the old machine writes a **move
  manifest**: every item above with its size and hash. The new machine
  checks every hash before the agent resumes, and the old copy goes to the
  old machine's trash (LD-6) only after that check and the resume. A move
  that fails says which item failed to arrive or to match. The manifest is
  kept in the restart report format (PER-28) so the restart doctor can
  finish or undo a move cut off by a reboot.

### Undoing an archive

- **MV-15** Archiving shows a toast naming the session and the disk it
  frees, with **Undo** (frame 32). The session is archived at once, but its
  clones and folder are deleted only when the toast goes: after 10 seconds,
  or at once when the app quits. **Undo** before then brings the session
  back exactly as it was, uncommitted work included, and starts its agent
  again with its conversation. After that only **Restore** brings it back
  (LD-4), from what was pushed. A restart in between finishes the delete
  (PER-27). (provisional, D110)

## 8. Lifecycle and disk

```text
             start                 Archive                     Delete
  (new) ──────────▶ live ─────────────────▶ archived ─────────────────▶ in trash
                    │  ▲   unpushed-work     │  ▲  keeps conversation,      │
                    │  │   check (MV-9)      │  │  record, scrollback,      │ 30 days
                    │  │   frees clones,     │  │  diff summary             ▼
                    │  │   and caches        │  └── Restore (re-clone   (emptied)
                    │  └──────────────────── ┘      the branch)
                    │                                   ▲
                    └── machine gone: Forget machine ───┘ (archived from the mirror;
                                                           unpushed work listed as lost)
```

### Archive frees the disk

- **LD-1** Archiving frees the session's disk once the unpushed-work check
  (MV-9) passes: Humata deletes its session clones, the build caches and
  other files in its session folder, once the Undo window has passed (MV-15).
- **LD-2** An archived session keeps its conversation, its session record
  (title, agent, machine, channel, repos and branches, fork parent, board
  items, journal), its scrollback and a diff summary: per repo, the branch,
  the last pushed commit and the files changed with their +/− at archive time.
- **LD-3** The channel's Sessions tab has an **Open / Archived / All** switch
  (frame 18). Archived lists the archived sessions in the same tree table.
  Opening an archived session shows its scrollback, its conversation and its
  diff summary, read-only, with **Restore** and **Delete**.
- **LD-4** **Restore** asks for a machine (the original one first if it is
  still in the channel), clones each repo's branch from its remote, copies
  the conversation back from the host (fixing path-keyed records) and resumes
  the agent with it. A branch that was never pushed can't come back: the
  dialog says that work was deleted at archive and that the repo will start
  from `main`.

### Delete and trash

- **LD-5** **Delete** (on an archived session, or Archive's "and delete it"
  choice) removes everything of the session: record, conversation,
  scrollback, diff summary and any clones. Everything goes to trash first,
  after the same unpushed-work check.
- **LD-6** Each machine has one Humata trash at `<root>/.trash/`; the host's
  trash also holds deleted session records and conversations. Files deleted
  in the file manager (FM-3) go to the same trash.
- **LD-7** Anything in trash can be restored until it is emptied. Trash
  empties items 30 days after they were deleted; **Empty trash** on the
  Storage page empties it now, after saying how much it frees.

### The host keeps the history

- **LD-8** Every session's conversation is mirrored to the host
  continuously, with its pointers (SL-3), over the machine's existing link
  as an offset-keyed stream (TR-4); the record itself lives on the host
  (SL-1). A machine that was offline catches up when it reconnects. What is
  mirrored is scrubbed first (LD-21).
- **LD-9** Because of the mirror and pointers, losing or deleting a machine
  loses no history up to its last sync; only its clones' unpushed commits
  and dirty files are lost.

### Removing and forgetting machines

- **LD-10** **Remove machine** (machine page) is for a machine that is still
  reachable: for every session on it, in any channel, Humata asks to move it
  to another machine or archive it (CH-8, MV-1, MV-7), then removes the
  machine from every channel and stops its daemon. For a machine a provider
  made, Delete then runs the provider's delete command (MP-21).
- **LD-11** **Forget machine** is for a machine that is already gone. Humata
  does not try to reach it. The confirmation lists every session on it and,
  per session, the work that is lost: dirty files and unpushed commits from
  the last pointers (SL-3). Its sessions become archived from the host's
  records and mirror (LD-2), and each offers **Rebuild on another machine**
  (SL-4).
- **LD-12** A forgotten machine that is added again is a new machine. Humata
  leaves the old folders on it alone and says where they are, so the user can
  recover lost work by hand.

### Storage page

```text
┌ Hosts › devbox-1 › Storage ─────────────────────── 212 GB free of 500 GB ┐
│ KIND             WHAT                               SIZE   USED    ACTION │
│ Session clones   #payments › API rate limiter       1.2 GB  2h   Archive… │
│                  #payments › Flaky test hunt        0.9 GB  9d   Archive… │
│ Main clones      #payments: api, ledger             0.6 GB  —    —        │
│ Build caches     API rate limiter: target/          14 GB   2h   Clear    │
│ Trash            41 items · oldest 12 days          3.1 GB  —    Empty…   │
│ Logs             humatad, holders                   0.4 GB  —    Trim…    │
└──────────────────────────────────────────────────────────────────────────┘
```

- **LD-13** Each machine's page has a **Storage** page listing what Humata
  keeps there: session clones (per channel and session), main clones, build
  caches in session folders, trash, and logs. Each row shows its size and
  last use, and has the one action that frees it (Archive…, Clear, Empty…,
  Trim…). Main clones have no action while their channel uses the machine.
- **LD-14** Sizes are measured when the Storage page opens, not continuously.
- **LD-15** Actions on the Storage page go through the same checks as
  elsewhere: Archive… runs MV-9, and a running session's build cache is
  cleared only after asking, since its next build will be slower.

### Retention rules (opt-in)

- **LD-16** Retention rules are off by default. In Settings (defaults for new
  channels) and per channel, the user can turn on: **Archive PR reviews N
  days after the PR merges or closes**; and **Archive sessions inactive for N
  days**. A session is inactive when, for N days, its terminals had no
  output and no input and the user did not open it. The Idle status plays
  no part.
- **LD-17** A retention rule never deletes work: a session with uncommitted
  or unpushed work is not archived by a rule. Such skips are listed under the rule in Storage
  with the session and the reason.
- **LD-18** Each archive or deletion by a rule is recorded in the channel's
  Activity with the rule that did it.

### Low disk

- **LD-19** Low disk is shown where it blocks an action, and on the Hosts
  page, where **Free space…** sits beside it (HO-3); nowhere else. When a
  clone, fork, move or transfer would not fit, or would leave the machine
  with less than 5 GB free, the dialog says how much it needs and how much
  is free, and lists the largest items from that machine's Storage page with
  their actions, so the user can free space and continue in place. (The
  5 GB floor is provisional, D107.)
- **LD-20** When a machine runs out of disk while sessions run, those
  sessions show "<machine> is out of disk" with **Free up space** (opens its
  Storage page). Sessions, the sidebar and Home carry no other disk notice.

### The mirror and secrets

- **LD-21** Before any part of a conversation leaves its machine for the
  mirror, every exact value of that machine's host variables in it is
  replaced with `‹NAME›`, the variable's name. The host's mirror never holds
  such a value. Other secrets an agent prints (a token it read from a file,
  say) are mirrored as printed, and the Storage page says so.
  (provisional, D12)
- **LD-22** A session's mirror is kept as long as its session record: it
  stays while the session is live or archived, goes to the host's trash when
  the session is deleted (LD-5), and is gone when that trash empties.
  (provisional, D12)

### Searching sessions on every machine (owner direction 2026-11-14, D63)

- **LD-23** The host keeps a full-text index over the mirror (LD-8), so
  one search covers every session on every machine, archived ones included,
  and works while a machine is offline (results from it say "as of its last
  sync"). The index is updated as the mirror stream arrives, never by
  scanning, and is rebuildable: it carries a format version and is rebuilt
  when that changes (D62).
- **LD-24** A hit names the channel, session, machine, agent and turn, and
  opens that turn. Calls to Humata's own tools are not indexed. The ⌘K
  palette searches it, and so does `humata sessions search`.
- **LD-25** Agents search it with the MCP tool `sessions_search` (query,
  channel, machine, agent, time range) and read a hit's turns with
  `session_read` (a session and a turn range, paged). Both answer from the
  host, so an agent on any machine searches all of them. By default an
  agent searches its own channel; other channels need "Search other
  channels" (Ask). What it returns has already been scrubbed (LD-21).
  `humata sessions search` and `sessions_search` return the same results.

## 9. Review across N repos: diff, files, editor

A session has 0, 1 or more repos. Every review surface groups by repo and shows
each repo's branch.

### Diff

- **RV-1** The Diff panel compares the session's files against a base the user
  picks: **origin/main** (where the branch started), **Uncommitted only**,
  **Since I last reviewed**, **Since the fork from its parent** (forks only),
  **Another session's branch…** (lists branches of other sessions in the
  channel).
- **RV-2** The diff is grouped per repo. Each repo group shows its name,
  branch and totals (+/−), and has its own **Push** / **Open PR** actions.
  Above the groups: number of files, number of repos, overall +/−.
- **RV-3** Each file shows repo chip, path, added/modified/deleted, +/−, and
  **Open in editor** (SE-17; frame 01). Clicking the file's path opens it big
  (RV-15). Hunks show their header, line numbers and +/− marks, with
  syntax highlighting by file type.
- **RV-4** Clicking a line number opens a comment box under that line (frame
  42 draws it): the text, a line saying where it will go ("Typed into claude
  when its turn ends"), **Cancel** and **Comment**. What is written is a
  draft at once and shows under its line marked **Draft**. **Cancel**
  discards it, and **Comment** sends it (SM-9).
- **RV-5** **Send N comments** (the panel's footer, disabled at zero; frame
  01) sends every draft of the session as one message (SM-9). The comments
  then show as sent and the session is marked working.
- **RV-6** **Mark reviewed** records the current state of every repo, so that
  "Since I last reviewed" starts there.
- **RV-7** With no changes against the chosen base, the panel says so; with
  no changes at all, "No changes yet".
- **RV-8** The Diff strip icon shows the number of unsent comments.

### Files

- **RV-9** The Files panel is an explorer over all repos of the session: one
  foldable root per repo with its branch, nested folders, file icons by type.
- **RV-10** Folders that contain changed files are open by default and
  coloured; others are folded. Changed files carry an **A** (added) or **M**
  (modified) badge.
- **RV-11** A **Go to file…** filter flattens the tree to matching paths per
  repo ("No matches" where none). A **Changed only** toggle shows the number of
  changed files and hides the rest.
- **RV-12** Picking a file opens a line-numbered, highlighted viewer with a
  breadcrumb `repo › folder › file`, added lines marked, and **Open big**.
  Narrow panel: the viewer replaces the tree, with a back arrow. Full-screen
  panel: tree left, viewer right.
- **RV-13** For a session on another machine, the panel says the files are
  read from that machine over SSH; while it is offline, it shows the last copy
  read and says so.
- **RV-14** A session with no repos shows "This session has no repos" in Diff,
  Files and editor.

### Editor tab ("Open big")

- **RV-15** Open big opens the whole file as a tab in the session's tab row
  (SE-8). Each file has its own tab; opening a file that is already open
  selects its tab. The side panel stays as it was.
- **RV-16** The editor's left list shows the session's changed files across
  repos (A/M badge, name, folder), the current one highlighted, and "Browse
  files…" which opens the Files panel.
- **RV-17** For a changed file, the editor offers three modes: **Inline**
  (old and new line numbers, removed and added lines interleaved), **Side by
  side** (removed lines paired with the added lines after them; empty cells
  hatched), **Full file** (RV-21). An unchanged file
  shows as a plain file.
- **RV-18** Unchanged regions fold to "⋯ N unchanged lines — show"; clicking
  expands them and the choice is remembered per file.
- **RV-19** The editor shows the number of changes and ↑/↓ buttons that jump
  to the previous/next change, wrapping around.
- **RV-20** Comments work in the editor as in the diff (RV-4), on new-side line
  numbers, and share the same drafts. When there are drafts, a footer shows
  "N draft comments" with **Send N comments** (RV-5).

### Full-file mode

```text
┌ api › src › ledger › client.rs   [Inline] [Side by side] [Full file]  ☑ Show changes   2 of 5 ⌃⌘↓ ┐
│  38      38 │   let account = repo.find(id)?;                                              ▕   │
│  39         │-  let cents = amount * 100;                  (red, old line number)          ▕ ▬ │
│         39  │+  let cents = amount.round_half_even(2)?;    (green)                         ▕   │
│  40         │-  ledger.post(cents);                        deleted row kept in place       ▕ ▬ │
│         40  │+  ledger.post(Posting::debit(cents))?;                                       ▕   │
│         41  │+  audit.record(&account, cents);             added                           ▕ ▬ │
│  41      42 │   Ok(())                                                                     ▕   │
└──────────────────────────────────────────────────────────────────── scrollbar marks changes ┘
```

- **RV-21** **Full file** shows the whole changed file, scrollable, with
  changes inline against the compare base (RV-1): added lines green, deleted
  lines kept in place as red rows with their old line numbers, and modified
  lines as the red old line followed by the green new one. Nothing is folded.
- **RV-22** **Show changes** toggles the inline changes (on by default); off
  shows the plain current file.
- **RV-23** Next and previous change have shortcuts (⌃⌘↓ / ⌃⌘↑) and a counter
  ("2 of 5"); the scrollbar marks where the changes are, and clicking a mark
  jumps there.
- **RV-24** Full-file mode works for every repo of a multi-repo session,
  each against its own base, and follows the Long lines setting (ST-7 to
  ST-10). Opening a changed file from Files (RV-12) or with Open big uses it
  when it was the last mode chosen.

### Comments on a session's own changes

- **RV-25** Reviewing a session's own changes (not a PR) supports line and
  range comments in the diff, the editor's three modes, full-file mode and
  Files, against any base (RV-1) and in any repo of the session. They use
  the one comment model (SM-13).
- **RV-26** Comments are drafted first, then sent one at a time (**Comment**)
  or as a batch (**Send N comments**) to the session's terminal, as one
  bracketed paste that waits for the end of a turn (SM-9, SM-10, SM-14).
- **RV-27** Each sent comment carries its repo, file, line range with its side
  (old or new), the base it was compared against, and the quoted code, so
  the agent can find it even after the file has changed.

### Since I last reviewed (owner direction 2026-11-14, D55)

- **RV-28** **Mark reviewed** (RV-6) stores, per repo, a snapshot of exactly
  what the user saw: the commit plus a tree of the working tree as shown
  (staged, unstaged and the untracked files the diff included), written as
  git objects under a private ref (`refs/humata/reviewed/<session>/<n>`).
  Nothing is copied outside git's object store, and unchanged files cost
  nothing, so "Since I last reviewed" is a plain tree-to-worktree diff
  however large the repo.
- **RV-29** Each file can also be marked **Viewed** on its own, recording
  its content hash. A viewed file whose content changed since is shown again
  with only the change since it was viewed, and "Viewed" is cleared; an
  unchanged one stays folded as viewed.
- **RV-30** Earlier snapshots are kept (the last 20 per session, and every
  one a comment was sent from), so the user can compare against any earlier
  review ("Since review 3, Tuesday 14:02").

### Agent-prepared reviews (owner direction 2026-11-14, D55)

- **RV-31** A **diff spec** says, per repo, what a review shows: the base
  (the branch's merge base with a branch, as GitHub shows a PR
  (`origin/main...HEAD`); a branch or commit directly; a reviewed snapshot,
  RV-28; the parent's fork point), what's compared (commits only; plus
  staged; plus unstaged; plus untracked, all or by path), path filters, and
  whether whitespace and renames count. Every base of RV-1 is a diff spec,
  and the user can edit the one showing ("Comparing: origin/main...HEAD +
  unstaged ▾").
- **RV-32** An agent can prepare one with the MCP tool `review_prepare`
  (gated by "Prepare views for you", like VW-1): a diff spec, an order for
  the files, groups with titles, and notes on files, hunks and lines. It
  arrives as a review view offer (VW-2) and opens as the Diff panel or a
  review tab showing exactly that; **Revert** goes back (VW-9).
- **RV-33** Humata ships two skills for it, synced to every machine
  (SK-1): **/render-humata-review** turns the user's words ("against what
  GitHub will show", "just my unstaged changes", "since commit abc, plus the
  new files in migrations/") into a diff spec, and **/order-humata-review**
  orders the files top-down (entry points first) or bottom-up (leaf changes
  first), groups them and writes the notes. The ordering skill asks the
  agent to hand the work to a small, fast model where the agent supports
  subagents (e.g. Claude Code's Haiku), and runs inline otherwise.
- **RV-34** The order and notes stay attached to the review while the
  changes move under them: a note on a line that changed is kept and marked
  outdated, as drafts are (PR-16).

### Upload into a session

- **RV-35** The Files panel's **Upload…** (frame 06) copies files the user
  picks on their computer into the folder shown, as a transfer (FM-5–FM-8).

## 10. PR reviews

```text
 paste https://github.com/acme/api/pull/482  (⌘K, or #pr-reviews)
┌ Review a pull request ────────────────────────────────────────────────────┐
│ acme/api #482  Fix refund rounding for partial captures                   │
│ by rkhan · uhvesta/fix-refund-rounding → main · 7 files +214 −38          │
│ checks: 11 passed, 1 failed (lint)                                        │
│ Also review  [+ add a PR in another repo]   acme/ledger #91 (linked)  ✕   │
│ Machine  [This Mac ▾]  (This Mac · devbox-1 · gpu-box · lima-dev)         │
│ Agent    [Claude Code ▾]                                                  │
│ Context  rounding on partial captures; anything that can double-refund    │
│ [ ] Let the agent push to the PR branch                                   │
│                                              [Cancel] [Start the review]  │
└───────────────────────────────────────────────────────────────────────────┘

 the review session
┌ #pr-reviews › acme/api #482 ● Claude Code ──────────────── [Submit review]┐
│ claude │ zsh                        │ Diff  PR #482 against main · 2 repos │
│ > why round() and not floor()?      │ api  uhvesta/fix-refund-rounding     │
│ ● review_focus refund.rs:140        │  refund.rs  +62 −21                  │
│   It rounds half away from zero …   │  140 │+ let cents = amount.round();  │
│ > note that on the line for me      │      ┆ note by Claude Code:          │
│ ● review_note refund.rs:140  [Undo] │      ┆ "round() is half-away-from-…" │
│ > █                                 │      ┆ [Edit] [Drop]                 │
│                                     │ ledger #91  uhvesta/refund-ledger    │
│ ⚠ 2 new commits · [Update the clone]│ Drafts: 4 (3 by agent, 1 by you)    │
└─────────────────────────────────────┴──────────────────────────────────────┘
```

### Starting a review

- **PR-1** Pasting a GitHub pull request URL (or `owner/repo#n`) into the ⌘K
  palette (the sidebar's **Search**) or the **PR reviews** channel opens
  **Review a pull request**. Other text in ⌘K searches as usual.
- **PR-2** The **PR reviews** channel holds every review session. Humata
  creates it, with its folders, the first time a PR is reviewed; its repos are
  whatever repos its PRs come from.
- **PR-3** Humata fetches the PR's title, author, base and head branches,
  size (files, +/−) and checks with the host's `gh`. If that fails, the
  dialog says why (not signed in to `gh`, no access to the repo, PR not found,
  rate limited) and what to do; for a rate limit it says to wait.
- **PR-4** The dialog picks the machine (This Mac or any SSH machine,
  provider-made ones included), the agent (preselected from the
  default, ST-15) and optional context for the agent. **Also review** adds PRs from other
  repos to the same session, so one review can span N repos.
- **PR-5** **Start the review** clones each PR's head into that machine's
  `~/Humata/pr-reviews/sessions/<review>/<repo>` (FC-6, FC-7) and starts the
  **review chat** (PR-22): an agent told each PR's URL, title, base and head,
  the user's context, and the tools of PR-23. It is not asked to review the
  PR, and waits for the user's first question.
- **PR-6** A review's clones are on the PR's own head branch name, tracking
  the PR's head (`refs/pull/<n>/head` for PRs from forks).

### Read-only by default

- **PR-7** A review session is read-only towards GitHub by default: the agent
  can edit files, build and test in its clones to answer a question, but pushes through Humata
  are refused, and the refusal tells it so. The channel's push rule (IN-24)
  does not apply to review sessions; only PR-8 does. An uncontained agent
  that pushes with the machine's own git credentials is not stopped
  (SEC-5).
- **PR-8** **Let the agent push to the PR branch** (in the dialog and in the
  session's Info panel) allows pushes for that session only. Pushes go
  through the host (FC-7).

### Draft comments

- **PR-9** When the user asks for one, the agent drafts a line comment for
  GitHub with the MCP tool `review_draft` (repo, path, line or range, side,
  text). Drafts are allowed by default and never posted by the agent.
- **PR-10** The Diff panel's base defaults to each PR's base ("PR #482 against
  main"), grouped per repo with the PR's number. The other bases of RV-1 stay
  available.
- **PR-11** Drafts appear under their lines in the diff, the editor and review
  views, marked with who wrote them, with **Edit** and **Drop**. The user can
  add their own drafts from the same comment box, which in a review session
  offers **Draft for GitHub** beside **Comment** (RV-4).
- **PR-12** Nothing is posted to GitHub until the user submits. **Submit
  review** lists the drafts per PR (each editable or droppable), a summary the
  agent may have drafted, and the verdict per PR: **Comment**, **Approve** or
  **Request changes**. Submitting posts one review per PR with the host's `gh`.
- **PR-13** If posting fails, nothing is lost: the drafts stay, and the
  message says what failed and what to do. A review that posted for some PRs
  and not others says which.
- **PR-14** After submitting, posted drafts show as posted with a link; the
  session stays open for follow-up.

### When the PR changes

- **PR-15** Every minute while a review session is open, and every 5 minutes
  while it is not, Humata checks each PR's head with the host's `gh`. New commits show
  "N new commits, update the clone" on the session (diff header and Home).
  GitHub offers nothing to push this, so it is one of the polls TR-7 allows
  and is listed in `api/GRPC.md`.
- **PR-16** **Update the clone** fetches the new head and tells the agent
  what changed: the new commits and the files changed since the previous
  head. A force-push is reported as such. Drafts on lines that changed are
  kept and marked outdated.
- **PR-17** A merged or closed PR shows "PR #482 was merged" (or closed) on
  its session with **Archive**. If drafts were never submitted, the Archive
  dialog names them before going ahead.
- **PR-18** The PR reviews channel's Sessions table shows, per review, the
  PR(s), author, state (open, merged, closed), drafts count and new commits.

### Asking about the PR (owner direction 2026-11-14, D55)

- **PR-19** Every line, range, hunk and file in a PR review offers **Ask**
  besides the comment box: the question goes, with its repo, file, lines,
  side and the quoted code, to the review chat (PR-22), never to GitHub.
- **PR-20** The user can also type any general question about the PR in
  the review chat. Answers that cite code link back to it in the diff.
- **PR-21** Where a review clones is the user's choice, as for any session
  (PR-4): This Mac or any machine. A review can be moved to another machine
  later (MV-1), drafts, notes, the review chat and reviewed snapshots
  included.

### The review chat (owner direction 2026-11-14, D59)

The agent in a review session does not review the PR. The user reviews it;
the agent is there to answer questions and to work the review's view, so the
user builds a code-level understanding of the change.

- **PR-22** A review session's agent terminal is the **review chat**. The
  agent has the clones to read, search, build and run, and the review's view
  to work with. It never posts to GitHub and gives no verdict of its own.
- **PR-23** The agent works the view with MCP tools, each scoped to its own
  review session:
  - `review_context`: what the user is looking at (the diff spec, the open
    file, the visible lines, the selection, the order and groups, Viewed
    files and notes), so "what does this do?" needs no copying;
  - `review_focus`: open a file, scroll to and select lines or a hunk, and
    highlight ranges (possibly in several files) with a short label;
  - `review_note`: an inline note **for the user** on a file, hunk, line or
    range, shown under it marked with the agent's name; notes are never
    posted, and **Make it a draft** turns one into a GitHub draft (PR-11);
  - `review_prepare` (RV-32) to order, group, filter and collapse files, and
    `review_draft` (PR-9) when the user asks for a GitHub draft.
- **PR-24** In the review chat, the agent's changes to the view show at
  once, without an offer (VW-2), because the user asked for them in the
  chat. Each one appears in the chat as a line ("● review_focus
  refund.rs:140") with **Undo**, and **Back** in the view returns to where
  the user was before the agent last moved it. The agent never moves the
  view while the user is scrolling, typing or selecting; it waits until they
  stop.
- **PR-25** The user can say how much the agent may do: in the review
  session's Info panel, **Agent may: Focus, Notes, Order** (all on by
  default). With one off, the tool returns a denial that says the user
  turned it off.
- **PR-26** Notes, highlights and order are part of the review: they persist
  across restarts and moves, stay on their lines as the PR changes (outdated
  when the line changes, as RV-34), and show in the review's Journal.

## 11. Markdown review and comments

One model covers reading Markdown and commenting on anything (owner
directions 2026-11-14 and 2026-11-15; D100, D101, D104). Any session, in a
channel or on the Desk, reviews Markdown this way; there is no separate spec
mode. Frames 39 and 41–43 draw it.

```text
┌ Desk › BRIN rollout RFC  ○ Claude Code · devbox-1 ───────────────────────────┐
│ claude │ zsh │ ▤ rfc-brin.md ✕ │ +        [Preview] [Source] [Side by side]  │
│ Scrolling is linked: the page follows the source.        ☑ Link scrolling    │
├──────────────────────────────────────┬───────────────────────────────────────┤
│  7 ▢ ledger_entries is 410 GB and …  │ Problem                               │
│   ┌ You · line 7 · sent to claude ┐  │ ledger_entries is 410 GB and its      │
│   │ Where are the sizes from?     │  │ btree on posted_at is 38 GB. …        │
│   └───────────────────────────────┘  │                                       │
│ 13 ▢ - Keep the btree until the      │ Proposal                              │
│ 14     backfill ends, then drop …    │ ▌• Keep the btree until the      ▢ 1  │
│   ┌ You · lines 13–14  [Draft] ───┐  │ ▌  backfill ends, then drop it …      │
│   │ Keep both until range p95 …   │  │                                       │
│   │ Typed into claude when its    │  │                                       │
│   │ turn ends  [Cancel] [Comment] │  │                                       │
│   └───────────────────────────────┘  │                                       │
└──────────────────────────────────────┴───────────────────────────────────────┘
   source on the left, the rendered page on the right; comments under their lines

 comment status:  draft ──Comment / Send N──▶ queued ──turn ends──▶ sent
                    ▲                                                 │ the agent
                    │                                                 ▼ changes the lines
                    └────── Reopen ◀── resolved ◀── Resolve ───── addressed
                  archived: hidden and kept, by the user only (SM-20)
```

### Markdown tabs

- **SM-3** Markdown renders fully: headings, tables, fenced code with syntax
  highlighting, task lists, links, and ASCII art and code blocks in monospace
  with their spacing kept.
- **SM-4** A Markdown file opens as a tab in the session's tab row like any
  file (RV-15), from Files, the **+** menu or ⌘K. The tab shows **Preview**,
  **Source** or **Side by side**; switching keeps the position in the file.
- **DK-7** In any session, a markdown file tab can sit in the second tab
  group (SE-27) beside the agent's terminal, so the user reads the rendered
  file while the agent stays in view (frame 41).
- **DK-8** Side by side puts the source on the left and the rendered page on
  the right. Scrolling is linked (the page follows the source), a bar under
  the tabs says so, and **Link scrolling** turns it off (frame 42). The
  terminal can stay in the first tab group beside both (frame 43). The agent
  answers a comment in its terminal, not in the comment thread, unless it
  uses DK-9.

### Comments

- **SM-6** Comments are made on source lines. In Source or Side by side,
  clicking a line number or selecting lines opens the comment box under them
  (RV-4). The comment is anchored to those source lines and keeps their
  text. The rendered page lights each block that has comments and shows
  their count; clicking the count shows them in Side by side. Selecting text
  in the rendered page offers no comment.
- **SM-7** When the file changes, each comment re-anchors to its passage. If
  the passage is gone, the comment says so and offers **Re-attach** (select a
  new passage) or **Drop**.
- **SM-8** Comments show inline under their lines, as in a diff (frame 42),
  not in a margin. Each shows who wrote it, its lines and its status ("You ·
  line 7 · sent to claude"). Writing, editing or dropping a draft never
  types anything into the agent's terminal.
- **SM-9** A comment is a **draft** from its first keystroke until it is
  sent or cancelled, and drafts survive restarts (PER-25). **Comment** on
  the box sends that one comment. **Send N comments** (RV-5) sends every
  draft of the session as one message. This is the one place the message is
  defined: it is typed into the session's main agent (SE-29) as one
  bracketed paste holding, per comment, a line `[comment on <path>:<lines>]`
  and then the comment's text (frame 43). The path starts with the repo when
  the session has more than one. A comment made in a diff adds its side and
  base to that line and the quoted code under it (RV-27).
- **SM-10** The user can send comments one at a time as they write them,
  or keep drafts and send them together (SM-9). Explicit Send follows SE-23:
  input goes to the terminal and its CLI interprets it. Only comments held
  by Humata before terminal delivery may say **Waiting to send**. After
  acknowledged delivery they may say **sent to <agent>**; this makes no
  claim about an agent-internal queue or whether the agent has acted on them.
  (Owner clarification, 2026-11-15; D126.)
- **SM-11** Each comment has a status: **draft** → **waiting to send** → **sent** →
  **addressed** (by the rule in SM-16). The user can **Resolve** a comment
  or **Reopen** a resolved one. Reopening sets it back to draft, whoever
  resolved it, so the user can edit it and send it again; its replies stay
  in the thread.
- **SM-12** Resolved comments are hidden by default and shown with **Show
  resolved**.
- **DK-9** The agent can also answer in the comment thread with the MCP
  tool `comment_reply` (`comment.reply`), on a comment that was typed into
  its own session. The reply shows under the comment, marked with the
  agent's name, and the page's count goes up. The agent still answers in
  its terminal as usual; the tool's description asks it to hand the call to
  a cheap subagent (TP-17). (Built after DK-1–DK-8.)

### One comment model everywhere

- **SM-13** Comments on Markdown source (SM-6), diffs (RV-4), the editor (RV-20),
  files and PR reviews (PR-11) are one model: anchored to a repo, path and
  source lines with the quoted passage, with the same statuses, re-anchoring
  and batch send.
- **SM-14** A comment's destination is the session's terminal (SM-9, SM-10),
  or, for a PR review draft, GitHub (PR-12). A PR draft goes through the same
  draft → sent (posted) states and is never posted without the user's
  submit.
- **SM-15** The Diff strip badge and footers count draft comments across
  every view of the session.
- **SM-16** A sent comment becomes addressed when a change the agent makes
  after it was sent alters any of the comment's anchored lines. A change
  that only alters whitespace does not count. A passage moved elsewhere
  unchanged is re-anchored (SM-7) and stays sent. Changes the user makes
  never address a comment. (provisional, D13)

### Resolving and archiving (owner direction 2026-11-15, D101)

- **SM-17** A resolved comment stays resolved. Re-anchoring, its passage
  changing, the agent rewriting the file or a PR update marking it outdated
  never reopens it or brings it back into view. Only **Reopen** does.
- **SM-18** An addressed comment shows its passage before and after the
  agent's change, with **Resolve**. **Resolve N addressed** resolves every
  addressed comment in the file (or the session's Diff) at once.
- **SM-19** The agent can resolve a comment with the MCP tool
  `comment_resolve` (`comment.resolve`), and only with a reply: the call
  carries a short note on what it did, which shows in the thread marked with
  the agent's name, as `comment_reply` (DK-9) does. A comment the agent
  resolved says so ("Resolved by claude") and can be reopened like any other.
  The action has its own agent access rule (Allow by default); with Never,
  the tool returns a denial that says the user turned it off.
- **SM-20** Only the user can **archive** a comment; no MCP tool or CLI
  command the agent can reach does it. Archiving hides the comment
  everywhere, including under Show resolved, without marking it dealt with.
  It works on any comment, drafts included; an archived comment is never
  sent and is left out of every count (SM-15). Archiving shows an Undo toast.
- **SM-21** Archived comments are listed under **Archived** in the session's
  Info panel, with **Unarchive**, and stay in the session's Journal. Drop
  (SM-7) deletes a comment; archive keeps it.

## 12. File manager and transfers

```text
┌ Files ─────────────────────────────────────────────────────────────────────┐
│ [This Mac ▾] ~/Downloads/fixtures       ⇄   [gpu-box ▾] ~/Humata/ml/shared  │
├──────────────────────────────────────┬─────────────────────────────────────┤
│ ▸ ..                                 │ ▸ ..                                │
│ ▾ datasets/          2.1 GB          │   weights/            —             │
│     train.parquet    1.8 GB          │   notes.md            4 KB          │
│     eval.parquet     310 MB          │                                     │
│   README.md          2 KB            │   ◂── drop here, or drag from Finder│
├──────────────────────────────────────┴─────────────────────────────────────┤
│ [New folder] [Rename] [Copy →] [Move →] [Delete]      Trash on gpu-box (3)  │
├────────────────────────────────────────────────────────────────────────────┤
│ Transfers  datasets/ → gpu-box   1.4 / 2.1 GB  ▓▓▓▓▓▓▓░░░  paused: link down │
│            resumes on reconnect · checksummed                    [Cancel]  │
└────────────────────────────────────────────────────────────────────────────┘
```

- **FM-1** The file manager has two panes. Each pane can show any machine
  (This Mac or a remote), so the user can work between this computer and a
  machine, or between two machines.
- **FM-2** In either pane the user can create a folder, rename, copy, move and
  delete files and folders, and copy or move between the panes.
- **FM-3** Delete moves to the machine's Humata trash (LD-6). The user can
  restore it from the trash until it empties after 30 days (LD-7); the trash
  shows its item count.
- **FM-4** The user can drag files from Finder into a pane and from a pane out
  to Finder.
- **FM-5** Transfers run over the machine's existing SSH link; no other port
  or connection is opened.
- **FM-6** Transfers are chunked, resumable and checksummed. A file counts as
  transferred only when its checksum matches on the destination.
- **FM-7** A transfers tray lists every transfer with source → destination,
  progress and state, and lets the user cancel one.
- **FM-8** When the link drops, the transfer pauses and says why; on reconnect
  it continues from the last confirmed offset without user action.
- **FM-9** Machine ↔ machine transfers go through the host (AR-4).
- **FM-10** The file manager can open any folder Humata owns on a machine
  (channel folders, `shared/`, session folders) as well as the user's home on
  that machine.
- **FM-11** A machine's page lists its Humata folders by channel; clicking one
  opens it in the file manager.
- **FM-12** Each pane has its own machine picker; offline machines are listed
  but cannot be picked. A pane whose machine goes offline shows the last
  listing read, says so, and disables actions on it.
- **FM-13** When either end of a transfer is offline (or the host, for a
  machine ↔ machine transfer), the transfer pauses and the tray says which
  machine it waits for; it continues on its own when that machine is back.
- **FM-14** A move across machines deletes the source only after every file
  has arrived with a matching checksum; the source goes to its machine's
  Humata trash.
- **FM-15** The file manager is its own page, reached from ⌘K and from a
  machine's folders (FM-11); the sidebar has no row for it. It remembers
  each pane's machine and folder across restarts.

## 13. Boards

Frames 21–24 draw the Board page, frames 07 and 12 a session's Board panel,
and frame 37 a Desk session's.

- **BD-1** There are three kinds of board: the **Global** board (the
  sidebar's **Board**), one board per **channel**, and one board per
  **session**. All have the columns Backlog, To do, In progress, In review,
  Done, with the count per column.
- **BD-2** The user can create an item on any board with **New item** or the
  **+** on a column (title, status), drag items between columns, and open an
  item in a drawer.
- **BD-3** The item drawer (frame 22) shows: id; title; **Status**;
  **Channel** (which board it is on); **Sessions**, the sessions working on
  it, each with its status dot and opening that session (BD-16); **Depends
  on** (BD-14); a pending request about the item (BD-15); description;
  history; who owns it ("You made this. Agents need your approval to change
  it." or "Made by <agent · session>. That agent may change it freely; others
  must ask you."); and **Start a session for this item** (SE-3).
- **BD-4** An item card shows its id, its channel on a board that spans
  channels, its title, and a chip for each session working on it with that
  session's status dot. A card with a pending request shows the Inbox mark
  and **Review** (BD-15).
- **BD-5** The channel board's **Scope** picker offers: **Channel board**,
  **Channel + every session board**, **A session and its forks** (one entry
  per session that has forks), **One session's board** (one entry per
  session).
- **BD-6** New item on the channel board creates a channel item, except in
  "one session's board" scope, where it goes on that session's board.
- **BD-7** The Global board shows global items and every channel's items (not
  session items). Its **Scope** picker offers **All channels** (everything,
  the default), **Global items only**, or one channel. Cards there also show
  their channel.
- **BD-8** The session strip's Board panel shows that session's board, one
  column under another. Made full screen, it shows the columns side by side
  (frame 12). A Desk session's Board panel is DK-4's.
- **BD-9** An agent creating, editing, moving or deleting items follows the
  channel's agent access rules (IN-17): by default it may do anything on its
  own session board, add to the channel board and edit, move or delete
  channel items it created; anything else is an Inbox request.
- **BD-10** What a fork's session board starts with is open (q5).
- **BD-11** Every board page has in its header (frames 21–24): the **Scope**
  picker, a **Board / Table / Dashboard** switch, a **Filter** field that
  narrows what is shown by text (id, title, session, channel), and **New
  item**. The view, scope and filter are kept per board (SE-13).
- **BD-12** **Table** lists the same items one per row, grouped by status
  with each group's count, each group foldable: Key, Title, Worked on by
  (the item's sessions), Channel. A row opens the drawer. A row with a
  pending request is highlighted and has **Review** (BD-15).
- **BD-13** **Dashboard** (frame 24) counts the items in scope, and every
  figure opens what it counts (NV-7):
  - tiles for **Waiting on you** (**Review the asks** opens those requests),
    **In progress** (its line names what needs the user among them, such as
    "1 session exited", and opens it), **Done this week** (with the change
    on last week; opens those items in the Table) and **Unassigned**
    (**Start a session** opens them to pick one);
  - **By channel**: a bar per channel split by status, with its count, which
    opens that channel's board;
  - **Done per day** over 14 days: a bar opens that day's items;
  - **By session**: each session that has items, with its status, its count
    and **Open** or **Resume**.
- **BD-14** An item can depend on other items, on any board. The drawer
  lists them under **Depends on**, each marked ✓ once it is Done, and each
  opens. The user edits the list in the drawer; an agent sets it with the
  board tools, under the rule for editing that item (BD-9). An edge that
  would close a loop is refused, and the refusal names the loop.
  Dependencies block nothing: no status change or session start is refused
  because of one. (provisional, D106)
- **BD-15** When an agent's Inbox request is about one item (moving, editing
  or deleting it), the item shows it in place. The card and the table row
  get **Review**, which opens the drawer with the ask in it: who asks and
  for what ("Claude Code asks to move this to Done"), the agent's reason,
  quoted, and buttons named after the change (**Move to Done**, **Keep in
  progress**) with **Open session**. The first allows the request and the
  second denies it, exactly as answering its Inbox card does (IN-12, IN-13,
  IN-15). (provisional, D106)
- **BD-16** An item can have several sessions working on it. Starting a
  session for the item adds that session (SE-3); the user adds and removes
  others in the drawer, from the sessions of the item's channel. An item
  with none counts as unassigned. (provisional, D106)

## 14. Inbox and agent access

### Where the Inbox appears

- **IN-1** There are three Inbox scopes: **global** (all channels and the
  Desk; the sidebar shows the pending count), **channel** (the channel's
  Inbox tab), and **session** (the strip's Inbox panel).
- **IN-2** The session Inbox has a toggle **This session · n** / **With its
  forks · n** when the session has forks. Opening it from a tree count opens
  it in "With its forks".
- **IN-3** Wide inboxes (global, channel) show the list on the left and the
  selected request in full on the right (frame 26). The global list is
  grouped by channel, and each row names the request, the agent and its
  session. The first pending request is selected by default.
- **IN-4** The narrow session Inbox shows rows; clicking a row opens its card
  in place under it, clicking again closes it.
- **IN-5** Each row shows an icon for its kind (board ▦, bridge ⇄, session ▶),
  the action and its age, or "approved"/"denied" once answered.
- **IN-6** **Allow all n** and **Deny all** appear when more than one request
  is pending in the current scope and apply only to that scope.
- **IN-7** Wide inboxes have a **Waiting / Answered / All** switch (frame
  26). Answered requests show dimmed, with their answer and any note. The
  session Inbox panel lists waiting requests only; what was answered is in
  its log (IN-8).
- **IN-8** Every Inbox scope has an **Allowed without asking** log: what the
  sessions in scope did that their rules allowed, and every answer, with
  when. In the session panel it sits under the requests (frame 03). In the
  wide inboxes it is a folded group under Answered and All.

### The decision card

```text
┌───────────────────────────────────────────────────────────────────────────┐
│ ● Run the benchmark against the staging database                       3m │
│ Codex CLI in Benchmark BRIN vs btree · network_connect                    │
│ devbox-1 · #payments                                       Open session ↗ │
│ ┃ “I need numbers from data shaped like production. The staging copy has  │
│ ┃  40M postings; my local fixture has 10k …”   — the agent, with its ask  │
│                                                                           │
│ WHAT WILL HAPPEN                                                          │
│   Runs on devbox-1, connecting to staging-db                              │
│   $ psql "host=staging-db.internal port=5432 user=bench_ro" -f bench/…    │
│   • Read-only user bench_ro: it cannot write                              │
│   • About 6 minutes, 6 queries                                            │
│                                                                           │
│ WHAT IT WAS DOING                                                         │
│   ● Wrote bench/brin_vs_btree.sql (6 queries)                             │
│   ● Local run: BRIN 2.1× faster — fixture too small to trust              │
│   ⏳ network_connect staging-db:5432                                       │
│                                                                           │
│ Asks you because: Network connection through the host   Change this rule  │
│                                                                           │
│ ☐ Always allow connecting to staging-db:5432 for this session             │
│ [Allow] [Deny]                                                   [Reply…] │
└───────────────────────────────────────────────────────────────────────────┘
```

- **IN-9** Every request card carries what the user needs to decide, laid
  out as frames 03 and 26 draw it. In order:
  1. **The action** as a title, with the requester's status dot and the
     request's age.
  2. **Requester**: "<agent> in <session>" and the operation (TP-16); in a
     wide inbox also its machine and channel, and **Open session ↗**
     (IN-11).
  3. **The reason**: the agent's reason, and why this needs the user.
  4. **What will happen**, previewed exactly as labelled rows (IN-10).
  5. **What it was doing**: its last few steps, the pending call last. Left
     out in the session Inbox panel, where the terminal is beside the card.
  6. **The rule** that sent it to the user, with **Change this rule** (opens
     the channel's Agent access tab).
  7. **Whether it can be undone**: a line in red, shown only when it cannot
     be undone or undoing it costs something.
  8. The grant checkbox (IN-14), then **Allow**, **Deny** and **Reply…**. A
     question (IN-25) shows its choices and a reply field in their place.

  The frames draw parts 1–4 and 8. Parts 5–7 and Open session ↗ keep the
  owner's rule that a request carries its full decision context; frames 03,
  04, 26 and 27 are to gain them. (provisional, D105)
- **IN-10** The preview depends on the kind of request:
  - Board item change: the item (opens its drawer) and each field before →
    after.
  - New board item: which board, title, status, description.
  - File read or copy: source session and machine, mode (e.g. "read only, for
    this turn"), and each path, each openable in that session's Files panel.
  - Command: where it runs, the exact command, and notes.
  - Typing into another terminal: the target session and agent, the last
    lines of its terminal, and the exact text to be typed, highlighted.
  - New session or fork: agent, machine, kind, repos and how they are copied,
    the size of what will be cloned or copied, and a usage note in words
    ("Starts a new Codex CLI agent; its usage counts against your Codex
    plan"). No price is shown.
  - Network connection: the machine it opens from, the host and port, and
    that it stays open until the session stops (HO-11).
  - Push or pull request: each repo, its branch, the commits to push and the
    PR's title (IN-24).
  - Question for you: the question, its choices and a reply field (IN-25).
  - Access asked ahead: the kind of action, its target and the grant that
    approving makes (IN-26).
  - Cross-machine copy: source machine and path → destination machine and
    path, sizes.
  - Context hand-off or note to another session: the exact content to be
    sent.
- **IN-11** **Open session ↗** opens the requester's session with its Inbox
  panel on "This session" and the request selected.
- **IN-12** **Allow** performs the action whether or not the agent's call is
  still waiting (TP-13). If the call is waiting, it returns the action's
  result. If the agent already got "pending" (TP-11), Humata types one line
  into its terminal at the end of its turn (SE-23): "… — approved by you",
  with the request id to wait on. (provisional, D4)
- **IN-13** **Deny** returns a denial naming the user to the agent's call, or
  to its next wait on the request (TP-12), and does nothing else.
  **Reply…** asks for a note and denies the request with it; the agent gets
  the note with the denial. If the agent already got "pending", Humata types
  "… — denied by you" as IN-12 does. (Reply… always denying is provisional,
  D105.)
- **IN-14** The card's checkbox is **Always allow …**. With it ticked,
  **Allow** approves this request and also adds a grant for the requesting
  session and exactly the target IN-22 names for this kind of action,
  nothing wider. Its label says what it covers ("Always allow connecting to
  staging-db:5432 for this session"). A kind of action with no target in IN-22
  shows no checkbox. The agent cannot choose a wider scope. The grant is
  listed and removable in Agent access (IN-19).
- **IN-15** Answering a request removes it from every scope's pending list at
  once and updates every count. Only the first answer counts: a second
  answer from anywhere fails and says who answered it, how and when (AW-10).
  (provisional, D3)

### Agent access

- **IN-16** Some actions need no rule, and no setting stops them: reading its
  own session (title, status, repos, branches), reading and writing its own
  clones, its own views, its own requests, and replies to comments sent to
  its own session (DK-9). A context-only fork reads its
  parent's clones the same way but can never write them (IN-20). Every other
  action is checked against the channel's rules, including its own session
  board and the channel items it created, whose rules default to Allow. The
  rules govern calls made through Humata only (SEC-5).
- **IN-17** The Agent access tab lists rules per kind of action, each with
  **Allow / Ask me / Never**, and explains that anything asked lands in the
  Inbox batched with every other agent's requests. Defaults:

  | Action | Default |
  | --- | --- |
  | Read and write its own clones | Always (IN-16; no setting) |
  | Its own session board: add, edit, move, delete | Allow |
  | See its channel: machines, sessions, the channel board | Allow |
  | Channel board: add items | Allow |
  | Channel board: edit, move or delete items it created | Allow |
  | Channel board: edit, move or delete anyone else's items | Ask me |
  | Global board, or other channels and their boards: read (D127.4) | Allow |
  | Write into `shared/` on its machine | Allow |
  | Fetch files from `shared/`, the channel folder or a main clone on its machine into its own clone | Allow |
  | Prepare views for you (you choose whether to open them) | Allow |
  | Draft PR review comments (review sessions) | Allow |
  | Resolve comments sent to its own session, with a reply (SM-19) | Allow |
  | Ask you a question, or ask ahead for access (IN-25, IN-26) | Allow |
  | Read another session's files (same machine) | Ask me |
  | Read files on another machine (bridged) | Ask me |
  | Copy files into its clone or `shared/` from another session or machine | Ask me |
  | Send a note or question to another session | Ask me |
  | Type into another session's terminal | Ask me |
  | Start or fork sessions (same machine) | Ask me |
  | Start or fork sessions on another machine | Ask me |
  | Start a PR review | Ask me |
  | Change its own session: rename, add or drop a repo, own copy | Ask me |
  | Stop, archive or delete its own session | Ask me |
  | Move a session to another machine | Ask me |
  | Push its branch or open a pull request (IN-24) | Ask me |
  | Open a network connection through the host (HO-11) | Ask me |
  | Add, create, start or stop machines; sync skills (MP-24) | Ask me |
  | Restore from trash | Ask me |
  | Create a channel | Ask me |
  | Stop or delete any session but itself | Never |
  | Change channel settings or repos | Never |
  | Remove, forget or delete machines (MP-24) | Never |

  Access rules themselves are not in this list: only the user changes them
  (IN-23).

- **IN-18** "Never" returns a denial to the agent at once, with the rule named,
  and adds nothing to the Inbox.
- **IN-19** Scoped "Always allow …" rules are listed under their rule with their
  scope and can be removed.
- **IN-20** A context-only fork's writes to its parent's clones are never
  allowed, whatever the rules say (FK-7).
- **IN-21** Rule changes take effect for the next call; pending requests stay
  pending. When a pending request is answered, the answer is checked against
  the rule as it is then: approving a request whose rule became Never fails
  and says the rule changed.
- **IN-22** A grant made by **Always allow …** covers exactly this target:

  | Kind of action | Target the grant covers |
  | --- | --- |
  | Read or copy another session's files | That session, and only the paths under the folder asked for |
  | Read or copy files on another machine | That machine, and only the paths under the folder asked for |
  | Note, question or typing into another session | That session |
  | Start or fork a session | That machine and that agent |
  | Edit, move or delete others' items | That board |
  | Network connection | That machine, host and port |
  | Push or pull request | That repo and branch |
  | Machines | That machine and that action |

  A grant belongs to one session. Forks and rebuilt sessions get none of
  their parent's grants. A moved session keeps its grants. A grant ends when
  its session is archived or deleted. A grant whose rule is set to Never
  stops applying until the rule changes back. An agent cannot widen a grant.
- **IN-23** Only the user changes access rules, the While I'm away rule,
  grants, defaults for new channels, Settings and agents' launch commands.
  No rule lets an agent do it. An agent's attempt is denied at once, names
  this rule and adds nothing to the Inbox. (provisional, D6)
- **IN-24** A push or pull request that an agent asks Humata for
  (`session_push`) goes through the "Push its branch or open a pull request"
  rule. The user's own **Push and open PR(s)** (MV-12) needs no rule. Review
  sessions follow PR-7 and PR-8 instead. An uncontained agent
  that pushes with its own git credentials is not stopped by this rule
  (SEC-5).
- **IN-25** An agent can ask the user a question with up to 4 choices. The
  card shows the question, the choices and a reply field. The user's choice
  or reply is the result of the agent's call. Waiting works as TP-11.
- **IN-26** An agent can ask ahead for access: the kind of action, its target
  and its reason. The card shows them; approving makes the same grant as
  **Always allow …** for that target (IN-22), and denying returns a denial.

## 15. Agent-prepared views and the Journal

- **VW-1** An agent can call `view_prepare` to prepare a view for the
  user: a **review walkthrough** (ordered steps, each a repo, path and note,
  with a title and summary), a **board view** (a channel board in a given
  scope, with a title and summary) or a **pull request view** (VW-13). Gated
  by the "Prepare views for you" rule.
- **VW-2** Preparing a view changes nothing on screen. Humata shows an offer
  bar: "<agent> in “<session>” prepared a view for you: <title>", the summary,
  and **Open it**, **Later**, **Not now**. (Frame 03 draws only Open it and
  Later; Not now is to be added to it, D112.)
- **VW-3** Review offers appear on the session page. Board offers appear on
  the session page and on the channel board.
- **VW-4** **Later** hides the offer and shows it again the next time the
  user returns to that session.
- **VW-5** **Not now** hides the offer for good; the view stays in the
  Journal.
- **VW-6** **Open it** on a review opens it as a tab in the session's tab
  row: the left list shows the title, who prepared it and when, the summary,
  and numbered steps with notes; the main area shows the step's file in the
  editor (RV-17) with the agent's note above it, and **‹ Prev** / **Next ›**.
- **VW-7** **Open it** on a board view opens the channel board in the view's
  scope.
- **VW-8** While an opened view is showing, a bar says "Showing “<title>”,
  prepared by <agent>" with **Revert** and **Keep**.
- **VW-9** **Revert** restores exactly what the user had before opening:
  route, terminal tab, open panel, full-screen state and board scope. **Keep**
  removes the bar and leaves the view as it is.
- **VW-10** The Journal panel lists every view prepared in the session and its
  forks, newest first, with its title, which session (for forks), kind, when,
  summary, state (Waiting for you, Later, Not now, Opened, Reverted, Opened and
  closed) and **Open**. Any view can be opened again at any time.
- **VW-11** With no views, the Journal says how to get one (e.g. ask the agent
  to "prepare the PR review for me").
- **VW-12** Views work for sessions on remote machines: the view is stored
  with the session on the host and its files are read over the bridge.
- **VW-13** A pull request view holds, per repo, a title, a body, a base
  branch and whether it is a draft. Preparing it pushes nothing. **Open it**
  shows the fields, editable, with **Push and open PR(s)**, which runs the
  push and pull request action (MV-12) with what is shown.
- **VW-14** A prepared view is a draft until the agent marks it ready. A
  draft is not offered and is not in the Journal. The agent can revise its
  draft as often as it likes. Marking it ready shows the offer (VW-2).

## 16. Agents' view of the topology (MCP tools)

Each `humatad` serves MCP tools to the agents on its machine. The MCP server
(`humatad mcp`, one per agent) is a thin client: it proves which session it
belongs to and passes each call to its machine's `humatad`, which decides.
On the host the daemon acts itself; on any other machine it forwards calls
that reach outside the session to the host. The host may be a Mac or a
Linux machine (AR-7).

- **TP-1** An agent can ask which machine it is on, which other machines the
  channel has, and which sessions run on each (title, agent, status).
- **TP-2** An agent can read files of its own session freely.
- **TP-3** An agent can read files of another session or another machine,
  subject to the rules (Inbox when asked).
- **TP-4** An agent can copy files from another session or machine into its
  own clone or into `shared/`, subject to the rules.
- **TP-5** An agent can send a note or summary to another session, or ask it a
  question, subject to the rules. A delivered note is typed into the target's
  terminal at its next turn end (SE-23) as one message naming the sender. A
  question is typed the same way with its id; the target answers it with
  `session_answer`, and the answer is the result of the asking agent's
  call. Until then the question is pending, and waiting works as TP-11.
- **TP-6** An agent can start a fork of itself on another machine, subject to
  the rules; the Inbox card shows agent, machine, kind, repos, the size to
  copy and a usage note (IN-10).
- **TP-7** An agent can create, edit, move and delete board items on boards it
  may use (BD-9), and prepare views (VW-1).
- **TP-8** Every cross-machine call goes through the host and is checked there
  against the channel's rules; calls on anything the agent doesn't own become
  Inbox requests.
- **TP-9** While the host is unreachable, calls behave as the table in AW-7 says.
  The session keeps running either way. (provisional, D3)
- **TP-10** While a call waits, Humata types nothing into the agent's
  terminal; the agent's CLI shows the call as it shows any waiting tool call.
  The only line Humata types is the one in IN-12.
- **TP-11** A call that needs an answer waits up to `wait_seconds` (default
  45, at most 600). While it waits it reports progress every 15 seconds. Then
  it returns "pending" with a request id, and the request stays in the
  Inbox. The agent waits again on that id. (provisional, D4)
- **TP-12** Waiting again on a request returns what the original call would
  have: the result once approved, the denial with any note, or "pending"
  again.
- **TP-13** An approved request runs its action whether or not the agent is
  waiting. Its result is kept for 24 hours for the agent to collect.
  (provisional, D4)
- **TP-14** Repeating a call with the same request (a client retry) returns
  the same Inbox request; it never makes a second one.
- **TP-15** A request is withdrawn when the agent withdraws it, when its
  session stops, is archived or is deleted, or when the agent's MCP process
  exits while the call is open and no "pending" was returned. A withdrawn
  card greys out and can't be answered. A request that returned "pending"
  stays answerable after the MCP process exits.
- **TP-16** Every MCP tool is named `<area>_<verb>` (`session_fork`,
  `board_item_move`) on the MCP server `humata`. Activity, item history and
  Inbox cards show the dotted operation id (`session.fork`).
- **TP-17** Humata's skills and MCP tool descriptions encourage agents to
  hand tedious, mechanical MCP and tool calls (replying to comments, board
  edits, collecting results) to a cheap subagent where the agent's CLI has
  subagents: Haiku in Claude Code, Luna in GPT-based agents. The subagent
  runs with the smallest context window and medium reasoning effort where
  the CLI lets the caller choose them. Where it has no subagents, the agent
  makes the calls itself. (owner direction 2026-11-15, D100)

## 17. Hosts and the bridge

- **HO-1** The user can add a machine over SSH (plain SSH host, MP-10) by
  picking a host from `~/.ssh/config` (or typing one). Humata then shows steps with progress:
  Connect over SSH; Check the system (arch, disk, git, gh); Install humatad;
  Start it and open the link; Find agents. When done, the machine shows in
  Hosts and in every machine picker.
- **HO-2** If a step fails, Humata stops there, says what failed and how to fix
  it, and offers Try again.
- **HO-3** The Hosts page (frame 28) has a table with one row per machine:
  status; the machine with its system and how it is reached (ssh or a
  provider); its number of sessions; its disk, as a bar and the free space
  (an offline machine shows when it was last seen, a stopped one "stopped");
  and the one action the row needs: **Free space…** when its disk is low
  (opens its Storage page), **Reconnect** when it is offline, **Start** when
  its provider says it is stopped (MP-21). The bar turns red, and the
  low-disk line shows, once more than 85% of the disk is used (provisional,
  D107). Selecting a row, or a machine in the sidebar, shows that machine's
  card beside the table: its daemon version, uptime and how it is linked;
  the tools found on it with their versions (`gh` and the agents); the
  low-disk line when it applies ("11 GB free: new clones stop at 5 GB",
  LD-19); and **Free space…** and **Open a shell**. A **Bridge** card lists
  each link from the host, with its latency or since when it is offline.
- **HO-4** A machine's page shows its OS, address and daemon version;
  **Reconnect now** when offline; **Open a shell** for remote machines;
  sessions on it (each opens) and **New session on <machine>**; a health card
  (gh signed in as which user with which scopes, agents installed, disk free,
  variable names); its Humata folders by channel; and what its agents can
  reach through the host with each rule's setting.
- **HO-5** When a machine goes offline, its sessions keep running there; the
  sidebar marks it offline, its sessions show "<machine> is offline", and
  its page says only the link is down, Humata
  retries every 5–30 s, and its files show the last copy read.
- **HO-6** Humata reconnects on its own with backoff. **Reconnect now** tries
  at once. On reconnect each session catches up its output as TR-4 says:
  exactly while the missed output is still kept, otherwise from a fresh
  screen. Humata says how many sessions reattached.
- **HO-7** Host variables: only their names are sent to clients; values stay
  on the machine. The machine's page lists the names with **Set value…** and
  **Remove** (HO-13).
- **HO-8** GitHub access uses the `gh` CLI; there is no GitHub OAuth app. The
  host's GitHub token never leaves the host.
- **HO-9** Remote machines never talk to each other directly. Possibly later:
  direct machine-to-machine SSH under the same rules.
- **HO-10** The bridge carries only Humata's own calls and file transfers.
  Nothing else crosses it unless the user approves a port forward (HO-11).
  (provisional, D5)
- **HO-11** A port forward is opened only when the user approves it. The card
  shows the machine it opens from, the host and port it reaches, and that it
  stays open until the session stops. Low-risk rules never cover it, and it
  can't be approved while the host is away. It closes when the session stops.
  (provisional, D5)
- **HO-12** The network rule promises only that Humata forwards nothing
  without approval. An uncontained agent can still open its own connections
  from its machine (SEC-5); a contained agent cannot (SEC-6). The rule's row
  in Agent access says so. (provisional, D5, D1)
- **HO-13** **Set value…** asks for a host variable's value in a masked field
  and stores it on that machine. Nobody can read a value back, from the app,
  the CLI or an agent; it can only be replaced or removed. Only the user sets
  or removes values. `humata host var set <machine> <NAME>` reads the value
  from the terminal with echo off, or from standard input, never from an
  argument. Agents launched after the change get the new value in their
  environment; running agents keep the one they started with.

### Skills sync

```text
 The host                                     devbox-1
 ~/.claude/skills/, plugins       one way     ~/Humata/.skills/   (Humata-managed;
 other agents' skill dirs   ───────────▶    claude/, copilot/  agents pointed here)
   minus exclude patterns,                    ~/.claude/skills/   (made on devbox-1:
   minus .env, keys, *.pem …                                       never touched)
```

- **SK-1** The host's skills (each agent's skill and plugin folders) sync one
  way to machines. Nothing syncs back.
- **SK-2** `humata skills sync [machine…]` syncs to the named machines, or to
  all; Hosts and each machine's page have **Sync skills**.
- **SK-3** Settings has **Sync skills to machines automatically** (on connect
  and whenever the host's skills change), with an opt-out per machine.
- **SK-4** Sync writes only into a Humata-managed skills folder on the
  machine, and agents started by Humata there are pointed at it. Skills made
  on the machine itself are never overwritten or deleted.
- **SK-5** Before a machine's first sync, Humata shows what will be added,
  updated and removed, and asks. When a machine-local skill has the same name
  as a synced one, the user picks which one that machine's agents use.
- **SK-6** Exclude patterns can be set in Settings. Files that look like
  secrets (`.env`, private keys, `*.pem`, token files) are never synced.
- **SK-7** A machine that is offline when skills change syncs when it
  returns.
- **SK-8** A failed sync is shown where it blocks, on a session that needs the
  missing skill, with the reason and **Retry**.
- **SK-9** A sync replaces a machine's whole synced set at once. An agent
  reading skills mid-sync sees the old set or the new one, never a mix.
- **SK-10** A sync to several machines skips the offline ones and says which
  were skipped; they sync when they return (SK-7).
- **SK-11** Every sync, not only the first, can be previewed: what will be
  added, updated and removed, and any name clashes with machine-local skills.
  Automatic syncs apply without asking except when there is a new clash.

### Tools and scripted syncs (D49)

- **TS-0** The desktop app installs the host's own copies the same way: on
  first launch and on every launch after, it puts `humatad2`, `humata2_holder`
  and `humata` from its bundle into `~/.humata2/bin` (checksums, atomic swap,
  never under a live holder, as TS-1), registers `humatad2` to start at
  login (launchd on macOS, PER-20; the systemd user service on Linux, PER-18)
  and puts `humata` on the user's PATH. Nothing on the
  host needs a separate installer, and an app update updates the daemon at
  its next safe moment (PER-6). (D51)

- **TS-1** Adding a machine, and every connect after, installs Humata's own
  binaries there automatically, with no prompt and no download: `humatad2`,
  `humata2_holder` and the `humata` CLI, into `~/.humata2/bin`, a folder only
  Humata uses. Sessions Humata starts there have it on their PATH. A machine
  whose copies differ from the app's (by checksum) gets the app's, replaced
  atomically and only when no session's holder uses the old ones.
- **TS-2** The desktop app embeds these binaries for every machine platform
  it supports (macOS and Linux, arm64 and x86_64), as Humata 1 did. Setup
  reads the machine's OS and architecture and copies the matching set, so a
  machine needs no network, toolchain or package manager. An unsupported
  platform fails at Add machine, naming it.
- **TS-3** Other things can be synced by script: a sync rule names a source
  on the host (a folder or a command's output), a destination on machines, and
  optional `before`/`after` commands run there, as command templates like
  providers'. Rules run on connect and on demand (`humata sync run [rule]
  [machine…]`), with the same preview, secret filtering, offline catch-up and
  whole-set replacement as skills (SK-6, SK-7, SK-9, SK-11).
- **TS-4** Skills sync is the built-in rule; scripted rules can't write into
  Humata's managed folders or outside the user's home.

## 18. Machine providers

Every machine runs `humatad` and is reached through one command. A provider is
a few command templates the user configures; Humata knows nothing about the
tool behind them. A plain SSH host is the simplest case.

```text
 ┌──────────────────────────────────────────────────────────────────────────┐
 │ Same for every machine: channels, sessions, clones, forks, moves, Inbox, │
 │ file manager, transfers, persistence tiers, MCP topology tools           │
 ├──────────────────────────────────────────────────────────────────────────┤
 │ gRPC (tonic) over:  humatad serve --stdio                                │
 ├──────────────────────────────────────────────────────────────────────────┤
 │ reach the machine, one of:                                               │
 │  connect  "<tool> … {name} … {cmd}"   ssh host {cmd} · limactl shell     │
 │           (primary)                   {name} {cmd} · gcloud compute ssh  │
 │                                       {name} --command {cmd} · kubectl   │
 │                                       exec -i {name} -- {cmd}            │
 │  identity SSH JSON (+ host_key pin)   Humata's own known_hosts           │
 │  plain    SSH host or alias typed in                                     │
 ├──────────────────────────────────────────────────────────────────────────┤
 │ lifecycle (optional): create · start · stop · delete · status            │
 │ templates: {name} shell-quoted, also $HUMATA_NAME; declared params only  │
 └──────────────────────────────────────────────────────────────────────────┘
```

### Templates and params

- **MP-1** Every machine is treated the same in every respect, whatever made
  it or however it is reached.
- **MP-2** A provider is a TOML entry on the host (Settings › Machine
  providers) of command templates: `create`, `connect` or `identity`, and
  optional `start`, `stop`, `delete`, `status`. An action appears only when
  its command is defined.
- **MP-3** `{name}` in a template is replaced by the machine's name,
  shell-quoted; it is also exported as `HUMATA_NAME`.
- **MP-4** `HUMATA_<PARAM>` variables exist only for params the provider
  declares; each declared param is a field in Add machine. Commands get no
  other variables from the host or from Humata's secrets, except host
  variables the provider lists by name in `env`.
- **MP-5** Commands run on the host by default, or on a named machine
  (`run_on`).
- **MP-6** Any scriptable setup fits: a cloud CLI, Terraform, an internal
  tool, anything that can run a command on the machine or print its SSH
  identity.

### Reaching the machine

- **MP-7** **connect** (`"<tool> … {name} … {cmd}"`) is the primary form.
  Humata substitutes `{cmd}` and uses this one command for everything:
  probe (`uname -sm` for os and arch); install (stream `humatad` in through
  stdin, check its checksum, move it into place); write the systemd user
  service (PER-18); and the bridge (`humatad serve --stdio` carries the gRPC
  stream, TR-2).
- **MP-8** **identity** is the alternative: it prints JSON with `name`, `ssh`
  (`host`, and `ssh_config` or `port`, `user`, `identity_file`), optional
  `ssh.host_key`, `os`, `arch`. Paths only; output with key material fails
  validation and is neither stored nor shown.
- **MP-9** When `ssh.host_key` is given, Humata pins it with no fingerprint
  prompt. Humata keeps its own known_hosts and never reads or writes
  `~/.ssh/known_hosts`.
- **MP-10** The third option is a plain SSH host or alias typed in (HO-1).
- **MP-11** A bad or missing identity field fails, naming the field and the
  command to fix.

Minimal provider:

```toml
[providers.mine]
create = "create machine {name}"
connect = "connect machine {name} -- {cmd}"
delete = "delete machine {name}"      # optional
```

### Interactive login

```text
┌ Add machine · corp-vm ─────────────────────────────────────────────────────┐
│ corp-ssh corp-vm -- 'echo HUMATA_READY_7f3a'          (real terminal)      │
│ Verification code: ▍                                                      │
│                                                                           │
│ Reconnecting will need you: sessions keep running, but corp-vm can't be   │
│ reached until you log in again.                                           │
│                                          [Cancel]  [Done, I'm logged in]  │
└───────────────────────────────────────────────────────────────────────────┘
 after login:  echo HUMATA_READY_<nonce> ─▶ seen ─▶ stream humatad in if missing
               ─▶ stty raw -echo; exec humatad serve --stdio --sync <nonce>
               ─▶ drop all bytes before the sync marker ─▶ gRPC
```

- **MP-12** A provider can set `interactive = true`; any connect step also
  offers **Open interactively**.
- **MP-13** Humata tries non-interactive first. If the command hangs waiting
  for input, or prints a password, verification code, `(yes/no)?` or
  device-login prompt, Humata offers to open it interactively.
- **MP-14** Interactively, the connect command runs in a real terminal inside
  the dialog and the user completes the login.
- **MP-15** Takeover: Humata types `echo HUMATA_READY_<nonce>` and waits for
  it, streams `humatad` in if missing, then sends `stty raw -echo; exec
  humatad serve --stdio --sync <nonce>`, and discards everything before the
  daemon's sync marker.
- **MP-16** The dialog states the cost: reconnects need the user. Sessions
  keep running in their holders, but the machine is unreachable until the
  user logs in again. "Needs you to reconnect" shows only when the user opens
  something on it, with **Log in**.
- **MP-17** After an interactive login succeeds, Humata offers to make it
  unattended (add a Humata key to `authorized_keys`, or keep the provider's
  cached token) and switches the machine to the non-interactive form.
- **MP-18** A menu-only tool with no shell can't host the bridge; Humata says
  so and points to the identity form or a wrapper script.

### Worked example: a cloud VM

```toml
[providers.hcloud]
label  = "Cloud VM (hcloud)"
params = [{ name = "type", prompt = "Server type", default = "cx22" }]
env    = ["HCLOUD_TOKEN"]          # a host variable's name; value never shown

create = """
d="$HOME/.humata/providers/hcloud/$HUMATA_NAME"; mkdir -p "$d"
ssh-keygen -q -t ed25519 -N '' -f "$d/host_key"
printf '#cloud-config\nssh_keys:\n  ed25519_private: |\n%s\n  ed25519_public: %s\n' \
  "$(sed 's/^/    /' "$d/host_key")" "$(cat "$d/host_key.pub")" > "$d/user-data"
hcloud server create --name {name} --type "$HUMATA_TYPE" \
  --image ubuntu-24.04 --ssh-key humata --user-data-from-file "$d/user-data"
"""

identity = """
d="$HOME/.humata/providers/hcloud/$HUMATA_NAME"
jq -n --arg n "$HUMATA_NAME" --arg ip "$(hcloud server ip {name})" \
  --arg hk "$(cut -d' ' -f1,2 "$d/host_key.pub")" \
  '{name: $n, ssh: {host: $ip, port: 22, user: "root",
    identity_file: "~/.ssh/humata_ed25519", host_key: $hk},
    os: "linux", arch: "x86_64"}'
"""

delete = "hcloud server delete {name}"
status = "hcloud server describe {name} -o json | jq -c '{state: .status}'"
```

```text
┌ Add machine · Cloud VM (hcloud) ──────────────────────────────────────────┐
│ Name ci-box-1      Server type cx22                    [Cancel] [Create]  │
├───────────────────────────────────────────────────────────────────────────┤
│ ✓ create      hcloud server create --name ci-box-1 …   (output streamed)  │
│ ✓ identity    root@203.0.113.7:22 · host key pinned · linux x86_64        │
│ ✓ connect     attempt 3 of 10 (sshd starting) … connected                 │
│ ✓ check       Linux x86_64 · git · disk 38 GB free                        │
│ ✓ install     humatad 2.0.4, checksum ok                                  │
│ ● service     systemd user service + linger                               │
│ ○ register    hosts.json                                                  │
└───────────────────────────────────────────────────────────────────────────┘
```

- **MP-19** After identity, the SSH steps are: connect with retry and
  backoff while the machine boots; check (os, arch, git, disk); install
  `humatad` (checksum verified); systemd user service plus `loginctl
  enable-linger`, with the admin fallback of PER-18 (exact command shown) or
  PER-19; register the machine in `hosts.json`, mode 0600, holding no
  secrets.
- **MP-20** A failed step stops there, keeps its output, says what to fix,
  and offers **Try again**, plus **Delete what was made** when `delete` is
  defined.
- **MP-21** A provider machine's page has the actions its provider defines;
  **Delete** first removes the machine (LD-10), then runs `delete`. A machine
  whose `status` says stopped shows as offline with **Start**.

### Testing

- **MP-22** Humata ships a Lima provider (`connect = "limactl shell {name}
  {cmd}"`) as an example. Machines made with it let the user try every remote
  flow by hand, and integration tests use it to create test machines for
  cross-machine forks, moves and copies, a machine going offline
  mid-transfer, and the kill matrix (PER-23).

### Who may set up machines

- **MP-23** Only the user adds or changes a provider, in Settings. Humata
  never loads a provider from a repo or from any file an agent can write.
  Importing a provider file shows its full text and every command it will
  run; it is added only after the user confirms.
- **MP-24** An agent may ask to create, start or stop a machine; under the
  defaults each goes to the Inbox (IN-17). Removing, forgetting or deleting
  a machine is Never by default.

## 19. Persistence and resume

This is the core requirement: sessions survive the app, the network, the
daemon and, as far as physically possible, the machine.

```text
 What dies                         What survives                    Tier
 ─────────────────────────────────────────────────────────────────────────
 App quit / crash                  the live process, screen,         1
 Host sleep / offline              scrollback; reattach gets a
 SSH link drop                     snapshot + output meanwhile
 ─────────────────────────────────────────────────────────────────────────
 humatad crash / upgrade           holders untouched; new daemon      2
                                   reattaches to them
 ─────────────────────────────────────────────────────────────────────────
 Planned reboot (VM tier on)       sessions inside a small VM whose   3
                                   memory is saved and restored:
                                   exact processes come back
                                   (CRIU on Linux: best effort)
 ─────────────────────────────────────────────────────────────────────────
 Crash, power loss, reboot         scrollback on disk; agent resumed  4
 without tier 3                    with its pinned conversation id;
                                   plain shells in the same folder;
                                   running programs (vim, builds) lost
```

### Where state lives

```text
 Host (e.g. your Mac): source of truth          Each machine: only what must be local
 ┌─────────────────────────────────────────┐   ┌──────────────────────────────────┐
 │ channels · session records · boards     │   │ code clones                      │
 │ Inbox · grants · comments · views       │   │ agent's live context:            │
 │ settings · PR review drafts             │◀──│   conversation files, holders    │
 │ conversation mirror                     │   │                                  │
 │ pointers per session and repo:          │   │ local journal (append-only):     │
 │   path · branch · HEAD sha · upstream · │   │   MCP writes made while the host │
 │   pushed? · dirty-file count ·          │◀──│   is unreachable, replayed in    │
 │   conversation id                       │   │   order on reconnect             │
 └─────────────────────────────────────────┘   └──────────────────────────────────┘
```

- **SL-1** The host (your Mac by default, or a Linux machine, AR-7) is the source of truth for
  everything that can be centralized: channels, session records and metadata,
  boards and item states, the Inbox and grants, comments, views, settings, PR
  review drafts, and the conversation mirror (LD-8).
- **SL-2** Each machine stores only its code clones and the agent's live
  context (conversation files, holders).
- **SL-3** Each machine continuously reports thin pointers to the host: per
  session, each repo cloned, its path, branch, HEAD sha, upstream, whether it
  is pushed, its dirty-file count, and the agent's conversation id.
- **SL-4** If a machine disappears and can't be restarted, the host still has
  every session's record, conversation mirror and pointers. The user can
  rebuild a session on another machine: Humata re-clones each repo at its
  branch, or at the last pushed sha, resumes the conversation from the mirror,
  and marks what is lost (unpushed commits, dirty files) from the last
  pointers.
- **SL-5** While the host is unreachable, the host-state writes that AW-7
  marks "journaled" (board items, comment and view drafts, questions to the
  user) and requests made on the machine go to a local append-only journal.
  The journal replays to the host in order on reconnect. Nothing else is
  journaled. (provisional, D3)
- **SL-6** The host is authoritative. A replayed write is applied field by
  field against the revision it was made on. Where the host changed the same
  field since, the later write wins and the other value is kept in the
  item's history, marked as a lost conflict. Inbox answers never work this
  way: the first answer wins (AW-10). (provisional, D3)
- **SL-7** Every board item, comment and view has a revision that goes up
  with each change. An edit made from a stale revision is not applied: the
  editor shows the current value and asks again. Board fields, marks and
  view state may be written without a revision; then the later write wins
  and the earlier one is kept in history.
- **SL-8** A delete is kept as a tombstone in history. An edit or move that
  arrives for a deleted item does not bring it back; it is kept in history.
  A delete made from an older revision is applied only if nothing it would
  remove changed since; otherwise the item stays and the delete is kept in
  history as lost.

### Tier 1: the app, the host's link, SSH

- **PER-1** Each session's PTY is held by its own small holder process on the
  session's machine. The holder is not a child of the SSH connection or of
  the app, and keeps running when either goes away.
- **PER-2** The holder keeps the terminal's VT screen state (via a terminal
  emulator library) and flushes scrollback to disk at least once a second.
- **PER-3** When a client reattaches, it gets a full screen snapshot and then
  all output produced since its last offset, with nothing lost or repeated,
  while that output is still kept. When it is not, the client gets a fresh
  snapshot and a count of the lines not kept (TR-4).
- **PER-4** Quitting or crashing the app, the host sleeping or going offline,
  or the SSH link dropping, leaves every session's process running.

### Tier 2: the daemon

- **PER-5** A `humatad` crash or upgrade leaves holders untouched. A new
  daemon finds the running holders and reattaches to them.
- **PER-6** A holder is never upgraded under a live session; new holder
  versions apply to new sessions and to sessions after their next restart.
- **PER-7** One daemon runs per machine, enforced by a lock file. A second
  daemon exits with a message naming the running one.

### Tier 3: planned reboot (optional)

- **PER-8** Per machine, the user can turn on a VM tier: sessions run inside a
  small VM whose memory is saved to disk before a planned reboot and restored
  after, so the exact processes come back.
- **PER-9** On Linux, CRIU checkpoint/restore may be used as a best effort;
  when it fails, the session falls back to tier 4 and says so.
- **PER-10** Without tier 3, a reboot is handled by tier 4.

### Tier 4: crash, power loss, unplanned reboot

- **PER-11** Before launching an agent, Humata writes and fsyncs the session
  record: folder, clones, branch, agent, arguments, variable names and
  conversation id.
- **PER-12** The conversation id is pinned at launch (e.g. `claude
  --session-id <uuid>`), never "the latest conversation".
- **PER-13** On boot, the daemon reconciles: every session marked running
  whose holder is gone is resumed with its pinned conversation id, in its
  folder, with its arguments.
- **PER-14** Resume is verified: Humata checks the agent's conversation file
  exists and that the agent actually resumed it. Then it tells the agent it
  was interrupted and to check `git status` before continuing.
- **PER-15** A failed resume is never a silent fresh start. The session shows
  "Couldn't resume" with the reason, **Try again**, and **Start fresh with a
  summary** (a new conversation seeded with a summary of the old transcript).
- **PER-16** Plain shells come back as a fresh shell in the same folder.
  Programs that were running (vim, builds) are lost after a reboot, and the
  terminal says so plainly.
- **PER-17** After a reboot, the scrollback from before is shown above the
  resumed session, with a line marking the reboot and how the agent was
  started again. Agents that were mid-turn come back idle; Humata says so and
  that they need to be told to continue.

### Daemon lifecycle (no root needed)

- **PER-18** On Linux, `humatad` runs as a systemd **user** service, with
  `loginctl enable-linger` so it keeps running after logout and starts at
  boot. When the user can't enable lingering, Humata says an admin must run
  it once and gives the exact command.
- **PER-19** Fallback on Linux without systemd: `setsid` plus a crontab
  `@reboot` entry.
- **PER-20** On macOS, `humatad` is a launchd login item.
- **PER-21** The host reconnects to every machine with backoff.
- **PER-22** tmux is not required. (tmux has the same logout-kill caveat.) A
  bundled static tmux in control mode is a possible fallback holder; it
  degrades graphics, OSC 52 and passthrough.

### Tests

- **PER-23** A kill matrix is part of the test suite: `kill -9` a holder,
  `kill -9` the daemon, cut the SSH link, hard-reset a VM. Each test checks the
  outcome its tier promises: output kept and caught up exactly, or the agent
  resumed with its conversation, or "Couldn't resume" shown.
- **PER-24** Killing a holder (its process dies) is handled like tier 4 for
  that session alone.

### Restarts never lose state (owner direction 2026-11-14, D52)

- **PER-25** Turning any machine off at any instant (a closed lid, a power
  cut, a crash or a reboot, of the host, a remote machine or both) loses
  nothing the user had: every agent session (so each one can be recovered),
  sessions and their tabs, which screen was showing and what was open on it,
  which machines were connected, composer and comment drafts, review
  comments and their send queue, Inbox requests, answers and grants,
  journals, pins, settings, and terminal output up to the last sync (500 ms
  at most). Whatever the user did is on disk, fsynced, before Humata says
  it's done; drafts are saved at least once a second. Exact scroll positions
  and other fine-grained view state are not kept.
- **PER-26** Turning it back on puts the user back where they were: the
  daemons start at login (PER-18–PER-20), the app opens on the same screen
  with the same sessions and agents open, and local sessions reattach or
  resume (PER-13–PER-16). The host tries to reconnect to every machine that
  was connected (PER-21); a machine that needs the user (a login, a
  passphrase, a VM to start) waits for a manual Connect, which is fine.
  Once a machine is connected again, its daemon reports what it held, and
  each of its sessions is reattached, resumed, or listed as recoverable
  with what's needed, so nothing an agent had on that machine is lost
  silently.
- **PER-27** Every multi-step operation (adding a machine, installing tools,
  takeover, sync rules, resume, fork and move, file transfers, the review
  send queue, journal replay) is either finished or cleanly undone after a
  restart at any step, never left half done. A review comment is sent to an
  agent exactly once across restarts.
- **PER-28** The test suite proves it: a power-cut simulation kills every
  process and drops writes not yet fsynced at each fault point, for the host
  alone, a machine alone and both; real hard resets of local VMs cover Linux
  machines and a Linux host (AR-7). Each case checks that everything in
  PER-25 came back or is reported as PER-29 says.
- **PER-29** Every restart writes a **restart report** under the data
  folder's `restarts/<time>/`, in the bug report format (D47, secrets
  masked): what was running before, what was restored, and for anything
  that wasn't, the step that failed, the error, the related log lines, the
  versions, and the files it relied on (conversation files with sizes and
  times, holder pid and scrollback files, store schema version). The app
  shows one line per failed session linking to it.
- **PER-30** Humata ships a **restart doctor** skill and `humata doctor
  restarts [report]`. It reads restart reports, explains what went wrong and
  where, tries repairs that can't lose anything (resume again, relaunch with
  the recovered conversation id, reattach a found holder, restore the store
  from its last good copy) and asks before anything that could. When it
  can't repair, it writes a diagnosis with the cause and proposed fixes to
  Humata itself, ready to file as a bug report.
- **PER-31** The store keeps a last good copy (a periodic `VACUUM INTO`
  snapshot, kept until a newer one checks out), so a damaged database is
  restored, not lost; what changed after that copy is listed in the restart
  report.

## 20. Async: working while the host is away

- **AW-1** Sessions on remote machines keep working while the host is asleep,
  offline or off.
- **AW-2** From any plain SSH terminal on a machine, `humata attach <session>`
  attaches to a session's terminal; detaching leaves it running. `humata ls`
  lists the machine's sessions. Attaching with input is user-only and needs
  presence (SEC-10); an agent's attempt is refused. Several writers may be
  attached at once (SE-24).
- **AW-3** While the host is away, each remote daemon keeps its agents'
  requests in its local journal (SL-5). `humata inbox` on that machine shows
  them, and nothing else shows them until the host is back.
- **AW-4** Each channel has a **While I'm away** rule for requests that would
  ask: **Wait** (the call waits), **Skip the step** (the call is denied with a
  note that the user is away), or **Allow low-risk kinds** (the user picks
  which kinds count as low-risk, from those AW-11 allows).
- **AW-5** The user can answer requests later from the host, or on the machine
  with `humata inbox` over SSH. Answering on the machine while the host is
  away needs the approval passphrase (SEC-13).
- **AW-6** When the host reconnects, each journal replays into the host's Inbox
  in order (SL-5, SL-6). A request answered on the machine arrives answered,
  credited to the user on that machine. A request still pending becomes an
  ordinary Inbox request on the host.
- **AW-7** What each call does:

  | The call | Host reachable | Host away |
  | --- | --- | --- |
  | Reads and writes of its own session's files, clones and `shared/` | done on the machine | done on the machine |
  | Reads of channel state (topology, boards, its comments) | answered by the host | answered from the last copy, marked with its age |
  | Writes the rules allow (own board, channel board add, its own items, comment and view drafts, questions to the user) | done | journaled (SL-5) |
  | Anything the rules say Never | denied | denied |
  | An Ask on something on this machine (another session here, start or fork here, type into a session here) | Inbox request; the call waits | per the While I'm away rule |
  | Anything that needs another machine or GitHub (read or copy from another machine, start or fork elsewhere, a note to a session elsewhere, push, port forward) | done or asked | waits, or under Skip denied saying the host is away; never low-risk |
  | User-only calls | user-only (SEC-9) | only answering requests the machine holds, with the passphrase |

  (provisional, D3)
- **AW-8** While the host is away, a machine judges calls against the last copy
  of the rules, grants and While I'm away rule the host sent it. Nothing on
  the machine can change that copy. (provisional, D3)
- **AW-9** On reconnect, every journaled write is checked again against the
  rules as they are then. A write a tightened rule no longer allows is not
  applied; it is shown in the item's history and as one line in the agent's
  terminal. (provisional, D3)
- **AW-10** The first answer to a request wins, as IN-15 says, wherever the
  answers come from. A deny never undoes an approval that already ran.
  (provisional, D3)
- **AW-11** Low-risk kinds can never include anything that needs another
  machine, a network connection, typing into another session, or any action
  that is Never by default. (provisional, D3)
- **AW-12** **Always allow …** given on the machine while the host is away
  applies there at once and becomes a grant on the host at replay, with the
  same target (IN-22). (provisional, D3)

## 21. Transport: gRPC streaming

- **TR-1** `humatad` speaks gRPC (tonic). The app and the CLI reach the host's
  daemon over a local unix socket, or over SSH when the host is another
  machine (AR-7).
- **TR-2** The host reaches each remote daemon over one SSH connection per
  machine, carrying gRPC either over a stdio tunnel or a forwarded unix
  socket. All calls for that machine are multiplexed on it.
- **TR-3** Terminal output and events are server streams; terminal input and
  chunked file transfer are client or bidirectional streams; commands are
  unary calls.
- **TR-4** Every stream is keyed by an offset (terminal output position, event
  sequence, transfer byte offset). After a reconnect, the client resumes each
  stream from its last offset and gets exactly what it missed, while that is
  still kept. A terminal behind its kept scrollback gets a fresh screen and a
  count of the lines not kept. An event stream behind the kept events gets
  the current state of everything it watches, then new events.
- **TR-5** No port is opened on any machine besides SSH.
- **TR-6** A slow or unreachable machine never stalls the app or other
  machines' streams: with one machine not answering, the app keeps
  responding to input and other machines' terminals keep updating at their
  usual rate.
- **TR-7** Nothing polls across a link. Every change a peer needs (terminal
  output, events, Inbox requests and answers, machine and session state,
  pointer reports, transfer progress) is pushed on a stream the moment it
  happens. Inside a machine, changes Humata doesn't make itself (agent
  conversation files, clones' git state, `shared/`) are watched with the
  OS's file notifications (FSEvents, inotify). A poll is allowed only where
  no notification exists; each one is listed in `api/GRPC.md` with its
  interval and backs off while nothing changes.
- **TR-8** Link work never runs on the UI thread or blocks a request:
  decoding, decompression, diffing and highlighting run on background
  tasks, and the UI receives state ready to draw. Queues between them are
  bounded; output and events keep their order, while state that only
  matters at its latest value (progress, presence, scroll) is coalesced.
- **TR-9** Idle costs nothing. With every machine connected and nothing
  happening, the app and the daemons wake only for each link's keepalive
  (an HTTP/2 PING at most every 30 s), which is also how a dead link is
  noticed; no status is polled. Tests count wakeups and timers on a fake
  clock; release benches measure idle CPU.
- **TR-10** A link adds little over the network itself: a keystroke in a
  remote terminal echoes within the SSH round trip plus 5 ms, and an event
  on a machine reaches the app within the round trip plus 10 ms, measured
  in release benches against a local VM.
- **TR-11** Anything that can be large (file transfers, file reads,
  diffs, terminal snapshots, scrollback catch-up, bug reports) moves in
  chunks (1 MiB at most) and is compressed per chunk. The codecs are **lz4**
  (fast, the default on the local socket and on fast links) and **zstd** (smaller,
  the default on slow links), plus none; each chunk names its codec, both
  ends say which they support when the link starts, and data that doesn't
  shrink (images, archives) is sent as is. Settings › Advanced picks
  Auto, lz4, zstd or Off, globally and per machine. (D56)
- **TR-12** Inside a machine, only files that matter are watched: each
  live agent's own conversation file, and each clone's `HEAD`, index and
  current branch ref. Working-tree changes are read when they're needed (at
  the end of an agent's turn, when a Diff panel is showing, before a
  pointer report), not watched recursively.
- **TR-13** Every compressed payload names its codec, on the wire and at
  rest, as a number, never a name: chunk messages carry a `codec` enum
  (one varint byte), and every stored compressed
  blob (store rows, content-addressed blob files, snapshots, scrollback,
  restart and bug reports) starts with a small binary header: magic, a
  one-byte codec id (0 none, 1 lz4, 2 zstd; ids are never reused), a
  numeric message type and the format version, so any reader can decode it without
  context and the codec can change without a migration. A reader that
  meets a codec it doesn't know says which codec and which file or stream,
  and refuses rather than guessing. (D60)

## 22. Navigation and UI principles

- **NV-2** The sidebar is the only navigation column; there is no icon rail
  (D76). Top to bottom (frames 01–43): **Search** (⌘K); a **Pinned** section
  when anything is pinned (NV-21); **Home**, **Inbox** (pending count),
  **Board** (the Global board) and **Desk** with its open sessions (DK-1);
  **Channels**, with **+** to make one, each channel folding over its session
  tree (CH-16); **Hosts**, each machine with its dot, marked offline when it
  is; and **Settings** at the bottom.
- **NV-3** A **Back** button appears only after drilling in from a page
  (e.g. opening a session from a channel table, an Inbox card or an item). It
  is labelled with where it goes ("‹ #payments-rewrite").
- **NV-4** Lateral moves get no Back and clear the trail: the sidebar, tabs,
  breadcrumbs, the palette.
- **NV-5** ⌘[ goes back (same as the Back button). Escape closes the dialog,
  then the drawer, in that order.
- **NV-6** Pages that sit inside others show breadcrumbs (channel › parent
  session › session; Desk › session; Hosts › machine).
- **NV-7** Every screen offers at least one action on what it shows. A list
  or table of state is shown only where each row opens something or can be
  acted on, or where NV-8 lists it as kept on purpose.
- **NV-8** Humata does not detect or show what the user can't act on from
  that spot, and shows a problem only where it blocks an action. Kept on
  purpose: the Hosts table and machine card, the Bridge card, clone counts,
  the "Allowed without asking" log, Home's Working and Recent groups, the
  Activity tab, the Board dashboard (every figure opens its items, BD-13),
  Info's "Survives restarts", Settings' context-fork column and the Started
  column.
- **NV-9** Problems that concern a session are shown on that session, not in a
  separate list.
- **NV-10** Short confirmations appear as a toast at the bottom, sometimes
  with one action ("Show Home", "Undo").
- **NV-11** Retired (D128): a fresh install opens on Home, whose Get set
  up card (DK-6) does the welcome's job; frame 33 is gone.
- **DK-6** With no channel, Home shows **Get set up** (make a channel, add a
  machine, use the Desk) under its empty Needs you and Working groups, with
  **New desk session** in its header, and the sidebar shows "No channels
  yet" with **New channel…** and This Mac with "Add a machine…" (frame 34).
  A channel with no sessions shows its own checklist: add a repo, pick
  machines, start a session (frame 35).

### Resizable and collapsible columns

```text
 expanded                                        left sidebar collapsed
┌──────────┬┬──────────────────────┬┬─────────┐  ┌─┬──────────────────────┬┬─────────┐
│ Sidebar  ││ terminal / editor    ││ Diff  ⇤✕│  │›│ terminal / editor    ││ Diff  ⇤✕│
│ [«]      ││                      ││ [»]     │  │ │                      ││ [»]     │
│ # pay…   ││                      ││         │  │ │                      ││         │
│  ├─ ●    ◀▶ drag divider         ◀▶ drag    │  │ │                      ◀▶        │
│  └─ ●    ││                      ││         │  │ │                      ││         │
└──────────┴┴──────────────────────┴┴─────────┘  └─┴──────────────────────┴┴─────────┘
   «  collapses the column        thin strip (›) opens it again at its last width
```

- **NV-12** Every sidebar and panel can be resized by dragging its divider:
  the left sidebar, the session's side panel (Diff, Files, Inbox, Board,
  Journal, Info), the two tab groups (SE-27), the editor's changes list, the
  Files panel's explorer side when the viewer is beside it, and each pane of
  the file manager.
- **NV-13** Each of those columns has a collapse button. A collapsed column
  leaves a thin strip; clicking the strip opens it again at the width it had
  before it was collapsed.
- **NV-14** A divider stops at a minimum width for each column and never lets
  the terminal or the main content shrink below its own minimum; dragging
  past a column's minimum collapses it.
- **NV-15** Double-clicking a divider returns that column to its default
  width.
- **NV-16** Column widths and collapsed states persist across app restarts:
  the left sidebar and the file manager panes globally, the session's
  panels, tab groups and editor list per session (with the rest of SE-13).
- **NV-17** When the window is too narrow for every open column at its saved
  width, columns shrink toward their minimums without changing the saved
  widths; widening the window brings them back.

### Expand all / Collapse all

- **NV-18** Every tree or grouped list has **Expand all** and **Collapse all**
  on its header and in the palette: the sidebar's channels and fork trees,
  the diff's repo groups and files, file trees, Inbox groups and the Board
  table's groups.
- **NV-19** ⌥⌘→ expands all and ⌥⌘← collapses all in the tree or list that
  has focus. The result is kept like any other fold state (SE-13).

### Pinned and VIP

```text
┌ Sidebar ─────────────────────┐
│ PINNED                       │
│ ● Ledger schema migration    │
│     needs input · 2m         │
│ # pr-reviews › acme/api #482 │
│     3 new commits            │
│ ◈ gpu-box                    │
│ ──────────────────────────── │
│ Home   Inbox 7               │
│ # payments-rewrite           │
│  ├─ [VIP] API rate limiter ✉ │
└──────────────────────────────┘
```

- **NV-20** The user can pin and mark VIP sessions, channels, machines, PR
  reviews and board items, from ⋯, the palette, or a shortcut. Agents can't
  set or clear either.
- **NV-21** Pinned items appear in a **Pinned** section at the top of the
  sidebar (unless Settings › **Sidebar shows Pinned** is off, ST-18) and of
  Home, as well as in their usual places. A pinned item with an
  event (needs input, finished, failed, new PR commits) shows that event
  there.
- **NV-22** A VIP item that needs the user shows a **VIP** badge everywhere
  it appears (Inbox, sidebar, notifications, palette), and its requests sort
  first in the Inbox.
- **NV-23** VIP notifications break through quiet hours only when the user
  allows it in Settings.
- **NV-24** Pins and VIP marks are kept on the host. The app and `humata
  pinned` show the same items in the same order.
- **NV-25** Pinned items keep the order the user gives them, by dragging in
  the Pinned section or from the palette. A newly pinned item goes at the
  end.
- **NV-26** A thing that is deleted or forgotten loses its pin and VIP mark.
  Archiving keeps both.

### Confirmations

- **NV-27** A confirmation acts only on the list it showed. If what the action
  would touch changed while the dialog was open (a session started, a
  commit appeared, an item was added), confirming does nothing; the dialog
  shows the new list and asks again. This holds for archiving, deleting,
  dropping a repo from a session, removing a repo or machine from a channel,
  forgetting a machine, emptying the trash, changing a Humata root and
  submitting a review.

### The window and quiet chrome (owner directions 2026-11-15: D76, D74 as revised)

The pane workspace of D74 is gone: NV-28–NV-31a and NV-35 were dropped, and
NV-32–NV-34 keep what still holds.

- **NV-32** The chrome stays quiet. There is no status bar of text at the
  top of the window and no button that repeats something the screen or the
  palette already offers. A session's state is its mark (§1), and its header
  adds the words only when the session needs attention (SE-6). Other actions
  live in ⋯ menus, right-click menus and ⌘K.
- **NV-33** Dialogs and pop-ups are kept for what needs a decision (a
  destructive action, a form to fill). Anything else opens as a tab, a panel
  or a toast, so the user is never stacked in modals.
- **NV-34** What is hidden costs nothing: a tab or panel that isn't visible
  renders nothing, and a hidden terminal keeps only its byte stream until it
  is shown, so ten open tabs cost no more than the visible ones.
- **NV-36** The window is three columns: the sidebar, the screen in the
  middle and, on a session, its side panel with the strip that picks it
  (SE-11). There is no pane tree, no Panes sidebar and no layout setting.
  The only split is a session's tab row, into two tab groups (SE-27); more
  than that is a fork.
- **NV-37** On a session, the terminal or the side panel can each fill the
  window's middle (frames 10–12): **Terminal full screen** (⇧⌘↩) hides the
  sidebar and the panel, and the panel's **Full screen** covers the terminal
  (SE-12). ⌥⌘0 hides and shows the side panel. Both states are kept per
  session (SE-13). ⌘W closes the active tab and never stops a program (D73).

## 23. Home, global Inbox, Global board, palette

- **HM-1** Home lists what needs the user first (frame 25). **Needs you · n**:
  every session that needs input, exited or is offline, with its channel
  (and machine when it is not This Mac), why, and one button: **Review** for
  a request, **Resume** for an exited agent, **Open** otherwise. Then
  **Working · n**: the sessions whose agents are working, each with the last
  step it reported; a row opens the session.
- **HM-2** Home ends with **Recent**: the latest events from every channel's
  Activity, each with a button where one applies (**Open PR**, **Review**).
  Home's header has **New session**, which asks for the channel first; with
  no channel it is **New desk session** (DK-6).
- **HM-3** The global Inbox (IN-1) shows "<n> waiting" beside its title.
- **HM-4** The Global board is described in BD-7.
- **HM-5** ⌘K opens the command palette (KP-1 to KP-6).

### Command palette (⌘K)

```text
┌ ⌘K ───────────────────────────────────────────────────────────────────────┐
│ move                                                                      │
├───────────────────────────────────────────────────────────────────────────┤
│ THIS SESSION · API rate limiter                                           │
│ ▸ Move session to another machine…                                  ⌥⌘M   │
│   Fork…                                                             ⌘⇧F   │
│ RECENT                                                                    │
│   # payments-rewrite › Ledger schema migration                            │
│ EVERYWHERE                                                                │
│   Files: move…                                                            │
│   Settings › Long lines                                                   │
├───────────────────────────────────────────────────────────────────────────┤
│ Move session › pick a machine:  [This Mac]  [gpu-box]  [lima-dev]         │
└───────────────────────────────────────────────────────────────────────────┘
```

- **KP-1** Every action in the app is reachable from the palette: commands,
  navigation to channels, sessions, machines, files and views, and every
  Settings entry.
- **KP-2** Search is fuzzy; recent items come first, and each command shows
  its shortcut when it has one.
- **KP-3** It is context-aware: actions for the current session or machine
  rank first.
- **KP-4** Pasting a PR URL offers **Review this PR** (PR-1). Pasting a path or
  a repo (URL or `owner/repo`) offers to open it: the file, the session that
  has it, or Add repo (CH-18).
- **KP-5** Commands that need arguments ask for them inline in the palette
  (e.g. Move session → pick a machine), then run.
- **KP-6** ⌘K opens it everywhere, including from inside a terminal tab, where
  ⌘K is taken by Humata and not sent to the terminal. Agents can't open it or
  run its commands.

## 24. Settings

- **ST-1** **Agents**: each agent with its launch command, whether it supports
  context forks, and **Edit** (launch command and its variables). The page
  explains that a context fork needs the agent to copy its conversation under
  a new id, and that agents that can't offer only code-only forks. Each
  agent also shows where its turn ends come from (SE-21): its hooks, or
  watching its output.
- **ST-2** The user can add an agent (name, command, resume and fork
  arguments).
- **ST-3** **Terminal**: font, theme (follows the system by default),
  scrollback length (kept on each machine).
- **ST-4** **Default access for new channels**: the same rules as a channel's
  Agent access tab, applied when a channel is created.
- **ST-5** **Machines**: per machine, the Humata root folder (FC-10) and the
  VM tier switch (PER-8).
- **ST-6** **While I'm away** default for new channels (AW-4).
- **ST-7** **Long lines: Wrap / Scroll**, Wrap by default. It applies to code
  in the Diff panel, the editor tab (all three modes), the Files viewer and
  review views. The setting persists across restarts and changing it takes
  effect at once in every open view, keeping the line at the top of each view
  in place.

```text
 Long lines: Wrap (default)                  Long lines: Scroll
┌────┬────────────────────────────────┐    ┌────┬────────────────────────────────┐
│ 40 │+ let balance = ledger.balance( │    │ 40 │+ let balance = ledger.balance(a│
│    │    account_id, Currency::Usd)? │    │ 41 │  Ok(balance.cents)             │
│ 41 │  Ok(balance.cents)             │    │    │◀━━━━━━━━━━━━━━━━━━━━▶ scroll   │
└────┴────────────────────────────────┘    └────┴────────────────────────────────┘
  number on the first visual line;            one visual line per line; a
  no horizontal scroll bar                    horizontal scroll bar
```

- **ST-8** In Wrap mode, long lines wrap at the view's width with no
  horizontal scroll bar; continuation lines are indented to the line's own
  indentation; the line number, +/− mark and comment target stay on the
  first visual line, and the gutter stays aligned with it.
- **ST-9** In Wrap mode, side by side keeps the paired old and new lines
  starting on the same row: the shorter side is padded so the next pair lines
  up.
- **ST-10** Resizing a view in Wrap mode rewraps the lines without moving the
  line at the top of the view.
- **ST-11** In Scroll mode each line is one visual line and the view scrolls
  horizontally; in side by side both sides scroll together.
- **ST-12** The terminal is not affected: it wraps as the agent CLI draws it.
- **ST-13** **Branch prefix** (e.g. `uhvesta/`), default the `gh` login
  followed by `/`; can be empty; overridable per channel (FC-12).
- **ST-14** **Retention** defaults for new channels, all off (LD-16).
- **ST-15** **Default agent**, any built-in or user-added agent, with an
  optional per-channel override. New session, fork and PR review preselect
  it. Humata sets no model: each agent starts with its own configured
  defaults (SE-2).
- **ST-16** **Sync skills to machines automatically** and skill exclude
  patterns (SK-3, SK-6).
- **ST-17** **Security**: **Contain agents** and its per-channel override
  (SEC-6), the approval passphrase (SEC-13), and CLI logins and script
  tokens, each with **Revoke** (SEC-14).
- **ST-18** Settings is one page with its sections listed on the left (frame
  29): **General**, **Agents**, **Machines**, **Layout**, **Agent access**,
  **Storage**, **Notifications**, **Keyboard**. Layout holds **Appearance**
  (System, Light or Dark; it follows the system unless the user picks one),
  **Terminal font** (used by every terminal and the diff) with the rest of
  ST-3, **Sidebar shows Pinned** and Long lines (ST-7). General holds the
  branch prefix (ST-13). Agents holds ST-1, ST-2, ST-15 and ST-16. Machines
  holds ST-5, machine providers (MP-2), compression (TR-11) and web serving
  and pairings (AR-8, AR-9). Agent access holds ST-4, ST-6 and Security
  (ST-17). Storage holds ST-14. Frame 29 also draws a **Panes** choice
  (Fixed, Simple, Free) under Layout; D76 dropped it with the pane
  workspace, and it is to be removed from the frame. (provisional, D111)
- **ST-19** **Notifications**: which events notify (a session needs you, a
  turn finished, an agent exited, a machine went offline), quiet hours, and
  whether VIP items break through them (NV-23). (provisional, D111)
- **ST-20** **Keyboard** lists every command with its shortcut. Each can be
  changed or cleared, and a shortcut already in use is refused, naming the
  command that has it. (provisional, D111)

## 25. Security and secrets

- **SEC-1** Values of host variables are never sent to clients and never
  written to project files; only their names cross. They reach the agents on
  their machine as environment variables (HO-13). Before a transcript is
  mirrored to the host, exact values are replaced (LD-21).
- **SEC-2** The GitHub token never leaves the host. Remote clones without `gh`
  use bundles and pushes through the host (FC-7).
- **SEC-3** Agents get only what the rules allow for calls made through
  Humata (SEC-5); a context-only fork can never write its parent's files
  through Humata.
- **SEC-4** The local socket is readable only by the user.
- **SEC-5** An agent that is not contained runs as the user and can read and
  write anything the user can on its machine, and open network connections,
  without asking Humata. The rules don't stop that. The Agent access tab and
  each session's Info panel say so in these words for uncontained sessions.
  (provisional, D1)
- **SEC-6** **Contain agents** is off by default and can be turned on in
  Settings, with a per-channel override. A contained agent can write only
  inside its session folder and `shared/`. Its reads of other sessions'
  folders, Humata's own folders, `~/.ssh` and the `gh` config fail. Its only
  network connection is to its daemon's socket. A change applies when a
  session next launches or resumes. (provisional, D1)
- **SEC-7** Some actions are the user's only, and no rule or grant gives
  them to an agent: answering Inbox requests; changing access rules, grants,
  the passphrase, Settings, agents and providers; pins and VIP; submitting a
  review or letting an agent push (PR-8); marking files reviewed and sending
  comments; emptying the trash; setting host variables; changing a machine's
  root or VM tier; opening a shell or attaching with input; and deleting a
  channel. An agent that tries one is refused, told the action is the
  user's, and pointed at asking the user (IN-25).
- **SEC-8** A `humata` command run inside a session's process tree acts as
  that session's agent, with that agent's rules, wherever it was typed.
  (provisional, D2)
- **SEC-9** A process with no session token, script token, CLI login or
  app key is refused everything except asking who it is (`humata whoami`)
  and logging in (SEC-10). There is no unverified read tier. Only a test
  daemon built in admin mode takes such a process for the user; release
  builds can't. (provisional, D2)
- **SEC-10** The user shows presence in one of three ways: confirming in the
  app (a dialog naming the command, with **Allow** and **Don't Allow**);
  `humata login`, which lasts up to 12 hours or until `humata logout`; or the
  approval passphrase while the host is away (SEC-13). Typing "yes" at a
  prompt never counts. (provisional, D2)
- **SEC-11** A login covers only the terminal session it was made in. A
  process that leaves that session (for example with `setsid`) is not
  covered.
- **SEC-12** A presence prompt shown inside a session's own terminal is
  refused: an agent could answer it.
- **SEC-13** The approval passphrase is set in Settings › Security. It has at
  least 5 random words. Five wrong tries lock it for 15 minutes on that
  machine. The Settings page explains how to answer over SSH with the host
  closed: `humata login` with the passphrase, then `humata inbox`.
- **SEC-14** Scripts use tokens the user makes in Settings › Security. A token
  lists what it may do, which channels and when it expires. Tokens can be
  listed and revoked. A token can never do a user-only action.
- **SEC-15** No tool takes a caller's identity as an argument: an agent is
  always the session it was launched in (a session's added agent has no
  identity at all, SE-29). If a session's identity is used
  from outside that session's process tree, the call is refused, the
  identity is replaced at the session's next resume, and the Activity tab
  records it.

## 26. UX flows

### 26.1 Create an all-remote channel with repos and machines

1. The user clicks **+** next to Channels.
2. Types the name `ml-training`; the folder shows `~/Humata/ml-training`.
3. Picks machines: gpu-box and devbox-1 (This Mac unticked).
4. Searches repos, ticks `acme/ml-pipelines` and `acme/infra`.
5. Clicks **Create channel and clone 2 repos**.
6. Humata creates `~/Humata/ml-training/{main,sessions,shared}` on both
   machines and clones both repos into `main/` on each: with `gh` where it is
   signed in, by bundle from the Mac where not. Progress is shown per
   machine.
7. The channel opens on its Sessions tab, empty, with **New session**.

### 26.2 Start a session on a machine

1. In `#payments-rewrite`, the user clicks **New session**.
2. Types the title, picks Claude Code and devbox-1.
3. Keeps "Own clone, new branch from main" for `acme/api`, leaves `acme/ledger`
   out.
4. Types a first message, clicks **Start session**.
5. devbox-1's daemon writes the session record, clones `acme/api` into
   `sessions/<slug>/api` on `uhvesta/<slug>`, launches `claude --session-id <uuid>`
   in a holder, and types the first message.
6. The session page opens with the agent's terminal live; Activity records
   the start.

### 26.3 Review a 3-repo diff and send comments

1. In a session with `api`, `ledger` and `web`, the user opens **Diff**.
2. Picks "Since I last reviewed". The panel shows three repo groups, each with
   branch and totals.
3. Makes the panel full screen to read.
4. Clicks line 40 in `api/src/ledger/client.rs`, writes a comment and leaves
   it as a draft. Repeats on `ledger/…0042_accounts.sql` and `web/…/Cards.tsx`.
5. Opens `client.rs` big, switches to Side by side, uses ↓ to step
   through changes, adds a fourth comment there.
6. Clicks **Send 4 comments**. The agent's terminal gets one message with a
   `[comment on repo/path:line]` line and the text for each (SM-9).
7. Clicks **Mark reviewed**.
8. Later, pushes `api` and opens its PR from the api group, leaving the other
   repos for later.

### 26.4 Approve an Inbox request after opening the requester

1. The sidebar shows Inbox 7. The user opens the global Inbox.
2. Selects "Read files of “Ledger schema migration” on This Mac" from "API
   rate limiter" (devbox-1).
3. Reads the reason and the two paths; clicks a path to see the file in that
   session.
4. Clicks **Open session ↗** on the card. "API rate limiter" opens with its
   Inbox panel and the request selected, beside its terminal.
5. Reads the terminal: the test failing on `accounts.balance_cents`.
6. Clicks **Allow**. The agent's waiting call returns the files and the
   agent continues; nothing is typed into its terminal (IN-12). Counts drop everywhere.

### 26.5 Agent forks itself onto a GPU machine

```text
 Benchmark session (devbox-1)        This Mac (bridge)            gpu-box
 ─────────────────────────────      ──────────────────          ─────────────
 session_fork(gpu-box) ──────────▶  check rules: Ask me
                                    Inbox card: agent, machine,
                                    kind, repos, size, usage
                                    user: Approve
           ◀── pause point ─────────
 branch + uncommitted files  ─────▶ relay (stream, offsets) ──▶ session clone
 conversation + records      ─────▶ relay ───────────────────▶ path-keyed
                                                                records fixed
                                    write session record ────▶ fsync, launch
                                                                holder + agent
                                                                --resume <copy>
           ◀── fork id, status ───────────────────────────────  running
```

1. The agent in "Benchmark BRIN vs btree" on devbox-1 calls the fork tool for
   gpu-box, code + context.
2. The Mac checks the rule "Start or fork sessions on another machine": Ask
   me. A card appears with agent, machine, kind, repos, the size to copy and
   a usage note.
3. The user approves.
4. The branch and uncommitted files stream from devbox-1 through the Mac into
   new session clones on gpu-box; the conversation is copied and its path-keyed
   records fixed.
5. gpu-box writes the session record, starts a holder and resumes the copied
   conversation.
6. The fork appears under its parent in the tree on gpu-box; the parent's
   call returns the fork's id.

### 26.6 Move a session between machines

1. The user opens ⋯ → **Move to another machine…** on "API rate limiter"
   (devbox-1) and picks This Mac.
2. Humata pauses the agent and shows "Moving to This Mac…" with steps.
3. Copies the branch, uncommitted files and conversation; fixes the agent's
   path-keyed records.
4. Resumes the agent on This Mac and checks it runs.
5. Deletes the copy on devbox-1. The session keeps its id, tree place, board,
   requests and journal.
6. If any step fails, the session is still running on devbox-1 and Humata
   says which step failed.

### 26.7 Transfer a folder Mac → remote, recover after an SSH drop

1. The user opens the file manager: This Mac `~/Downloads/fixtures` on the
   left, gpu-box `~/Humata/ml-training/shared` on the right.
2. Drags `datasets/` from Finder onto the right pane.
3. The tray shows the transfer, checksummed, in chunks.
4. At 1.4 of 2.1 GB the SSH link drops. The transfer shows "paused: link
   down".
5. Humata reconnects with backoff; the transfer resumes from the last
   confirmed offset.
6. When done, every file's checksum matches; the tray shows it complete.

### 26.8 Mac reboot

1. The Mac reboots with sessions running locally and on devbox-1.
2. At login, launchd starts `humatad`. It takes the lock, reads the session
   records, finds the local holders gone, and resumes each running agent with
   its pinned conversation id; plain shells come back in their folders.
3. It verifies each resume and tells each agent it was interrupted and to
   check `git status`.
4. The app opens; devbox-1's sessions were never interrupted and reattach with
   a screen snapshot and their output meanwhile.
5. A toast says how many local sessions were started again and that mid-turn
   agents are idle, with **Show Home**.
6. Any session that could not resume shows "Couldn't resume" with Try again
   and Start fresh with a summary.

### 26.9 Remote machine power loss and resume

1. devbox-2 loses power. Its sessions show "devbox-2 is offline";
   Hosts shows it offline.
2. Power returns. The systemd user service (linger) starts `humatad` at boot.
3. Boot reconciliation resumes every session marked running, verifies each,
   and tells the agents they were interrupted.
4. The Mac reconnects, catches up the terminals and shows "devbox-2 is back.
   n sessions reattached."
5. Running programs in plain shells are reported lost.

### 26.10 Working away from the Mac

1. The Mac is closed. Sessions on devbox-1 keep working.
2. The user SSHes to devbox-1 from another computer, runs `humata login` and
   enters the approval passphrase (SEC-10, SEC-13), then runs `humata ls`
   and `humata attach api-rate-limiter`.
3. They type into the agent's terminal, then detach; the session keeps running.
4. An agent asks to read another session's files; the channel's While I'm away
   rule is Wait, so the request waits in devbox-1's inbox.
5. The user runs `humata inbox`, reads the request and approves it.
6. When the Mac reconnects, the inbox merges; the approved request shows as
   answered.

### 26.11 Archiving with unpushed work

1. On "Flaky test hunt", the user opens ⋯ → **Archive session…**.
2. The dialog says the transcript and scrollback are kept, the agent stops and
   the clones are deleted, and warns: "3 uncommitted files on uhvesta/flaky-test-hunt,
   never pushed. Archiving deletes that work."
3. The user clicks **Commit, push and open a PR, then archive**.
4. Humata commits, pushes, opens a draft PR with `gh` through the Mac, then
   archives and goes to the Sessions tab.

### 26.12 Opening an agent-prepared review, then Revert

1. The user asks the agent in "Ledger schema migration" to prepare the PR
   review.
2. The agent calls `view_prepare` and marks the view ready. An offer bar
   appears: "PR review: ledger schema" with its summary.
3. The user clicks **Open it**. A review tab opens: step 1
   `0042_accounts.sql` with the agent's note; a bar says "Showing “PR review:
   ledger schema”" with Revert and Keep.
4. The user steps with **Next ›** to step 2, `client.rs`, side by side, and
   adds a comment.
5. The user clicks **Revert**. The diff panel and terminal tab they had before
   come back exactly; the comment stays queued.
6. The view is in the Journal as "Reverted" and can be opened again.

### 26.13 Resize and collapse columns

1. On a session page, the user drags the divider between the terminal and the
   Diff panel to the left; the panel widens and the terminal narrows.
2. They click the left sidebar's collapse button; it leaves a thin strip and
   the terminal takes the space.
3. In the editor tab, they drag the changes list narrower, then double-click
   its divider; it returns to its default width.
4. They quit and reopen the app. The sidebar is still collapsed and the Diff
   panel has the width they set.
5. They click the sidebar's strip; it opens at the width it had before.

### 26.14 Switch long lines between Wrap and Scroll

1. In the Diff panel, a long line in `client.rs` wraps over two visual lines;
   its line number and + mark sit on the first, and there is no horizontal
   scroll bar.
2. The user widens the panel; the line rewraps to one visual line and the line
   at the top of the view stays put.
3. In Settings they set **Long lines** to **Scroll**. The open diff and editor
   switch at once: one visual line per line, with a horizontal scroll bar.
4. They restart the app; Scroll is still set.
5. They set it back to **Wrap**.

### 26.15 Copy a folder between two remote machines

1. In the file manager the user picks devbox-1 in the left pane and gpu-box in
   the right.
2. Selects `~/Humata/ml-training/shared/weights/` on devbox-1 and clicks
   **Copy →**.
3. The tray shows devbox-1 → gpu-box via This Mac.
4. gpu-box goes offline; the transfer pauses and the tray says it waits for
   gpu-box. The right pane shows its last listing and disables actions.
5. gpu-box comes back; the transfer continues from the last confirmed offset
   and completes with checksums matching.

### 26.16 Add a cloud VM with a provider, then a login-gated one

1. In Hosts, the user clicks **Add machine…**, picks Cloud VM (hcloud), names
   it `ci-box-1` and keeps type cx22.
2. `create` runs with its output streamed; `identity` prints the address and
   host key, which Humata pins in its own known_hosts.
3. Humata connects with retry while the VM boots, checks it, installs
   `humatad`, sets up the systemd user service with linger, and registers it
   in `hosts.json`. `ci-box-1` is a normal machine.
4. They fork "Ledger schema migration" onto it; the branch and uncommitted
   files stream over the bridge and the agent resumes there.
5. Next they add `corp-vm` with a provider whose connect asks for a
   verification code. The non-interactive try prints the prompt, so Humata
   offers **Open interactively**; the user types the code in the dialog's
   terminal.
6. Humata takes over the stream and the daemon starts. The dialog offers to
   make it unattended; they add a Humata key and the machine switches to
   the non-interactive form.
7. Later, **Delete** on `ci-box-1` asks to move or archive its sessions, then
   runs `hcloud server delete`.

### 26.17 Review a pull request

1. The user pastes `https://github.com/acme/api/pull/482` into Search (⌘K).
2. **Review a pull request** opens with the title, author, size and checks
   fetched with `gh`. They add `acme/ledger#91` under Also review, pick
   devbox-1 and Claude Code, and type some context.
3. **Start the review** clones both heads into
   `~/Humata/pr-reviews/sessions/…` on devbox-1 and starts the review chat
   with the PRs and the context. The agent waits.
4. The user asks "where does a partial capture get refunded?". The agent
   focuses `refund.rs:140`, highlights the two callers and leaves a note on
   the rounding line. The user asks it to order the files bottom-up; it does,
   and the user undoes one move.
5. The user turns the agent's note into a draft, edits it, and adds one of
   their own as Draft for GitHub.
6. The author pushes 2 commits. The session shows "2 new commits, update the
   clone"; the user clicks it and the agent is told which files changed.
7. The user opens **Submit review**, picks Request changes for #482 and
   Comment for #91, and submits. Both reviews post through the Mac's `gh`.
8. Days later #482 merges; its session offers **Archive**, and the user
   archives it.

### 26.18 Forget a lost machine and free disk elsewhere

1. A cloud box, devbox-2, was deleted outside Humata. Its sessions show
   "devbox-2 is offline".
2. On its page the user picks **Forget machine**. The dialog lists its 3
   sessions and, for one, "2 unpushed commits on uhvesta/cache-warmup, lost".
3. They confirm. The 3 sessions appear under Archived with their
   conversations and diff summaries from the mirror.
4. They restore one on devbox-1: its branch is cloned again from GitHub and
   the agent resumes its conversation.
5. Forking onto devbox-1 next, the fork dialog says it needs 14 GB and 6 GB
   are free, and lists devbox-1's largest items. The user clears an old build
   cache in place and the fork proceeds.

### 26.19 Branch names from the work

1. In Settings the user sets the branch prefix to `uhvesta/`.
2. They start a session titled "Fix refund rounding"; the dialog shows the
   branch `uhvesta/fix-refund-rounding`, and they keep it.
3. They start a second session with the same title in the same channel; the
   dialog shows `uhvesta/fix-refund-rounding-2`.
4. They rename the first session before pushing and tick "Rename the branch
   too".
5. They push and open a PR; its branch name carries no session id or
   machine.

### 26.20 Iterate on a document with the agent

1. On the Desk, the user starts "BRIN rollout RFC" and asks the agent to
   write `rfc-brin.md`.
2. The agent writes it. The user opens it from Files as a tab, drags the tab
   into a second tab group beside the terminal and reads it in Preview, with
   its table and ASCII diagram.
3. They switch to Side by side, click line 7 in the source, write "Where are
   the sizes from?" and press **Comment**. The agent is mid-turn, so the
   comment shows queued. When the turn ends it is typed as `[comment on
   rfc-brin.md:7]` and its text, and shows as sent.
4. They write two more comments and leave them as drafts, then click **Send
   2 comments**. Both arrive as one paste.
5. The agent answers in its terminal and rewrites the proposal. One comment
   becomes addressed and shows its lines before and after; another's lines
   are gone, and it offers Re-attach or Drop.
6. The user resolves the addressed one, re-attaches the other to the new
   paragraph and sends it again.

### 26.21 Add a repo to a channel, then to one session

1. On `#payments-rewrite` › Repos and machines, the user clicks **Add a
   GitHub repo…** and pastes `https://github.com/acme/web`.
2. The `acme/web` row fills in per machine as main clones finish; gpu-box is
   offline and its cell says it waits for gpu-box.
3. Existing sessions don't change. In "API rate limiter" the user picks **Add
   repo to this session** › `acme/web`.
4. Humata clones it on the session's machine on `uhvesta/api-rate-limiter`,
   tells the agent the repo and its path, and a `web` group appears in Diff
   and Files.
5. gpu-box comes back and clones `acme/web`.
6. Later the user removes `acme/ledger` from the channel. The dialog lists 2
   sessions using it; they drop it from one (no unpushed work) and archive
   the other. The repo is removed.

### 26.22 Act from the palette

1. In the `claude` terminal tab of "API rate limiter", the user presses ⌘K;
   the palette opens over the terminal and the keystroke doesn't reach the
   agent.
2. They type "move"; Move session to another machine… is first, as it belongs
   to the current session, with its shortcut shown.
3. Return asks inline for a machine; they pick gpu-box and the move starts
   (MV-2).
4. They press ⌘K again and paste a PR URL; Review this PR opens the dialog
   with the PR filled in.
5. They press ⌘K, type "long" and open Settings › Long lines.

### 26.23 A machine dies for good

1. devbox-2's disk fails. Its sessions show "devbox-2 is offline".
2. Days later the user gives up on it and picks **Forget machine**. The dialog
   lists its 3 sessions from the host's records and, from the last pointers,
   what is lost: "uhvesta/cache-warmup: 2 unpushed commits, 4 dirty files".
3. They confirm. The sessions become archived, with records and conversation
   mirrors intact.
4. They pick **Rebuild on another machine** for one and choose devbox-1.
   Humata re-clones each repo at its branch (the last pushed sha for the one
   with unpushed commits), resumes the conversation from the mirror, and
   tells the agent which commits and files were lost.
5. A board edit an agent made on devbox-2 while the Mac was asleep had
   reached the host from its journal before the disk failed; it is there.

## 27. Non-goals

- A chat layer, message queue, rewind, compaction or image handling: the
  agent CLI does these in its terminal.
- Teams, sharing, multiple users.
- A GitHub OAuth app or sign-in inside Humata.
- Windows machines.
- Direct machine-to-machine links (for now).
- Status that the user can't act on: commits behind main, shared-clone
  notices, standing unpushed warnings, per-session latency or disk.
- Recovering running programs after a reboot without the VM tier.
- A general pane workspace: the window is the sidebar, the screen and one
  side panel, and a session splits into two tab groups at most (D76).
- Choosing an agent's model: the agent's own TUI does that (SE-2).

## 28. The prototype

The clickable prototype (`apps/humata2/prototype`) and `ui-prototype` were the
first sketches of Humata 2. They are no longer a reference for how it looks or
behaves: the design goldens and this spec replaced them, and where either
still differs, it is the one that is out of date. The list of differences
that used to be here is gone with it.

## 29. Open questions

- **q1** Channel feed: events only, or can the user also post notes into it?
- **q2** Answered: comments are pasted into the agent's terminal as one
  message (SM-9); the agent replies and resolves over MCP (DK-9, SM-19).
- **q3** Answered: context-only forks are read-only on their parent's files
  (FK-7, IN-20).
- **q4** Main clones: read-only reference only, or may a plain shell session
  work in them?
- **q5** Does a fork's session board start as a copy of its parent's, empty,
  or shared with it?
- **q6** The earlier planning-thread idea: drop it, or make "spec" a kind of
  board item?
- **q7** Across the bridge, what may cross (file reads, file writes, terminal
  input, MCP calls), and which always ask? Answered provisionally (D5):
  only Humata's own calls and file transfers, each under its rule; port
  forwards only on approval (HO-10, HO-11, HO-12).
- **q8** Confirm dropping Humata 1's chat features (queue, /rewind, /compact,
  images). Humata's own queue of messages to type at a turn end (SE-23) is a
  different thing and stays.
- **q9** Should Humata also show a structured view of tool calls beside the
  terminal?
- **q10** Answered: channels span many machines.

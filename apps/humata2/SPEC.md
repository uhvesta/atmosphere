# Humata 2: behavioural spec

As of Nov 14, 2026. This is the behavioural spec for Humata 2. It records what
the clickable prototype (`apps/humata2/prototype`) shows and the decisions the
owner approved on 2026-11-14. Where the two disagree, the decisions win. Section
28 lists those places.

Humata 2 is a native app for directing coding agents: Claude Code, Codex CLI,
Copilot CLI, OpenCode, and plain shells. The UI is written in Rust on GPUI, with
its state in the repo's `libs/tca-rust`. It runs on macOS and Linux. A Rust
daemon, `humatad`, runs on every machine Humata uses: your Mac and any machine
you reach over SSH. A session is the agent CLI's own terminal, held by
`humatad`, so it outlives the app, the SSH link and the daemon itself.

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
11. [Spec mode and comments](#11-spec-mode-and-comments)
12. [File manager and transfers](#12-file-manager-and-transfers)
13. [Boards](#13-boards)
14. [Inbox and agent access](#14-inbox-and-agent-access)
15. [Agent-prepared views and the Journal](#15-agent-prepared-views-and-the-journal)
16. [Agents' view of the topology (MCP tools)](#16-agents-view-of-the-topology-mcp-tools)
17. [Hosts and the bridge](#17-hosts-and-the-bridge)
18. [Machine providers](#18-machine-providers)
19. [Persistence and resume](#19-persistence-and-resume)
20. [Async: working while the Mac is away](#20-async-working-while-the-mac-is-away)
21. [Transport: gRPC streaming](#21-transport-grpc-streaming)
22. [Navigation and UI principles](#22-navigation-and-ui-principles)
23. [Home, global Inbox, Global board, palette](#23-home-global-inbox-global-board-palette)
24. [Settings](#24-settings)
25. [Security and secrets](#25-security-and-secrets)
26. [UX flows](#26-ux-flows)
27. [Non-goals](#27-non-goals)
28. [Where the prototype differs](#28-where-the-prototype-differs)
29. [Open questions](#29-open-questions)

## 1. Concepts

| Concept | What it is |
| --- | --- |
| Channel | A project. It has a name, a set of GitHub repos, a set of machines, a board, an Inbox, agent access rules and an activity feed. Shown like a chat channel (`#payments-rewrite`) but it holds sessions, not messages. |
| Machine (host) | This Mac, or any machine reached over SSH that runs `humatad`, whether the user owns it or a provider made it (§18). A channel can use any mix of machines and can be all-remote. |
| Main clone | One clone per repo per machine of a channel, on the default branch. A read-only reference that Humata fetches on its own. |
| Session clone | A session's own clone of a repo, on its own branch, named from the work (FC-11). |
| Shared folder | `~/Humata/<channel>/shared/` on each machine: files anyone in the channel shares. |
| Session | One agent CLI (or a plain shell) running in a terminal on one machine, with 0, 1 or more repos. |
| Holder | A tiny per-session process on the session's machine that owns its PTY and screen state. |
| PR review | A session in the PR reviews channel that reviews one or more pull requests; its comments are drafts posted to GitHub only when you submit. |
| Spec mode | A session where you and the agent iterate on Markdown files, with comments anchored to passages. |
| Archived session | A stopped session whose clones are deleted; conversation, record, scrollback and diff summary are kept and it can be restored. |
| Fork | A new session made from another one. Forks form a tree of any depth. Three kinds: code + context, code only, context only. |
| Board | Jira-style columns of items: Backlog, To do, In progress, In review, Done. There is a Global board, a board per channel and a board per session. |
| Inbox | Requests from agents for actions they don't own. Seen globally, per channel and per session. |
| Agent access rule | Per channel, per kind of action: Allow, Ask me, or Never. |
| Prepared view | A review walkthrough or board view an agent prepared for you over MCP. Opened only if you choose to. |
| Journal | Per session (with its forks): every view its agents prepared. |
| Machine journal | A machine's own append-only list of writes and requests made while the host is away (SL-5). Not the Journal of views. |
| Host | The machine whose `humatad` holds the source of truth (SL-1): your Mac by default, or a machine you host (AR-7). Where this spec says "the Mac", read "the host". "Away" means a machine has no link to it. |
| Bridge | The host. Every machine has one SSH link to it, and every call between machines goes through it. |
| Turn | From a message reaching the agent until the agent waits for input again. How Humata learns a turn ended is per agent (SE-21). |
| Own clones | The clones under a session's own folder (`sessions/<session>/`). A context-only fork has none until it gets its own copy (FK-9). |
| Grant | An "Always let …" permission for one session and one target (IN-14, IN-22). |
| Presence | Proof that the person, not an agent, is giving a command from the CLI (SEC-10). |
| Inactive | A session with no terminal output, no input and not opened by the user for a given time (LD-16). Not the Idle status. |
| Revision | A counter every editable thing carries; it goes up on each change (SL-7). |

Session statuses, shown as a dot everywhere a session appears:

| Status | Label | Dot |
| --- | --- | --- |
| working | Working | green, pulsing |
| needs | Needs you | orange |
| idle | Idle | grey |
| probably idle | Probably idle (SE-22) | grey, half-filled |
| exited | Agent exited — resumable | hollow ring |
| offline | Host offline — reconnecting | red |

Built-in agents (Claude, Codex and Copilot are the defaults; users can add
more, ST-2):

| Agent | Command | Context forks | Turn end (SE-21) |
| --- | --- | --- | --- |
| Claude Code | `claude` | yes | the agent's hooks |
| Codex CLI | `codex` | yes | output fallback |
| Copilot CLI | `copilot` | yes | the agent's hooks |
| OpenCode | `opencode` | no | output fallback |
| Plain shell | `zsh` (the login shell) | no | none: no turns |

## 2. Architecture

```text
 Your Mac
 ┌──────────────────────────────────────────────────────────────────────────┐
 │  Humata app (Rust, GPUI + tca-rust)                                      │
 │     │ gRPC over a local unix socket                                      │
 │     ▼                                                                    │
 │  humatad (Mac role): channels, boards, Inbox, access rules, journal,     │
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
 │ humatad (host role)            │        │ humatad (host role)            │
 │  ├─ holder ─ PTY ─ codex       │        │  ├─ holder ─ PTY ─ claude      │
 │  ├─ holder ─ PTY ─ claude      │        │  └─ own inbox while Mac away   │
 │  ├─ ~/Humata/<channel>/…       │        │                                │
 │  └─ humatad mcp ◀─ agents      │        │                                │
 └────────────────────────────────┘        └────────────────────────────────┘
      devbox-1 never talks to gpu-box directly: devbox-1 ─▶ Mac ─▶ gpu-box
```

- **AR-1** Every machine Humata uses runs one `humatad`. The Mac's daemon is
  the source of truth for everything that can be centralized (SL-1); other
  machines keep only clones and live agent context (SL-2).
- **AR-2** The app, the `humata` command-line tool and the MCP server are all
  clients of the same gRPC API.
- **AR-3** The app holds no session state that matters: quitting it changes
  nothing on any machine. View state the user expects back is kept by the
  host instead (AR-5).
- **AR-4** Remote machines reach each other only through the Mac (for now).
  Every cross-machine call is checked on the Mac against the channel's agent
  access rules. A call that stays on one machine is checked by that machine
  against its latest copy of the host's rules (AW-8). These checks cover
  calls made through Humata only (SEC-5).
- **AR-5** View state is stored on the host, not in the app: open panels and
  tabs, folds, scroll positions and filters (SE-13), pane widths and
  collapsed panes (NV-16), expand and collapse results (NV-19) and the
  pinned order (NV-25). After the app is quit, crashes or is reinstalled,
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
  SSH machines). A channel with no Mac in it is valid.
- **CH-2** The folder name is derived from the channel name (lowercase,
  non-alphanumerics become `-`) and shown live while typing. The user never
  picks a path.
- **CH-3** The repo picker lists repos from the `gh` CLI on the Mac, with fuzzy
  search and checkboxes; repos already in the channel are left out. The create
  button says how many repos it will clone ("Create channel and clone 2
  repos").
- **CH-4** Offline machines are shown in machine pickers but cannot be chosen.
- **CH-5** When a channel is created, Humata creates its folders and main clones
  on every chosen machine and shows progress until the clones finish, then
  opens the channel.
- **CH-6** The user can add a repo to an existing channel from the Repos &
  machines tab. Humata then makes its main clone on every machine of the
  channel.
- **CH-7** The user can add a machine to a channel. Humata creates the
  channel's folders and main clones on it.
- **CH-8** When the user removes a machine from a channel that has sessions on
  it, Humata asks what to do with each of those sessions: move it to another
  machine of the channel, or archive it. The machine is removed only after
  every session is handled.
- **CH-9** The Repos & machines tab shows a grid: one row per repo, one
  column per machine, each cell the number of session clones of that repo on
  that machine (a cell opens the sessions it counts). Each column heading
  shows the machine and its session count and opens its page; offline
  machines are marked offline. It leads to the two actions above ("Add a
  GitHub repo…", "Add a machine…") and to removing a repo or machine
  (CH-8). It shows no clone health, "behind main" or similar state.

```text
┌ Repos & machines ─────────────────────────────────────────────────────────┐
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
  **Inbox · n** (count shown only when nonzero), **Board**, **Repos &
  machines**, **Agent access**, **Activity**. The page header has the channel
  name and a **New session** button.
- **CH-12** The Sessions tab is one tree table, the only list of sessions on
  that page. Columns: Session (tree connectors `├─ └─ │`, status dot, title,
  fork kind "⑂ code + context / context only / code only", and a red "✉ n"
  inbox count for the session and all its forks), Agent, Machine ("This Mac"
  shown faint), Status (Needs you in orange), Started.
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
  or session with unread output is shown bold.
- **CH-17** Opening a session clears its unread mark.

### Growing and shrinking a channel's repos

```text
┌ Repos & machines ─────────────────────────────────────────────────────────┐
│                     This Mac · 6      devbox-1 · 3      gpu-box · offline │
│ acme/api            4 clones          2 clones          1 clone           │
│ acme/ledger         5 clones          1 clone           —                 │
│ acme/web  (new)     ▓▓▓▓▓▓░░ 62%      ✓ main clone      waits for gpu-box │
│                                                                           │
│ [Add repo…]  saved repositories · GitHub URL · local path                 │
└───────────────────────────────────────────────────────────────────────────┘
```

- **CH-18** **Add repo…** is available at any time on the Repos & machines
  tab. The user picks from saved repositories, pastes a GitHub URL, or picks
  a local path on the Mac (its remote is used for clones; a repo with no
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
  it from the Mac's trash within 30 days brings the channel back with its
  sessions archived. (provisional, D11)
- **CH-25** Only the user can delete a channel. An agent has no way to ask
  for it, and the CLI needs presence (SEC-10). (provisional, D11)

```text
┌ # payments-rewrite ───────────────────────────────────────── [New session] ┐
│ Sessions · 11 │ Inbox · 7 │ Board │ Repos & machines │ Agent access │ Activity│
├──────────────────────────────────────────────────────────────────────────── ┤
│ SESSION                                   AGENT      MACHINE   STATUS  STARTED│
│ ● Ledger schema migration  ✉ 7            Claude     This Mac  Working  2h   │
│ ├─ ● Try Postgres partitioning ⑂ c+c ✉ 5  Claude     This Mac  Needs you 40m │
│ │  └─ ● Partition by month ⑂ c+c ✉ 3      Claude     This Mac  Idle     30m  │
│ │     └─ ● Monthly + BRIN ⑂ c+c ✉ 3       Claude     This Mac  Working  20m  │
│ │        ├─ ● Benchmark BRIN ⑂ code ✉ 2   Codex      devbox-1  Needs you 8m  │
│ │        │  └─ ● Check vacuum ⑂ ctx ✉ 1   Claude     devbox-1  Working  2m   │
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
  named by Humata.
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
- **FC-7** On a remote machine without a signed-in `gh`, the Mac makes a git
  bundle and sends it over the SSH link; pushes from that machine go back
  through the Mac. The GitHub token never leaves the Mac.
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
- **SE-2** The New session dialog asks for: title, agent and model (preselected from
  the default, ST-15), machine (any online machine of the channel), and per repo of the
  channel whether to include it ("Own clone, new branch from main" or "Leave
  this repo out"); plus an optional first message that is typed into the agent
  once it starts. With no first message the session starts at the agent's
  prompt.
- **SE-3** When started from a board item, the title defaults to the item's
  title, the dialog shows "for PAY-7", the first-message placeholder names the
  item, the item is assigned to the new session, and an item in Backlog or To
  do moves to In progress.
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
│    [Open it] [Later] [Not now]                                (offer bar, SE-12)  │
├────────────────────────────────────────────┬─────────────────────────┬────────────┤
│ claude │ zsh │ ▤ client.rs ✕ │ ◧ PR review ✕│ Diff            [⇤] [✕] │  ✉ Inbox 7 │
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
   terminal area (tabs)                        panel (opens beside,      right strip
                                               widens ⇤, closes ✕)
```

- **SE-6** The header shows breadcrumbs (`# channel › parent session › this
  session`, each a link except the last), the status dot, the agent, the
  machine when it is not This Mac, and the status when it needs attention
  (Needs you, exited, offline). Buttons: **Resume agent** (only when the agent
  exited), **Fork**, and **⋯** (session menu).
- **SE-7** The session menu offers: Rename, Move to another machine…, Stop the
  agent (keeps terminal and clones), Push and open a pull request, Archive
  session….
- **SE-8** The terminal area has tabs: the agent's terminal (named by its
  command, e.g. `claude`), a `zsh` shell in the session's folder, one tab per
  file opened big (with ✕), and one tab per opened agent-prepared review view
  (with ✕; closing it keeps it in the Journal).
- **SE-9** The user can open more shell tabs. Each tab is its own terminal
  held by `humatad` (PER-1) and survives like the agent's terminal.
- **SE-10** The terminal is the agent CLI's own terminal: keystrokes go to it
  exactly as in a normal terminal, and everything the CLI draws is shown as
  drawn (colours, cursor, alternate screen, mouse, OSC 52 clipboard). There is
  no chat layer: queueing, rewind, compaction and images are whatever the CLI
  offers.
- **SE-11** The right strip has, top to bottom: **Inbox** (badge: pending
  requests of the session and its forks), **Diff** (badge: unsent comments),
  **Files**, **Board** (the session's board), **Journal**, **Info**. Clicking
  one opens its panel beside the terminal; clicking it again closes it.
- **SE-12** An open panel has a header with its name, **Widen** (⇤, covers the
  terminal) / **Show the terminal again** (⇥), and **Close** (✕).
- **SE-13** Sessions are stateful. When the user comes back to a session,
  Humata restores the open panel, whether it was widened, the active terminal
  tab, open editor tabs and their files, scroll positions, the diff base, the
  file browser's filter, folds and open file, and the inbox mode. This state
  survives an app restart.
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
- **SE-17** "Open in editor" opens the file in the user's editor on the
  session's machine.

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
- **SE-23** Messages Humata queues for a session (comments, notes, queued
  typing, the approval line of IN-12) are typed when the turn ends. When the
  end comes from the agent's hooks, they are typed on their own. When it
  comes from the output fallback, or the session is a plain shell, they are
  not: the queue shows "waiting: turn end not certain" with **Send now**.
  Each message is typed whole as
  one bracketed paste and never in the middle of another. (provisional, D7)
- **SE-24** Several writers may type into one terminal at once: the app,
  `humata attach` and Humata's own queued messages. Each writer's terminal
  shows "n others can type here" while others are attached with input.
  (provisional, D14)
- **SE-25** `humata attach --read-only` shows the terminal and sends no
  input; it does not count as a writer in SE-24. (provisional, D14)

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
  listed by name, when the agent can fork at a turn); agent and model
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
  channel from the session menu.
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
  which to push and open a PR for. PRs are opened with `gh` through the Mac.

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
  other files in its session folder.
- **LD-2** An archived session keeps its conversation, its session record
  (title, agent, machine, channel, repos and branches, fork parent, board
  items, journal), its scrollback and a diff summary: per repo, the branch,
  the last pushed commit and the files changed with their +/− at archive time.
- **LD-3** The channel's Sessions tab has a folded **Archived · n** group under
  the tree. Opening an archived session shows its scrollback, its
  conversation and its diff summary, read-only, with **Restore** and
  **Delete**.
- **LD-4** **Restore** asks for a machine (the original one first if it is
  still in the channel), clones each repo's branch from its remote, copies
  the conversation back from the Mac (fixing path-keyed records) and resumes
  the agent with it. A branch that was never pushed can't come back: the
  dialog says that work was deleted at archive and that the repo will start
  from `main`.

### Delete and trash

- **LD-5** **Delete** (on an archived session, or Archive's "and delete it"
  choice) removes everything of the session: record, conversation,
  scrollback, diff summary and any clones. Everything goes to trash first,
  after the same unpushed-work check.
- **LD-6** Each machine has one Humata trash at `<root>/.trash/`; the Mac's
  trash also holds deleted session records and conversations. Files deleted
  in the file manager (FM-3) go to the same trash.
- **LD-7** Anything in trash can be restored until it is emptied. Trash
  empties items 30 days after they were deleted; **Empty trash** on the
  Storage page empties it now, after saying how much it frees.

### The Mac keeps the history

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

- **LD-19** Low disk is shown only where it blocks an action. When a clone,
  fork, move or transfer would not fit, the dialog says
  how much it needs and how much is free, and lists the largest items from
  that machine's Storage page with their actions, so the user can free space
  and continue in place.
- **LD-20** When a machine runs out of disk while sessions run, those
  sessions show "<machine> is out of disk" with **Free up space** (opens its
  Storage page). There is no standing low-disk warning elsewhere.

### The mirror and secrets

- **LD-21** Before any part of a conversation leaves its machine for the
  mirror, every exact value of that machine's host variables in it is
  replaced with `‹NAME›`, the variable's name. The Mac's mirror never holds
  such a value. Other secrets an agent prints (a token it read from a file,
  say) are mirrored as printed, and the Storage page says so.
  (provisional, D12)
- **LD-22** A session's mirror is kept as long as its session record: it
  stays while the session is live or archived, goes to the Mac's trash when
  the session is deleted (LD-5), and is gone when that trash empties.
  (provisional, D12)

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
  **Open big**. Hunks show their header, line numbers and +/− marks, with
  syntax highlighting by file type.
- **RV-4** Clicking a line number opens a comment box for that line ("Comment
  for the agent — it gets file and line with it"). Add comment queues it;
  Cancel discards. Queued comments are shown under their line as "not sent
  yet".
- **RV-5** **Send N comments to agent** (disabled at zero) pastes all queued
  comments into the agent's terminal as one message: a header "Review comments
  from you (N):" then one line per comment `repo/path:line — text`. The
  comments then show "sent to agent" and the session is marked working.
  (Whether the agent should fetch them over MCP instead is q2.)
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
  Narrow panel: the viewer replaces the tree, with a back arrow. Widened panel:
  tree left, viewer right.
- **RV-13** For a session on another machine, the panel says the files are
  read from that machine over SSH; while it is offline, it shows the last copy
  read and says so.
- **RV-14** A session with no repos shows "This session has no repos" in Diff,
  Files and editor.

### Editor tab ("Open big")

- **RV-15** Open big opens the whole file in an editor tab in the terminal
  area and closes the side panel. One editor tab per session; opening another
  file replaces its file.
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
  numbers, and share the same queue. When comments are unsent, a footer shows
  "N comments not sent" with **Send to agent**.

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
- **RV-26** Comments are drafted first, then sent one at a time (**Send now**)
  or as a batch (**Send N**) to the session's terminal, as one bracketed
  paste that waits for the end of a turn (SM-9, SM-10, SM-14).
- **RV-27** Each sent comment carries its repo, file, line range with its side
  (old or new), the base it was compared against, and the quoted code, so
  the agent can find it even after the file has changed.

## 10. PR reviews

```text
 paste https://github.com/acme/api/pull/482  (Home, ⌘K, or #pr-reviews)
┌ Review a pull request ────────────────────────────────────────────────────┐
│ acme/api #482  Fix refund rounding for partial captures                   │
│ by rkhan · uhvesta/fix-refund-rounding → main · 7 files +214 −38          │
│ checks: 11 passed, 1 failed (lint)                                        │
│ Also review  [+ add a PR in another repo]   acme/ledger #91 (linked)  ✕   │
│ Machine  [This Mac ▾]  (This Mac · devbox-1 · gpu-box · lima-dev)         │
│ Agent    [Claude Code ▾]                                                  │
│ Focus    rounding on partial captures; anything that can double-refund    │
│ [ ] Let the agent push to the PR branch                                   │
│                                              [Cancel] [Start the review]  │
└───────────────────────────────────────────────────────────────────────────┘

 the review session
┌ #pr-reviews › acme/api #482 ● Claude Code ──────────────── [Submit review]┐
│ claude │ zsh                        │ Diff  PR #482 against main · 2 repos │
│ > Reviewing #482 with focus: …      │ api  uhvesta/fix-refund-rounding     │
│ ● Bash: cargo test -p payments      │  refund.rs  +62 −21                  │
│   test result: ok. 31 passed        │  140 │+ let cents = amount.round();  │
│ ● review_draft refund.rs:140        │      ┆ draft by Claude Code:         │
│ ● review_draft ledger …:22          │      ┆ "round() is half-away-from-…" │
│ > █                                 │      ┆ [Edit] [Drop]                 │
│                                     │ ledger #91  uhvesta/refund-ledger    │
│ ⚠ 2 new commits · [Update the clone]│ Drafts: 4 (3 by agent, 1 by you)    │
└─────────────────────────────────────┴──────────────────────────────────────┘
```

### Starting a review

- **PR-1** Pasting a GitHub pull request URL (or `owner/repo#n`) into Home's
  field, the ⌘K palette or the **PR reviews** channel opens **Review a pull
  request**. Other text in ⌘K searches as usual.
- **PR-2** The **PR reviews** channel holds every review session. Humata
  creates it, with its folders, the first time a PR is reviewed; its repos are
  whatever repos its PRs come from.
- **PR-3** Humata fetches the PR's title, author, base and head branches,
  size (files, +/−) and checks with the Mac's `gh`. If that fails, the
  dialog says why (not signed in to `gh`, no access to the repo, PR not found,
  rate limited) and what to do; for a rate limit it says to wait.
- **PR-4** The dialog picks the machine (This Mac or any SSH machine,
  provider-made ones included), the agent and model (preselected from the
  default, ST-15) and an optional focus. **Also review** adds PRs from other
  repos to the same session, so one review can span N repos.
- **PR-5** **Start the review** clones each PR's head into that machine's
  `~/Humata/pr-reviews/sessions/<review>/<repo>` (FC-6, FC-7) and starts the
  agent with a first message giving each PR's URL, title, base and head, the
  focus, and how to draft comments (PR-9).
- **PR-6** A review's clones are on the PR's own head branch name, tracking
  the PR's head (`refs/pull/<n>/head` for PRs from forks).

### Read-only by default

- **PR-7** A review session is read-only towards GitHub by default: the agent
  can edit files, build and test in its clones, but pushes through Humata
  are refused, and the refusal tells it so. The channel's push rule (IN-24)
  does not apply to review sessions; only PR-8 does. An uncontained agent
  that pushes with the machine's own git credentials is not stopped
  (SEC-5).
- **PR-8** **Let the agent push to the PR branch** (in the dialog and in the
  session's Info panel) allows pushes for that session only. Pushes go
  through the Mac (FC-7).

### Draft comments

- **PR-9** The agent drafts line comments with the MCP tool
  `review_draft` (repo, path, line or range, side, text). Drafts are
  allowed by default and never posted by the agent.
- **PR-10** The Diff panel's base defaults to each PR's base ("PR #482 against
  main"), grouped per repo with the PR's number. The other bases of RV-1 stay
  available.
- **PR-11** Drafts appear under their lines in the diff, the editor and review
  views, marked with who wrote them, with **Edit** and **Drop**. The user can
  add their own drafts from the same comment box, which in a review session
  offers **Draft for GitHub** and **Send to agent**.
- **PR-12** Nothing is posted to GitHub until the user submits. **Submit
  review** lists the drafts per PR (each editable or droppable), a summary the
  agent may have drafted, and the verdict per PR: **Comment**, **Approve** or
  **Request changes**. Submitting posts one review per PR with the Mac's `gh`.
- **PR-13** If posting fails, nothing is lost: the drafts stay, and the
  message says what failed and what to do. A review that posted for some PRs
  and not others says which.
- **PR-14** After submitting, posted drafts show as posted with a link; the
  session stays open for follow-up.

### When the PR changes

- **PR-15** Every minute while a review session is open, and every 5 minutes
  while it is not, Humata checks each PR's head with the Mac's `gh`. New commits show
  "N new commits, update the clone" on the session (diff header and Home).
- **PR-16** **Update the clone** fetches the new head and tells the agent
  what changed: the new commits and the files changed since the head it
  reviewed. A force-push is reported as such. Drafts on lines that changed are
  kept and marked outdated.
- **PR-17** A merged or closed PR shows "PR #482 was merged" (or closed) on
  its session with **Archive**. If drafts were never submitted, the Archive
  dialog names them before going ahead.
- **PR-18** The PR reviews channel's Sessions table shows, per review, the
  PR(s), author, state (open, merged, closed), drafts count and new commits.

## 11. Spec mode and comments

```text
┌ #payments › Refund spec  ● Claude Code · spec mode ──────────────────────────┐
│ claude │ ▤ docs/refund-spec.md ✕                                              │
├──────────────┬──────────────────────────────────────────┬─────────────────────┤
│ Contents     │ [Rendered] Source       ◐ changed since  │ Comments · 3        │
│ 1 Goals      │                           you last read  │                     │
│ 2 Rounding ◐ │ ## 2. Rounding                           │ ┌ L41–43  sent ───┐ │
│ 3 Ledger     │ ▌Partial captures round half to even,    │ │"half to even"   │ │
│ 4 Tests      │ ▌per line item, before summing.          │ │ Why per line    │ │
│              │                                          │ │ item, not total?│ │
│              │ | Case        | Input  | Refund |        │ └─────────────────┘ │
│              │ |-------------|--------|--------|        │ ┌ L58  draft ─────┐ │
│              │ | split       | 10.005 | 10.00  |        │ │"split"          │ │
│              │                                          │ │ Add a 3-way case│ │
│              │ - [x] per-item rounding                  │ └─[Send now]──────┘ │
│              │ - [ ] refund ledger entries              │ ┌ L12  addressed ─┐ │
│              │                                          │ │ [Resolve][Reopen]│ │
│              │   capture ──▶ ledger ──▶ refund          │ └─────────────────┘ │
│              │   (ASCII art in monospace)               │ [Send 1 as a batch] │
└──────────────┴──────────────────────────────────────────┴─────────────────────┘

 comment status:   draft ──Send──▶ sent ──agent changes the passage──▶ addressed
                     ▲                                                    │
                     └──────────── Reopen ◀── resolved ◀── Resolve ───────┘
```

### Spec mode

- **SM-1** A session can be in **spec mode** (New session dialog, or the
  session menu): the user and the agent iterate on Markdown files in its
  clone. The session page opens the spec files as tabs beside the agent's
  terminal; the agent works on them as on any file.
- **SM-2** The user picks which Markdown files are the session's spec files;
  a Markdown file the agent creates in the session can be added with one
  click. Any Markdown file in Files can also be opened rendered.
- **SM-3** Markdown renders fully: headings, tables, fenced code with syntax
  highlighting, task lists, links, and ASCII art and code blocks in monospace
  with their spacing kept.
- **SM-4** A **Rendered / Source** toggle switches the view; the position in
  the file is kept. A table of contents from the headings sits beside the
  file, and clicking an entry scrolls to it.
- **SM-5** When the agent rewrites the file, passages changed since the user
  last read them are highlighted, and their contents entries are marked. The
  highlight clears when the user marks the file read or scrolls past it.

### Comments

- **SM-6** Selecting text in the rendered view (or lines in source) offers
  **Comment**. The comment is anchored to the source lines of the selected
  passage and keeps the quoted text.
- **SM-7** When the file changes, each comment re-anchors to its passage. If
  the passage is gone, the comment says so and offers **Re-attach** (select a
  new passage) or **Drop**.
- **SM-8** Comments sit in the margin beside their passage. Adding, editing or
  dropping a comment never types anything into the agent's terminal.
- **SM-9** **Send now** on one comment, or **Send N as a batch**, types one
  message into that session's terminal: the file's path, then for each
  comment the quoted passage with its line numbers and the comment.
- **SM-10** The message is a bracketed paste, so it arrives as one input. If
  the agent is mid-turn, the message waits in Humata's queue for that session
  and is typed when the turn ends, as SE-23 says; the comments show "queued"
  until then.
- **SM-11** Each comment has a status: **draft** → **sent** → **addressed**
  (by the rule in SM-16). The user can
  **Resolve** a comment or **Reopen** a resolved one; reopening sets it back to
  draft.
- **SM-12** Resolved comments are hidden by default and shown with **Show
  resolved**.

### One comment model everywhere

- **SM-13** Comments in rendered Markdown, diffs (RV-4), the editor (RV-20),
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
  (This Mac or a remote), so the user can work Mac ↔ machine or machine ↔
  machine.
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
- **FM-9** Machine ↔ machine transfers go through the Mac (AR-4).
- **FM-10** The file manager can open any folder Humata owns on a machine
  (channel folders, `shared/`, session folders) as well as the user's home on
  that machine.
- **FM-11** A machine's page lists its Humata folders by channel; clicking one
  opens it in the file manager.
- **FM-12** Each pane has its own machine picker; offline machines are listed
  but cannot be picked. A pane whose machine goes offline shows the last
  listing read, says so, and disables actions on it.
- **FM-13** When either end of a transfer is offline (or the Mac, for a
  machine ↔ machine transfer), the transfer pauses and the tray says which
  machine it waits for; it continues on its own when that machine is back.
- **FM-14** A move across machines deletes the source only after every file
  has arrived with a matching checksum; the source goes to its machine's
  Humata trash.
- **FM-15** The file manager is its own page (rail, sidebar and ⌘K reach it);
  it remembers each pane's machine and folder across restarts.

## 13. Boards

- **BD-1** There are three kinds of board: the **Global** board, one board
  per **channel**, and one board per **session**. All have the columns
  Backlog, To do, In progress, In review, Done, with the count per column.
- **BD-2** The user can create an item on any board (title, status), drag
  items between columns, and open an item in a drawer.
- **BD-3** The item drawer shows: id, which board it is on, a link to its
  assigned session, title, who owns it ("You made this. Agents need your
  approval to change it." or "Made by <agent · session>. That agent may change
  it freely; others must ask you."), status, assigned session (any session of
  its channel, or Nobody), description, **Open its session**, **Start a
  session for this item**, and its history.
- **BD-4** An item card shows its id, title, a "by <session>" chip when an
  agent created it (not shown for your own or for a session's own items on
  its own board), and a "◦ <session>" chip when it is a session board's item.
- **BD-5** The channel board has a scope picker: **Channel board**,
  **Channel + every session board**, **A session and its forks** (one entry
  per session that has forks), **One session's board** (one entry per
  session).
- **BD-6** New item on the channel board creates a channel item, except in
  "one session's board" scope, where it goes on that session's board.
- **BD-7** The Global board shows global items and every channel's items (not
  session items), with a filter: Everything, Global items only, or one
  channel. Cards there also show their channel.
- **BD-8** The session strip's Board panel shows that session's board.
- **BD-9** An agent creating, editing, moving or deleting items follows the
  channel's agent access rules (IN-17): by default it may do anything on its
  own session board, add to the channel board and edit, move or delete
  channel items it created; anything else is an Inbox request.
- **BD-10** What a fork's session board starts with is open (q5).

## 14. Inbox and agent access

### Where the Inbox appears

- **IN-1** There are three Inbox scopes: **global** (all channels; the rail
  and sidebar show the pending count), **channel** (the channel's Inbox tab),
  and **session** (the strip's Inbox panel).
- **IN-2** The session Inbox has a toggle **This session · n** / **With its
  forks · n** when the session has forks. Opening it from a tree count opens
  it in "With its forks".
- **IN-3** Wide inboxes (global, channel) show the list on the left, grouped
  by session (with channel in the global one), and the selected request in
  full on the right. The first pending request is selected by default.
- **IN-4** The narrow session Inbox shows rows; clicking a row opens its card
  in place under it, clicking again closes it.
- **IN-5** Each row shows an icon for its kind (board ▦, bridge ⇄, session ▶),
  the action and its age, or "approved"/"denied" once answered.
- **IN-6** **Approve all n** and **Deny all** appear when more than one request
  is pending in the current scope and apply only to that scope.
- **IN-7** **Show answered** reveals answered requests, dimmed, with their
  answer and any note.
- **IN-8** The global Inbox has a folded **Allowed without asking (n)** log:
  what each session did that its rules allowed, and every answer, with when.

### The decision card

```text
┌───────────────────────────────────────────────────────────────────────────┐
│ ● Benchmark BRIN vs btree  [Codex CLI] [devbox-1]  #payments · 3m  Open ↗ │
│                                                                           │
│ Run the benchmark against the staging database                            │
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
│ Asks you because: Network connection through the Mac  Change this rule    │
│ Nothing to undo: read-only. Stop it any time from its terminal.           │
│                                                                           │
│ [Approve] [Deny] [Deny and reply…]     [Always let this session connect…] │
└───────────────────────────────────────────────────────────────────────────┘
```

- **IN-9** Every request card shows, in order:
  1. **Requester**: status dot, session title, agent, machine, channel, age,
     and **Open session ↗** (hidden when already on that session).
  2. **The action** as a title.
  3. **The agent's reason** in its own words, quoted.
  4. **What will happen**, previewed exactly (IN-10).
  5. **What it was doing**: its last few steps, the pending call last.
  6. **The rule** that sent it to you, with **Change this rule** (opens the
     channel's Agent access tab).
  7. **Whether it can be undone**, in red when it cannot.
  8. Buttons: **Approve**, **Deny**, **Deny and reply…**, and a specific
     **Always let …** whose label names the session and the target it
     covers (IN-22).
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
- **IN-12** Approve performs the action whether or not the agent's call is
  still waiting (TP-13). If the call is waiting, it returns the action's
  result. If the agent already got "pending" (TP-11), Humata types one line
  into its terminal at the end of its turn (SE-23): "… — approved by you",
  with the request id to wait on. (provisional, D4)
- **IN-13** Deny returns a denial naming the user to the agent's call, or to
  its next wait on the request (TP-12), and does nothing else. **Deny and
  reply…** asks for a note; the agent gets the note with the denial. If the
  agent already got "pending", Humata types "… — denied by you" as IN-12
  does.
- **IN-14** **Always let …** approves this request and adds a grant for the
  requesting session and the target IN-22 names for this kind of action.
  The label names both ("Always let “Benchmark BRIN” connect to
  staging-db:5432"). The agent cannot choose a wider scope. The grant is
  listed and removable in Agent access (IN-19).
- **IN-15** Answering a request removes it from every scope's pending list at
  once and updates every count. Only the first answer counts: a second
  answer from anywhere fails and says who answered it, how and when (AW-10).
  (provisional, D3)

### Agent access

- **IN-16** Some actions need no rule, and no setting stops them: reading its
  own session (title, status, repos, branches), reading and writing its own
  clones, its own views and its own requests. A context-only fork reads its
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
  | Global board, or other channels and their boards: anything | Ask me |
  | Write into `shared/` on its machine | Allow |
  | Fetch files from `shared/`, the channel folder or a main clone on its machine into its own clone | Allow |
  | Prepare views for you (you choose whether to open them) | Allow |
  | Draft PR review comments (review sessions) | Allow |
  | Ask you a question, or ask ahead for access (IN-25, IN-26) | Allow |
  | Read another session's files (same machine) | Ask me |
  | Read files on another machine (bridged) | Ask me |
  | Copy files into its clone or `shared/` from another session or machine | Ask me |
  | Send a note or question to another session | Ask me |
  | Type into another session's terminal | Ask me |
  | Start or fork sessions (same machine) | Ask me |
  | Start or fork sessions on another machine | Ask me |
  | Start a PR review | Ask me |
  | Change its own session: rename, add or drop a repo, spec mode, own copy | Ask me |
  | Stop, archive or delete its own session | Ask me |
  | Move a session to another machine | Ask me |
  | Push its branch or open a pull request (IN-24) | Ask me |
  | Open a network connection through the Mac (HO-11) | Ask me |
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
- **IN-19** Scoped "Always let …" rules are listed under their rule with their
  scope and can be removed.
- **IN-20** A context-only fork's writes to its parent's clones are never
  allowed, whatever the rules say (FK-7).
- **IN-21** Rule changes take effect for the next call; pending requests stay
  pending. When a pending request is answered, the answer is checked against
  the rule as it is then: approving a request whose rule became Never fails
  and says the rule changed.
- **IN-22** A grant made by **Always let …** covers exactly this target:

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
  **Always let …** for that target (IN-22), and denying returns a denial.

## 15. Agent-prepared views and the Journal

- **VW-1** An agent can call `view_prepare` to prepare a view for the
  user: a **review walkthrough** (ordered steps, each a repo, path and note,
  with a title and summary), a **board view** (a channel board in a given
  scope, with a title and summary) or a **pull request view** (VW-13). Gated
  by the "Prepare views for you" rule.
- **VW-2** Preparing a view changes nothing on screen. Humata shows an offer
  bar: "<agent> in “<session>” prepared a view for you: <title>", the summary,
  and **Open it**, **Later**, **Not now**.
- **VW-3** Review offers appear on the session page. Board offers appear on
  the session page and on the channel board.
- **VW-4** **Later** hides the offer and shows it again the next time the
  user returns to that session.
- **VW-5** **Not now** hides the offer for good; the view stays in the
  Journal.
- **VW-6** **Open it** on a review opens it as a tab in the session's terminal
  area: the left list shows the title, who prepared it and when, the summary,
  and numbered steps with notes; the main area shows the step's file in the
  editor (RV-17) with the agent's note above it, and **‹ Prev** / **Next ›**.
- **VW-7** **Open it** on a board view opens the channel board in the view's
  scope.
- **VW-8** While an opened view is showing, a bar says "Showing “<title>”,
  prepared by <agent>" with **Revert** and **Keep**.
- **VW-9** **Revert** restores exactly what the user had before opening:
  route, terminal tab, open panel, widened state and board scope. **Keep**
  removes the bar and leaves the view as it is.
- **VW-10** The Journal panel lists every view prepared in the session and its
  forks, newest first, with its title, which session (for forks), kind, when,
  summary, state (Waiting for you, Later, Not now, Opened, Reverted, Opened and
  closed) and **Open**. Any view can be opened again at any time.
- **VW-11** With no views, the Journal says how to get one (e.g. ask the agent
  to "prepare the PR review for me").
- **VW-12** Views work for sessions on remote machines: the view is stored
  with the session on the Mac and its files are read over the bridge.
- **VW-13** A pull request view holds, per repo, a title, a body, a base
  branch and whether it is a draft. Preparing it pushes nothing. **Open it**
  shows the fields, editable, with **Push and open PR(s)**, which runs the
  push and pull request action (MV-12) with what is shown.
- **VW-14** A prepared view is a draft until the agent marks it ready. A
  draft is not offered and is not in the Journal. The agent can revise its
  draft as often as it likes. Marking it ready shows the offer (VW-2).

## 16. Agents' view of the topology (MCP tools)

Each `humatad` serves MCP tools to the agents on its machine. Tools that reach
outside the session go through the Mac.

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
- **TP-8** Every cross-machine call goes through the Mac and is checked there
  against the channel's rules; calls on anything the agent doesn't own become
  Inbox requests.
- **TP-9** While the Mac is offline, calls behave as the table in AW-7 says.
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

## 17. Hosts and the bridge

- **HO-1** The user can add a machine over SSH (plain SSH host, MP-10) by
  picking a host from `~/.ssh/config` (or typing one). Humata then shows steps with progress:
  Connect over SSH; Check the system (arch, disk, git, gh); Install humatad;
  Start it and open the link; Find agents. When done, the machine shows in
  Hosts and in every machine picker.
- **HO-2** If a step fails, Humata stops there, says what failed and how to fix
  it, and offers Try again.
- **HO-3** The Hosts page has a table: status, machine and SSH address,
  system, daemon version, latency, number of sessions; and a folded "How
  machines connect" bridge diagram generated from the real machines and
  session counts.
- **HO-4** A machine's page shows its OS, address and daemon version;
  **Reconnect now** when offline; **Open a shell** for remote machines;
  sessions on it (each opens) and **New session on <machine>**; a health card
  (gh signed in as which user with which scopes, agents installed, disk free,
  variable names); its Humata folders by channel; and what its agents can
  reach through the Mac with each rule's setting.
- **HO-5** When a machine goes offline, its sessions keep running there; the
  sidebar marks it offline, the rail shows "!", its sessions show "Host
  offline — reconnecting", and its page says only the link is down, Humata
  retries every 5–30 s, and its files show the last copy read.
- **HO-6** Humata reconnects on its own with backoff. **Reconnect now** tries
  at once. On reconnect each session catches up its output as TR-4 says:
  exactly while the missed output is still kept, otherwise from a fresh
  screen. Humata says how many sessions reattached.
- **HO-7** Host variables: only their names are sent to clients; values stay
  on the machine. The machine's page lists the names with **Set value…** and
  **Remove** (HO-13).
- **HO-8** GitHub access uses the `gh` CLI; there is no GitHub OAuth app. The
  Mac's GitHub token never leaves the Mac.
- **HO-9** Remote machines never talk to each other directly. Possibly later:
  direct machine-to-machine SSH under the same rules.
- **HO-10** The bridge carries only Humata's own calls and file transfers.
  Nothing else crosses it unless the user approves a port forward (HO-11).
  (provisional, D5)
- **HO-11** A port forward is opened only when the user approves it. The card
  shows the machine it opens from, the host and port it reaches, and that it
  stays open until the session stops. Low-risk rules never cover it, and it
  can't be approved while the Mac is away. It closes when the session stops.
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
 This Mac (main host)                         devbox-1
 ~/.claude/skills/, plugins       one way     ~/Humata/.skills/   (Humata-managed;
 other agents' skill dirs   ───────────▶    claude/, copilot/  agents pointed here)
   minus exclude patterns,                    ~/.claude/skills/   (made on devbox-1:
   minus .env, keys, *.pem …                                       never touched)
```

- **SK-1** The Mac's skills (each agent's skill and plugin folders) sync one
  way to machines. Nothing syncs back.
- **SK-2** `humata skills sync [machine…]` syncs to the named machines, or to
  all; Hosts and each machine's page have **Sync skills**.
- **SK-3** Settings has **Sync skills to machines automatically** (on connect
  and whenever the Mac's skills change), with an opt-out per machine.
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
  never under a live holder, as TS-1), registers `humatad2` as the launchd
  login item (PER-20) and puts `humata` on the user's PATH. Nothing on the
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
  on the Mac (a folder or a command's output), a destination on machines, and
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
- **MP-2** A provider is a TOML entry on the Mac (Settings › Machine
  providers) of command templates: `create`, `connect` or `identity`, and
  optional `start`, `stop`, `delete`, `status`. An action appears only when
  its command is defined.
- **MP-3** `{name}` in a template is replaced by the machine's name,
  shell-quoted; it is also exported as `HUMATA_NAME`.
- **MP-4** `HUMATA_<PARAM>` variables exist only for params the provider
  declares; each declared param is a field in Add machine. Commands get no
  other variables from the Mac or from Humata's secrets, except host
  variables the provider lists by name in `env`.
- **MP-5** Commands run on the Mac by default, or on a named machine
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
 Mac sleep / offline               scrollback; reattach gets a
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
 Host (this Mac): source of truth              Each machine: only what must be local
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

- **SL-1** The host (the Mac running the app) is the source of truth for
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

### Tier 1: the app, the Mac's link, SSH

- **PER-1** Each session's PTY is held by its own small holder process on the
  session's machine. The holder is not a child of the SSH connection or of
  the app, and keeps running when either goes away.
- **PER-2** The holder keeps the terminal's VT screen state (via a terminal
  emulator library) and flushes scrollback to disk at least once a second.
- **PER-3** When a client reattaches, it gets a full screen snapshot and then
  all output produced since its last offset, with nothing lost or repeated,
  while that output is still kept. When it is not, the client gets a fresh
  snapshot and a count of the lines not kept (TR-4).
- **PER-4** Quitting or crashing the app, the Mac sleeping or going offline,
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
- **PER-21** The Mac reconnects to every machine with backoff.
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

## 20. Async: working while the Mac is away

- **AW-1** Sessions on remote machines keep working while the Mac is asleep,
  offline or off.
- **AW-2** From any plain SSH terminal on a machine, `humata attach <session>`
  attaches to a session's terminal; detaching leaves it running. `humata ls`
  lists the machine's sessions. Attaching with input is user-only and needs
  presence (SEC-10); an agent's attempt is refused. Several writers may be
  attached at once (SE-24).
- **AW-3** While the Mac is away, each remote daemon keeps its agents'
  requests in its local journal (SL-5). `humata inbox` on that machine shows
  them, and nothing else shows them until the Mac is back.
- **AW-4** Each channel has a **While I'm away** rule for requests that would
  ask: **Wait** (the call waits), **Skip the step** (the call is denied with a
  note that the user is away), or **Allow low-risk kinds** (the user picks
  which kinds count as low-risk, from those AW-11 allows).
- **AW-5** The user can answer requests later from the Mac, or on the machine
  with `humata inbox` over SSH. Answering on the machine while the Mac is
  away needs the approval passphrase (SEC-13).
- **AW-6** When the Mac reconnects, each journal replays into the host's Inbox
  in order (SL-5, SL-6). A request answered on the machine arrives answered,
  credited to the user on that machine. A request still pending becomes an
  ordinary Inbox request on the Mac.
- **AW-7** What each call does:

  | The call | Mac reachable | Mac away |
  | --- | --- | --- |
  | Reads and writes of its own session's files, clones and `shared/` | done on the machine | done on the machine |
  | Reads of channel state (topology, boards, its comments) | answered by the Mac | answered from the last copy, marked with its age |
  | Writes the rules allow (own board, channel board add, its own items, comment and view drafts, questions to the user) | done | journaled (SL-5) |
  | Anything the rules say Never | denied | denied |
  | An Ask on something on this machine (another session here, start or fork here, type into a session here) | Inbox request; the call waits | per the While I'm away rule |
  | Anything that needs another machine or GitHub (read or copy from another machine, start or fork elsewhere, a note to a session elsewhere, push, port forward) | done or asked | waits, or under Skip denied saying the Mac is away; never low-risk |
  | User-only calls | user-only (SEC-9) | only answering requests the machine holds, with the passphrase |

  (provisional, D3)
- **AW-8** While the Mac is away, a machine judges calls against the last copy
  of the rules, grants and While I'm away rule the Mac sent it. Nothing on
  the machine can change that copy. (provisional, D3)
- **AW-9** On reconnect, every journaled write is checked again against the
  rules as they are then. A write a tightened rule no longer allows is not
  applied; it is shown in the item's history and as one line in the agent's
  terminal. (provisional, D3)
- **AW-10** The first answer to a request wins. A second answer from anywhere
  fails and says who answered, how and when. A deny never undoes an approval
  that already ran. (provisional, D3)
- **AW-11** Low-risk kinds can never include anything that needs another
  machine, a network connection, typing into another session, or any action
  that is Never by default. (provisional, D3)
- **AW-12** **Always let …** given on the machine while the Mac is away
  applies there at once and becomes a grant on the Mac at replay, with the
  same target (IN-22). (provisional, D3)

## 21. Transport: gRPC streaming

- **TR-1** `humatad` speaks gRPC (tonic). The app and the CLI reach the Mac's
  daemon over a local unix socket.
- **TR-2** The Mac reaches each remote daemon over one SSH connection per
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

## 22. Navigation and UI principles

- **NV-1** The left rail has: Home, Channels, Inbox (pending badge), Global
  board, Hosts ("!" when a machine is offline), and at the bottom Settings.
- **NV-2** The sidebar has Home, Inbox (count), Board, the channels with their
  session trees, Hosts (each machine, marked offline when it is), and
  Settings; plus a search button (⌘K).
- **NV-3** A **Back** button appears only after drilling in from a page
  (e.g. opening a session from a channel table, an Inbox card or an item). It
  is labelled with where it goes ("‹ #payments-rewrite").
- **NV-4** Lateral moves get no Back and clear the trail: sidebar, rail, tabs,
  breadcrumbs, the palette.
- **NV-5** ⌘[ goes back (same as the Back button). Escape closes the dialog,
  then the drawer, in that order.
- **NV-6** Pages that sit inside others show breadcrumbs (channel › parent
  session › session; Hosts › machine).
- **NV-7** Every screen offers at least one action on what it shows. A list
  or table of state is shown only where each row opens something or can be
  acted on, or where NV-8 lists it as kept on purpose.
- **NV-8** Humata does not detect or show what the user can't act on from
  that spot, and shows a problem only where it blocks an action. Kept on
  purpose: the Hosts table columns, the host health card, the bridge diagram,
  clone counts, the "Allowed without asking" log, Home's recent feed, the
  Activity tab, Info's "Survives restarts", Settings' context-fork column and
  the Started column.
- **NV-9** Problems that concern a session are shown on that session, not in a
  separate list.
- **NV-10** Short confirmations appear as a toast at the bottom, sometimes
  with one action ("Show Home").
- **NV-11** First run with no channels: the main area explains that a channel
  is a project (its repos, machines, board and sessions) and offers **Create
  your first channel** and **Add a machine over SSH**, noting that GitHub
  access comes from the `gh` CLI with no sign-in in Humata.

### Resizable and collapsible panes

```text
 expanded                                        left sidebar collapsed
┌──────────┬┬──────────────────────┬┬─────────┐  ┌─┬──────────────────────┬┬─────────┐
│ Sidebar  ││ terminal / editor    ││ Diff  ⇤✕│  │›│ terminal / editor    ││ Diff  ⇤✕│
│ [«]      ││                      ││ [»]     │  │ │                      ││ [»]     │
│ # pay…   ││                      ││         │  │ │                      ││         │
│  ├─ ●    ◀▶ drag divider         ◀▶ drag    │  │ │                      ◀▶        │
│  └─ ●    ││                      ││         │  │ │                      ││         │
└──────────┴┴──────────────────────┴┴─────────┘  └─┴──────────────────────┴┴─────────┘
   «  collapses the pane          thin strip (›) opens it again at its last width
```

- **NV-12** Every sidebar and panel can be resized by dragging its divider:
  the left sidebar, the session's side panel (Diff, Files, Inbox, Board,
  Journal, Info), the editor's changes list, the Files panel's explorer side
  when the viewer is beside it, and each pane of the file manager.
- **NV-13** Each of those panes has a collapse button. A collapsed pane leaves
  a thin strip; clicking the strip opens the pane again at the width it had
  before it was collapsed.
- **NV-14** A divider stops at a minimum width for each pane and never lets the
  terminal or the main content shrink below its own minimum; dragging past a
  pane's minimum collapses it.
- **NV-15** Double-clicking a divider returns that pane to its default width.
- **NV-16** Pane widths and collapsed states persist across app restarts: the
  left sidebar and the file manager panes globally, the session's panels and
  editor list per session (with the rest of SE-13).
- **NV-17** When the window is too narrow for every open pane at its saved
  width, panes shrink toward their minimums without changing the saved widths;
  widening the window brings them back.

### Expand all / Collapse all

- **NV-18** Every tree or grouped list has **Expand all** and **Collapse all**
  on its header and in the palette: the sidebar's channels and fork trees,
  the diff's repo groups and files, file trees, Inbox groups, and spec
  contents sections.
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
  sidebar and of Home, as well as in their usual places. A pinned item with an
  event (needs input, finished, failed, new PR commits) shows that event
  there.
- **NV-22** A VIP item that needs the user shows a "[VIP]" prefix everywhere
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

## 23. Home, global Inbox, Global board, palette

- **HM-1** Home has **Needs you**: every session that needs input, exited or
  is offline, with channel, machine, why, and **Open** or **Resume**.
- **HM-2** Home has **Recent across channels**: the latest events from every
  channel's Activity.
- **HM-3** The global Inbox (IN-1) shows "<n> waiting, everywhere".
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

- **ST-8** In Wrap mode, long lines wrap at the pane's width with no
  horizontal scroll bar; continuation lines are indented to the line's own
  indentation; the line number, +/− mark and comment target stay on the
  first visual line, and the gutter stays aligned with it.
- **ST-9** In Wrap mode, side by side keeps the paired old and new lines
  starting on the same row: the shorter side is padded so the next pair lines
  up.
- **ST-10** Resizing a pane in Wrap mode rewraps the lines without moving the
  line at the top of the view.
- **ST-11** In Scroll mode each line is one visual line and the view scrolls
  horizontally; in side by side both sides scroll together.
- **ST-12** The terminal is not affected: it wraps as the agent CLI draws it.
- **ST-13** **Branch prefix** (e.g. `uhvesta/`), default the `gh` login
  followed by `/`; can be empty; overridable per channel (FC-12).
- **ST-14** **Retention** defaults for new channels, all off (LD-16).
- **ST-15** **Default agent and model**, any built-in or user-added agent,
  with an optional per-channel override. New session, fork and PR review
  preselect it.
- **ST-16** **Sync skills to machines automatically** and skill exclude
  patterns (SK-3, SK-6).
- **ST-17** **Security**: **Contain agents** and its per-channel override
  (SEC-6), the approval passphrase (SEC-13), and CLI logins and script
  tokens, each with **Revoke** (SEC-14).

## 25. Security and secrets

- **SEC-1** Values of host variables are never sent to clients and never
  written to project files; only their names cross. They reach the agents on
  their machine as environment variables (HO-13). Before a transcript is
  mirrored to the Mac, exact values are replaced (LD-21).
- **SEC-2** The GitHub token never leaves the Mac. Remote clones without `gh`
  use bundles and pushes through the Mac (FC-7).
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
  approval passphrase while the Mac is away (SEC-13). Typing "yes" at a
  prompt never counts. (provisional, D2)
- **SEC-11** A login covers only the terminal session it was made in. A
  process that leaves that session (for example with `setsid`) is not
  covered.
- **SEC-12** A presence prompt shown inside a session's own terminal is
  refused: an agent could answer it.
- **SEC-13** The approval passphrase is set in Settings › Security. It has at
  least 5 random words. Five wrong tries lock it for 15 minutes on that
  machine. The Settings page explains how to answer over SSH with the Mac
  closed: `humata login` with the passphrase, then `humata inbox`.
- **SEC-14** Scripts use tokens the user makes in Settings › Security. A token
  lists what it may do, which channels and when it expires. Tokens can be
  listed and revoked. A token can never do a user-only action.
- **SEC-15** No tool takes a caller's identity as an argument: an agent is
  always the session it was launched in. If a session's identity is used
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
3. Widens the panel (⇤) to read.
4. Clicks line 40 in `api/src/ledger/client.rs`, writes a comment, Add
   comment. Repeats on `ledger/…0042_accounts.sql` and `web/…/Cards.tsx`.
5. Clicks **Open big** on `client.rs`, switches to Side by side, uses ↓ to step
   through changes, adds a fourth comment there.
6. Clicks **Send 4 comments to agent**. The agent's terminal gets one message
   listing `repo/path:line — text` for each.
7. Clicks **Mark reviewed**.
8. Later, pushes `api` and opens its PR from the api group, leaving the other
   repos for later.

### 26.4 Approve an Inbox request after opening the requester

1. The rail shows Inbox 7. The user opens the global Inbox.
2. Selects "Read files of “Ledger schema migration” on This Mac" from "API
   rate limiter" (devbox-1).
3. Reads the reason and the two paths; clicks a path to see the file in that
   session.
4. Clicks **Open session ↗** on the card. "API rate limiter" opens with its
   Inbox panel and the request selected, beside its terminal.
5. Reads the terminal: the test failing on `accounts.balance_cents`.
6. Clicks **Approve**. The agent's waiting call returns the files and the
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

1. devbox-2 loses power. Its sessions show "Host offline — reconnecting";
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

### 26.13 Resize and collapse panes

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

1. The user pastes `https://github.com/acme/api/pull/482` into Home.
2. **Review a pull request** opens with the title, author, size and checks
   fetched with `gh`. They add `acme/ledger#91` under Also review, pick
   devbox-1 and Claude Code, and type a focus.
3. **Start the review** clones both heads into
   `~/Humata/pr-reviews/sessions/…` on devbox-1 and starts the agent with the
   PRs and the focus.
4. The agent builds and runs the tests, then drafts three comments with
   `review_draft`. They show as drafts under their lines.
5. The user edits one, drops one, adds one of their own as Draft for GitHub.
6. The author pushes 2 commits. The session shows "2 new commits, update the
   clone"; the user clicks it and the agent is told which files changed.
7. The user opens **Submit review**, picks Request changes for #482 and
   Comment for #91, and submits. Both reviews post through the Mac's `gh`.
8. Days later #482 merges; its session offers **Archive**, and the user
   archives it.

### 26.18 Forget a lost machine and free disk elsewhere

1. A cloud box, devbox-2, was deleted outside Humata. Its sessions show
   "Host offline — reconnecting".
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

### 26.20 Iterate on a spec with the agent

1. The user starts "Refund spec" in spec mode and asks the agent to draft
   `docs/refund-spec.md`.
2. The agent writes it; the user adds it as a spec file and reads it
   rendered, with its contents list and ASCII diagram.
3. They select "half to even" and comment, then select a table row and
   comment again. Nothing reaches the agent yet.
4. They click **Send 2 as a batch**. The agent is mid-turn, so the message
   waits; when the turn ends, it arrives as one paste with the path, quoted
   passages, line numbers and comments.
5. The agent rewrites section 2. The changed passages are highlighted; one
   comment becomes addressed, the other's passage is gone and offers
   Re-attach or Drop.
6. The user re-attaches it to the new paragraph, resolves the addressed one,
   and sends the rest when ready.

### 26.21 Add a repo to a channel, then to one session

1. On `#payments-rewrite` › Repos & machines, the user clicks **Add repo…**
   and pastes `https://github.com/acme/web`.
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

1. devbox-2's disk fails. Its sessions show "Host offline — reconnecting".
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

## 28. Where the prototype differs

The prototype snapshot predates some decisions. This spec follows the
decisions:

- The prototype gives a channel one machine ("Main clones live on"); a
  channel has a set of machines (CH-1).
- The prototype's Repos tab is "Repos & machines" here (CH-9, CH-11).
- The prototype's New session dialog offers "Share the clone of …" and "The
  main clone (shell-style, careful)"; this spec offers only an own clone or
  leaving the repo out (SE-2), pending q4.
- The prototype limits context-only forks to the parent's machine; they may go
  to any machine of the channel (FK-8).
- The prototype has no file manager, transfers, move flow, While I'm away
  rule, `humata attach`, "Couldn't resume" state, or per-repo push/PR; they are
  specified here.
- The prototype's Access tab has no cross-machine copy, message, remote fork
  or network-reach rules; IN-17 adds them.

## 29. Open questions

- **q1** Channel feed: events only, or can the user also post notes into it?
- **q2** Diff comments: pasted into the agent's terminal as one message (as
  specified), or kept in Humata for the agent to fetch over MCP?
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
  images).
- **q9** Should Humata also show a structured view of tool calls beside the
  terminal?
- **q10** Answered: channels span many machines.

# Humata 2 prototype coverage

This directory is the UI and UX iteration surface. Data, failures, account
names and machines are hardcoded. No shell, agent, GitHub, SSH, provider or
filesystem operation is performed. Reloading resets the state.

The original HTML prototype stays untouched. The current Humata 2 spec is
the baseline for the catalog; the work model below supersedes its generic
board-item terminology in this prototype only.

## Work model

Global or channel → milestones containing steps, plus standalone tasks.
Steps inherit their milestone’s scope. A task cannot have a parent.
Sessions execute work; they do not own its lifetime. The session Work panel
shows linked channel/global work rather than a second set of duplicate
project tasks. Milestones show step progress and have explicit status.

List is the default, with expandable milestones and one vertical scroll.
Board shows the same records by status. Item details, comments, blockers and
history form one scrolling document. Scope and kind are always visible.

## Connected workspace interactions

- Global and channel work; milestone/task/step creation; scope selection;
  nested steps; linked sessions; title/description editing; Markdown preview;
  comments; change history; status menus; drag between board columns.
- Blocked work with prerequisite records or an external reason; cycle and
  self-dependency rejection; Won’t do alongside ordinary statuses.
- Three fork types; four levels of fixture nesting; parent navigation;
  session-local terminal and pane state; creating another fork at any depth.
- Real `terminal_view` renderer on an in-memory grid: typing, paste,
  selection/copy, resize, scroll and a cursor. Agent feedback and shell
  messages are separate. No PTY or process is opened.
- Multi-repository diffs with old/new coordinates; file comments and dragged
  or Shift-clicked line ranges including deleted and mixed changed lines.
  Session-owned drafts, discard, queue, delivery
  to the owning agent terminal, explicit Send now for uncertain turn ends,
  waiting for a working agent, stopped process or disconnected machine.
- Close a terminal view and reopen it without stopping the process; stop
  an agent and resume its conversation; unavailable resume with a context
  fork fallback; archive and restore; Trash and restore. Forks survive.
- Manual machine reconnect; offline input disabled; retained terminal output.
- Scope-filtered Inbox and approvals; requests can open their owning session.
- Movable session-bound panes, horizontal/vertical splits, independent
  shells, multiple visible sessions, edge drops from pane titles and sidebar
  sessions, and a right rail for opening tools. Clicking an open tool focuses
  it. Pane menus contain layout and session actions. Close/reopen preserves
  terminal input and unsent feedback; maximize/restore keeps divider sizes.
- A generic Docs pane for specs, documents and notes: Preview, Edit and
  Changes, source diffs and rendered before/after revisions. Passage
  selection in preview and source, inline draft comments, recipient selection
  and delivery to another agent terminal. Anchors preserve the original
  Markdown source syntax (rather than rendered text) and flag changed passages. All state remains in memory.
- Markdown headings, emphasis, quotes, tables, task lists, links and
  syntax-highlighted fenced code; actual Rust Mermaid flowchart, sequence and state
  rendering, bundled diagram fonts, and actionable syntax errors.

## Broader flow studies

Scenarios → Workspace flow studies opens staged, clickable designs with
hardcoded intermediate states, failure/retry, cancellation and completion.
These explore the UI contract; they do not yet implement the full feature
widgets or service behavior. The distinction is deliberate and visible in
the scenario catalog.

| Spec journey | Surface |
| --- | --- |
| 26.1 all-remote channel | Create a channel |
| 26.2 start a session | Connected New session dialog |
| 26.3 review three repositories | Connected Diff and terminal delivery |
| 26.4 approve a request | Connected Inbox |
| 26.5 fork onto GPU | Fork dialog and agent Inbox request |
| 26.6 move a session | Move-session study and machine picker |
| 26.7 interrupted upload | Files and transfers study |
| 26.8 app/Mac reboot | Reboot and crash recovery study |
| 26.9 remote power loss | Recovery study and resume scenarios |
| 26.10 host away | Access and away-mode study |
| 26.11 archive unpushed work | Connected archive confirmation/recovery |
| 26.12 prepared view and Revert | Journal and walkthrough study |
| 26.13 resizing | Connected resizable panes |
| 26.14 long-line wrapping | Connected Diff Wrap control |
| 26.15 remote-to-remote files | Transfer study; specialized two-pane widget still to refine |
| 26.16 provider and login | Add provider machine study |
| 26.17 PR review/update/submit | Pull-request review study |
| 26.18 lost machine/storage | Lost machine and storage studies |
| 26.19 branch naming | Channel setup study |
| 26.20 specification iteration | Connected Docs pane with selected-passage feedback; broader spec lifecycle study |
| 26.21 repository changes | Add/remove repository study |
| 26.22 palette navigation | Search sessions, work, channels and machines |
| 26.23 permanent loss | Lost-machine reconstruction study |

Additional studies cover agent/appearance settings, skills synchronization,
retention previews, scoped approvals, expired requests and outdated anchors.
A flow study is a starting design, not a claim of complete behavioral parity.

## Exercising the prototype

Native launch and test targets select GPUI Fast and optimized builds. The
current browser bundle predates movable panes; a Bazel Wasm target is pending.
The `automation` feature drives actual controls through GPUI’s test APIs,
including input, drag, scroll and clicks. It records PNGs plus registered
roles, labels and bounds under `target/ui-captures/`.

```sh
.tools/bin/bazel test -c opt //apps/humata2/ui-prototype:ui_test --test_output=errors
```

The tests exercise the connected workspace flows and walk every staged
flow through failure/retry and completion. Rendered screenshots still need
visual inspection; model assertions alone do not establish visual quality.

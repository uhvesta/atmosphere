# Humata 2 use cases

These are test cases written in prose. Each one says how a person or an agent
uses Humata 2, step by step, and what should happen. They are not code yet.
Later they can become tests, driven by the events and state each step names.

Every case has the same parts. It starts with a starting state. Then come the
steps, each with **I do**, **I see**, **State and events** and a **Status**.
Then the branches, the expected outcomes, a status table, and the gaps and
likely bugs. [`briefs/_GUIDE.md`](briefs/_GUIDE.md) is the template.
[`MAP.md`](MAP.md) is the layer map the cases were written from.

## How to read a status

Each step and branch was tagged by reading the code and tests, not the spec.

| Tag | Meaning |
|---|---|
| **Works (real shell)** | A test drives a real shell or real SSH. Most are manual lab targets in `terminal/lab/tests`, using the `h2-terminal-c1` VM. |
| **Works (real daemon)** | A test runs a real `humatad2`, in `daemon/humatad/tests`, `clients/live/tests` or `desktop/tests/live_*`. These use fake agents, a fake `gh` and fake boxes over a local pipe, so they do not prove real GitHub, real agents or real SSH between machines. |
| **Fakes only** | Only reducer, fixture or headless-window tests cover it (`features/*/tests`, `desktop/tests/native`). Engine-only terminal tests (`terminal/vt/tests`) count here too. The case says what is unproven live. |
| **Code, no test** | The code exists, but no test covers it. |
| **Not built** | The feature is missing. The case gives its milestone or ID, or says "no milestone". |
| **Likely bug** | The code seems to do the wrong thing. The case gives file:line and how to confirm it. |

## Counts

| Layer | Cases | Rows tagged | Works (shell) | Works (daemon) | Fakes only | Code, no test | Not built | Likely bug |
|---|---|---|---|---|---|---|---|---|
| Micro: keys and terminal | 27 | 344 | 26 | 55 | 123 | 101 | 35 | 4 |
| Macro: whole working days | 16 | 378 | 2 | 287 | 36 | 19 | 33 | 1 |
| Agents: CLI and MCP | 20 | 374 | 0 | 271 | 34 | 41 | 28 | 0 |
| Real: app, SSH VM and agent CLIs | 14 | 249 | 16 | 101 | 20 | 45 | 37 | 23 |
| **All** | **77** | **1,345** | **44** | **714** | **213** | **206** | **133** | **28** |

The counts come from each case's status table. A row with mixed tags counts
under its first tag. "Works (real daemon, fake box)" and the like count as real
daemon.

What the counts say:

- The terminal layer has the least live proof. Most key, paste and mouse
  behaviour is proven only on the engine or in a headless window.
- The daemon and its CLI are well covered by real-daemon tests. The app's
  screens on top of them are mostly proven on fakes.
- The real layer (`real/`) is the least proven. Real SSH, vim, paste and
  resize are proven only on terminal components, never through the app's own
  path to the VM. Six rows are proven only by a manual REALFLOW run with real
  Copilot and Codex CLIs ("Works (real CLI)", not in the columns above), and one
  is not run by rule (rebooting the shared VM). Resume and fork with real CLIs
  fail live (B5, B7, B8, B9, B10), and every machine-script step is Not built.
- Agent calls are proven on a real daemon, but 38 of the 81 MCP tools that are
  advertised fall through to "not built".

## Index

### Micro: keys and terminal detail

| # | Case |
|---|---|
| 01 | [A typo fixed with editing keys runs the corrected line](micro/01-type-fix-a-typo-and-run.md) |
| 02 | [⌘⌫, ⌘← and ⌘→ edit the shell line as they do in Mac text fields](micro/02-mac-command-line-shortcuts.md) |
| 03 | [Option moves and deletes by word only when Option acts as Alt, which no setting allows](micro/03-option-word-moves.md) |
| 04 | [Tab and Shift-Tab reach the shell, and the terminal keeps focus](micro/04-tab-completion.md) |
| 05 | [↑ and ↓ recall the shell's commands, and the history survives an app restart](micro/05-history-and-reverse-search.md) |
| 06 | [A pasted line reaches the shell as typed, and a multi-line paste waits for my OK](micro/06-paste-text.md) |
| 07 | [A large paste is refused with a reason, and a pasted image is saved privately for the agent](micro/07-large-and-image-paste.md) |
| 08 | [⌃C stops a program, ⌃D ends a shell, and ⌃Z suspends a job that fg resumes](micro/08-interrupt-eof-suspend.md) |
| 09 | [Resizing reaches the program, and a split opens a second group but never a third](micro/09-resize-and-split.md) |
| 10 | [Scrolling holds the view still, and find steps through matches and closes with Esc](micro/10-scrollback-and-find.md) |
| 11 | [A selection copies as plain text, and ⌘C never interrupts a program](micro/11-select-and-copy.md) |
| 12 | [⌘-click opens a link, a plain click does not, and a double-click copies a path](micro/12-open-links.md) |
| 13 | [IME, emoji, wide and right-to-left text reach the shell once and sit in the right cells](micro/13-ime-emoji-wide-and-rtl.md) |
| 14 | [The mouse drives htop and vim, ⇧ selects text instead, and less scrolls its file](micro/14-mouse-in-full-screen-programs.md) |
| 15 | [Neovim gets every key, and the shell comes back as it was when vim or less quits](micro/15-vim-and-less.md) |
| 16 | [Typing over a slow or dropped SSH link is sent in order, dropped while down, and resumes after catch-up](micro/16-typing-over-a-slow-or-dropped-ssh-link.md) |
| 17 | [A shell keeps its program and its output when the app restarts](micro/17-terminal-survives-an-app-restart.md) |
| 18 | [After a Mac reboot, sessions come back in their conversations, and a lost one says so](micro/18-terminals-after-a-host-reboot.md) |
| 19 | [Typing into a terminal that cannot take input is refused, and I can get back to a writable one](micro/19-typing-into-an-exited-or-read-only-terminal.md) |
| 20 | [Two viewers on one shell share its output, each writer hears about the other, and a refresh keeps it running](micro/20-two-viewers-on-one-terminal.md) |
| 21 | [From a focused terminal, the palette moves a session, reaches Settings and opens a file](micro/21-command-palette.md) |
| 22 | [Keyboard navigation opens sessions, folds the sidebar and reaches every sheet](micro/22-keyboard-navigation.md) |
| 23 | [Session tabs open, split and close from the tab row, and the tab shortcuts I expect do nothing](micro/23-session-tabs-and-splits.md) |
| 24 | [Focus follows intent: keys go to the terminal I am in](micro/24-focus-follows-intent.md) |
| 25 | [A wrong archive is undone from the toast, and a typo is fixed with ⌘Z](micro/25-undo-and-mistakes.md) |
| 26 | [A Finder drop uploads into the Files page, and other drops are not built](micro/26-drag-and-drop.md) |
| 27 | [I pick machines, never folders: roots, Copy to a machine, Somewhere specific…](micro/27-roots-and-placement.md) |

### Macro: whole working days

| # | Case |
|---|---|
| 01 | [A teammate's question becomes a Desk session, its doc is copied to the channel, and the session closes](macro/01-pinged-on-chat-start-a-desk-session.md) |
| 02 | [A bug report becomes a channel over three repos, reviewed and pushed](macro/02-pinged-to-fix-a-bug-make-a-channel.md) |
| 03 | [On call: triage a page in a Desk session, dig in on box-1, answer its asks, stop at shift end](macro/03-on-call-shift.md) |
| 04 | [A proposal doc, written by the agent and commented on, ends up in the channel's shared folder](macro/04-write-a-proposal-doc.md) |
| 05 | [A long-running project goes from spec to board items to sessions on two machines, through a reboot](macro/05-long-running-project.md) |
| 06 | [A teammate's pull request is reviewed with an agent: drafts, marks that survive a force push, one submit](macro/06-review-a-pull-request.md) |
| 07 | [A session moves to box-1, a file comes back, and box-1 drops off for an hour](macro/07-working-across-two-machines.md) |
| 08 | [First run: host tools install, the first channel, and box-1 added over SSH with a new host key](macro/08-first-run-and-adding-a-machine.md) |
| 09 | [Stopping every session at day's end holds through a reboot, and the morning start reports the stop](macro/09-end-of-day-stop-and-start.md) |
| 10 | [A power cut mid-work leaves each operation undone or finished, and the restart report says which](macro/10-reboot-or-power-cut-mid-work.md) |
| 11 | [Planning the week on the board: items added, moved and linked, agents move only what they may](macro/11-planning-on-the-board.md) |
| 12 | [Renaming, moving, deleting and restoring files works across machines, and a cut copy finishes or undoes](macro/12-files-across-sessions-and-machines.md) |
| 13 | [A remembered sentence is found in last week's conversation, opened at its turn, pinned; a file is found by name](macro/13-finding-anything.md) |
| 14 | [While this Mac sleeps, I answer box-1's agent over SSH, and the answer reaches this Mac when it wakes](macro/14-host-away-working-on-a-box.md) |
| 15 | [A new release holds box-1 back until its VIP Migrate item is answered, then box-1 upgrades in place](macro/15-upgrade-day.md) |
| 16 | [A month in, I clean up closed Desk sessions, archive and undo, delete a channel, and empty the trash](macro/16-cleaning-up-old-sessions-and-channels.md) |

### Agents: the `humata` CLI and MCP

| # | Case |
|---|---|
| 01 | [Each caller learns who it is, and only the right credential acts](agents/01-who-am-i-and-who-may-act.md) |
| 02 | [An agent's typing goes where it may, and only the user opens shells](agents/02-typing-into-sessions.md) |
| 03 | [A permission prompt waits in the Inbox, and the answer reaches the hook](agents/03-permission-prompt-through-the-inbox.md) |
| 04 | [An ask-ahead grant covers only what was approved, and revoking it makes the next ask wait](agents/04-ask-ahead-and-grants.md) |
| 05 | [Reading other channels follows the rule: Allow lists at once, Ask waits, Never is refused](agents/05-reading-under-the-rules.md) |
| 06 | [An agent's repo proposal becomes an Inbox request, and the repo is cloned only after I approve](agents/06-proposing-a-repo.md) |
| 07 | [An agent forks itself, and each fork kind carries what it says](agents/07-forking-itself.md) |
| 08 | [An agent forks onto box-1 only after I approve, and a failed fork leaves nothing behind](agents/08-forking-onto-another-machine.md) |
| 09 | [A session moves with its work and conversation, and a failed move leaves it where it was](agents/09-moving-a-session.md) |
| 10 | [A conversation roams from Claude to Codex to Copilot, and each picks up where the last left off](agents/10-roaming-between-agents.md) |
| 11 | [An agent that tries to stop the machine is denied, asks, or stops it only when I allow it](agents/11-trying-to-stop-the-machine.md) |
| 12 | [A machine at another release refuses starts, typing and writable attaches until migrated; reads work](agents/12-refused-until-migrated.md) |
| 13 | [A stopped machine ends its agents, refuses their calls, and Start brings sessions back with a report](agents/13-working-on-a-stopped-machine.md) |
| 14 | [A command that needs an answer asks for presence once, waits or exits 12, and a declined change changes nothing](agents/14-waiting-from-the-cli.md) |
| 15 | [A prepared view is a draft until marked ready, then waits for Open or Later, and a restart keeps it](agents/15-preparing-a-view.md) |
| 16 | [The review chat moves my view, leaves notes and orders the review, and I can undo it](agents/16-review-chat-drives-the-view.md) |
| 17 | [A message queued for a turn end waits, then goes once the turn ends](agents/17-queueing-a-message-for-turn-end.md) |
| 18 | [A script with its own token sees and does only what the token lists](agents/18-scripts-with-a-token.md) |
| 19 | [An agent keeps its tab, its Inbox and its conversation across a daemon restart and a Mac reboot](agents/19-an-agent-across-a-restart.md) |
| 20 | [An agent on a box keeps working while the host is away, and each call says what happens](agents/20-an-agent-on-a-box-while-the-host-is-away.md) |

### Real: the real app, the SSH VM and real agent CLIs

[`briefs/real.md`](briefs/real.md) holds this layer's extra parts, its shared facts and the REALFLOW results.

| # | Case |
|---|---|
| 01 | [The SSH VM is added as a machine in the real app and gets Humata's tools](real/01-the-vm-becomes-a-machine.md) |
| 02 | [A shell on the VM in the real app takes every key, copy, paste and resize](real/02-a-shell-on-the-vm-in-the-app.md) |
| 03 | [A dropped SSH link reconnects without replaying input, and an app restart keeps the VM's holders running](real/03-the-vm-link-drops-and-the-app-restarts.md) |
| 04 | [A test puts pinned Copilot and Codex CLIs on the VM in their own homes, and takes them away after](real/04-agent-clis-on-the-vm-for-a-test.md) |
| 05 | [A Copilot session on Claude Haiku 5.5 does a tiny task, asks through the Inbox, and resumes after a stop](real/05-copilot-on-the-cheapest-model-on-this-mac.md) |
| 06 | [The same Copilot conversation moves to the VM, forks back to this Mac, and moves home](real/06-copilot-moves-to-the-vm-and-back.md) |
| 07 | [A Codex session on GPT-6 Luna does a tiny task, asks through the Inbox, and resumes after a stop](real/07-codex-on-the-cheapest-model-on-this-mac.md) |
| 08 | [The same Codex conversation moves to the VM, forks back to this Mac, and moves home](real/08-codex-moves-to-the-vm-and-back.md) |
| 09 | [Claude, Codex and Copilot work in one channel across this Mac and the VM, and each answers, moves and continues correctly](real/09-three-agents-in-one-channel-across-machines.md) |
| 10 | [A script I wrote runs on a new machine as soon as Humata's tools are on it](real/10-a-script-runs-when-a-machine-is-registered.md) |
| 11 | [A script runs once per reconnect, and a session that moves gets its scripts without risk to the session](real/11-scripts-on-reconnect-and-session-arrival.md) |
| 12 | [A dropped link leaves every agent running on the VM, and reconnect re-attaches each with no lost and no replayed input](real/12-the-link-drops-and-every-agent-resumes.md) |
| 13 | [After the VM powers off and after this Mac reboots, each session resumes on its exact conversation, and a restart report names each outcome](real/13-the-vm-and-this-mac-reboot.md) |
| 14 | [Every agent kind forks on this Mac and onto the VM, by code and context, code only, context only and from an earlier turn, and the fork resumes in the real CLI while the parent stays as it was](real/14-forking-every-agent-kind.md) |

### Board: a person and agents plan and track work

[`board/README.md`](board/README.md) holds this layer's shared rig and its findings. The cases go with flow [10](../10-the-board.md).

| # | Case |
|---|---|
| 01 | [An item is made in a column, gets its key, and moves between statuses](board/01-items-and-statuses.md) |
| 02 | [Depends-on edges are set, refuse a loop, and block nothing](board/02-depends-on-edges.md) |
| 03 | [Kanban, Table and Dashboard show the same items, and every figure opens what it counts](board/03-kanban-table-dashboard.md) |
| 04 | [The Graph view draws the depends-on edges](board/04-graph-view.md) |
| 05 | [The drawer shows an item whole and edits its sessions and edges](board/05-the-drawer.md) |
| 06 | [An agent's ask is answered on the item](board/06-approvals-on-the-item.md) |
| 07 | [Who made an item decides who may change it](board/07-ownership.md) |
| 08 | [The scope picker and the session panel show the right boards](board/08-scopes-and-session-panel.md) |
| 09 | [An agent finds out that the Board exists](board/09-agent-learns-the-board-exists.md) |
| 10 | [An agent reads boards through the CLI and through MCP](board/10-agent-reads.md) |
| 11 | [An agent creates, edits, moves and deletes through MCP](board/11-agent-writes-through-mcp.md) |
| 12 | [An agent creates, edits, moves and deletes through the CLI](board/12-agent-writes-through-cli.md) |
| 13 | [An agent asks for approval and waits for the answer](board/13-agent-asks-for-approval.md) |
| 14 | [A real Copilot agent on Claude Haiku 5.5 plans a small task on the Board](board/14-real-copilot-plans-a-task.md) |

### Verify: checks carried into the VERIFY milestone

[`verify/README.md`](verify/README.md) says where each case came from. Results of the real-agent runs are in [`results/`](results/).

| # | Case |
|---|---|
| 01 | [Each agent kind resumes its exact conversation after a stop and after a daemon restart, and `humata session read` shows its entries](verify/01-resume-after-stop-and-restart.md) |
| 02 | [Each agent kind forks on this Mac with code and context, code only, context only, and from an earlier turn, and the fork resumes while the parent stays as it was](verify/02-fork-every-kind-on-this-mac.md) |
| 03 | [`humata attach` picks the agent's tab, and an attach made during a resume waits for it](verify/03-attach-tabs-during-resume.md) |
| 04 | [`humata session type` submits its text once, including after a retry and right after a resume](verify/04-type-submits-once.md) |
| 05 | [A rule on added, connected or upgraded runs its steps once per link-up, and a failed step stops its rule](verify/05-machine-rules-added-connected-upgraded.md) |
| 06 | [A moved session's arrival rule types into it on the new machine, and its leaving rule runs first on the old one without holding the move](verify/06-session-arrived-and-leaving.md) |
| 07 | [Machine scripts are listed in Settings and on the machine page, and Run now updates the row by push](verify/07-scripts-settings-and-machine-page.md) |
| 08 | [`humata machine script` checks the rules, runs a rule and reads its log, and an agent can't run one by default](verify/08-machine-script-cli.md) |
| 09 | [Turning Copilot off in Settings › Agents keeps it out of every new start and leaves the sessions it already runs alone](verify/09-copilot-off.md) |
| 10 | [With Install when missing off, a start of a missing CLI is refused with a fix; with it on, one start installs once, a found CLI is never replaced, and nothing else installs an agent](verify/10-install-when-missing.md) |
| 11 | [A Linux add on stale machine tools is refused until This Mac is migrated, and Migrate This Mac fixes it from the Add machine dialog or from the CLI](verify/11-linux-add-refused-until-migrate.md) |
| 12 | [Adding the real test VM stops at the disk check, 1.3 GiB free against 2 GiB needed, and that is an environment block, not a product bug](verify/12-real-vm-add-disk-low.md) |
| 13 | [Archiving a review keeps its unsubmitted pull request drafts, and restoring it brings them back](verify/13-archive-keeps-pr-drafts.md) |
| 14 | [Each commit and each Save version makes a numbered version, and History says who made it](verify/14-commits-and-saves-make-versions.md) |
| 15 | [Stepping back to an old version reads it as it was, and Restore adds a version and keeps every other one](verify/15-step-back-compare-restore.md) |
| 16 | [Closing a Desk session that holds files asks first, and Save version and close keeps the folder](verify/16-close-with-files-asks-first.md) |
| 17 | [A plain shell on devbox-1 runs as its own Terminal session that never reads Working, and a shell tab beside an agent runs on that machine](verify/17-remote-plain-shell.md) |
| 18 | [Adding a machine through a provider, logging in by hand, and making it unattended leaves a machine that starts without me](verify/18-provider-add-with-login.md) |
| 19 | [A merged pull request says so in its review session and offers Archive, while the other pull request in the review stays open](verify/19-merged-pr-offers-archive.md) |

## Gaps: what is not built

Every **Not built** item from the cases, most user-visible first. "No
milestone" means no milestone or task in `workflow/` owns it yet. Those are the
ones to decide on.

### Every day: keys, tabs and the terminal

| # | Gap | Milestone or ID | Cases |
|---|---|---|---|
| 1 | ⌘⌫, ⌘← and ⌘→ do nothing in a terminal. On a Mac only ⌘C, ⌘V and the ⌘ scroll keys reach the terminal (`terminal/view/src/keys.rs:57-65`). | no milestone | micro-02 |
| 2 | No setting makes Option act as Alt, so ⌥←, ⌥→ and ⌥⌫ never move or delete by word. `set_option_as_alt` has only test callers, and it defaults to Off (`terminal/vt/src/terminal.rs:223, 774`). D73 says a setting decides. | no milestone | micro-03 |
| 3 | ⌘W does not close the active tab, and ⇧⌘↩ does not toggle Terminal full screen. | NV-37 | micro-23 |
| 4 | ⌘T and ⌃Tab are not bound. `register_host_shortcuts` has only a test caller (`terminal/client/src/replica.rs:54-76`). | no milestone | micro-23 |
| 5 | A shell tab cannot be closed: it has no ✕ and no other way to close it (`ui/src/session_tabs.rs:155`). | no milestone | micro-23 |
| 6 | A tab group has no ✕, and a tab cannot be moved from its menu. Only drag moves it. | SE-27 | micro-23 |
| 7 | ⌥⌘→ and ⌥⌘← work only in the sidebar, not in Inbox, Board or diff groups (`ui/src/keys.rs:100-101`). | NV-18, NV-19 | micro-22 |
| 8 | A bell neither shows nor sounds. | no milestone | micro-04 |
| 9 | Pasting an image into an agent terminal in the app (the daemon saves it; the app never sends it). | D30, OVERVIEW I6 | micro-07 |
| 10 | A paste over 256 KiB is refused, never split (`terminal/wire/src/lib.rs:10`, `replica.rs:277`). | no milestone | micro-07 |
| 11 | There is no clear-screen command. ⌘K stays the palette. | no milestone (KP-6) | micro-10 |
| 12 | Nothing says when old output falls off the history limit. | no milestone | micro-10 |
| 13 | A file path or `file:` link in output cannot be opened. Links are only for web and mail URLs (`grid/src/links.rs:1-2`). | no milestone | micro-12 |
| 14 | No consent prompt before a program reads the clipboard with OSC 52. | no milestone | micro-11 |
| 15 | Right-to-left text is not reordered or joined. | no milestone | micro-13 |
| 16 | A pane has no Reconnect control. HO-6 covers only the machine. | no milestone | micro-16 |
| 17 | No "Reconnected; caught up N lines" line. | SE-16 (TERM, MACH) | micro-16 |
| 18 | Terminal snapshots are never compressed or sent in pieces to the app (`clients/terminal_link/src/lib.rs:61`). | no milestone | micro-17 |
| 19 | Humata has no rewind of its own. Only the agent CLI's `/rewind` exists. | SE-10, no milestone | micro-25 |

### Often: files, drops and editing

| # | Gap | Milestone or ID | Cases |
|---|---|---|---|
| 20 | Open in editor, and editing a Markdown file in Humata, are drawn disabled (`ui/src/panel_files.rs:306`, `panel_changes.rs:289`). | SE-17 | macro-04, macro-12 |
| 21 | The session Files panel's Upload… button is disabled (`ui/src/panel_files.rs:313-319`). A Finder drop on the Files page does upload. | no milestone | micro-26, macro-12 |
| 22 | Dropping a file on a shell tab does not type its quoted path. | no milestone | micro-26 |
| 23 | Dropping a file on an agent terminal does not attach it. | no milestone | micro-26 |
| 24 | Dropping a file on the session's Files panel does nothing. | no milestone | micro-26 |
| 25 | A file cannot be dragged out of a pane to Finder. | FM-4 | micro-26 |
| 26 | An archived session cannot be restored or deleted from its page. The daemon RPC exists. | LD-3, LD-4 | micro-25 |
| 27 | An archive does not finish at once when the app quits. | MV-15 | micro-25 |

### After a restart, reboot or stop

| # | Gap | Milestone or ID | Cases |
|---|---|---|---|
| 28 | The app does not show which sessions did not come back, or that a damaged store was restored from its last good copy. | PER-29 | micro-18, macro-10 |
| 29 | The "Restart report" link opens Hosts. There is no report view (`features/app/src/restart.rs:108`). | PER-29 | micro-18 |
| 30 | The restart report does not name a stop the user made. `after_stop` is never read outside the daemon. | no milestone | macro-09 |
| 31 | The app polls for the last start rather than being told. | PER-15, OVERVIEW I7 | macro-10, micro-16 |
| 32 | This computer is not started at login on Linux. Only launchd is built. | PER-18 | micro-18 |

### Moves, machines and viewers

| # | Gap | Milestone or ID | Cases |
|---|---|---|---|
| 33 | A moving session shows no "Moving to <machine>…" line and no step progress. | MV-4, no milestone | micro-19, agents-09 |
| 34 | A Desk session cannot move to another machine. The client says "not in this release" (`clients/live/src/desk.rs:453-455`). | DK-3, MV-17 (ROAM) | macro-01, agents-09 |
| 35 | Take control from a read-only viewer is ignored (`clients/terminal_link/src/lib.rs:103`). | no milestone | micro-19, micro-20 |
| 36 | Machine-offline state for a local tab. `WRITABLE_MACHINE_OFFLINE` is produced only for remote relays (`remote_terminal.rs:591`). | SE-16 | micro-19 |
| 37 | Viewing from a browser, or from a second computer. | D34, ANYHOST (blocked) | micro-20, macro-07 |
| 38 | Another machine's storage cannot be read (`machine_service.rs:650`). | FILES lane | macro-07 |
| 39 | A changed host key cannot be trusted, from the app or the CLI (`machine_service.rs:1400`, `cli/src/commands/hosts.rs:233`). | MACH T10 | macro-05, macro-08 |
| 40 | Host variables. | MACH T10 | macro-05 |
| 41 | A macOS remote has no launchd setup. | MACH T10 | macro-07, macro-08 |
| 42 | Changing a machine's root folder. | FC-10, ST-5, no task | macro-05 |
| 43 | Setting a VM tier. | PER-8, not in 1.0 | macro-05 |
| 44 | Diffs are not served over the peer link. | no milestone | macro-07 |
| 45 | The Add machine dialog does not list hosts from SSH config. | no milestone | macro-08 |
| 46 | The app does not move itself into Applications on first open. The release tool does that. | no milestone | macro-08 |

### Channels, sessions and cleanup

| # | Gap | Milestone or ID | Cases |
|---|---|---|---|
| 47 | The repo picker has no "used before" source and no GitHub users and orgs source from Settings. | NEWCH, CH-18 | macro-02 |
| 48 | A repo cannot be removed from a channel, from the app, the CLI or MCP (`channel_service.rs:1121-1142`). | CH-22, DESK batch D4 (MV-9) | macro-02, agents-06 |
| 49 | ~~The Repos and machines grid (`GetGrid`).~~ Built in the daemon and `humata channel grid` (D764); the app's Repos and machines tab is not built yet. | CH-9 | macro-02 |
| 50 | A repo cannot be added to a running session (`session_service.rs:2366-2372`). | no owner (ROAM waits on it) | agents-06 |
| 51 | A session cannot be deleted, and a repo cannot be dropped from one (`session_service.rs:2331-2386`). | no milestone | macro-16 |
| 52 | The app cannot delete a channel. Only the CLI can. | no milestone | macro-16 |
| 53 | A channel that other machines hold, or that has live sessions elsewhere, cannot be deleted (`channel_service/delete.rs:425-433`). | no milestone | macro-16 |
| 54 | The Storage page has no Empty trash, and deleted sessions and channels have no restore screen. | no milestone | macro-16 |
| 55 | A session cannot be handed to another person. | no milestone | macro-03 |

### Inbox and permissions

| # | Gap | Milestone or ID | Cases |
|---|---|---|---|
| 56 | A permission card has no "Not now". | no milestone | macro-03 |
| 57 | A permission card has no Always allow. An Always answer is refused (`inbox/permission.rs:238`). | no milestone | agents-03 |
| 58 | A CLI command that needs presence gets no Allow or Don't Allow card in the app. | no milestone | agents-14 |
| 59 | An agent cannot ask ahead to write into a folder. Writes into another session's clone are refused (`policy.rs:728-729`). | no milestone | agents-04 |

### Review, views and the board

| # | Gap | Milestone or ID | Cases |
|---|---|---|---|
| 60 | The "Review a pull request" dialog. | PR C1, C5, D1 (D197) | macro-06 |
| 61 | A pasted PR link does not open Review this PR. The palette offers it only for a PR a session already reviews. | PR-1 (D197) | macro-13 |
| 62 | The review chat does not move the app's view, and there is no Undo, Back, or Info panel switches. No app crate calls `WatchReviewChanges`, `UndoReviewChange`, `FocusReview` or `SetAgentSwitches`. | PR C2, B2, PR-25 | macro-06, agents-16 |
| 63 | The PR commit list and the "only what changed" view. | PR C4, D2 | macro-06 |
| 64 | The Submit review screen. | PR C3, D3 | macro-06 |
| 65 | The "Draft for GitHub" and "Make it a draft" buttons. The daemon accepts both. | PR C6 | macro-06 |
| 66 | A note cannot be resolved, reopened or reattached (`review.rs:981, 998, 1007`). | no milestone | macro-04, agents-16 |
| 67 | Views: the offer bar's Open, Later and Not now, Revert and Keep, the tab's contents and the Journal states. | VIEWS (VW-12) | agents-15 |
| 68 | `ViewStateService` answers every call "not built". | no milestone | agents-15 |
| 69 | The board drawer cannot edit an item's title or description. | D177, waits on the owner | macro-11 |
| 70 | A board item cannot be moved by keyboard or deleted by the user. | no milestone | macro-11 |
| 71 | No graph view of dependencies. BD-14 is a list. | no milestone | macro-05 |

### Finding and upgrades

| # | Gap | Milestone or ID | Cases |
|---|---|---|---|
| 72 | ⌘K inside a focused agent terminal. | KP-6, FIND §26.22 step 1 | macro-13 |
| 73 | The multi-machine live check of finding things. | FIND §26.22 step 3 | macro-13 |
| 74 | `humata sessions search` has no `--channel` or `--machine`. | FIND T5a | macro-13 |
| 75 | The session ⋯ menu has no Pin or VIP (`ui/src/channel.rs:179`). | NV-20, no milestone | macro-13 |
| 76 | The Migrate Inbox card, its Migrate action, and its phases. | UPGRADE U7 | macro-15 |
| 77 | Sessions on an old holder do not move to the new one when idle. | UPGRADE U6 | macro-15 |
| 78 | The app does not say "add it again" for an identity mismatch, or show last seen and next retry. | LD-12 | macro-15 |

### Agent surface: CLI and MCP

| # | Gap | Milestone or ID | Cases |
|---|---|---|---|
| 79 | MCP tools advertised but not dispatched: `session_move`, `files_find`, `channel_add_repo`, `session_own_copy`, `machine_stop`, `session_start`. `topology_get` is built (D772). The rest fall to `not_built` in `daemon/humatad/src/mcp.rs` (`serves()`). | MV-1 for move; the rest no milestone | agents-06 to 13 |
| 80 | Typing queued for a turn end: `Type` with `TURN_ENDS`, `ListQueued`, `SendQueuedNow`, `CancelQueuedTyping` (`daemon/humatad/src/terminal.rs:911-914`). | SE-23 (optional, D126) | agents-02, agents-17 |
| 81 | An agent's fork cannot be made before a turn, with only some repos, with a first message, on a named branch, or with another agent. The daemon supports most of these over gRPC. | no milestone | agents-07, 08, 10 |
| 82 | A fork's Inbox card names no branch (`inbox.proto:293-297`). | no milestone | agents-07, 08 |
| 83 | Codex and Copilot as built-in agent kinds, carrying their conversations on fork and move, and the Copilot adapter. Only claude and opencode exist (`daemon/sessions/src/agents.rs:139-166`). | ROAM batch 3, C4, X2 | agents-10 |
| 84 | An agent reading another channel under Ask is refused, not asked (`history_service.rs:331-339`). | FIND T4b | agents-05 |
| 85 | An agent cannot list another session's turns from MCP or the CLI, and `humata conversation` calls the unbuilt `ReadConversation`. | no milestone | agents-05 |
| 86 | `humata inbox wait` does not exist. The wait is `--wait` or MCP `inbox_wait`. | no milestone | agents-14 |
| 87 | Settings › Security has no screen for script tokens. The daemon side is built; only the CLI can make or revoke one. | ST-17, no milestone | agents-18 |
| 88 | Messages waiting for a turn end live only in the app. The daemon and CLI never see them, and `humata turn` does not send them. | no milestone | agents-17 |

## Likely bugs

Each one was found by reading the code. Each says where to look and how to
confirm it. Items marked **(coach-checked)** were re-read by the coach.

### Terminal

1. **⌘-click on a link opens nothing, and pasted images, pasted Finder files, bells, titles and folder changes are dropped.** `ui/src/terminals.rs:178-212` handles only Focused, SplitRequested, ClipboardWrite, Failed and InputUncertain; the rest hit `_ => {}`. The lab client handles OpenLink (`terminal/client/src/lib.rs:452-455`). Confirm: ⌘-click a URL in a shell tab; ⌘V a copied Finder file. (coach-checked) — micro-04, 07, 12, 26
2. **The app drops the daemon's notices.** `clients/terminal_link/src/lib.rs:191` drops Notice frames, so "connection lost, the session keeps running" (`remote_terminal.rs:596`) and the Exited notice never show. A shell tab shows no exit line; only tab 0 draws one (`ui/src/session_tabs.rs:632`). Confirm: `exit` in a shell tab. (coach-checked) — micro-08, 16, 19, macro-07
3. **An exited shell shows "Lost the connection…".** `terminal/client/src/replica.rs:530-536` treats exited as not authorized, and the pane draws the disconnected banner (`terminal/pane/src/pane.rs:903`). Confirm: `exit` in a shell tab. (coach-checked) — micro-08, 19
4. **Keys typed before the screen arrives, or into a read-only, exited or moving tab, are dropped with no message.** `terminal/client/src/replica.rs:305-307`, `terminal/pane/src/host.rs:119-122`. Confirm: type into a tab right after opening it. — micro-17, 19
5. **Edit › Copy, Edit › Paste and Edit › Select All do nothing in a terminal.** `desktop/src/main.rs:244-246` routes them to input actions the terminal does not handle; its input handler reports no selection (`terminal/view/src/view.rs:1541-1557`). Confirm: use the menu with a terminal focused. — micro-01, 06, 11
6. **OSC 52 clipboard reads get an empty clipboard with nobody asked.** `ui/src/terminals.rs:211`, `terminal/host/src/lib.rs:2111-2121`. — micro-11
7. **Option-as-Alt is Off with no way to change it.** `terminal/vt/src/terminal.rs:223`. — micro-03
8. **⇧ does not stop the wheel from reaching a mouse-tracking program.** `wheel_bytes` sends mouse reports whenever the program tracks the mouse, whatever the modifiers (`terminal/pane/src/host.rs:169-191`). Confirm: ⇧-scroll in htop. (coach-checked) — micro-14
9. **A program that exits with mouse mode on leaves the shell getting mouse reports.** No reset at `terminal/pane/src/host.rs:281-283`. — micro-14
10. **⌘PgUp is eaten by the app in a full-screen program.** `terminal/view/src/keys.rs:61`, `terminal/pane/src/host.rs:336-349`. Confirm in htop. — micro-10
11. **An IME composition survives a disconnect, and its commit is dropped silently.** `terminal/pane/src/pane.rs:256-266`, `host.rs:350-355`. — micro-13
12. **The IME candidate window opens at the live cursor while scrolled back.** `terminal/view/src/view.rs:1609-1628`. — micro-13
13. **Double-clicking `src/parse.rs:42` selects only the path.** `:` is a word boundary (`terminal/view/src/selection.rs:10`). Decide whether that is wanted. (coach-checked) — micro-12
14. **Every writable viewer asks for a resize, so two windows of different sizes take turns sizing the shell.** `ui/src/terminals.rs:43` (`FitPolicy::Controlling`), `replica.rs:326-330`, daemon applies the last (`attach.rs:336-345`). D271 point 3 says otherwise. — micro-20
15. **The other-writer count never reaches the app.** `clients/terminal_link` does not map `other_writers`, and `terminal_peers` (`clients/src/workspace.rs:35`) has no live producer. — micro-20
16. **"Reconnected; caught up N lines" would always say 0.** `caught_up_lines: 0` at `attach.rs:294` and `remote_terminal.rs:603, 634`. (coach-checked) — micro-16
17. **A pasted image retried after a daemon restart is saved twice.** The retry memory is in-process only (`daemon/humatad/src/terminal.rs:694-710`). The same holds for `session_type`'s idempotency key. — micro-07, agents-02

### Keys and navigation

18. **On Linux no app shortcut works with Ctrl.** Every binding in `ui/src/keys.rs:94-118` is `cmd-`, which is Super on Linux, so Ctrl+K opens nothing. (coach-checked) — micro-21, 22
19. **The palette shows shortcuts that are not bound:** ⌘4 (Hosts), ⌘5 (Files) (`features/palette/src/lib.rs:673-674`) and ⇧⌘↩ (`:699-701`). (coach-checked) — micro-21, 23
20. **A second ⌘0 opens the hidden sidebar as an overlay instead of docking it** (`ui/src/shell.rs:515-521`), and **⌘L with the sidebar hidden focuses a sidebar that is not drawn** (`:535-537`). — micro-22
21. **Move in the palette with no other online machine says "Nothing matches. Try fewer letters."** and gives no label while it asks for a machine (`ui/src/overlay_palette.rs:191`). — micro-21
22. **A palette search with no hits shows nothing.** The "nothing found" helper is used only for errors (`ui/src/palette_hits.rs:41-43, 121`). — macro-13
23. **The palette's Files group is empty on a real host.** The live workspace never fills `session_files` (`clients/live/src/daemon.rs:269-279`). (coach-checked) — macro-13
24. **⌥⌘M does nothing, silently, when no session is open** (`ui/src/shell.rs:541-549`). Low. — macro-07
25. **The side panel's collapsed state is one value for the whole app**, not per session as NV-37 and SE-13 say (`features/app/src/layout.rs:72-77`). — micro-23
26. **A session archived from another window keeps its page open here.** `Delegate::Left` is declared but never sent (`features/session/src/lib.rs:271`). (coach-checked) — micro-24
27. **The focus guard leaves out the Desk sheet and the Add machine dialog**, so a terminal can take focus from them (`ui/src/shell.rs:278-282`). Unconfirmed. — micro-24
28. **The archive toast's Undo goes away after 4 s, while the undo window is 10 s.** `features/app/src/lib.rs:65` vs `daemon/humatad/src/session_service/archive.rs:40` and MV-15. (coach-checked) — micro-25

### Sessions, restarts and machines

29. **Every resumed idle session says "Was mid-turn when its host restarted."** The daemon has no mid-turn field (`ui/src/restart_report.rs:153`). (coach-checked) — micro-18
30. **A moving session shows as Idle.** `clients/live/src/map.rs:198`; a unit test pins it (`:384`). (coach-checked) — micro-19, agents-09
31. **Typing into a moving session is not refused.** The Type RPC has no moving check (`daemon/humatad/src/terminal.rs:877-915`), unlike attach (`attach.rs:323`). — micro-19
32. **The restart report is read once, on a session event, and may be missed.** `clients/live/src/events.rs:115`, `clients/live/src/daemon.rs:81-95` vs `daemon/humatad/src/restart.rs:174`. — macro-10
33. **A fork's conversation copy and a moved folder's rename are not power-cut safe.** `daemon/sessions/src/fork.rs:505-530` and `own_copy.rs:360` skip `humata2_durable`. SAFE T6b is pending. — macro-10
34. **Writes and moves to a machine at another release are not refused.** Only start, type and writable attach call `machine_ready`. File forwarding (`files/forward.rs:471-480`) and moves (`move_session.rs:162`) do not. UPGRADE.md:48 says they are. (coach-checked) — agents-12
35. **A failed Inbox withdrawal still reports the machine stopped**, so a request can stay pending (`daemon/humatad/src/stop/boot.rs:83-86`). Low. — macro-09
36. **A script's stop is recorded as an agent's, with no session** (`stop/service.rs:184-188`). Low. — macro-09
37. **The changed-key message says "trust the new key", but the Add dialog shows no card for it.** `daemon/machines/src/error.rs:275` vs `machine_service.rs:971-990`. And the Confirm card cannot show a different pinned key, so Confirm replaces it (`ui/src/overlay_add_machine.rs:582`, `known_hosts.rs:45`). — macro-08
38. **A rate-limited `gh` is told to run `gh auth login`.** Every `Failed` gets that fix (`channel_service/gh_repos.rs:71-73`). It should say to wait. (coach-checked) — macro-02

### Inbox, review, files and the board

39. **The Reply box on an Inbox card cannot be typed into.** Reply… opens it, but the draft is drawn as text and nothing sends `ReplyText` (`ui/src/request_card.rs:339-350`), so Send reply denies with an empty note. (coach-checked) — macro-03, agents-04
40. **A comment with no destination is told "CreateComment for a pull request is not built".** `daemon/humatad/src/review.rs:849-855`. The PR path is built. (coach-checked) — macro-06
41. **In the app's review model, a sent comment whose passage is gone becomes Addressed, not a detached draft** (`review/src/model/comments.rs:676-682`). The daemon gets it right. Check which one the Diff panel draws. — macro-04
42. **A user's own edit to a commented passage marks the comment addressed**, which SM-16 forbids (`daemon/humatad/src/review/comments.rs:657-660`). — macro-04
43. **Two writers to one file overwrite each other.** `WriteFile` has no base-version check (`files.proto:145-151`, `daemon/humatad/src/files.rs:245`). — macro-04
44. **A multi-file or folder drop stops at the first failing file while earlier copies carry on**, so it is not all or nothing (`clients/live/src/file_manager.rs:880`). (coach-checked) — micro-26
45. **The board's Status menu offers Blocked and Won't do, which the live client refuses.** `ui/src/board.rs:1131-1139` lists `ItemStatus::ALL`; `clients/live/src/boards.rs:31-39` has no column for either. (coach-checked) — macro-11
46. **A view marked Not now still shows in the user's list.** `every_offered` filters only Draft (`daemon/humatad/src/review/views.rs:235-247`). VW-5 says Not now hides it. (coach-checked) — agents-15
47. **A deleted session keeps its pin, shown by id.** `Marks::forget` has no caller (`daemon/humatad/src/mark_service.rs:127-146`). — macro-13

### Agent and CLI

48. **38 of the 81 MCP tools in `tools/list` answer "not built".** `daemon/mcp/src/tools.rs:274-281` advertises them all; `daemon/humatad/src/mcp.rs:1020-1064` dispatches 43. Agents will try them. — agents-06 to 13
49. **The reason an agent gives for a fork or a channel repo add is dropped.** The fork schema requires it, the handler never reads it, and the card has an empty reason (`tools.rs:1111`, `mcp.rs:698-737`, `fork_card.rs:75`). `AddRepoRequest` has no reason field (`channels.proto:118-122`). — agents-06, 07
50. **A fork to an unknown machine, on a host with no boxes, says "not built" and tells the agent to update** (`fork.rs:355-357`, `fork_remote.rs:198-200`, `mcp.rs:1254-1260`). Several other unbuilt modes also say "update Humata" (`status.rs:131-135`), which cannot help. — agents-02, 08
51. **`session_type` is documented as one bracketed paste but types raw.** `daemon/humatad/src/terminal.rs:920`; the comment at `:1157` says raw. One must change. (coach-checked) — agents-02
52. **A stopped machine's refusal says nothing the caller can do will help**, but the user can start it. `daemon/access/src/verdict.rs:338-340` maps MachineStopped to `Fix::Nothing`. (coach-checked) — agents-13
53. **Declining "Continue?" exits 130, "interrupted", with no message.** 130 is for ⌃C (`cli/src/error.rs:151`). (coach-checked) — agents-14
54. **The CLI asks for a passphrase to answer a request, but the daemon never reads it**, so the retry is refused again (`cli/src/call.rs:232-238`, `daemon/humatad/src/inbox.rs:807`). — agents-14
55. **A wait that ends with no answer exits 70 and blames a version mismatch**, though the request is still pending (`cli/src/error.rs:292`). — agents-14
56. **A wrong away passphrase gets the generic refusal and loses the tries-left count** (`daemon/humatad/src/cli_login.rs:337-339`). — macro-14
57. **The lock message gives the unlock time in raw Unix seconds** (`daemon/humatad/src/status.rs:369-376`). (coach-checked) — macro-14
58. **A Codex parent's handover falls back to scrollback.** The digest reads only Claude's JSON shape (`session_service/start_fresh.rs:891-892`). Latent until Codex is a built-in kind. — agents-10
59. **A queued message is typed into a Claude permission prompt.** Every status but Working maps to `Turn::Ended` (`clients/live/src/map.rs:289-292`), and the app sends Claude's queue at Ended (`features/session/src/lib.rs:673-674`), so Needs you counts as a turn end. Confirm: queue a message while Claude waits on a permission card. (coach-checked) — agents-17
60. **OpenCode never sends its queue, and Copilot sends it about 3 s after output stops.** Only Claude and Copilot send at Ended (`features/session/src/lib.rs:673-674`). OpenCode is hook-backed (`daemon/sessions/src/agents.rs:116`), and Copilot's output fallback (`agents.rs:406`) is what SE-23 forbids. — agents-17
61. **`Turn::Uncertain` is never produced live**, so the "not sure the turn ended" state is only in fixtures (`clients/src/fixture/sessions.rs:45`). — agents-17
62. **A queued multi-line message is typed raw, not as a bracketed paste** (`clients/live/src/daemon.rs:702-716`, `daemon/humatad/src/terminal.rs:1158-1160`). SE-23 asks for a paste. — agents-17
63. **An app that quits mid-send can type a queued message twice.** Each send gets a new idempotency key (`features/session/src/lib.rs:705`, `clients/live/src/connection.rs:323-327`). — agents-17
64. **MCP's refusal for `turn_end` tells the agent to update Humata or use the app** (`daemon/humatad/src/status.rs:131-140`). No update adds it, and an agent cannot use the app. — agents-17
65. **Making or revoking a script token never checks presence**, so the CLI's presence step is skipped (`daemon/humatad/src/cli_login.rs:489-493, 603-607`, `cli/src/call.rs:185-199`). Owner to confirm whether presence should gate these. — agents-18
66. **A token's channel list does not limit a machine stop** (`daemon/access/src/policy.rs:1190`). SEC-14 says a token lists its channels. This may be meant; the owner should decide. — agents-18
67. **The CLI drops the age header on a board read made from a box's copy**, so a copied read shows no age. `cli/src/live.rs:110-117` keeps only the body; MCP keeps it (`daemon/humatad/src/mcp/boards.rs:55-68`). (coach-checked) — agents-20

## Docs that look stale

- `OVERVIEW.md:457` lists Files Upload as missing; a Finder drop uploads.
- `OVERVIEW.md:458` says the Reply editor is disabled; the button works but the box takes no typing.
- `OVERVIEW.md:537` says "Couldn't resume" is not drawn; `ui/src/session_tabs.rs:681-688` draws it.
- `OVERVIEW.md:566` says the away uplink carries nothing; `daemon/humatad/src/away/uplink.rs` sends the journal.
- `PR.md` "What exists" and `REVIEW.md` ("MCP review_draft tools not built") are behind the code.
- `FIND.md` says channel-session MoveSession is not built; `session_service/move_session.rs` and its tests exist.
- `FORK.md:47` says own-copy works over MCP; there is no MCP arm.
- `api/CLI.md:117` marks `humata conversation` stale; it still calls the unbuilt `ReadConversation`.
- `daemon/humatad/tests/desk.rs` calls DeskService parked; it is served.
- `clients/live/tests/boards.rs::a_session_started_for_an_in_review_item_leaves_its_column` asserts the item stays.
- `OVERVIEW.md:576` lists script tokens as open; the daemon side is built (ASK batch 4). Only the Settings screen is open.
- `api/MCP.md:251` leaves out `turn_end_uncertain`, which `mcp.rs:1008` returns. Neither queue field is ever set.
- The doc comment on Submit (`features/session/src/lib.rs:432`) says it sends now at turn end; it queues for every agent but Claude and Copilot.

## Proof gaps (tests, not features)

- No terminal key, paste or mouse behaviour is proven in the app against a real shell. The lab tests use the standalone client, and they are manual.
- No live test covers ⌃Z and fg, less, htop, a mouse wheel, or quitting Neovim with `:wq`.
- `terminal/lab/tests/interactive.rs:476` expects at least 12,000 rows, while the default history is 10,000 lines (`terminal/wire/src/lib.rs:42`). The lab's own settings may differ. Run it to see which is true.
- The peer link between machines has never run over real SSH. Only `terminal/host/src/roaming_tests.rs` and the lab use real SSH.
- `gh`, agents and boxes are fakes in every daemon test. Real GitHub and real agents are unproven.
- The power-cut suite (SAFE T5) is written but not gated. STOP T4 to T7 are written but not gated.
- No live PR review test exists (`desktop/tests/live_flows/pr_review.rs`, PR E1).
- No live test covers a session's tab row, the side panel toggle, or a palette move with a machine picked.
- After a Mac reboot a tab gets a fresh token (`daemon/humatad/src/restart.rs:340-400`); no test covers it. No test covers a restart while a hook holds a card, or that an interrupted turn is not repeated.
- No test covers a script's machine stop, or a script token used across two daemons.
- HOOKS live checks are owed (`workflow/milestones/HOOKS.md:45`).

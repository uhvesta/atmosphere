# Map of Humata 2 use cases

This map lays out the use cases in three layers. **Micro** cases are single
keystrokes and gestures. **Macro** cases are whole working days. **Agents**
cases cover an agent acting through the `humata` CLI or MCP. **Real** cases
run the real desktop app, real SSH to the `h2-terminal-c1` VM, and the real
Claude, Codex and Copilot CLIs.

Each item names its branch points and what should happen at each one. The
cases in `micro/`, `macro/`, `agents/` and `real/` write each item out as a prose test
case. `README.md` indexes them and lists the gaps.

These branch points recur through the map:

- **Error**: the step fails. The message says what failed and how to fix it.
- **Offline**: the machine or link is gone. Nothing is lost, and the screen
  says it is offline. Input is refused, not silently dropped.
- **Wrong choice**: the user picks the wrong thing. They can step back with
  Esc or Undo, or the choice is refused before it does harm.
- **Permission**: the step needs the user. An Inbox request appears, and the
  caller waits for the answer or is refused.
- **Concurrent**: two writers or two answers arrive at once. The first one
  wins, and the second hears about it.
- **Restart or reboot**: the app, daemon or host restarts in the middle of the
  step. The step is either undone or finished, never left half done, and a
  restart report says which.

## Micro: typing and terminal detail

| # | Case | Branch points and expected outcomes |
|---|---|---|
| 01 | Type a command, fix a typo, run it | **Branch points:** Backspace, ⌃H, ⌃W, ⌃U, ⌃K, ⌃A/⌃E, and arrows to fix a typo in the middle of the line. **Expected:** the shell gets the standard control bytes, and Enter runs exactly the corrected line. **Offline:** input is dropped and the pane says so. |
| 02 | Mac line-editing shortcuts | **Branch points:** ⌘⌫, ⌘← and ⌘→. **Expected:** they delete to the line start, or jump to the start or end. **Today:** the terminal ignores them (likely bug). |
| 03 | Word moves with Option | **Branch points:** ⌥←/⌥→, ⌥B, ⌥F, ⌥D and ⌥⌫, with plain zsh, plain bash and a configured zsh. **Expected:** word moves. **Today:** Option types ∫, ƒ and ∂, because Option-as-Alt can't be turned on. |
| 04 | Tab completion | **Branch points:** one match, many matches with a double Tab, no match (bell), Shift-Tab in a menu, and completion over SSH while the link lags. |
| 05 | History and reverse search | **Branch points:** ↑/↓ in normal and application cursor mode, ⌃R, ⌃P/⌃N, and a history that is still there after the app restarts. |
| 06 | Paste text | **Branch points:** one line, many lines with bracketed paste on and off, a multi-line paste that waits for confirmation, a paste that tries to end bracketed paste early, and a paste into vim. |
| 07 | Large and image paste | **Branch points:** the pending-input cap (1 MiB or 1024 events), an image paste saved privately, an image over the limit refused, and a retry after a dropped paste. |
| 08 | Interrupt, end, suspend | **Branch points:** ⌃C during runaway output, ⌃D on an empty and a non-empty line, ⌃Z then `fg`, ⌃\, ⌃S/⌃Q, and ⌃C on every platform even when text is selected. |
| 09 | Resize and split | **Branch points:** window resize, ⌘\ split, the two-group limit, the program hearing the resize, a vertical resize over SSH, and two viewers of different sizes. |
| 10 | Scrollback and find | **Branch points:** wheel, ⇧/⌘ PgUp and Home, the view holding while output streams, ⌘F, ⌘G/⇧⌘G, "No matches", a bad regex, and ⌘K, which opens the palette instead of clearing. |
| 11 | Select and copy | **Branch points:** drag, double- and triple-click, ⌥-drag block selection, a selection across a wrapped line, ⌘C versus ⌃C, Ctrl+Shift+C on Linux, a program writing the clipboard (OSC 52), and copy over SSH. |
| 12 | Open links | **Branch points:** ⌘-click a URL, OSC 8 links, a plain click that must not open, double-click to copy a path, and a link to a file on a remote machine. |
| 13 | IME, emoji, wide and RTL text | **Branch points:** an IME preedit and commit, emoji and wide-character width, combining marks, and Persian or Arabic text. **Note:** IME is untested in a real window. |
| 14 | Mouse in full-screen programs | **Branch points:** a TUI takes the mouse, ⇧ overrides it, the wheel sends arrows on the alternate screen, and focus is reported to the program. |
| 15 | vim and less | **Branch points:** open, edit, resize in vim, suspend vim with ⌃Z, colors, and quitting, which restores the shell screen. |
| 16 | Typing over a slow or dropped SSH link | **Branch points:** lag, the link dropping mid-word, input refused while disconnected, Reconnect, and an echo that arrives late. |
| 17 | Terminal survives an app restart | **Branch points:** quit while output is streaming, reopen, catch up from the cursor, type only once synced, and keep the scroll position. |
| 18 | Terminal after a host reboot | **Branch points:** the shell comes back, the agent resumes its conversation, a conversation is lost ("Couldn't resume"), and the restart report. |
| 19 | Typing into an exited or read-only terminal | **Branch points:** the shell has exited, the session is moving (read-only), there is a read-only attach, and the machine is stopped. |
| 20 | Two viewers on one terminal | **Branch points:** desktop and browser mirror the same holder, both type, writers hear about each other, and a refreshed viewer keeps its holders. |
| 21 | Command palette | **Branch points:** ⌘K, fuzzy ranking, Esc stepping back, an inline machine pick, pasting a PR URL, a settings search, a file result, and keys that must not leak into the terminal. |
| 22 | Keyboard navigation | **Branch points:** ⌘L, arrows, Enter, ⌥⌘→/←, ⌘0, ⌘1/2/3, ⌘[, ⌘/, ⌘, and Esc. |
| 23 | Session tabs and splits | **Branch points:** plus menu, split, join, close by mouse, the panel (⌥⌘0), and the missing ⌘W, ⌘T, ⌃Tab and ⇧⌘↩. |
| 24 | Focus follows intent | **Branch points:** a new session gets input, a click on a split focuses it, an unfocused terminal ignores keys, and focus returns after an overlay closes. |
| 25 | Undo and mistakes | **Branch points:** Undo after archiving, Undo in text fields, rewind for an agent turn, and a wrong channel or machine picked. |
| 26 | Drag and drop | **Branch points:** a Finder drop into Files uploads, a drop onto a terminal, a drop onto the composer, and a drop that is too large. |

## Macro: whole days

| # | Case | Branch points and expected outcomes |
|---|---|---|
| 01 | Pinged on chat: start a desk session and look | **Branch points:** a new desk session from the form, a folder named from its title, cloning a repo or not, a clone that fails, closing, reopening, and a document copied to a channel. |
| 02 | Pinged to fix a bug: find the repos and make a channel | **Branch points:** the repo picker with fuzzy search, a new channel, sessions per repo, a fork to try a second fix, a review of the diff, and a push gated by the rules. |
| 03 | On call: open the on-call channel, paste into a desk, make a session | **Branch points:** a pinned channel, a pasted log or image, a session started on the right machine, the machine offline, and Stop all at the end of the shift. |
| 04 | Write a proposal doc | **Branch points:** a desk session for a doc, an agent drafting it, document comments, a review of the doc, a copy into a channel's shared folder, and the doc edited while the agent writes. |
| 05 | Long-running project: spec to milestones to sessions to a remote machine | **Branch points:** board items with dependency edges, sessions spawned per milestone, a move to a box, a new remote machine, and a reboot in the middle. |
| 06 | Review a pull request | **Branch points:** paste the PR URL, a review channel, the review chat drives the view, drafts, submit, a force push, a rate limit, and a crash after posting. |
| 07 | Work across two machines | **Branch points:** this Mac and a box, a session moved, an offline box, a fork onto the box, files copied across, and a desktop and a browser at once. |
| 08 | First run and adding a machine | **Branch points:** the installer, the first channel, Add machine over SSH, a host key, a box that can't be reached, and missing host tools. |
| 09 | End of day: stop everything, start next morning | **Branch points:** Stop all sessions…, terminals attached at the time, a pending Inbox, a restart while stopped, and Start sessions with its report. |
| 10 | Reboot or power cut in the middle of work | **Branch points:** a cut during a fork, move or save, the last good copy, the restart report, "Couldn't resume", and Try again. |
| 11 | Plan on the board | **Branch points:** items, drag between columns, dependencies, an agent adding items, an edit while the host is away, and edits that conflict. |
| 12 | Files across sessions and machines | **Branch points:** the Files page, copy, move, rename, delete to the Trash, a Finder upload, Open in editor, and another machine's files. |
| 13 | Find anything | **Branch points:** the palette, a turn search, a conversation found, pins, and results on an offline machine. |
| 14 | The host is away while I work on a box | **Branch points:** the passphrase login, asks held or skipped, writes journaled, and a replay when the link returns. |
| 15 | Upgrade day | **Branch points:** VIP "Migrate X" items, refused starts, an agent that can't answer, Answer all, and the machine still readable. |
| 16 | Clean up old sessions and channels | **Branch points:** Clean up…, archive with Undo, work not pushed holds a folder, Trash restore, and a delete that is not built. |

## Agents: what an agent can and can't do

| # | Case | Branch points and expected outcomes |
|---|---|---|
| 01 | Who am I, and who may act | **Branch points:** a session token, a script token, a login, nobody, `whoami`, and an unknown token. |
| 02 | Type into a session | **Branch points:** its own session, another session (Ask), an exited shell, a shell or paste (user only), and a closed tab's token. |
| 03 | A permission prompt goes to the Inbox | **Branch points:** a hook holds, Allow, Deny with a note, Always, the same ask twice, the hook stops, the daemon is gone, and asks turned off. |
| 04 | Ask ahead and grants | **Branch points:** a folder grant, Always only for the target, revoke, Approve all skipping ask-aheads, and a grant still there after a restart. |
| 05 | Read other sessions and channels under the rules | **Branch points:** the default rule, Ask, Never, and a deferred call that runs or never runs. |
| 06 | Propose a repo for the channel | **Branch points:** the proposal becomes a request, approving clones it, another channel is denied, a local path is refused, and the user has already added it. |
| 07 | Fork itself | **Branch points:** code and context, code only, context only, at a turn, another agent, a cut at each step, and nesting. |
| 08 | Fork onto another machine | **Branch points:** an offline box, a branch name that is bad or taken, a context-only fork refused across machines, a preview, and a host restart. |
| 09 | Move a session | **Branch points:** the copy fails, an offline target, a cancel, the terminal read-only while moving, and a cut at each step. |
| 10 | Roam a conversation between Claude, Codex and Copilot | **Branch points:** a handover with a summary, the layout found, an unknown agent refused, and a cut after launch typed once. |
| 11 | Try to stop the machine | **Branch points:** the default rule denies, Ask, Allow, the agent can't start the user's stop, and a stop without a token. |
| 12 | Refused until migrated | **Branch points:** start, type and attach are refused with a Migrate item, a CLI at another release is refused, and reads still work. |
| 13 | Work on a stopped machine | **Branch points:** agents and scripts are refused, pending requests are withdrawn, and Start brings sessions back. |
| 14 | Wait from the CLI | **Branch points:** presence asked in the app, a passphrase, `--json` exits 12, `--wait`, ⌃C exits 130, and denied. |
| 15 | Prepare a view for the user | **Branch points:** draft, revise, offer, Open or Later, dismissed, and kept through a restart. |
| 16 | Review chat drives the view | **Branch points:** focus, notes, the review order, the user's undo, and turning it off. |
| 17 | Queue a message for turn end | **Branch points:** the hook delivers it at turn end, an uncertain turn end, offline, and the daemon's RPCs not built. |
| 18 | Scripts with a token | **Branch points:** `HUMATA_TOKEN`, a capability missing, a stop refused, and token minting. |
| 19 | An agent across a restart | **Branch points:** the token is kept, a pending deferred call is withdrawn, an ask-ahead waits, and the hook prints nothing while the daemon is gone. |
| 20 | An agent on a box while the host is away | **Branch points:** its own files, a question journaled, a push denied or held, reads from the copy, and a replay when the link returns. |

## Real: the real app, SSH VM and agent CLIs

| # | Case | Branch points and expected outcomes |
|---|---|---|
| 01 | The SSH VM is added as a machine in the real app and gets Humata's tools | **Branch points:** add over SSH, the host key, tools installed (Linux tools missing from the bundle), and a tools copy that fails. |
| 02 | A shell on the VM in the real app takes every key, copy, paste and resize | **Branch points:** a shell, vim and less, Esc and ⌃ keys, copy and paste, resize, and a key sent while the link is down. |
| 03 | A dropped SSH link reconnects without replaying input, and an app restart keeps the VM's holders running | **Branch points:** an SSH outage, reconnect with no replayed input, an app restart that keeps live holders, and a host-key change. |
| 04 | A test puts pinned Copilot and Codex CLIs on the VM in their own homes, and takes them away after | **Branch points:** pinned CLIs fetched by sha, isolated homes, login copied or Needs the owner, cleanup, and a sha mismatch. |
| 05 | A Copilot session on Claude Haiku 5.5 does a tiny task, asks through the Inbox, and resumes after a stop | **Branch points:** folder trust, `/model` for this session only, an Inbox ask, turn end, diff, stop, resume and fork. |
| 06 | The same Copilot conversation moves to the VM, forks back to this Mac, and moves home | **Branch points:** move to the VM, continue, fork back to this Mac, move home, and an offline target. |
| 07 | A Codex session on GPT-6 Luna does a tiny task, asks through the Inbox, and resumes after a stop | **Branch points:** "Hooks need review", `/model`, an Inbox ask, turn end, diff, stop and resume (B9). |
| 08 | The same Codex conversation moves to the VM, forks back to this Mac, and moves home | **Branch points:** move to the VM, continue, fork back to this Mac, move home, and Codex resume failing (B9). |
| 09 | Claude, Codex and Copilot work in one channel across this Mac and the VM, and each answers, moves and continues correctly | **Branch points:** three kinds at once, asks in order, moves and forks across machines, and continue in another agent. |
| 10 | A script I wrote runs on a new machine as soon as Humata's tools are on it | **Branch points:** the added trigger, steps run, humata, start and send, the run log, a failure ask with Retry, and the capability off. |
| 11 | A script runs once per reconnect, and a session that moves gets its scripts without risk to the session | **Branch points:** once per reconnect, session-arrived and session-leaving, a timeout, dedup after restart, and a fork. |
| 12 | A dropped link leaves every agent running on the VM, and reconnect re-attaches each with no lost and no replayed input | **Branch points:** an agent mid-turn, an Ask pending during the cut, an answer refused while offline, reconnect, and no replay. |
| 13 | After the VM powers off and after this Mac reboots, each session resumes on its exact conversation, and a restart report names each outcome | **Branch points:** a throwaway VM powers off, this Mac reboots, each conversation resumes, the restart report, and compared with Stop for the night. |
| 14 | Every agent kind forks on this Mac and onto the VM, by code and context, code only, context only and from an earlier turn, and the fork resumes in the real CLI while the parent stays as it was | **Branch points:** Claude, Codex and Copilot; same machine and the VM; code and context, code only, context only, and before a turn. |

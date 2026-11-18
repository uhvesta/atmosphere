# VERIFY: Every user flow is reproduced, reviewed for usability, and its fixes are covered by tests

- **Status:** open (owner, 2026-11-17 14:05 PST): runs alongside the v1 close-out, before RELEASE. VERIFY checks that the user flows fit together, so it doesn't wait for v1's last UI nits. Find, don't fix, until main says "v1 closed".
- **Depends on:** RELEASE for the final pass on the released app; started early on the current tree (owner, 2026-11-17)
- **Unblocks:** RELEASE2
- **Layer:** L9 · **Lane:** cases, reproduction, usability review, TDD fixes

## Outcome
Every user flow is reproduced, reviewed for usability, and its fixes are
covered by tests. It covers the app's behaviour as completely as it can.

## Exit criterion
- Every case is pass, or blocked by the environment with a reason.
- Every real-agent flow (below) has passed with Copilot on its cheapest
  model. Codex (cheapest) and Claude Code (Haiku 5.5 low) are **deferred by
  the owner** (2026-11-17 14:10), not blocked: they run in a later pass.
- Every feedback item has a passing test.
- The gate is green.

## Pipeline
1. **Two kinds of Haiku 5.5 drafters run at once**, under an Opus 5.5 coach:
   - **Writers** draft plain-words cases into `flows/cases/` (brief
     `flows/cases/briefs/verify.md`). The coach merges, dedupes and numbers
     them into `flows/cases/verify/`.
   - **Explorers** step through every screen, sheet, menu, setting and CLI
     command. They snapshot headless: `native_test`, `live_flows`
     `.view`/`.png`, `design_test` `.new.png`, and the CLI against an
     isolated `humatad`. They record in markdown what happens at each step,
     in `flows/cases/explored/` (brief `flows/cases/briefs/explore.md`).
2. **One Haiku 5.5 reproducer** runs every case and recording. For each it
   records the steps, what happened, pass, fail or blocked, and the evidence,
   in `flows/cases/results/`.
3. **A Fable 5.2 reviewer** judges usability. For each case that doesn't
   make sense it drafts concise feedback, tagged UX or BUG.
4. **An Opus 5.5 low coach runs Haiku workers.** For each feedback item they
   write an e2e or snapshot test that fails on the current code, then fix it
   test-first until it passes.

## Real-agent flows (owner, 2026-11-16)

The cases must be real runs, not only fakes. Agents drive real agent CLIs on
their cheapest models, with isolated homes (HOME, CODEX_HOME, COPILOT_HOME,
COPILOT_CACHE_HOME), never the owner's own:

- **Copilot** on its Haiku model, and **Codex** on the cheapest GPT 6 Luna
  model, come first. Make every flow below work for these two at a high level.
- **Claude Code** on Haiku 5.5 at low effort comes after.

Flows to run, each on the host and where it applies on a remote machine (the
Lima VM `h2-terminal-c1`):

- **Where it runs:**
  - Run a session on the host.
  - Run a session on a remote machine.
- **Moves:**
  - Move a session from a remote machine to the host.
  - Move a session from one remote machine to another.
  - Move a session between channels and between sessions, as far as that is supported.
  - Move a session from the desk to a session.
- **Sessions and channels:**
  - Make a new session.
  - Grow a channel.
  - Fork a session: from the start, before a turn, and with a different agent.
- **Repos:**
  - Add a repo.
  - An agent asks to add a repo, or adds one.
- **Reviews and PRs:**
  - An agent makes a review.
  - An agent prepares a review for the owner: it reads the diff or PR, groups
    the changes, drafts notes and suggested comments, and hands it over as a
    review ready to read. The owner then opens it, asks it questions, edits or
    drops its drafts, and submits it.
  - Review a diff.
  - Review a PR.
  - Ask questions about a PR in the review chat.
  - Submit a PR review.
  - Leave inline feedback and send it to the agent's TUI.
- **Files:**
  - Transfer files from host to remote.
  - Transfer files from remote to host.
  - Transfer files from one remote to another.
- **Agent setup sync:**
  - Sync the host's Copilot, Codex and Claude Code setup (skills, MCP
    servers, other config) onto a remote machine with one deterministic,
    automatic action. If no such button exists, record that as a gap with a fix.
- **Bug reports (record and replay, then snapshot):** prove both work end
  to end, in the released app and with `humata report`:
  - Record and replay: start a recording from the menu, ⇧⌘R or
    `humata report record`, and check the title-bar dot. Do a real flow while
    it records (with a real agent), then stop it. Replay the recording and
    check it reproduces the same steps, views and client calls, with
    secrets scrubbed.
  - Snapshot bug report: take a snapshot (menu, ⇧⌘S or
    `humata report snapshot`), use draw mode to circle the problem (pen,
    undo, clear, done), fill the form with comments and marks, and copy
    each kind of report. Check the copied prompt names the right elements
    and the report folder holds what it should. Paste it into an agent and
    check the agent can act on it.
- **The Board (our mini Jira):** prove it works for a person and for
  agents. That covers items, statuses, depends-on edges, the kanban, table,
  dashboard and Graph views, the drawer, approvals, and ownership. Agents
  must know the Board exists and use it through the CLI and MCP: reading
  items, creating them, moving status, asking for approval. Every Board
  flow gets written up in `flows/` with plain-words cases, then actually run.
  The owner left the details open: a strong agent that knows the product
  surface (Fable 5.2) writes the Board's case list when
  VERIFY opens.
- **Everything else:** every flow in `flows/`, plus whatever the explorers find.
  The list above is a floor, not the whole set.

Each run records the agent, its model, the machine, the steps, what happened
and the evidence. A flow that can't run because of the environment (for
example the VM's disk) is recorded as blocked, with the reason.

### Test setup (owner, 2026-11-17)

- **Logins.** Copilot, Codex and Claude Code logins are copied from the owner's homes, which are read and never modified.
  - They go only into test VMs we spin up, and on the host into isolated homes.
  - Never onto other machines, and never into tracked files.
  - This is allowed until 2026-11-18 00:00 PST (owner extended it again at 14:10; earlier end 16:30); after that, copy no more logins, but logins already in place may keep being used.
- **Spend.** There is no cap on runs or spend for the real-agent flows.
- **PR flows.** PR flows use this repo, atmosphere. Make git worktrees with dummy changes used only for the test, push test branches, and open PRs from them. Review, ask about and submit reviews on those PRs, then close the PRs and delete the branches. The worktrees are for git only: builds and tests stay in the main worktree's single bazel.
- **Add-repo flows.** Add-repo flows use several of the owner's existing GitHub repos, read and clone only.
- **Remote→remote moves.** These use a second Lima VM alongside h2-terminal-c1. Disk for it comes from clearing caches.

### Carried-in checks (from the gate-30 fixes)

- **Add machine without a workspace.** If the workspace fails to load and Add machine can still be opened, the "Migrate This Mac" offer is quietly missing. Check whether the UI can reach that state.
- **Home and deliberate stops.** Home's needs-attention list shows every Exited session, so a session the person stopped on purpose shows a red ✕ (D124 amendment). If that is noisy, the daemon should say "stopped by you" and Home should skip it. Don't go back to IDLE.
- **Fake and daemon stop status.** Check that the fake daemon's StopSession gives Exited, as the real daemon does.

## Out of scope
- Performance: FAST.
- Linux and browser hosts: cut (D485).

## Lane (written by the coach)

Coach: VERIFY (Opus 5.5), opened 2026-11-16.

### Log
- 2026-11-16: read flows, cases, briefs, CONFIG, WORKLOG claims, AGENTKINDS, AGENTSET, MACHINESCRIPTS, D513, D689. Wrote `flows/cases/briefs/verify.md` (areas A–I). Step 1 starting: nine Haiku writers, one area each, drafts to scratchpad `verify/drafts/`.
- 2026-11-16: owner correction (via main): step 1 adds explorers, Haiku agents that record actual behaviour per step from the headless harnesses into `flows/cases/explored/` (brief `flows/cases/briefs/explore.md`). Phase 1 maps from code and snapshots without Bazel; phase 2 runs focused Bazel one explorer at a time after "VERIFY: tree green". Explorers X-A, B, C, E, F, G, H, I, J are running; X-K (Inbox, Board), X-L (Files, Find, marks), X-M (other Settings panes), X-N (whole CLI) start when slots free (20-agent cap).
- 2026-11-16: **Paused** (owner correction via main: VERIFY is the final jank review, run only once every v1 milestone is done; resume on main's "VERIFY: start (v1 closed)", which replaces "tree green"). Where it stopped:
  - Step 1 only; no Bazel run by any VERIFY agent, no repo code touched. Steps 2–4 not started.
  - All 14 explorers had already launched (none queued). Every running writer and explorer was told to finish its current file and hand back.
  - Writer drafts (scratchpad `verify/drafts/`, not yet merged or numbered): A-1, A-2, B-1, B-2, C-1, C-2, D-1, D-2, E-1, E-2, F-1, F-2, F-3, G-1, H-1, H-2, H-3, I-1, I-2, I-3; writer H2 (flow gaps 02–04) was still writing.
  - Explorer phase 1 records done in `flows/cases/explored/`: X-A, X-C, X-F, X-G, X-I (plus partial X-H). Pending hand-backs at pause: X-B, X-E, X-H, X-J, X-K, X-L, X-M, X-N. Each record lists its PENDING RUN commands for phase 2.
  - On resume: collect the remaining hand-backs, merge/dedupe/number drafts into `flows/cases/verify/`, re-check findings against the then-current code (v1 work will have moved it), then phase 2 one explorer at a time, then the reproducer.
  - X-L wrote no record before the pause; its notes are kept in scratchpad `verify/drafts/X-L-notes-paused.md`.
  - Hand-backs after the pause: writers A (A-2 is over 200 lines, trim on resume), F (F-1..3 done), H2 (nothing written; notes in `verify/drafts/H2-notes-paused.md`); explorers X-B (26 steps), X-H (4 records, 58 steps), X-I (46 steps), X-K (inbox + board, 31 steps; CLI record not written), X-N (4 of ~100 commands written). Side finding: `flows/cases/macro/15-upgrade-day.md` wrongly says upgrade_holders tests don't exist.
  - Writer C handed back: C-1, C-2 done. Note: C-1 says D689 text is stale (upgraded trigger is wired).
  - X-J handed back (keys-and-menus, 18 steps; sidebar, palette, overlays records not written). X-M handed back (settings-pages, 22 steps, complete for phase 1). Only X-E is still out.
  - X-E handed back (agent-switches, 16 steps). All VERIFY agents are now stopped; none running. Waiting for "VERIFY: start (v1 closed)".
- 2026-11-17 14:05: **Resumed early** (owner via main): VERIFY starts before v1 closes; v1's remaining work is UI nits in three other lanes. Rules until main says "v1 closed": find, don't fix (results and UX/BUG feedback only; pipeline step 4 held), except a BUG that blocks a real-agent flow, fixed outside the UI files being edited (ui/src/panels.rs, sidebar.rs, sidebar_desk.rs, panel_changes.rs, session-menu code) after telling main. One bazel: targets go through the batch queue; main builds run binaries.
- 2026-11-17 14:10: owner via main: login window runs to 2026-11-18 00:00 PST. **Copilot only** for now (cheapest model); Codex and Claude Code deferred by owner. Run the Copilot flows in parallel, one Haiku worker per flow area with its own isolated home and session, up to 10 lanes; draft merge and explorers run alongside.
- 2026-11-17 14:13: **Copilot login in place.** Host: isolated home template `<scratchpad>/verify/homes/t0` (own COPILOT_HOME, COPILOT_CACHE_HOME), token from `gh auth token --user uhvesta` in `<scratchpad>/verify/homes/copilot.env` (0600, untracked); `copilot -p` on `claude-haiku-5.5` answered. VM h2-terminal-c1: Copilot CLI 1.0.113 linux-arm64 in `~/.local/bin`, token in `~/verify/copilot.env` (0600), isolated home `~/verify/homes/t0`; smoke test answered. Owner's ~/.copilot only read (config.json copied). Note: reading the Copilot keychain item would pop a keychain dialog, so it is not used. Second VM: `humata-h1` resized to 1.5 GiB / 5 GiB and started (h2-terminal-c1 untouched; its holders kept).
- 2026-11-17 14:20: Second VM is **humata2-linux** (Ubuntu 26.04 arm64, 2 CPU / 4 GiB / 20 GiB), not humata-h1: Copilot's linuxmusl build fails on Alpine (Node-API symbols not loaded, both the release tarball and npm), so humata-h1 was stopped again (its resize to 1.5 GiB / 5 GiB stays). Copilot 1.0.113 + token (`~/verify/copilot.env`, 0600) + isolated home `~/verify/homes/t0` on humata2-linux; smoke test answered. Logins in place on host, h2-terminal-c1 and humata2-linux.
- 2026-11-17 14:30: Writer drafts merged into `flows/cases/verify/` 01–19 + README (Haiku merger; F-1+F-2 → 11; A-2 trimmed). H2 (flows 02–04) and X-L (files, find, marks) cases still unwritten. Statuses kept as drafted, not re-checked.
- 2026-11-17 14:45: **BUG-T1 found (blocks real-agent flows):** `humata session type` and `session start --message` put the text into Copilot 1.0.113's prompt but never submit it (CLI says typed). Enter through `humata attach`, or tmux paste + Enter, submits. Opus debugger root-causing (no edits); flows meanwhile use the harness `say.sh` workaround and record whether `session type` submits. **Eight parallel Copilot lanes started** (Haiku 5.5 high, each its own humatad2_admin, isolated home, socket and VM user; harness and brief in `<scratchpad>/verify/harness/`): host sessions and moves, forks, remote sessions and moves (humata2-linux + c1 sparse user), file transfers (two users on humata2-linux), repos (owner's GitHub, read/clone only), reviews and PRs (own worktree, `verify/pr-flows`, closed and deleted after), agent setup sync, bug reports. Results go to `flows/cases/results/copilot-<lane>.md`. Fable Board case writer also running.
- 2026-11-17 15:00: Fork lane: 4 pass / 2 fail / 2 deferred (owner). Context forks of Copilot 1.0.113 sessions are refused because the strict parse doesn't know its `model.*` and `instruction.update_delivery` records; an Opus worker is fixing it test-first (daemon/conversation). Sync lane: FAIL. **Add machine killed its own setup script on systemd Linux** (`pkill -f '[h]umatad2 serve --role machine'` matched the `sh -c` script holding the unit's ExecStart). Fixed in daemon/machines/src/service.rs with whole-command-line matching (`-x -f '[^ ]*humatad2 …'`); test `no_script_takes_itself_for_the_daemon`, in batch run 57. Remote and files lanes wait for rebuilt binaries for their machine flows. Board flow 10 and 14 cases written (Fable); index rows added to flows/README.md and flows/cases/README.md (with the verify/ layer). Product gaps (Board/MCP not given to agents, skills-only sync, no links, no watcher) go to `flows/cases/results/FEEDBACK.md` with SPEC IDs and Fable rulings against the agent-access decisions; fixes held for step 4 unless one blocks a Copilot flow tonight.
- 2026-11-17 15:30: First pass of lanes in (`flows/cases/results/copilot-*.md`):
  - host: 2 pass, 2 partial, 1 fail
  - forks: 4 pass, 2 fail, 2 deferred
  - remote: 2 pass, 3 fail
  - repos: 2 pass, 3 pass with notes, 2 partial
  - PR: 6 of 7 pass, 1 blocked (no MCP for Copilot)
  - bug reports: 9/9 headless targets pass; real pipeline needs the app open, kept for one end run on the owner's screen
  - sync: 0/1, blocked by the Add-machine bug
  - files: still running
  Flow-blocking fixes under way, test-first, outside the held UI files:
  - BUG-T1: type doesn't submit (holder KeysRead)
  - the Copilot 1.0.113 strict parse, which blocks continue, context forks, Desk-to-channel and machine moves
  - remote permission hook never reaches the Mac Inbox
  - a no-turn Copilot session can't move to the host
  - `--dry-run` on access set, review start and session move applies the change (safety)
  Add-machine pkill fix passes machines_unit_test (runs 59/60). Fable keeps `flows/cases/results/FEEDBACK.md` and rules on the MCP/Board/agent-folder items. verify/bin briefly held Linux binaries (14:36–14:40); restored from bazel-bin.

- 14:45–14:50 VERIFY coach: Fable's FEEDBACK.md has 56 items (23 BUG, 18 UX, 15 GAP), 13 of them marked Now. New fixes started:
  - an Opus worker connects Humata's own MCP server, its tools on PATH, and instructions naming each clone to every agent start when the agent's switch is on (VF-1, VF-2, VF-42; Copilot gets the live check tonight);
  - a Haiku worker makes the Board send revisions from the CLI, gives every door the same default board and column, and makes column + a single call (VF-20, VF-21, VF-25).
  - `session fork --channel` will be refused with a pointer to `session continue --channel` (VF-46, Fable's ruling).
  - Still waiting on runs: the Copilot submit fix, the parse fix, the remote permission hook, the dry-run fix, the no-turn move, and --model.
- 14:52 VERIFY coach: Owner rule D743: Humata never tells an agent where its repos are. FEEDBACK VF-1 and VF-42 are reworded to match, and the agent tooling worker is corrected: no paths or branches in MCP instructions; agents read repos only when they ask. A worker is searching the code for any repo add or drop message sent to the agent.
  Files lane: 5 of 6 flows pass. These lanes ran on binaries from before the pkill fix, so their VMs were pre-staged.
  New fix: an agent's `humata cp` fails with "no destination.session_clone.session". `humata cp --dry-run` starts a real transfer, and that goes to the dry-run fix.
- 14:56 VERIFY coach: Sync rerun on the fixed build is PARTIAL. Add machine now works with no workaround, which proves the pkill fix live. Synced skills reach the VM, but the remote Copilot doesn't see them, so VF-5 is now a Now fix: link the synced skills into each agent's own folder. MCP and plugin sync (VF-3) and the host-change watcher (VF-6) are held for step 4. Restore changing the file mode (VF-58) is being fixed now; the 13 per-call approvals (VF-57) go to Fable for a ruling. Lanes running: none. Fixes in flight: 12.

- 15:16 Run 66 passed two fixes. VF-59: copied, trashed and moved folders keep their exact mode, and a read-only folder can now be trashed (daemon/transfers disk.rs, engine.rs, fsutil.rs, trash.rs). A session that hasn't had a turn yet now moves and starts fresh in its new folder (daemon/revive/src/agent.rs, start_moved). The other fixes wait on run 67.

- 16:10 VERIFY coach: four fixes pass.
  - An agent's `humata cp` now reaches its own and its channel's machines (agent_send_machine_test, run 67).
  - `--dry-run` refuses anything that isn't read-only, and attach under dry-run is a usage refusal (all cli targets pass, run 72).
  - Typed text is refused, never typed blind, into a holder from before the keys wait (D753).
  - `session type` presses Enter by default, and `--no-submit` leaves the text in the input (D754). Both pass in run 73.
  - Add machine waits behind This Mac's daemon Migrate item (D752, run 73).
  Fable ruled:
  - An agent never writes shared/ on another machine; its own refusal gives a pull hint (D755, worker on it). Main updated SPEC FC-8 and TP-4.
  - An agent's bare destination word that names no visible machine is a usage refusal unless it has a dot (D756, worker on it).
  Queued:
  - hook --dry-run is a usage refusal (main's ruling).
  - The keyed-input fallbacks for older holders are being removed (force-upgrade rule; no live holder is killed).
  Reruns start when these land on one green build.

- 2026-11-17 18:10: Rebuilt the VERIFY bins (humata, humatad2, humatad2_admin, humata2_holder, tools_linux). The old bins are in bin.old-1817 and bin-linux.old-1817. Five Copilot rerun lanes run in parallel: remote 1–2, fork, sync VF-5, files 4 and 5 (pull), and t1 without say.sh. channel_attempt's lost-link retry (D758) is in this build: every changed source file was saved by 17:58:48, before the build started at about 17:59. The lanes test it as a finding. The dev build sits at release "", so it posts no Migrate items. After 0.2 is installed, a Migrate lane will run the stamped 0.2 bundle against an older daemon and holder on a throwaway VM user.

### 2026-11-17 19:10 — rerun wave 2
- Tree compiled again after runs 95–99 broke on partial edits (rule now: a shared-type edit updates all uses in one pass). Full `bazel build //apps/humata2/...` green 18:59; run 100 green.
- Bins rebuilt 19:05 (old ones in verify/bin.old-1905, bin-linux.old-1905). In: mirror double-move fix, N1 conversation-file watcher, D760 re-add/remove (first version), D761 ask-user Inbox, D762 input gate, D763 held start message, D764 GetGrid, D767 trust dialog Inbox question, D768/D769 remote cp, agent_busy.
- Not in (still red or in flight at build time): D760 channels-after-commit change, N1 remote watcher push, revive note gate, topology_get, SendNote/AskSession/AnswerQuestion, provider import, settings sections, remote GiveOwnCopy.
- Lanes: remote (T1b, D762 refusal, type after move, read after two moves, D760 live, GetGrid) and files+t1 (flow 5 pull, dotted cp, ask-user Inbox, agent_busy, N1 resumed fork). Deadline 23:00. 0.2 Migrate lane waits on 0.2.

### 2026-11-17 19:40 — rerun wave 2 results
- remote: 4 of 6 pass. files+t1: 7 of 10 pass. Wave total: 11 of 16 (69%).
- 3 failures are artifacts: red-proof cuts from the D760 and N1 lanes were in the tree when the 19:05 bins were built. New rule: a cut is restored as soon as its run starts building, and is never in the tree at a checkpoint.
- Real open bugs, each with a lane, test first:
  - the trust card parses only "Yes" and its reply doesn't press the row; the held message sometimes needs a second Enter
  - a remote agent's pull from This Mac compares a machine name to an id
  - agent_busy is true on idle
- In: D774 (notes/questions), D763 (revive gate + the remote ask journaled to the host).

### 2026-11-17 19:51 — freeze list (all 11 build lanes are in, none parked)
- D760 remove/re-add: green in runs 99, 100 and 110. The machine leaves its channels only after the removal commits.
- N1 history watcher, local and remote push: remote red in 109, green in 110.
- D768/D769 agent cp on a machine, and the pull fix: green in 102, 104 and 110.
- D763 revive gate, and the remote ask going to the host Inbox: green in 107 and 108.
- D764/D772 GetGrid and topology_get: green in 100, 108 and 110.
- D770/D771 settings sections and provider import: green in 109.
- D773 remote GiveOwnCopy: red in 108, green in 111.
- D774 SendNote, AskSession and AnswerQuestion: green in 106 and 108.
- D775 agent_busy on idle: red in 110, green in 111.
- D767/D763 trust card: all rows are offered, the reply presses the row, and Enter is pressed again at most once. Red in 110, green in 111 and 112.
- Every lane confirmed in writing that its files hold no temporary edits. A tree grep finds no REDPROOF and no dbg!.
- Owed live in wave 3: D760, N1, the trust reply, the remote pull, agent_busy, the remote unsent-message Inbox, and the 0.2 Migrate.

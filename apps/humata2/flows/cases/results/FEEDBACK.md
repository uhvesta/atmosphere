# VERIFY feedback (pipeline step 3)

One list of feedback items from the usability review. IDs are stable: new items are appended, and a closed item keeps its number.

- **Tags:** UX (it works but misleads), BUG (it does the wrong thing), GAP (the spec or VERIFY promises it and it isn't there).
- **Now** means it blocks a Copilot real-agent flow tonight. It is fixed now, outside `ui/src/panels.rs`, `sidebar*.rs`, `panel_changes.rs`, `pages.rs` and `machine_page.rs`.
- **Held** means step 4, after v1 closes.
- Paths are relative to `apps/humata2/`. "Ruling" is the reviewer's call where DECISIONS.md doesn't settle the fix.

## Sources read

- `flows/cases/results/copilot-sync.md`
- `flows/cases/results/copilot-fork.md`
- `flows/cases/results/copilot-repos.md`
- `flows/cases/results/copilot-report.md`
- `flows/cases/results/copilot-pr.md` (F1 to F9)
- `flows/cases/results/copilot-host.md` (F1 to F14)
- `flows/cases/results/copilot-remote.md` (findings 1 to 10)
- `flows/cases/results/copilot-files.md` (flows 4 and 5: VF-57, VF-58)
- `flows/cases/board/README.md` (What the cases found) and `flows/10-the-board.md`
- Main's rulings by message, 2026-11-17 (socket path, untested report parts, `session queue`, remote permission hook)
- Not yet read: any lane not listed above

The Board README lists seven "Not built" and six "Likely bugs" today, not ten. VF-1 and VF-15 to VF-28 cover all of them; add items if the list grows.

## Now, at a glance

VF-1, VF-2, VF-5 (confirmed live 2026-11-17), VF-7, VF-11 (fixed, pending batch run 57), VF-14, VF-20, VF-21, VF-23, VF-25, VF-29, VF-41, VF-42, VF-54, VF-58, VF-59.

## Agents and Humata's tools

### VF-1 (GAP): Every agent Humata starts has Humata's MCP server, and learns from it that `humata` and the Board exist
- **Evidence:** board README "Agents are never told the Board exists" (case 9); `flows/10-the-board.md:54,100`; `copilot-pr.md` F3 (a review agent has no `review_draft` or `view_prepare`). `daemon/sessions/src/agents.rs:552` (Codex), `:572` (Copilot), `:615` (Claude) pass no MCP config; `daemon/agent_hooks/src/lib.rs:373` (the Copilot plugin holds hooks only); `daemon/mcp/src/server.rs:552` (the greeting never names the Board); `api/MCP.md:13-17`.
- **SPEC:** BD-9 "An agent creating, editing, moving or deleting items follows the channel's agent access rules"; IN-16, IN-17; SEC-3 "Agents get only what the rules allow"; RV-32 and PR-10 (the review tools). VERIFY: "Agents must know the Board exists and use it through the CLI and MCP".
- **Decisions:** D32 (one stdio `humatad mcp` per agent, bound by `HUMATA_SESSION_TOKEN`); D38; D193 (Copilot's hooks go in by `--plugin-dir`, Codex's by `-c`; no `COPILOT_HOME` or `CODEX_HOME`); D191 and D199 (permission asks go to the Inbox); D720 and D721 (the per-agent switch); D180 (board rows and defaults); D619 (Copilot conversations holding MCP records "stay refused until proven").
- **Ruling: fix it now, as a flow-blocking exception.** It blocks the agent planning on the Board (case 14) and the agent preparing a review (PR flow 2). Humata 2 has no switch that gates the CLI or MCP, and it needs none tonight: what an agent may do is the channel's Agent access rules, checked in the daemon on every call, with destroying and reaching past its own work already Ask me or Never. The switch that exists is the per-agent one in Settings › Agents (D720).
  - **Agent switch on:** every start, resume, fork and rebuild of that kind gets three things, all scoped to the session and none written into the agent's user config or the repo: (1) the `humata` MCP server; (2) Humata's tools folder on PATH (VF-2); (3) the token and socket it already gets.
  - **Agent switch off:** the kind isn't started, so nothing is given. A session that already exists still gets all three when it is resumed (D721: the switch gates new starts, not access).
  - **"Install when missing" and "Ask for permission in the Inbox":** no effect on this.
  - **Plain shell and custom kinds:** PATH, token and socket only. A custom kind gets the server when its launch command uses a `{humata_mcp}` slot.
  - **How, per agent** (the worker confirms each flag against the CLI's own `--help` before using it): Copilot, tonight: `--additional-mcp-config @<session state>/humata-mcp.json`, or a `.mcp.json` in the session plugin D193 already passes, whichever 1.0.113 loads without a prompt. Claude: `--mcp-config <file>`, without the strict flag, so the person's own servers stay. Codex: `-c mcp_servers.humata.command=…` and `.args=…`, beside the hooks `-c`. The file holds the command and socket only; the token stays in the environment.
  - **No double asking:** the agent's own permission prompt is pre-allowed for the `humata` server's tools and the `humata` command (Copilot `--allow-tool`, Claude `--allowedTools`), because Humata's rules already decide each call. Otherwise every board read waits in the Inbox (VF-35).
  - **What the agent is told:** only the server's `initialize` instructions and its tool descriptions. They say what the tools do and nothing about where anything is: "You are running inside Humata as the session “<title>”. Humata's tools are on this server and as the `humata` command (`humata --help`): this session's and its channel's boards (`board_*`), review drafts and views, files, and asking the user (`inbox_request`)." Then today's three sentences. No repo path, clone path, branch or folder appears in the instructions or in any tool description. Nothing is typed into the TUI, nothing is written into the repo, AGENTS.md or CLAUDE.md, and nothing says how to work in the repo (no repo hints). A tool or `humata whoami` may return the session's repos when the agent asks (IN-16): that is a read the agent chooses to make, not a hint Humata pushes.
  - **Corrected 2026-11-17:** this ruling first had the instructions name the session folder and each clone's path and branch. The owner decided on 2026-11-16 that Humata does not tell agents where repos are, and a person prompts the TUIs (AGENTSET, D720 to D729). A ruling can't override an owner direction, so those lines are gone. The server, the tools on PATH, the token and socket, and the pre-allow all stay, behind the per-agent switch.
  - **Reason:** the access rules are the gate the owner signed off; an agent that can't see the tools can't follow them, and a second gate would only be a way to forget to turn it on.
  - **Reverse:** one row under each agent in Settings › Agents, "Give it Humata's tools", on by default.
- **Depends on:** VF-7. D619 refuses a Copilot conversation that holds MCP records, so once this lands every context fork of a Copilot session that called a Humata tool is refused until the adapter knows those records. Re-run the fork lane after wiring.
- **Test:** start a session of each built-in kind against a stub CLI that records its argv and reads the config it is given: the stub reaches a server named `humata`, its `initialize` instructions and tool descriptions name the board, review and inbox tools and hold none of the session's folder, clone paths or branches (corrected 2026-11-17, as above), a `board_item_list` call answers as that session, and the repo's working tree and the agent's user config are unchanged. A disabled kind is refused at start as before.
- **Now.** Copilot tonight; Codex and Claude by the same design when their lanes open. Files: `daemon/sessions`, `daemon/agent_hooks`, `daemon/mcp`.

### VF-2 (BUG): `humata` runs by name in every session tab
- **Evidence:** board README rig note "Call `humata` by its full path: it is not on the tab's `PATH`"; `flows/10-the-board.md:54`. `daemon/humatad/src/identity.rs:502` (`tab_env` sets the token and socket only); no code under `daemon/holder`, `daemon/sessions` or `daemon/humatad` adds `~/.humata2/bin` to a tab's PATH.
- **SPEC:** TS-1 "Sessions Humata starts there have it on their PATH"; TS-0 "puts `humata` on the user's PATH".
- **Decisions:** D137 "Sessions already get `~/.humata2/bin` on PATH (TS-1)", which is not true today; D51; D738 (Humata sets no prompt, so only PATH is touched).
- **Ruling:** settled by TS-1 and D137. Prepend the daemon's own tools folder (the `--humata` binary's folder, `~/.humata2/bin` in a release) to each tab's PATH, on the host and on machines, for agent tabs and shell tabs alike. Not behind a switch: it is Humata's own tool, and it acts as the session's agent under the same rules.
- **Test:** start a zsh session, type `command -v humata` and `humata whoami`; the first prints the daemon's own binary and the second names that session.
- **Now.** Cases 12 to 14 need it. File: `daemon/humatad/src/identity.rs`.

## Agent setup sync

### VF-3 (GAP): One sync carries each agent's skills, plugins and MCP servers to a machine, and says plainly what it leaves behind
- **Evidence:** `copilot-sync.md` G1. `daemon/humatad/src/peer_skills.rs:48-54` (`AGENT_FOLDERS` is the three `skills` folders only).
- **SPEC:** SK-1 "each agent's skill and plugin folders"; SK-6 (secrets never sync); VERIFY "skills, MCP servers, other config ... one deterministic, automatic action".
- **Decisions:** D140 (engine, secret names); D49 (one way only); D174; "Portable config" (secrets never in files).
- **Ruling:** widen the built-in rule, in this order: (1) plugin folders, as SK-1 already says; (2) MCP servers as a per-agent managed entry (Copilot `mcp-config.json`, Codex's `mcp_servers` table, Claude's `mcpServers` key only, never the whole `~/.claude.json`), with any server whose entry holds an inline secret (`env`, headers, tokens) skipped and named in the preview; (3) nothing else. Settings, models, trust and logins stay per machine, and the preview says so. Reason: the owner asked for one action, but a whole-config copy would carry auth and per-machine state.
- **Test:** a host home with a skill, a plugin and two MCP servers (one with a secret in `env`) syncs to a fake machine: the skill, plugin and clean server arrive, the secret one doesn't, and the plan names it as skipped. A second sync plans no change.
- **Held.** VERIFY asks for the gap to be recorded with a fix, which this is.

### VF-4 (GAP): Scripted sync rules run on connect and on demand, or the spec stops promising them
- **Evidence:** `copilot-sync.md` G1, second bullet. `daemon/machines/src/sync_rules.rs` parses rules; there is no `humata sync run` in `cli/src/args.rs` and no caller in `daemon/humatad`.
- **SPEC:** TS-3 "Rules run on connect and on demand (`humata sync run [rule] [machine…]`)"; TS-4.
- **Decisions:** D49; D140 (rule config); D680 to D688 (machine scripts, which do run); D689 (starting agents needs nothing new beyond D682's `start`).
- **Ruling:** don't build a second engine. Machine scripts already run host-authored steps on `connected` and on demand. Fold TS-3 into them: a rule with a `source` becomes a machine-script step that sends a folder with the skills machinery, and `humata sync run` goes from the spec. Reason: two rule files that both mean "run this on connect" is one too many.
- **Test:** a rule naming a host folder and a destination, with the trigger `connected`, lands that folder on a fake machine at link-up with secrets filtered, and a rule aimed at `~/Humata/.skills` or `~/.humata2` is refused at check.
- **Held.**

### VF-5 (GAP, confirmed live 2026-11-17, Now): An agent Humata starts on a machine sees the synced skills beside the machine's own
- **Evidence:** `copilot-sync.md` G2 (read from code; the machine never reached Ready). The set lands in `~/Humata/.skills/<agent>/` (`daemon/humatad/src/peer_skills.rs:81`, `daemon/machines/src/skills.rs:517`); nothing reads it at launch.
- **SPEC:** SK-4 "agents started by Humata there are pointed at it. Skills made on the machine itself are never overwritten or deleted"; SK-5 (clash picks); SK-9 (old set or new, never a mix).
- **Decisions:** D193 rules out `COPILOT_HOME` and `CODEX_HOME` ("it moves auth and state"); D140 (the `.skills` link flips atomically; clash picks in `.skills-choices.json`).
- **Ruling:** no home variables, for D193's reason, and that goes for `CLAUDE_CONFIG_DIR` too. After each sync the machine links every synced skill into the agent's own folder as `~/.<agent>/skills/<name>` → `~/Humata/.skills/<agent>/<name>` (through the stable `.skills` link, so SK-9 holds). It never replaces a real folder or a link that isn't its own: a clash follows the SK-5 pick. Links whose skill left the set are removed. Reason: every agent already reads its own folder, so this needs no per-agent flag and hides nothing local.
- **Test:** on a fake machine with a local skill `mine` and a synced set `{a, mine}`: after the sync the agent's folder lists `a` (resolving into the set) and the local `mine` untouched; a later sync without `a` removes only that link. Then one live Copilot session on a VM lists the synced skill.
- **Fixed 2026-11-17 (D745), live rerun pending:** after each sync the machine links every synced skill into `~/.<agent>/skills/<name>`; tests in `daemon/humatad/src/peer_skills.rs` and `daemon/machines/tests/skills.rs`.
- **Confirmed live 2026-11-17 (sync rerun): a remote Copilot session listed none of the synced skills, so this is Now, being fixed.** Original note: held until run live; Now if the run confirms it, since the sync flow's outcome ("a Copilot session there sees them") can't pass without it. The run is unblocked by VF-11.

### VF-6 (GAP): A skill edited on the host reaches connected machines without a reconnect or a command
- **Evidence:** `copilot-sync.md` G3. `daemon/humatad/src/skills_service.rs:17-18` "Nothing polls (TR-7): a sync runs when the user asks, and once each time a machine's link comes up".
- **SPEC:** SK-3 "on connect and whenever the host's skills change"; SK-7; SK-11 "Automatic syncs apply without asking except when there is a new clash"; TR-7.
- **Decisions:** D52 (links push, never poll); D140; D174.
- **Ruling:** watch with OS file events (FSEvents, inotify), which TR-7 allows inside a machine; no timer. Watch the sources VF-3 defines, settle for 2 s, then run the `on_link_up` path for each linked machine with auto-sync on. A missing agent folder is watched through its parent so a first skill is seen. A first sync or a new clash still asks (SK-5).
- **Test:** with auto-sync on and a fake machine linked, writing a file into a host skill folder changes the machine's set with no call made; with auto-sync off, or the machine opted out, it doesn't.
- **Held.**

## Forks

### VF-7 (BUG): A context fork of a real Copilot 1.0.113 conversation is made, from now and before a turn
- **Evidence:** `copilot-fork.md` flows 3 and 4, F1; `copilot-host.md` F1 (continue in another channel and the Desk-to-channel move are refused too; `instruction.update_delivery` is in every session; the `model.*` records come from Copilot's own side calls, F12); `copilot-remote.md` finding 5 (no session with a turn can move machines). `daemon/conversation/src/copilot.rs` (`DATA_FIELDS`, about 228–500; `is_known`, about 887) lists none of `model.turn_started`, `model.model_call_started`, `model.captured_assignment_context`, `model.model_call_success`, `model.message`, `model.response`, `model.turn_ended`, `model.messages_snapshot`, `instruction.update_delivery`.
- **SPEC:** FK-5 "Same agent and the agent supports context forks: the conversation" is copied; FK-3; CV-1; CV-3.
- **Decisions:** D556 (unsupported formats are refused before anything changes); D619 (proven set; the rest refused until proven); D673 (records known from the CLI's shipped event schema); D614 (no raw-copy fallback).
- **Ruling:** settled by D673's method: add the record types from the CLI's own schema and this run's capture, keep refusing a record with an unknown field that holds a path. Add a capture of this conversation to `testdata/copilot/1.0.113-1`.
- **Test:** the captured two-turn conversation forks from now (the fork's copy lists both turns) and before turn 2 (it lists one); a conversation with a made-up record type is still refused and its file is unchanged.
- **Now.** Fix in progress. File: `daemon/conversation/src/copilot.rs`.

### VF-8 (UX): A refused context fork names only the ways forward that work
- **Evidence:** `copilot-fork.md` F2. `daemon/conversation/src/error.rs:15` says "Until then, fork it from now or start fresh with a summary"; fork from now is the refused operation.
- **SPEC:** CV-3 "the fork dialog says in one line that this <agent> ..." can't be read; FK-6; AGENTS.md Errors ("say what failed and how to resolve it").
- **Decisions:** D556; D212 (handover file plus summary); D331 ("refusals say what to do"); D674.
- **Ruling:** the advice names a code-only fork (fresh agent, same code) and a start fresh with a summary, and nothing else. It stays one message for the CLI, MCP and the dialog.
- **Test:** for an unsupported conversation, every fork kind the refusal's fix names succeeds as a dry run on that same session.
- **Held.** VF-7 removes the trigger for tonight's flows.

### VF-9 (UX): A fork's plan says which agent will run it, and never promises a conversation to a shell
- **Evidence:** `copilot-fork.md` F3; evidence `10-zsh-dryrun.txt`. `daemon/humatad/src/session_service/fork.rs:601-608` and `fork_remote.rs:89` print "starts a fresh conversation" for any agent, a plain shell included.
- **SPEC:** FK-4 "a preview of the result as text ... the fork's clones, branch and conversation"; FK-6 "The dialog says the new agent knows what was said and done but not the exact internal state".
- **Decisions:** D212 ("The preview ... says what the new agent will and won't know"; "the access check names the new agent"); D730.
- **Ruling:** the plan line names the agent by its display name ("starts Codex on a fresh conversation"); for a kind with no conversation it says "opens a shell; no agent conversation".
- **Test:** dry-run forks to another agent and to zsh return plans whose structured fields carry the target agent, and the zsh plan carries no conversation.
- **Held.**

### VF-10 (UX): The refusal says what is really unavailable, and agrees with `session read`
- **Evidence:** `copilot-fork.md` F4. The refusal says "its turns can't be listed" (`daemon/conversation/src/error.rs:15`) while `humata session read <parent>` listed all three turns (`14-parent-read-after-forks.json`).
- **SPEC:** CV-3 "A format it can't read (CV-7: unknown) lists no turns"; FK-3 (fork before an earlier turn).
- **Decisions:** D556 (turns go through the adapter); D675 (the history mirror).
- **Ruling:** `session read` is right to show what the mirror holds. The refusal changes: "can't be forked at a turn or carried", without claiming the turns can't be listed. The turn list used for fork points stays empty for an unread format.
- **Test:** for an unsupported conversation, `session read` returns its turns and the fork-point list is empty, and the refusal's structured detail names the fork, not listing.
- **Held.**

## Machines

### VF-11 (BUG): Add machine on a systemd Linux machine finishes and the machine is Ready
- **Evidence:** `copilot-remote.md` finding 1; `copilot-sync.md` F1 (3 of 3 adds failed at `start`, exit 255; `pkill-self-match-repro.txt`). `daemon/machines/src/service.rs` (`stray` helper; the unit's `ExecStart` at `:68`, heredoc at `:152-153`); `daemon/machines/src/quote.rs:43`.
- **SPEC:** HO-1 (the add steps); HO-2; TS-1; PER-18.
- **Decisions:** D49; D407; D39.
- **Ruling:** none needed. Fixed: the pattern matches only a real daemon.
- **Test:** `no_script_takes_itself_for_the_daemon` (`daemon/machines/src/service.rs:460`). Ask of step 4: also run the generated systemd script through a real `sh -c` beside a stub `humatad2`, since the unit tests stub `systemctl`.
- **Now.** Fixed, pending batch run 57; then re-run the sync lane.

### VF-12 (UX): A host key Humata has never seen is reported as new, not as changed
- **Evidence:** `copilot-sync.md` F2. `daemon/humatad/src/machine_service.rs:1125-1131` maps `HostKeyUnconfirmed` to `Kind::HostKeyChanged` with an empty `expected_fingerprint`; `daemon/mcp/src/daemon.rs:207`.
- **SPEC:** HO-1; flow 26 "host key, which Humata pins in its own known_hosts" (`SPEC.md:3801`); HO-2.
- **Decisions:** D333 (a changed key is cleared by remove and re-add); D25 (error kinds follow the proto).
- **Ruling:** add a `host_key_unconfirmed` kind carrying the presented fingerprint; keep `host_key_changed` for a key that differs from the pinned one. Reason: "changed" tells a script, and a person, that something may be intercepting, which is false on first contact.
- **Test:** a first add to an unknown machine fails with the new kind and the presented fingerprint; an add where the pin differs still fails with `host_key_changed` and both fingerprints.
- **Held.**

### VF-13 (UX): A failed setup step with no output says there was none
- **Evidence:** `copilot-sync.md` "What happened" and F1 "Also". `daemon/machines/src/error.rs:349` "the output is below. Fix what it reports and try again" (same wording at `:250`, `:264`, `:291`, `:337`).
- **SPEC:** HO-2 "says what failed and how to fix it"; AGENTS.md Errors.
- **Decisions:** D674 (an error's chain names each cause once).
- **Ruling:** when the tail is empty the message says the step ended with no output and the exit code, and points at the machine's `~/.humata2/logs`. One helper for all five messages.
- **Test:** a setup step that exits non-zero silently gives an error whose output field is empty and whose fix doesn't refer to output; one that prints keeps today's.
- **Held.**

## Typing

### VF-14 (BUG): `humata session type` and a start's first message are submitted to Copilot 1.0.113
- **Evidence:** BUG-T1 in `workflow/milestones/VERIFY.md` (log, 14:45); `copilot-fork.md` flow 1 and F5; `copilot-repos.md` finding 8; `copilot-report.md` finding 1; `copilot-pr.md` F1 and F9 (the review chat's first prompt sat unsent). `daemon/humatad/src/terminal.rs:957,1908` (the `<key>/submit` step).
- **SPEC:** SE-2 "an optional first message that is typed into the agent once it starts"; SE-23.
- **Decisions:** D671 (text ending in a newline is typed without it, then return is its own write 400 ms later; the pause is provisional); D324 (`Type` types exactly what it is given); D325; D337; D426; D200.
- **Ruling:** D324 stays for the RPC. The CLI's `session type` and every first message submit by default (the CLI adds the newline; `--no-submit` leaves it in the prompt), and the submit lands for Copilot whatever its paste handling needs. Reason: a command that answers `typed: true` and leaves the agent idle reads as done when nothing ran.
- **Test:** against the recorded Copilot TUI input behaviour, a typed line starts a turn (a turn-start is heard) without any other key; with `--no-submit` no turn starts.
- **Now.** Fix in progress.

## The Board

### VF-15 (GAP): The Graph view draws the depends-on edges
- **Evidence:** board README (case 4); `flows/10-the-board.md:37,101`. `ui/src/board.rs:256` has three modes.
- **SPEC:** BD-11 names "Board / Table / Dashboard" only; BD-14. VERIFY lists "Graph views".
- **Decisions:** D106 (boards as frames 21–24 draw them); D76 (the UI matches the design goldens).
- **Ruling:** build it, read-only first: items as nodes, edges as drawn lines, a click opens the drawer. It needs a frame, so Fable picks the golden; BD-11 gains the fourth mode. Reason: VERIFY names it and the edges otherwise show only in the drawer.
- **Test:** a board with A depends on B shows both and one edge in Graph mode, and a click on B opens B's drawer.
- **Held.**

### VF-16 (GAP): An item can be renamed, described and deleted in the app
- **Evidence:** board README (case 5); `flows/10-the-board.md:102`. `features/board/src/lib.rs:439`.
- **SPEC:** BD-3 (the drawer's fields); BD-2; SL-7.
- **Decisions:** D177 ("No title or description editor in the drawer this milestone ... deferred to the owner's next wave"); D383 (a title save stays Not built).
- **Ruling:** build it in the drawer: click the title or description to edit, Delete item in the drawer's menu with a confirm. Title and description saves send the revision (D383, SL-7).
- **Test:** editing a title in the drawer changes it on every view; a save from a stale revision is refused and shows the current value; delete removes the item and its inbound edges.
- **Held.**

### VF-17 (GAP): The drawer says who made the item
- **Evidence:** board README (case 7); `flows/10-the-board.md:103`. `clients/src/model.rs:1467` (`created_by`) is never drawn.
- **SPEC:** BD-3; BD-9; IN-17 ("items it created" against "anyone else's items").
- **Decisions:** D155; D180.
- **Ruling:** one quiet line in the drawer, "Made by <session or You>". Reason: ownership decides what an agent may change, so the person needs to see it where they decide.
- **Test:** the drawer of an agent-made item names that session, and of a person-made item says You.
- **Held.**

### VF-18 (BUG): A Table group folds when its chevron is clicked
- **Evidence:** board README (case 3). `ui/src/board.rs:537` draws the chevron with no action.
- **SPEC:** BD-12 "grouped by status with each group's count, each group foldable".
- **Decisions:** D106; "actionable UIs only" (owner).
- **Ruling:** wire it; the fold is kept per board with the view and filter (BD-11).
- **Test:** clicking a group's chevron hides its rows and keeps its count, and the fold survives leaving and reopening the board.
- **Held.**

### VF-19 (GAP): `humata board` reads one item and lists any board the caller may read
- **Evidence:** board README (cases 10, 12); `cli/src/commands/boards.rs:84,112`; `cli/src/args.rs:549`.
- **SPEC:** BD-9; IN-17 ("Global board, or other channels and their boards: read", Allow); `api/MCP.md` "the same call through the CLI does the same thing".
- **Decisions:** D155; D127.4; D331.
- **Ruling:** add `humata board show <key>` (with history) and `--board`/`--channel`/`--global` on `ls`, matching the MCP tools one for one.
- **Test:** from a session tab, `board show PAY-1` returns the item and its history, and `board ls --global` returns the Global board's items.
- **Held.** Case 14 can run on MCP once VF-1 lands.

### VF-20 (BUG): A board write from the CLI carries the revision the caller read, so a stale write is refused
- **Evidence:** board README (case 12): `humata board` "sends no revision"; `cli/src/commands/boards.rs:55`.
- **SPEC:** SL-7 "An edit made from a stale revision is not applied ... Board fields ... may be written without a revision; then the later write wins".
- **Decisions:** D383 (main ruled a silent stale win a bug; the drawer now sends the revision); D155 (a write without a revision never conflicts).
- **Ruling:** `board edit` and `board move` take `--revision <n>`; `board show` and `ls --json` print it. Without the flag the write still goes through (SL-7 allows it), but the MCP tools and the CLI then behave alike, and the Board skill text tells agents to pass it.
- **Test:** an item read at revision 3 and changed elsewhere refuses `board move --revision 3` with a revision conflict naming both, and the item is unchanged.
- **Now.** An agent and the person moving the same item is the Board real-agent flow. File: `cli/src/commands/boards.rs`.

### VF-21 (BUG): An agent's new item lands on the same board and in the same column through the CLI and through MCP
- **Evidence:** board README likely bug 4 (case 12). `humata board add` goes to the channel board (`cli/src/commands/boards.rs:31`); `board_item_create` needs `board`, and MCP's unset default is the session board (`daemon/humatad/src/mcp/boards.rs:35`).
- **SPEC:** `api/MCP.md` "MCP adds no behaviour of its own"; BD-1 (three kinds of board); BD-9; IN-17 ("Its own session board: add, edit, move, delete", Allow).
- **Decisions:** D155; D180; D216 (session boards).
- **Ruling:** one default, resolved in the daemon, not in either door: a call bound to a session with no board named goes to that session's board; a call from the person names a board or a channel. The default column is To do in both. Reason: the session board is the agent's own and the least surprising place for work it invents; the channel board is one flag away.
- **Test:** from one session, `humata board add "X"` and `board_item_create {title:"Y"}` make X and Y on the same board in the same column.
- **Now.** Case 14 checks where a real agent's items land. Files: `cli/src/commands/boards.rs`, `daemon/humatad/src/mcp/boards.rs`.

### VF-22 (GAP): An agent's CLI ask carries its reason, and the person reads it on the card
- **Evidence:** board README (case 13): `humata board` "gives no reason"; `cli/src/commands/boards.rs`.
- **SPEC:** BD-15 "who asks and for what ..., the agent's reason"; IN-10.
- **Decisions:** D37 ("`reason` and `wait_seconds` on every rule-governed tool"); D177 (the quoted reason); D93; "inbox requests carry full decision context" (owner).
- **Ruling:** `--reason` on every `humata board` write, sent as the MCP tools send it.
- **Test:** `board move GLB-1 done --reason "shipped"` from a session tab raises a request whose reason is "shipped".
- **Held.** MCP carries the reason tonight.

### VF-23 (BUG): The model named at a start reaches the agent, or the start is refused
- **Evidence:** board README (case 14); `copilot-remote.md` finding 8 (the session asked for `claude-haiku-5.5`; Copilot's footer shows "GPT-5.6 Luna", evidence `24`–`28`, `44`). `daemon/humatad/src/session_service.rs:488` records `model`; `daemon/sessions/src/agents.rs` has no slot for it; `cli/src/args.rs:431`.
- **SPEC:** SE-2 "the dialog does not select or override its model"; ST-15 "Humata sets no model". VERIFY: "Copilot on its cheapest model".
- **Decisions:** D76 and the New session revision at `DECISIONS.md:584` (no model row in the app); D212 ("The fork's model is the new agent's default").
- **Ruling:** the app keeps no model picker. The CLI and MCP `--model` is honoured through a `{model}` slot in the kind's start and resume args (Copilot and Claude `--model`), and refused for a kind with no slot. It is never recorded and ignored. Reason: scripts need it, and a recorded model that didn't apply is a false record that costs money.
- **For the lanes:** every result so far that says `claude-haiku-5.5` ran on Copilot's default unless the lane home's own config set it. Until the fix, set the model in each lane home's Copilot config, and correct the model line in the results.
- **Test:** a start with `--model m` launches the stub CLI with that model in its argv, and so does its resume; a start of a kind with no model slot is refused naming the flag.
- **Now.** It breaks VERIFY's "cheapest model" criterion and spends on the wrong model. Files: `daemon/sessions/src/agents.rs`, `daemon/humatad/src/session_service.rs`.

### VF-24 (UX): A card's ask line says what is being asked
- **Evidence:** board README likely bug 1 (case 6); `flows/10-the-board.md:105`. `ui/src/board.rs:421`, `ui/src/panel_board.rs:170` show "Asks to move to Done" for every ask.
- **SPEC:** BD-15 "who asks and for what".
- **Decisions:** D177 (move, edit and delete wordings); D155 (the app words the buttons from the change).
- **Ruling:** settled by D177: move "Asks to move to X", edit "Asks to edit this", delete "Asks to delete this".
- **Test:** cards with a pending move to Review, an edit and a delete show three different lines naming each change.
- **Held.**

### VF-25 (BUG): A new item made with + on a column is in that column, or isn't made
- **Evidence:** board README likely bug 2 (case 1); `flows/10-the-board.md:25`. `features/board/src/lib.rs:699` creates, then moves; a failed move leaves the item in To do.
- **SPEC:** BD-2 "create an item on any board with New item or the" column's +.
- **Decisions:** D155 (ranks, one rank space per column); D442.
- **Ruling:** one call: the create carries its column. Reason: two calls also make two events and a brief wrong card for every other viewer.
- **Test:** with the daemon refusing moves, + on In progress either makes the item in In progress or makes nothing; one BoardItem created event is seen.
- **Now**, as the "default column differs by door" half of VF-21: the create call gains the column for both. File: `features/board/src/lib.rs` and the create request.

### VF-26 (BUG): The app and the daemon agree on the board's columns
- **Evidence:** board README likely bug 3 (case 1). The daemon has five columns; the app model has seven (`clients/src/model.rs:1390`, `clients/live/src/boards.rs:38`).
- **SPEC:** BD-1; BD-3 (**Status**); BD-13 (Dashboard counts).
- **Decisions:** D334 ("Blocked and Won't do have no board column, so the menu leaves them out; an item that already has one still shows it").
- **Ruling:** five everywhere. Drop Blocked and Won't do from the app model and its fixtures; "blocked" is what Depends on shows. Reason: no door can set them, so they can only appear in fakes and mislead tests.
- **Test:** every status the app's model can hold round-trips through the live client to the daemon and back unchanged.
- **Held.**

### VF-27 (UX): A refused write on the session panel's board shows why
- **Evidence:** board README likely bug 5 (case 8). `ui/src/panel_board.rs` never reads `board.error`; `ui/src/board.rs` draws it on the page only.
- **SPEC:** BD-8 (the session strip's Board panel); SL-7 "the editor shows the current value and asks again".
- **Decisions:** D383 ("the board shows why, reads the board again and keeps the drawer open"); D216; "show problems only where they block an action" (owner).
- **Ruling:** the panel draws the same error line as the page, with the same dismiss.
- **Test:** a stale move made from the session panel leaves the card where the daemon has it and shows the refusal's line in the panel.
- **Held.**

### VF-28 (BUG): An agent's pending ask survives a daemon restart
- **Evidence:** board README likely bug 6 (case 6): "The agent's waiting call ends; the card goes".
- **SPEC:** PER-25 "Turning any machine off at any instant ... loses nothing the user had"; BD-15; IN-1.
- **Decisions:** D155 ("Asks don't survive a daemon restart, as other deferred calls don't", provisional); D38.9 ("store-backed requests next"); D52 (restarts never lose state); D127; D268.
- **Ruling:** PER-25 and D52 win over D155's provisional line. The request is stored; after a restart the card is back, and the agent's `inbox_wait` with its `request_id` finds it again. An approval then runs the write fresh, as D155 says.
- **Test:** raise an ask, restart the daemon, approve it: the item moves and a new `inbox_wait` on the same id returns approved.
- **Held.** No flow tonight restarts a daemon under a pending ask. VF-41's fix touches the same path, so don't make it harder there.

## Repos and agent access

### VF-29 (BUG): `--dry-run` never changes anything, on any command
- **Evidence:** `copilot-repos.md` findings 1 and 2; `copilot-pr.md` F4 (`review start --dry-run` started a review session); `copilot-remote.md` finding 4 (`session move --dry-run` ran the move). `humata access set … --dry-run` wrote the rule (revision 0 → 1): `cli/src/commands/inbox.rs:237-250` sends `SET_CHANNEL_RULE` with `ctx.present_json`, so `cli/src/call.rs:275` is never reached. Likely the same for `channel repo add` (`cli/src/commands/looking.rs:110-116`, not run).
- **SPEC:** `api/CLI.md:64` "`--dry-run`: Print the plan ... change nothing"; IN-17; SEC-13.
- **Decisions:** D6 (access-rule changes are user-only); D730 ("A plan never asks; the call that acts does"); D30.
- **Ruling:** a command with no plan path refuses `--dry-run` as a usage error ("this command has no plan; nothing was changed"), enforced once in `call.rs` so no command can skip it. Commands worth a plan get one later.
- **Test:** for every CLI command, run with `--dry-run` against a daemon that records calls: it either prints a plan or exits with the usage error, and no mutating RPC is recorded.
- **Now.** `session move --dry-run` ran a real move in the remote lane and is being fixed; put the guard in `cli/src/call.rs` in the same change so `access set`, `channel repo add` and `review start` are covered too. Until it lands, lanes pass `--dry-run` only to commands `api/CLI.md` lists with `--plan`.

### VF-30 (UX): The Inbox card for an agent's repo add names the repo, where it will be cloned and why
- **Evidence:** `copilot-repos.md` finding 3: `preview.access_ahead.target = null`, `reason = ""`, title "repos-s1 asks to add a repo to a channel". `daemon/humatad/src/inbox/deferred.rs:142-147` (reason only from a board request, about `:133`).
- **SPEC:** CH-26 "The card (IN-10) shows the repo, its source ..., the machines it will be cloned" on; IN-10.
- **Decisions:** D304 (agents propose a repo); D563; D37 (`reason` on every rule-governed tool); "inbox requests carry full decision context" (owner).
- **Ruling:** settled by CH-26. The title names the repo ("repos-s1 asks to add acme/shared-lib to #repos-a"), and the CLI's `--reason` is carried.
- **Test:** an agent's repo add under Ask me raises a request whose preview holds the repo, its source and the machines, and whose reason is the one given.
- **Held.** The flow passed; the person approved blind.

### VF-31 (UX): The fix after a failed clone fits the channel it failed in
- **Evidence:** `copilot-repos.md` finding 4. `daemon/humatad/src/channel_service.rs:874`: an add to an existing channel says "create the channel again under another name".
- **SPEC:** HO-2-style "what failed and how to fix it"; CH-21; AGENTS.md Errors.
- **Decisions:** D163 (AddRepo takes CreateChannel's path, hence the shared text); D164.
- **Ruling:** two fixes: after a create, today's; after an add, "remove it from the channel, or correct the name and add it again". It depends on VF-32 for the remove.
- **Test:** a failed add to an existing channel returns a fix whose named action succeeds against that channel.
- **Held.**

### VF-32 (BUG): A repo whose clone failed is shown as failed and can be removed
- **Evidence:** `copilot-repos.md` finding 5: after the failed add, `humata --json channels` lists the repo as normal; `channel repo rm` answers `unimplemented`; later session starts will try to clone it.
- **SPEC:** `api/CLI.md:85` (`channel repo add|rm`); CH-21; "main clone cloning/ready/failed" (`DECISIONS.md:891`).
- **Decisions:** D163; D164; D311; "show problems only where they block an action" (owner).
- **Ruling:** a failed add leaves no repo in the channel (the add is undone, as a failed machine add is). `channel repo rm` is built for the repos that do exist. Reason: a failed repo nobody can remove blocks every later start.
- **Test:** after a failed add, the channel's repos are what they were and a session start clones only those; `channel repo rm` removes a healthy repo with a plan.
- **Held.**

### VF-33 (GAP): `humata channel grid` answers
- **Evidence:** `copilot-repos.md` finding 6 (`unimplemented`, GetGrid); `copilot-host.md` F5 ("update Humata on this machine" on a fresh build).
- **SPEC:** `api/CLI.md:85` lists `humata channel grid <c>`; the Repos and machines tab's grid.
- **Decisions:** D331 ("MCP lists only the tools the daemon runs").
- **Ruling:** apply D331's rule to the CLI: a command the daemon doesn't serve is hidden from `--help` until it is built. Build GetGrid with the tab it feeds.
- **Test:** every command `humata --help` lists answers something other than `unimplemented` against the daemon it shipped with.
- **Held.**

### VF-34 (UX): `--wait` waits until the operation ends
- **Evidence:** `copilot-repos.md` finding 7: `channel create`, `channel repo add` and `session start` return `running` at once, even with `--wait 300`.
- **SPEC:** `api/CLI.md` (`--wait`); OP rows for operations.
- **Decisions:** D4 (long waits); D442 (live waits park); D730.
- **Ruling:** `--wait n` on a command that starts an operation follows it to done or failed, up to n seconds, then prints its last state and the `humata ops <id>` line. Reason: "running" after a wait reads as done to a script.
- **Test:** `channel repo add --wait 60` against a slow fake clone returns only after the operation is done, and with `--wait 1` returns `running` plus the operation's id.
- **Held.** Lanes poll `humata ops` meanwhile.

### VF-35 (BUG): A session whose agent waits for an approval says so where the person is looking
- **Evidence:** `copilot-repos.md` finding 9: each Copilot tool call, `humata --help` included, waited in the Inbox, one for 2 min 26 s, with nothing on the session saying so (`f3-inbox-1.json`); `copilot-pr.md` F8 (`gh pr diff` and `humata diff` each waited); `copilot-host.md` F6 (status `working` while a request was pending, and `needs_input` after it was answered and the turn had ended); `copilot-remote.md` finding 2.
- **SPEC:** IN-1 (session scope: the strip's Inbox panel); SE-21; the needs-you status.
- **Decisions:** D191 (the permission hook raises one Inbox card); D192; D199 ("Ask for permission in the Inbox", on by default); D178 (the session panel's card); D124.
- **Ruling:** no new chrome. The session's status is "needs you" exactly while a request of its is pending (not after), whatever the agent's turn state, in `session ls`, the sidebar and the tab, and `humata session read` says "waiting for your approval: <tool>". Read-only tool calls still ask: that is the agent's own permission mode, and D199's switch is the way out. Humata's own tools are the exception (VF-1).
- **Test:** while a permission request is pending, the session's status is needs-you and names the request; once answered it returns to running.
- **Held.** The Inbox itself shows the request, so no flow is blocked.

### VF-36 (UX): The docs say `session repo add` works
- **Evidence:** `copilot-repos.md` finding 10: the gap doc and `workflow/milestones/ROAM.md:67` say `AddRepoToSession` is not built (`daemon/humatad/src/session_service.rs:2366-2372`); it worked in flow 2.
- **SPEC:** CH-21 "Add repo to this session".
- **Decisions:** D563; "OVERVIEW is the living doc" (owner).
- **Ruling:** fix the docs and the stale comment; no code change.
- **Test:** none beyond the lane's passing flow 2; covered by keeping that case in `flows/cases/`.
- **Held.**

## Bug reports

### VF-37 (GAP): The real record, replay, snapshot and Copy-report run happens with the app open
- **Evidence:** `copilot-report.md` flows 1 and 2: BLOCKED. `humata report record|stop|snapshot` talk to the desktop app (`report/src/remote.rs:84-92`); the lane was headless.
- **SPEC:** VERIFY "Bug reports (record and replay, then snapshot): prove both work end to end, in the released app and with `humata report`".
- **Decisions:** D31 (reports reuse the recorders); "No windows over the screen" (owner).
- **Ruling:** an environment block, not a product bug. Main: the app-open run happens at the end, in one session with the owner's screen. Nothing to fix.
- **Test:** that run: a real recording replays to the same steps, views and client calls with secrets scrubbed, and an agent acts on the app's own Copy-report prompt.
- **Held** (until that session).

### VF-38 (GAP): The Help menu's report items, the title-bar dot's stop click and "Clear all marks" each have a test
- **Evidence:** `copilot-report.md` flow 3, "Not covered by any headless test": `desktop/src/main.rs:296-299` (menu), `ui/src/report_view.rs:246-262` (the dot), `ui/src/report_view.rs:553` (Clear all marks); the form's look has no golden.
- **SPEC:** VERIFY "start a recording from the menu, ⇧⌘R or `humata report record`, and check the title-bar dot ... (pen, undo, clear, done)".
- **Decisions:** D31; D76 (goldens); `DECISIONS.md:1252` (the draw toolbar).
- **Ruling:** logged as untested, per main. Add a golden for the report sheet with the three tests.
- **Test:** choosing each menu item starts or stops a recording or takes a snapshot; clicking the dot stops the recording and opens the form; Clear all marks leaves no strokes in the saved report.
- **Held.**

### VF-39 (BUG): A deep HOME doesn't break the app's socket
- **Evidence:** `copilot-report.md` finding 2: `$HOME/.humata2/app.sock` at 157 bytes exceeds macOS's 104; the connect error skips `NotRunning` (`report/src/remote.rs:226-234`) and reports `app_unreachable` (`cli/src/error.rs:357-359`, "Quit and reopen the Humata 2 app").
- **SPEC:** SEC-4 "The local socket is readable only by the user"; PER-25.
- **Decisions:** D178 (the tab's socket); "Portable config" (owner).
- **Ruling:** by main: a BUG. When the path would be too long, the app and daemon bind in a short user-only directory (0700) and leave its path in a file under `~/.humata2`, which clients read. The same rule covers `humatad.sock`. The error wording then needs no special case.
- **Test:** with HOME set to a 150-byte path, the app's and the daemon's sockets bind, `humata report snapshot` and `humata whoami` connect, and the socket's directory is not readable by others.
- **Held** (main's ruling).

### VF-40 (BUG): `humata session queue` lists what is waiting to send
- **Evidence:** `copilot-report.md` finding 3: `terminal.list_queued` answers `unimplemented`, "update Humata on this machine" (`queue-unimplemented.json`). The lane's daemon may be a stale build.
- **SPEC:** SE-23 "A bar says **Waiting to send** with the number of held messages; **Show** lists them".
- **Decisions:** D38.13 (ListQueued returned `peer_version_unsupported`); D381; D325; D331.
- **Ruling:** re-check on fresh binaries first. If it still fails: build it, or hide the command (as VF-33), and never tell the person to update a daemon that is current.
- **Test:** with one message held for a busy agent, `session queue` lists it; with none it lists nothing and exits 0.
- **Held.** Re-check on fresh binaries.

## Remote

### VF-41 (BUG): A Copilot permission ask on a remote machine reaches the Mac's Inbox, and the answer reaches the agent
- **Evidence:** `copilot-remote.md` finding 2: the tool call sits on `humata hook copilot permission` on the VM, `humata inbox` on the Mac is empty, the session stays "working", and neither `humatad2.log` nor `peer-link.log` on the VM has an entry (evidence `27`–`38`). Also: `humata login` in a headless tmux says "Confirm it in the Humata app on the Mac…" with nothing to approve, and without a tty "needs a terminal to bind" (`32`, `33`).
- **SPEC:** IN-1; AW-7 (what each call does, host reachable and away); TR-7 "Inbox requests and answers ... pushed on a stream the moment it happens"; MP-1 "Every machine is treated the same".
- **Decisions:** D191 ("An agent's permission hook holds a stream to its own daemon, which raises one Inbox card"); D192; D199; D268 (linked: the host holds the request; away: the machine holds it and journals); D139 (the relay allowlist).
- **Ruling:** settled by D191 and D268: the machine's daemon forwards the card to the host while linked, and holds it while away. No fallback to the agent's own prompt unless D199's switch is off. A hook that can't reach its daemon writes why to the machine's log; it never waits in silence.
- **Test:** with a fake machine linked, a permission hook fired there raises one request on the host; approving it on the host ends the hook as allowed. With the link down the request is held on the machine and appears on the host when the link returns.
- **Now** (main's ruling). Fix in progress. It blocks every remote Copilot flow.

## Where an agent starts, and reviews

### VF-42 (BUG): An agent asked to work in "the repo" writes inside its clone
- **Evidence:** `copilot-pr.md` F2 (PR flow 1): the file landed in `sessions/<title>/`, the agent said "it isn't a git repo", and `humata diff` showed nothing. The agent starts in the session folder and is told nothing about what is in it.
- **SPEC:** FC-3 "one session clone per chosen repo"; SE-5 "the session starts in an empty folder" (no repos); SE-8 "a `zsh` shell in the session's folder"; CH-21 "tells the agent the repo and its path in one message" (for a repo added later); MV-16; FK-7.
- **Decisions:** D187 ("A context-only fork launches in its parent's session folder, so its cwd holds the parent's clones"); D185; D612 and D347 (conversations are keyed and carried by the session folder); D212 ("relative to the agent's folder"); D460 (the person never picks folders); D634 (a review's clones).
- **Ruling:**
  1. **It is a bug, in what the agent can find out, not in where it starts.** The start folder is by design; the defect is that an agent that asks Humata what its session holds gets no answer.
  2. **The start folder stays the session folder, in every case.** One repo: the session folder, with the clone as its one subfolder. Several repos: the session folder, the only place that holds them all. A review session: the same, with the PR's clones in it. A start folder that depended on the repo count would change meaning the moment a second repo is added (the cwd can't move under a running agent), and forks, moves and conversation carries all key on this folder.
  3. **The fix:** the agent can ask. `humata whoami`, and a tool on the session's MCP server (VF-1), return the session's repos when called. That is a read the agent chooses to make. Humata pushes nothing: the `initialize` instructions and the tool descriptions say what the tools do, with no repo paths, branches or folder hints, and Humata types no opening line into the TUI. The person's prompt says where to work.
  - Reason: least change to proven machinery, and it fixes every repo count the same way.
  - **Corrected 2026-11-17:** this fix first put the folder and each clone's path and branch in the `initialize` instructions, and in a first-message line for agents with no MCP server. The owner decided on 2026-11-16 that Humata does not tell agents where repos are, and a person prompts the TUIs (AGENTSET, D720 to D729). A ruling can't override an owner direction, so both are gone and only the read on request stays.
  - Owed in step 4: a live check per agent that the repo's own instruction files (AGENTS.md, CLAUDE.md, skills) load when the clone is a subfolder. If one doesn't, a Fable call decides between passing the clone to that agent's add-dir option and starting one-repo sessions in the clone.
- **Test:** a session with one repo and one with two each start a stub agent: its `initialize` instructions and tool descriptions hold no folder, clone path or branch, and its `humata whoami` call (and the same read over MCP) returns every repo of the session, each of which exists and is a git work tree. Then the live case: a person's prompt that names the repo has Copilot add hello.txt to it, and `humata diff` shows the change.
- **Now.** It rides on VF-1's change. Files: `daemon/mcp/src/server.rs`, `daemon/humatad` (whoami).

### VF-43 (UX): `humata review start` with no machine reviews on this computer
- **Evidence:** `copilot-pr.md` F5: "this command needs --machine <name|id>".
- **SPEC:** PR-21 "Which machine a review clones on is the user's choice, as for any session (PR-4): This Mac or any machine"; PR-4.
- **Decisions:** D460 (the person picks machines, never folders); D543 (This Mac reads as `this-mac`).
- **Ruling:** default to the host, as `session start` does; the result names the machine it chose.
- **Test:** `review start <pr>` with no `--machine` starts the review on the host, and with `--machine m` on m.
- **Held.**

### VF-44 (UX): A CLI error's fix names CLI commands
- **Evidence:** `copilot-pr.md` F6: `humata view open no-such-view` says "list the session's views (views_list or ListViews)"; the command is `humata views`.
- **SPEC:** `api/CLI.md` (exit codes and fixes); AGENTS.md Errors ("for the person who has to fix the problem").
- **Decisions:** D331 ("its refusals say what to do"); D25; D674.
- **Ruling:** the daemon's fix names the operation id; each door renders it in its own words (the CLI as `humata …`, MCP as the tool). One table, so no message hard-codes a door.
- **Test:** for every operation a fix can name, the CLI's rendering is a command `humata --help` lists, and MCP's is a tool `tools/list` returns.
- **Held.**

### VF-45 (GAP): The owner can add, edit and drop a PR draft from the CLI
- **Evidence:** `copilot-pr.md` F7: `humata comment` has add, send, ls and rm, no edit; `comment add` in a review session makes a terminal-destined comment, so `review submit` lists `drafts: []`.
- **SPEC:** PR-11 "Drafts ... with **Edit** and **Drop**. The user can add their own drafts ... **Draft for GitHub** beside **Comment**"; PR-12 "each editable or droppable"; `api/MCP.md` "the same call through the CLI does the same thing".
- **Decisions:** D138 and D151 (one comment model, drafts and hand-over); D287 (submitting a review safely); D104.
- **Ruling:** not intended. Add `humata comment add --draft` (PR-destined, review sessions only), `comment edit <id>`, and let `comment rm` drop a draft. An agent's `review_draft` stays the agent's door.
- **Test:** in a review session, a draft added with `--draft`, then edited, shows in `review submit --dry-run` with the edited text; after `comment rm` the plan lists none.
- **Held.** PR flow 2's "edit one draft" runs in the app meanwhile.

## Host sessions, Desk and moves

### VF-46 (BUG): `session fork` never takes a channel it then ignores
- **Evidence:** `copilot-host.md` F2: `session fork <s> --kind code-only --channel host-ch2` forked into the parent's channel with no error (`04-fork-code-only.json`, `06-ls-final.txt`); the plan names no channel.
- **SPEC:** FK-1 "The new session sits under its parent in the tree"; CV-8 "**Continue in another channel…** starts a new top-level session in the chosen channel"; `api/CLI.md:109-110`.
- **Decisions:** D551 (continue in another channel reaches the CLI and MCP); D172 (a fork's parent is shown, never rewritten).
- **Ruling:** a fork stays in its parent's channel. `session fork --channel` is a usage error whose fix names `humata session continue <s> --channel <c>`.
- **Test:** `session fork --channel other` exits with the usage error and makes nothing; `session continue --channel other` makes the session in `other`.
- **Held.**

### VF-47 (BUG): A Desk-to-channel move that will be refused changes nothing first
- **Evidence:** `copilot-host.md` F11: the refusal came after `add_repos` had run, so the channel gained repos for a move that didn't happen. `api/proto/humata/v1/desk.proto:120-128` says it fails before anything changes.
- **SPEC:** DK-15 to DK-19; CV-3.
- **Decisions:** D556 ("refused as AgentLacksFeature before anything changes"); D305 (moving a Desk session into a channel); D730.
- **Ruling:** settled by D556. The conversation check runs in the plan and again as the operation's first step, before `add_repos`. The proto comment loses its Copilot-specific line: the refusal is about an unreadable conversation, whatever the agent.
- **Test:** a Desk session with an unreadable conversation is refused the move, and the target channel's repos, folders and sessions are what they were.
- **Held.** VF-7 removes tonight's trigger.

### VF-48 (BUG): A Desk start that fails doesn't stay "starting"
- **Evidence:** `copilot-host.md` F10: after the holder failed to start, `host desk one` (`se_2fbd1f5f…`) stayed in `starting` for the rest of the run.
- **SPEC:** DK-1; PER-25; HO-2-style "says what failed and how to fix it".
- **Decisions:** D182 (the Desk record and DeskService); D731 (a Desk start survives the host dying before the answer); D432; D124 (Exited needs attention).
- **Ruling:** a start that fails ends as failed with its reason and Try again, and is removable; a `starting` record with no live operation is settled the same way when the daemon next reads it.
- **Test:** a Desk start whose holder can't spawn ends as failed with the cause; retrying it starts the same record, and it never lists as `starting` once its operation has ended.
- **Held.**

### VF-49 (GAP): The CLI can start a Desk session and move it into a channel
- **Evidence:** `copilot-host.md` F3: `humata desk` has only `keep`; the lane used raw gRPC (`DeskService/StartDeskSession`, `MoveDeskSessionToChannel`). `cli/src/args.rs` about 230–340 and 488.
- **SPEC:** DK-1 (Desk sessions); DK-15 to DK-19 (the move into a channel); `api/MCP.md` "Every action a person can take in Humata has an MCP tool or is listed in §7 as user-only ... the same call through the CLI". No requirement moves a channel session to another channel: CV-8's copy (`session continue --channel`) is that flow, and VERIFY says "as far as that is supported".
- **Decisions:** D182; D305; D551; D460 (no folders asked).
- **Ruling:** add `humata desk start [--agent a] [--machine m] [--title t]` and `humata desk move <s> --channel c [--dry-run|--plan t]`. Don't add a channel-to-channel move: the session's clones belong to its channel's repos, so the honest operation is CV-8's copy. `flows/` should call that flow "continue in another channel".
- **Test:** `desk start` makes a Desk session; `desk move --dry-run` prints the plan (repos to add, where it lands) and changes nothing; with the plan it lands in the channel with its conversation.
- **Held.** The RPCs work; only the door is missing.

### VF-50 (UX): A machine is found by the name `host ls` prints
- **Evidence:** `copilot-host.md` F4: `--to "This Mac"`, `host show "This Mac"` and `"this mac"` fail with "no machine is named …"; `this-mac` works.
- **SPEC:** HO-3 (the Hosts table); MV-1.
- **Decisions:** D543 ("This Mac's root is stored on This Mac and reads as `this-mac`"); D164; D460.
- **Ruling:** a machine argument matches the shown name (case ignored), the slug or the id; a miss lists the machines by name.
- **Test:** `host show "This Mac"`, `"this mac"`, `this-mac` and the id return the same machine, and a wrong name's error carries the known names.
- **Held.**

### VF-51 (UX): A fresh session at its prompt is Idle, not Probably idle
- **Evidence:** `copilot-host.md` F7: Copilot sessions and a code-only fork showed `probably_idle` before any prompt. `clients/live/src/map.rs:199,314`.
- **SPEC:** SE-22 "A session whose turn end comes from the output fallback shows **Probably idle**, never Idle"; SE-21 (Copilot's turn ends come from its hooks).
- **Decisions:** D193 ("the kinds keep `turn_end: output` until a live check shows the hooks run"); D7; D202.
- **Ruling:** no new state. A kind whose hooks have been heard on this machine (D202's record) starts as Idle; Probably idle is kept for kinds truly on the output fallback. These runs are the live check D193 was waiting for, so Copilot moves to hooks.
- **Test:** a new session of a hook kind with a heard hook lists as idle before its first turn; a custom kind with no hooks lists as probably idle.
- **Held.**

### VF-52 (BUG): `session read` shows the agent's reply when its turn ends
- **Evidence:** `copilot-host.md` F8 and F13: the last reply was missing from `session read` for at least 1.5 minutes while `humata conversation` had it; `copilot-fork.md` flow 6 saw the same about 5 s after a prompt.
- **SPEC:** TR-7 "pushed ... the moment it happens ... Inside a machine, changes Humata doesn't make itself" are watched; LD-21 (the mirror).
- **Decisions:** D675 (the history mirror learns a conversation); D211; D389 (the rig waits on turn ends).
- **Ruling:** the turn-end hook is the mirror's cue: it reads the conversation's tail then, not at the next prompt.
- **Test:** after a stub agent appends a reply and fires its turn-end hook, `session read` returns that reply with no further input.
- **Held.**

### VF-53 (UX): An approval's diff preview has well-formed headers
- **Evidence:** `copilot-host.md` F9: `--- a/dev/null` and `diff --git a/private/tmp/…` for a new file at an absolute path.
- **SPEC:** IN-10 (previews by kind of request).
- **Decisions:** D191 (the permission card); D466 (paths are hidden in Humata's own chrome).
- **Ruling:** a new file's old side is `/dev/null`; a path inside the session folder is shown relative to it, and one outside it in full.
- **Test:** the preview of a new-file write parses as a unified diff with `/dev/null` as its old side and the file's session-relative path as its new one.
- **Held.**

### VF-54 (BUG): A remote session that never had a turn moves back to the host, or is refused before anything is copied
- **Evidence:** `copilot-remote.md` finding 6: pack and copy passed, then resume failed with "the agent's conversation file is missing", and the session stayed on the remote (`71`–`73`). Host to remote with no turns works.
- **SPEC:** MV-2 (a move carries the conversation); MV-13; MP-1 "Every machine is treated the same"; D670's note that Copilot writes `events.jsonl` once the first prompt is taken.
- **Decisions:** D195; D207 (the host recovers a move); D675; D741.
- **Ruling:** a conversation that was never written is not missing: the move carries the session and starts the agent fresh on its recorded id, in both directions.
- **Test:** a session with no turns moves remote to host and host to remote, comes up at its prompt, and its first turn after the move is recorded under the same conversation id.
- **Now.** Fix in progress (main).

### VF-55 (UX): A new session doesn't stop at the agent's folder-trust prompt
- **Evidence:** `copilot-remote.md` finding 3: Copilot's "Confirm folder trust" shows for every new session folder on a VM (`24`, `25`, `41`). The same holds on a Mac that has no lane-made trust.
- **SPEC:** SE-2 "an optional first message that is typed into the agent once it starts"; MP-1; SEC-5.
- **Decisions:** D193 (no `COPILOT_HOME`; no `--dangerously-bypass-hook-trust`, "it also runs the user's own untrusted hooks"); D202 (Codex's hook trust is the person's own step); D722 (Humata never installs or upgrades agent CLIs unasked).
- **Ruling:** yes, Humata trusts its own session folders, narrowly. At each start, on the machine it runs on, the session's folder alone (never the root, never the home) is passed to the agent as trusted: by a launch flag where the CLI has one, else by one added entry in the agent's trusted-folder list, removed when the session is archived. Host and machines alike. Hook trust, tool approvals and MCP trust are not touched. Reason: the person chose the repo and the agent, which is the decision the prompt asks for; and with VF-14 fixed, a first message would otherwise be typed into the trust prompt. It is less safe than the prompt for a repo an agent proposed, so the Inbox card for a repo add (VF-30) says the repo's files will be trusted by agents.
- **Test:** a first start in a new session folder reaches the stub agent's prompt with no trust question and its first message starts a turn; after archive the agent's trusted list is what it was before the start.
- **Held.** Lanes pre-trust in the harness tonight. Raise to Now if the VF-14 rerun shows a first message eaten by the prompt.

### VF-56 (UX): A failed clone on a machine can be tried again
- **Evidence:** `copilot-remote.md` finding 9: `humata ops retry <op>` says "can't run again"; removing and re-adding the machine to the channel is the only way (`19`–`21`).
- **SPEC:** HO-2 "says what failed and how to fix it, and offers Try again".
- **Decisions:** D164 (a channel clones one call at a time); D163; D311.
- **Ruling:** the failed clone step is retryable from its operation and from the channel's Repos and machines row.
- **Test:** a clone that failed for a reason since fixed succeeds on `ops retry`, with no machine removed.
- **Held.**

## Files, and an agent's shell approvals

### VF-57 (UX): An agent's repeated shell calls don't each wait in the Inbox
- **Evidence:** `copilot-files.md` flow 5 (`93`, `94`) and finding 5 (`122`): one copy task made 13 "wants to run bash" cards, each approved by hand. Most were `humata --help`, `humata cp --help`, `humata files --help`, `humata channels` and the `humata cp` attempts; the first was `find / -name agent-note.txt`. Same shape as `copilot-repos.md` finding 9 and `copilot-pr.md` F8 (VF-35).
- **SPEC:** IN-14 (the card's **Always allow …** adds a grant "for the requesting session and exactly the target", "The agent cannot choose a wider scope"); IN-19 (grants are listed and removable); IN-21; IN-22 (the target table, which has no row for a shell command); IN-23 (only the user changes grants); SEC-5 (an agent that isn't contained runs as the user); SEC-8. The task named PER- and AC- requirements: PER is persistence and holds no access rule, and SPEC.md has no AC- ids, so IN-14 to IN-23 are the ones that apply.
- **Decisions:** D191.3 ("No "Always allow": the agent's own rules are a different thing from a Humata grant (deferred)"; Answer all skips these cards) and D191.2 (a payload not read whole gets no decision); D199 (one host-wide switch sends every ask back to the agent's own prompt); D155 and D180 (a standing approval is made per target, by the person, and shown as a grant line); D743 (no flag, variable, message or tool description names a clone's path or branch); VF-1 (the `humata` command and server are pre-allowed at launch); VF-35 ("Read-only tool calls still ask … D199's switch is the way out").
- **Ruling:** three parts.
  - **The `humata` command needs no new rule.** VF-1 already pre-allows it at launch, because the daemon checks every call against the channel's rules. Once VF-1 is in, most of flow 5's 13 cards are never raised (the lane names at least eight `humata` commands). If a run with VF-1 in still shows a card for a `humata` command, that is a VF-1 bug, not this item.
  - **No "bash allowed" rule,** for a session, a channel or an agent kind. A shell is everything the person can do (SEC-5), pushes and deletes included, which Humata's own rules hold at Ask me or Never; one tick made while clearing a pile of cards would end the only gate there is. A person who wants an agent that never asks has two deliberate ways already: the agent's own allow-all flag in that kind's launch command (IN-23), or D199's switch and the agent's own prompt.
  - **Yes to one program, for one session.** This closes what D191.3 deferred.
    - **Scope:** the session that asked, and one program name (`find`, `git`, `ls`). Not its forks, not the channel, not the agent kind, not other sessions. A tool that isn't a shell is granted by its tool name the same way.
    - **Who sets it:** only the person, while answering a card: the IN-14 checkbox, labelled "Always allow `find` commands in this session", or `humata inbox approve <request> --always`. No setting, no agent call (IN-23), not Answer all, and never the While I'm away rule.
    - **When the checkbox is offered:** only when the whole command was read as one program. Never for a program that runs other commands (a shell, `env`, `xargs`, `sudo`, an interpreter), since that would be "bash allowed" by another name.
    - **How it applies:** the daemon answers the hook "allow once" with no card when the command is read whole as plain commands joined by `|`, `&&`, `||` or `;` and every program in it is granted to that session or is `humata`. A substitution, a redirect into a file, a subshell or anything not read whole asks as before (D191.2). Matching a prefix of the text is not enough: `find . ; rm -rf ~` starts with `find`.
    - **How it is shown:** a grant line in the channel's Agent access, as IN-19 lists the others ("<session>: runs `find` without asking"), and in `humata access grants`. No new chrome on the session.
    - **How it is revoked:** Remove on that line, or `humata access revoke <grant>`; the next call asks again (IN-21). It ends when the session is archived. It is kept with the session's other grants, so it survives a restart and a move.
    - **D743:** the grant is a program name held in the daemon. Nothing is passed to the agent (no flag, variable or message), and the rule never names a folder. A rule such as "allow the shell inside the clone" is refused for this reason: it would have to hand the agent a path. The agent's `find /` for a file it wasn't told the place of follows from D743 and stays the agent's to solve by asking `humata`.
  - **Reverse:** drop the checkbox from permission cards; D191.3 stands as written. Needs a DECISIONS.md entry amending D191.3 and a row for IN-22 when built.
- **Test:** against the stub agent's permission hook: (1) approve `find a` with Always allow, then `find b` and `find b | humata whoami` are allowed with no card; (2) `find b; rm c`, `find $(cat d)` and `find b > e` each raise a card; (3) the same `find b` from a fork and from another session of the channel raises a card; (4) a card for `sh -c …` offers no Always allow, and an agent's own call to add the grant is refused; (5) after the grant is removed the next `find` raises a card; (6) the agent's launch arguments and environment are the same with and without the grant.
- **Held.** Approvals work, so no flow is blocked; VF-1 removes most of the cards tonight.

### VF-58 (BUG): A file keeps its exact mode through the trash, a restore, a copy and a move
- **Evidence:** `copilot-files.md` flow 4 (`66`) and finding 8 (`128`): after `humata mv` sent `from-vma.txt` to `~/Humata/.trash/<id>/content` on vm-a and `humata trash restore` brought it back, its mode was 664, not the 644 it had; the sha was the same (`files/f4-trash-vma-after-mv.json`, `files/f4-trash-restore.json`).
- **SPEC:** LD-7 "Anything in trash can be restored"; FM-3; FM-6 (a file counts as transferred when its checksum matches, which says nothing of its mode); FM-14.
- **Decisions:** D142 (the trash's `item.json` and `content`; a move on one file system is a rename, otherwise copy, verify, then the source goes to trash; "restore never overwrites"); D156 (transfers between machines). None of them names the mode.
- **Ruling:** a file's mode is part of the file. A restore gives back the exact mode the file had when it was trashed, whatever the daemon's umask; the move into the trash keeps it; and `cp` and `mv`, on one machine and between machines, write the source's exact mode at the destination. This holds for every permission bit, the executable bit included, and for each file inside a folder. A 644 that comes back 664 is group-writable, and a script that comes back without its executable bit doesn't run.
- **Test:** files with modes 644, 600 and 755, alone and inside a folder: after trash then restore, after `cp`, and after `mv` to another file system and its restore from the trash, each file's mode and sha equal the original's, with the daemon run under a umask of 002 and of 077.
- **Now.** Being fixed now, test-first: the failing test is written before the fix. Files: `daemon/transfers`.

### VF-59 (BUG, Now): A copied folder keeps its exact mode
- **Evidence:** found in code while working on VF-58. `daemon/transfers/src/disk.rs:166` (`Disk::make_folder`) creates a copied folder as `mode | 0o700` and never applies the exact mode afterwards, so a 0555 folder arrives as 0755.
- **Ruling:** same as VF-58. The mode is part of the file. Apply each folder's exact mode once its children are written, locally and on the remote end.
- **Test:** a copied tree with folders at 0555, 0700 and 0750 arrives with those exact modes, locally and through the remote end.
- **Now** (silently wrong data). VF-58 note: the lane recorded no `stat`, the code keeps file modes through trash and copy, and the umask-002 tests are queued. Unless they fail, VF-58 is "not reproduced on current code".

### VF-60 (BUG, Now): A Copilot session whose agent ran a background shell can still be forked, continued and moved
- **Evidence:** D746 "Still refused: `system.notification` (background shells); a session that ran one forks code-only" and `daemon/conversation/src/copilot.rs` ("stays unknown"). The record was met in a VERIFY repos-lane session, but `copilot-repos.md` doesn't name it and `daemon/conversation/testdata/copilot/` holds no capture of it, so the shape below was read from a real signed-in 1.0.113-1 `events.jsonl` (six records, all `shell_completed`; field names and kinds only) and from the CLI's own `copilot-sdk/generated/session-events.d.ts`.
- **SPEC:** CV-3 (a format it can't read lists no turns; a move refuses); CV-7 ("an unknown record kind or path field makes it unknown"); CV-5 (background jobs and shells the agent started are not carried, and the dialogs say so); CV-2; CV-4; FK-3; MV-3.
- **Decisions:** D673 (records known from the CLI's own event schema, marked X2 until captured); D746 (1.0.113-1's records known with exactly the fields seen; this one left for a separate ruling); D307.
- **Ruling:** allowlist it, narrowly. Keeping the refusal protects against nothing here and costs every flow VERIFY runs: agents start background shells for builds, tests and servers all the time, and one such call made the whole conversation unforkable at a turn, uncontinuable and unmovable.
  - **Fields known:** top level `type`, `id`, `timestamp`, `parentId` and the optional `agentId`; `data.content` (text: `<system_notification>Shell command "<description>" (shellId: N) has completed …</system_notification>`); `data.kind` with `type`, `shellId`, optional `exitCode` (a number) and optional `description`; `data.responsesReasoning` with `model`, `initialEffort`, `effort`.
  - **Kinds known:** `kind.type` of `shell_completed` (captured) and `shell_detached_completed` (same fields without `exitCode`; from the schema, marked X2 as D673 does). The schema's other kinds (`agent_completed`, `agent_idle`, `new_inbox_message`, `instruction_discovered`) stay refused until a capture shows them: `instruction_discovered` has two path fields of its own (`sourcePath`, `triggerFile`) and `agent_completed` holds a whole prompt.
  - **Paths:** no field is a path field. No session id, no cwd, no file name; `shellId` is a small counter of the process that ran the shell. `description` is the model's own label for the command and `content` repeats it, so either can quote the folder as any prose can. They get the same whole-line rewrite of the old folder that a fork and a carry already give `tool.execution_start`'s `arguments`; nothing new is rewritten. The strict rule stays on: a field not listed above that holds a path still refuses the conversation.
  - **Turns:** it is not a `user.message` and lists no turn (CV-2). The reply it triggers is an `assistant.turn_start` … `assistant.turn_end` with no prompt before it, and belongs to the turn above. A cut never leaves a notification without its finished reply: fork before turn n keeps all of them above prompt n, and from now, while that reply is still running, ends after the last finished `assistant.turn_end` before the notification.
  - **The shell itself** is not carried (CV-5, said in the dialogs already). A notification for a shell that finished stays in the history as what happened; a `shellId` in a carried conversation names nothing live, as the CLI's own resume in a new process already has it.
  - **If it is still refused** (another kind, or an unknown path field): nothing new. CV-3's lines stand: the fork dialog offers from now, a move refuses before anything is copied and offers **Start fresh with a summary**.
  - **Reverse:** drop the entry and the fixture; D746's "Still refused" line holds again. Needs a DECISIONS.md entry amending D746, and the record's row in `copilot.rs`'s table.
- **Test:** a scrubbed real capture added to `testdata/copilot/1.0.113-1/` (a turn that starts a background shell, its `system.notification`, the reply that follows, then one more prompt): (1) the strict parse accepts it and lists two turns, none for the notification; (2) fork before turn 2 holds the notification and its reply, and the fork resumes by id; (3) a carry to another home and folder, with the old folder quoted in `description` and `content`, leaves no old path in any line and resumes there; the parent is unchanged; (4) the same record with an extra field holding a path, and one with `kind.type: instruction_discovered`, are each `Unsupported`; (5) cut from now while the notification's reply has no `assistant.turn_end` drops the notification and the partial reply.
- **Now.** It blocks Copilot's fork at a turn, Continue in another channel and move tonight for any session that ran a background shell. Files: `daemon/conversation/src/copilot.rs`, `daemon/conversation/testdata/copilot/1.0.113-1/`.

## Looked at, not feedback

- **"Run on: This computer" on a remote session** (`copilot-remote.md` finding 7) and the footer's model name are Copilot's own footer, drawn on the machine it runs on. Humata doesn't rewrite terminal content (D738). The model itself is VF-23.
- **Copilot's own side model calls** (`copilot-host.md` F12) are the agent's behaviour; they matter only through VF-7.
- **Harness and environment:** `lane-up.sh` copying the owner's `~/.copilot/config.json`; `verify/bin` briefly holding Linux binaries; VMs that can't reach GitHub; the batch runner dropping cached passes from `runN.result`.


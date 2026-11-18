# MACHINESCRIPTS: I script what happens when a machine is added, reconnects, or a session arrives on or leaves it

- **Status:** in progress (lane MACHINESCRIPTS)
- **Depends on:** MACH (machine add, reconnect, peer link), the move (`MoveSession`), AGENTKINDS (codex and copilot as built-in kinds), UPGRADE (the `~/.humata2/config` push and `bin/humata` on machines)
- **Unblocks:** CASES rows for remote setup flows; later TS-3 sync rules (the same file shipping and run log)
- **Layer:** L4 · **Lane:** daemon + CLI + clients + UI

## The owner's words (2026-11-16)
"make it so we can script what we do on transfer or registration of a remote machine. Say on registration of a remote machine I want to run `/restart` after opening copilot or run some arbitrary bash script I have or something akin to that. I need to be able to script stuff like that... The goal is to relieve the friction present when working on remote machines or across remote machines or getting setup with remote machines."

## Outcome
The owner scripts what happens when a machine is added, reconnects, or a session arrives on it or leaves it. A step can run a bash script, run a `humata` command, or open an agent and type into it, for example "open copilot, wait until it is ready, send `/restart`". The owner sees each run's result on the machine page, and in the Inbox only when a run fails.

## Exit criterion
- With a rule `on = "added"` whose steps are a script and "start copilot, then send `/restart`", adding a fake machine runs the script there with the trigger's variables. A copilot session then starts in the rule's channel, and `/restart` is typed once the session is idle. The machine page shows the run as passed, with each step's exit and the end of its output.
- A rule `on = "session-arrived"` types into the moved session's own agent on the new machine after the move ends. A rule `on = "session-leaving"` runs on the old machine before its agent stops, and never holds or fails the move.
- A failing step stops its rule. The run shows as failed on the machine page, and one Inbox item "Script <rule> failed on <machine>" appears with **Retry**. A second failure of the same rule replaces that item. A later pass withdraws it.
- `humata machine script list|check|run|log` work against a fake daemon and the live host. `run` is refused for agents unless the channel's access allows `machine_script_run`, which is Never by default.
- Tests: rule parsing and checks (machines crate), the run engine on fake steps (humatad unit tests: order, stop on failure, timeouts, `once`, the wait for idle, the dedup of one link-up), the CLI on `FakeDaemon`, and the machine page reducer on `FakeMachines` (TestStore). None of them asserts on wording.

## Design (spec)

### Where rules and scripts live (D680)
- **Rules** are in one file on the host, `~/.humata2/config/machine-scripts.toml` (portable config). There are no compiled-in rules, so with no file nothing runs.
- **Scripts** are in a folder Humata owns, `~/.humata2/scripts/`. A rule names a script by its file name (`run = "setup-tools.sh"`), never by a path. A name with `/` or `..` in it is refused when the file is checked. The owner never picks a folder. The app's Settings › Machines › Scripts lists the rules and scripts by name, with "Edit rules" and "Reveal scripts" buttons, and Humata makes the folder and a commented sample file the first time.
- The rules file goes to every machine at `~/.humata2/config/machine-scripts.toml` the same way `stack.toml` and `repos.toml` do (D411, RL-12): on setup and on each link-up when it differs, and with the secret filter applied. A machine that later acts as a host (ANYHOST) therefore has the same rules.
- Scripts are not stored on machines. Each run streams the script's bytes to `bash -s` over the machine's link, so a machine always runs the host's current copy and never keeps an old one.
- **No secrets.** Neither file may hold one: secret-looking `key = value` lines are dropped when the file is sent, as for stack.toml. Host variables (HO-7) are never passed to scripts. A script reads the machine's own environment through the login shell (`bash -l`).

### Triggers (D681)
| `on =` | Fires | Where steps run | Variables |
|---|---|---|---|
| `added` | once, at the first link-up after Add machine succeeds | the new machine | `HUMATA_TRIGGER`, `HUMATA_MACHINE` |
| `connected` | every later link-up: reconnect, daemon restart, network back, reboot | the machine | as above |
| `session-arrived` | after a move ends on its new machine, including a move finished by recovery | the new machine; agent steps go to that session | plus `HUMATA_SESSION`, `HUMATA_CHANNEL`, `HUMATA_SESSION_DIR`, `HUMATA_FROM` |
| `session-leaving` | during a move, after the copy is verified and before the old agent stops | the old machine | plus `HUMATA_TO` |
| `upgraded` | once after a Migrate of the machine finishes (its item ends migrated, not withdrawn); queues like `connected` | the machine | `HUMATA_TRIGGER`, `HUMATA_MACHINE` |

- `added` takes the place of `connected` for that first link-up, so a rule that wants both lists both: `on = ["added", "connected"]`.
- A link that flaps runs `connected` once per link-up, never twice for the same one. A link-up while a run for the same machine is still going queues one more run after it; it doesn't start a parallel run.
- `This Mac` (the host) has no triggers, so its own Migrates fire no `upgraded`. "Machine upgraded" is `upgraded` (D689, built in w9).

### Steps (D682)
A rule is `[[rule]]` with `name`, `on`, optional `machines`, `once`, `timeout`, and `[[rule.step]]` entries in order. Each step is exactly one of:
- `run = "<script name>"`, with optional `args = [...]`: the script from `~/.humata2/scripts/`, streamed to `bash -s -- args…` on the machine.
- `humata = ["session", "list"]`: the machine's own `~/.humata2/bin/humata` with these arguments (TS-1 put it there), run there through the link.
- `start = { agent = "copilot", channel = "<channel name>", title = "…" }`: starts a session of that agent kind on the machine in the named channel, through the same StartSession every client uses. Kinds come from Settings › Agents and the built-ins (AGENTKINDS); none are hard-coded here. On the session triggers, `start` is refused when the file is checked: the session is already there.
- `send = "/restart"`, with optional `keys = "enter"` (the default) or `"none"`, and `wait = "idle"` (the default) or `"none"`: types into a session's agent once it is idle, keyed so that a retry never types twice. The session is the one the step before started, or, on a session trigger, the moved session. Waiting uses the session's turn state (SE-21). A kind whose turn end is unknown (`TurnEnd::None`) fails the step "can't tell when <kind> is ready; set `wait = "none"`" and does not type blind.

Order: rules run in file order and steps in step order. The first failed step stops its rule, and the next rule still runs.

### Scope, timeouts, idempotence (D683, D684)
- `machines = ["*"]` is the default, or a list of machine names. A name that isn't a machine is a check warning, not an error, because the machine may be added later.
- Timeouts: a step defaults to 5 minutes and a rule's `timeout` caps all its steps (at most 1 hour). A wait for idle defaults to 2 minutes. Each `session-leaving` step is capped at 60 seconds so a move is never held long. A step that times out is killed and counts as failed.
- `once = true` runs the rule at most once per machine. The first pass is recorded on the host, and a failure is retried at the next trigger. `humata machine script run` runs it again on purpose. Without `once`, scripts should be idempotent, since `connected` fires often. The sample file says so.

### Results (D686)
- **Run log** on the host: under the daemon's state, per machine, the last 50 runs. Each run records rule, trigger, the session if any, start, duration, outcome (passed, failed, timed out, skipped), and per step the exit and the last 40 lines of output. Nothing is polled; the machine page reads the log, and the feed says when it changes.
- **Machine page:** a "Scripts" section on the Overview shows the last run of each rule that applies to this machine, with its outcome and time. A run's row opens its steps and output tails, and each rule has **Run now**.
- **Inbox:** only on failure. A host ask, "Script <rule> failed on <machine>", carries the failed step, its exit, the output tail, and the trigger. **Retry** runs the rule again and **Dismiss** drops the item. There is one item per (machine, rule), so a newer failure replaces the older one and a later pass withdraws it. A failure never blocks other work, so it shows nowhere else.

### CLI and gRPC (D687)
- `humata machine script list [<machine>]`: the rules, their triggers, and for each machine the last outcome.
- `humata machine script check`: reads and checks the rules file and the script names. Exit 0 when the file is valid, 2 when it isn't (usage), with every problem listed.
- `humata machine script run <rule> <machine> [--session <s>] [--wait]`: runs a rule now as an Operation. The trigger is `manual`, or the session's trigger when `--session` is given.
- `humata machine script log <machine> [--rule <r>] [--limit n] [--json]`: the run log.
- MachineService gains ListMachineScripts, CheckMachineScripts, RunMachineScript (Operation kind `MACHINE_SCRIPT`), ListMachineScriptRuns, and the feed payload `MachineScriptRun`. There is no RPC that writes rules or scripts: they change only in the owner's files.

### Safety (D688)
- Rules and scripts come only from the host owner's files. No RPC, CLI or MCP tool writes them, and the scripts folder and rules file sit under `~/.humata2`, which TS-4 keeps out of every sync rule's destination.
- Triggered runs are the owner's own automation, not an agent action, so they need no access check. A machine (peer caller) can't call any of these RPCs.
- **Run now** from an agent (CLI or MCP with an agent's identity) is gated by a new capability, `machine_script_run`, tier NeverByDefault, with its Settings switch under the channel's Agent access. `list`, `check` and `log` are reads and follow `machine_read`.
- An agent session's sandbox (CT) does not let it write `~/.humata2/scripts` or the rules file.

## Read these (pointers)
- SPEC: TS-0 to TS-4, SK-6, SE-21/22, UP-1, IN (host asks, D348), PL (no folder picking). DECISIONS: D49, D51, D348, D411, RL-12, D680–D689.
- Code: `daemon/machines/src/{sync_rules,stack_config,link,manage,runner}.rs`; `daemon/humatad/src/machine_service.rs` (`tell_link`, `add_ssh`), `machine_service/stack_settings.rs`; `session_service/move_session.rs` (`Mover::run`, `hand_over`, `finish`); `terminal.rs` `type_in`, `session_service/start_fresh.rs` `type_seed`, `session_service/turns.rs`; `inbox/host_asks.rs`; `daemon/access/src/capability.rs`.
- API: `api/CLI.md` §5.6, `api/GRPC.md` §4 and §11, `api/proto/humata/v1/hosts.proto`.

## Out of scope
TS-3 file sync rules (the same file shipping, later). Triggers on This Mac. Editing scripts inside the app (the owner's editor opens them). A "machine upgraded" trigger (D689 open).

## Lane (written by the manager)
- Manager: MACHINESCRIPTS (Opus 5.5), opened 2026-11-16. Phase 1 spec above; Phase 2 tasks below.
- Claims (new files unless marked SEAM): `daemon/machines/src/machine_scripts.rs`, `daemon/machines/tests/pending/machine_scripts.rs`; SEAM `daemon/machines/src/{lib,runner,link,manage,stack_config}.rs`; `daemon/humatad/src/machine_scripts/` (new module); SEAM `daemon/humatad/src/{machine_service.rs,machine_service/box_storage.rs,machine_service/stack_settings.rs,session_service/move_session.rs,serve.rs,main.rs or lib root}`; SEAM `api/proto/humata/v1/{hosts,events,operations,capabilities}.proto`; `cli/src/commands/machine_scripts.rs`, SEAM `cli/src/{args,daemon}.rs`, `cli/src/commands/{mod,hosts}.rs`; `cli/tests/pending/machine_scripts.rs`; SEAM `daemon/access/src/capability.rs`, `daemon/humatad/src/guard.rs`; `clients/src/machines.rs` + `machines/fake.rs` SEAM, `clients/live/src/machines.rs` SEAM; `features/hosts/src/scripts.rs`, SEAM `features/hosts/src/page.rs`; SEAM `ui/src/machine_page.rs`; docs SEAM `api/{CLI,GRPC,MCP,BOUNDARIES}.md`.
- Tasks:
  - [ ] T1 rules file: parse and check (`machine_scripts.rs` in machines), `Purpose::MachineScript`, `Machines::script_invocation`, ship the file with stack.toml · worker A
  - [ ] T2 run engine in humatad on step traits (script, humata, start, send), run log, Inbox host ask, triggers in `tell_link`/`add_ssh`/move · worker B
  - [ ] T3 proto + RPCs + CLI `machine script` + docs · worker C (after T1 types)
  - [ ] T4 capability `machine_script_run` (NeverByDefault) · in T3
  - [ ] T5 app: `Machines::scripts/run_script` (fake + live), machine page Scripts section reducer + view · worker D (after T3 proto)
- Scripts: `$S/mscripts/` (apply_*.py, each with `--check`).
- Ready to commit: no

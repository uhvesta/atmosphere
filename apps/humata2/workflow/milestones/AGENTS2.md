# AGENTS2: Claude Code and Codex are first-class agents, read through their own structured channels

- **Status:** next (owner, 2026-11-18: "after ur current roadmap is done begin working on claude code and codex integration")
- **Depends on:** RELEASE3: starts after 0.3 ships
- **Unblocks:** nothing yet
- **Layer:** L13 · **Lane:** daemon + agent_hooks + clients + UI

## Outcome
Claude Code and Codex do everything Copilot does in Humata 2: start, resume,
stop, fork and move, End for good, their own questions in the Inbox, and
conversation tracking that survives `/clear` (Claude Code) and `/new` (Codex).
Each is opt-in per agent in Settings › Agents (AGENTSET, D720–729).

The owner's direction: "I don't like the fact you parse the terminal screen.
Not saying to remove it but just keep that in mind." So Humata reads each
agent's state and questions through the agent's own structured channel first:
- hooks: Claude Code's hooks, Copilot's plugin hooks;
- JSON or stream output;
- Codex's app-server protocol;
- session files.

Screen parsing (`screen_ask`, D786) stays only as a fallback.

## Exit criterion
- With each agent switched on, the VERIFY Copilot cases pass with Claude Code and with Codex in place of Copilot.
- `/clear` (Claude Code) and `/new` (Codex), followed by Stop and Resume, resume the newest conversation. This mirrors the Copilot test in `daemon/humatad/tests/conversation_follows.rs`.
- An approval the agent asks for arrives in the Inbox from its structured channel, not from the screen.
- The fast tier passes. A live drive on the owner's screen shows each agent end to end, with screenshots.

## Read these (pointers, nothing else)
- `daemon/agent_hooks/src/lib.rs`:
  - `parse_conversation`;
  - the Copilot plugin manifest, which shows how a hook reports `conversation`.
- `daemon/sessions/src/agents.rs`: `Agents::follow_conversation`.
- `daemon/humatad/src/session_service/hooks.rs`.
- AGENTKINDS.md (D670–675) and HOOKS.md for the existing kinds and hooks.
- D786 (`screen_ask`) for the fallback being replaced.
- Memory: never install an agent CLI unless that agent's opt-in is on; never modify `~/.claude`, `~/.codex` or `~/.copilot`.

## Out of scope
- Removing screen parsing.
- Agents other than Claude Code, Codex and Copilot.

## Lane

Manager: AGENTS2 manager (Opus 5.5). State: **batch 5 (C3–C5, F1) in progress, 2026-11-18 evening**; batches 1–4 committed. Live checks moved to Owed. Decisions: D795–D799 (Fable 5.2's rulings).

### Research: what each CLI offers (installed: claude 2.1.326, codex-cli 0.169.0, copilot 1.0.114-0)

**Claude Code: hooks, through `--settings <state>/claude-hooks.json`, which is already shipped.**
- SessionStart `source` is one of startup, resume, clear, compact or fork. Every payload carries `session_id` and `transcript_path`. `agent_id` is present only when a hook fires inside a subagent.
- Hooks from every settings file are held back until workspace trust is accepted ("Skipping hook execution - workspace trust not accepted"). So a heard SessionStart means Claude is past its trust screen. That makes it the ready signal (D797).
- PreToolUse matched to `AskUserQuestion` may return `{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow","updatedInput":{questions…, "answers":{…}}}}` (D796). A deny's `permissionDecisionReason` is shown to the model, which is the fallback.
- The PermissionRequest output (`behavior` allow|deny, `message`) is already used.
- The Notification hook's types include permission_prompt, idle_prompt and elicitation_dialog. They are observe-only (D798).
- The transcript is written asynchronously. A conversation is followed once its file exists, and a later hook names it again, as for Copilot.
- Not used: Channels (a research preview), `-p --output-format stream-json` (it replaces the TUI), and the SDK.

**Codex: the app-server protocol, under a wrapper (D795). Its hooks need a one-time `/hooks` trust, so they stay only as the degraded path.**
- The hook events are SessionStart (source startup, resume, clear, compact or fork), SessionEnd, UserPromptSubmit, PreToolUse, PermissionRequest, PostToolUse, PreCompact, PostCompact, Stop, Interrupt, Subagent* and PreModelRequest. Hooks from session flags run only after review in `/hooks`.
- The TUI is itself an app-server client. Codex was verified live on 0.169.0 (HOOKS-research.md › Codex --remote second client) running `codex app-server --listen unix://PATH` with `codex --remote unix://PATH`:
  - Handshake: a second client sends `initialize` and then `initialized`. `thread/loaded/list` returns the TUI's thread, and `thread/resume {threadId, excludeTurns:true}` subscribes to it.
  - Notifications: `thread/started`, `thread/status/changed`, `turn/started`, `turn/completed` (status `interrupted` on an interrupt), `item/*` and `serverRequest/resolved`.
  - Requests the server sends: `item/commandExecution/requestApproval` and `item/fileChange/requestApproval`, answered `{"decision":"accept"|"decline"}`, where the first answer wins; and `item/tool/requestUserInput`, with `{threadId, turnId, itemId, questions:[{id, header, question, isOther, isSecret, options:[{label, description}]}]}` answered `{"answers":{"<id>":{"answers":["…"]}}}`.
  - Request ids are per-connection integers, so key on threadId plus itemId.
  - `approval_policy` in config is rejected; use `-a`.
  - Transport: websocket framing on the unix socket. The JSON schemas were generated to the scratchpad `agents2/schema/` (with CODEX_HOME and HOME set to scratch).
- Rollout files are at `<home>/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<id>.jsonl`. A new conversation has a different date folder and file name, so the follow must find the file by id.

**Copilot:** unchanged. Its plugin hooks (sessionStart, userPromptSubmitted, agentStop conversation; preToolUse ask_user) are the model the other two now match.

### Design

1. **Claude conversation follow (D799).**
   - `parse_conversation` accepts Claude: it reads `session_id` and refuses a payload carrying `agent_id`.
   - `hooks_json` adds the conversation hook to SessionStart (matcher removed, so every source fires it), UserPromptSubmit and Stop.
   - The daemon's existing `follow_conversation` does the rest. A Claude file is `<conversation>.jsonl` in the same project folder.
2. **Claude questions (D796).**
   - `parse_question(Claude)` reads AskUserQuestion's `tool_input.questions[{question, header, options[{label, description}], multiSelect}]`.
   - `render_question(Claude)` returns allow plus updatedInput with `answers`.
   - "Answer in the terminal", dismissal, failure and timeout all mean no output.
   - `hooks_json` adds a PreToolUse `AskUserQuestion` matcher running `humata hook claude question`, with a long timeout.
3. **Codex wrapper (D795).**
   - A new crate `daemon/codex` (`humata2_codex`), shaped like `daemon/opencode`:
     - `ws`: a tokio-tungstenite client over `UnixStream`;
     - `rpc`: JSON-RPC frames, parsed tolerantly;
     - `event`: notifications and server requests mapped to `Turn`, `Thread`, `Ask` and `Resolved`;
     - `serve`: spawns the app-server, owns the 0700 folder and the 0600 socket, and waits for the socket;
     - `link`: the handshake, `thread/loaded/list` and `thread/resume`, then the event loop, reconnecting on loss;
     - `wrapper`: runs the TUI with `--remote`, passes the exit status through, and falls back to exec'ing plain `codex -c hooks=…` when the handshake fails.
   - The new CLI command `humata codex` reaches the daemon over the same RPCs as `humata opencode`:
     - turns go through `ReportTurn`;
     - the thread goes through the Hook stream's CONVERSATION event, carrying `{"session_id": threadId}`, so `parse_conversation(Codex)` takes it;
     - approvals go through the Hook stream's PERMISSION event, carrying the Codex hook payload shape `parse_permission` already reads, and the answer maps to `decision`;
     - questions go through the Hook stream's QUESTION event, with `parse_question` and `render_question` for Codex requestUserInput.
     - `serverRequest/resolved` closes that ask's Hook stream, which withdraws the card.
   - So no proto changes are needed.
   - In the codex kind (agents.rs), the program becomes `humata codex -- <args>` and resume is `humata codex -- resume {conversation}`. It is bound from the CONVERSATION event, and the D670 scan stays only for import and for older running sessions.
4. **Codex follow by id (D799).** `Agents::follow_conversation` and `bind` get a kind-aware file lookup. For Codex, they search `<home>/.codex/sessions/**/rollout-*-<id>.jsonl` (the validated id only, depth-bounded) instead of replacing the id in the old path. The other kinds keep the replace.
5. **Ready gate (D797).**
   - The daemon marks a session *heard* on its first CONVERSATION event since launch, from SessionStart, thread/started or sessionStart.
   - The `StartGate`/`WriteGate` for kinds with a signal holds Humata's own sends (start message, composer, Inbox, MCP) until the session is heard. It never sends blind, and the person's keys pass straight through.
   - 20 s after launch with nothing heard, the session needs you and the D781-form card "<Agent> is waiting in its terminal" appears, with its one action "Send it now". The card is withdrawn when the session is heard, and the queue then flushes in order.
   - Kinds with no signal keep `screen_question`.
6. **Doubles (D798).**
   - `screen_ask` raises nothing for a heard session, and is quiet again on re-link.
   - A Claude Notification of `permission_prompt` or `elicitation_dialog` with no held ask raises the waiting card.
   - Held asks are withdrawn at turn end, interrupt, SessionEnd, Stop or End.
   - A late Inbox answer does nothing.
7. **VERIFY by kind (D799).** The `flows/cases/results/copilot-*.md` cases get an agent-kind parameter. Main runs them with claude (Haiku 5.5 low) and codex (cheapest model), then does the live drive with screenshots.

### Owed: live checks, for when the owner can watch (main, 2026-11-18)

The owner's approval to copy agent logins into scratch homes ended at 08:20, so no live check runs now. Every part is built with its fallback wired and covered by tests on recorded fixtures (hook payloads and app-server messages). Each check's result then only flips a choice, made in one place, and nothing is rewritten.

- **O1, Claude AskUserQuestion answered by the hook.** Does PreToolUse `allow` plus `updatedInput.answers` keep the dialog closed and give the model the answers?
  - Switches `ClaudeAnswer::{Updated, DenyWithReason}`, a private const in `agent_hooks/src/question.rs`, defaulting to `Updated`.
  - Both renders are built and tested. If the check fails, flip it to `DenyWithReason` (Copilot's way).
- **O2, Codex's `thread/started` as its ready signal.** Does it come after the trust and first-run screens and before the first prompt, and again with a new id on `/new`?
  - Switches `ReadySignal::{Structured, Screen}` for the codex kind, set per kind in `agents.rs` and read by `ready.rs`. It defaults to `Structured`.
  - The screen check path stays wired and tested for kinds marked `Screen`. If the check fails, mark codex `Screen`. Following `/new` then rests on the link's later `thread/started` or the folder scan.
- **O3, Claude `/clear` carries the new conversation id.** Does SessionStart `source=clear` carry the new `session_id` and `transcript_path`?
  - Switches nothing if it passes. If it fails, the conversation hooks on UserPromptSubmit and Stop (A2) already name the new id at the next turn. The test covers both: a SessionStart that names it, and a first follow at a later hook. So a failure only drops `clear` from the SessionStart matcher.

- **O4, Codex under the wrapper.** Do `codex app-server --listen unix://S -c k=v` take the session's `-c` overrides (the Humata MCP server), and does `codex --remote unix://S resume <id>` resume the thread in the TUI?
  - Switches nothing if both pass. If `-c` is refused by `app-server`, drop `config_pairs` in `daemon/codex/src/wrapper.rs` (the TUI still gets them). If `resume` refuses `--remote`, the wrapper's resume falls back to plain `codex resume <id>` with the hooks.

### O1–O4 runbook (slow tier before 0.4.0; main runs it, 2026-11-19)

Every check runs in an isolated scratch home. Never touch `~/.claude`, `~/.codex` or `~/.copilot`. Tokens are set in the environment only and never written to a file.

**Setup.** Copy the logins into the scratch home only as the owner approved before (the same scratch homes as VERIFY), then run:
```sh
export S=$(mktemp -d)                         # scratch home, deleted at the end
export HOME=$S CLAUDE_CONFIG_DIR=$S/.claude CODEX_HOME=$S/.codex
mkdir -p "$CLAUDE_CONFIG_DIR" "$CODEX_HOME"   # Codex refuses a CODEX_HOME that does not exist
mkdir -p $S/work && cd $S/work && git init -q
```

**O1: Claude AskUserQuestion answered by the hook.**
1. Start a Humata 2 test daemon with a Claude session on `$S/work` (`humata session start --agent claude`), or run `claude` there with the hooks humatad writes.
2. Prompt: `Use AskUserQuestion to ask me which colour I like, red or blue, then write my answer to colour.txt`.
3. Answer "blue" on the Agent inbox card.

Pass: no dialog appears in Claude's terminal, and colour.txt says blue. Fail: flip `ClaudeAnswer` to `DenyWithReason` in `daemon/agent_hooks/src/question.rs`, then rerun `bazel test //apps/humata2/daemon/agent_hooks:questions_test`.

**O2: Codex `thread/started` as the ready signal.**
1. Run `humata codex` on a fresh `$S/work`, with no trust yet.
2. Watch the daemon log for the ready mark (`HUMATA2_LOG=debug`), and type `/new` once.

Pass: ready comes after the trust and first-run screens are answered, and again with a new thread id after `/new`. Fail: mark codex `ReadySignal::Screen` in `agents.rs`, then rerun `bazel test //apps/humata2/daemon/humatad:ready_gate_test`.

**O3: Claude `/clear` carries the new conversation id.**
1. In the O1 session, type `/clear`.
2. Check the session's conversation id in `humata session show <id> --json`, or look in the hook log.

Pass: the id and transcript path change at the SessionStart (source=clear). Fail: drop `clear` from the SessionStart matcher in `agent_hooks` config, then rerun `bazel test //apps/humata2/daemon/agent_hooks:config_test //apps/humata2/daemon/humatad:conversation_follows_test`.

**O4: Codex under the wrapper.**
- Part 1 needs no login:
  ```sh
  codex app-server --listen unix://$S/s.sock -c 'mcp_servers.humata={ command = "humata" }' & sleep 3; ls -l $S/s.sock; kill %1
  ```
  Pass: the socket exists and the run has no "unknown argument" error. Fail: drop `config_pairs` from the app-server command in `daemon/codex/src/wrapper.rs`, then rerun `bazel test //apps/humata2/daemon/codex:wrapper_test`.
- Part 2:
  1. Run `humata codex` and prompt once.
  2. Quit, then resume the session from Humata.

  Pass: the TUI reopens the same thread through `--remote unix://… resume <id>`. Fail: the wrapper's resume falls back to plain `codex resume <id>`.

**Cleanup.** Run `rm -rf "$S"` at the end. Kill only the processes this run started.

### Tasks (workers write code; only main runs bazel and git)

| # | Task | Files (claim) | Worker | Tests first |
|---|---|---|---|---|
| A1 | `parse_conversation` for Claude (`session_id`, refuses `agent_id`) and Codex (`session_id`); `Agent` match only | `daemon/agent_hooks/src/lib.rs` (fn `parse_conversation` only), `daemon/agent_hooks/testdata/claude/{session_start_clear,session_start_subagent}.json`, `testdata/codex/thread_started.json` | Haiku 5.5 high | `daemon/agent_hooks:conversations_test` |
| A2 | Claude conversation hooks in `hooks_json` (SessionStart for every source, UserPromptSubmit, Stop) | `daemon/humatad/src/session_service/turns.rs` (fn `hooks_json` only) | Haiku 5.5 high | `daemon/humatad:conversation_follows_test` (Claude case) |
| A3 | `parse_question` and `render_question` for Claude AskUserQuestion (single, multi, Other; both the `Updated` and `DenyWithReason` renders, O1) and Codex requestUserInput | `daemon/agent_hooks/src/question.rs`, `testdata/claude/ask_user_question*.json`, `testdata/codex/request_user_input.json` | Sonnet medium | `daemon/agent_hooks:questions_test` |
| A4 | The PreToolUse AskUserQuestion matcher with a long timeout; the question hook leaves no output on failure | `turns.rs` (`hooks_json`), `daemon/humatad/src/session_service/hooks.rs` (question arm) | Haiku 5.5 high | `daemon/humatad:screen_questions_test` gets a new `claude_question_*` case, or a new `daemon/humatad/tests/agent_questions.rs` |
| B1 | Codex follow and bind by id (bounded rollout search) | `daemon/sessions/src/agents.rs` (`follow_conversation`, `bind`, a new private `codex_rollout`) | Sonnet medium | `daemon/sessions:sessions_unit_test` (`a_codex_conversation_is_found_by_id_in_a_later_day_folder`, `a_codex_id_with_a_path_in_it_is_refused`) |
| C1 | New crate `daemon/codex`: ws, rpc, event (tolerant) | new `daemon/codex/**` | Sonnet medium | `daemon/codex:events_test` (recorded frames: turns, interrupted, thread, approval, question, resolved, unknown method ignored) |
| C2 | `daemon/codex` serve and link: spawn, perms, handshake, resume, reconnect; driven by a fake app-server replaying frames | `daemon/codex/src/{serve,link}.rs`, `daemon/codex/tests/{link,serve}.rs`, `daemon/codex/testdata/*.jsonl` | Sonnet medium | `daemon/codex:link_test`, `daemon/codex:serve_test` |
| C3 | `daemon/codex` wrapper: TUI `--remote`, exit status passed through, falls back to hooks when the handshake fails | `daemon/codex/src/wrapper.rs`, `tests/wrapper.rs` | Sonnet medium | `daemon/codex:wrapper_test` |
| C4 | `humata codex` CLI command (the opencode.rs pattern) | new `cli/src/commands/codex.rs`; **SEAM**: `cli/src/args.rs` and the commands registration, `cli/BUILD.bazel` dep | Sonnet medium | `cli:codex_test` (wrapper to a fake daemon: turn, conversation, permission, question) |
| C5 | Codex kind to the wrapper; resume through it; bind from CONVERSATION; the D202 row only for degraded or user-defined kinds | `daemon/sessions/src/agents.rs` (codex kind), `daemon/humatad/src/inbox/permission/codex_trust.rs` | Haiku 5.5 high | `daemon/sessions:sessions_unit_test` (kind args), `daemon/humatad:conversation_follows_test` (Codex `/new`) |
| E1 | Ready gate (per-kind `ReadySignal`, both paths tested, O2): heard state, held sends, 20 s waiting card, "Send it now", flush | new `daemon/humatad/src/ready.rs`, `daemon/humatad/src/input_gate.rs`, `session_service/hooks.rs` (mark heard); **SEAM**: card kind in `inbox.proto` and `clients/src/model.rs` if the D781 card can't carry the action | Sonnet medium | new `daemon/humatad:ready_gate_test` |
| E2 | Doubles: screen_ask quiet for heard sessions; Notification to the waiting card; withdrawals; late answer does nothing | `daemon/humatad/src/screen_ask.rs`, `turns.rs` (Notification hook), `session_service/hooks.rs` | Sonnet medium | `daemon/humatad:screen_questions_test` (`a_heard_session_raises_no_screen_card`), `ready_gate_test` |
| F1 | VERIFY cases by agent kind | `flows/cases/results/copilot-*.md` gain a kind parameter (renamed by main), `flows/cases/*.md` | manager | none: run by main |

**Seams for main:**
- `tokio-tungstenite` in `Cargo.toml`/`MODULE.bazel` (at least 3 days old), plus the `daemon/codex` members entry;
- `cli/src/args.rs` registration;
- an optional inbox card action in `inbox.proto`/`model.rs` (E1);
- `daemon/humatad/tests/common/mod.rs`, additive only: a Claude/Codex hook-payload helper and a fake Codex app-server.

### Claims

- `daemon/agent_hooks/src/{lib.rs (parse_conversation),question.rs}`, `daemon/agent_hooks/testdata/{claude,codex}/` (new files), `daemon/agent_hooks/tests/{conversations,questions}.rs`
- `daemon/sessions/src/agents.rs` (codex kind, follow/bind)
- `daemon/humatad/src/session_service/{turns.rs (hooks_json),hooks.rs}`, `daemon/humatad/src/{input_gate.rs,screen_ask.rs,ready.rs (new)}`, `daemon/humatad/src/inbox/permission/codex_trust.rs`
- `daemon/humatad/tests/{conversation_follows.rs,screen_questions.rs,ready_gate.rs (new),agent_questions.rs (new)}`
- new `daemon/codex/**`, new `cli/src/commands/codex.rs`
- `flows/cases/` (VERIFY by kind)

### First failing tests (written before any code; run by main)

- `//apps/humata2/daemon/agent_hooks:conversations_test`: a Claude SessionStart(clear) names its new conversation; a payload with `agent_id` names none; a Codex thread payload names its thread.
- `//apps/humata2/daemon/agent_hooks:questions_test`: Claude AskUserQuestion becomes a question with options and multi-select; an answer renders allow with `answers`; no answer renders nothing; Codex requestUserInput round-trips.
- `//apps/humata2/daemon/sessions:sessions_unit_test`: a Codex conversation is found by id in a later day folder; an unsafe id is refused.
- `//apps/humata2/daemon/humatad:conversation_follows_test`: Claude `/clear` and Codex `/new` are followed, and Stop then Resume take up the newest conversation (mirroring the Copilot test).
- `//apps/humata2/daemon/codex:events_test`: recorded app-server frames map to turns, thread, ask and resolved; an unknown method is ignored.
- `//apps/humata2/daemon/humatad:ready_gate_test`: a start message waits until the session is heard; after 20 s the waiting card is raised; it is withdrawn and the message is sent when the signal arrives; "Send it now" sends; typed keys always pass.

### Status (2026-11-18)

- 11:15: "go" received after 0.3.2 (df0f1684). Failing tests are written for the targets listed above:
  - `agent_hooks:conversations_test`
  - `agent_hooks:agent_questions_test`, which is new
  - `agent_hooks:questions_test`, whose `render_question` now takes the payload
  - `sessions:sessions_unit_test`, with three Codex follow-by-id tests
  - `humatad:conversation_follows_test`, with Claude's `/clear`, Stop and prompt, and Codex's `/new`
  - `codex:events_test`, in the new crate
- Fixtures were written from the documented and schema shapes. They are not live recordings, because those are owed.
- Main adds the new crate (`daemon/codex`, with its Cargo.toml and BUILD.bazel) to the members list and repins.
- Workers dispatched at 11:16:
  - A1+A2: conversation parsing and Claude's hooks file (Haiku);
  - A3+A4: questions (Sonnet);
  - B1: Codex follow by id (Sonnet);
  - C1: reading codex frames (Sonnet).
- 11:18: A1+A2, A3+A4, B1 and C1 report their code written. I fixed a clippy precedence issue in `codex_rollout` and updated the `hooks_json` unit test for the new SessionStart matcher and the PreToolUse entry. Batch 1 went to main, adding `humatad:humatad2_unit_test`.
- 11:25: wrote the failing `codex:link_test` against a fake app-server on a unix socket. It covers:
  - the handshake and the loaded thread;
  - `/new` naming another thread;
  - turns, including an interrupted one shown as stopped;
  - an approval and a question each going to the Inbox and back;
  - an ask answered in the TUI being withdrawn;
  - an undecided ask sending nothing;
  - reconnecting after a drop.

  C2, the link, is dispatched (Sonnet).
- G1 (main): agent_hooks `conversations_test`, `agent_questions_test` and `questions_test`, plus `codex:events_test`, PASS. `sessions_unit_test`, `conversation_follows_test` and `humatad2_unit_test` did not build, only because LIVEHOLDER's holder crate doesn't compile yet; main reruns them.
- 11:30: C2 wrote `codex/src/link.rs`, which is not compiled yet. It includes:
  - the handshake and resume of loaded threads, with naming deduped across reconnects;
  - asks raised as tasks keyed by request id, aborted on serverRequest/resolved;
  - approvals answered accept or decline from the hook-shaped answer, and an empty body sending nothing.

  I added `codex:codex_unit_test` for its decision-mapping unit test. Batch 2 to main: `codex:codex_unit_test`, `codex:link_test` and `codex:events_test`.
- 11:30: wrote the failing `humatad:ready_gate_test` (tests/ready_gate.rs, D797). It covers:
  - Claude's first message waiting for SessionStart, and Codex's for its thread, not for a clear screen;
  - keys typed into the terminal passing while the message is held;
  - the 20 s "waiting in its terminal" card, with Approve as "Send it now";
  - the signal withdrawing the card and sending the message once;
  - no card for a session heard early;
  - format none keeping the screen check.

  E1 is dispatched (Sonnet): a new `humatad/src/ready.rs` with a per-session heard signal set by conversation hooks, gating the first message by kind.
- Gate 1 rerun (main): every batch 1 target PASSES, including `sessions_unit_test`, `conversation_follows_test` and `humatad2_unit_test`. humatad2, cli and clients/live build. Gate 2, the codex targets, is running.
- 11:40: E1 is written, not compiled yet:
  - a StartSignal trait in sessions' first_message.rs;
  - humatad/src/start_ready.rs, with a heard registry set by any conversation hook and a 20 s card from Inbox::raise_waiting, whose Approve is "Send it now";
  - Claude and Codex gated; Copilot and none keep the screen check.

  Batch 3 to main: `ready_gate_test`, `sessions_unit_test`, `humatad2_unit_test` and `screen_questions_test`. The worker overwrote and then restored `humatad/src/ready.rs`; main confirmed it is identical to HEAD. Batch 3 runs as gate 3 after gate 2.
- 11:32 (main): gates 2 and 3 are green: `codex_unit_test`, `link_test`, `events_test`, `ready_gate_test`, `sessions_unit_test`, `humatad2_unit_test` and `screen_questions_test`. Main is committing batches 1–3.
- 11:33: E2 (D798), first part. `screen_ask::look` raises nothing for a session whose structured channel was heard since its launch (`start_ready::is_heard`), and `screen_ask::gone` forgets it (`start_ready::forget`). There are two new cases in `ready_gate_test`: heard means no screen card, not yet heard means the screen card still comes. Batch 4 to main: `ready_gate_test`, `screen_questions_test` and `humatad2_unit_test`.
  E2, still to do:
  - the fallback resuming when the Codex link drops (the link must report a drop to the daemon);
  - a structured card replacing a screen card for the same ask;
  - a Claude Notification `permission_prompt` or `elicitation_dialog` raising D797's waiting card;
  - withdrawal rules per channel, including "answered in the terminal" on a late Inbox answer.
- Not started, next after the owner's 12:00 stop:
  - C3–C5: serve and the wrapper, `humata codex`, and the kind switch;
  - the composer, Inbox and MCP sends going through E1's gate (only the start message is gated so far);
  - F1: VERIFY by kind;
  - O1–O3: still owed live.
- Evening (fresh manager): batch 5.
  - Failing tests first, by the manager: `codex:wrapper_test` (new tests/wrapper.rs: private 0700 socket folder, app-server gets only the `-c` pairs and no hooks, TUI on `--remote`, thread named, exit status passed through, app-server and folder cleaned up; app-server failing runs plain `codex -c hooks=…`; codex missing is NotInstalled), the codex kind argv in `sessions_unit_test`, a codex env-wrapper case in agent_switches' `cli_to_find` test, and `the_wrapped_codex_needs_no_trust_step` in codex_trust.
  - C3 (Sonnet): `daemon/codex/src/wrapper.rs`. C5 (Haiku): `agents.rs` codex kind is `/usr/bin/env HUMATA_SOCKET=… humata codex --hooks {codex_hooks} -- [resume {conversation}]`, `version_args` `codex --version`; `cli_to_find` names codex; `carrying` skips the value after `--hooks`. C4 (Sonnet): `cli/src/commands/codex.rs`, args/mod/error registration, `cli/tests/codex.rs`.
  - F1 (manager): `flows/cases/verify/20-claude-and-codex-in-place-of-copilot.md` and its README row.
  - Provisional picks for main to log (D-number): the wrapper gives the app-server only the session's `-c` pairs and never the hooks (hooks there would double every ask); the socket lives in a fresh 0700 `h2cx-<hex>` folder under the temp dir (or /tmp when that path is too long for a unix socket), removed on exit; the wrapped Codex kind shows no D202 trust row, and the fallback prints that Codex asks to trust the hooks once in `/hooks`.
  - Left for a later batch (low priority per owner): composer/Inbox/MCP sends waiting for the ready signal (today they go through the screen guard, which already refuses a trust screen); E2's rest (link drop reported to the daemon, Claude Notification to the waiting card, per-channel withdrawals).

- Batch 5 closed. C3 wrapper, C4 `humata codex` and C5 kind switch are done and their presubmits pass. Fix after main's report: `cli_to_find` had looked up `codex` by name for every kind called codex, so rigs that stand a script in for Codex (move_session_test) were refused. Now only the `env`-wrapped built-in is looked up by name; any other program is found by its own path (unit case added). Left: the composer, Inbox and MCP sends go through the ready gate; the rest of E2; O1–O4 owed live.
- Batch 6 (main's go after D806): the deferred items.
  - Send gate (D797): `start_ready` holds Humata's own sends behind a held first message. `expects` marks it pending and the new `StartSignal::delivered` (sessions first_message.rs) releases it. `terminal.rs` `type_in` waits at most 30 s in `behind_start`, then refuses with FailedPrecondition and types nothing. The composer, Inbox replies and MCP session_type all go through Type. Keys typed into the terminal pass. A session with nothing held (after a daemon restart, or no signal) passes at once. Test: `ready_gate_test` › `a_send_while_the_first_message_waits_goes_after_it_once_the_session_is_heard`.
  - Link drop (D798): `Humata::lost` (default no-op). `run_link` calls it when a link that had named its thread drops, and names the thread again on reconnect. `humata codex` sends Hook event `lost`, and the daemon then calls `start_ready::unheard`, so `screen_ask` reads the screen again. Once the session is heard again, a screen card raised in the gap stays until its screen stops asking. Tests: `codex:link_test` › `a_dropped_app_server_is_told_connected_again_and_its_thread_named_again`; `ready_gate_test` › `a_dropped_link_brings_back_the_screen_card`.
  - Notification and withdrawals (worker): Claude's `Notification` (permission_prompt|elicitation_dialog) runs `humata hook claude notification`. With no held ask, and asks not kept in the terminal, it raises a "waiting in its terminal" notice card. A turn report withdraws it. An Idle turn withdraws every held ask as Resolved, and a late answer is refused. Tests: 5 new cases in `hooks_test`, plus the `hooks_json` unit.
  - Provisional: the notice card is plain, not D781 on_screen (hooks.rs has no tab index), and only by its content is it Dismiss-only. The UI should show only Dismiss for key `notice-in-terminal`. The idle sweep also runs on SessionStart's idle report.

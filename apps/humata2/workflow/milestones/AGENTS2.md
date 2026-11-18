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

Manager: AGENTS2 manager (Opus 5.5). State: **plan accepted by main; waiting for "go"** (after 0.3.2 is committed). Live checks moved to Owed. Decisions: D795–D799 (Fable 5.2's rulings).

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

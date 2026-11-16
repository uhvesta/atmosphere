# ACP and live TUI agents: findings (2026-11-15)

Constraint from the owner: the agent always runs as its own interactive TUI in our PTY, on the user's
normal subscription. A headless SDK or ACP adapter process never replaces it. ACP is wanted only if
it can talk to that same live TUI.

## Short answer

No agent in the list lets an ACP client attach to its running interactive TUI. In every case ACP
is a separate, headless entry point: an adapter or a `--acp` mode, which the client spawns as its
own process. The protocol itself assumes this, because its only defined transport is "the client
launches the agent as a subprocess" over stdio. So ACP does not fit the owner's constraint for any
of the five agents.

Two agents do have a non-ACP way to drive the live TUI over a socket:

- **Codex**: the TUI can run as a client of `codex app-server` (`codex --remote`).
- **OpenCode**: the TUI is a client of its own HTTP server.

Claude Code has two channels into the live TUI: Channels (a research preview) and hooks. Copilot
and Gemini have only hooks.

## 1. ACP v1, what matters here

Source: https://agentclientprotocol.com/protocol/v1/overview

| Area | What the spec says | Source |
| --- | --- | --- |
| Transport | One defined transport, stdio. "The client launches the agent as a subprocess." Agents MUST support stdio. Streamable HTTP is still an RFD. Custom transports MAY be added. | [transports](https://agentclientprotocol.com/protocol/v1/transports.md) |
| Lifecycle | `initialize` (version and capabilities) → optional `authenticate` → `session/new` or `session/load` → `session/prompt` turns | [overview](https://agentclientprotocol.com/protocol/v1/overview), [initialization](https://agentclientprotocol.com/protocol/v1/initialization.md) |
| Session setup | `session/new {cwd, mcpServers}` returns `sessionId`. `session/load` (needs `loadSession`) replays history as `session/update`. `session/resume` (needs `sessionCapabilities.resume`) does not replay. `session/close` is optional. Concurrent clients on one session are not specified. | [session-setup](https://agentclientprotocol.com/protocol/v1/session-setup.md) |
| Prompt turn | `session/prompt {sessionId, prompt: ContentBlock[]}`. Its response carries `stopReason` (`end_turn`, `max_tokens`, `max_turn_requests`, `refusal`, `cancelled`). That response is a precise turn end. The spec only says another prompt may follow once a turn completes: no queueing, no steering. | [prompt-turn](https://agentclientprotocol.com/protocol/v1/prompt-turn.md) |
| Cancel | `session/cancel` is a notification. The client answers pending permission requests with `cancelled`. The agent MUST end the prompt with `cancelled`. | same |
| session/update | `agent_message_chunk`, `user_message_chunk`, `agent_thought_chunk`, `tool_call`, `tool_call_update`, `plan`, `available_commands_update`, `current_mode_update`, `config_option_update`, `session_info_update`, `usage_update`, plus capability-gated `notice` and compaction updates | same |
| Permission | `session/request_permission {sessionId, toolCall, options[]}`. Option kinds are `allow_once`, `allow_always`, `reject_once`, `reject_always`. Outcome is `selected {optionId}` or `cancelled`. | [tool-calls](https://agentclientprotocol.com/protocol/v1/tool-calls.md) |
| fs | Client-side `fs/read_text_file` and `fs/write_text_file`, behind capabilities | [overview](https://agentclientprotocol.com/protocol/v1/overview) |
| Terminals | Client-side `terminal/create`, `output`, `wait_for_exit`, `kill`, `release`. The client runs the commands, which is a different idea from our PTY. | [terminals](https://agentclientprotocol.com/protocol/v1/terminals.md) |
| Plans | `plan` update with `entries[{content, priority, status}]`, replaced in full each time | [agent-plan](https://agentclientprotocol.com/protocol/v1/agent-plan.md) |
| Modes | `availableModes` at setup, `session/set_mode`, `current_mode_update`. Deprecated in favour of Session Config Options. | [session-modes](https://agentclientprotocol.com/protocol/v1/session-modes.md) |
| Slash commands | `available_commands_update` lists them. They are invoked as plain `/cmd` text in `session/prompt`. | [slash-commands](https://agentclientprotocol.com/protocol/v1/slash-commands.md) |
| Extensibility | `_meta` on types, `_`-prefixed custom methods and notifications, custom capabilities under `_meta` with reverse-DNS names | [extensibility](https://agentclientprotocol.com/protocol/v1/extensibility.md) |

The parts of ACP that would help us are an exact turn end (the `stopReason`), structured permission
asks, a real cancel, and plans. All of them assume the ACP peer *is* the agent process.

## 2. Can an ACP client attach to a running interactive TUI?

### Claude Code: no

- Claude Code has no native ACP. The feature request is anthropics/claude-code#6686 ([issue](https://github.com/anthropics/claude-code/issues/6686)). A mirror shows it closed on 2026-02-09, with a follow-up still open ([mirror](https://claudeissues.com/issue/24411-feature-request-add-support-for-agent-client-protocol-acp)). Not verified on GitHub itself.
- The ACP adapter is `agentclientprotocol/claude-agent-acp` (formerly claude-code-acp). It "implements an ACP agent by using the official Claude Agent SDK" and bundles "the Claude CLI (`claude`) it runs". `CLAUDE_CODE_EXECUTABLE` swaps in your own CLI. Its v2 serves `session/list` and `session/resume` with replay over sessions it runs. It never mentions attaching to an interactive TUI. ([README](https://github.com/agentclientprotocol/claude-agent-acp))
- **Not ACP, but it reaches the live TUI: Channels** (research preview, v2.1.80+). An MCP server that declares `experimental['claude/channel']` pushes `notifications/claude/channel {content, meta}` into the *running interactive session*.
  - Events "queue into the session and are processed in order". If several arrive while Claude is busy, "they're delivered together on the next turn".
  - Two-way channels expose a reply tool.
  - With `claude/channel/permission`, Claude Code sends `notifications/claude/channel/permission_request {request_id, tool_name, description, input_preview}`. The channel answers with `notifications/claude/channel/permission {request_id, behavior: allow|deny}`. The terminal dialog stays live, and the first answer wins.
  - Channels are ignored in `-p` and the Agent SDK, so they work only in the interactive TUI.
  - They need a claude.ai login; Console or API-key auth isn't supported. They also need `--channels` from an allowlist, or `--dangerously-load-development-channels`, which shows a startup warning dialog. Team and Enterprise orgs must enable them.
  - Claude Code doesn't acknowledge delivery.
  - Sources: [channels reference](https://code.claude.com/docs/en/channels-reference.md), [channels](https://code.claude.com/docs/en/channels).
- **Not ACP: Remote Control.** `/remote-control` lets claude.ai or the mobile app drive the local interactive session. It is a closed relay, so we can't be the client. ([docs](https://code.claude.com/docs/en/remote-control.md))
- **Hooks** (33 events) run in the TUI:
  - `Stop` and `StopFailure` mark the turn end.
  - `Notification` has the matchers `permission_prompt` and `idle_prompt`.
  - `PermissionRequest` returns `decision.behavior` allow or deny.
  - `UserPromptSubmit` fires on each prompt.
  - `TaskCreated` and `TaskCompleted` track tasks.
  - `type: "http"` hooks POST to a URL.
  - The input has `transcript_path`, which is written asynchronously.
  - Source: [hooks](https://code.claude.com/docs/en/hooks.md).

### Codex CLI: no ACP attach, but the TUI can share its own app-server

- The ACP adapter is Zed's `codex-acp`: a separate process with no PTY mode ([Zed blog](https://zed.dev/blog/codex-is-live-in-zed), [repo](https://github.com/zed-industries/codex-acp)).
- **Codex's own protocol**, not ACP:
  - `codex app-server --listen ws://…` or `--listen unix://` serves JSON-RPC.
  - `codex --remote ws://127.0.0.1:4500` runs the *TUI* as a client of that server ([CLI features](https://developers.openai.com/codex/cli/features), [app-server](https://learn.chatgpt.com/docs/app-server)).
  - The app-server is per-connection subscribed: `thread/start` subscribes a connection, `thread/unsubscribe` removes one, and a thread unloads 60 s after "the last subscriber" leaves. So several connections can share one thread.
  - Methods: `thread/resume`, `thread/loaded/list`, `turn/start`, `turn/steer` (adds input to the active turn), `turn/interrupt`, and `turn/completed` with a status.
  - Approvals: `item/commandExecution/requestApproval` with `accept`, `acceptForSession`, `decline` or `cancel`, then `serverRequest/resolved`, plus `item/fileChange/requestApproval`.
  - WebSocket is "experimental and unsupported".
- openai/codex#11166 asked for `codex serve`/`attach`, citing chat bridges that "scrape terminal output". It was closed as completed on 2026-04-02 ("This feature is now implemented"; PR #10693) ([issue](https://github.com/openai/codex/issues/11166)).
- **Not verified:** that a second client sees and answers the TUI's approvals, which client wins, and whether `--remote unix://` works. Only third-party blogs claim these.
- **Hooks**: `UserPromptSubmit`, `PermissionRequest` (`decision.behavior` allow or deny, or no decision so the normal prompt runs), `Stop`, `Interrupt`, `SessionStart` and `SessionEnd`. Project hooks need trust via `/hooks`. ([hooks](https://learn.chatgpt.com/codex/hooks))

### Copilot CLI: no

- `copilot --acp [--stdio | --port N]` is a standalone server process. In stdio mode it exits when its parent closes stdin. ([ACP server reference](https://docs.github.com/en/copilot/reference/copilot-cli-reference/acp-server), [changelog 2026-01-28](https://github.blog/changelog/2026-01-28-acp-support-in-copilot-cli-is-now-in-public-preview/))
- A user asked on 2026-03-28 whether `copilot --acp --port 3000` could start the TUI *and* ACP "so I can attach my own daemon to an interactive session". No staff answer. ([discussion #185860](https://github.com/orgs/community/discussions/185860))
- One issue reports ACP mode auto-approving without sending `session/request_permission` ([copilot-cli#845](https://github.com/github/copilot-cli/issues/845)).
- **Hooks** are now documented: `sessionStart`, `userPromptSubmitted`, `preToolUse` (`permissionDecision` allow, deny or ask), `permissionRequest` (`behavior` allow or deny), `agentStop` (main agent finished a turn; `block` forces another), `notification`, `errorOccurred`, `sessionEnd`, and more. They live in `.github/hooks/*.json` or `~/.copilot/hooks/*.json`. ([hooks reference](https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-hooks-reference)) This answers D169's "Copilot's hook format isn't verified".

### Gemini CLI: no

- `gemini --acp` (formerly `--experimental-acp`) is "designed for programmatic control" over stdio. It is a separate mode with `newSession`, `loadSession` and `setSessionMode`. ([ACP mode](https://geminicli.com/docs/cli/acp-mode/))
- The same page says Gemini CLI was replaced by Antigravity CLI for unpaid and Google One users on 2026-06-18.
- **Hooks**: `AfterAgent` fires once per turn. `Notification` with `ToolPermission` is observe-only and can't approve or deny. Others are `BeforeAgent`, `BeforeTool`/`AfterTool` and `SessionStart`/`SessionEnd`. ([hooks reference](https://geminicli.com/docs/hooks/reference.md))

### OpenCode: no ACP attach, but its TUI is already a client of a local HTTP server

- `opencode acp` is spawned by the editor over stdio ([ACP docs](https://opencode.ai/docs/acp/)).
- PR #7340 would have added `acp --attach <url>` and `--session`. It was closed unmerged on 2026-09-01: "with v2 it would have to be completely reimplemented". ([PR](https://github.com/anomalyco/opencode/pull/7340))
- **OpenCode's own API**, not ACP: "When you run `opencode` it starts a TUI and a server". `--port` and `--hostname` pin it, and `OPENCODE_SERVER_PASSWORD` adds basic auth. ([server docs](https://opencode.ai/docs/server/)) Endpoints:
  - `POST /session/:id/prompt_async`
  - `POST /session/:id/abort`
  - `GET /event` (SSE)
  - `POST /session/:id/permissions/:permissionID {response, remember}`
  - `POST /tui/append-prompt` and `/tui/submit-prompt`
- `opencode attach <url>` and `run --attach` show that several clients sharing one server is the intended model.
- **Not verified:** the exact SSE event names for idle and permission (the docs point at `/doc`).

## 3. Billing: adapter vs TUI

| Agent | ACP adapter runs on | Billing |
| --- | --- | --- |
| Claude Code | `claude-agent-acp` runs on the **Claude Agent SDK** ([README](https://github.com/agentclientprotocol/claude-agent-acp)) | See below. This is the owner's worry, and it is a policy risk rather than today's billing. |
| Codex | `codex-acp` drives Codex with its own login | Zed: "login with ChatGPT … allows you to use your existing, paid ChatGPT subscription" ([Zed external agents](https://zed.dev/docs/ai/external-agents)). The app-server supports `chatgpt` and `apiKey` logins ([app-server](https://learn.chatgpt.com/docs/app-server)). Same pool as the TUI when signed in with ChatGPT. |
| Copilot | same `copilot` binary in `--acp` mode | The docs don't say. Inference: the same GitHub Copilot entitlement and premium requests as the TUI, or BYOK. |
| Gemini | same `gemini` binary in `--acp` mode | The docs don't say. Inference: the same auth as the TUI. |
| OpenCode | same `opencode` binary | Whatever provider key or login OpenCode is configured with. ACP vs TUI doesn't change it. |

Claude Code billing in detail:

- **Today:** the Anthropic help center (2026-10-07 update) says that when you're signed in with your Claude plan, the Agent SDK and `claude -p` draw from your subscription limits. The Console-org monthly API credits apply to the SDK and `-p` only when they use an API key from the linked Console org. ([help center](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan))
- **The paused split:** the 2026-06-15 split, which would have moved ACP, `-p` and SDK use to a separate monthly Agent SDK credit (\$20 Pro, \$100 Max 5x, \$200 Max 20x), was paused. Anthropic says it will give advance notice before any future change. ([Zed blog](https://zed.dev/blog/anthropic-subscription-changes))
- **Why the TUI is safer:** Zed's advice for keeping subscription limits was to run the official `claude` CLI in a terminal. Channels also require a claude.ai login and are interactive-only.

## 4. How Humata 2 drives agents today and what would improve

Today (SPEC SE-21–SE-24, PER-1; DECISIONS D7, D14, D126, D169, D170):

- **Process and input:** a holder owns each session's PTY, and the agent's own TUI runs in it. Prompts and queued messages are typed as serialized bracketed paste (SE-23).
- **Turn end:** Claude uses `--settings <state>/claude-hooks.json`. All its hooks are async and run `humata turn <state>`, which reaches `SessionService.ReportTurn`:
  - SessionStart → idle
  - UserPromptSubmit and PostToolBatch → working
  - Stop and StopFailure → idle
  - PermissionRequest and Elicitation → needs input
- **Fallback:** Codex, Copilot (deferred as unverified) and OpenCode use a 3 s output-quiet timer that shows "Probably idle". D169 notes that Stop doesn't fire on a user interrupt.
- **Mirror, search and fork:** these read the agent's conversation JSONL (`conversation_file`).
- **Permission asks:** they only flip status to "needs input". They aren't routed to the Inbox, and the user answers in the terminal.

Per interaction:

| Interaction | Claude Code | Codex | Copilot | Gemini | OpenCode |
| --- | --- | --- | --- | --- | --- |
| Prompt delivery | PTY paste stays. Channels could carry agent-to-agent messages and comments, but they arrive as a `<channel>` event, not a user turn: slash commands and images aren't typed. | `turn/start` via app-server if the TUI runs with `--remote`. Otherwise PTY. | PTY | PTY | `prompt_async`, or `/tui/append-prompt` + `submit-prompt` |
| Queueing | Channels queue natively and batch into the next turn. Otherwise our SE-23 buffer. | `turn/steer` adds to the active turn; `turn/start` after `turn/completed` | SE-23 buffer + `agentStop` | SE-23 buffer + `AfterAgent` | SE-23 buffer + SSE idle |
| Turn end | Hooks already exact (D169). Add `Notification idle_prompt` as a backstop. | `turn/completed` (exact, includes interrupted), or `Stop` + `Interrupt` hooks right now | `agentStop` hook, which replaces the fallback | `AfterAgent` hook | SSE event (name not verified) |
| Permission asks into Inbox | A synchronous `PermissionRequest` hook (an http hook to the daemon) can return allow or deny from the Inbox. Channel permission relay does the same with the terminal dialog kept live and first answer winning, which is the better fit. | `requestApproval` via app-server, or a synchronous `PermissionRequest` hook | `permissionRequest` / `preToolUse` hook allow or deny | No: Notification is observe-only, so it stays "needs input" | `POST /session/:id/permissions/:id` |
| Cancel | PTY Esc stays. There is no API. | `turn/interrupt` | PTY Esc | PTY Esc | `POST /session/:id/abort` |
| Plans | `TaskCreated` and `TaskCompleted` hooks, or the transcript JSONL | app-server item events (not checked in detail) | none found | none found | SSE (not checked) |

ACP itself would give none of this for a live TUI. The agents' own hooks and IPC already cover what
ACP would bring:

- turn end everywhere;
- permission decisions for Claude, Codex and Copilot;
- prompt, cancel and permissions for OpenCode, and for Codex in `--remote` mode.

## Recommendation

1. Don't adopt ACP for live sessions. Keep PTY paste as the universal path.
2. **Hooks first** (cheap, works in the TUI, no billing risk):
   - Move Codex and Copilot from the output fallback to hooks: Codex `Stop`, `Interrupt` and `UserPromptSubmit`; Copilot `agentStop`, `userPromptSubmitted` and `permissionRequest`.
   - Add Gemini `AfterAgent` if Gemini joins the table.
   - Route permission asks into the Inbox with a *synchronous* `PermissionRequest` hook (Claude, Codex, Copilot) that the daemon answers. The terminal prompt stays the fallback when the daemon is down or the hook times out. Verify each agent's hook timeout first.
3. **OpenCode:**
   - Launch with `--hostname 127.0.0.1 --port <n>` and `OPENCODE_SERVER_PASSWORD`.
   - Use its HTTP API for prompt, abort and permissions, and SSE for idle.
   - The TUI stays in the PTY.
4. **Codex, spike:**
   - Run `codex app-server --listen unix://…` under the holder and the TUI with `codex --remote`.
   - Verify that the daemon, as a second subscriber, gets `turn/completed` and can answer approvals and `turn/interrupt`.
   - The transport is marked experimental, so keep PTY as the fallback.
5. **Claude Channels, spike:**
   - Worth a spike for Inbox permission relay and agent-to-agent messages.
   - Blockers: research preview, the custom-channel allowlist (dev flag + startup dialog), and the org policy on Team/Enterprise.
   - Don't make it load-bearing yet.

## Verified from primary sources (HOOKS manager, 2026-11-16)

Checked against the agents' own docs and source, plus `--help` of the installed CLIs
(claude 2.1.325, codex-cli 0.169.0, copilot 1.0.112). Items still marked UNVERIFIED need a live capture.

| | Claude Code | Codex CLI | Copilot CLI |
| --- | --- | --- | --- |
| Per-session config | `--settings <file>` (layers over the user's files; already used, D169) | `-c hooks.<Event>=[…]` lands in the `<session-flags>` config layer (`codex-rs/hooks/src/engine/discovery.rs`); no docs example: UNVERIFIED live. `CODEX_HOME` moves auth too. | `--plugin-dir <dir>` loads a session-only plugin whose `hooks.json` holds hooks (`copilot --help`; [plugin reference](https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-plugin-reference)); that its hooks run: UNVERIFIED live. `COPILOT_HOME` moves auth and state, so not used. |
| Trust | none for `--settings` | "Non-managed hooks must be reviewed and trusted before they run … recorded against the hook's current hash" ([hooks](https://learn.chatgpt.com/docs/hooks)); `--dangerously-bypass-hook-trust` skips that for the whole invocation, the user's own hooks included | plugin hook trust not documented |
| Turn end | `Stop`, `StopFailure` | `Stop` (must print JSON on exit 0: `{}`), `Interrupt` (exists; 1 s default, at most 3 s) | `agentStop` (`stopReason` always `end_turn`); no interrupt event |
| Prompt sent | `UserPromptSubmit` | `UserPromptSubmit` | `userPromptSubmitted` |
| Permission hook | `PermissionRequest`; stdin `tool_name`, `tool_input`, optional `permission_suggestions` | `PermissionRequest`; stdin `turn_id`, `tool_name` (`Bash`, `apply_patch`, `mcp__…`), `tool_input.command`, optional `tool_input.description` | `permissionRequest`; stdin fields not documented beyond `toolName`/`toolInput`: UNVERIFIED |
| Decision on stdout | `{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"\|"deny","message"?}}}` | same wrapper; `updatedInput`, `updatedPermissions`, `interrupt` "fail closed today": never send them | flat `{"behavior":"allow"\|"deny","message"?}`; an allow doesn't pre-approve a sandbox bypass |
| No decision | exit 0 with no output, or exit 2, timeout, other failure: the normal dialog | exit 0 with empty stdout, a timeout or a failure: the normal prompt (from `permission_request.rs`, `command_runner.rs`) | empty output or `{}`: the normal flow; timeout and failures fail open |
| Deny by exit code | exit 2 is no decision for PermissionRequest | exit 2 with stderr text is a deny (source) | exit 2 is a deny |
| Hook timeout | `timeout` in seconds, default 600 for command hooks (30 on UserPromptSubmit) | `timeout` seconds, default 600 (Interrupt and SessionEnd 1 s, max 3) | `timeoutSec` (alias `timeout`), default **30** |

So no hook may exit 2 (Codex and Copilot read it as a deny); "no answer" is always exit 0 with empty
stdout, except Codex Stop, which prints `{}`.

Timeouts Humata writes (D192), each set explicitly so no agent default applies. `agent_hooks:config_test` checks each one and checks that the hook exits 0 when `humata` is missing:

| Hook | Claude Code | Codex | Copilot |
|---|---|---|---|
| Blocking permission hook | 3600 s | 3600 s | 3600 s (`timeoutSec`) |
| Turn hooks (prompt, tool, stop) | 10 s, async | 10 s | 10 s |
| Interrupt | n/a | 3 s (Codex caps it at 3) | n/a |

The daemon ends a hold with no decision at 3590 s, and `humata hook` gives up at 3595 s. That way the agent never hits its own timeout, which it would count as a hook failure.

OpenCode (source `anomalyco/opencode` `dev`; the docs lag it; SUPERSEDED by the v2.3.0 section at the end, which was checked against the real binary):
- SSE `GET /event`: first `server.connected`, then bus events. `session.status {sessionID, status:{type:"idle"|"busy"|"retry",…}}`; `session.idle {sessionID}` is deprecated.
- `permission.asked {id (per_…), sessionID, permission, patterns[], metadata, always[], tool?}`, `permission.replied {sessionID, requestID, reply}`.
- Reply values `once | always | reject` (`packages/schema/src/v1/permission.ts`). Two endpoints exist: docs' `POST /session/:id/permissions/:permissionID {response, remember?}` and source's `POST /api/session/{id}/permission/{requestID}/reply {decision, message?}`. Which one the shipped version serves, and whether the SSE envelope is `{type, properties}` or `{type, data}`: UNVERIFIED, capture live against the pinned version.
- Server: `--port` (4096), `--hostname` (127.0.0.1); `OPENCODE_SERVER_PASSWORD` turns on basic auth, user `opencode` unless `OPENCODE_SERVER_USERNAME`.

### OpenCode, verified against v2.3.0 (2026-11-16)

Run against the real `opencode-darwin-arm64` v2.3.0 (published 2026-11-13, sha256 `7907df2b12720a840fbb49c5b978b9b82925b6bf75479a5fd216afa8932b6702`), `opencode serve` in an isolated HOME/XDG, with the free built-in `big-pickle` model (public key, no login). Fixtures and the endpoint table are in `daemon/agent_hooks/testdata/opencode/` (`api.md`). The v2 line replaced the v1 API: the `/session/:id/prompt_async`, `/abort`, `/event` and `/permissions/:id` routes in the bullets above do not exist in v2.3.0.

- **Reply endpoint (captured):** only `POST /api/session/{id}/permission/{requestID}/reply {decision: once|always|reject, message?}`, 204 on success; `{response, remember}` is not served. Resolves the "which endpoint" item.
- **SSE envelope (captured):** `{id, created, type, location?, data, durable?}`, with `data`, not `properties`. Route `GET /api/event`. First frame `{"id":"evt_...","type":"server.connected","data":{}}`, then `: heartbeat` comments. The stream is server-wide, no replay. Resolves the envelope item.
- **Idle and busy (captured, differs from the source note above):** no `session.status` and no `session.idle` frame was emitted. Use `session.execution.started` (busy), then `session.execution.succeeded | interrupted {reason} | failed {error}`. `GET /api/session/active` returns `{data:{<id>:{type:"running"}}}` while busy and `{data:{}}` after. `session.status` and `session.idle` appear in the `types` filter enum, and `session.idle` has a schema, but neither fired (from schema, not captured).
- **permission.asked (captured):** `data {id: per_..., sessionID, action: "shell", resources: ["echo hi"], save: ["echo *"], source: {type: "tool", messageID, id}}`. The field names are `action`/`resources`/`save`, not `permission`/`patterns`/`always`. `permission.replied` (captured): `data {sessionID, requestID, reply}`.
- **Pending list (captured):** `GET /api/permission/request` (all sessions of the location) and `GET /api/session/{id}/permission`; both returned the pending ask and an empty list after the reply.
- **Rule gotcha (captured):** the bash tool asks under action `shell`. A session rule `{action:"bash",...,effect:"ask"}` and a project `opencode.json` `permission.bash: "ask"` did not make it ask; a session rule `{action:"*",resource:"*",effect:"ask"}` did. Default agents allow everything except `read *.env*`.
- **Prompt (captured):** `POST /api/session/{id}/prompt {text}` returns at once with the queued user message (`delivery: "steer"`); there is no separate `prompt_async`.
- **Abort (captured):** `POST /api/session/{id}/interrupt` returns `{interrupted:true}`; events were `session.step.failed {error:{type:"aborted"}}` then `session.execution.interrupted {reason:"user"}`. A second call returns `{interrupted:false}`.
- **Auth (captured):** basic auth, user `opencode`; no credentials gives 401 `UnauthorizedError`, with credentials 200. `/health` and the web UI (every non-`/api` path, including `/doc`) are open. OpenAPI is `GET /openapi.json` (authenticated), 139 paths, not `/doc`.
- **Flags (captured):** `opencode serve` takes `--hostname`, `--port`, `--cors`, `--service`, `--stdio`. The root TUI command takes only `--standalone`, `--server <url>`, `--auto`, `-c`, `-s`, `--prompt`: **no `--port` or `--hostname`**. The old plan item "launch the TUI with `--hostname 127.0.0.1 --port <n>`" no longer works.
- **TUI server (captured):** `opencode --standalone` under `script` opened its own listener on a random 127.0.0.1 port, but it rejected `opencode:<our password>` with 401, so the daemon cannot reach it. By default the TUI uses a background service (`opencode service start|stop|status`). So the driver should own the server (`opencode serve`) and start the TUI with `--server <url>`. That the TUI under `--server` shows sessions created over the API was NOT verified (it started, showed a UI and logged no requests within 10 s; it was not given a password for the server beyond the env var).
- **Still unverified:** `always` and `reject` replies (only `once` was run); `message` on a reject; `/tui/*` control routes (not in the v2.3.0 OpenAPI paths; the TUI has no `/api/tui` route); permission asks for non-shell tools; v1.19.x (a v1 line still released, newest seen v1.19.5) which may still serve the old routes.
- **Driver plan check (captured, 2026-11-16):**
  - **Port discovery:** `opencode serve --hostname 127.0.0.1 --port 0` is allowed and prints `server listening on http://127.0.0.1:<port>` on stdout. Parse that line.
  - **TUI auth:** `opencode --server <url> -s <id>` reads the password from env `OPENCODE_PASSWORD` or `OPENCODE_SERVER_PASSWORD`. URL userinfo (`http://opencode:<pw>@host:port`) also reached the UI. No password exits with "requires a password; set OPENCODE_PASSWORD"; a wrong one with "rejected the password". There is no password flag. A non-default username env was not tested.
  - **Second client works:** with the TUI attached to an API-created session, an API prompt ran and showed on the TUI screen. A `*` ask rule raised `permission.asked`, listed at `GET /api/session/{id}/permission` and shown in the TUI. An API reply (`once`, 204) cleared it, emitted `permission.replied` and the run ended with `session.execution.succeeded`.
  - **PTY fallback works:** text plus `\r` written to the TUI's pty submitted a prompt to the same session (new execution started and succeeded).
  - **Resume:** `-s` accepts an API-made id. Sessions and messages survived a server restart with the same XDG_DATA_HOME. An unknown `-s` id opens a blank new session; what happens after a first prompt there was not checked.
  - **Not verified:** the server-side request log of the TUI (serve logs nothing), and `OPENCODE_SERVER_USERNAME` for the TUI.
- **Round 3 (captured, 2026-11-16; details in `daemon/opencode/testdata/api.md`):**
  - **Shared store:** `serve --service` and two plain `serve --port 0` share one SQLite db (`opencode.db` with wal/shm) and log; sessions from each show in all, no lock errors. A plain `serve` writes no registration. `serve --service` writes `config/opencode/service.json` (random password) and `state/opencode/service.json` (id, version, url, pid, password) and ignores `OPENCODE_SERVER_PASSWORD`. A default TUI attaches only to the `--service` server, never a plain one, so a plain `serve` is safe from a stray default TUI.
  - **Replies:** `reject` with `message` gives 204; `permission.replied` carries only `reply:"reject"`; the text reaches the model through `session.tool.failed {error:{type:"permission.rejected", message}}`, and the model quoted it. The run ends `succeeded`. `always` saves a rule (`GET /api/permission/saved`, e.g. `shell` `echo *`) that silences later matching asks in every session of the store. Replying to an id the TUI already answered gives 404 `PermissionNotFoundError`.
  - **Env leak:** tools see `OPENCODE_SERVER_PASSWORD` (names captured, values redacted). Treat the password as visible to the model.
  - **Event noise:** title generation emits only `session.renamed` on the same session; no separate session and no extra `session.execution.*`. Subagent/task tool: not tried.
  - **Custom id:** `ses_`-prefixed ids are accepted and kept (a repeat POST returns the same session); other ids give 400.
  - **RSS:** plain `serve` about 146 MB idle (215 MB just after a prompt), `--service` 247 MB, attached TUI 178 MB.


### Codex `--remote` second client, verified live against codex-cli 0.169.0 (2026-11-16)

Setup: a spike in its own CODEX_HOME with a fake local Responses provider, so no real model and no network, and ~/.codex untouched. Scripts are in the session scratchpad `codex-spike/`.
- `codex app-server --listen unix://PATH`, then the TUI with `codex --remote unix://PATH -a on-request`. The unix transport speaks websocket framing (HTTP/1.1 101 on connect), so the daemon would have to be a websocket client. There is no stdio attach.
- A second client sends `initialize`/`initialized`. `thread/loaded/list` returns the TUI's thread even before the first prompt (`thread/list` is empty until then). `thread/resume {threadId, excludeTurns:true}` subscribes it. It then gets `thread/status/changed` (`activeFlags:["waitingOnApproval"]`), `turn/started`, `item/*`, `turn/completed` and `serverRequest/resolved`. A late subscriber is sent the pending approval request again.
- Answers: the second client's `{"decision":"accept"|"decline"}` closes the TUI's popup. The first answer wins. Later answers, from either side, are dropped silently (a late `y` lands in the composer as text). Every client gets `serverRequest/resolved`, so treat that as the truth. If an answer arrives within about 0.1 s of the request, the TUI may not have drawn its popup yet.
- `turn/interrupt {threadId, turnId}` works: both clients get `turn/completed` with status `interrupted`, and the pending approval is resolved.
- Risks: app-server and `--remote` are `[experimental]` and the schema is churning (deprecation notices already). Approval request ids are small per-connection integers, so key by threadId+itemId. The socket path resolves into a shared /private/tmp/codex-daemon-<uid>/ folder, so bind a path we own. `approval_policy` in config is rejected; use `-a`.
- Consequence: this is a possible later Codex driver that keeps the TUI on the subscription, shaped like OpenCode's (D201): a `humata codex` wrapper runs app-server plus `--remote`, and its link reports turns and passes approvals to the Inbox. It is not built. The `-c hooks` path (D191-D193) stays the shipped one.

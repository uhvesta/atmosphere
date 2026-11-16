# Provider event adapters

`terminal_provider` normalizes bounded, explicitly supplied hook JSON without
reading provider files or terminal output. It performs no I/O, model calls,
configuration edits, process inspection or notification side effects.

The caller registers a trusted context containing its provider identity,
versioned schema key, optional configured hook name and explicit recovery
state paths. The registry accepts at most 256 KiB before parsing. It skips
unknown JSON fields rather than retaining transcripts, prompts, model text,
argv or environment. A notification's text is accepted only from a supported
notification event and remains ephemeral. State paths come only from the
trusted binding and are absent from normalized UI events.

Builtins are explicit schema contracts:

| Schema | Supported input |
| --- | --- |
| `claude-hooks-v1` | Claude Code `hook_event_name`, `session_id`, `cwd`; session/turn lifecycle and Notification |
| `codex-hooks-v1` | Codex hook lifecycle with `hook_event_name`, `session_id`, `cwd`, including Interrupt |
| `copilot-camel-hooks-v1` | Copilot `sessionId`, `cwd`; configured camelCase hook selector |
| `copilot-vscode-hooks-v1` | Copilot PascalCase `hook_event_name`, snake_case session metadata |
| `opencode-events-v2` | OpenCode v2 `type` and `data.sessionID`; session creation, execution/idle and deletion |

The schema version names describe this adapter's contracts, independently
of human provider labels or installed CLI release numbers. References were
checked on November 15, 2026: [Claude hooks](https://code.claude.com/docs/en/hooks),
[Codex hooks](https://learn.chatgpt.com/docs/hooks),
[Copilot hooks](https://docs.github.com/en/copilot/reference/hooks-reference),
and [OpenCode's current OpenAPI](https://opencode.ai/docs/openapi.json).
No undocumented legacy OpenCode `properties.info` shape is inferred.
OpenCode location references are not assumed to be filesystem paths, so
that adapter leaves cwd absent. Legacy Codex notify payloads and their
assistant messages are not interpreted as the documented hooks interface.

Unknown events return `None`; malformed supported events return typed errors.
Documented hook session IDs and cwd are required. Session IDs are bounded to
512 bytes, provider labels to 64, cwd/state paths to 4096, state paths to 16,
notification kinds to 128, titles to 512 and bodies to 8192. Control characters
are refused except newline/tab in notification bodies. Builtins ignore child
agent events; explicit child bindings can register a distinct adapter.
Registering an existing schema is an error, preserving its previous adapter.

The host binds each event to a terminal separately. Only `SessionStarted`
is eligible to replace persistent provider recovery identity. Other events
with a different session ID must be dropped or rejected as stale. `Stop`
means the provider finished a turn; neither it nor `SessionEnded` means the
PTY exited. `validate_event(context, event)` applies the same bounds and
trusted identity checks to native caller-supplied normalized events.

```sh
bazel test //apps/humata2/terminal/provider:provider_test --test_output=all
```

## Exact conversation recovery

`ResumeRegistry` plans argv without reading state, starting a process, changing
permissions or choosing an executable/endpoint. `ResumePolicy` defaults to
`Disabled`; `Enabled { adapter, provider, args }` is trusted launcher policy.
`ResumeIdentity` contains the provider, exact structured session ID and working
directory saved by the host. Policy/provider identity must agree. The host must
independently establish process death: an unavailable SSH connection is not
proof of death and must not trigger a second agent.

Recovery arguments are configured separately from initial launch arguments.
They are never reconstructed from a saved prompt. Builtins support a narrow
set of documented options for models, configuration and hooks; positional
prompts, conflicting selectors and unknown options return explicit errors.
Registered adapters cannot silently replace one another. Custom trusted
adapters can support other CLI versions or opaque ID formats; registry bounds
apply to their output too (128 arguments, 16 KiB each, 64 KiB combined).

| Adapter | Complete argv, excluding the trusted executable |
| --- | --- |
| `claude-cli-resume-v1` | configured resume options, `--resume`, full UUID |
| `codex-cli-resume-v1` | `resume`, configured resume options, full UUID |
| `copilot-cli-resume-v1` | configured resume options, `--resume=full-UUID` |

The UUID restriction excludes names, prefixes, transcript paths and selectors
such as latest/continue. It is the provider conversation ID, independent of the
terminal's durable UUID. No builtin adds a prompt, forks a conversation, changes
permission policy or passes an implicit submit. Copilot's `--session-id` is not
used because it can create a new conversation when its ID is absent.

CLI inspection on November 16, 2026 used isolated empty configuration folders:
Codex 0.169.0, Claude Code 2.1.325 and Copilot 1.0.112-0 support the recorded
forms. Primary references: [Claude CLI](https://code.claude.com/docs/en/cli-reference),
[Codex source](https://github.com/openai/codex), and
[Copilot CLI](https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-command-reference).
OpenCode was not installed. Its current documentation specifies
`opencode mini --session ID` for its minimal interactive UI and
`opencode run --session ID PROMPT` for automation; no full-screen startup flag
is guessed. A verified custom adapter can select that explicitly documented
mode. See [OpenCode commands](https://opencode.ai/docs/cli/commands/).

The manual smoke target executes the actual installed Codex CLI with its own
empty home/configuration and a deterministic local Responses API server:

```sh
bazel test //apps/humata2/terminal/provider:codex_resume_test \
  --test_env=TERMINAL_CODEX_BIN=/opt/homebrew/bin/codex --test_output=all
```

It creates one saved turn, reads `thread.started` JSON, resumes that exact ID,
terminates only its own CLI with `SIGKILL` while the local server holds a
request, then resumes again and checks that the next request carries the prior user and assistant
context. It uses no personal sessions, authentication or external model calls.
This is real CLI persistence/recovery against a synthetic model response, not
an upstream model or interactive terminal rendering certification. Its evidence
is separate from Host recovery integration and its synthetic agent fixtures.

`tests/codex_resume.py --prepare-host` is the same owned local-server fixture
for a separate Host integration test. It first saves a completed Codex turn,
then emits one JSON line with the structured session ID, real binary, isolated
cwd, trusted resume-only arguments, executable wrapper, PID receipt path and
expected historical marker. The wrapper enforces its private environment,
requires the exact seeded resume ID, records its PID before executing the real
CLI, and uses normal terminal colors. It does not trust project hooks or
change the planner's flags. An interactive trust prompt remains an explicit
integration condition rather than being bypassed.

The fixture holds its server and state until a control line or stdin EOF.
The driver must explicitly stop its owned terminal children/holders before
closing that input: Host shutdown intentionally preserves processes. PID
receipts show the requested ID and owned process, not a claim about conversation
contents. A resumed TUI must independently display the saved marker. The helper
is exported only to the Host test package; it has no network or personal-state
configuration surface.

### Claude Code recovery smoke

```sh
bazel test //apps/humata2/terminal/provider:claude_resume_test \
  --test_env=TERMINAL_CLAUDE_BIN=/absolute/path/to/claude --test_output=all
```

This manual target runs the installed Claude Code CLI with an empty private
`HOME` and `CLAUDE_CONFIG_DIR`. The documented `ANTHROPIC_BASE_URL` points only
to an owned loopback Messages API fixture; its API-key header is a fixed fake
fixture value, never a real credential. Nonessential traffic and updates are
disabled. No user project, personal settings or authentication state is loaded.

The seed supplies one new UUID and confirms it in structured `session_id`
events. The test completes a synthetic turn, resumes that exact ID, kills only
its owned CLI while a local request is pending, and resumes the same ID again.
The next request must contain the saved user and assistant context. Installed
Claude Code 2.1.325 passed this sequence; `claude-resume-proof.json` records the
CLI version and scope. It proves real CLI persistence against synthetic model
responses, independently of Host automatic-recovery integration or real model
reasoning. No permission bypass or latest-session selector is used.

Both CLI smokes also resume a nonexistent, complete UUID. The installed Codex
and Claude Code versions reject it before issuing another local model request;
Codex emits no new `thread.started` event. This checks that recovery does not
silently become a new conversation. The positive context assertion covers
committed user/assistant history; an uncommitted interrupted turn remains the
provider's own persistence policy and is never re-injected by this component.

Recovery policy JSON is explicitly tagged: `{"mode":"disabled"}` or:

```json
{
  "mode": "enabled",
  "adapter": "codex-cli-resume-v1",
  "provider": "codex",
  "args": []
}
```

Both variants reject unknown executable/endpoint fields. The registry validates
adapter registration before Host startup; the saved conversation identity and
adapter-specific argv are validated again before recovery is planned.

`//apps/humata2/terminal/host:agent_resume_codex_test` additionally exercises the
actual Codex TUI inside the real Host and holder. It confirms saved history in
VT snapshots before and after killing the owned CLI, automatic launch of one
new terminal UUID/PID with the same provider conversation ID, preservation of
the exited terminal and its predecessor link, and no additional model request.
This stronger integration uses the same isolated helper and local response
server. It does not claim an upstream model call or pixel-level UI rendering.

### Forked conversation persistence

```sh
bazel test //apps/humata2/terminal/provider:codex_fork_test \
  //apps/humata2/terminal/provider:claude_fork_test \
  --test_env=TERMINAL_CODEX_BIN=/absolute/path/to/codex \
  --test_env=TERMINAL_CLAUDE_BIN=/absolute/path/to/claude --test_output=all
```

These manual targets invoke real provider operations, not session-file copies:
Codex 0.169.0 uses `exec fork PARENT_ID`, and Claude Code 2.1.325 uses
`--resume PARENT_ID --fork-session`. Both passed with distinct child identifiers
from structured CLI JSON. The child inherits committed parent history, commits
its own turn, survives an owned-process kill during a pending request and then
resumes its exact child ID with both sets of committed context intact.

A later explicit parent verification request excludes every child-only marker.
This proves conversation isolation before that verification turn, not identical
filesystem metadata: a provider may update branch relationships or last-used
metadata. The seed parent process has exited; this case does not certify two
simultaneous interactive agents. Responses are deterministic loopback fixtures,
with no real credentials, user project, personal session state or upstream model.

The pending, uncommitted user turn remains provider-defined. The measured Codex
fork retains it, while the measured Claude fork rolls it back. Both preserve
the committed fork history. Proof artifacts are `codex-fork-proof.json` and
`claude-fork-proof.json` in their Bazel undeclared outputs. Subsequent recovery
uses only the saved child ID; repeating a fork flag there would create another
branch, so the recovery planner continues rejecting fork options.

Copilot 1.0.112-0 has a separate `copilot_fork_test`: it launches the real TUI
in an owned PTY, invokes `/fork branch-name`, records the distinct foreground
child ID and resumes that exact child. After committed branch work and a killed
pending child turn, another exact-ID resume retains parent and child history.
An explicit parent verification excludes child-only conversation messages;
provider fork metadata may change in both files. The unfinished Copilot turn is
rolled back. The fixture accepts only its own directory trust prompt and uses
private state and deterministic local responses. It does not simulate a fork
by copying session files, nor certify resurrection of background parent jobs.

The Host's native `ProviderBinding::activate_fork` integration and persistence
boundary are documented in [agent recovery](../host/AGENT_RECOVERY.md).

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

# Milestone: AGENTFOLDERS (ATM-1..7)

Agent session artifacts folders visible in the Files tab as a read-only, live-updated, collapsed-by-default section.

## Authoritative Decisions
- Locked-in UI/UX: `DECISIONS.md` §5 (*Agent folders in Files*, ATM-19)
- Approved render: `renders/05-files-links/01-files-agent-collapsed.png`
- Containment policy: NO symlinks into the session folder; expose via a dedicated read-only `SessionAgentState` file root so `svc_files` containment (`Target::checked`, `unfollowed`) refuses symlinks leaving the root.
- Access policy: user-only by default (`Root::AgentState(SessionRef)` refuses agents with `OutsideHumataFolders`); cross-session agent reads remain gated / off by default.

## Step Summary

### ATM-1: Per-provider artifacts folder resolver (`daemon/conversation`)
- Added `SessionArtifacts` (`provider`, `root`, `watch_dirs`, `side_paths`), `Adapter::artifacts_folder`, and `resolve_from_launch` in `apps/humata2/daemon/conversation/src/{lib,claude,codex,copilot}.rs`.
- **Claude**: resolves `<CLAUDE_CONFIG_DIR|~/.claude>/projects/<dashed-folder>` (with canonical path fallback) plus conversation side files (`file-history/<id>/`, `todos/<id>-agent-*.json`, `plans/<slug>.md`).
- **Codex**: resolves the rollout directory under `<CODEX_HOME|~/.codex>/sessions/YYYY/MM/DD/` (falling back to `sessions/` before rollout creation).
- **Copilot**: resolves `<COPILOT_HOME|~/.copilot>/session-state/<id>/`.
- Unit tests cover all three providers, custom config dirs, side files, canonical fallback, and invalid/traversal IDs.

### ATM-2: Antigravity (`agy`) investigation & explicit status
- Investigation: Antigravity (`agy`) writes per-session artifacts under `~/.gemini/antigravity-cli/brain/<conversation-id>/`, but Humata launches `agy` directly with `ConversationFormat::None` (`daemon/sessions_core/src/agents.rs:783-800`) and no wrapper, hook, or `--session-id` flag to pin or learn `<conversation-id>`.
- Decision: `agy` / `antigravity` is **explicitly marked unsupported** (`is_unsupported_agent`, `resolve_from_launch` returns `None`). Per `DECISIONS.md` §5, the Files tab hides the Agent files footer row for unsupported agents rather than showing a dead "not shown" row.

### ATM-3: Read-only `SessionAgentState` file root (`svc_files`, proto & access seams)
- Proto seam (`api/proto/humata/v1/common.proto`): `FileLocation.session_agent_state = 11` with `SessionAgentState { SessionId session = 1; string conversation_id = 2; }`.
- Access seam (`daemon/access/src/{call,policy,channel_desk}.rs`): `Root::AgentState(SessionRef)` is user-only by default (`OutsideHumataFolders` for agents on read & write).
- `daemon/svc_files/src/roots.rs`: resolves `SessionAgentState` for current or previous (`store.previous_conversations`) conversation IDs with `writable: false` and read-only error messaging.

### ATM-4: Re-point root & watcher on session-id switches (`pointers.rs`, `svc_files`)
- `daemon/humatad/src/session_service/pointers.rs`: `Cache::watch` includes `session_agent_watch_dirs(&store, &home, &session)`. Because conversation-id switches (`/clear`, first bind, fork, start-fresh, rebuild, restart) emit `Session` `Updated` and poke `pointers`, `Cache::watch` detects the changed `wanted_paths` set, drops the old watcher, and watches the new folder while streaming live changes via `FileChanges`.
- Previous conversation folders remain browsable via `SessionAgentState { session, conversation_id: old_id }`.

### ATM-5: Files tab UI (`ui/workspace/src/panel_files.rs`) & `FindFiles` (`svc_files/src/find.rs`)
- `svc_files/src/find.rs`: includes `SessionAgentState` (current + previous conversations) for user callers (skipping on `PermissionDenied` so agent `FindFiles` calls remain unaffected).
- `ui/workspace/src/panel_files.rs`:
  - Separates `@agent-state*` virtual repo entries from normal repositories.
  - Collapsed by default as a quiet footer row (`▸ Agent files · <Provider> · <N files> 🔒 read-only`).
  - Expands on demand with human-readable labels (`Transcript (current)`, `Subagents (N)`, `Tool outputs (N)`, `Plans (N)`, `Todos (N)`, `Earlier before /clear · N`) and quiet empty state.
  - Hides unsupported agents (`Shell`, `Agy`, `OpenCode`).

### ATM-6: Unit, containment, access & watcher tests
- `//apps/humata2/daemon/conversation:conversation_unit_test` (+ `claude_test`, `codex_test`, `copilot_test`)
- `//apps/humata2/daemon/svc_files:svc_files_unit_test` (per-provider fixtures, read-only enforcement, symlink escape containment, previous conversation access, agent access refusal)
- `//apps/humata2/ui/workspace:workspace_unit_test` (visibility, default collapsed state, partitioning, human-readable labels)
- `pointers.rs` unit test (watcher re-pointing when `agent_dirs` changes on conversation-id switch)

## Follow-Up Wiring Seams (if needed in `clients/live_daemon`)
- Map `FileLocation.session_agent_state` in `FindFiles` client decoding to `SessionFile { repo: "@agent-state" | "@agent-state:prev:<id>", path, folder }` and route `read_session_file` for `@agent-state*` to `FileLocation::SessionAgentState`.

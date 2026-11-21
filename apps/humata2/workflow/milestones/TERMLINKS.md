# TERMLINKS — Clickable Terminal File Links (ATM-14..18)

## ATM-14: Audit & Link Flow

### Click-to-Dispatch Path
1. **Pointer hit / click:** `TerminalView::mouse_down` (`terminal/view`) checks for an OSC-8 hyperlink or detected link under the cell.
2. **Host callback:** `GhosttyHost::open_link` emits `PaneEvent::OpenLink(uri)` through `TerminalPane`.
3. **Replica forwarding:** `TerminalReplica` forwards `ReplicaEvent::Pane(PaneEvent::OpenLink(uri))`.
4. **Workspace dispatch:** `apps/humata2/ui/workspace/src/terminals.rs` handles `ReplicaEvent::Pane(PaneEvent::OpenLink(uri))`:
   - Web and mail URIs (`http:`, `https:`, `mailto:`, no control chars) continue to open via `cx.open_url(uri)`.
   - All other links are passed through `resolve_roots_for_session` + `resolve_terminal_file_link(uri, &roots)`.
   - A resolved in-session file dispatches `SessionAction::OpenFileAt { repo, path, line }`.
   - Any unresolved or rejected link shows the existing refusal toast and stays closed.

### URI Shapes Handled
- OSC-8 `file:///abs/path` and `file://localhost/abs/path` (case-insensitive scheme and `localhost`, percent-decoded, rejecting `%00`, invalid UTF-8, and non-`localhost` hosts).
- Plain absolute and relative paths with optional `:line`, `:start-end`, or `:line:col` suffixes (1-based `u32`, `start >= 1`, `end >= start`).
- Non-file schemes (`http://`, `https://`, `mailto:`, `javascript:`, `ssh://`, `x-apple.systempreferences:`, etc.), control characters, `..` traversal, directories, and symlinks escaping a session root are rejected.

## ATM-15: Safe In-Session Path Resolver (`ui/workspace/src/terminals.rs`)
- `ResolveRoots` models the session folder (`repo: ""`), session clones (`repo: "owner/name"`), channel `main/<repo>` clones (`read_only: true`, `repo: "main/<repo>"`), and channel `shared/` folder (`read_only: true`, `repo: "shared"`).
- **Local sessions (`local: true`):** canonicalizes both root and target via `std::fs::canonicalize`, requiring `canonical_target.starts_with(&canonical_root) && canonical_target != canonical_root && canonical_target.is_file()`. Relative paths are checked across all roots and deduplicated by canonical path (preferring clone matches over session-folder matches); ambiguous matches (>1 distinct canonical file) return `None`.
- **Remote sessions (`local: false`):** performs lexical normalization (rejecting `..`) and prefix matching so `OpenFileAt` still dispatches and the remote daemon enforces canonical containment.

## ATM-16..18: `SessionAction::OpenFileAt` & Highlight Fade (`features/session/src/lib.rs`)
- `FileJump { first: u32, last: u32, generation: u64 }` stored in `SessionState::file_jump` (keyed by `(SessionId, repo, path)`), evicted on `evict_session` and when the last tab for that file closes.
- `SessionAction::OpenFileAt { repo, path, line }` opens `Tab::File` beside the terminal (`split: true`), triggers `read_file`, and when `line` is `Some((start, end))` with `start >= 1`, records `FileJump` and schedules a `FILE_JUMP_FADE` (2.5s) timer yielding `SessionAction::ClearFileJump { session, repo, path, generation: Some(generation) }`.
- `SessionAction::ClearFileJump` clears the highlight when `generation` is `None` (user click/scroll) or matches the stored generation (ignoring stale timers from earlier jumps).

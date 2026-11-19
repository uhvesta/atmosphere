# Humata 2: the MCP server agents use

Agents reach Humata only through MCP. Every action a person can take in
Humata has an MCP tool or is listed in §7 as user-only, and each tool is one
call (or a fixed set of reads) on the internal gRPC API ([GRPC.md](GRPC.md)),
checked against one capability ([BOUNDARIES.md](BOUNDARIES.md)). MCP adds no
behaviour of its own: the same call through the CLI ([CLI.md](CLI.md)) does
the same thing under the same check.

## 1. The server

- Binary: `humatad mcp`, the same binary as the daemon. Transport: stdio.
  Server name `humata`. One process per agent process.
- Humata starts it, not the agent (D748): the daemon writes MCP config files
  into its state folder, naming `humatad2 mcp --socket <socket>` and no
  token. Each built-in agent is started and resumed with flags that load the
  server on top of the agent's own (`claude --mcp-config=<file>`, `copilot
  --additional-mcp-config=@<file>`, `codex -c mcp_servers.humata={…}`) and
  pre-allow its tools. Nothing is written into the agent's own config. The
  holder sets `HUMATA_SESSION_TOKEN` (128 random bits, new on
  every launch and resume) in the agent's environment.
- On start the server connects to its machine's socket and presents the
  token. The daemon binds the connection to that session only after checking
  that the connecting process descends from that session's holder (the
  holder's pid and start time, so a reused pid doesn't pass). A token copied
  into another session's process tree, or into a process outside every
  session, fails with `identity_mismatch`.
- **No tool takes a requester, session-of-caller, channel-of-caller or
  principal argument.** The caller is the bound session. Tools that act on
  another session take it as the target, and the target is what the
  capability check is about.
- `humata` run inside a session's terminal (by the agent or by anything it
  starts) binds the same way and acts as that session's agent, so the CLI is
  never a way around MCP's checks (BOUNDARIES.md §2.2).

## 2. Names

Tool names are `<area>_<verb>`, because many agents' function-calling
formats reject dots. The operation id, used in audit, history, the Inbox card
and the journal, is `<area>.<verb>`. The spec's `humata.review.draft` is the
tool `review_draft` on server `humata` with operation id `review.draft`;
`humata.view.prepare` is `view_prepare`; `humata.board.move` is
`board_item_move`; `humata.session.fork` is `session_fork`;
`humata.bridge.connect` is `network_connect`.

## 3. Results and errors

Every successful tool result has `structuredContent` with this envelope, and
a one-line `text` content saying the same for agents that read only text.

```json
{
  "$id": "https://humata.dev/mcp/v1/envelope.json",
  "type": "object",
  "required": ["outcome"],
  "properties": {
    "outcome": {
      "enum": ["applied", "journaled", "pending"],
      "description": "applied: done on the host or this machine. journaled: the host is away; written to this machine's journal and applied when it returns (GRPC.md §9). pending: waiting for the user; call inbox_wait with request_id."
    },
    "result": { "description": "The tool's output; set when applied. For a journaled write it is empty: a new item has no id until the host replays it." },
    "request_id": { "type": "string", "description": "Set when pending, and when an applied call went through the Inbox." },
    "journal": {
      "type": "object",
      "properties": { "machine": { "type": "string" }, "seq": { "type": "integer" } }
    },
    "stale_as_of": {
      "type": "string", "format": "date-time",
      "description": "Reads only: the host is away and this is the last copy, from this time."
    }
  }
}
```

A failure is a tool result with `isError: true` and `structuredContent`:

```json
{
  "$id": "https://humata.dev/mcp/v1/error.json",
  "type": "object",
  "required": ["error"],
  "properties": {
    "error": {
      "type": "object",
      "required": ["kind", "message", "fix"],
      "properties": {
        "kind": { "type": "string", "description": "errors.proto kind, e.g. denied, user_only, revision_conflict, host_unreachable, rate_limited" },
        "message": { "type": "string", "description": "What failed." },
        "fix": { "type": "string", "description": "How to fix it, or why the obvious fix is unsafe." },
        "retry": {
          "type": "object",
          "properties": {
            "retryable": { "type": "boolean" },
            "after": { "type": "string", "format": "date-time" }
          }
        },
        "details": { "type": "object", "description": "The kind's fields, e.g. denied.rule, revision_conflict.current_revision" }
      }
    }
  }
}
```

Protocol errors (JSON-RPC errors) are reserved for malformed calls the
schema already rejects. Agents branch on `error.kind`, never on the text.

`denied` names the rule and says only the user can change it in Agent
access; it never suggests retrying (IN-18). `user_only` says the action is
the user's, that no rule lets an agent do it (IN-23), and that the agent can
tell the user what it needs with `inbox_ask`; it adds nothing to the Inbox.
`rate_limited` says to wait until `retry.after`.

## 4. Waiting for the user (TP-10..15)

When a call needs an answer (a rule says Ask me, or the call is `inbox_ask`
or `session_ask`), the daemon makes an Inbox request and the tool call stays
open.

- TP-11: the server sends `notifications/progress` every 15 s when the client gave
  a `progressToken`, so clients that reset their timeout on progress keep
  waiting. Many clients don't; so each waiting tool takes `wait_seconds`
  (default 45, at most 600) and returns `{outcome: "pending", request_id}`
  when it passes. The request stays in the Inbox.
- TP-12: `inbox_wait {request_id, wait_seconds?}` waits again and returns the same
  envelope the original call would have: the result once approved, `denied`
  with the user's note, or `pending` again (WaitResponse.result or .error).
- TP-13: **approval runs the call**, under the original idempotency key, whether or
  not the agent is waiting at that moment. The result is kept for 24 hours
  for `inbox_wait` (`result_kept_until`), and a line is typed into the
  agent's terminal when its turn ends (IN-12, SE-23). TP-14: an agent that
  gets `pending` must not repeat the call with a new key; repeating it with
  the same key returns the same request.
- TP-15: a request is **withdrawn** (the card greys out and can't be answered) when
  the agent calls `inbox_withdraw`, when its session stops, archives or is
  deleted, or when its MCP process exits while the call is still open and
  no `pending` was returned. A request that returned `pending` survives the
  MCP process: a resumed agent can still `inbox_wait` on it. The card
  records why (`InboxRequest.withdraw_reason`).
- TP-10: while a call waits Humata types nothing into the agent's terminal.
- While the host is away, requests follow the channel's While I'm away rule
  (AW-7, GRPC.md §9.3):

  | Away rule | What a call that would ask does |
  | --- | --- |
  | Wait (default) | waits as above; the user can answer on the machine with `humata login` + `humata inbox` (SEC-13) |
  | Skip the step | fails at once with `user_away`; nothing is queued (AW-4) |
  | Allow low-risk | the low-risk set (AW-10) runs and is journaled for review; anything else waits |

  Never-rule and user-only calls are refused the same way with the host
  away. `network_connect` is never low-risk and can't be approved on the
  machine while the Mac is away (HO-11, AW-11): under Wait and Allow
  low-risk it waits for the Mac; under Skip it fails at once.
  Rules, grants and the away rule are the last copy the Mac sent (AW-8).

## 5. Offline behaviour, per tool

Each tool below lists one of:

- **local**: answered by the agent's own machine; the same with the host away.
- **journal**: applied on the host; with the host away, journaled (§3) if the
  machine's policy copy allows it, else per the away rule.
- **host**: needs the host; with the host away it waits (TP-9) or, under
  Skip, fails with `host_unreachable`.
- **stale**: a read of host state; with the host away, answered from the last
  copy with `stale_as_of`.

## 6. The tools

Notation: `T?` optional, `A|B` one of, ids are strings. Common types:

```text
Board      = "session" | "channel" | "global" | {session: SessionId}
Location   = {root: "clone", repo: "owner/name", session?: SessionId, machine?: MachineId, path}
           | {root: "shared", path, machine?: MachineId}
           | {root: "channel_folder", path, machine?: MachineId}
           | {root: "main_clone", repo, path, machine?: MachineId}     -- read-only
           | {root: "session_folder", session?: SessionId, path, machine?: MachineId}
             -- the session's own folder, where a Desk session's documents sit (D382);
             -- judged as that session's clone: its own agent reads it, another's asks
             session and machine default to the caller's own
Item       = {id, board, title, description, column, rank, sessions: SessionId[], depends_on: [{id, title, done}], done_at?, created_by, revision, pinned, vip}
Column     = "backlog" | "to_do" | "in_progress" | "in_review" | "done"
Reason     = string shown on the Inbox card as the agent's words (IN-9)
Wait       = wait_seconds?: int
```

### 6.1 Default: allowed for its own session and channel

"none" in the capability column is IN-16's own-session set: no rule
applies and none can be set. Everything else is a rule whose default is
Allow (IN-17); a user can set any of those to Ask me or Never.

| Tool | Op / gRPC | Capability | Input → output | Side effects | Host away |
| --- | --- | --- | --- | --- | --- |
| `whoami` | `daemon.whoami` / WhoAmI | none | `{}` → `{session, channel, machine, agent, host_reachable, policy_as_of}` | none | local |
| `session_info` | `session.get` / GetSession(self) | none | `{}` → `{title, status, repos:[{repo, branch, head, pushed, dirty}], fork_of?}` | none | local |
| `topology_get` | `channel.topology` / WhoAmI + GetChannel + ListSessions + GetGrid | CHANNEL_READ | `{}` → `{this_machine, machines:[{id, name, reachability, sessions:[{id, title, agent, status}]}]}` (TP-1). This machine first, then the channel's, then any other a live session is on; `reachability` is `online`, `offline` or `unknown` (no grid cell), read from the grid (D772) | none | stale |
| `access_explain` | `access.explain` / Explain | none (self only) | `{capability, target?}` → `{decision, source, grant_id?, host_away, away_mode}` | none | local (from policy copy) |
| `board_items_list` | `board.items_list` / ListItems | OWN_BOARD_WRITE rule's read (session board); CHANNEL_READ (channel board) | `{board, columns?: Column[], page_token?}` → `{items: Item[], counts, next_page_token?}` | none | stale |
| `board_item_get` | `board.item_get` / GetItem | as list | `{id}` → `Item` | none | stale |
| `board_item_history` | `board.item_history` / GetItemHistory | as list | `{id, page_token?}` → `{entries:[{at, by, change, lost_conflict}]}` | none | stale |
| `board_item_create` | `board.item_create` / CreateItem | OWN_BOARD_WRITE (session board), CHANNEL_BOARD_ADD (channel board) | `{board?, title, column?, description?, sessions?: SessionId[], depends_on?: ItemId[], reason?}` (no `board`: the caller's session board; a column defaults to to_do) → `Item` (journaled: `{}`, with the journal position) | adds an item | journal |
| `board_item_update` | `board.item_update` / UpdateItem | OWN_BOARD_WRITE (session board); CHANNEL_BOARD_EDIT_OWN for channel items it made | `{id, title?, description?, sessions_add?, sessions_remove?, depends_on_add?, depends_on_remove?, expected_revision?, reason?}` (a stale revision fails only when a field it sets changed since; lists never conflict, D155) → `Item` | changes fields given | journal |
| `board_item_move` | `board.item_move` / MoveItem | as update | `{id, column, after?: ItemId, expected_revision?, reason?}` → `Item` | moves it | journal |
| `board_item_delete` | `board.item_delete` / DeleteItem | OWN_BOARD_WRITE (session board); CHANNEL_BOARD_EDIT_OWN for channel items it made | `{id, expected_revision?, reason?}` → `{deleted: true}` | tombstone, kept in history | journal |
| `view_prepare` | `view.prepare` / PrepareView | VIEWS_PREPARE | `{title, summary, content: {review: {steps:[{repo, path, lines, note}]}} \| {board: scope} \| {pull_request: {repos:[{repo, title, body, base_branch, draft}]}}}` → `{view_id, state: "draft"}` | a draft view; nothing opens | journal |
| `view_revise` | `view.revise` / ReviseView | VIEWS_PREPARE, own views | `{view_id, title?, summary?, content?, expected_revision?}` → view | replaces the draft | journal |
| `view_mark_ready` | `view.mark_ready` / MarkReady | VIEWS_PREPARE, own views | `{view_id}` → `{state: "waiting"}` | the offer appears (VW-2); preparing a PR pushes nothing | journal |
| `views_list` | `view.list` / ListViews(self) | none | `{include_forks?}` → `{views:[{id, title, state}]}` | none | stale |
| `review_draft` | `review.draft` / CreateComment(dest PR) | REVIEW_DRAFT, PR review sessions only | `{pr: "owner/repo#n", path, lines: {first, last}, side?: "old"\|"new", quoted, text}` → `{comment_id, status: "draft"}` | a draft, never posted (PR-12) | journal |
| `review_draft_edit` | `review.draft_edit` / EditComment | REVIEW_DRAFT, own drafts | `{comment_id, text, expected_revision?}` → comment | | journal |
| `review_draft_drop` | `review.draft_drop` / DropComment | REVIEW_DRAFT, own drafts | `{comment_id}` → `{dropped: true}` | | journal |
| `review_commits` | `review.commits` / ListPullRequestCommits(self) | none, PR review sessions only (D299) | `{pr: "owner/repo#n"}` → `{head_sha, commits: [{sha, subject, patch_id, state: "unreviewed"\|"reviewed"\|"carried", reviewed_as_sha, reviewed_at?, reviewed_head_sha, by?: {actor, gh_login}}], history: [{seq, at, kind: "marked"\|"unmarked"\|"carried"\|"whole_pr", commits, head_sha, snapshot_n?, by?}]}`; only the user marks commits | none | stale |
| `review_context` | `review.context` / GetReviewContext(self) | none, PR review sessions only | `{}` → `{diff_specs, open: {repo, path, visible_lines, selection?}, order, groups, viewed, notes, stack?: [{title, purpose?, repos: [{repo, range? \| worktree?, commits: [resolved sha, subject], drift?}]}], layer: title or "" for Whole stack}` | none | stale |
| `review_focus` | `review.focus` / FocusReview | REVIEW_FOCUS (PR-25 "Focus"), own review | `{layer?: title ("" = Whole stack), repo?, path?, lines?: {first, last}, side?, highlights?: [{repo, path, lines, label?}]}`; with no `repo` and `path`, only the layer changes → `{applied: true, undo_id}` | moves the view now, after the user stops scrolling or typing (PR-24) | live |
| `review_note` | `review.note` / CreateComment(dest note) | REVIEW_NOTE (PR-25 "Notes"), own review | `{repo, path, lines?, hunk_at?, side?, quoted?, layer?: title (absent = Whole stack), text}` → `{note_id, undo_id}` | a note for the user, never posted (PR-23); outdated when its lines change (RV-34) | journal |
| `review_note_edit` | `review.note_edit` / EditComment(note) | REVIEW_NOTE, own notes | `{note_id, text}` → `{note_id, undo_id}` | new text | journal |
| `review_note_drop` | `review.note_drop` / DropComment(note) | REVIEW_NOTE, own notes | `{note_id}` → `{dropped: true, undo_id}` | | journal |
| `review_prepare` | `review.prepare` / PrepareView(review_plan) | VIEWS_PREPARE (RV-32); in the review chat REVIEW_ORDER (PR-25 "Order") | `{title, spec?: {repos:[{repo, base: {merge_base_with \| rev \| reviewed_snapshot \| fork_point \| index}, upto?, untracked?, untracked_paths?, include_paths?, exclude_paths?, whitespace?, renames?}]}, stack?: {layers: [{title (unique, max 60), purpose? (max 120), repos: [{repo, range?: {base, head}, worktree?: {include_paths}, pull_request?}] (each repo one of range or worktree), order?: [{repo, path}], groups?: [{title, note?, first}], importance?}] (max 50, bottom first)}, from_stack?: {repo, tip?, topics?: [{topic, title?, purpose?}]}, importance?: [{repo, path, tier: key \| normal \| skim}], order?: [{repo, path}], groups?: [{title, note?, first}], notes?: [{repo, path, lines?, hunk_at?, side?, layer?, text}], reason?, wait_seconds?}`. Send `spec` alone, or `stack` and/or `from_stack`, never `spec` with either → `{view_id, state: "draft"}`, or in the review chat `{applied: true, undo_id}` | a review view offer (VW-2); in the review chat the view changes at once (PR-24) | journal |
| `review_summary_draft` | `review.summary_draft` / DraftSummary | REVIEW_DRAFT | `{pr, summary}` → `{}` | the drafted summary the user edits | journal |
| `comments_list` | `comment.list` / ListComments(self) | none (a read of its own session, D21) | `{status?: ("sent"\|"queued"\|"addressed")[], path?}` → `{comments:[{id, repo, path, lines?, side?, quoted, text, status, addressed_text?, replies:[{by, text}]}]}` (q2); without `status`, all three; never drafts or resolved ones | none | stale |
| `comment_reply` | `comment.reply` / ReplyToComment | COMMENT_REPLY, comments typed into its own session (DK-9) | `{comment_id, text}` → `{comment_id, status, replies}` | a reply in the comment's thread, under the agent's name | journal |
| `comment_resolve` | `comment.resolve` / ResolveComment | COMMENT_RESOLVE, comments typed into its own session (SM-19) | `{comment_id, reply}` → `{comment_id, status: "resolved", replies}` | resolves the comment; the reply is required and joins the thread; the user can reopen it | journal |
| `doc_versions_list` | `doc.versions_list` / ListDocVersions(self) | none (a read of its own session's folder, D21) | `{path, location?: {session?}}` (default: its own session) → `{versions:[{id, number, unix_ms, by: "you" or the agent, message, added, removed, has_comments}], unsaved_changes}` (newest first) | none | local |
| `doc_version_read` | `doc.version_read` / ReadDocVersion(self) | none (own session's folder, D21) | `{path, version, location?: {session?}}` → `{text, comments?:[{id, lines, quoted, text, status, replies}]}`; `comments` absent when the version has no record (DV-12) | none | local |
| `files_list` | `file.list` / ListDirectory | none (own clones, `shared/` and the channel folder on its machine) | `{location: Location, page_token?}` → `{entries:[{path, kind, size, changed, writable}]}` | none | local |
| `file_read` | `file.read` / ReadFile | none (as above) | `{location, offset?, max_bytes?}` → `{text \| base64, size, sha256, truncated}` | none | local |
| `files_find` | `file.find` / FindFiles(self) | none | `{query}` → `{entries}` | none | local |
| `file_write` | `file.write` / WriteFile | none (own clone); SHARED_WRITE (`shared/` on its machine; another machine's `shared/` is refused, D755) | `{location, text \| base64, replace?: bool}` → `{path, size, sha256}` | creates or replaces one file; a replaced file goes to trash | local |
| `folder_create` | `file.folder_create` / CreateFolder | as write | `{location}` → `{path}` | | local |
| `file_fetch` | `file.fetch` / StartTransfer(copy) | FILES_FETCH_LOCAL: from `shared/`, the channel folder or a main clone on its machine into its own clone | `{source: Location, destination: Location, on_existing?: "fail"\|"replace"\|"keep_both"}` → `{transfer_id, files, bytes}` | copies | local |
| `inbox_ask` | `inbox.ask` / AskUser | INBOX_ASK | `{question, choices?: string[] (at most 4), wait_seconds?}` → `{reply, choice?}` | a question card (IN-25) | waits; journaled with the host away, answerable on the machine |
| `inbox_request` | `inbox.request` / RequestAccess | INBOX_ASK | `{capability, target, reason, wait_seconds?}` → `{decision, grant_id?}` | IN-26: a card naming the action, target and reason; approving makes the IN-22 grant for that target. A Never rule is denied at once; a user-only capability fails with `user_only` | as `inbox_ask` |
| `inbox_wait` | `inbox.wait` / WaitForAnswer | none (own requests) | `{request_id, wait_seconds?}` → the original call's envelope | none | local while journaled, else host |
| `inbox_withdraw` | `inbox.withdraw` / WithdrawRequest | none (own requests) | `{request_id}` → `{state: "withdrawn"}` | the card greys out | journal |
| `inbox_mine` | `inbox.mine` / ListRequests(self) | none (own requests) | `{pending_only?}` → `{requests:[{id, action_title, state, created_at}]}` | none | local + stale |
| `session_answer` | `session.answer` / AnswerQuestion | none (questions asked of it) | `{question_id, answer}` → `{}` | returns to the asking agent's call (TP-5) | host |
| `journal_status` | `journal.status` | none | `{}` → `{host_reachable, entries:[{seq, operation, state: "waiting"\|"applied"\|"lost_conflict"\|"rejected", error?}]}` | none | local |
| `operation_get` | `operation.get` / GetOperation | none (operations it started) |
| `network_close` | `bridge.close` / ClosePortForward | none (its own forwards) | `{forward_id}` → `{}` | closes the forward early | local | `{operation_id}` → `{kind, state, steps:[{key, title, state}]}` | none | stale |

### 6.2 Needs approval

These exist for agents but go to the Inbox under the defaults: Ask me
(approve, deny, deny with a note, or approve and "Always allow …" with the
card's scope). Those marked **Never** by default are denied at once with the
rule named until the user changes the rule (IN-18).

| Tool | Op / gRPC | Capability (default) | Input → output | Side effects | Host away |
| --- | --- | --- | --- | --- | --- |
| `channels_list` | `channel.list` / ListChannels | GLOBAL_READ (Allow by default, D127.4; Global board and other channels are one row, D21) | `{}` → `{channels:[{id, name, machines, sessions}]}` | none | stale |
| `repo_search` | `repo.search` / SearchRepos | GLOBAL_READ (Allow by default, D278) | `{query, owners?: string[], all_of_github?: bool, page_token?}` → `{library:[{repo, remote, default_branch, added_at, last_used_at, last_used_in?, last_used_channel?}], github:[{repo, description, private}], scope, problem?: {kind, message, retry_at?}, unknown_owners, next_page_token?}` | none; `owners` or `all_of_github` widen this one call and are never saved (RL-11, D479); a GitHub failure is `problem`, not an error (RL-10) | host; from a machine it is forwarded, and `host_unreachable` while the host is away (RL-12, D568) |
| `repo_library` | `repo.library` / ListRepoLibrary | GLOBAL_READ (Allow by default, D278) | `{}` → `{repos:[{repo, remote, default_branch, added_at, last_used_at, last_used_in?, last_used_channel?}]}`, most recently used first | none | host; forwarded from a machine (RL-12, D568) |
| `board_items_list` (global or another channel) | as 6.1 | GLOBAL_READ (Allow by default, D127.4) | as 6.1 | none | stale |
| `board_item_create/update/move/delete` on another's channel item | as 6.1 | CHANNEL_BOARD_EDIT_OTHERS (Ask) | as 6.1 | as 6.1 | per away rule |
| `board_item_*` reads on the Global board or another channel's board | as 6.1 | GLOBAL_READ (Allow by default, D127.4) | as 6.1 | none | stale |
| `board_item_create/update/move/delete` on the Global board or another channel's board | as 6.1 | GLOBAL_BOARD_WRITE (Ask, D155), whoever made the item | as 6.1 | as 6.1 | per away rule |
| `files_list`, `file_read` in another session, same machine | as 6.1 | FILES_READ_OTHER_SESSION (Ask); also `humata attach --read-only` of another session (D21) | `location.session` set | none | per away rule (local) |
| `files_list`, `file_read` on another machine | as 6.1 | FILES_READ_OTHER_MACHINE (Ask) | `location.machine` set | none | host |
| `doc_versions_list`, `doc_version_read` in another session, same machine | as 6.1 | FILES_READ_OTHER_SESSION (Ask), as `file_read` (DV-9) | `location.session` names a session on this machine | none | per away rule (local) |
| `doc_versions_list`, `doc_version_read` on another machine | as 6.1 | FILES_READ_OTHER_MACHINE (Ask), as `file_read` (DV-9) | `location.session` names a session on another machine; the rule is checked, then the call is refused until the SAFE milestone reads that machine's versions | none | host |
| `files_copy_in` | `file.copy_in` / StartTransfer | FILES_COPY_IN (Ask) | `{sources: Location[], destination: Location (own clone or shared/ on its machine), on_existing?, reason?}` → `{transfer_id}` | copies into its own clone or `shared/` on its own machine only (TP-4, D755) | host (other machine) / per away rule (same machine) |
| `session_note` | `session.note` / SendNote | SESSION_NOTE (Ask) | `{session, text, when?: "now"\|"turn_end"}` → `{delivered: "typed"\|"queued"}` | typed into the target's terminal as one message naming the sender (TP-5) | host for another machine; per away rule here |
| `session_ask` | `session.ask` / AskSession | SESSION_NOTE (Ask) | `{session, question, wait_seconds?}` → `{answer}` | the target is asked; its `session_answer` returns here | as note |
| `session_type` | `session.type` / Type | SESSION_TYPE (Ask) | `{session, text, submit?: boolean (default true), when?: "now"\|"turn_end"}` → `{typed, submitted, agent_busy, queued_id?}` (`agent_busy`: written while the agent was mid-turn, so the agent decides whether to queue it) | types the text and presses Enter; `submit: false` leaves it in the input; for one paste wrap the text in ESC[200~ and ESC[201~ yourself | as note |
| `session_start` | `session.start` / StartSession | SESSION_START_SAME_MACHINE or _OTHER_MACHINE (Ask) | `{machine, agent?, model?, repos?: string[], title?, first_message?, reason}` → `{operation_id, session}` | a new session in its channel | per away rule / host |
| `session_fork` | `session.fork` / ForkSession(self) | as start (Ask) | `{machine, kind: "code_and_context"\|"code_only"\|"context_only", agent?, before_turn?, repos?, title?, first_message?, reason}` → `{operation_id, session}` | a fork of itself (TP-6); the card shows agent, machine, kind, repos, cost | as start |
| `session_continue_in_channel` | `session.continue_in_channel` / ContinueInChannel(self) | as start (Ask) | `{channel, before_turn?, reason}` → `{operation, session}` | a new session in another channel of its own conversation copied (CV-8); itself untouched; the card shows the channel and the turn | as start |
| `network_connect` | `bridge.connect` / BridgeService.OpenPortForward | NETWORK_REACH (Ask; Ask me or Never only, D20) | `{host, port, reason, wait_seconds?}` → `{forward_id, socket_path}` (a unix socket in its session folder, bridged through the Mac) | HO-11: the card shows its machine, host and port and that it stays open until the session stops; it closes then. Never low-risk; can't be approved while the Mac is away. Nothing else crosses the bridge (HO-10) | host: waits for the Mac, or fails under Skip |
| `session_rename`, `session_add_repo`, `session_drop_repo`, `session_own_copy` | `session.*` (self) | OWN_SESSION_MANAGE (Ask) | `{title}`, `{repo}`, `{repo}` (+ plan), `{}` (every repo, FK-9) | change its own session | host |
| `session_stop`, `session_archive`, `session_delete` (self) | `session.*` (self) | OWN_SESSION_END (Ask) | `{}`; archive and delete show the plan on the card | end its own session | host |
| `sessions_search` | `session.search` / SearchSessions | none for its own channel; SESSIONS_SEARCH_OTHER (Ask) for others | `{query, channel?, machine?, agent?, since?, until?, page_token?}` → `{hits:[{session, channel, machine, agent, turn, snippet, as_of}], next_page_token?}` | none | host |
| `session_read` | `session.read` / ReadTurns | as `sessions_search` | `{session, first_turn?, last_turn?, page_token?}` → `{turns:[{seq, role, text}], next_page_token?}` (scrubbed, LD-21) | none | host |
| `session_move` | `session.move` / MoveSession | SESSION_MOVE (Ask) | `{session?: self, machine, reason}` → `{operation_id}` | moves a session (MV-*) | host |
| `session_push` | `session.push` / PlanPush + PushAndOpenPullRequests | GIT_PUSH (Ask, IN-24); review sessions follow PR-7 and PR-8 instead | `{repos?: string[], open_pr?: bool, title?, body?}` → `{operation_id}` | pushes and opens PRs; the card shows repos, branch, commits and PR title. An uncontained agent pushing with its own git credentials is not stopped (SEC-5) | host |
| `pr_review_start` | `review.start` / StartReview | PR_REVIEW_START (Ask) | `{prs: string[], machine, agent?}` → `{operation_id, session}` | a PR review session, read-only (PR-7) | host |
| `machine_add_ssh`, `machine_create`, `machine_start`, `machine_stop`, `machine_reconnect`, `skills_sync` | `machine.*`, `skills.sync` | MACHINE_PROVISION (Ask) | `{ssh_host, name}`, `{provider, name, params}`, `{machine}`… → `{operation_id}` | as in Hosts | host |
| `provider_import` | `provider.import` / RequestProviderImport | PROVIDER_IMPORT (Ask me or Never; no Allow, no Always allow, never in Approve all; D792) | `{file_text (at most 64 KiB), file_name?, reason, wait_seconds?}` → `{decision, imported, revision}` | MP-23: the card shows the file whole, every command each provider runs (on the Mac, or `run_on`, when a machine is made with it) and the ids it replaces; approving writes it, or fails `plan_stale` when the providers file changed since the ask. A file with problems is refused at once (`invalid_argument`) and nothing is asked. No path is read | host; a machine's daemon doesn't forward it yet |
| `trash_restore` | `trash.restore` / RestoreFromTrash | TRASH_RESTORE (Ask) | `{trash_item_id}` → `{restored}` | | host |
| `channel_create` | `channel.create` / CreateChannel | CHANNEL_CREATE (Ask) | `{name, repos, machines}` → `{channel}` | | host |
| `session_stop`, `session_archive` of another session | `session.*` | SESSION_STOP_OTHER (**Ask me**, D306; archive joins this row, D21) | `{session}` | | waits for the user |
| `session_delete` of another session | `session.*` | SESSION_DELETE_OTHER (**Never**, D306) | `{session}` | | denied at once |
| `session_restore`, `session_rebuild` | `session.restore` / RestoreSession, `session.rebuild` / RebuildSession | SESSION_START_SAME_MACHINE or _OTHER_MACHINE (Ask, D21) | `{session, machine?, reason}` → `{operation_id}` | an archived session back, or rebuilt from its record (SL-4) | host |
| `channel_rename`, `channel_add_repo`, `channel_remove_repo`, `channel_add_machine`, `channel_remove_machine` | `channel.*` | CHANNEL_CHANGE (**Never**) | | | denied at once |
| `machine_remove`, `machine_forget`, `machine_delete` | `machine.*` | MACHINE_DESTROY (**Never**) | | | denied at once |
| `machine_script_run` (not built yet) | `machine.script_run` / RunMachineScript | MACHINE_SCRIPT_RUN (**Never** by default, D688) | `{rule, machine, session?}` → `{operation_id}` | runs a rule from the owner's machine-scripts file now; there is no tool that writes rules or scripts | host |

### 6.3 Example schemas

`board_item_update`:

```json
{
  "name": "board_item_update",
  "description": "Change a board item's title, description or assignee. Your own items and your session board need no approval; other items go to the user's Inbox.",
  "inputSchema": {
    "type": "object",
    "required": ["id"],
    "additionalProperties": false,
    "properties": {
      "id": { "type": "string", "description": "PAY-7" },
      "title": { "type": "string", "maxLength": 200 },
      "description": { "type": "string", "maxLength": 20000 },
      "assignee": { "type": ["string", "null"], "description": "A session of this channel; null for nobody" },
      "expected_revision": { "type": "integer", "description": "From your last read; a stale one fails with revision_conflict" },
      "reason": { "type": "string", "maxLength": 500 },
      "wait_seconds": { "type": "integer", "minimum": 0, "maximum": 600, "default": 45 }
    }
  },
  "outputSchema": { "$ref": "https://humata.dev/mcp/v1/envelope.json" },
  "annotations": { "destructiveHint": false, "idempotentHint": true }
}
```

`session_fork`:

```json
{
  "name": "session_fork",
  "description": "Start a fork of this session. Usually asks the user first; the card shows agent, machine, kind, repos and cost.",
  "inputSchema": {
    "type": "object",
    "required": ["machine", "kind", "reason"],
    "additionalProperties": false,
    "properties": {
      "machine": { "type": "string", "description": "A machine id or name from topology_get" },
      "kind": { "enum": ["code_and_context", "code_only", "context_only"] },
      "agent": { "type": "string", "description": "The fork's agent; default: this session's agent" },
      "before_turn": { "type": "integer", "minimum": 2, "description": "Fork from just before this turn; default: from now" },
      "repos": { "type": "array", "items": { "type": "string" }, "description": "Default: all of this session's repos" },
      "title": { "type": "string" },
      "first_message": { "type": "string" },
      "reason": { "type": "string", "maxLength": 500 },
      "wait_seconds": { "type": "integer", "minimum": 0, "maximum": 600, "default": 45 }
    }
  },
  "outputSchema": { "$ref": "https://humata.dev/mcp/v1/envelope.json" },
  "annotations": { "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
}
```

Every tool's MCP call carries an idempotency key derived from the MCP
request: the server uses the JSON-RPC id plus the session's launch nonce, so
a client retry of the same request id is deduplicated, and a new request is a
new intent.

## 7. User-only: never available to agents

These have no tool. They are absent from `tools/list`; a client that calls
one by name anyway gets `user_only`, whose `fix` says to ask the user with
`inbox_ask`. The CLI offers them to the user with presence (CLI.md §3).

| Operation | Why |
| --- | --- |
| `channel.delete` | CH-25: only the user deletes a channel; there is no request for it either |
| `inbox.answer`, `inbox.answer_all` | An agent approving requests would approve its own |
| `access.set_rule`, `access.set_away_rule`, `access.set_defaults`, `access.revoke_grant`, `access.set_passphrase` | Changing access is how an agent would widen its own grant (BOUNDARIES.md §6) |
| `settings.update`, `channel_settings.update`, `agents.add`, `agents.update`, `agents.set_enabled` | Launch commands and their variables run as the user |
| `marks.set`, `pinned.reorder` | NV-20, NV-25 |
| `repo.remove`, `repo.forget`, `repo.update_scope` | RL-4, RL-11, D479: removing a repo from the library, forgetting the library and saving the search scope change the owner's lasting state; an agent may widen one `repo_search` call only |
| `review.submit`, `review.set_push_allowed` | PR-8, PR-12: only the user posts or lets an agent push |
| `diff.mark_reviewed`, `view.set_state`, the user's own comments and `comment.send` | The user's review state (RV-*, VW-*, SM-*) |
| `trash.empty` | Not undoable |
| `host.set_variable`, `host.remove_variable`, `host.trust_new_key`, `providers.set_config`, `provider.login`, `provider.make_unattended` | Secrets and host identity (HO-13, MP-23). An agent asks to import a provider with `provider_import` (§6, D792) |
| `machine.change_root`, `machine.set_vm_tier` | MACHINE_SETTINGS: root and the VM tier change what every agent there can reach |
| `machine.shell`, `terminal.attach` with input, `terminal.open_shell_tab`, `terminal.send_queued_now` | TERMINAL_INPUT: a shell is beyond every rule. An agent types into another session only through `session_type` |
| `bridge.close` of a forward the agent didn't open | The user's view of the bridge (HO-4) |
| `transfer.*` between locations that aren't the agent's own clone or `shared/`; pause or cancel of others' transfers | FM-* is the user's file manager; agents copy in only (TP-4) |
| `view_state.*`, `events.subscribe`, `audit.list` | The app's own state and the user's record of what agents did |
| writes into another session's clones | Not an agent action; for a context-only fork's parent a hard limit (IN-20) |

## 8. Requirements covered

Every requirement that crosses a boundary, with its tools, RPCs and
commands, is in [TRACE.md](TRACE.md).

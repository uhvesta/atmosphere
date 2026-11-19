# BOARDSIMPLE: Planning work on boards is simpler

- **Status:** open (owner, 2026-11-19: "I suspect our todo list or jira might be too complicated")
- **Depends on:** nothing. Revisits BOARD (done), D106, D155, D177, D216 and D334.
- **Unblocks:** nothing yet.

## Outcome
Planning work takes one obvious list that I can filter and view nicely, with the metadata I need, including due dates. It doesn't make me learn board kinds and scopes.

## The owner's direction
- Move towards a lightweight to-do model:
  - **one global list**, with channels and sessions as filters, not separate boards;
  - **optional metadata** on items (labels or key–value fields) and a nicer item view;
  - **due dates**.
- Do not name or link any outside project in code or docs.

## Today's model (to compare against)
- Three board kinds: Global, channel, session (BD-1). A Scope picker (BD-5, BD-11). New item lands on a board that depends on scope (BD-6, BD-7).
- Five fixed columns: Backlog, To do, In progress, In review, Done (`BoardColumn` in `api/proto/humata/v1/common.proto`).
- Three views: board, table and dashboard (BD-11–BD-13, frames 21–24).
- Item fields (`BoardItem` in `api/proto/humata/v1/boards.proto`): title, description, column, rank, created_by, pinned, VIP, several sessions (BD-16), Depends on (BD-14), done_at, pending agent asks (BD-15), revision and history.
- **Missing:** due dates, labels and custom metadata.
- Agents create and move items through MCP, with access checks and Ask me (BD-9, D180).

## The target model in detail (owner's reference, 2026-11-19)
A ranked to-do list instead of a board of columns:
- **One list, ranked for me.** Each item has a rough **effort** (s/m/l), an **impact**, a **need**, and an optional **due date** (relative input works, e.g. "next week"). The app ranks everything by need, age, how close the deadline is, and value per effort. It keeps the ranking current as time passes, shows each position's top reasons, and holds the order steady while I'm looking (re-ranking animates).
- **Kinds:** task, note, and reminder (hidden until its date). A note or reminder can become a task and keeps its key and history.
- **States, not columns:** open, in progress, blocked, closed. Blocked items wait on another item and are hidden by default. Closed items can be reopened for a week and appear in a Closed view.
- **Metadata:** tags with descriptions, which can be removed from items but never deleted; per-list custom fields with ordered options and weights that feed ranking and filters; numbered steps inside an item; comments; an audit log of every write.
- **One view, many controls:** the ranked list with sort (rank, due, effort, impact), filters (tag, effort, impact, field), state toggles and `/` search. The view state lives in the URL or saved per window. No kanban, no dashboard.
- **Agents:** an agent can claim the next item and work it, ask a question, request review or propose closing. Skills for "add", "status" and "do next".

## Compare and contrast
| | Ours today | Target |
|---|---|---|
| Scope | three board kinds + Scope picker | one list; channel/session are filters |
| Order | manual rank within five fixed columns | computed rank (need, age, due, value/effort), with reasons |
| Status | Backlog / To do / In progress / In review / Done | open / in progress / blocked / closed (+ reopen window) |
| Due dates | none | yes, relative input, feeds rank |
| Size/impact | none | effort, impact, need |
| Metadata | pinned, VIP | tags + custom weighted fields |
| Kinds | item only | task / note / reminder |
| Subtasks | none | numbered steps |
| Dependencies | Depends on, blocks nothing | blocked-by, hides until unblocked |
| Views | board, table, dashboard | one ranked list with sort/filter |
| Agents | MCP create/move with Ask me, several sessions per item | claim next / ask / request review / propose close |
| History | revisions + history | audit log |

## Proposed direction (for Fable to judge)
- **Merge:** the three board kinds into one list with filters for channel and session. "In review" folds into "in progress" plus a review request on the item, which is already an Inbox ask.
- **Add:** due date, effort and impact, a computed rank with reasons, tags, reminders, steps.
- **Cut or hide:** the Scope picker, the dashboard (a filtered list covers it), VIP (pinned covers it), and kanban as the default (keep it at most as an alternate layout).
- **Keep:** several sessions per item, Start a session from an item (SE-3), agent edits with Ask me, history.
- **"Do next":** an agent (or I) asks for the top-ranked open item and starts a session on it. This ties the list to the terminal loop UXSIMPLE is simplifying.

## Owner's direction on the look (2026-11-19)
The owner likes the reference to-do app's UI and wants ours to look like it: a calm ranked list, not a kanban board. Renders go to the owner in small batches (2–4 frames), each reviewed before the next is drawn.

## PR review requests in the To-do (owner, 2026-11-19)
- Pull requests where someone requested my review show up as To-do items. They come from GitHub through the gh token Humata already uses for saved repositories, via GitHub's review-requested search or notifications, fetched gently.
- An item shows the PR, its author and repo, and how long it has been waiting, and it is ranked like any other item. Opening it starts the PR review session.
- The item closes itself when I submit my review, the request is removed, or the PR is closed or merged.
- I don't hand-edit them beyond Pin, Remind me and Hide. Several PRs from one stack show as one item.

## Names (owner, 2026-11-19)
- The sidebar Inbox becomes the **Agent inbox**: agents stopped and waiting on me.
- The other inbox needs a clearer name than "inbox". Candidates: **To-do** with an "Agent suggestions" group, "Up next", or "Plan". Fable proposes, the owner picks.

## Agents' asks about to-do items: "Needs attention" (owner, 2026-11-19)
- An item an agent wants to change (close, a new due date, an edit) gets a **Needs attention** mark and sorts into a Needs attention group at the top of To-do. A proposed new item is an item in that group, marked Suggested.
- Opening the item shows the ask first: the change as a question, the agent's reason, the evidence (commits, PR, tests), before → after, and who asked. Then **Yes** or **No**.
- Moves and copies of files or sessions, and access asks, go to the **Agent inbox** with the same level of detail.
- There is no separate "From agents" group.

## Sign-off log
- 2026-11-19: the owner signed off on batch 1 (the To-do list with the Needs attention group, an opened item with its ask and Yes/No, and the filtered view; `mockups/boardsimple/b1-*.png`): "looks good". The xhigh review's fixes (one check circle per row, one control per state bucket, no repeated reasons, fewer filters, "Hide until", unranked agent items until Yes, overdue never hidden) apply on top.
- 2026-11-19: the owner signed off on batch 2 (Agent inbox: empty, file move card, session move card; one column of full cards, no list/detail split; `mockups/boardsimple/b2-*.png`): "looks good".
- 2026-11-19: the owner signed off on all nine batches (27 frames, `mockups/boardsimple/`, one line each in its FRAMES.md): "looks good enough". Batch 1 as touched up with the xhigh fixes is the signed-off version. Picks this settles: the Agent access middle value is **Ask** (not "Propose"); "Hide until" replaces "Remind me"; one "Show closed" toggle; the Agent inbox is one column of full cards. Still to settle while building (Fable rules): an overdue item never folds into Waiting (ruled, not drawn); undo windows (7 / 30 days) and the "Always allow for this session" wording are proposals. Reviews: BOARDSIMPLE_REVIEW.md, BOARDSIMPLE_XHIGH_REVIEW.md.

## Steps
1. Compare and contrast today's model with the target. Say what to merge (e.g. three board kinds into one list with filters), what to cut (e.g. VIP if pinned covers it, dashboard if a filtered table covers it), and what to add (due date, labels/fields). For each change, give the proto field numbers it touches and how existing items migrate (UPGRADE: a VIP Inbox "Migrate" message, no mixed-version fallback).
2. Fable 5.2 judges the proposal and renders the frames: list, item drawer with due date and fields, filtered by channel/session, and overdue/due-soon.
3. The owner approves before any change. Picks are logged in DECISIONS.md.

## Pointers
- SPEC BD-1–BD-16 (`grep -n '\*\*BD-' apps/humata2/SPEC.md`), frames 07, 12, 21–24 and 37.
- Code: `features/board`, `ui/src/board.rs` (or its split parts), `daemon/store/src/boards.rs`, `daemon/humatad/src/board_service.rs`, `cli/src/commands/boards.rs`, `api/MCP.md`.

## Lane (BOARDSIMPLE manager, Opus 5.5, 2026-11-19)

**Goal (release-plan step 10, 0.3.16):** one ranked To-do list replaces the boards. **Exit:** the To-do page and the session To-do panel draw from one list (channel and session are filters), with three states, a computed rank with reasons and Pin, optional due/effort/impact/tags/steps/Hide until, quick add, Needs attention items with Yes/No, PR review items and Do next; today's boards migrate keeping the owner's order; kanban, table, dashboard, the Scope picker and VIP are gone; the design goldens are redrawn from the signed-off frames and passed by Fable; focused targets green.

### Design notes
- **Domain crate `//apps/humata2/todo` (`humata2_todo`, new, pure, serde only).** The one place the to-do rules live, so the daemon (rank for MCP "do next"), the clients and the features agree:
  - `Day` (civil date, no clock), `Effort {S,M,L}`, `Impact {Low,Medium,High}`, `State {Open, InProgress, Closed}`, `Todo` (key, title, channel, sessions, state, due, effort, impact, tags, steps, hide_until, pinned + pin order, depends_on, blocks, suggested, pending ask, PR review, start rank from migration, created/closed days).
  - `rank(items, today) -> Ranked`: groups (Needs attention, In progress, Next, Waiting, Hidden until a date, Closed this week), the order within Next with each row's reasons, and Do next.
  - `quick_add::parse(line, today, channels)`, `filter` (channel, session, due Overdue/Next 7 days, tags, search), `migrate` (old columns and order to start ranks and "Was 1st in To do" reasons), `due_chip` wording.
- **Rank rule (D830):** pinned first in pin order; then score = due urgency + value per effort (impact ÷ effort); ties by the migrated start order, then age (older first). Age never adds score. PR reviews score by days waiting. Reasons appear only when the row doesn't already say it.
- **Overdue never folds** into Waiting or Hidden until (owner rule, sign-off log).
- **Proto (`boards.proto` only, D832):** a new `TodoService` (ListTodo, CreateTodo, UpdateTodo, NextTodo, Get/DismissTodoMigration) with `TodoItem` = `BoardItem` + `TodoFields` (key, state, due, effort, impact, tags, steps, hide_until, pin_order, suggested, start, review, closed_at). Adding fields to `BoardItem` broke ~60 struct literals in other lanes' files, so it was reverted for this additive shape. `BoardColumn` stays in `common.proto`, unrenumbered; migration reads it into `TodoStart`.
- **Daemon (`svc_board`), held until Gemini's edit there is committed:** store the new fields; derive `state` from `column` for old rows (Backlog/To do → Open, In progress/In review → In progress, Done → Closed); migration writes start_rank from (column, board, rank); VIP → pinned; archived sessions' items migrate closed; MCP `todo_next`, `todo_close` (an ask), `todo_add` (suggested until Yes).
- **Features `features/todo` (new, TCA):** state = items + filters + selection + folds + quick-add draft; actions for Yes/No, Pin, Close/Reopen, Hide until, Do next, quick add. Replaces `features/board`'s Mode/Filter/ChannelView once wired (seam lines for main).
- **UI:** `ui/workspace/src/todo.rs` (page) and `todo_panel.rs` (session panel), replacing `board.rs`/`panel_board.rs` when main wires them.
- **Goldens:** `desktop/tests/design/todo.rs` (+ `todo_more.rs`) drawn from the frames; 21–24 replaced, new frames numbered 180+. The global sidebar rename (Board → To-do, Inbox → Agent inbox) touches every frame's sidebar, so it is batched with lane INBOX's rename in one re-record (D831).

### Tasks
| # | Task | Files | Tests | Worker | Status |
|---|---|---|---|---|---|
| T1 | Proto fields | `api/proto/humata/v1/boards.proto` | build `//apps/humata2/api/...` boards targets | manager | — |
| T2 | Domain crate | `apps/humata2/todo/**` (new) | `//apps/humata2/todo:todo_test` | Sonnet medium | — |
| T3 | Goldens: list frames (b1-1/2/3, b4-2, b5-2, b5-3, b8-1/2/3, b9-1) | `desktop/tests/design/todo.rs` (new) | design_test candidates | Sonnet medium | — |
| T4 | Goldens: other frames (b4-3, b6-3, b7-1/2/3, b9-3) | `desktop/tests/design/todo_more.rs` (new) | design_test candidates | Sonnet medium | after T3 |
| T5 | Features reducer | `features/todo/**` (new) | TestStore tests | Sonnet medium | after T2 |
| T6 | UI page + panel | `ui/workspace/src/todo*.rs` (new) | native frames | Sonnet medium | after T5 |
| T7 | Daemon + migration + MCP | `daemon/svc_board/**` | away_board profile | — | held for Gemini |

### Claims
`api/proto/humata/v1/boards.proto`; `apps/humata2/todo/**` (new); `features/todo/**` (new); `features/board/**`; `ui/workspace/src/{board,panel_board,todo,todo_panel}.rs`; `desktop/tests/design/{board,todo,todo_more}.rs`; additive lines in `desktop/tests/design.rs`; goldens 07, 12, 21–24, 31, 37, 122 and 180+; `daemon/svc_board/**` after hand-over.

### Log
- 2026-11-19: plan written. Main: Gemini's `svc_board` edit not yet handed over; T7 held.
- 2026-11-19: T1 first tried new fields on BoardItem; dependents failed (missing field in literals); reverted by hand and redone as TodoService (D832). Workers: T2 (Sonnet, domain crate), T3 (Sonnet, list goldens), T4 (Sonnet, session/migrate goldens) started. Main: draw frames in INBOX's WIDE registry at 1600x950; forwarded to T3/T4. Decisions D830–D834 logged.
- 2026-11-19: T1 green: `board_rust`, `grpc:all`, `svc_board`, `humatad2`, `clients/live`, `cli`, `svc_mcp` build. T2 green: `//apps/humata2/todo:all` (6 test targets). INBOX (D846): To-do asks stay out of the Agent inbox via `ListRequestsRequest.agent_inbox = 4`; Needs attention reads `BoardItem.pending`; Yes/No answers with `InboxService.AnswerRequest`. T5 (features/todo + TodoClient + FakeTodo) started.
- 2026-11-19: T5 green: `clients/src/todo.rs` (TodoClient, Entry with revision and ask id), `clients/src/todo/fake.rs` (FakeTodo, frame b1-1 fixture), `features/todo` (reducer, 13 TestStore tests). Clients seam (api.rs module, lib.rs block, FAKE_SRCS) approved by main. `Clients.todo` field and features/app wiring left as seam lines. T4 rendered 188–191 as WIDE_CANDIDATES; their sidebar waits on T3's to-do flag. design_test build break from todo.rs fixed. The CLI break was INBOX's `agent_inbox` field, reported to main. T6 (ui/workspace todo.rs, todo_panel.rs) started.
- 2026-11-19: Main: Gemini's svc_board edit is committed (b765f4a0), so svc_board is ours. T7 started (TodoService impl, journaled fields, migration, unit tests; serve.rs, MCP and tools.rs as seam lines). Fable ruled that wide mockup frames use INBOX's `design/mock_sidebar.rs`, so T3 is switching todo.rs and todo_more.rs to it and dropping the `Side.todo` flag. The ui/changes break (DIFFUI) is fixed. T6's todo.rs compiles.
- 2026-11-19: Fable 5.2, round 1 on the 16 To-do candidates. 184 (empty) APPROVED; 185 (bare list) APPROVED, to be re-rendered after the shared fixes. 21–24, 180–183 and 186–191 REJECTED, mostly for row and chip density, the missing Suggested second line, a solid Show closed chip, hairlines under headers, missing row keys with the drawer open, and session-view chrome. Wording and order matched. T3 is applying the fix lists (kept in scratchpad/verdicts) for round 2.
- 2026-11-19: T7 green: TodoService in `daemon/svc_board` (`src/todo.rs`, `src/todo_fields.rs`, `src/test_support.rs`, lib.rs wiring), 53 unit tests in `svc_board_unit_test`; `humatad2`, `cli`, `svc_mcp` build. Fields in a journaled sidecar, migration runs once (D835–D838). Seam lines to main: serve.rs `TodoServiceServer` registration and the start-up `migrate_boards` call. Next: MCP todo tools and a live TodoClient (T8).

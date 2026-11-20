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
- 2026-11-19: Main committed TodoService (svc_board), boards.proto, D835–D838. Main approved additive tools.rs variants. T8 (live TodoClient in clients/live_daemon) and T9 (MCP todo_next/todo_add/todo_close with agent-access switches) started. Final seam set (serve.rs, AppState, Clients, Shell) goes to main after T6 round 2.
- 2026-11-19: T6 round 2 green (40/40 tests across todo, features/todo, clients, ui/workspace; full desktop build): ask detail (evidence, affects, change, asked-ago, ended session), description + History link, search, more filters with count footer, inline What changed, channel placeholder, Show all paging, check circle closes. New seam actions: AskWhy, OpenSession, ShowHistory. Fable round 2 (verdicts-r2): 21, 181 APPROVED (184 from round 1); the other 13 REJECTED mainly for a 2px row pitch (55 vs 53), check-circle size and text x, plus per-frame lists. T3 on round 3. design_test failures at this point (01, 02, 11, 12, 15–17, 30, 50–53, 62, 65, 84–86 and unrecorded 122–124) come from DIFFUI/INBOX work in flight, not from To-do code.
- 2026-11-19: T9 green: MCP `todo_next`, `todo_add`, `todo_close` (daemon/mcp tools.rs + tests/tools.txt; svc_mcp/src/mcp/todo.rs, mcp.rs dispatch, boards.rs helpers pub(super)); 9 test targets in mcp + svc_mcp pass; humatad2 and cli build. Own Settings switches need capabilities.proto + access crate (seam, D839).
- 2026-11-19: Fable round 3 (verdicts-r3): 181, 184, 185, 189 APPROVED; 21 and 12 others REJECTED for small items plus two regressions (fold-row gap, rail "Agent inbox" on one line). T3 added `sidebar_pr_reviews_open` to mock_sidebar.rs (additive, approved by main; `sidebar()` output unchanged, design_test passes). T10 (To-do access switches) started. T3 on round 4.
- 2026-11-19: T10 green: CAPABILITY_TODO_SUGGEST = 72 (Default Allow) and CAPABILITY_TODO_CLOSE = 73 (Ask me) in capabilities.proto and daemon/access; capability_name arms in mcp tools.rs; svc_board checks them for agent creates/closes on top of the board rules (4 unit tests: default close waits for approval, Allow closes, Never refuses suggest, the owner's close is unaffected). Compiler-flagged arms in humatad guard.rs, access_service.rs and svc_inbox inbox.rs. Settings rows come from Capability::ALL on the daemon; the fake ROWS fixture (feeds channel-tabs goldens) left alone. access, svc_board, mcp, svc_mcp, svc_inbox pass; humatad2, cli, desktop build.
- 2026-11-19: Main committed T10 (b65cb113). T8 green: `clients/live_daemon/src/todo.rs` (`LiveTodo` over TodoService; `live_daemon` unit tests pass, `clients/live:all` and `desktop:humata2` build); two live seam lines to main. Main approved the fixture rows, so `todo_suggest` (Allow) and `todo_close` (Ask) are in `clients/src/fixture/rules.rs` ROWS; clients/settings/app access tests pass. The channel-tabs golden re-record is queued with T3's next record, through main. T3 round 4 rendered (design_test passes, nothing recorded); Fable round 5 judging all 16.
- 2026-11-19: Main committed T8 and the fixture rows (490d7049) and asked the lane to apply seam parts 2–5 itself; T11 is wiring Route::Todo, Clients/Fakes.todo, the live client and the shell page and panel, test first. serve.rs part 1 goes to INBOX. The nav route flip is prepared but stays unapplied until INBOX's label edit (D834) is in. Decisions D840 (To-do capabilities) and D841 (list view details) logged. Fable round 5 (verdicts-r5): 189 APPROVED; the other 15 REJECTED, mostly for one regression (the Do next suffix's dot gaps widened the button 8px and pushed the toolbar left), plus the folded chevron, the session rail pitch of 54, and small per-frame items (180 headers, 183 Add gap, 184 disabled button, 190 muted text, 191 field block). T3 is on round 5.
- 2026-11-19: Main committed the T11 seam: `Route::Todo` (Load, then Watch), `Clients.todo` and `Fakes.todo`, `LiveTodo` in desktop/src/live.rs, the shell's todo page, the palette's "Go to To-do", and the app handling AskWhy, OpenSession and ShowHistory, with a fail-first test in `features/app/tests/todo_route.rs`. parity_comments failed because the review_remote test daemon never served BoardService. `svc_board` was added to `humatad2_test_review_remote` and its `_faults` twin, and SPLIT_CAPABILITY_MATRIX.md was updated; parity_comments, live_review and live_diff now pass. pr_review_remote was already failing without that change; main routed it to INBOX's rig fixes. Fable round 6 approved 13 frames. In round 7, 183 and 190 were approved and 191 was rejected for its field and steps blocks; the shared chip-gap change half-landed, so 7 of the 13 were re-rejected. T3 applied the round-7 batch (6px chip gap, name-to-count +0.5, the 191 positions, 190 padding); Fable round 8 is judging all 16.
- 2026-11-19: Fable rounds 8–10. In round 8 the selected chip came out 3px narrow (its medium-weight label runs short) and 191's field block overshot. In round 9, padding by label overshot the "All N" chips and 191's step pitch widened. Round 10 (verdicts-r10): all 16 To-do frames APPROVED (21–24, 180–191), with every edge within 2px and nothing else moved. The selected chip's right padding is a lookup by label (All 10, payments-rewrite 12, pr-reviews and No channel 11, others 10). Waiting for main's go-ahead before T3 records them and moves them from WIDE_CANDIDATES to WIDE.
- 2026-11-19: Closed the four To-do gaps left after a9b64fe4 (test first, reviewed by a `pro` subagent):
  - **Gap 1 (Typed due dates):** Added `parse_day` and explicit `ParseDayError` (`Empty`, `InvalidDate`, `Unrecognized`) in `apps/humata2/todo/src/day.rs` supporting `"today"`, `"tomorrow"`, `"next week"`, weekday names (`"fri"`), month+day (`"nov 28"`, `"28 nov"`, optional 4-digit year with rollover), and ISO (`"2026-12-01"`). Added `TodoAction::SetDueText`, `EditField::Due`, and `TodoState::due_error` in `features/todo/src/lib.rs`, plus clickable due-date inline editing and error display in `ui/workspace/src/todo.rs`.
  - **Gap 2 (Agent questions `ask_detail`):** Updated `BoardSvc::domain_item` and `ranked()` in `daemon/svc_board/src/todo.rs` to fill `ask.kind`, `ask.by`, and `ask_detail` (`evidence` from `InboxRequest.facts`, `affects` from `InboxRequest.affects`, `change` from `BoardChangePreview`, `asked_minutes_ago` from `created_at`, and `session_ended` from archived sessions / `WithdrawReason`).
  - **Gap 3 (Evidence links):** Added `TodoAction::OpenEvidence(String)` no-op reducer action in `features/todo/src/lib.rs`, and rendered each linked `Evidence` item as a clickable link in `ui/workspace/src/todo.rs` `ask_card` (with `evidence_links` helper).
  - **Gap 4 (Session "None yet" filter):** Added `SessionFilter::NoneYet` in `apps/humata2/todo/src/filter.rs`, `SessionPick::NoneYet` in `features/todo/src/lib.rs`, and the `"None yet"` chip in `ui/workspace/src/todo.rs` `filter_popover` (and counted `session.is_some()` in the `Filters · N` badge).
  - **Fail-first tests observed before fixes:**
    - `//apps/humata2/todo:quick_add_test` (`parses_relative_month_day_and_iso_dates_and_rejects_unreadable_input`): failed to build (`E0425: cannot find function parse_day`).
    - `//apps/humata2/todo:filter_test` (`session_filter_none_yet_keeps_only_items_with_no_session`): failed to build (`E0433: cannot find type SessionFilter`).
    - `//apps/humata2/features/todo:todo_test` (`typing_a_due_date_parses_it_and_bad_input_records_an_error`, `opening_an_evidence_link_changes_nothing_here`, `session_none_yet_filter_keeps_only_items_with_no_session`): failed to build (missing `SetDueText`, `OpenEvidence`, `SessionPick`).
    - `//apps/humata2/ui/workspace:workspace_unit_test` (`evidence_links_produce_open_evidence_actions`): failed to build (`E0425: cannot find function evidence_links`, `E0599: no variant OpenEvidence`).
    - `//apps/humata2/daemon/svc_board:svc_board_unit_test` (`domain_item_fills_ask_detail_from_the_inbox_request`): FAILED in 2.8s (exit 101: empty `ask_detail`).
  - **Targets passed (both `--config=fast` and default `--config=lint`):** 42/42 test targets in `//apps/humata2/todo:all`, `//apps/humata2/features/todo:all`, `//apps/humata2/daemon/svc_board:all`, `//apps/humata2/clients:all`, and `//apps/humata2/ui/workspace:all`.
  - **Left undone (outside allowed file set):** `apps/humata2/features/app/src/lib.rs` (around line 1157) has no explicit `AppAction::Todo(TodoAction::OpenEvidence(_))` arm yet; currently falls through to `Todo`'s no-op reducer until app-side link opening is wired.
- 2026-11-19 (minion BOARDSIMPLE, resumed after 0d2d50d0): what landed: T1–T11 and the four gaps (c6d66420); goldens 21–24 and 180–191 are recorded in WIDE. Found open toward the exit: (a) the shipping daemon never registered `TodoServiceServer` (every live To-do call was Unimplemented); (b) a fresh install showed a "moved 0 items" banner; (c) web evidence links opened nothing; (d) the session panel still draws `panel_board`; (e) Do next only sets `TodoState.do_next`, nothing draws b4-3's prefilled New session; (f) Start the review on a PR review item is not wired; (g) nothing fetches PR review requests from GitHub; (h) the sidebar still routes Board/Inbox and the board UI (kanban, table, dashboard, Scope picker, VIP) is still there.
  - T12 (manager): (a) one registration line in `daemon/humatad/src/wire_board.rs` (seam) + fail-first live test `clients/live_daemon/tests/todo.rs` (`:todo_test`, away_board profile); (b) `clients/live_daemon/src/todo.rs` `notice` hides a move of 0 items; (c) `ui/workspace/src/todo.rs` `opens_evidence` opens http(s) evidence in the browser.
  - T13 (Sonnet): (d)(e)(f) app/UI wiring. T14 (Sonnet): (g) PR review requests via gh into svc_board. (h) waits for main: the sidebar rename re-records every golden's sidebar (D834).
- 2026-11-19 13:40: batch gate-ready. T12 (manager), T13 and T14 (Sonnet) are done. `bq --fast //apps/humata2/app` builds, and 53/53 focused tests pass (log .bq/logs/20261119-133122-457b63.log). The lint run is not done yet. D870 is logged. Still open: (h) the sidebar rename, board UI removal and the golden re-record go through main; the New session sheet links a todo key as a board ItemId (needs checking); live PR rows have no size or checks.
- 2026-11-19 (T15, after Fable rejected 07, 12, panels board-* and 31/37): "To-do" everywhere the Board was named (sidebar row to `Route::Todo`, panel title, rail, full-screen title, View menu, ⌘3); frame 191's "‹ To-do" header while an item is open; Desk sessions use the To-do panel; the fake list ties items to fixture sessions (`FakeTodo::for_workspace`); FakeTodo `insert` tells watchers; design frames 07, 12, 31, 37 and the design sidebar redrawn in the To-do design (D872). Left: deleting `Route::Board`, the board crate and page, the Scope picker and VIP with their tests, which needs a build loop.
- 2026-11-19 (T15 fixes after the re-record): panels board tests failed because the To-do panel rows had no `test_support()`, so `try_find` and the view tree never saw them (they drew). Rows and the back button now carry it. Drawer redrawn at panel width to frame 191 (D872 addendum); PAY-3 fake gets 191's step titles and two-sentence reason; native 12 uses Working; design 07's ask body is readable in dark and Impact takes its own row; channel tab and panel name say "To-do". Native board approve: the inbox lane's fixture turned Ledger's PAY-3 request into a folder move, so the board test drops its approve step (renamed `board_creates_in_selected_column_and_keeps_channel_scope`, flow docs updated).
- 2026-11-19 (Fable round 4, native 07/12): the panel's open item is a column of the scrolling drawer and a fixed History · Pin · Hide until… row with a divider (`todo::footer`, shared with the page drawer, which still ends with it); "Add a step…" stays in the scrolling body under the steps; in the panel Clear is an × inside the date field so Today · Tomorrow · Next week keep one line.
- 2026-11-19 (D874 batch, uncommitted): `Route::Board`, `features/board`, the kanban/table/dashboard page, the board panel and Desk lanes, and the Scope picker are deleted with their tests; saved board routes restore as To-do; a channel's To-do tab is the To-do list narrowed to the channel; board-ask approve coverage is back as `desktop/tests/native/todo_asks.rs`. Goldens 21–24 (design and native) stay in the tree for the owner to decide; no test reads them. Open, handed to Gemini (not started here):
  - **Live To-do coverage** lost with `live_boards.rs`: add `desktop/tests/live_flows/todo_asks.rs` (register it in `live_flows.rs`) with (1) an agent's `todo_close`/move ask on a person's item answered Yes and No from the item's To-do drawer (`todo-row <key>`, then `todo-yes`/`todo-no`): the agent's call returns success/failure, the request leaves the list, the item is closed/stays; (2) "Start a session for this item". Note that the To-do drawer has no button for (2) yet: `AppAction::StartNewForItem(ItemId)` exists (`features/app/src/new_session.rs`) but nothing in `ui/workspace/src/todo.rs` sends it, so the button needs adding (frame b4-3, Do next's prefilled New session) before the live test can click it. The old live harness (agent script running the real `humata` CLI from its tab) is in `git show 3c4c7346:apps/humata2/desktop/tests/live_boards.rs`.
  - **Flow docs** `flows/cases/board/01`–`07`, `09`–`14` and `README.md` still describe the kanban, table, dashboard, Scope picker and drawer pickers, and name deleted tests (`live_boards`, `live_flows/board_*`, `native/board.rs`, `features/board` tests). Point them at the To-do screens and at `native/todo_asks.rs`, `features/todo` tests, `features/app/tests/{todo_route,views_revert,presentation}.rs` and the new live file. 08 is already updated.
- 2026-11-19 (Gemini lane):
  - **Drawer "Start a session on this" button:** added `start_session_label` and `todo-start-session` quiet button on the left of the page drawer footer in `ui/workspace/src/todo.rs` (frames b1-2 / b4-2: "Start a session on this" when no running session, "Start another session on this" when running; omitted on suggested items, PR review items, and in the session panel `ui/workspace/src/todo_panel.rs` so frames 07/12/191 stay unchanged). Sends `TodoAction::StartSession(key)` — seam lines provided to main for `features/todo/src/lib.rs`, `features/app/src/lib.rs` (routing to `self.start_new_for_item(state, ItemId::new(key))`), and an optional `state.todo.items` fallback in `features/app/src/new_session.rs`. Added native click test `starting_a_session_from_the_todo_drawer_opens_the_prefilled_new_session_dialog` in `desktop/tests/native/todo_asks.rs` and unit test `start_session_label_tracks_running_sessions` in `ui/workspace/src/todo.rs`.
  - **Live To-do flows:** added `desktop/tests/live_flows/todo_asks.rs` and registered `mod todo_asks;` in `desktop/tests/live_flows.rs` with three real-daemon cases: (1) approving an agent's `humata --json --wait 600 board move <key> done` ask via `todo-row <key>` -> `todo-yes` (agent exits 0, request leaves pending list, item closes); (2) denying via `todo-no` (agent exits 4, request leaves pending list, item stays open); (3) clicking `todo-start-session` opens the prefilled New session sheet, creates the session, attaches it to the item, and moves the item to In progress.
  - **Flow docs:** updated `apps/humata2/flows/cases/board/README.md`, `01`–`07`, and `09`–`14` to describe the To-do screens and point at current tests (`live_flows/todo_asks.rs`, `native/todo_asks.rs`, `features/todo/tests/todo.rs`, `features/app/tests/{todo_route,views_revert,presentation,board_start}.rs`, `todo/tests/*`, `clients/live_daemon/tests/todo.rs`, `daemon/svc_board` and `daemon/svc_mcp` unit tests); left `08-scopes-and-session-panel.md` untouched. Waiting on main to apply the seam lines and run Bazel builds/tests.

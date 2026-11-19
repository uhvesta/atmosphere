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

# BOARDSIMPLE and INBOXHOOKS review: one ranked to-do list, one short Inbox

Judged 2026-11-19 against BOARDSIMPLE.md, SPEC BD-1–BD-16, `boards.proto`, D106/D155/D177/D216/D334, INBOXHOOKS.md "Two inboxes", SPEC IN-1–IN-26, `inbox.proto`, the UXSIMPLE loop and the reference list. Brought in line on 2026-11-19 with the owner's Needs attention model, the names (Agent inbox, To-do), pull-request reviews as to-dos, INBOXHOOKS "Humata's own asks carry enough to decide", and the second opinion in XHIGH_REVIEW.md. Nothing here is built; the owner approves before any change.

## 1. Verdict

Today's board asks a new user to learn 3 board kinds, 4 scopes, 5 columns, 3 views, pinned and VIP. The target in BOARDSIMPLE swings too far the other way: it adds kinds, a need field, custom weighted fields, comments and a blocked state. The simplest model that fits Humata takes the reference list's shape (one calm ranked list, a reason per position, state toggles) and leaves out most of its extras.

## 2. Rulings on the proposed direction

| Proposal | Ruling |
|---|---|
| **Merge** three board kinds into one list with channel and session filters | **Yes.** An item has an optional channel and zero or more sessions. Session boards stop existing. This also closes BD-10 (the open fork question). |
| **Merge** "In review" into "In progress" plus a review request | **Yes, and name it by what it is:** the agent **asks to close**. UXSIMPLE keeps "review" for PR reviews only, so there is no "request review" verb here. |
| **Add** due date | **Yes.** |
| **Add** effort and impact | **Yes**, three sizes each, both optional. **No "need" field**: Pin does that job. |
| **Add** computed rank with reasons | **Yes.** Manual drag order is cut; Pin is the one manual override. |
| **Add** tags | **Yes**, plain names, one set for the whole list. No descriptions, no tag management page yet. |
| **Add** reminders | **Yes, but as "Hide until <date>" on any item**, not as a kind. It never notifies, so it isn't called a reminder. |
| **Add** steps | **Yes.** A checklist in the item. Agents tick steps. |
| **Cut** the Scope picker | **Yes.** |
| **Cut** the dashboard | **Yes.** "Asks for me" and "Session exited" become filters; the done-per-day chart is dropped. |
| **Cut** VIP | **Yes, for items.** VIP on sessions, channels and machines (NV-20) is untouched. |
| **Kanban as an alternate layout** | **Cut it entirely.** A second layout doubles the goldens and keeps the five columns alive. The Table view goes too: the list is the table. |
| **Keep** several sessions per item, Start a session (SE-3), agent edits with Ask me, history | **Yes to all four.** |
| **"Do next"** | **Yes.** It is the list's one primary button and the tie to the session loop. |

### Missing from the proposal
- **Rename the page "To-do".** "Board" no longer describes it. The wire keeps `BoardService`.
- **No kinds.** Task, note and reminder collapse into one item. A note is an item with no effort or impact; a reminder is an item hidden until a date.
- **No blocked state.** "Waiting" is derived: an item waits while something it depends on is still open. This finally gives Depends on (BD-14) a job. "Blocked on something outside" is "Hide until". An overdue item is never folded into Waiting; it stays in the main list with "Waits on …" as its reason.
- **No custom fields, no comments, no saved views.** Tags cover fields for now; the session and history cover comments. Add fields only when the owner misses them.
- **The session's Board panel and the Desk board** are the same list with a filter preset, not their own surfaces.
- **Agent-made items never rank until I say yes.** An item an agent adds shows under Needs attention, marked Suggested, with no key and no place in the ranking. Do next never picks it.
- **Pull-request reviews are items** (section 6).

## 3. The model: seven concepts

1. **Item.** Key, title, description. Lives in one list.
2. **Where.** An optional channel and the sessions working on it. Both are filters.
3. **State.** Open, In progress, Closed. Starting a session moves an item to In progress. Closed items can be reopened from the Closed toggle.
4. **Rank with reasons.** The list orders open items and says why.
5. **Due and Hide until.** A due date pulls an item up; a Hide until date folds it away until then.
6. **Tags and steps.** Free labels to filter by; a checklist inside the item.
7. **Needs attention.** An agent's ask to change an item, answered with Yes or No on the item. Do next starts a session on the top item.

### Fields
| Field | Values | Notes |
|---|---|---|
| Title, description | text | as today |
| Channel | one or none | "No channel" replaces Global |
| Sessions | any number | as BD-16 |
| State | Open / In progress / Closed | |
| Due | a date, optional time | typed as "fri", "next week", "in 3d", or picked |
| Effort | S / M / L | optional |
| Impact | Low / Medium / High | optional |
| Tags | names | made by typing; quick add takes `#bug` |
| Steps | ordered, each ticked or not | row shows 4/5 |
| Waits on | other items | today's Depends on |
| Hide until | a date | folded under "Hidden until a date" until then |
| Pinned | on / off | ranks first |

### Ranking
- Pinned items first (two pinned items order between themselves by the same score), then a score from: how close or overdue the due date is, impact against effort, how long it has been open, and how many items wait on it.
- Each row shows its top reason as a chip and at most one more in words: "High impact for a small job · PAY-9 waits on it". A reason never repeats what the row's due chip already says.
- The order is computed by the daemon and holds still while the list is open; edits animate the row to its new place.
- Needs attention sits on top, then In progress, both unranked. "Waiting on another item" and "Hidden until a date" are folded groups under the list, and that fold is their only control.
- After migration the old order is the starting rank: In progress, then To do, then Backlog, each in the order the owner left it, until dates and impact say otherwise.
- Sort offers Ranked, Due and Newest. Nothing else.

### Due dates and reminders
- Chips: "Overdue · 2 days" in red, "Due today" and "Due tomorrow" in amber, then a plain date.
- Overdue shows only as a chip and a rank reason. No badge on Home or the sidebar (no unfixable detection).
- When a Hide until date arrives, the item rejoins the list at its rank. Nothing is sent to the Inbox and nothing notifies. The daemon pushes the change; nothing polls.

### Views
One list. The header has search (`/`), Sort, Filters and Do next. Under it: a chip per channel and one "Show closed" toggle. Filters leads with Due and Tags; session, effort, impact and "only Needs attention / pull request reviews" sit under More. Each row has one check circle to close it; there is no play or pause icon on a row. Filters that are on show as chips that each remove themselves. The view is kept per window (SE-13).

### Agents and Needs attention
- **Do next** (me): the button names the top open item that isn't waiting. It opens New session filled from that item (SE-3) in the item's channel, with "Skip to" the one after.
- **Do next** (agent): `todo_next` returns the top item the session may take, within its channel, and adds the session to it.
- **An agent's change to a to-do** (close, edit a field, add an item, delete) never blocks it. Its call returns at once with "asked". The item gets an amber **Needs attention** tag ("close?", "new due date?", "add?") and sorts into the Needs attention group as an ordinary row.
- **Opening the item** shows the ask on top of the drawer: the change as a question, the agent's full reason, its evidence (commits, pull request, test run), before → after, what else it affects (items that stop waiting, sessions that keep running), who asked (and "session ended" when it has), then **Yes** and **No**. Ask why and Open session are beside them.
- **Every Humata write tool requires a reason**; a close also requires evidence. A call without them is refused.
- The agent is told the answer in its terminal either way.
- Agents tick steps and edit items they made without asking. Rules are in Settings › Agent access: Allow / Ask / Never, with the place the ask lands named per row.
- Three skills: add, status, do next.

## 4. What each change touches

Field numbers are in `api/proto/humata/v1/boards.proto` unless said. New numbers are the next free ones.

| Change | Proto | SPEC | Migration |
|---|---|---|---|
| One list | `BoardItem.board` = 2 deprecated; `channel` = 3 is the only placement. `BoardRef.session` = 3 (common.proto) refused. `BoardView` 1–4 and `GlobalWithChannels` replaced by `ListFilter` on `ListItemsRequest` = 4 (channels, no_channel, sessions, states, tags, effort, impact, due window, has_ask, text) and `sort` = 5. `CreateItemRequest.board` = 2 deprecated, `channel` = 10. | BD-1, BD-5, BD-6, BD-7 rewritten or deleted; BD-8 and DK-4 become filter presets; BD-10 deleted; BD-11 new header; D216 retired | Session-board items become items in the session's channel with that session added to `sessions`. Global items keep no channel. Keys don't change. |
| States | `column` = 6 deprecated; `state` = 20 (`ItemState`: OPEN, IN_PROGRESS, CLOSED). `done_at` = 18 kept as closed-at. `Dependency.column` = 3 deprecated, `state` = 4. `ListItemsRequest.columns` = 2 and `ColumnCount` replaced by state counts. `MoveItemRequest` 3–4 deprecated; new `SetItemState`. `CreateItemRequest.column` = 4 deprecated. `BoardColumn` (common.proto) deprecated. `HISTORY_KIND_MOVED` reads as a state change. | BD-1, BD-2, BD-3; D334 retired | Backlog and To do → Open. In progress and In review → In progress. Done → Closed. An In review item with no pending ask gets a history line saying where it came from. |
| Rank | `rank` = 7 deprecated, never filled. `position` = 21 and `reasons` = 22 (`RankReason`: kind, text) filled by ListItems. New `NextItem` rpc. | BD-2 (no drag), BD-4 | Manual order is dropped. Old Backlog items start with no impact, so they rank below To do items of the same age. |
| Pin, no VIP | `vip` = 13 deprecated. `ItemPatch.pinned` = 9. | NV-20 (items only) | VIP items become pinned. |
| Due | `due` = 23; `ItemPatch.due` = 10, `clear_due` = 11; `CreateItemRequest.due` = 11 | BD-3, BD-4 | Unset. |
| Effort, impact | `effort` = 24, `impact` = 25; patch 12, 13; create 12, 13 | BD-3, BD-4 | Unset. |
| Tags | `tags` = 26; patch `tags_add` = 14, `tags_remove` = 15; create 14 | BD-3, BD-4, BD-11 | None. |
| Steps | `steps` = 27 (`Step`: id, title, done); patch `step_add` = 16, `step_set` = 17, `step_remove` = 18; create 15. `HISTORY_KIND_STEPS` = 7. | BD-3, BD-4 | None. |
| Hide until | `hidden_until` = 28; patch 19, 20; create 16 | BD-3, new line | Unset. |
| Waiting | `waiting` = 29, derived. `depends_on` = 17 unchanged. | BD-14: still refuses nothing, but now folds the item and Do next skips it. Amends D106. | None. |
| Needs attention | `PendingAsk` kept on the item and gains `evidence` = 6 (repeated links) and `affects` = 7; `reason` = 5 becomes required. A suggested new item is an item in `ITEM_STATE_SUGGESTED`. Writes return `Delivery` "asked" instead of blocking. `REQUEST_KIND_BOARD` = 1 (inbox.proto) no longer listed by InboxService. | BD-15 wording; BD-9 | A pending "move to Done" ask becomes Needs attention · close? on its item. Other pending moves between columns that no longer differ are dropped with a history line. |
| No table, dashboard, kanban | none | BD-12, BD-13 deleted | Saved view "table" or "dashboard" opens the list. |
| Agent access | `CAPABILITY_GLOBAL_BOARD_WRITE` kept | BD-9, IN-17: the "own session board" row becomes "items it made" | Rules carry over; D180's row stays. |

- **Upgrade:** one VIP Inbox message, "Migrate the to-do list", runs the store migration on the host. No mixed-version fallback; deprecated fields are refused when sent.
- **Goldens:** 21 and 22 redrawn; 07, 12 and 37 redrawn as the filtered list; 23 and 24 deleted; 49 reworded.
- **Decisions:** D106 (views, dependencies block nothing), D155 (keys stay; ranks go), D177 (drawer gains editors), D216 (session boards) and D334 (five-column Status menu) are each amended or retired by one new decision.

## 5. Agent inbox and To-do

**Ruling: two places with two plain names, and no third page.** The sidebar's Inbox becomes the **Agent inbox**: agents stopped and waiting on me. Everything about to-dos, agents' asks included, is in **To-do**. The user learns "Agent inbox needs me now; To-do goes at my pace".

| | Agent inbox | To-do |
|---|---|---|
| Holds | moves and copies of files or sessions, asks for new access, agents' questions, Humata's own "Migrate" message | the ranked list; items an agent wants to change, marked Needs attention; pull-request reviews asked of me |
| Raised by | hooks and Humata's MCP/CLI only | Humata's MCP/CLI; GitHub for reviews |
| The agent | is stopped until I answer | carries on |
| Lifetime | gone when answered or when the agent moves on | waits until I answer |
| Answer | **Yes** / **No** (or the question's choices), Ask why | **Yes** / **No** in the item, Ask why |
| Interrupts | badge on Agent inbox and the session's dot; one system notification per ask | never: no badge, no count, no notification |

- **Not in either:** an agent's own permission prompts (shell commands, edits, pushes). The owner is fine with them in the terminal as they are; Humata doesn't repeat them.
- **Never interrupts:** anything in To-do, items coming back from Hide until, overdue items, anything a rule already allows, and anything read off a screen.
- **The Agent inbox is one column of full cards**, with no list-and-detail split. It should rarely hold more than two or three.
- **Every card answers before Yes is pressed:** the ask as a question with names, not IDs; from → to and sizes; the agent's reason and the to-do it is working on; what else it affects, worked out by Humata (files replaced, uncommitted changes lost, running terminals that move along and keep running, what must happen first); whether it can be undone; and the rule that sent it here, with Change this rule.
- **Ask why** holds the request and shows the agent's answer in the card.
- **A session's Agent inbox panel** is the Agent inbox filtered to that session and its forks. **A session's To-do panel** is the list filtered to its channel, its own items first, and an item opens inside the panel.
- **Home** shows a stopped agent under "Stopped until you answer" with Answer. The same agent's to-do ask is one quiet line there, with no button.

### IN-1 to IN-26
| Line | Ruling |
|---|---|
| IN-1 three scopes | **Cut.** One Agent inbox with a chip per channel. |
| IN-2 This session / With its forks | **Cut.** The session panel always includes forks. |
| IN-3, IN-4 list and card | **Cut the split.** One column of full cards, at page and panel width. |
| IN-5 kind icons | **Cut.** The card's title says what it is. |
| IN-6 Allow all / Deny all | **Cut.** |
| IN-7 Waiting / Answered / All | **Cut the switch.** The Agent inbox shows what waits; **History** lists what was answered. |
| IN-8 Allowed without asking | **Moved** into History. |
| IN-9 the card | Rewritten as above. Buttons are **Yes**, **No**, **Ask why**; a note can go with No. |
| IN-10 previews | Board previews leave; file and session previews become "what else it affects". |
| IN-11–IN-13, IN-15 | Keep. IN-15 gets simpler with one scope. |
| IN-14, IN-19, IN-22 Always allow | Keep. It is how the Agent inbox gets shorter. |
| IN-16, IN-18, IN-21, IN-23 | Keep. |
| IN-17 rules | Rows are Allow / Ask / Never. To-do rows ask on the item and never stop the agent; file, session and access rows ask in the Agent inbox. |
| IN-20 | Already dropped. |
| IN-24 push and PR through Humata | Keep, as an Agent inbox card. |
| IN-25 question | Keep; each choice is a button, plus a typed answer. |
| IN-26 ask ahead | Keep. |

- **Also cut**, as INBOXHOOKS says: every Inbox item read off a screen. A session that stops for input with no hook gets the tab's bell mark and nothing else.
- **Proto (`inbox.proto`):** `InboxScope` (1–3) and `SessionInbox.with_forks` = 2 deprecated in favour of a filter on `ListRequestsRequest` (channels, sessions); `include_answered` = 2 serves History only; `REQUEST_KIND_BOARD` = 1 stops being raised; requests gain `reason` (required), `affects` and an Ask why exchange.
- **Migration:** pending board requests become Needs attention on their items. Other pending requests stay. The "Migrate" message says what changes, what keeps running and how to restore.
- **Goldens:** 25, 26, 27 and 48 redrawn; 114–116 (screen questions) deleted.

## 6. Pull-request reviews as to-dos
- A pull request where someone asked for my review is an item, fetched gently from GitHub with the token Humata already has. A stack of pull requests is one item.
- The row shows the title, a "Pull request review" chip, the pull requests and the author. Its reason is how long it has waited on me, and it ranks like any item, climbing the longer it waits.
- Opening it shows the author's request, the pull requests with size and checks, who else was asked, and **Start the review**, which opens the PR review session with the stack laid out.
- It closes itself when I submit my review, the request is withdrawn, or the pull requests merge or close. It has no check circle and no editable fields; only Pin, Hide until and Hide apply.
- Proto: `BoardItem.review` = 30 (repos, numbers, author, requested_at), set only by the daemon. No migration.

## 7. Frames
Twenty-seven frames in nine batches of three are in `frames/`, with one line each in `frames/FRAMES.md`: 1 To-do; 2–3 Agent inbox; 4 answering and Do next; 5 Home, quick add and the folded groups; 6 Agent access, dark and migration; 7 pull-request reviews and a suggested item; 8 empty, bare and 60-item lists; 9 the day after migration, the terminal after No, and an ask opened in a session's To-do panel.

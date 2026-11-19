# Second opinion: the simpler to-do list and the two inboxes, batch 1

Fable 5.2, 2026-11-19. Read-only review of `BOARD_REVIEW.md` (the 00:29 version, with section 5 on the two inboxes) and the redrawn frames `b1-1-list.png`, `b1-2-item-open.png` and `b1-3-channel-due-soon.png` (00:28). Frame numbers below are b1-1, b1-2 and b1-3.

**Verdict.** Not ready for the owner. The direction is right and needs no rethink: one list, no Scope picker, no kanban, the sidebar Inbox for what needs the owner now and the To-do page for everything that can wait. The frames are not yet the calm list the owner asked for, and five rulings in the model should change first. That is a redraw of batch 1, not a touch-up.

**Checked:** the three frames and their generator (`frames/src/gen.py`), BOARDSIMPLE.md, INBOXHOOKS.md, UXSIMPLE_REVIEW.md and its frames, goldens 07, 21–26, SPEC BD-1–BD-16 and IN-17, `boards.proto` (every new field number the proposal names is free), D106 and D216.
**Not checked:** SPEC IN-3–IN-26 beyond IN-17; goldens 12, 15, 37, 47 and 48; `inbox.proto` beyond the request kinds; how pins are written today. Contrast figures come from the mockup's CSS colours, not the app's tokens.

## 1. Does it meet the goals?

| Goal | Met? | Why |
|---|---|---|
| Few concepts | No | 33 named things in three frames, about 19 concepts (below). The proposal says seven. |
| Lose board kinds, Scope, five columns, three views, VIP | Mostly | Kinds, Scope, views and VIP are gone. The five columns come back as five toggles labelled "States" (b1-1 top right, b1-3 popover). |
| A calm ranked list with metadata and due dates | Partly | One ranked list with due chips, tags and reasons is there. It is not calm: two-line rows, about sixty pill shapes, four filled blue buttons, and the ranked list starts 520 px down a 950 px window with six rows showing. |
| Two inboxes, minimal interruption | Model yes, drawn half | The placement is right. The Inbox isn't drawn, proposals are painted in the needs-you amber, and Home still carries board asks. |
| Fits the simplified loop | Partly | Do next ties the list to sessions. The frames break two rules the loop review set: one primary per panel, and every control says what it does. |
| No outside project named | Yes | |

### Concepts as drawn

Counting anything with its own name on screen whose behaviour a new user must learn:

1. Item and its key (PAY-3, TD-3)
2. Channel chips and "No channel"
3. From agents: three kinds of proposal, six button labels
4. In progress
5. Next (called Open in the toggle)
6. Waiting on another item
7. Later
8. Closed
9. The Show toggles
10. Rank number
11. Reason chip and reason words
12. Pinned
13. Due chip
14. Effort
15. Impact
16. Tags
17. Steps count
18. Waits on
19. Session chips on an item, with "+ Add"
20. "session exited · Resume"
21. Do next · KEY
22. The play circle
23. The pause circle
24. The check circle
25. Quick add that reads "fri #bug"
26. Sort
27. The Filters popover (Show, Narrow down, eight groups)
28. "Filters on" chips and "Hidden by the due filter"
29. The State control in the drawer
30. History
31. Start another session on this
32. Remind me later
33. Search

Grouped generously this is 19 concepts: item, channel, proposals, states, waiting, later, rank with reasons, pin, due, effort, impact, tags, steps, sessions on an item, do next, quick-add grammar, sort, filters, history. Today's board, grouped the same way, is about 15 (item, board kinds, scope, views, dashboard, columns, sessions on an item, depends on, asks in place, start a session, filter text, pin, VIP, history, drag order). The redesign swaps structure for metadata; it does not reduce the count. The proposal's seven are bundles: its seventh, "Agents", is Do next, ask, propose and ask to close.

A list a new user can read needs about nine on the page (item, channel, From agents, In progress, Next, due, tags, steps, Do next) and four more inside the drawer (effort and impact, waits on, hide until, history).

## 2. Problems in the frames, worst first

### Blocks sign-off

1. **b1-1 and b1-3, the two circles at the left of every row.** In progress rows lead with an orange pause, Next rows with a play, and both then have a check.
   - A pause button beside a live session reads as "stop the agent". Humata's rule is that terminals keep running unless the owner ends them, so this is the wrong control to leave ambiguous. Nothing in the proposal says what it does.
   - The play starts a session and sits where every to-do list puts the tick. A user who clicks the first circle to tick an item off starts an agent instead.
   - The check, the one action a to-do list is for, is the faintest mark on the row (about 1.6:1 on white). The play is the darkest.
2. **b1-1 top right, b1-3 popover, b1-1 bottom: one bucket, three controls, two names.**
   - The Show strip has five toggles. The popover calls the same five "States". The drawer (b1-2) says State is Open, In progress or Closed. The model says three states and the user sees five, which is the old column count.
   - Waiting and Later each have a toggle in the strip, a chip in the popover and a fold at the bottom of the list.
   - On and off are not legible. In one strip, black fill means selected (All), grey fill means not selected (the channel chips), grey fill with a border means on (Open), amber fill means on (In progress) and a dashed outline means off. The popover then uses black for on.
3. **b1-1, the Next rows: the reasons repeat the row and don't explain the order.**
   - Of 14 reason phrases, 10 restate something on the same row: "Overdue by a day" under a red "Overdue · 1 day" chip, "small job" beside an S, "high impact" beside "high". Four add anything: Pinned, "PAY-9 waits on it", and the two ages.
   - Row 6 (small, high impact, due in 25 days) sits below row 4 (small, medium impact, no date) and row 5 (large, high impact, no date). By its own chips it has more going for it than row 4. The frame that is meant to sell "ranked, with reasons" shows an order the reasons can't account for.
4. **b1-1 and b1-2, From agents.**
   - The first row offers a one-click **Close item** on a cut-down reason. The full reason in the drawer ends "Step 5 is yours: the production cut-over", and the steps show 4 of 5. The row hides the sentence that says not to close.
   - In b1-2 the rows drop the reason and the metadata but keep the buttons, so **Add** adds an item sight unseen.
   - Four ways to say no: Keep open, Keep in progress (the drawer, for the same proposal), Dismiss, Keep. "Keep" alone doesn't say what is kept.
   - The group that "never interrupts" is the first thing on the page and holds three filled blue buttons. With Do next that is four primaries in b1-1 and five in b1-2, against the loop review's one per panel.
   - The proposing session's dot, the row's "claude asks to close" and the drawer card are all in the orange and amber Humata uses for "needs you". A calm proposal looks exactly like golden 22's blocking ask.
   - Nothing caps the group. Twelve proposals push the list off the screen.
5. **b1-3, the Filters popover.** Today's board has one Filter text field. This has 31 chips in eight groups for an 11-item list.
   - Channel is in the chip row and in the popover. States are in the strip and in the popover.
   - "Later" appears twice with two meanings: the hidden-until bucket under States, and "due after this week" under Due.
   - "This week" is on, and the list shows an item due Mon 23 Nov when today is Thu 19 Nov. It means the next seven days.
   - The button says "Filters · 2"; one chip is shown as on.
   - "Hidden by the due filter · 1" names PAY-9, which is hidden because Waiting is off. The Waiting fold from b1-1 is gone from this frame.
   - "Agents: Asks for me" is left over from before asks moved to the Inbox and proposals to the top group.
   - Tags as chips stop working at about fifteen tags; channels as chips at about eight channels.

### Should be fixed in the redraw

6. **b1-1, density and hierarchy.** The eye lands on Do next, then the blue proposal buttons, the orange circles, the red chips, and only then a title. Nine item rows fill the window.
7. **b1-2, the drawer reads as an issue tracker.** Eight labelled field rows come before any content; the description is second from the bottom. The proposal, the one thing that wants an answer, is under seven rows of fields.
   - Three ways to close are on screen at once: the State control, the proposal's button and the row's check.
   - The State control offers Open while a session is attached and running. What happens to the session is not said.
   - Pin is called the one manual override, and it is a grey star in the drawer's footer. Remind me later, the second date, sits there too, apart from Due.
   - "Waits on: PAY-2 ✓" labels a finished dependency as something still waited on.
8. **"Open" means three things.** The header's "11 open" counts in progress and waiting items. The toggle's "Open 6" doesn't. The section is called "Next · 6". Then "Keep open" is offered on an item that is In progress.
9. **Do next · INF-6** (all frames, top right). It names a key, not what happens or which item. It duplicates the play on row 1. It follows the filter (PAY-7 in b1-3), and nothing says what it picks under Sort: Due or Newest.
10. **Captions that explain the UI.** "ranked for you; the order holds still while you look", "proposals wait here until you get to them; nothing here notifies you", "their places in the whole list are 2 and 3". Each marks something the layout failed to say. The third exists only because a filtered list shows global rank numbers.
11. **Items are referred to by key where the key is the faintest thing on a row** (about 2.6:1, far right). "proposes a new date for INF-2", "PAY-9 waits on it", "WEB-13 waits on PAY-3" and Do next all make the reader hunt down the right edge.
12. **`#bug` against `# payments-rewrite`.** Quick add uses `#` for a tag. The drawer's Channel row (b1-2), like every Humata screen, uses `#` for a channel.

### Minor

13. Column headers "Effort · Impact" and "Channel" sit on the In progress heading, below the group they also describe.
14. The sidebar is drawn without Humata's icons, the Channels "+", fork marks or the hollow idle circle (it uses a grey dot). Fine for a mockup; the golden must use real chrome.
15. The fixture has a session named "API rate limiter" while the item of that name sits in Next with a play button, which contradicts "starting a session moves an item to In progress".
16. The drawer's expand and close marks are about 12 px, the smallest targets on the page.

### States no frame shows

- An empty list on first run, and what Do next says then.
- A bare item: a title and nothing else. Every drawn row has effort, impact, tags and usually a date. Quick add can't set effort or impact, so bare rows will be the common case.
- The list the day after migration: every item with no date, effort, impact or tag.
- Many: 60 open items, 12 proposals, 12 channels, 30 tags.
- An overdue item that is also Waiting or Later, which today's frames would hide.
- A long title, a narrow window, and the session panel's width (about 420 px at this scale; b1-2's 740 px list has already dropped three columns).
- Closing an item with unticked steps.

## 3. Problems in the model

Rulings I agree with: one list with channel and session as filters; no Scope picker; no kinds; no need field; no custom fields, comments or saved views; the page renamed To-do; "asks to close" in place of In review; VIP cut for items; kanban, table and dashboard all cut; Inbox for now and To-do for later, with no third page.

1. **The computed rank is not predictable enough as specified.** Keep it, since the owner asked for it, with four changes.
   - *Age pushes up what the owner has been ignoring.* An item nobody wants rises a little every day, so the list differs each morning with no cause the owner can name. Use age only to break ties.
   - *Migration throws away the owner's order.* "Manual order is dropped", and effort and impact migrate unset. The claim that old Backlog items then rank below To do items has nothing behind it: both arrive with nothing set, so day one is ordered by age. Carry the old order over as the tie-break (To do before Backlog, then the old `rank`), and keep `rank` = 7 rather than deprecating it.
   - *Pin is on or off.* With three pinned items there is no way to put one before another. Let pinned items be dragged among themselves, and let a drag into them pin. That is one rule ("the list ranks itself; what I pin stays where I put it") and it reuses a field that already works.
   - *A reason must explain a position relative to its neighbours.* Show a reason on a row only when the row doesn't already say it, and give the full "why here" in the drawer.
2. **A derived Waiting is right; hiding it by default with no exception is not.**
   - An item due today that waits on something disappears from the list. Keep a waiting item in view once it is overdue or due soon, or say so on the fold ("1 due this week").
   - D106 is owner-settled and says dependencies block nothing. Folding the item and skipping it in Do next changes what the owner agreed to, so name that in the sign-off rather than as a wire note.
   - It is not a state. Don't list it under States.
3. **Cutting kanban entirely is right, but it has not actually gone.** With three states a board shows nothing the two sections don't. The five-toggle strip keeps the five buckets alive. Add Proposed and the lifecycle is six stages against today's five columns.
4. **"Remind me" never reminds.** Section 3 now says nothing is sent and nothing notifies when the date comes. A user who sets "Remind me Mon 23 Nov" and doesn't open To-do that day hears nothing. A date on any item is the right shape; the name is wrong. Call it **Hide until**, and use one word for the bucket everywhere. If the owner wants a real reminder, that is a separate, owner-set notification and should be asked as such.
5. **The two inboxes: the placement is right, the edges are open.**
   - *Home and the sidebar still interrupt.* Golden 25's first "Needs you" row is "Asks to move PAY-3 to Done", and its pinned line repeats it. The proposal's golden list leaves 25 out. Proposals must leave Home, the pinned line and the session's orange dot, or the to-do inbox interrupts through them.
   - *Are an agent's new items free or proposed?* Section 2 says scratch to-dos land in the shared list and rank low. Section 3 says a new item is a proposal, then that rules follow IN-17, whose default for adding to the channel is Allow. b1-1 draws it as a proposal. Rule: nothing an agent makes enters the ranked list until the owner presses Add; an agent's working checklist is the steps on its item.
   - *Three states or four?* The States row lists Open, In progress, Closed. The proposals row adds `ITEM_STATE_PROPOSED`.
   - *What does the agent hear?* Its call returned "proposed" and it carried on. Nothing says whether Close item or Dismiss ever reaches its session.
   - *`todo_next` lets an agent take the next item and attach itself with no one asking.* It needs a row in agent access and a default.
   - *Migration of session boards.* D216 hides the items of an archived session's board. Moving every session-board item into its channel would bring those back into the ranked list. Say they stay hidden with their session, or migrate closed.
   - *A proposal is answered in three places* (top group, row, drawer). Keep the top group as the place; the row and drawer should carry the same two labels.
6. **"Asks to close" has no answer for "the agent is done and the rest is mine".** The drawn example is exactly that case, and both buttons are wrong for it: Close loses the owner's step, Keep in progress says an agent is still on it. Either an agent may propose closing only when every step is ticked, or closing with steps left says so on the button ("Close with 1 step left").
7. **In progress is half derived, half manual.** Starting a session sets it. Nothing says what unsets it when the session exits (b1-1's INF-2), or whether an item can be In progress with no session. Decide, then drop the three-way State control for a single Close or Reopen.
8. **Effort and impact** stay, as the owner listed them, but belong in the drawer and the rank. They don't need a list column, two filter groups and half the reason phrases.

## 4. Fixes

### Batch 1, redrawn

| Where | Change |
|---|---|
| Row, left edge | One circle: the check, drawn dark enough to see. No pause. Start is a text button on the selected or hovered row and in the drawer. |
| Next rows | One line. No rank number. A reason chip only when the row doesn't already say it. Keep the two-line row for In progress, where the second line is the sessions. |
| Order in the fixture | Make it follow the stated rule, so the Apple certificate row sits above the two undated rows. |
| Show strip, folds, States in Filters | One control per bucket. I would keep the three folds at the bottom (Waiting, Later, Closed), since they say what is hidden, and delete the strip and the States row. If the strip stays because the owner likes it, delete the folds. |
| From agents | Up to three rows, then "and 9 more". Neutral colour. Outlined buttons, so Do next is the page's one primary. The accept button names the change; the other is always **Dismiss**. Always show the item's title, steps count and the whole reason, or no button. |
| Do next | Say the item: "Start next: Rotate the staging…". State that it is the top-ranked item among those shown, whatever the sort. |
| Filters | Channel chips, search, and a small popover with Due and Tags only. Due reads "Overdue" and "Next 7 days". Drop Effort, Impact, Session and Agents. The count on the button matches the chips shown. |
| Drawer | Title, then the proposal, then description and steps. Then Channel, Sessions, Due, Tags. Effort, Impact, Depends on and Hide until under More. Pin beside the title. One Close or Reopen button. |
| Words | "Open" means one thing. Refer to items by title, with the key after it. Tags get a mark other than `#`, or none. Delete the three captions. |
| Colour | Amber and orange only for what is in the Inbox. |

### What batches 2–4 must show

1. The Inbox and the To-do page for one agent that has both a question and a proposal at the same moment, with Home beside them. This is the test of whether the split can be learned.
2. What the agent's terminal shows after Close item and after Dismiss.
3. The session's To-do panel at its real width. Move it into batch 2: it decides the row design.
4. The list the day after migration, with nothing set on any item, and the order it comes up in.
5. A bare item, in the list and in the drawer.
6. Many: 60 items, 12 proposals, 12 channels, 30 tags.
7. Empty, on first run.
8. An overdue item that is waiting, and a hidden item on the day it comes back.
9. Closing with a step left; the Closed fold with Reopen.
10. Quick add showing how it read the line before it commits, including which channel the item goes to under All.
11. Agent access with Allow, Propose and Never, including taking the next item.
12. Dark.

## 5. Verdict

**Sign-off after fixes, and they are not small.** No rethink of the direction. Before the owner sees it: redraw batch 1 to the table above, and change the five rulings in section 3 (age and migration order in the rank, pin by drag, Waiting kept in view when due, "Hide until" in place of "Remind me", nothing agent-made in the ranked list until Add). The owner cannot sign off the two inboxes on batch 1 alone, because the Inbox, Home and the agent's side of a proposal are not drawn.

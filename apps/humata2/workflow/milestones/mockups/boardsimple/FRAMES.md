# Frames: the simpler To-do and Agent inbox

HTML mockups at 1600x950, not renders of the app. Sources are in `src/` (`sh src/shoot.sh <batch>` rebuilds and screenshots one batch). "Today" in the frames is Thu 19 Nov.

## Batch 1: To-do
- `b1-1-list.png`: I open To-do. I see one list for every channel: on top the 3 items an agent wants to change, as ordinary rows with an amber Needs attention tag; then what is in progress; then everything else ranked, each with its due chip, tags and a reason. A pull request waiting for my review is item 3. Shows: one list instead of three board kinds and five columns; one check circle per row; Do next names the item it would start.
- `b1-2-item-open.png`: I open "Ledger schema v2". I see the ask on top as a question, the agent's full reason, its evidence, the change before → after, what else it affects, who asked, and Yes / No. The item's own fields are below. Shows: I can decide without opening the terminal.
- `b1-3-channel-due-soon.png`: I pick payments-rewrite and filter to overdue or due this week. I see 3 of the channel's 4 items, a chip for the filter that is on, and what the filter leaves out. Shows: Due and Tags lead the filters; the Scope picker is gone.

## Batch 2: Agent inbox
- `b2-1-inbox-empty.png`: Nothing is waiting. I see what would stop an agent and ask me, today's counts, and a pointer to To-do. Shows: the Agent inbox is usually empty.
- `b2-2-file-move.png`: An agent asks to move a folder to another machine. I see from → to by name, the size, its reason, the 2 files that would be replaced, whether it can be undone, and Yes / No / Ask why. Shows: one full card per ask in a single column.
- `b2-3-session-move.png`: An agent asks to move its session to a bigger machine. I see that its 2 running terminals move along and keep running, and what has to happen first. Shows: a session move never ends a terminal.

## Batch 3: Agent inbox, continued
- `b3-1-copy-ask-why.png`: An agent asks to copy 3 files from another session, and I pressed Ask why. I see that my uncommitted changes would be overwritten, and the agent's answer in the card while the copy is on hold. Shows: Ask why is answered in place.
- `b3-2-question.png`: An agent has a question. I see it as the title, what the agent needs to know, and each choice as a button, with a field for my own answer. Shows: a question has no Yes / No.
- `b3-3-session-inbox-panel.png`: In that agent's session, I see its terminal stopped at the question and the same card in the session's Agent inbox panel at real width. Shows: the panel is the Agent inbox filtered to this session.

## Batch 4: answering a to-do ask, and Do next
- `b4-1-after-yes.png`: I said yes to closing "Ledger schema v2". In the agent's session I see its terminal told "The owner said yes", and the To-do panel with the item closed and Reopen. Shows: the agent never waited; it hears the answer where it works.
- `b4-2-field-edit-ask.png`: I open "Postgres 17 upgrade". I see the ask to move its due date, before → after, and that the session that asked has ended. Shows: a field change is the same ask as a close; Ask why becomes "Resume the session to ask why".
- `b4-3-do-next.png`: I press Do next. I see New session filled from the top item: channel, agent, machine, repository and a first message, with "Skip to" the next item. Shows: the list starts the terminal loop in one step, with no folder to pick.

## Batch 5: Home, adding, and the folded groups
- `b5-1-home-across.png`: On Home I see the one agent that is stopped with its question and Answer; its to-do suggestion is a quiet line under it. To-do has one row at the bottom. Shows: only a stopped agent is raised; to-do asks never are.
- `b5-2-quick-add.png`: I type "fix login redirect loop fri #bug web s high". I see it read into a title, a due date, a tag, a channel, an effort and an impact, and where it would rank. Shows: one line adds an item; only the title is needed.
- `b5-3-waiting-hidden-closed.png`: In one channel I unfold Waiting and Hidden until a date and turn on Show closed. I see what each waits on or when it comes back, and this week's closed items with Reopen. Shows: each group has one control; a review item closed itself.

## Batch 6: rules, dark, migration
- `b6-1-agent-access.png`: In Settings I see each kind of action as Allow / Ask / Never, with where an Ask lands: on the item in To-do, or in the Agent inbox where the agent waits. Shows: to-do rules never stop an agent; ending sessions is Never; agents' own prompts aren't repeated.
- `b6-2-dark-notification.png`: The list in dark, with the one kind of notification Humata sends: an agent is stopped on a session move. Shows: the notification says what and opens the Agent inbox; it has no Yes button.
- `b6-3-migrate-message.png`: After updating I see one card: what the boards become, with counts, what keeps running, what goes away, and how to restore. Shows: the migration is one decision with the same detail as any ask.

## Batch 7: pull-request reviews, and a suggested item
- `b7-1-pr-review-item.png`: I open the review item. I see the author's request, the 2 pull requests of the stack with size and checks, who else was asked, and Start the review. Shows: nothing to edit; only Pin, Hide until and Hide.
- `b7-2-pr-review-session.png`: I press Start the review. I see a session in pr-reviews with the stack's 2 layers laid out in Diff and Submit review. Shows: the item is the way in; it closes itself when I submit.
- `b7-3-suggested-item.png`: I open the item an agent suggested. I see "Add this item to your list?", its reason and evidence, where it would rank, and fields I can change before saying yes. Shows: an agent-made item has no rank until I say yes.

## Batch 8: list sizes
- `b8-1-empty.png`: With no items I see the add field and three sentences on how the list fills. Shows: nothing to set up.
- `b8-2-bare-list.png`: With six title-only items I see them oldest first, each with its age as the reason, and one hint. Shows: the list works with no metadata.
- `b8-3-sixty-items.png`: With 60 items I see Needs attention and In progress folded to one line each, the first 12 of 48 ranked, and a strip saying where I am. Shows: the same rows at size; the list scrolls.

## Batch 9: after migration, after No, and the session panel
- `b9-1-day-after-migration.png`: The day after migrating I see my items in the order I left them, each saying where it was ("Was 1st in To do"), and one banner. Shows: the old order is the starting rank; nothing is invented.
- `b9-2-after-no.png`: I said no to the copy, with a note. I see the agent's terminal with my note, its narrower second try allowed by my rule, and both in the session's Agent inbox history. Shows: No carries a note; the agent adapts.
- `b9-3-session-todo-ask.png`: In a session's To-do panel I open the item its agent wants to close. I see the same ask, evidence and Yes / No at panel width, while the agent keeps working beside it. Shows: I can answer without leaving the session.

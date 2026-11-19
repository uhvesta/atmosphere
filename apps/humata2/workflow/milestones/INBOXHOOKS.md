# INBOXHOOKS: Two simple inboxes that interrupt me as little as possible

- **Status:** open (owner, 2026-11-19, restated: screen parsing is optional, "we can do it if you want but don't spend too much effort on it"; earlier: "we don't need to parse the screens for stuff to appear in our inbox… use the hooks and if they fail then that's fine… The Inbox is mainly for stuff that needs my actual attention like risky permission approvals and also the todo list thing")
- **Depends on:** nothing. Fits UXSIMPLE (fewer concepts) and BOARDSIMPLE (to-do asks stay in the Inbox).
- **Lane:** daemon (session_service, inbox) + features/inbox + ui

## Two inboxes (owner, 2026-11-19)
"Simplify our inbox so we get the minimal interruptions and we have maybe 2 types of inbox."
1. **Agent inbox (the name in the app, owner 2026-11-19): needs me now.** It holds one agent's asks in the moment: risky permission approvals and its questions, raised by hooks. It's short-lived: answered or gone when the agent moves on. Low-risk actions are governed by rules and never show up here.
2. **To-do (final name open, see BOARDSIMPLE): the higher level.** It includes PR reviews people requested from me. It's the ranked to-do list (BOARDSIMPLE) plus agents' proposals about it: new items, "propose close", "request review", edits to items I own, and reminders coming due. It's calm and goes at my pace, never interrupting.

Simplify today's Inbox (IN-1 to IN-26) towards that:
- the three scopes (global, channel, session) and the "This session / With its forks" toggle (IN-1, IN-2) become filters on the two inboxes;
- review Waiting / Answered / All (IN-7), Allow all / Deny all (IN-6) and the Allowed without asking log (IN-8) for cutting or hiding;
- each card has one obvious action;
- interruptions: only the agent inbox badges or notifies, and only for risky actions; the to-do inbox never does.

Fable 5.2 proposes the model and renders it to the owner in small batches, together with BOARDSIMPLE's frames, because the to-do inbox and the list are one surface or close to it. Nothing is built before sign-off.

## Humata's own asks carry enough to decide (owner, 2026-11-19)
"When agents invoke the MCPs for like moving files or like changing statuses… it doesn't have enough details for me to review it or approve it."
- **In scope:** asks that go through Humata's own MCP/CLI, such as moving or copying files and sessions, changing a to-do item's status or fields, and creating items. The owner is fine with the agents' own permission prompts (e.g. `rm -rf`) as they are.
- **Why they're thin:** the tool inputs are IDs and a target ("move K-12 to Done", "move session 4f2… to box-2"). The card shows that, but not what the thing is, why, or what else it affects.
- **Fix:**
  - **Every Humata MCP write tool requires a `reason`, and an ask without one is refused.** Status changes also carry evidence: for a close, what was done and links to its commits, PR or test run; for a move, why there.
  - **Cards show names, not IDs:** the item's title, channel and current state, before → after; the file or session's name, size and where it lives now → where it goes.
  - **Cards show what else it affects:** items that depend on it, sessions working on it, running terminals that move with it (they keep running), and files that would be overwritten.
  - **Ask why** holds the request and asks the agent to explain, with the answer shown in the card.
- **Fewer of them:** under the new To-do, an agent's status or field change marks the item **Needs attention** (rule value Ask) and never blocks it. Opening the item shows the reason, evidence and before → after, with Yes / No (signed off 2026-11-19, `mockups/boardsimple/`). Only moves and copies of files or sessions, and access asks, stay in the Agent inbox.
- **Exit:** a live agent's close of a to-do item, a file move and a session move each show a card or suggestion that a person can decide on without opening the terminal. Fable judges.

## Outcome
The Inbox shows only things that need my decision:
- risky permission approvals;
- agents' questions;
- to-do list asks.

Each one arrives through an agent's hook or Humata's own MCP/CLI. Hooks are what we rely on. If a hook isn't there or fails, that's fine: I see the question in the terminal. Screen reading is best effort (below).

## Keep (hook- or API-raised)
- Permission hooks: D191, D192, D193, D673.
- Copilot's ask-user via its `preToolUse` hook: D761.
- Claude Code's AskUserQuestion via its PreToolUse hook: D796.
- Codex's app-server wrapper events: D795.
- Agent access asks (IN-9 to IN-26), board and to-do asks (BD-15), push/PR asks (IN-24).

## Screen reading: best effort only
Hooks are the only path we invest in. Existing screen reads may stay while they work for free, but they get no new work: no new fixes, cases, tests or goldens for them. Any that break or cost upkeep are cut, not fixed. Cut first, because they already cost effort:
- D767: a question read off an agent's screen raises an Inbox question (e.g. Copilot's "Confirm folder trust").
- D781: the Inbox item with the live terminal under it.
- D785: cut-off screen choices.
- D786: growing the holder to 50 rows for a question box. Holders still start at 80x50 unless that was only for this.
- The screen-read "waiting" notices that offer only Dismiss.
- Code to find: `grep -rn "screen" daemon/humatad/src/session_service/` and the Inbox kinds that carry a screen excerpt.

## Decide (Fable 5.2)
- D762's input gate also reads the screen. It refuses typed text while an agent shows a question or when the screen can't be read. Keep it as a safety check (it raises nothing), or cut it too.
- What a screen-only question (e.g. folder trust) looks like without the Inbox. Proposed: the session's tab gets the bell mark (D424) when the agent stops for input, with no Inbox item.

## Exit
- Live flows: a Claude Code and a Copilot permission ask each raise one card through hooks, and with the hook removed, nothing is raised and the session still works.
- Removed decisions are marked Superseded in DECISIONS.md with this milestone named.

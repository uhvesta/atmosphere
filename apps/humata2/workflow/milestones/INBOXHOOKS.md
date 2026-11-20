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

## Lane (INBOX manager, Opus 5.5, 2026-11-19)

**Goal (release-plan step 11):**
- The Agent inbox is one column of full cards, for agents stopped until the owner answers.
- To-do asks mark their item Needs attention in To-do instead.
- Humata's own asks carry the reason, names, before → after, side effects, undo, and the rule with Change.
- Agent access is grouped Allow / Ask / Never rows.
- D785/D786 are cut.

### Design notes
- **Frames.** Wide candidates 200–210 in `desktop/tests/design/agent_inbox{,_more}.rs`, drawn at 1600x950 from boardsimple b2-1, b2-2, b2-3, b3-1, b3-2, b3-3, b4-1, b5-1, b6-1, b6-2 and b9-2.
  - The WIDE registry in design.rs (main's go-ahead, 2026-11-19) draws them at the mockups' size.
  - The sidebar is BOARDSIMPLE's `Side.todo` switch.
  - The old inbox frames 25–27, 48 and 114–116 are retired once the real UI matches.
- **To-do asks are not Agent-inbox cards.**
  - Board/to-do write asks (`BoardChange` and `NewBoardItem` previews) already answer the agent `approval_pending` at once, so the agent carries on.
  - The daemon leaves them out of the Agent inbox's list, its `pending_count` and its notifications.
  - BOARDSIMPLE reads them from `BoardItem.pending` (boards.proto, theirs) and answers them through `InboxService.AnswerRequest`.
- **Card detail (inbox.proto only, never common.proto).** `InboxRequest` gains:
  - `facts` (label/value rows: From, To, Size, Carries, Files);
  - `affects` (lines with a bold lead and a warn/ok mark, worked out by Humata);
  - `undo_text`;
  - `working_on` (the item the session is on);
  - `asked_why` (the owner's Ask why and the agent's reply);
  - `rule_label` (the Agent access row's name).
- **Ask why:** `InboxService.AskWhy(id)` holds the card and tells the agent the owner asked why. The agent's reply arrives through MCP `inbox.explain` and is shown in the card's blue "You asked why" panel.
- **Missing pieces to fix:**
  - the cross-machine copy card drops its preview (`wire_transfers.rs` `TransferInboxAdapter::ask` ignores the board ask);
  - `MovePreview` is never filled;
  - `MoveSessionRequest` has no reason;
  - the IN-8 counts ("2 answered · 31 actions followed your rules") have no source.
- **Agent access rows → capabilities.** Each b6-1 row maps to one or more capability rules. A row's setting writes all of them, and the notes come from the setting.

### Tasks
| # | Task | Files | Tests | Worker | Status |
|---|---|---|---|---|---|
| I1 | Design frames 200–210 | `desktop/tests/design/agent_inbox{,_more}.rs` | design_test candidates, Fable judges | 2 Sonnet | running |
| I2 | Card detail proto + daemon fill (file/session move, copy, fork) + transfer preview fix | `api/proto/humata/v1/inbox.proto`, `svc_inbox`, `svc_transfers` adapter, session move | svc_inbox unit, transfers profile | — | next |
| I3 | To-do asks out of the Agent inbox list/count/notify | `svc_inbox` | svc_inbox unit | — | next |
| I4 | Ask why (RPC + MCP explain) | inbox.proto, `svc_inbox`, `svc_mcp` | svc_inbox unit | — | after I2 |
| I5 | Cut D785/D786 | `svc_inbox/src/screen_ask.rs`, `inbox/permission.rs`, screen_questions tests | screen_questions_test | — | next |
| I6 | Model + features/inbox: one column, chips, History, Ask why, Always allow for this session | `features/inbox/**`, model.rs seam lines for main | TestStore tests | — | after I2 |
| I7 | UI: inbox page, request card, session panel, Home stopped group, sidebar rename | `ui/workspace/src/{inbox_page,request_card,panel_inbox}.rs` | native frames | — | after I1, I6 |
| I8 | Settings › Agent access grouped rows | `ui/settings`, features settings | TestStore + native | — | after I1 |

### Claims
- `daemon/agent_hooks/**`;
- `daemon/svc_inbox/**`, and `daemon/svc_mcp/**` ask paths;
- `api/proto/humata/v1/inbox.proto`;
- `features/inbox/**`;
- `ui/workspace/src/{inbox_page,request_card,panel_inbox,inbox_terminal}.rs`;
- Agent access in `ui/settings`;
- `desktop/tests/design/agent_inbox{,_more}.rs` and frames 200–210;
- additive lines in `desktop/tests/design.rs`.

### Log
- 2026-11-19:
  - Plan written.
  - The WIDE registry was added to design.rs (main's go-ahead).
  - Fable 5.2 ruled on the screen-read cut: D845 (screen dialogs are terminal-only, needs-you hold; D762 kept and frozen; 80x50 kept). D846 and D847 logged.
  - AGENTS2 follow-up: `hooks` and `screen_questions` moved from the power_cut to the catalog test profile. The power_cut profile lacks AgentService.
  - I3 done: `ListRequestsRequest.agent_inbox` leaves out to-do asks (`inbox/agent_inbox.rs`, unit-tested). Every literal sets it; live_daemon keeps `false` until To-do shows Needs attention.
  - I5 done (D845):
    - screen_ask now only holds NEEDS_INPUT with no card; the grow, the cut-off handling and the card typing are gone.
    - `screen_questions`, `ready_gate`, `hooks` and `svc_inbox_unit` are green.
  - I1, first pass: Fable approved 205, 208 and 210. 200–204 pass once they use the mockups' own sidebar (ruling A, a shared `mock_sidebar.rs`). 206, 207 and 209 were rejected over bold titles, a clipped line, and 209's own To-do list. A worker is fixing them.
  - The codex wrapper follows the app-server socket's symlink, which real Codex makes.
  - I1 second pass: Fable approved 200–204, which are now in WIDE and recorded. Only those five goldens were written; the others were restored. 205–210 were rejected for vertical rhythm, the session chrome's icon colours, tab borders and spacing, and 209's chip colours. A worker is fixing them.
  - I2 done:
    - Copy asks get From/To/Carries rows, affects and undo lines (`inbox/facts.rs`).
    - Every deferred card gets its rule name and "Working on".
    - The copy undo line says it is undone by hand, because transfers has no undo (D847).
    - Session-move and file-read previews are still not built.
  - `inbox_copy` moved from the transfers profile to the power_cut profile, because transfers lacks inbox and channel relay.
  - Rig root cause: after the split nobody set `peer_dial`, the relay guard was dropped, and ConnId/PeerFacts were missing, so BeginRelay was refused. All three are fixed in humatad wire_machines, serve, wire_peer and identity, and svc_peer ports. The stop stubs, the fork_remote mcp routing, the move_cheap hold and fork restart recovery are in progress.

### Batch 2 (minion INBOXHOOKS, 2026-11-19 13:20, after LIVEHOLDER's 64aa343a)
- Checked: D864 (AGENTS2, 8aebb01a) already catches Claude Code's auto-mode offer in the ASKED table and the input gate; nothing to add. D767/D781/D785/D786 are already marked Superseded by D845.
- Found: the MCP `session_move` tool has a schema but no handler (`not_built`), and agents have no file-move MCP tool; a file move reaches the Inbox through `TransferService` (CLI) with a CrossMachineCopy preview that always said "copied, not moved".
- Proto seam (manager, additive): inbox.proto `AskWhy` + `ExplainAsk` RPCs, `WaitResponse.asked_why`, `MovePreview` fields 5–11, `CrossMachineCopyPreview.moves`/`replaced` + `ReplacedFile`; sessions.proto `MoveSessionRequest.reason`.
- Workers:
  - W-A (Sonnet): frames 205–210 second-pass fixes, `desktop/tests/design/agent_inbox_more.rs` (+ `mock_sidebar.rs`). Fable judges.
  - W-B (Sonnet): daemon. Session-move card (MovePreview filled, reason), MCP `session_move`, transfer move/replaced facts, reason required on every asking MCP tool, Ask why (svc_inbox + MCP `inbox_explain`).
  - W-D (Sonnet): app. Client model card detail + AskWhy, features/inbox one column + chips + Ask why + History, ui inbox page/request card/panel per frames 200–205.
  - W-E (Sonnet): Settings › Agent access grouped Allow/Ask/Never rows (frame 208).

#### Batch 2 result (13:45)
- Done:
  - D871
  - Session-move card and reason refusal
  - Move/replace facts on file cards
  - `reason` required in `asks()` (reverted: it broke every asking tool, reads included)
  - AskWhy/ExplainAsk in svc_inbox, with live client `ask_why` wired
  - Agent inbox one-column page, cards per 200–204, chips, History, Ask why, Change this rule
  - Agent access rows
  - Frame fixes for 205–210
- Left:
  - MCP `session_move` handler and `inbox_explain` tool, plus the asked_why hint in the wait reply
  - Refusing an empty-string reason in svc_mcp
  - Forwarding AskWhy/ExplainAsk to the host (svc_away Method registry)
  - File-read preview
  - Home stopped group (207) and notification (209) in the real UI
  - Global Agent access page (settings.rs stub)
  - Recording native goldens 26/27/117
  - Fable judging 205–210
  - Removing dead Bulk reducer code

### Batch 3 (minion INBOXHOOKS, 2026-11-19, after 3c4c7346 / 0.3.9; unbuilt)
- Done: MCP `session_move` handler; `inbox_explain` tool + `asked_why` hint (`Reply::PendingAskedWhy`); central empty-reason refusal (`Operation::requires_reason`); AskWhy/ExplainAsk in svc_away's registry; Home's stopped group (207); frame 209 notification content (`features/inbox/src/notice.rs`); global Agent access page (live via Get/SetDefaultRules); empty Agent inbox text; dead Bulk code removed; "Agent inbox" labels in uxsimple/screen_question/roam/upgrade mocks; request card footer 8 px gap fix.
- Fable: 205, 207, 208, 210 approved; 206 and 209 rejected (check rings, 209 toast height and list rhythm) and fixed in `agent_inbox_more.rs`, to re-judge after a render.
- Left: host-side replay for forwarded AskWhy/ExplainAsk (svc_board Method/ROWS/replay + `svc_inbox` route-up, not this lane's files); the "actions followed your rules" count (IN-8); file-read preview.

### Batch 4 (Gemini lane, 2026-11-19; unbuilt)
- Done:
  - Host-side forwarding of `AskWhy` and `ExplainAsk`: `svc_away::Forwarder::call_as_user` (sends `Principal::User(MachineUser { outside_sessions: true })`); `svc_board` `Method::{AskWhy, ExplainAsk}`, `ROWS`, `BoardInbox::{ask_why_as, explain_as}`, and `replay.rs` `run_forwarded` routing (accepting outside-sessions peer `MachineUser` as `Caller::User(UserVia::Host)` for `AskWhy`, refusing agents and in-session users, and routing `ExplainAsk` for sessions placed on the forwarding machine); `svc_inbox` `ask_why`/`explain_ask` route-up (`ask_why_on_host`, `explain_on_host`) + host entry points (`ask_why_as`, `explain_as`) + explicit user-only guard in `ask_why` allowing `Caller::User(UserVia::Host)` and refusing `Agent`/`Script`/`Peer`. In-process unit tests in `svc_away`, `svc_board`, and `svc_inbox`. Seam lines written for `daemon/humatad/src/wire_board.rs`.
  - File-read preview: `svc_inbox` `deferred.rs` now populates `FileAccessPreview` (resolving repo prefix via `store.session_detail`, source session, source machine, `MODE_READ_THIS_CALL`) and `facts.rs` fills `facts` (`From`, `Files`, `Access`), calm `affects` (resolving source session title/agent and channel name from `Store`), and `undo_text` for every `Call::ReadFiles` ask; added `Inbox::ask_read_reason` so `ReadFileRequest.reason` can flow through via a small seam in `svc_files`/`wire_files.rs`. Unit tests in `svc_inbox` (`facts.rs` and `inbox.rs`).
  - IN-8 ("actions followed your rules" count): added default `InboxGuardPort::allowed_without_asking_today` in `svc_inbox/src/ports.rs`, `humata2_inbox::today_summary` in `features/inbox/src/lib.rs` (tested in `agent_inbox.rs`), and wired `ui/workspace/src/inbox_page.rs` to call `today_summary(answered, 0)`. Designed full end-to-end seam lines for `inbox.proto`, `guard.rs`, `wire_inbox.rs`, `svc_inbox/src/inbox.rs`, `clients/src/workspace.rs`, `clients/live_daemon/src/{inbox,daemon}.rs`, and the 1-line swap in `inbox_page.rs`.
- Left:
  - Apply seam lines (`wire_board.rs`; IN-8 seams across `inbox.proto`/`guard.rs`/`wire_inbox.rs`/`clients`/`inbox_page.rs`; optional `ReadFileRequest.reason` seam in `svc_files`/`wire_files.rs`; D871 Batch 4 note in `DECISIONS.md`).
  - Build and test the focused targets; re-judge frames 206 and 209 with Fable (carried over from Batch 3).



## E2E 0.2.3 2026-11-18

Lane e2, This Mac only, admin twin of the 0.2.3 daemon, Copilot 1.0.114-0. Model seen: claude-haiku-5.5 (events: selectedModel/currentModel/model). Copilot's own events also show gpt-5.4-nano, which looks like an internal helper. Evidence: V/evidence/e2/e2e023/ (V = the verify scratchpad).

### Case 4: Discovery and adopt
Steps: made a session, deleted its channel (the plan has no keep-on-Desk option, and the agents stop); made an orphan with `humata shell <machine>` (tab with session=null, channel=null); `session running` lists it; tried `attach`; `session end --tab`.
- Discovery: PASS. The orphan holder is listed with session null and channel null (shell-running.json).
- Adopt and attach: FAIL (GAP). `humata attach` needs a session (cli/src/args.rs:98), so `attach --tab <tab>` on a session-less tab says `no session is named`. No adopt command exists (the daemon RPC is machine_service.rs:1975). A fix is in flight.
- End of the orphan via `session end --tab`: PASS (orphan-end.json).
Result: PARTIAL, 1/3.

### Case 5: per-agent "Ask in the Inbox"
- Switch on: a Copilot bash request reached the Inbox (c5-inbox-on.json) and approving it ran the command. PASS.
- Switch off: `agents.toml` `[agents.copilot] asks_in_inbox=false`, read without a restart. The next request stayed in Copilot's own dialog and the Inbox showed 0 pending (c5-inbox-off.json, c5-switch-off-screen.txt). PASS.
- Humata's own request: `humata access ask-ahead network_reach network:example.com:443` run by the agent still reached the Inbox with the switch off (c5-ask-ahead-inbox.json). PASS.
- GAP: the switch can't be set from the CLI (`settings set agents.permissions_in_agent` is rejected, and `agents edit` has no flag). I wrote the lane's agents.toml directly.
Result: PASS 3/3, with the CLI GAP.

### Case 6: Inbox terminal
- Trust prompt: the item carries on_screen {tab:0} and kind question. PASS.
- Size: no size in `session tabs --json`, so I measured the pty with `stty -f`. The tab is 24x80. A writer `humata attach` resizes it (to 50x150, 40x120, 30x90) and the size stays after detach. `--read-only` attach doesn't resize. The "no resize" behaviour is app-side and can't be exercised through the CLI. PARTIAL.
- Answer in the terminal: pressing Enter on the attach made the item withdrawn (c6b-inbox-after.json). PASS.
- Dismiss (`inbox deny`): the screen was byte-identical before and after (c6-before-dismiss.txt, c6-after-dismiss.txt) and the item was denied. PASS.
- BUG: at the tab's own 24x80, Copilot clips the box to "1. Yes", so the card offers only ["Yes"]. After a resize it is withdrawn and re-raised under a new id with all 3 choices. Root cause: daemon/humatad/src/screen_ask.rs:502-540 parses only the visible rows and ignores the "pgup/pgdn to scroll" footer. Fix: don't raise a card whose list is clipped. Test: a screen_questions.rs case with a 24-row clipped box.
Result: PASS 3/4 checks, with 1 PARTIAL and 1 BUG.

| case | result |
|---|---|
| 4 Discovery and adopt | PARTIAL (discovery and end pass; adopt and attach GAP) |
| 5 Ask in the Inbox switch | PASS (CLI setter GAP) |
| 6 Inbox terminal | PARTIAL (BUG: only "Yes" offered; CLI cannot show no-resize) |

Findings (new)
1. GAP: no attach to a session-less tab, no adopt command (cli/src/args.rs:98).
2. GAP: the per-agent switch has no CLI setter (`agents edit` has no asks_in_inbox flag; agent_switches::set exists).
3. BUG: the screen card offers only "Yes" on a clipped 24-row box (screen_ask.rs:502-540).
4. UX: writer attach resizes the tab's pty for good, and `session tabs` shows no size.

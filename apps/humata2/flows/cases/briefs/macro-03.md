# Brief macro-03: On call, I open the on-call channel, paste into a desk and make a session
Output: `apps/humata2/flows/cases/macro/03-on-call-shift.md`

Scenario: I am on call. A page fires at 02:10. I open the pinned "oncall" channel, paste the alert text and a screenshot into a desk session to triage, then start a channel session on box-1 (which has the prod tools) to dig in, answer two permission asks from my Inbox, and at the end of the shift Stop all sessions… on box-1.

Steps: pinned channel in the sidebar; a desk session for triage; paste of text and image; new session on a chosen machine; Inbox shows the agent's permission asks with Allow / Deny / Always / Not now; Home "needs you"; Stop.

Branches: box-1 offline (start refused, says offline); an ask arrives while I am typing (no focus steal); I deny with a note; the agent asks to read another session's turns (rule asks); the shift handover: I fork the session for the next person? (who? single-user app — say so); Stop with a terminal attached.

Pointers: features/app/tests/home_needs_you.rs, inbox_flow.rs, permission_alerts.rs, marks.rs (pin); desktop/tests/live_inbox.rs (real daemon); daemon/humatad/tests/hooks.rs, inbox.rs, stop_box.rs, stop_sessions.rs; features/hosts/tests/stop.rs.

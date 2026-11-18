
## E2E 0.2.3 2026-11-18

Agent: copilot 1.0.114-0, model seen: claude-haiku-5.5 (events.jsonl, evidence model-s1.txt). Machine: This Mac. Lane e1, admin twin daemon of 0.2.3. Evidence: V/evidence/e1/e2e023/ (V = verify scratch dir).

### Case 1 Stop and resume: PASS
1. Started s1, typed "remember PINEAPPLE42"; agent answered OK.
2. `session stop`: holder pid 23311 and its group gone (ps), record Stopped, `session running` lists it stopped:true.
3. `session resume`: new holder 25706; follow-up "what was the secret word" answered PINEAPPLE42.
4. Shell (machine shell tab): stop gone by ps, listed stopped; daemon restart did not revive it (pid 0 stopped:true); resume gave a new shell pid; end --tab removed it.

### Case 2 End for good: FAIL (2 findings)
- end --yes archived s2, pid and group gone. PASS.
- F1 BUG: an ended (archived) session can be resumed and its agent starts. Root cause daemon/humatad/src/session_service.rs:1018-1075 (`resume`) has no archive check. Fix: refuse when record.archive is set. Test: end, resume, expect error and no holder. Evidence s2-end.json, s2-resume-after-end.json. `session stop` on the stray agent reached it and removed it (s2-stop-archived.json); status stayed archived.
- F2 GAP: Undo as Stopped is not reachable by CLI. cli/src/commands/sessions.rs:332-337 hardcodes `stopped: false`; the daemon honors it (archive.rs:772). `session restore` brought s2 back running (pid 80734). Fix: add `--stopped`. Evidence s2-restore.json, s2-restore2.json.

### Case 3 Keep running: PASS
Detached (tmux killed), restarted daemon without LANE_FRESH: s1 still running, same pid 25706, listed in `session running`, re-attach showed the same screen with the earlier turns. Minor: `attached` showed false even while my headless attach was open (not verified further).

| case | result |
|---|---|
| 1 Stop and resume | PASS 4/4 |
| 2 End for good | FAIL (1 of 3 checks pass: archived+no process; resume-refusal fails; Undo-as-Stopped untestable) |
| 3 Keep running | PASS 3/3 |

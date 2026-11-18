# Copilot t1: `session type` submits (BUG-T1 re-check, build after D758)

Agent copilot 1.0.113, claude-haiku-5.5. Machines: This Mac, then humata2-linux (fresh user vt1). Evidence: `<scratchpad>/verify/evidence/t1/`.
Case: flows/cases/verify/04-type-submits-once.md (live Copilot version of it).

## Old session from the earlier run
Steps: after `lane-up.sh t1`, `humata --json session type <old smoke session> "old holder probe" --no-submit`; `humata --json inbox`.
What happened: typing was accepted (`typed:true, submitted:false`). No "Outdated ... Migrate the holders" refusal, and the inbox had no Migrate item (`old-type.json`, `old-inbox.json`). Likely the holder was already current or restarted with the daemon. Not conclusive about old holders. Result: PARTIAL (nothing to refuse).

## 1. start --message runs with no extra input
- This Mac: PASS. hello.txt = hi; turn ran by itself (`local-start.json`).
- humata2-linux: FAIL (BUG-T1b). The start message sat in Copilot's input unsubmitted, session status showed `working`/`probably_idle`, no file. Reproduced on a second remote session (`remote-c1-screen.txt`, `remote2-screen.txt`). `session type <s> ""` then submitted it and hello.txt appeared (`remote-c1-enter.json`).

## 2. `session type <s> "create bye.txt containing bye"`
PASS on both: `submitted:true`, turn ran, bye.txt = bye (`local-c2.json`, `remote-c2.json`).

## 3. `--no-submit "draft"` then `type ""`
PASS on both: screen shows `❯ draft` unsent, `submitted:false`; empty type pressed Enter once, Copilot received "draft" and asked what to draft (`*-c3a-screen.txt`, `*-c3b-screen.txt`). Answered via `session type` (the ask-form is not an Inbox request, so typing is the only reply path; no Inbox item appeared).

## 4. Text ending in newline submits once
PASS on both. Copilot's own events: user.message count equals the number of intents (local 5, remote 7: hello, bye, draft, answer, nl[, run1, run2]); no extra messages. Note: a stray second Enter on an empty input would not create a message, so a double return is not fully excluded (same gap as the case file).

## 5. Type while a turn runs
PASS on both: Copilot itself queues it ("[pending]" in its UI, `*-c5-screen.txt`) and runs it after the turn; run1.txt and run2.txt exist. Humata reports `queued:null, submitted:true`; it does not know the text was queued.

## 6. Permission prompts
File-edit permission requests all reached the Inbox as `files` requests and were approved with `humata inbox approve` (local and remote). Not a BUG. (Inbox request preview shows an absolute path to the file; fine.)

## Model seen
All sessions: claude-haiku-5.5 (local 76 events, remote 93 events, `model-local.txt`, `model-remote.txt`). Copilot also used gpt-5.4-nano for its own helper calls (unrelated to the session model).

| flow | result |
|---|---|
| old holder typing | PARTIAL |
| 1 start --message, This Mac | PASS |
| 1 start --message, humata2-linux | FAIL |
| 2 type, Mac | PASS |
| 2 type, Linux | PASS |
| 3 no-submit + Enter, Mac | PASS |
| 3 no-submit + Enter, Linux | PASS |
| 4 trailing newline once, Mac | PASS |
| 4 trailing newline once, Linux | PASS |
| 5 type during turn, Mac | PASS |
| 5 type during turn, Linux | PASS |
| 6 prompts via Inbox | PASS |

Pass 10 of 12 rows (1 FAIL, 1 PARTIAL).

## Findings (new)
- **BUG-T1b: `session start --message` on a remote machine leaves the message unsubmitted.** Steps: `session start --channel t1r --machine <linux vm> --agent copilot --model claude-haiku-5.5 --message "create hello.txt containing hi, then stop"`. Expected: turn starts. Actual: text in Copilot's input box, nothing runs, status shows working/probably_idle, no Inbox item. Works locally. Likely the Enter is sent before the TUI is ready (D671 400 ms pause, no readiness check) or the remote path skips the submit; I did not find the code. Workaround found: `session type <s> ""`.
- **GAP: session status is wrong while the message is unsubmitted** (`working`/`probably_idle` with an agent waiting at an empty-turn input) and while Copilot waits on an ask-user form (`working`, no Inbox request). The person cannot tell it needs an answer.
- **GAP: no Migrate / Outdated signal observed for old holders**; could not exercise a truly old holder.
- Env: VM cannot clone verify/demo from GitHub; used the insteadOf shim from the earlier remote run (git config in the vt1 user), and an unshimmed `channel machine add` with a repo fails with a clone error (expected env). No dropped link or remote restart happened, so channel_attempt retry (D758) not exercised.

## Rerun 2 2026-11-17 (build 19:05, after D767)

Evidence: V/evidence/t1/rerun2/. Lane `t1` fresh, This Mac only, channel `tt`. Copilot 1.0.113. Model seen: claude-haiku-5.5 for both sessions (13 selectedModel, 29 currentModel, 359 `model` lines; gpt-5.4-nano is Copilot's helper), `model-t1.txt`.

### Case 3 (D761): ask_user becomes an Inbox question
Steps: session `ask` (se_ce7b502e...). Typed "use your ask_user tool to ask me whether to name the file a.txt or b.txt ..." Polled `humata --json inbox`, `ls`. Replied `humata inbox reply <id> b.txt`. Then a second ask (c.txt/d.txt) and `humata inbox deny <id> --note "no thanks"`.
What happened: inbox held one `question` card "Should I name the file a.txt or b.txt? / File name" with choices ["a.txt","b.txt"] (c3-inbox.json). Session status `needs_input`, not working (c3-status.txt). The screen shows only "Asking user ..." running; no form opened. After reply, Copilot showed "Denied by preToolUse hook: The user answered this question in Humata's Inbox, so the form was not shown", then created b.txt containing "hi" (in the session folder, not the clone: the agent's choice). Decline: card state `denied`, agent said "You declined ... so I haven't created a file" and stopped; session idle.
Result: PASS (both reply and decline)

### Case 4: type while a turn runs
Steps: asked for `sleep 40` then a file; 15 s later (status needs_input for a pending bash approval, then working) typed two more prompts.
What happened: both typed commands returned `typed:true, submitted:true, agent_busy:true, queued:null`. Copilot itself queued them ("[pending]" on screen, c4-screen-mid.txt) and ran them: slow.txt, q1.txt, q2.txt exist. `queued` is a bare null although Humata knows agent_busy is true and the text was typed during a turn. Also `agent_busy:true, queued:null` is returned on a fresh idle session (first `type ""` and the first prompt), so agent_busy does not tell idle from working.
Result: FAIL (queued is a bare null)

### Case 5 (N1): resumed fork read gap
Steps: `session fork <ask> --kind code-and-context --title fk`; `session stop`; `session resume`; `session type fk "reply with exactly the word PINEAPPLE ..."`. Polled `session read` every 2 s.
What happened: the agent answered PINEAPPLE on the screen within ~10 s, but `session read` showed only the user turn (seq 25) for more than 100 s (checked until 19:29, plus `--json`); the agent's reply never appeared, no further event came. Fork status went to idle.
Result: FAIL (c5-read.txt, c5-screen.txt)

### Summary (rerun 2)
| case | result |
|---|---|
| 3 ask_user -> Inbox question, reply | PASS |
| 3 decline | PASS |
| 4 agent_busy / queued on typing mid-turn | FAIL |
| 5 resumed fork read shows the reply | FAIL |

Pass 2 of 4.

### Findings (new)
1. BUG: `session type` mid-turn gives `queued:null`. Expected: a known value (for example the count, or "queued by the agent"). Seen on every type (local-only run).
2. BUG (N1 still open): after stop + resume of a fork, `session read` misses the next agent reply (checked 100+ s). Repro as above. Parent session read is fine.
3. GAP: `agent_busy:true` on an idle session.
4. Note: Copilot's folder-trust screen now becomes an Inbox question card and `session type` refuses to type into it (good); `type ""` accepts the default.

### Coach note on rerun 2 (19:40)
- `queued: null` on a mid-turn `session type` is by design: the daemon never holds text, and Copilot queues on its own (D-type contract). This is not a bug. `agent_busy: true` on an idle session is still a bug, and a lane is on it.
- The resumed-fork read miss (N1) is not a verdict on the fix. The 19:05 bins were cut while a temporary red-proof cut (an early return in the history watcher) was in the tree. Recheck on bins from the frozen 0.2 tree.

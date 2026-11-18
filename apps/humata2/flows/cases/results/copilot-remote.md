# Copilot remote-machine session flows: results (lane `remote`)

Agent: copilot 1.0.113, claude-haiku-5.5 (Copilot's footer showed "GPT-5.6 Luna").
Machines: This Mac (host), humata2-linux (Ubuntu aarch64, Lima), h2-terminal-c1 (Debian aarch64, Lima, shared; never stopped or resized).
Channel: `remote-flows` (repo verify/demo). Evidence: `<scratchpad>/verify/evidence/remote/` (short names below are relative to that folder).

Two workarounds were used and are not product paths:

- **Hand-placed service.** Add machine fails on systemd Linux (see Finding 1). To get past it, I copied the `bin-linux/linux-aarch64` humatad2, humata2_holder, humata and stack binaries into `~/.humata2/bin` of my VM user on each machine (sha256 checked against the bundle manifest), wrote the exact unit text from `service.rs` to `~/.config/systemd/user/humatad2.service`, and ran daemon-reload, enable and start. Host add then detected the running service and succeeded (`06`, `13`, `08`). The human-run add that failed is in `03`/`04`/`ops-add-*`.
- **git insteadOf for verify/demo.** The VMs can't reach `https://github.com/verify/demo.git` (a seeded repo). Each VM user has `git config --global url.<~/verify-remotes/verify/demo.git>.insteadOf https://github.com/verify/demo.git` pointing at a bare copy made from the lane's seeded repo (`evidence/remote/vmshim/demo.bundle`). Channel clones succeeded only after this (`20`, `21`).

Also: the first `humata session move --dry-run` did not dry-run; it started a real move (see Finding 4).

---

## Machines: add flow

Steps: (1) `humata host add v-remote-<vm>` (host key stop, `01`, `05`); (2) read the VM's own key with `ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub` (`02`), matched the fingerprint the daemon presented; (3) re-add with `--trust-host-key` (`03`, `06`); (4) add fails at `start` with exit 255 (`ops-add-linux.json`, `ops-add-c1.json`); (5) workaround above; (6) re-add succeeds with connect, check, install done and start/find_agents not run (`08`, `13`); (7) `humata host show` health: copilot 1.0.113 installed, linger yes, disk free ~15 GB (humata2-linux) and ~2.5 GB (c1) (`10`, `14`). The c1 add did not stop at DiskLow: the sparse home had 2.8 GB free.

Channel: `humata channel machine add remote-flows <machine>` (`17`, `18`); first clone failed (no GitHub), the failed op cannot be retried (`ops retry` says "can't run again"), so it was re-added (`20`, `21`) after the insteadOf shim.

---

## Flow 1: Run a Copilot session on a remote machine and have it write a file

Outcome sentence: a Copilot session started on humata2-linux writes a file there, which I can see on the VM and in `humata session read`.

Steps:
1. `humata session start --channel remote-flows --agent copilot --model claude-haiku-5.5 --machine humata2-linux` (`22`, session se_027c70b4863c417988bee1d322908a1e). Start op succeeded (`ops-flow1-start.json`), and the session pointer is `/home/vremote/Humata/remote-flows/sessions/flow1-remote-copilot-write/demo` (`23`).
2. Accepted Copilot's "Confirm folder trust" prompt on the screen with Enter (`24`, `25`). This prompt appears for each remote session folder; the lane's Mac Copilot config pre-trusts its own folders but nothing did so on the VM.
3. Sent: "Create a file named hello.txt containing the single word hi in the current folder. Then stop and say done." with `harness/say.sh` (`26` is the inbox read; the screen is in `27`/`28`).
4. The agent showed "Edit Create hello.txt" and "Running apply_patch" for 3+ minutes (`27`, `28`, `30`). No file was created.
5. `humata --json inbox` returned no requests (`26`, `31`). `humata --json login` failed because it needs a terminal (`32`). In a headless tmux terminal, `humata login` printed "Confirm it in the Humata app on the Mac…" and waited (`33`); there is no app here to confirm it.
6. On the VM, the Copilot process was waiting on `humata hook copilot permission` (pid 15169, state S, 3+ minutes, stderr to /dev/null, fds to the VM daemon socket). The VM daemon log had nothing for it (`35`, `humatad2.log`, `peer-link.log`, and an empty `tb_*.log`).
7. The Copilot screen showed no approval prompt, so there was nothing to answer on screen. I sent no keystrokes blind.
8. `humata session stop` ended the session; the hook process went away; no hello.txt on the VM (`36`, `37`, `38`).

What happened: the permission request for a remote agent's tool call never reached the Mac inbox; the Mac saw the session as "working", not "needs input".

Result: **FAIL**. The file was not written, and the permission request is the finding below. The session did show on the remote machine with the folder and the agent running.

Evidence: `22`–`38`, `ssh` process listing `35`.

---

## Flow 2: Move a session from the remote machine to the host and resume its conversation

Outcome sentence: a session with a Copilot conversation moves from humata2-linux to This Mac and the agent there can say what it did.

Steps:
1. `humata session start ... --machine humata2-linux` (`40`, se_95aac2cc601c4bb98ec54486821269e7, "flow2 remote chat then move").
2. Accepted the trust prompt (`42`), sent a chat-only turn (no tools): "Remember this code word: heron. Do not use any tools. Reply with one short sentence." Agent replied "I'll remember the code word: heron." (`42`, `44`). Session idle; inbox empty (`43`).
3. `humata --json session conversation <s>` refuses on the Mac: the session runs on another machine, so the conversation file is only on the VM (`45`).
4. `humata session move --to ma_76a49f09… --dry-run <s>` (`46`). Not a dry run: it started a real move op (`op_9aa3fac9…`), which then failed at "Listing what moves" (`47`).
5. The error: "Humata can't read this Copilot conversation (CLI version 1.0.113) yet … can't be carried to another machine … Nothing was copied or deleted." The session stayed on humata2-linux (status idle).

What happened: the move failed cleanly before copying anything. The agent could not be asked what it did because the session never got to the Mac.

Result: **FAIL** (product refuses to move a Copilot 1.0.113 conversation). Evidence: `40`–`47`.

### Flow 2b (no-turn variant, to isolate the conversation): remote to host

1. `session start --machine humata2-linux`, no prompt sent (`70`, se_0647035dd3f44f688156ce494bd39adb).
2. `humata session move --to ma_76a49f…` (`71`, `72`). Check, pause, pack, copy and verify all done; **resume failed**: "the agent's conversation file is missing, so its conversation can't be resumed." Nothing was deleted; the session stayed on humata2-linux (`72`, `73`).

Result: **FAIL** at resume. Note the flow 4 (host to remote) variant with no turns did succeed, so this is specific to the remote-origin direction.

---

## Flow 3: Move a session from one remote machine to another (humata2-linux to h2-terminal-c1)

Outcome sentence: a session on humata2-linux moves to h2-terminal-c1 and resumes there.

Steps:
1. Started a session on humata2-linux with no prompt sent, so no conversation file (`50`, se_ca409ed5bd834211b0a38623e2ab460d). The conversation-bearing case is not tested here: the same refusal as Flow 2 is expected, but I did not run it.
2. `humata session move --to h2-terminal-c1 <s>` (`51`). Op `op_e9bcf312…` succeeded: check, pause, pack, copy, verify, resume, switch, release all done (`55`).
3. Session now reports machine h2-terminal-c1, pointer `/home/vremote/Humata/remote-flows/sessions/flow3-remote-to-remote-no-turns/demo`, branch `uhvesta/flow3-remote-to-remote-no-turns` at 2494c0b (`54`). The folder exists on c1 and is gone from humata2-linux's `sessions/` (`65`, `64`).

Result: **PASS** for the no-conversation case. Conversation-carrying case: not run (see Finding 5).

Evidence: `50`–`56`, `64`, `65`.

---

## Flow 4: Move a session from the host to a remote machine

Outcome sentence: a session on This Mac moves to humata2-linux and its folder lands under the remote's Humata root.

Steps:
1. `humata session start --machine ma_76a49f09… --channel remote-flows --agent copilot` with no prompt (`60`, se_b9aece7a44984592a1ec1ec279d3793c).
2. `humata session move --to humata2-linux <s>` (`61`). Op `op_ba997d43…` succeeded: check, pause, pack, copy, verify, resume, switch, release all done (`62`).
3. Session machine humata2-linux, pointer `/home/vremote/Humata/remote-flows/sessions/flow4-host-to-remote-no-turns/demo` (`63`, `64`).

Result: **PASS** for the no-conversation case. Conversation-carrying host-to-remote case: not run.

Evidence: `60`–`64`.

---

## Summary table

| flow | result |
|---|---|
| 1. Copilot session on humata2-linux writes a file | FAIL (permission request never reached the Mac inbox; no file) |
| 2. remote to host, with conversation, agent says what it did | FAIL (product refuses CLI 1.0.113 conversation; nothing moved) |
| 2b. remote to host, no turns | FAIL (resume: conversation file missing; session stayed remote) |
| 3. humata2-linux to h2-terminal-c1, no turns | PASS (conversation-carrying case not run) |
| 4. host to humata2-linux, no turns | PASS (conversation-carrying case not run) |

Machine add: humata2-linux and h2-terminal-c1 both Ready only through the workaround; the product path fails (Finding 1).

## Findings

1. **BUG (known, fixed in tree per lane owner): Add machine on systemd Linux fails at `start` with exit 255.** `apps/humata2/daemon/machines/src/service.rs`: `stray()` runs `pkill -u … -f '[h]umatad2 serve --role machine'`, but the script text passed to `sh -c` also contains `ExecStart=%h/.humata2/bin/humatad2 serve --role machine` (inside the UNIT heredoc), so the shell matches itself and kills itself. Reproduced on the VM: `sh -c 'echo ExecStart=humatad2 serve --role machine; pkill -u $(id -u) -f "[h]umatad2 serve --role machine"; echo SURVIVED'` prints the first echo and no SURVIVED, exit 255. Expected: add completes. Actual: "Start failed … (exit code 255)" with the output showing only the "Created symlink" line, so the real cause is not surfaced to the user (`setup.rs` ServiceFailed path). Evidence: `ops-add-linux.json`, `ops-add-linux-retry.json`, `ops-add-c1.json`, `35`.
2. **BUG: a remote agent's permission request never reaches the Mac inbox.** The Copilot tool call sits on `humata hook copilot permission` on the VM (pid 15169, sleeping, stderr to /dev/null), the Mac session stays "working" rather than "needs input", and `humata inbox` returns no requests. The VM's `humatad2.log` and `peer-link.log` have no entry. Expected: the request appears in the Mac inbox (or the session shows needs input). Actual: nothing; the agent waits until the session is stopped. Evidence: `27`, `28`, `30`, `31`, `35`, `36`–`38`. Also: `humata login` from a headless tmux printed "Confirm it in the Humata app on the Mac…" and nothing was approvable; `humata login` without a tty says "needs a terminal to bind" (`32`, `33`).
3. **UX: each new remote session asks for folder trust on the screen.** Copilot's "Confirm folder trust" appears for every new session folder on a VM. Expected: Humata pre-trusts its own session folders the way the Mac lane config does. Actual: manual Enter needed. Evidence: `24`, `25`, `41`. (Repro: start any remote session.)
4. **BUG: `humata session move --dry-run` runs the move.** Expected: print a plan and token, change nothing. Actual: returned `move_session` op `op_9aa3fac9…` in state running and actually started the move (it failed later). Repro: `humata session move --to <machine-id> --dry-run <session with turns>`. Evidence: `46`, `47`.
5. **Limitation (product): sessions with a Copilot 1.0.113 conversation can't be moved.** "Humata can't read this Copilot conversation (CLI version 1.0.113) yet … can't be carried to another machine." This blocks Flow 2 and makes the conversation-carrying cases of Flows 3 and 4 unverified. The refusal is clean (nothing copied or deleted) and the message says to update Humata. Evidence: `47`.
6. **BUG: a remote session with no turns can't be moved back to the host.** Flow 2b: pack and copy passed, then resume fails with "the agent's conversation file is missing". Flow 4 (host to remote) with no turns works, so the asymmetry matters. Expected: resume a session that never had a turn, or refuse before copying. Actual: copy done, resume failed, session left on the remote. Evidence: `71`, `72`, `73`.
7. **UX (candidate): remote sessions label their location as "This computer".** On the agent screen for a humata2-linux session, the footer reads "Run on: This computer" (`25`). Expected: the machine's name. Actual: "This computer", which reads like the Mac. Evidence: `25`.
8. **UX (candidate): agent footer model differs from the session's model.** Session model is claude-haiku-5.5, the footer shows "GPT-5.6 Luna" (`24`–`28`, `44`). Verify before reporting as a bug; it may be Copilot's own display.
9. **UX: a failed channel clone can't be retried.** `humata ops retry <op>` says "can't run again"; re-adding the machine to the channel is the only way (`20`, `21`). Evidence: `19`.
10. **Env, not product:** the VMs can't reach GitHub, so `verify/demo` clones fail unless shimmed (insteadOf above). Recorded as a workaround.

## Environment notes

- `V/bin` was briefly holding Linux binaries (14:36 to later); the lane owner reports macOS builds are back. The Mac daemon is still the one started at lane-up (pid 23515), unrestarted, per the owner's instruction. None of my CLI calls in the outputs above hit `exec format error`.
- Remote Linux binaries used for the workaround were the copies in `bin-linux/linux-aarch64`, placed before the owner's later instruction not to use `V/bin-linux`; they are not re-used now.

## Cleanup status

- Not done yet: stop the lane daemon, and remove the two VM users (`vm-user.sh remote <vm> remove`). Kept on purpose: the owner said the flows should be rerun on the fixed binaries once they are in `V/bin`; reruns need the VM users and machines.
- Sessions still on the Mac: se_95aac2cc… (idle, humata2-linux), se_0647035d… (working, humata2-linux), se_ca409ed5… (idle, h2-terminal-c1), se_b9aece7a… (idle, humata2-linux). se_027c70b4… exited.

---

## Rerun 2026-11-17 (build after D758)

Agent: copilot 1.0.113. Evidence: `<scratchpad>/verify/evidence/remote/rr/` (names below relative to it). Model seen (read from the agent's own events.jsonl, `model-*.txt`): **claude-haiku-5.5 for every session** (Copilot's footer now also reads "Claude Haiku 5.5"; the old "GPT-5.6 Luna" footer is gone). Sessions: rr-flow1 (humata2-linux, 15 events), rr-flow2 (Mac then linux), rr-flow2b (Mac), rr-flow3 (linux then c1, 7 events on c1).

### Upgrade (recorded as evidence)
1. Old state: lane daemon pid 23515 (started 14:24, bin.old-1817); VM daemons on both VMs were the old `bin-linux` copies placed by hand (`01`, `07`).
2. `lane-up.sh remote` restarted only my daemon (new pid 40735, `03`). Holders and sessions survived: session list intact (`08`), screens of old sessions still live (`09`), `session type --no-submit` into an old holder worked (`10`).
3. After the restart: `host ls` showed both VMs online at daemon_version 0.0.0 (`04`); `humata --json inbox` empty (`05`, `11`); no Migrate item, no `migration_needed`, no "Outdated ... Migrate the holders" refusal; nothing in daemon.log matching migrat/outdated. This matches D290 l (dev build, no bundle.json). The VM daemons were still the old binaries (sha256 differed from `bin-linux`) and nothing in the product offered to upgrade them.
4. Route that actually worked: `host remove` for both machines (`12`), `vm-user.sh remote <vm> remove` then recreate (`13`), `host add` (stopped at host key, matched against `ssh-keygen -lf` on the VM, `16`), then `host add --trust-host-key` (`17`, `18`). **Add machine now succeeds end to end with no hand-placed service** (connect, check, install, start done; the old Finding 1 is fixed). The new VM binaries match `bin-linux/linux-aarch64`. Then `channel machine add remote-flows <alias>` for both (`19`); clones worked with the same git insteadOf shim (env workaround, VMs can't reach GitHub; I copied `remotes/verify/demo.git` to `~/verify-demo.git` on each VM). Machine names are now the ssh aliases (`v-remote-humata2-linux`, `v-remote-h2-terminal-c1`). The old sessions went offline with the removed machines.

### Flow 1: Copilot session on a remote machine writes a file
Machine: v-remote-humata2-linux. Steps: start session (`21`); Enter on the folder-trust prompt via `session type ""` (`22`-`24`); `session type "Create a file named hello.txt containing the single word hi in the current folder. Then stop and say done."` submitted (`25`); the Mac inbox showed a request "rr-flow1 wants to run edit" within seconds (`26`); `humata inbox approve` (`27`); file written on the VM (`28`: contains "hi"), `session read` shows the create and "Done." (`29`). Model seen: claude-haiku-5.5.
Result: **PASS** (D744 verified). 

### Flow 2: remote to host, with conversation, agent says what it did
Steps: session on humata2-linux, chat turn "Remember this code word: heron" (`30`-`33`); `humata session move --to <Mac id>` (`35`): all 8 steps done (no 1.0.113 parse refusal, D746 verified); session now on This Mac (`36`); screen shows the old turns (`37`); asked "What code word did I ask you to remember?" and got "The code word you asked me to remember was "heron."" (`38`, `39`). Model seen: claude-haiku-5.5 (`model-se_d2299278-mac.txt`).
Result: **PASS**.

### Flow 2b: remote to host, no turns
Steps: session started on humata2-linux, trust accepted, no prompt; move to the Mac (`42`, `43`): all steps done, session working on the Mac (start_moved verified); then typed "Reply with the single word ok. No tools." and got "ok" (`44`, `45`). Result: **PASS**.

### Flow 3 (with conversation): humata2-linux to h2-terminal-c1
Steps: chat "Remember this code word: otter" -> "Noted: otter."; move to v-remote-h2-terminal-c1 (`f3c-move.json`, all steps done); asked again -> "otter." on c1 (`f3c-screen2.txt`). Model on c1: claude-haiku-5.5 (`model-se_f3-c1.txt`). Result: **PASS**.

### Flow 4 (with conversation): This Mac to humata2-linux
Steps: moved the heron session (now on the Mac) to v-remote-humata2-linux (`f4c-move.json`, all done); asked again; the agent answered "heron" from its conversation (`f4c-screen.txt`). Model: claude-haiku-5.5 (36 events on the VM). Result: **PASS**.

### channel_attempt (D758)
No dropped link or remote daemon restart happened during this run (peer links stayed up), so the retry was not exercised. Not tested.

| flow | result |
|---|---|
| Upgrade: no Migrate item on dev build | as designed (nothing seen); VM daemons not upgraded by product |
| Add machine (no workaround) | PASS |
| 1. remote Copilot writes a file, approval via Mac inbox | PASS |
| 2. remote to host with conversation | PASS |
| 2b. remote to host, no turns | PASS |
| 3. remote to remote with conversation | PASS |
| 4. host to remote with conversation | PASS |
| channel_attempt retry | not exercised |

Pass count: 6/6 flows (plus add machine).

### Findings (new)
1. **BUG: text typed right after a move to a remote machine is lost though `session type` says submitted.** Repro: move a session with a conversation to another machine (Flow 4 / Flow 3), wait ~8 to 20 s after the op succeeds, `humata session type <s> "<text>"`. Expected: the text appears. Actual: `{"typed":true,"submitted":true}`, but the screen shows only "MCP Servers reloaded" and no prompt; the second identical type, a few seconds later, works. Seen twice (Flow 4 `f4c-screen.txt` before the retry `f4c-type2.json`; Flow 3 `f3c-screen.txt`, retry `f3c-type2.json`). Likely the agent's UI is still starting when the input arrives; the move op should report ready only when input is accepted, or type should say it was not accepted.
2. **GAP: `session read` stops at the turns from before a move.** After Flow 4's move to the remote, a third turn was answered on screen (and is in the VM's events.jsonl) but `humata --json session read` still listed only turns 1 and 2 (`f4c-read.json`). Expected: the new turn shows.
3. **UX (still there): Copilot's "Confirm folder trust" prompt appears for each new remote session** (`23-f1-screen1.txt`); needs `session type ""` to pass.
4. **UX (still there): footer says "Run on: This computer" for a remote session.**
5. **UX: stale machine ids stay in the channel after `host remove`**: `channels` lists `humata2-linux` and `h2-terminal-c1` in `remote-flows` beside the new aliases (`channel machine` list after `19`); old sessions then show "offline".
6. **UX: `humata conversation` on a remote session refuses with a pointer to `session read`** (as before; fine, noted for the record).
7. **GAP: nothing offers to upgrade VM daemons running an older build** on a dev build (by design per D290 l), so the only route was remove + re-add; sessions on those machines were dropped.
8. **Env:** `channel grid` reports "GetGrid isn't built into this humatad2 yet" on this admin daemon (`channel grid remote-flows`).

---

## Rerun 2 2026-11-17 (build 19:05, after D767)

Agent: copilot 1.0.113 on the VM (1.0.114-0 on the Mac), model claude-haiku-5.5. Machine: v-remote-humata2-linux (fresh lane, fresh VM user, github url.insteadOf shim, remote folder NOT pre-trusted). Channel rr2. Evidence: `<scratchpad>/verify/evidence/remote/rerun2/` (numbers below). Model seen: **claude-haiku-5.5 for every session**, read from the agent's own events.jsonl (`model-c1.txt`, `model-remote-sessions.txt`, `model-loc-local.txt`).

### Flow 1: a remote start's first message is held until Copilot's input is ready (D763, D767)
Agent: copilot, claude-haiku-5.5. Machine: v-remote-humata2-linux.
Steps: `session start --machine ... --message "Create a file named hello.txt ..."` (`10`); screen (`11`), `ls` (`12`), `inbox` (`13`); `inbox reply <rq> Yes` (`30`); screens (`31`, `33`); `session type ""` (`34`), screens (`35`, `36`); `session type ""` again (`38`, `39`); approve edit; events (`41`). Repeated on a second session c2 (`50`-`56`).
What happened:
- Held start: the trust dialog was clean, no text typed into it; session status needs_input; inbox had a question card ("copilot is asking on its screen... Do you trust the files in this folder?", choices "Yes" and "Yes, and remember this folder for future sessions"; "No" is not offered). Humata did not answer by itself. PASS for those parts.
- **The card's reply did not answer the dialog.** `inbox reply <rq> Yes` returned state answered, the card left the inbox, but the dialog stayed on screen for over 2 minutes, in both sessions (`33`, `54`). Nothing was typed.
- Only `session type <s> ""` (Enter) answered the dialog. After that the held start message was typed into the input. In c1 it sat there unsubmitted for over a minute (`35`, `36`; no user.message in events) until a second `session type ""` submitted it (`39`); in c2 one Enter was enough and the message was submitted (`56`). Message reached the agent exactly once in both (`41`: one user.message with the text; the other is Copilot's own empty memory message).
Result: **FAIL** (the card answer is not delivered; start message submit is inconsistent).
Model seen: claude-haiku-5.5.

### Flow 2: typing into a session waiting on folder trust is refused (D762)
Steps: with the dialog up (c1, then the moved session `loc`), `session type <s> "hi"` (`20`, `67`); screens before/after (`11` vs `21`, identical).
What happened: refused with error kind `failed_precondition`, exit 7: "the agent ... is asking on its screen whether to trust the session's folder, so nothing was typed...", fix says answer in the Inbox or on screen, or `type ""` for Enter. Nothing typed; the screen is byte-identical (`diff` SAME). Same on the moved session.
Note: it is not a "retry later" kind (failed_precondition, not a retryable code), and its fix points to the Inbox, whose reply does not work (Flow 1).
Result: **PASS** (refused, nothing typed).

### Flow 3: typing after a move works and never answers a dialog
Steps: local session `loc` (`60`), turn 1 (`62`); `session move --to v-remote-humata2-linux` (`64`); trust card appeared again for the moved session (`65`, `66`); `type "hi"` refused (`67`); reply "Yes" did nothing (`68`); `type ""` (`69`, `70`); `type "What is the code word?..."` (`71`, `72`); read (`73`).
What happened: after the Enter answered the dialog, `session type` submitted normally (agent answered "heron"). A typed message never answered the dialog.
Result: **PASS**. Same card-reply defect as Flow 1.
Model seen: claude-haiku-5.5 (local and remote copies).

### Flow 4: read after moves shows every turn
Steps: turn 1 local; move to remote, turn 2; move back to This Mac (`74`, op succeeded, no card); turn 3 "kestrel" (`76`); reads (`63`, `73`, `78`).
What happened: `session read` showed turns 1-3 with replies (`78`). Turn 2's reply showed in the read 30 s after the answer (read at 19:20:22, the first poll). Turn 3's reply was in the read the first time I looked, 20 s after typing, with no further event (lag 0 s on top of the 20 s sleep). The N1 and earlier "read stops at pre-move turns" gap are fixed in this run.
Result: **PASS**.

### Flow 5: a removed machine can be added again and counts as new (D760)
Steps: `host remove` (`90`); `host add` right after (`91`); `host ls`, `channels`, `channel grid` (`92`-`94`); `host add` again (`95`), with `--trust-host-key` (`96`); inbox, `skills sync -y` (`97`, `100`, `101`).
What happened:
- Removal ran stop_daemon then forget, succeeded in under 10 s. `host ls` no longer shows the machine (good). **`channels` still lists `v-remote-humata2-linux` in the channel's machines, and the grid still has its cell as machine_offline** (`93`, `94`). Expected gone.
- `host add` caught while the removal ran: exit 6 but kind **name_taken** ("already added... pick another name"), not "still being removed, try again" (`91`). Expected: the new retry-later message. FAIL for that expectation (the op was not yet finished at that moment? it had state running when I issued add right after remove; `ops` later showed it succeeded).
- `host add` after removal: needs the host key again (counts as new, `95`), then succeeds (`96`), online, not re-added to the channel by itself but it was still listed there.
- No inbox card or ask for the skills sync, neither on the first add nor the re-add; `skills sync --machine <m> -y` just succeeded (`100`, inbox empty `101`). The lane has no user skills, so the "asks again" could not be seen.
Result: **FAIL** (channel still lists removed machine; name_taken instead of "still being removed"; skills-ask not observable).

### Flow 6: the channel grid answers (D764)
Steps: `--json channel grid rr2` with no sessions (`grid-1.json`), with 1 Mac and 2 remote sessions (`80`), with the machine offline (`81`; I made it offline by pointing my lane's ssh alias at a closed port and killing my VM user's humatad2), after (`82`).
What happened: one cell per repo x machine, with `state` ready and `sessions` counts matching (Mac 1, remote 2). Offline machine shows state `machine_offline` (host ls says offline). Back to ready after restoring the config.
Result: **PASS**.

| flow | result |
|---|---|
| 1. start message held until input ready, trust card, answer | FAIL (card reply not delivered; first-run submit needed a second Enter) |
| 2. type refused while trust dialog shows | PASS |
| 3. type after a move works, never answers a dialog | PASS |
| 4. read shows all turns across two moves, last reply without event | PASS |
| 5. removed machine re-added, counts as new | FAIL |
| 6. channel grid | PASS |

Pass count: 4/6.

### Findings (new)
1. **BUG: answering the "Confirm folder trust" card does not answer Copilot's dialog.** Steps: start a Copilot session on a remote machine in an untrusted folder (with or without `--message`); `humata --json inbox` shows the question; `humata --json inbox reply <rq> Yes`. Expected: dialog answered, agent proceeds. Actual: the request becomes `answered` and leaves the inbox, the dialog stays up (reproduced three times: c1, c2, moved session; `33`, `54`, `68`). Workaround: `session type <s> ""`. Because the card is gone, the owner has no cue left. Also the moved session's card offers only "Yes" (`66`) while fresh sessions offer two choices.
2. **BUG: held start message is typed after the trust answer but not always submitted.** In c1 the message sat in the input unsubmitted until a second Enter (`35`, `36`, `39`); in c2 it was submitted with the first Enter (`56`). Expected: submitted exactly once, every time.
3. **BUG: `host remove` leaves the machine in the channel's machine list and in the channel grid** (`93`, `94`). Expected gone from both (D760).
4. **BUG: `host add` during a running removal returns `name_taken`** ("already added... pick another name"), exit 6, instead of a clear "still being removed, try again" (`91`).
5. **GAP: D760's "first skills sync asks again" could not be seen.** No inbox card at the first add, at the re-add, or after `skills sync -y`. The lane has no user skills; needs a lane with skills to judge.
6. **UX: D762 refusal uses `failed_precondition` (exit 7)**, not a retry-later kind, and its fix points at the Inbox answer that does not work (finding 1).
7. **UX (still there): "Confirm folder trust" prompt appears for each new remote session and for each moved session** (`65`); footer still says "Run on: This computer" on the remote.
8. **ENV:** stopping `humatad2.service` of my VM user does not take the machine offline: the Mac re-dials and restarts it through ssh within seconds. To get an offline cell I had to break the ssh alias and kill the VM user's daemon processes.
9. Known fixed: BUG-T1b ordering (text never typed into the dialog), N1 (read lag), GetGrid.

### Coach note on rerun 2 (19:40)
- Case 5 (D760) is not a verdict on the fix. The 19:05 bins were cut while temporary red-proof mutations were in the tree: name_free always returned NameTaken, and the skills delete and channel removal were skipped. Those are exactly the name_taken and the kept channel/grid entry seen live. Recheck on bins from the frozen 0.2 tree.
- Case 1 (the trust card reply) stands. The card's choices were parsed as only "Yes", and the reply didn't press the row. A lane is on it.

---

## Wave 3 2026-11-17 (0.2, fe9eea4f)

Agent: copilot 1.0.113 on a Lima VM user (humata2-linux), claude-haiku-5.5. Fresh lane (`LANE_FRESH=1`), fresh VM users, github url.insteadOf shim, remote folder NOT pre-trusted. Channel w3. Evidence: `<scratchpad>/verify/evidence/remote/wave3/` (numbers below). **Model seen:** every conversation turn ran on claude-haiku-5.5 (events.jsonl of every session, `model-vremote.txt`). Some sessions (loc, m4) also have Copilot's own background "utility" calls on `gpt-5.6-luna-utility` at turn 0 (`model-luna-lines.txt`); not the conversation model.

### Case 1: trust card lists every row, Yes presses, No presses (D767 as amended)
- Fresh remote session t1: first card had choices Yes / "Yes, and remember..." (2 rows) while the dialog showed 3; it became the full 3 (adds "No") only after my screen attach (`11`, `13`). Same on t2, t3, m1-m4: the card stays at 2 rows for over 60 s until something re-reads the screen (`22`, `52`, `54`). Moved local session `loc`: first card offered only "Yes" (`74`), full list after a screen read (`76`).
- `inbox reply Yes` (on 3-row cards and on stale 2-row cards): dialog gone, Copilot input shows (`15`, `58-m1/m2`). PASS.
- Reply `No` on t2 (3-row card): the No row was pressed; Copilot exited, the session went `exited`, screen empty (`25`, `26`, `29`). Humata recorded the card answered.
- NotPicked try: t3: `session type "" ` answered the dialog on screen; the card was withdrawn within ~10 s and a later reply returned `request_withdrawn` (correct, `31`-`35`). t4: `type ""` then reply Yes immediately (`43`, `44`): reply returned state **answered** though the dialog was already answered by the Enter, so the reply pressed nothing (`45`). Card says answered without Humata having pressed anything: matches the BUG condition in the brief.
- Result: **PARTIAL** (Yes and No press; stale card rows; NotPicked shows "answered").

### Case 2: held start message typed once, submitted without manual Enter (D763)
Four sessions started with `--message` while the dialog was up (m1-m4), answered Yes through the Inbox. The message was submitted without a manual Enter in all four. m2, m3: typed once. **m1, m4: typed twice back to back into one user message** ("...then stop.Create a file named m1.txt containing the word m1, then stop."), seen in the agent's events (`59-events.txt`, `67-events.txt`) and screens (`58-m1-after.txt`, `66-m4.txt`). Result: **FAIL** (2 of 4 doubled).

### Case 3: unsent start message reaches the host Inbox (D763)
D763: still unsent after 30 min raises "Send it now / Leave it unsent" with the whole text. Session u1 started 21:05 with the trust dialog left unanswered; by ~21:35 the host's `humata --json inbox` showed the card with the full text and both choices (`101-inbox-unsent.json`). "Send it now" with the dialog still up typed nothing into the dialog (`104`). After I answered the trust card Yes, the message arrived **twice** in one user message (`107`, `108-u1-events.txt`). Result: **PARTIAL** (card and text pass; "send it now" did not send it once).

### Case 4: remove and re-add a machine (D760), on a second VM user (vm-b)
`host remove` took vm-b out of `host ls`, the channel's machines and the grid (`90`-`92`). `host add` 0.1 s after the remove: exit 6, kind aborted, "is still being removed" (`88`). Re-add stopped at the host key again (`93`), then succeeded (`97`). With a user skill alpha in the lane home: `skills sync --dry-run` said first_sync false before the removal (`86`), first_sync true after the re-add (`98`). No Inbox ask card appeared for the first sync. Result: **PASS**.

### Case 5: Identity probe: what of the old vm-a attaches
Removed vm-a, pointed the alias `v-remote-humata2-linux` at a second fresh VM user (other home, no `~/.humata2`), added it (it asked for the host key again, same key as the VM is the same; `123`, `124`). Snapshots `120-pre`, `122-postremove`, `125-postadd`.
- Machine id: remote machines in `host ls` are keyed by name only (`v-remote-humata2-linux` before and after). Inbox items show the internal `ma_` id: old ma_d6b777fd..., new machine's session shows ma_fc52d1a6...: not reused.
- Sessions: the 10 old sessions stay listed under the machine name; offline after removal, then **idle on the new machine though it has none** (`131`); `session type` on one says not_found. Ghost sessions.
- Channels: removed from channel w3 at removal; not back on re-add (I added it by hand).
- Grid: cell gone after removal; back only after channel add.
- History: `session read` of old sessions still works (host mirror), incl. the moved one with both turns.
- Pinned host key: cleared by the removal, asked again on re-add.
- Skills sync record: cleared (first_sync true).
- Inbox items: old requests from the removed machine (m1-m4, u1 files/session cards) stay pending with the old ma_ id.
- Transfer records: old transfer to vm-a (done) stays in `transfers`, destination shown by name.
- Not carried: nothing of the old daemon, folders or sessions on the new user.
Result: report only.

| case | result |
|---|---|
| 1 trust card rows, Yes, No, NotPicked | PARTIAL |
| 2 held start message typed once | FAIL (2 of 4 doubled) |
| 3 unsent message in host Inbox, Send it now once | PARTIAL |
| 4 remove / re-add (D760) | PASS |
| 5 identity probe | report |

### Findings (new)
1. **BUG: the trust card shows fewer rows than the dialog and doesn't catch up.** Steps: start a remote Copilot session in an untrusted folder; `inbox` shows choices Yes, "Yes, and remember..." only; the screen has "3. No (Esc)". It updated only after I attached to the session (`11`, `22`, `52`, `54`; moved session `74`). Expected: all three rows from the start (D767 amendment).
2. **BUG: `inbox reply` on the card says answered after the dialog was answered on screen** (t4, `43`-`45`); nothing was pressed.
3. **BUG: held start message typed twice.** Steps: `session start --machine <remote> --message "..."` in an untrusted folder, reply Yes to the card. Expected one copy. Actual: m1, m4, u1 doubled (`59`, `67`, `108`). u1: after "Send it now" and then the trust Yes.
4. **UX: "Send it now" while the dialog is up marks the card answered at once**; the message is typed later when the dialog clears, and a second delivery may follow (finding 3).
5. **GAP: re-added name picks up the old machine's sessions as idle ghosts and its stale Inbox cards** (Case 5).
6. **UX: no Inbox ask for the first skills sync after the re-add** (dry-run only shows first_sync true).
7. **Note: Copilot's own utility model is gpt-5.6-luna-utility** (loc, m4 events); turns ran on haiku.

# VERIFY lane `files`: file transfers, moves, trash and an agent-driven copy

Agent: copilot 1.0.113, claude-haiku-5.5 (flow 5 only; flows 1 to 4 and the big-file test were run from the CLI).
Machines: This Mac (host, lane daemon), `vm-a` (VM user `vfiles`, Ubuntu aarch64 on humata2-linux), `vm-b` (VM user `vfiles2`, same VM).
Channel: `files-flows` (repo verify/demo; channel machines: vm-a). vm-b is outside the channel.
Evidence: <scratchpad>/verify/evidence/files/

## Setup: machines added (workaround for the known Add-machine bug)

Steps:
1. `vm-user.sh files humata2-linux` and `vm-user.sh files2 humata2-linux`; merged the `v-files2-humata2-linux` block into `lanes/files/ssh.config`.
2. `humata host add v-files-humata2-linux --name vm-a` and `... v-files2-humata2-linux --name vm-b`. Both first stopped on the host key (same VM, so one fingerprint); re-added with `--trust-host-key`.
3. Both adds then failed at step `start`: "setting up the humatad2 service ... failed (exit code 255)". The daemon's own message shows `Created symlink ... humatad2.service` and nothing else.
4. Workaround: on each VM user, copied the four bundle binaries (humatad2, humata2_holder, humata, stack from `bin-linux/linux-aarch64`) to `~/.humata2/bin`, wrote the same unit text as `service.rs` UNIT, ran `enable --now`, and re-ran `host add`. Both then succeeded (`vm-a`, `vm-b` online, os linux).

What happened: the install step is skipped when the files are already present, so the start step is not exercised by the workaround. Reproduction of the root cause is under Findings.

Result: BLOCKED for the product path (Add machine fails on systemd Linux); PASS with the workaround.
Evidence: files/files-host-add-vm-a.json, files-host-add-vm-a-2.json, files-host-add-vm-b.json.

## Flow 1: Send a file and a folder from This Mac to a remote machine

Steps:
1. Created `shared/random-300k.bin` (300 KB random) and `shared/notes-folder/` (3 files, nested `sub/`) in the channel shared folder on This Mac. Recorded source sha256 (files/host-src-sha256.txt).
2. `humata --json --channel files-flows cp this-mac:shared/random-300k.bin vm-a`, then the same for `this-mac:shared/notes-folder`.
3. Polled `humata transfers` until done. Hashed every file under vm-a's `~/Humata/files-flows/shared`.

What happened: both transfers reached state done, and the folder landed as `notes-folder/` with its nested files. During the transfer the destination shows a staging folder `.humata-transfer-<id>/` holding zero-byte files; the final tree has no staging left. A listing taken during the copy showed a zero-byte file, which was my race, not a corrupt result (the final hashes match).

Result: PASS
Evidence: files/f1-cp-file-host-to-vma.json, files/f1-cp-folder-host-to-vma.json, files/f1-remote-vma-sha256.txt (matches host-src-sha256.txt, 5 files).

## Flow 2: Send from a remote machine to This Mac

Steps:
1. On vm-a created `shared/from-vma.txt` and `shared/vma-folder/{data.bin (120 KB), deep/d.txt}`.
2. `humata --channel files-flows cp vm-a:shared/from-vma.txt this-mac`, then the folder.
3. Compared hashes with `This Mac/received/vm-a/`.

What happened: both done. Machine-only destination `this-mac` landed under `received/vm-a/` (PL-4), not the channel's shared folder. That matches PL-3 because This Mac is not in the channel's machine list, but the destination is not the channel shared folder, so check that against your intent.

Result: PASS
Evidence: files/f2-vma-src-sha256.txt, files/f2-host-received-sha256.txt (3 files match).

## Flow 3: Send from one remote machine to another

Steps:
1. `humata --channel files-flows cp vm-a:shared/random-300k.bin vm-b`, then `cp vm-a:shared/vma-folder vm-b`. vm-b is outside the channel.
2. Hashed `vm-b:~/Humata/received/vm-a/`.

What happened: both done. Files landed under `received/vm-a/` on vm-b; hashes match vm-a sources (55d73e52... and 1ab627c7... and 673953e0...).

Result: PASS
Evidence: files/f3-cp-file-vma-to-vmb.json, files/f3-remote-vmb-sha256.txt.

## Flow 4: Move (mv) leaves the source in the trash, and restore

Steps:
1. `humata --channel files-flows mv vm-a:shared/from-vma.txt vm-b`.
2. `humata --json --channel files-flows trash --machine vm-a` listed `vm-a:82b3267b...` (description from-vma.txt, 31 bytes, empties in 30 days). On disk: `~/Humata/.trash/<id>/content` and `item.json`.
3. Checked vm-b's copy: sha matches the source.
4. `humata --json trash restore vm-a:82b3267b...`.

What happened: the move transfer reached done. The source was removed from shared and listed in trash. Restore put the file back in the channel's shared folder with the same sha. Trash became empty.

Minor: the restored file's mode changed from 644 to 664 (the trash path does not keep the mode).

Result: PASS
Evidence: files/f4-mv-file-vma-to-vmb.json, files/f4-trash-vma-after-mv.json, files/f4-trash-restore.json.

## Big file (50 MB): progress and cancel

Steps:
1. Created `shared/big-50m.bin` (52428800 bytes from /dev/urandom). Source sha f7252c14....
2. Copy to vm-a: polled `humata --json transfers` every second (files/f5-big-progress.txt).
3. Copy to vm-b: started it and cancelled as soon as done_bytes > 0 with `humata transfer cancel <id>` (files/f5-big-cancel-progress.txt).

What happened: copy to vm-a finished in about 2 s; the first sample showed 7340032 of 52428800 bytes, then done. The copy to vm-b was cancelled at 9437184 bytes; the transfer state is cancelled and the destination has no partial file or staging folder. The vm-a copy's sha matches the source.

Progress is only visible by polling; `cp --json` returns immediately in state running. Sampling is coarse for a LAN copy this fast.

Result: PASS (progress and cancel)

## Flow 5: Agent-driven transfer (Copilot session copies a file with the humata CLI)

Steps:
1. Created `shared/agent-note.txt` on This Mac (sha in files/f6-agent-src-sha256.txt).
2. `humata --json --channel files-flows session start --agent copilot --model claude-haiku-5.5 --title "files agent copy"` (session se_ec26583f...). Session on This Mac.
3. Typed the prompt (goal only, no syntax): "Copy the file agent-note.txt from this channel's shared folder to the machine vm-b with the humata CLI. Then reply with the command you ran and whether it worked." `session type` put it in the prompt unsubmitted (BUG-T1 again). Submitted with Enter via tmux.
4. Approved each bash request in the inbox with `humata inbox approve <id>`. Retried twice with explicit syntax.

What happened:
- The agent did not know the command at first. Its first bash was `find / -name agent-note.txt` (slow, more than 2 minutes; the agent stopped it itself), then `humata --help`, `humata cp --help`, `humata files --help`, `humata channels`.
- Every shell command was its own inbox request ("wants to run bash"). 13 requests in total, all approved, for one copy attempt.
- Attempt 1: `humata cp --channel files-flows --dry-run shared/agent-note.txt vm-b` failed: "the request has no destination.session_clone.session". The agent read the bare path as its own clone. It then said "vm-b isn't a registered machine" and "the only machine is This Mac" (`humata host show vm-b` returned no machine). From my shell the same machine is online.
  - Expected now (D756): `vm-b` names no machine the agent can see and has no `.`, so the CLI refuses it as usage (exit 2, `invalid_field`), starts nothing, and points to `humata host ls`, `./vm-b` or `vm-b/`. It no longer becomes a clone path or the session_clone error. A machine of the channel named by name (vm-a) resolves, and the send is then refused by D755 as a write on another machine; rerun flow 5 in the pull direction.
- Attempt 2 (after I sent the explicit form `cp this-mac:shared/agent-note.txt vm-b`, with a stray trailing dot from my typing): failed with `"." leaves its folder...`; the CLI message is clear.
- Attempt 3 (explicit form, no dot): failed with `no destination.session_clone.session`.
- Attempt 4 (to vm-a, which is in the channel): same failure.

What the agent got wrong vs what the product does: the agent could not complete the copy in any form. For the human principal the same machine-only destination works (flows 1 to 3). So a session principal cannot use a machine-only destination, and it seems to see only This Mac (or only channel machines) in `host show`.

Nothing was transferred; nothing in the destinations changed. The agent ended by asking the user for a destination file name.

Result: FAIL (the agent did not complete the copy; approvals worked, the command and location syntax were the blockers)
Evidence: files/f6-agent-final-screen.txt, files/f6-inbox-all.json (13 requests, all approved), files/f6-approve-*.json, files/f6-session-start.json, files/f6-say-*.txt.

Ruling (D755): the push this run asked for is not something an agent does. An agent writes `shared/` only on the machine it runs on, so sending a file to another machine's `shared/` is refused at once (kind `denied`, hard limit), nothing is written there, and the fix gives the pull hint: put the file in `shared/` here, and a session on the other machine copies it in, or the user sends it with `humata cp`. The rerun below uses the pull direction.

Rerun (pull direction):
1. `shared/agent-note.txt` on This Mac, as in step 1.
2. Start the Copilot agent session on vm-a, which is a machine of the channel. vm-b is outside the channel and is not used.
3. Prompt (goal only, no syntax): "Bring the file agent-note.txt from this channel's shared folder on This Mac into this channel's shared folder on your machine with the humata CLI. Then reply with the command you ran and whether it worked."
4. Approve the copy-in request in the Inbox if the rule asks.

Expected result: the agent runs a copy of `this-mac:shared/agent-note.txt` into its own `shared/` on vm-a. The file lands in `shared/` on vm-a, and its sha256 matches files/f6-agent-src-sha256.txt. Nothing changes on This Mac or vm-b. If the agent first tries to send the file to another machine's `shared/`, it is refused at once with the pull hint, nothing is written, and it then pulls.

Rerun result: not yet run.

## Findings

1. BUG (Add machine, systemd Linux, blocks every new Linux machine): step `start` fails with exit 255 after `Created symlink`.
   - Cause: the daemon runs the setup script as `sh -c '<script>'` (daemon/machines/src/quote.rs:43 `sh_command`). The script contains the unit text `ExecStart=%h/.humata2/bin/humatad2 serve --role machine` (daemon/machines/src/service.rs, UNIT around line 63), and the `stray` helper runs `pkill -u ... -f '[h]umatad2 serve --role machine'` (service.rs HELPERS, around line 35). The pattern matches the `sh -c` process's own command line, so pkill kills the script's shell; ssh returns 255.
   - Repro on the VM (user vfiles): `ssh host 'bash -c "echo one; pkill -u \$(id -u) -f \"[h]umatad2 serve --role machine\"; echo after ExecStart=x humatad2 serve --role machine"'` prints `one` and exits 255. The same script on stdin (`bash -s`) survives and exits 0.
   - Expected: Add machine succeeds on a fresh Linux VM. Actual: fails, and rolls back humatad2.
   - Known: the lead agent reported it as fixed in tree, with binaries not yet rebuilt. This run used the pre-fix binaries.
   - Workaround used: pre-staged install and unit (see Setup). Does not exercise the start step.

2. BUG: `humata cp --dry-run` (and `mv --dry-run` not tried) starts a real transfer. Repro: `humata --channel files-flows cp --dry-run this-mac:shared/hello-host.txt vm-a` printed a transfer id with state running and copied the file (transfer tr_361595a5... done). Expected: print the plan and token, change nothing (CLI help: "Print the plan and its token; change nothing"). Code not located.

3. BUG (agent principal): machine-only destination fails with `the request has no destination.session_clone.session` for both an in-channel machine (vm-a) and an outside one (vm-b). Human principal works. Expected: a machine-only destination lands by rule (PL-3, PL-12). Code not located (check how the agent principal fills the destination in cli/src/location.rs; a bare `vm-a` is parsed as a session clone when the caller is an agent).

4. UX / possible bug: the agent principal's `humata host show vm-b` says "no machine is named vm-b" while the human shell sees it online. Likely the agent's machine list is limited to the channel's machines or to This Mac. Not verified beyond this run. The agent then told the user vm-b must be added, which is misleading.

5. UX: the agent's first action was `find / -name ...` across the whole disk, taking more than two minutes before the agent stopped it. Each bash call needs its own approval (13 for one copy), which makes an agent copy slow to approve.

6. UX: `cp --json` returns at once in state running; progress exists only by polling `humata transfers`. There is no progress stream for the CLI user.

7. Note: during a folder transfer the destination holds a `.humata-transfer-<id>/` staging folder with zero-byte files until the copy finishes. Expected per design, but it is visible in the tree.

8. Minor: restored trash items get mode 664 instead of the original 644.

9. Harness note (not a product finding): between runs the macOS CLI and daemon binaries in V/bin were overwritten with Linux builds (14:36) and then restored. The lane's first daemon kept running on the old image throughout.

## Summary

| flow | result |
|---|---|
| Setup: two remote machines (with pre-staged install workaround) | PASS (workaround); product Add machine BLOCKED by finding 1 |
| 1. host to remote, file and folder | PASS |
| 2. remote to host, file and folder | PASS |
| 3. remote to remote (vm-b outside channel) | PASS |
| 4. mv, trash, restore | PASS |
| Big file 50 MB: progress and cancel | PASS |
| 5. agent-driven copy with humata CLI | FAIL |

Pass count: 5 of 6 product flows (including the big-file test), 1 FAIL (flow 5). Setup needed the workaround.

## Rerun 2026-11-17 (build after D758)

Evidence: V/evidence/files/r2/. Machines: This Mac (lane daemon), vm-a (VM user vfiles) and vm-b (VM user vfiles2), both fresh users on humata2-linux. Channel `files-flows` (vm-a in it, vm-b outside). The lane's daemon state from the first run was still in its temporary folder, so the old channel and machines existed; I ran `host forget` on both and re-added them.

### Setup: host add on fresh users
`humata host add v-files-humata2-linux --name vm-a --trust-host-key <fp>` and the same for vm-b worked plainly, no workaround: connect, check, install, start all done in about 7 s each (the earlier Add-machine bug is fixed). Evidence: host-add-vm-a.json, host-add-vm-b.json.
Result: PASS

### Flow 4 (VF-58, VF-59): mv, trash and restore keep the file mode
Agent: none (CLI).
Steps: on vm-a shared made f644.txt (644), f600.txt (600), f755.sh (755) and d750 (750) holding in.txt (664) and ro555 (555) with deep.txt. Then `mv vm-a:shared/... vm-b`, `trash restore` of the four items, `files rm` of the four (the read-only folder too), `trash restore` again, and `mv vm-b:received/vm-a/f600.txt d750 vm-a`. `stat -c '%a %n'` taken before and after each step.
What happened: modes before (f4-before.txt): 600 f600, 644 f644, 755 f755, 750 d750, 664 in.txt, 555 ro555, 664 deep.txt.
- After mv on vm-b (f4-after-mv-vmb.txt): same modes for all seven.
- Trash on vm-a after mv (f4-vma-after-mv.txt): content files 644/600/755, d750 content 750, ro555 555. The sources left shared.
- After restore (f4-after-restore-vma.txt): modes identical to before, trash empty. The old 644 -> 664 bug is gone.
- `files rm` of the folder with the 555 subfolder trashed fine; restore (f4-after-rm-restore.txt) gave identical modes.
- Moving back (f4-mv-back.txt): 750, 555 and 600 kept.
Model seen: not applicable (no agent).
Result: PASS (VF-58 and VF-59)

### Flow 5 (D755, D756): agent-driven pull
Agent: copilot 1.0.113, claude-haiku-5.5. Machine: vm-a (session `files pull`, se_21f55fad...), and for the refusal checks This Mac (channel `files-host`, session `files mac`, se_73e91042...). Source: shared/agent-note.txt on This Mac, sha256 e8731ba7... (f5-src-sha256.txt).
Setup note: the channel clone on vm-a first failed because the daemon clones `https://github.com/verify/demo.git` and the VM cannot reach it (harness limit, local remotes). I put a `url.insteadOf` rewrite to a copy of the seeded bare repo in the VM user's gitconfig; remove and re-add of vm-a then cloned. This Mac's clone for files-flows was empty (stale lane state), so the host-side checks used a new channel `files-host`.

Steps and what happened:
1. Typed the pull prompt on vm-a (no paths or branches). First `session type` returned `typed:true, submitted:true`, but the prompt never reached Copilot (screen empty, session read empty, no events). It was sent about 20 s after start, while Copilot showed "MCP Servers reloaded". Typed again later and it worked (BUG-new-1).
2. The agent tried the Humata file tools, then bash: `humata cp --dry-run --machine "This Mac" "channel_folder:agent-note.txt" ...`, `humata host ls`, then with the hint "this-mac": `humata cp --dry-run --machine this-mac "shared:agent-note.txt" "shared:agent-note.txt"`. Every one failed with `humata.v1.MachineService.ListMachines isn't built into this humatad2 yet ... update Humata on this machine`, exit 7. The agent's file tool also returned `peer_version_unsupported` for reading This Mac's files. Nothing was copied; the file is not in vm-a's shared. (FAIL, BUG-new-2.)
3. Asked it to send its shared agent-note.txt to vm-b: `humata cp shared:agent-note.txt vm-b` also died with the ListMachines error, so the D755 pull hint was never reached on vm-a.
4. `humata cp shared/agent-note.txt vm-b.txt` on vm-a: same ListMachines error (exit 7) even for a dotted name; no `vm-b*` file in vm-a's clone.
5. On This Mac (agent on the host, where machine lookup works):
   - `humata cp shared/agent-note.txt vm-a` -> exit 4: "an agent writes shared/ only on the machine it runs on ..., so nothing was written to shared/ on vm-a. put the file in shared/ here; a session on vm-a can copy it in from there, or the user can send it with `humata cp`". Nothing found on vm-a. PASS for D755.
   - `humata cp shared/agent-note.txt vm-b` (vm-b hidden) -> usage refusal: `"vm-b" is not a machine of this channel, so nothing was copied`, with the `humata host ls`, `./vm-b`, `vm-b/` hint. Nothing on vm-b, no file in the clone. PASS for D756 refusal.
   - `humata cp note.txt vm-b.txt` (note.txt placed in the clone) -> "there is no vm-b.txt there; check the path". No file created in the clone. Expected: a dotted name lands as a new file in its own clone. FAIL (BUG-new-3).
6. Inbox: every bash call was one request; all approved with `humata inbox approve` (f5-approvals*.log, f5-mac-app*.log). No request went missing.
Model seen: claude-haiku-5.5 for both sessions (model-vma-session.txt: selectedModel, currentModel and 120 `model` lines claude-haiku-5.5; Copilot's own helper calls show gpt-5.4-nano, which is its utility model, and the Mac state dir shows gpt-5.6-luna-utility/luna as well; model-mac-session.txt).
Result: FAIL for the pull (D755 pull on vm-a blocked); PARTIAL overall: D755 refusal and D756 vm-b refusal PASS on the host, dotted name FAIL.

### Summary (rerun)
| flow | result |
|---|---|
| Setup: host add vm-a, vm-b (fresh users, plain) | PASS |
| 4. mv, trash, restore keep mode (VF-58, VF-59) | PASS |
| 5a. pull on vm-a into own shared | FAIL (blocked by ListMachines) |
| 5b. send to another machine refused with pull hint (D755) | PASS (host agent); not reachable from vm-a |
| 5c. `cp x vm-b` hidden machine refused, nothing written (D756) | PASS (host agent) |
| 5d. dotted name lands in own clone | FAIL |

Pass 4 of 6 (Setup, Flow 4, 5b, 5c).

### Findings (new)
1. BUG-new-1 `session type` reports `typed:true, submitted:true` but drops the text when Copilot is still starting (screen shows "MCP Servers reloaded"). Repro: `session start --machine vm-a`, wait 20 s, `session type`; empty prompt, no conversation. Expected: wait for the prompt or report not typed. Second type 30 s later worked.
2. BUG-new-2 (blocks every cross-machine agent copy from a remote machine): in a session on vm-a, any `humata cp ... <machine>` or `humata host ls` fails with "humata.v1.MachineService.ListMachines isn't built into this humatad2 yet" (exit 7), and the agent file tool gets `peer_version_unsupported` for a read on This Mac. Both daemons report 0.0.0 from the same build; the lane host daemon is `humatad2_admin`, the VM runs `humatad2`, so this may be harness-related, but a user would see the same on a real remote machine. The agent's advice ("update Humata on this machine") is the product's own message and is wrong: the machine is current. Expected: the machine-role daemon either serves ListMachines or the CLI asks the host (D743, D755, D756 depend on it). The D755 pull hint was not reachable from vm-a.
3. BUG-new-3 `humata cp note.txt vm-b.txt` in an agent session on the host: "there is no vm-b.txt there". Expected: `vm-b.txt` has a dot, so it is a path in the session's own clone and the file is created. Actual: treated as an existing destination; nothing copied.
4. GAP: stale lane state. The lane daemon keeps its state outside the lane folder, in a temporary folder, so a fresh lane directory still has the old channel and machines, and an empty This Mac clone ("could not run git ... No such file", session start fails). Harness fix: lane-up should clear it or the run should use another name.
5. GAP (harness/product): VM machine clones use https://github.com/{owner}/{name}.git even for a local-remote lane, so a channel with a local repo cannot clone on a VM without a gitconfig rewrite.
6. Not seen: no dropped link, no remote daemon restart, so channel_attempt (D758) retry was not exercised (no PASS/FAIL).

## Rerun 2 2026-11-17 (build 19:05, after D767)

Evidence: V/evidence/files/rerun2/. Lane `files` fresh (LANE_FRESH=1). Machines: This Mac and vm-a (new VM user vfiles on humata2-linux, `url.insteadOf` for verify/demo in its gitconfig). Channel `ff`. Session `pull` (se_3deed1f4...) on vm-a, copilot 1.0.113. Model seen: claude-haiku-5.5 (selectedModel/currentModel and 180 `model` lines; gpt-5.4-nano is Copilot's helper), `model-vma-session.txt`.

### Setup
`host add` and `channel machine add ff vm-a` worked first time. New: when the first `session type` hit Copilot's folder-trust screen it was refused with a clear message, and an Inbox question card (Yes / Yes, and remember / No) existed for it. `session type <s> ""` accepted the default. PASS (f5-type.json, trust-inbox.json).

### Flow 5 pull: agent on vm-a uses `humata host ls` and `humata cp` from This Mac
Steps: put shared/agent-note.txt on This Mac (src-sha.txt). Asked the vm-a agent to run `humata host ls`, then `humata cp this-mac:shared/agent-note.txt shared/agent-note.txt`; then variants with the machine id, 'This Mac', a destination `./pulled.txt`, `shared/`, `--dry-run`, and `humata files ls this-mac:shared`.
What happened:
- `humata host ls` works on vm-a and lists This Mac and vm-a. No "ListMachines isn't built", no peer_version_unsupported (D768 holds). PASS for that part.
- Every real pull fails, exit 2: `session se_... is on machine vm-a, not ma_99d28dc2af7d4d39b66439560ce8eee5` / "leave the machine out, or name the session's own". Same with `this-mac:`, the machine id, 'This Mac', destination `shared/agent-note.txt`, `shared/`, `./pulled.txt`, `pulled3.txt`. Nothing was copied (f1-screen*.txt, f5-screen*.txt).
- `--dry-run` of the same pull succeeds (resolves source as This Mac channel-shared, destination as the clone).
- `humata files ls this-mac:shared`: exit 4, "read files on another machine needs the user's approval ... this call can't wait for it in the Inbox". No Inbox item was raised (the hint says to use inbox_ask).
- Push: `humata cp note.txt this-mac` exit 2: "the path on ma_0ccff... leads outside Humata's folders ... use a path inside the session's clone or shared/". It is not the D755 pull-hint refusal.
- Host dir received/ now holds both `ma_99d28dc...` and `vm-a`: the VM daemon's own machine id differs from the host's name for it.
Result: FAIL (pull blocked; machine listing PASS)

### Flow 2 (D769, D756): dotted cp to a new name, in the agent's own clone on vm-a
Steps: in the clone (`demo`), `humata cp note.txt vm-b.txt`, `note.txt copy2.txt`, `note.txt vmx`, `note.txt ./vmx`.
What happened: `vm-b.txt`, `copy2.txt` and `./vmx` were created with identical md5 (f2-vm-ls.txt), transfers reached done. `vmx` (bare, no dot, not a machine) exited 2: `"vmx" is not a machine of this channel, so nothing was copied`, with the `humata host ls` / `./vmx` / `vmx/` hint. `./vmx` created the file in the clone.
Note: the agent starts in the session folder, not the clone: `humata cp note.txt x` run from there gave exit 3 "there is no note.txt there" because relative paths mean the clone (`demo/`). Correct by design, but agents trip on it.
Model seen: claude-haiku-5.5.
Result: PASS

### Summary (rerun 2)
| flow | result |
|---|---|
| setup (host add, channel machine, trust card) | PASS |
| 1. `host ls` on remote agent (D768) | PASS |
| 1. pull `this-mac:shared/x` into vm-a | FAIL |
| 2. dotted cp creates file in clone (D769) | PASS |
| 2. bare non-machine word refused (usage, exit 2) | PASS |
| 2. `./name` is the clone | PASS |

Pass 5 of 6.

### Findings (new)
1. BUG-new-4: an agent on a remote machine cannot copy from another machine into its own clone or shared. Repro: session on vm-a, `humata cp this-mac:shared/agent-note.txt ./pulled.txt`. Expected: file lands in the clone (or a copy-in card). Actual: exit 2 `session X is on machine vm-a, not ma_99d28...`. `--dry-run` passes, so the failure is in the real run on the host: the destination clone location carries the VM daemon's own machine id (ma_99d28...) while the session record says "vm-a" (daemon/humatad/src/files/roots.rs:171-178, `given != session_machine`). Same-machine cp on vm-a works (Flow 2).
2. BUG-new-5: `humata cp note.txt this-mac` (bare machine, from vm-a) gives "path on This Mac leads outside Humata's folders", not the D755 pull-hint or a send to received/.
3. GAP: `humata files ls this-mac:shared` from a remote agent is refused with "can't wait for it in the Inbox" and raises no Inbox card, so the person is never asked.
4. GAP: `session type` returns `agent_busy:true, queued:null` even on an idle fresh session (see copilot-t1.md).
5. Not seen: dropped link / channel_attempt retry (D758) not exercised.

# Copilot e3 results

## E2E 0.2.3 2026-11-18 (remote, humata2-linux, fresh VM user ve3)

Agent: copilot 1.0.113, Model seen: claude-haiku-5.5 (VM ~/.copilot/session-state, evidence model-c1.txt). Tools on the VM match V/bin-023-tools/linux-aarch64 by sha256 (vm-tools.txt); daemon_version reports 0.0.0 on both sides. Evidence: V/evidence/e3/e2e023/.
Setup notes: a git insteadOf shim for verify/demo was needed on the VM and the lane HOME; the channel's Mac clone failed under LANE_REMOTE=github (shim added after). Copilot asks "Confirm folder trust" on every start and every resume (Inbox question; answered on screen).

### Case 1: stop and resume (agent half PASS, shell half GAP)
Steps: start c1 on the VM, ask it to create hello.txt and remember "marmalade"; stop; ps; resume; ask for the secret word.
Stop left no holder, copilot or mcp process (c1-ps-after-stop.txt); record shown stopped:true. Resume reused the same copilot session id and it answered "Marmalade". Shell half: GAP, no CLI opens a shell tab on a remote machine or inside a session.

### Case 2: end for good (PARTIAL/FAIL)
End archived c1 and left no process (PASS). Resume after End was accepted, started a holder with session:null and the op failed; restore then ran 2 minutes and failed, session stayed archived (FAIL, known in flight).

### Case 4: discovery and adopt (PARTIAL)
Orphan listed with session:null, channel:null (PASS). Adopt: GAP (no CLI). Attach to an orphan: refused. channel delete with a remote machine: unimplemented. host forget leaves an unendable agent holder running.

| case | result |
|---|---|
| 1 stop/resume, agent | PASS |
| 1 shell tab | GAP |
| 2 end for good | FAIL (resume after End allowed) |
| 4 discovery/adopt | PARTIAL (adopt GAP) |

Findings (new): see the hand-back; evidence files c2-*, c4-* in the evidence folder.

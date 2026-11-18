# LIVEHOLDER: My terminals keep running: a version bump, a rename or a move never ends them

- **Status:** open (owner, 2026-11-18: "when doing a version bump is there a way to gracefully transfer terminal sessions over so they don't end and restart? Add that to the roadmap")
- **Depends on:** RELEASE3 (the released holder and the "Migrate the holders" item, D373).
- **Owner rule, 2026-11-18:** "Instead of killing terminals. As much as possible I want terminals to keep running basically." Humata ends a running terminal only when the owner ends it (End, End for good, Stop) or when it can't be kept, and then it says why.
- **Unblocks:** a quieter upgrade path. Migrate the holders stops being a restart.
- **Layer:** L13 · **Lane:** daemon/holder + daemon/humatad upgrade + revive

## Today
- **A daemon upgrade or crash leaves holders untouched** (PER-5).
- **A new holder build restarts agent sessions.** "Migrate the holders" (D373) stops each idle agent session and resumes its conversation on the new holder. The program restarts, and anything not saved in the conversation is lost: running tool calls, shell state, environment.
- **Shells stay on the old holder**, without the newer frames (D753), until they are reopened.

## Outcome
After an upgrade, Migrate the holders moves each running terminal, agent or shell, onto the new holder build in place:
- the program keeps its pid and never sees a hangup;
- the screen and scrollback carry over;
- every viewer reconnects by itself.

Nothing is restarted and nothing is resumed. Only a holder from before this milestone, which can't hand itself over, still needs today's restart, one last time.

## Design to settle in the lane (record as a decision)
- **Re-exec in place (preferred).**
  - The holder writes its state to a handoff file or memfd:
    - the terminal emulator state, scrollback and size;
    - the save log position;
    - its socket and the PTY master as kept file descriptors;
    - the child pid.
  - It then `execve`s the new `humata2_holder` with `--take-over <state>`, keeping those descriptors open across the exec.
  - The pid stays the same, so the agent or shell is still its child: `waitpid` and exit status keep working, and nothing reparents.
- **Rejected alternative: a new holder process taking the descriptors over `SCM_RIGHTS`.** The child would be reparented to launchd, losing `waitpid` and the exit code. Note why in the decision.
- **The handoff format is versioned.** The new build must read every handoff version it may receive. If it can't, it refuses before the exec and the old holder keeps running: the force-upgrade rule applies, with no mixed-version running.
- **Failure safety.** If the new binary fails to start, an exec failure returns to the old code, which keeps serving. A crash after the exec is the case to test: the state must still be reattachable or the session reported, never silently lost.
- **Viewers and the daemon** see a short disconnect and reattach by the session's socket, the same path as a daemon restart.
- **Remote machines** follow the same path. The host's Migrate sends `Upgrade{thing = HOLDERS}` (D378), and the box re-execs its own holders.

## The audit: every path that ends a holder today
Each path below either keeps the terminal running or records why it can't, as a decision. The default is to keep it running.

- **On the same machine, the process survives a rename.** A same-filesystem `rename(2)` keeps every open file and the working directory valid. A folder rename or move on one machine should rename under the running program and update the record, without restarting it.
- **Paths to check:**
  - `channel_service/rename_folder.rs`: a channel folder rename. Keep the terminal running.
  - `desk_service/move_to_channel.rs`: a Desk session moved into a channel on the same machine. Keep it running.
  - `session_service/move_session.rs`, same machine: a relocation (D757 moves carry paths). Keep it running, and stop only when the move crosses machines.
  - `session_service/archive.rs`: Archive. Decide whether Archive should leave a live terminal running (hidden, still accounted for) instead of ending it.
  - `desk_service/close.rs`: closing a Desk tab. Closing a view must not end the program unless End was chosen.
  - `channel_service/delete.rs`: deleting a channel with live terminals. Ask, and offer keeping them running elsewhere (on the Desk).
  - `carry_service.rs`, `away/uplink.rs`, `fault.rs`, `terminal.rs`, `running_holders.rs`, `stop/`: list what ends a holder in each, keep it running where possible, and make sure no automatic path ends one (D784: sessions are accountable and never auto-killed).
- **Across machines, the program can't move.** A cross-machine move still stops and resumes, but LIVEHOLDER should keep the old terminal running until the new one is confirmed up.
- **Write a test per path:** the program's pid is unchanged after the action.

## Exit
- Tests: a shell and a stand-in agent run through a holder re-exec onto a newer build:
  - same pid, no SIGHUP;
  - screen and scrollback identical;
  - typed input after the re-exec reaches the program;
  - the exit code is still reported when the program exits.
- Tests: a failed exec, a refused handoff version, and a crash right after the exec. Each leaves the session running or clearly reported.
- "Migrate the holders" wording changes from "restart" to "move", and lists only holders too old to move.
- Slow tier: a live SSH session on h2-terminal-c1 survives a holder bump there.

## Pointers
D373 (Migrate the holders), D378 (box holders), D753 (old holder frames), PER-5/PER-6, `daemon/holder/`, `daemon/humatad/src/upgrade/holders.rs`, revive.

## Out of scope
Moving terminals between machines (that's a move, D757). Keeping a session alive across a reboot: the program can't survive a reboot, and the restart reports cover it.

# MACHINEARCHIVE: Archiving a stopped session from a machine's page always archives it or says why not

- **Status:** queued. It ships with NEWCHCLOSE in **0.10.2** (owner bug report, 2026-11-20).
- **Depends on:** FUSEFSWIRE (owner's order)

## Bug (owner, 2026-11-20)
"why is it when i archive closed sessions or stopped sessions or whatever in the hosts screen sometimes it doesnt delete. Is it that the state was errored to begin with or something?"

## What main found reading the code (not yet reproduced)
- The path is: Machine page row ⋯ › Archive goes to `RunningDelegate::ArchiveSession`, then `features/app/src/machines.rs:127`, then `desk.rs` `archive_session`.
  - That function temporarily swaps `state.session.open` to the row's session, sends `SessionAction::Archive`, and restores it.
  - Then `plan_archive` runs, and either the archive or the frame-65 dialog follows.
  - On success, the Running list reloads (`features/app/src/lib.rs` ~757).
- **Likely cause 1, the strongest:** every failure lands in `state.session.error`.
  - That covers both a `plan_archive` error and an `Archived` error, for example a session whose clone or machine is in an error state, or a daemon refusal.
  - Only the session page draws `state.session.error` (`ui/workspace/src/session_tabs.rs:705`).
  - So on the Machine page a failed archive is **silent**: the row just stays.
  - This fits the owner's guess that the session was already in an error state.
- **Likely cause 2:** the action carries no session id. `SessionAction::Archive` reads `state.open`, which `archive_session` swaps in and then restores before the effect runs. Check that no later step of the flow (ArchivePlanned, ConfirmArchive, Archived, or the `ending_for_good` bookkeeping) depends on `state.open` or on per-open-session state, so that a Machine-page archive can't be dropped or matched to the wrong session.
- **To check:** Desk sessions go to `DeskAction::AskClose`, so confirm that dialog shows on the Machine page. Also check whether the daemon's running-holder listing still returns an archived session's holder, which would bring it back after the reload.

## Daemon-side suspects (state machine sweep finder 05, not yet verified)
- **Archive refuses some sessions.** A session left in Launching or Resuming with no start running is refused with "still starting" / "being resumed" (`daemon/humatad/src/session_service/archive.rs:536-539`). Stop handles that same state. This fits "the state was errored to begin with".
- **The folder may never be freed.** Archive's "end terminals" step only stops agents that are Running, so the holder of an Exited or ResumeFailed agent keeps answering. `free_folder` then returns early (archive.rs:1125, :1348; watch.rs:329). It is deferred silently and never retried until a restart. Failures are only logged.
- **Stop does nothing.** Stop on an Exited or ResumeFailed session returns success and changes nothing (session_service.rs:1101).
- **A double archive can undo itself.** Two archive calls without an idempotency key can turn an Archived row back to Archiving (archive.rs:510, :673).

## Verified root cause (Opus verifier, 2026-11-20; see workflow/STATE_MACHINE_BUGS.md)
Any one of three causes is enough:
1. **The archive works but the row stays.**
   - The app always archives with `end: None`, so the session's shells, and the holder of an agent that already exited, keep running by design.
   - The daemon lists them with `archived` set (`daemon/svc_terminal/src/running_holders.rs:372-400`), but it hard-codes `archived: false` for stopped shells (`:446-448`).
   - The client's `holder()` drops the flag (`clients/live_daemon/src/running.rs:29-62`).
   - The "sometimes" is whenever a shell or an exited holder was still alive.
2. **The archive is refused and nobody sees why.**
   - Plan and archive errors go to `state.session.error` (`features/session/src/lib.rs` ~2008, ~2062). The Machine page draws only `running.error` (`ui/machines/src/machine_running.rs:361`).
   - The refusals on this path:
     - AlreadyArchived after a double click;
     - "still starting" or "being resumed" for a session stuck in Launching or Resuming with no start running (`archive.rs:536-540`);
     - git or store failures;
     - a pending approval.
3. **End for good ends nothing.**
   - On the Machine page, End for good sends the same archive (`features/hosts/src/running.rs:250-262`).
   - `Daemon::archive` always sends `end: None` (`clients/live_daemon/src/daemon.rs:650-675`).

**Fix:**
- Map `archived` into `RunningHolder`, draw those rows apart (with End), and set the flag on stopped shells.
- Show archive errors on the Machine page.
- Give `Daemon::archive` an `end` argument; End for good sets it.
- Let archive accept a stale Launching or Resuming state the way Stop does.
- Carry the session id in the action.

**Tests:**
- `//apps/humata2/features/app:machines_test`
- `//apps/humata2/daemon/svc_terminal:svc_terminal_unit_test`

## Exit criterion
- Reproduce with failing-first TestStore tests in features/app:
  - a stopped session whose `plan_archive` or archive fails;
  - one with unpushed work;
  - a Desk session.
- Fix it so that from the Machine page every archive either removes the row or shows the error there, saying what failed and how to fix it.
- Make the archive action carry its session id instead of relying on the swapped `state.open`.
- Main's gate passes; it ships in 0.10.2.

## Out of scope
- Redesigning the Machine page.

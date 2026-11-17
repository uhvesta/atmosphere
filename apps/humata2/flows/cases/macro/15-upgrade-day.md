# A new release on this Mac holds back box-1 until its VIP Migrate item is answered, and then box-1 upgrades in place

Layer: macro. Platforms: macOS for this Mac (the host and the app); box-1 is Linux. The web build shows the same Inbox items but cannot install a release.

## Starting state

- This Mac runs Humata 2 at release 2026.11.10. Its daemon, CLI and app are at that release.
- box-1 is a Humata machine at release 2026.11.10 too. Its sessions are running on its holders.
- I install release 2026.11.16 on this Mac. The app is put in Applications and its daemon, CLI and holder are staged beside the old ones in `~/.humata2/bin`.
- box-1 is still on 2026.11.10. Its link to this Mac is up.
- The Inbox is open on this Mac. Nothing is answered yet.
- The channel acme has the session "api rate limiter" on box-1. I want to start a second session there.
- An agent on box-1 is running in "api rate limiter". It has an open request to me in the Inbox.

## Steps

1. **I do:** install the new release and open the app from Applications.
   **I see:** the app opens on the new release. The Inbox shows an item at the top: "Migrate the daemon on box-1", marked VIP. Whether the item's text says what it changes and what keeps running is not checked by any test.
   **State and events:** the host sees box-1 at another release. The host posts one VIP Migrate item for box-1 (no requester).
   **Status:** Works (real daemon) for the item: `daemon/humatad/tests/upgrade_inbox.rs::a_machine_at_another_release_is_one_vip_migrate_item_with_no_requester`. The VIP mark is set in `daemon/humatad/src/upgrade/items.rs` (around line 599, `vip: true`). Not built for the card: the Inbox card with its one Migrate action is UPGRADE U7, and no file in `ui/src`, `features/` or `clients/src` names Migrate. The app's own item for this Mac's daemon text is in `desktop/src/main.rs` (around line 178); it is not checked by a test.

2. **I do:** open the Inbox card for box-1.
   **I see:** the card says Migrate the daemon on box-1, what it changes (box-1's release and its daemon, CLI and holder), and what keeps running (the sessions, which stay on their holders until the migrate is done).
   **State and events:** the card's phase is pending.
   **Status:** Not built (UPGRADE U7). `workflow/milestones/UPGRADE.md` lists U7 as an open task: "the Inbox card, its Migrate action and the machine page show migrating, migrated, failed and read-only". The reducers named there (`features/inbox/tests/migrate.rs`, `features/hosts/tests/upgrade.rs`) do not exist.

3. **I do:** try to start a second session "docs" on box-1 from the machine page.
   **I see:** the start is refused. The message says box-1 is at 2026.11.10, this host is at 2026.11.16, nothing runs across releases until box-1 is migrated, and to open "Migrate the daemon on box-1" in the Inbox and click Migrate.
   **State and events:** nothing starts on box-1. The refusal carries the Migrate item's id, so the app can point to it.
   **Status:** Works (real daemon). `daemon/humatad/tests/upgrade_refusals.rs::starting_a_session_on_a_machine_at_another_release_is_refused_with_its_item`. The message text is in `daemon/humatad/src/upgrade/gate.rs` (lines 73 and 114). The app's own pointer from the refusal to the item is Code, no test (U2's app side is not in `ui/src`).

4. **I do:** in "api rate limiter" on box-1, type a line into the agent's terminal (or attach with input).
   **I see:** the typing is refused, and the refusal names the Migrate item. Reading the terminal (attaching without input) still works.
   **State and events:** nothing is written to the session. A writable attach to box-1's session is refused the same way.
   **Status:** Works (real daemon). `daemon/humatad/tests/upgrade_refusals.rs::typing_into_a_session_on_a_machine_at_another_release_is_refused_with_its_item` and `daemon/humatad/tests/upgrade_refusals.rs::a_writable_attach_to_a_machine_at_another_release_is_refused_with_its_item`.

5. **I do:** read box-1's channel state, or the session's turns, from this Mac.
   **I see:** the reads answer. Nothing says "migrate first".
   **State and events:** reads are served from box-1's daemon at its own release.
   **Status:** Works (real daemon). `daemon/humatad/tests/upgrade_refusals.rs::a_machine_at_another_release_can_still_be_read`.

6. **I do:** run `humata ls` in a terminal on box-1, using box-1's old CLI (the one from 2026.11.10).
   **I see:** the CLI reports that it has to be migrated, and names the item to open. Exit code 7.
   **State and events:** the daemon refuses the call with migration_needed and names the item. Nothing is run.
   **Status:** Works (real daemon) for the refusal: `daemon/humatad/tests/upgrade_refusals.rs::a_cli_at_another_release_is_refused_with_migration_needed`. The exit code is a table entry: `cli/src/exit.rs:77` maps MigrationNeeded to PRECONDITION (7), and `cli/tests/exit_codes.rs` checks the table (Fakes only, no shell). A CLI at this Mac's release is served: `daemon/humatad/tests/upgrade_refusals.rs::a_cli_at_the_daemons_release_is_served`.

7. **I do:** the agent in "api rate limiter" runs `humata inbox approve <id>` on the Migrate item.
   **I see:** the agent's call is refused. The item stays where it was.
   **State and events:** nothing changes. The agent's refusal is recorded as denied.
   **Status:** Works (real daemon). `daemon/humatad/tests/upgrade_inbox.rs::an_agent_cannot_answer_a_migrate_item`.

8. **I do:** in the Inbox, click **Answer all** (or run `humata inbox approve --all`; `cli/tests/inbox.rs::approve_all_answers_only_what_is_pending_now` covers the CLI form).
   **I see:** the other requests are answered. The Migrate item is still waiting, with its Migrate action.
   **State and events:** Answer all leaves a Migrate item waiting. A Migrate item takes no answer but Migrate: no Deny, no Always, no reply (inbox.proto, lines 135 and following).
   **Status:** Works (real daemon). `daemon/humatad/tests/upgrade_inbox.rs::answering_all_leaves_a_migrate_item_waiting` and `daemon/humatad/tests/upgrade_inbox.rs::a_migrate_item_takes_no_answer_but_migrate`. The Answer all button in the app is Fakes only, if covered by `features/inbox/tests/`; the Migrate-only rule is not drawn in the app (U7 not built).

9. **I do:** click **Migrate** on the box-1 card (or run `humata inbox approve <id>` for it).
   **I see:** the card shows migrating. Then box-1 shows migrated, and the card reads "Migrated to 2026.11.16".
   **State and events:** the host stages the 2026.11.16 release on box-1 over the SSH path that installed box-1 (`.next` files, checked on box-1 before anything is swapped). box-1's daemon swaps the staged release in and restarts with the new release. The card is approved with "Migrated to …" (`daemon/humatad/src/upgrade/items.rs:208`).
   **Status:** Works (real daemon) for the stage over SSH and the come-back: `daemon/humatad/tests/upgrade_box.rs::approving_migrate_stages_the_release_over_ssh_and_the_machine_comes_back_at_it` and `daemon/humatad/tests/upgrade_box.rs::a_release_staged_over_ssh_becomes_the_machines_release_once_migrated`. The SSH is a test shim, not a real ssh (the remote rig uses an ssh shim: see `workflow/milestones/FIND.md`, the remote rig note). The card's migrating and migrated phases on screen are Not built (U7).

10. **I do:** wait until box-1's sessions show their status again.
    **I see:** "api rate limiter" is still running on box-1, now at the new release.
    **State and events:** the swap restarts the daemon and the holders keep their sessions (the design; the local test checks it on this Mac).
    **Status:** Works (real daemon) for this Mac: `daemon/humatad/tests/upgrade_local.rs::an_upgrade_swaps_the_staged_release_in_and_restarts_the_daemon_with_its_holders_untouched`. For box-1 the test checks the staged files (`daemon/humatad/tests/upgrade_box.rs`, the holder file is staged as `.humata2_holder.next`); it does not check the running session's pid or that its sessions keep running. Old holders moving sessions to the new holder when idle: Not built (UPGRADE U6). No `upgrade_holders.rs` exists and `src/upgrade/holders.rs` is absent.

11. **I do:** start the second session "docs" on box-1.
    **I see:** the start goes through; the Migrate item is gone from the Inbox.
    **State and events:** the item resolves when box-1 reports the host's release. `daemon/humatad/tests/upgrade_inbox.rs::a_machine_back_at_the_hosts_release_withdraws_its_item_as_resolved`.
    **Status:** Works (real daemon). The start itself after the migrate: `daemon/humatad/tests/upgrade_refusals.rs::a_cli_at_the_daemons_release_is_served` covers the release check, and the start path is not separately tested after a migrate.

12. **I do:** on this Mac, the app's own item for "Migrate the daemon on This Mac" (when the local daemon is older than the app) is answered with Migrate.
    **I see:** the local daemon restarts at the new release. My sessions on this Mac stay on their holders.
    **State and events:** the local daemon swaps in the staged release from `~/.humata2/bin`, restarts, and its holders are not touched.
    **Status:** Works (real daemon) for the swap, with fake launchd: `daemon/humatad/tests/upgrade_local.rs::an_upgrade_swaps_the_staged_release_in_and_restarts_the_daemon_with_its_holders_untouched` and `daemon/humatad/tests/upgrade_local.rs::the_upgrade_record_is_settled_into_last_json_once_the_new_daemon_is_ready`.

## Branches

### The upgrade is cut by a restart in the middle

- **When:** box-1's daemon or power is cut after the upgrade was recorded and before the swap is finished, or after the new daemon opened the store.
- **I do:** power off box-1 in the middle of step 9 and turn it on again.
- **I see:** box-1 comes back whole, on one release. Either the old or the new, never neither. The card shows the state it reached (migrating, then failed or migrated, once the UI is built).
- **State and events:** a rollback at start (`daemon/humatad/src/serve.rs:296`) puts the old release back if the new one did not finish. The upgrade record is kept until it is settled.
- **Expected outcome:** box-1 is never left without a daemon; the Migrate item says what state it reached.
- **Status:** Works (real daemon, power-cut rig). `daemon/humatad/tests/upgrade_cut.rs::a_cut_after_the_upgrade_was_recorded_and_before_anything_was_swapped_leaves_release_a`, `daemon/humatad/tests/upgrade_cut.rs::a_cut_between_the_daemon_and_the_bundle_being_swapped_leaves_release_a_whole`, `daemon/humatad/tests/upgrade_cut.rs::a_new_build_cut_after_keeping_the_store_copy_is_rolled_back_to_release_a`, and `daemon/humatad/tests/upgrade_cut.rs::a_new_build_cut_after_opening_the_store_at_the_new_schema_is_rolled_back_to_release_a`. The card's failed or retry state on screen is Not built (U7).

### A migrate that never comes back

- **When:** box-1 does not come back at the new release in time (the deadline the item sets).
- **I do:** wait.
- **I see:** the item goes back to pending, marked Failed, with the error. Migrate can be clicked again.
- **State and events:** `daemon/humatad/src/upgrade/items.rs:302` calls `stalled` after the deadline; `items.rs:285` marks the card Failed.
- **Expected outcome:** nothing half done; the item says what failed.
- **Status:** Code, no test. `daemon/humatad/src/upgrade/items.rs` (lines 298-312, `expect_by` and `stalled`) has the deadline and the Failed phase. No test named in `daemon/humatad/tests/upgrade_*.rs` covers a stall or a Failed card.

### A machine changes release again while its item waits

- **When:** box-1 is upgraded by hand, or to a third release, while the item is still pending.
- **I do:** nothing.
- **I see:** the same one item stays; it now says what box-1 is at.
- **State and events:** the item is updated, not posted again.
- **Expected outcome:** one item per machine.
- **Status:** Works (real daemon). `daemon/humatad/tests/upgrade_inbox.rs::a_machine_that_changes_release_again_keeps_its_one_item_showing_what_it_is_now`.

### A machine whose binary is off its bundle

- **When:** box-1's daemon binary does not match the release bundle it claims.
- **I do:** nothing.
- **I see:** box-1 counts as another release, and the item is posted.
- **State and events:** the daemon names no bundle for a binary that differs from its own entry.
- **Expected outcome:** the mismatch is caught as a release mismatch.
- **Status:** Works (real daemon). `daemon/humatad/tests/upgrade_inbox.rs::a_machine_whose_binary_is_off_its_bundle_counts_as_another_release` and `daemon/humatad/tests/upgrade_identity.rs::a_daemon_whose_binary_differs_from_its_entry_names_no_bundle`.

### Identity changes (a different machine answers as box-1)

- **When:** the machine that answers as box-1 is not the one that was enrolled (a new machine with the same name, or a changed key).
- **I do:** nothing.
- **I see:** the daemon names no bundle for the wrong machine. The link reports REACHABILITY_IDENTITY_MISMATCH with no retry (LD-12, `OVERVIEW.md:564`). The text telling me to add the machine again is not built (see Gaps).
- **State and events:** a Hello from a machine off its bundle names no bundle.
- **Expected outcome:** no migrate is offered to a machine the host cannot prove is box-1.
- **Status:** Works (real daemon) for the bundle rule: `daemon/humatad/tests/upgrade_identity.rs::the_hello_reply_of_a_machine_off_its_bundle_names_no_bundle`. The app's "add it again" text for IDENTITY_MISMATCH is Not built: `OVERVIEW.md:565` lists it as open (LD-12), and no milestone is named there.

### This Mac's daemon is the old one and a second install is staged

- **When:** the app and the daemon on this Mac disagree in release.
- **I do:** nothing.
- **I see:** the app's own item says the local daemon is older than the app, and the old one keeps running until Migrate.
- **State and events:** the new release is staged in `~/.humata2/bin` beside the old one.
- **Expected outcome:** the old one keeps running until Migrate.
- **Status:** Works (real daemon) for the staging and the swap: `daemon/humatad/tests/upgrade_local.rs::files_that_are_not_staged_are_refused_with_nothing_changed`. The text on the local item is in `desktop/src/main.rs` (around lines 174-178); it is Code, no test.

### A wrong or missing staged file

- **When:** the staged file is not the one the bundle names, or the release asked for is not the one staged.
- **I do:** click Migrate.
- **I see:** the migrate is refused and nothing changes on disk.
- **State and events:** nothing is swapped.
- **Expected outcome:** refused with nothing changed.
- **Status:** Works (real daemon). `daemon/humatad/tests/upgrade_local.rs::a_staged_file_that_is_not_the_one_the_bundle_names_is_refused_with_nothing_changed` and `daemon/humatad/tests/upgrade_local.rs::a_release_that_is_not_the_staged_one_is_refused_with_nothing_changed`.

### Migrate to the release already running

- **When:** the item is answered when box-1 is already at the host's release.
- **I do:** click Migrate.
- **I see:** nothing changes; the item resolves.
- **State and events:** no restart.
- **Expected outcome:** no restart and no duplicate upgrade.
- **Status:** Works (real daemon). `daemon/humatad/tests/upgrade_local.rs::an_upgrade_to_the_release_the_daemon_already_runs_changes_nothing`.

### An agent or a script tries to upgrade the daemon

- **When:** an agent or a script calls the upgrade directly, outside the Inbox.
- **I do:** nothing; the agent calls it.
- **I see:** refused.
- **State and events:** nothing is staged or swapped.
- **Expected outcome:** only the user, through the Migrate item, upgrades a daemon.
- **Status:** Works (real daemon). `daemon/humatad/tests/upgrade_local.rs::an_agent_or_a_script_cannot_upgrade_the_daemon`.

### Sessions on an old holder

- **When:** box-1's holder is the old one and a session is idle.
- **I do:** nothing.
- **I see:** the session moves to the new holder when idle; shells stay until closed.
- **State and events:** a holder move.
- **Expected outcome:** the move happens when idle, and shells stay.
- **Status:** Not built (UPGRADE U6, "Sessions on an old holder move to the new one when idle, shells stay until closed"). No `daemon/humatad/tests/upgrade_holders.rs` and no `daemon/humatad/src/upgrade/holders.rs`.

## Expected outcomes

- [ ] A VIP Migrate item appears for box-1 when it is at another release, once.
- [ ] box-1 refuses to start, type into, or writably attach a session, and each refusal points to the item.
- [ ] box-1 still serves reads.
- [ ] box-1's old CLI is refused with migration_needed, and exits 7.
- [ ] An agent cannot answer the Migrate item.
- [ ] Answer all leaves the Migrate item waiting.
- [ ] Migrate on box-1 stages the release over SSH and box-1 comes back at the new release.
- [ ] box-1's holders and sessions keep running through the swap.
- [ ] Once box-1 is at the host's release, the item is withdrawn.
- [ ] A cut upgrade leaves one whole release running.
- [ ] This Mac's daemon swaps its staged release in without touching its holders.
- [ ] Old-holder sessions move to the new holder when idle (not built).

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Install; VIP Migrate item posted | Works (real daemon) | `daemon/humatad/tests/upgrade_inbox.rs::a_machine_at_another_release_is_one_vip_migrate_item_with_no_requester` |
| 2. Inbox card with Migrate action | Not built (UPGRADE U7) | `workflow/milestones/UPGRADE.md`; no `Migrate` in `ui/src` |
| 3. Start on box-1 refused with item | Works (real daemon) | `daemon/humatad/tests/upgrade_refusals.rs::starting_a_session_on_a_machine_at_another_release_is_refused_with_its_item` |
| 4. Typing and writable attach refused | Works (real daemon) | `daemon/humatad/tests/upgrade_refusals.rs::typing_into_a_session_on_a_machine_at_another_release_is_refused_with_its_item`; `...::a_writable_attach_to_a_machine_at_another_release_is_refused_with_its_item` |
| 5. Reads still work | Works (real daemon) | `daemon/humatad/tests/upgrade_refusals.rs::a_machine_at_another_release_can_still_be_read` |
| 6. Old CLI refused with migration_needed, exit 7 | Works (real daemon) for refusal; Fakes only for the exit table | `daemon/humatad/tests/upgrade_refusals.rs::a_cli_at_another_release_is_refused_with_migration_needed`; `cli/src/exit.rs:77`; `cli/tests/exit_codes.rs` |
| 7. Agent cannot answer the item | Works (real daemon) | `daemon/humatad/tests/upgrade_inbox.rs::an_agent_cannot_answer_a_migrate_item` |
| 8. Answer all leaves it waiting | Works (real daemon) | `daemon/humatad/tests/upgrade_inbox.rs::answering_all_leaves_a_migrate_item_waiting` |
| 9. Migrate stages over SSH and box-1 comes back | Works (real daemon, SSH shim) | `daemon/humatad/tests/upgrade_box.rs::approving_migrate_stages_the_release_over_ssh_and_the_machine_comes_back_at_it` |
| 10. Holders survive (this Mac) | Works (real daemon, fake launchd) | `daemon/humatad/tests/upgrade_local.rs::an_upgrade_swaps_the_staged_release_in_and_restarts_the_daemon_with_its_holders_untouched` |
| 10b. Old holders move when idle (U6) | Not built | UPGRADE.md U6 |
| 11. Item withdrawn once at the host's release | Works (real daemon) | `daemon/humatad/tests/upgrade_inbox.rs::a_machine_back_at_the_hosts_release_withdraws_its_item_as_resolved` |
| 12. This Mac's local daemon migrates | Works (real daemon) | `daemon/humatad/tests/upgrade_local.rs::the_upgrade_record_is_settled_into_last_json_once_the_new_daemon_is_ready` |
| B1. Cut by restart or power | Works (real daemon, power-cut rig) | `daemon/humatad/tests/upgrade_cut.rs::a_cut_between_the_daemon_and_the_bundle_being_swapped_leaves_release_a_whole` |
| B2. Migrate never comes back (stall, Failed) | Code, no test | `daemon/humatad/src/upgrade/items.rs:285, 298-312` |
| B3. Machine changes release again | Works (real daemon) | `daemon/humatad/tests/upgrade_inbox.rs::a_machine_that_changes_release_again_keeps_its_one_item_showing_what_it_is_now` |
| B4. Binary off its bundle | Works (real daemon) | `daemon/humatad/tests/upgrade_inbox.rs::a_machine_whose_binary_is_off_its_bundle_counts_as_another_release` |
| B5. Identity changes | Works (real daemon) for the bundle rule; Not built for the app text | `daemon/humatad/tests/upgrade_identity.rs::the_hello_reply_of_a_machine_off_its_bundle_names_no_bundle`; `OVERVIEW.md:565` |
| B6. Local item text and staging | Code, no test (text); Works (real daemon) for staging | `desktop/src/main.rs` around line 178; `daemon/humatad/tests/upgrade_local.rs::files_that_are_not_staged_are_refused_with_nothing_changed` |
| B7. Wrong or missing staged file | Works (real daemon) | `daemon/humatad/tests/upgrade_local.rs::a_staged_file_that_is_not_the_one_the_bundle_names_is_refused_with_nothing_changed` |
| B8. Migrate to the running release | Works (real daemon) | `daemon/humatad/tests/upgrade_local.rs::an_upgrade_to_the_release_the_daemon_already_runs_changes_nothing` |
| B9. Agent or script upgrades the daemon | Works (real daemon) | `daemon/humatad/tests/upgrade_local.rs::an_agent_or_a_script_cannot_upgrade_the_daemon` |
| B10. Old holder, idle session | Not built (UPGRADE U6) | `workflow/milestones/UPGRADE.md` U6 |

## Gaps and likely bugs

**Not built**
- The Inbox card for a Migrate item, its one Migrate action, and the migrating, migrated, failed and read-only phases on the card and the machine page (UPGRADE U7). No Migrate text in `ui/src`, `features/` or `clients/src`, and the named reducer tests do not exist.
- Sessions on an old holder move to the new holder when idle, and shells stay until closed (UPGRADE U6). No `daemon/humatad/src/upgrade/holders.rs` and no `daemon/humatad/tests/upgrade_holders.rs`.
- The app's "add it again" text for IDENTITY_MISMATCH and the machine view's `last_seen_at` and `next_retry_at` (LD-12, `OVERVIEW.md:565`). No milestone named.
- The app's pointer from a refused start to its Migrate item (U2's app side). Code, no test.

**Likely bugs**
- None confirmed by reading alone. The stall and Failed path has code (`daemon/humatad/src/upgrade/items.rs:285, 298-312`) but no test, so it is Code, no test rather than a bug.

**Code, no test**
- The stall deadline and the Failed phase on a Migrate item (`daemon/humatad/src/upgrade/items.rs:285, 298-312`).
- The local item's text in `desktop/src/main.rs` (around lines 174-178).

**Corrections to the brief**
- The brief says "check not_built at daemon.rs:340". That not_built is a fallback for a daemon that has no upgrade service and no link (`daemon/humatad/src/daemon.rs:334-343`). The service is wired in `daemon/humatad/src/serve.rs:559-562` with `Runner::new`, so the Upgrade RPC runs on a real daemon. The brief's "when it applies" is: when the daemon was started with the upgrade service, which the real serve path does.
- The brief lists `upgrade_identity.rs` for identity changes, and the IDENTITY_MISMATCH gap. The identity tests are real-daemon and cover the bundle rule. The gap is the app's text, not the daemon.
- The brief says "the holders survive". The test that proves it is for this Mac (`upgrade_local.rs`). For box-1, the box test checks the staged holder file, not the running sessions.
- The brief says "one VIP Migrate item per machine". The daemon does this (`upgrade_inbox.rs`), but the app does not draw it yet (U7), so the user sees it only where the Inbox shows the daemon's items as generic requests.

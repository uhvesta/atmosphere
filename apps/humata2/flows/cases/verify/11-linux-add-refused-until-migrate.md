# A Linux add on stale machine tools is refused until This Mac is migrated, and Migrate This Mac fixes it from the Add machine dialog or from the CLI

Layer: macro (the dialog is a micro view inside it; the CLI route is the same refusal from a terminal). Platforms: macOS for this Mac, the app and the `humata` command. The web build has no Add machine dialog for This Mac's tools. The stale-tools check is host-side, so Linux is not covered here.

## Starting state

- This Mac runs Humata 2. The app was updated, so the Linux machine tools in `~/.humata2/machine-tools` are from release 2026.11.16.
- The running host daemon on this Mac is still release 2026.11.10. Its `bin` folder holds that release's `bundle.json`, so the two releases disagree.
- The Inbox holds one waiting item for This Mac's daemon: "Migrate the daemon on This Mac". It is VIP. It comes either from the daemon (when it reports its build, D370) or from the app (when the old daemon does not report one, D374). The CLI can see only the first kind.
- Dialog route: the Hosts page is open, with no Add machine dialog. I want to add `devbox-2` as `build-box`. Its host key is already trusted on this Mac.
- CLI route: I have a terminal on this Mac with the `humata` command on the PATH. I want to run `humata machine add box-1 --name box-1`. `box-1` is a fake Linux machine that I can reach over SSH with my key.
- Nothing is running on either machine yet. No agent is running.

## Reproduce headless

- Dialog state, with fakes (a tester runs these): `//apps/humata2/features/hosts:migrate_test` (blocked, migrating, failed and offer states; `same_add()` checks the second add has the same host and name), `//apps/humata2/features/app:add_machine_migrate_test`, `//apps/humata2/features/app:upgrade_reconnect_test`, and `//apps/humata2/desktop/install:migrate_old_daemon_test` (staged files put in place with a fake launchd).
- Daemon side of the upgrade, real: `//apps/humata2/daemon/humatad:upgrade_local_test` (a real `humatad2` in a temp folder; the staged release swaps in, holders untouched).
- The stale refusal in `daemon/humatad/src/serve.rs` (`a_linux_add_after_an_app_update_asks_to_migrate_this_mac_first`, in `//apps/humata2/daemon/humatad:humatad2_unit_test`). No target runs it through the gRPC add with a fake box. To make the stale case on `//apps/humata2/desktop:live_machines_test`, start the host without `--tools`, put the Linux tools in `<root>/.humata2/machine-tools` with a `bundle.json` that differs from the one beside the daemon binary, then add the fake box.
- CLI route, by hand, against a throwaway `humatad2` with its own `--root` and `HOME` under a temp folder (never the owner's `~/.humata2`): copy `humatad2` into `<root>/.humata2/bin/` with `bin/bundle.json` naming release a; put the Linux tools in `<root>/.humata2/machine-tools/` with a `manifest.json` and a `bundle.json` naming release b; start `humatad2 serve` with `--state <root>/.humata2/state` and no `--tools`. The fake box is `daemon/machines/testing/fake_machine.rs` on the daemon's PATH, as `daemon/humatad/tests/machines.rs` does (`Host::start`). Run the add, `humata inbox approve <id>` and the add again, with `HOME` and `HUMATA_SOCKET` set to that daemon. Capture each output and `echo $?`.
- Existing CLI targets: `//apps/humata2/cli:hosts_live_test` (a fake machine through the real daemon) and `cli/tests/exit_codes.rs` (the kind-to-exit table, fakes). No target runs the stale refusal through the CLI; the manual run above is the first real check.
- Evidence to capture: the dialog's accessibility names (`add-machine-needs-migrate`, `add-machine-migrate`) from a native view if one is added; the reducer assertions; the CLI output and exit codes; the host's log warning ("the machine tools in ... are from another release").
- Cannot be done headless: a real window (the card's native render is not tested today), a real Migrate of This Mac's installed daemon through launchd, and the app's own Migrate item for an old daemon, which exists only in the app. The nearest headless equivalents are the fake-launchd tests above and the app-side tests in the steps.

## Steps

Steps 1 to 9 are the dialog route. Steps 10 to 13 are the same add from a terminal.

1. **I do:** open Add machine from the Hosts page.  
   **I see:** the dialog opens with the host field and the Start button.  
   **State and events:** `AddMachineState.open` is true. Nothing is sent to the daemon yet.  
   **Status:** Fakes only. `desktop/tests/native/machines.rs::add_machine_failed_step_reference_image` opens the dialog and snapshots it (fakes).

2. **I do:** type `devbox-2` as the host and `build-box` as the name, then press Start.  
   **I see:** the steps Connect and Check run in order. Check shows the platform, git and the free disk.  
   **State and events:** the daemon writes the add as an operation. The Check step reads the machine's facts.  
   **Status:** Works (real daemon). `daemon/humatad/tests/machines.rs::the_check_step_of_an_add_carries_what_it_read_from_the_machine` (real daemon, fake box).

3. **I do:** wait for the Install step.  
   **I see:** Install is refused before anything is copied to the machine. The dialog shows a blocked card, not a failed step.  
   **State and events:** the daemon answers the operation with `host_needs_migrate` (errors.proto kind 61), with the platform `linux-x86_64` and the words "Migrate the daemon on This Mac first (the Inbox item), then add the machine again; nothing was installed on the machine". The add is undone at once. The app's client maps kind 61 to `AddOutcome::NeedsMigrate`.  
   **Status:** Fakes only. `daemon/humatad/src/machine_service.rs::host_needs_migrate_tests::a_host_behind_the_app_names_its_own_kind_and_platform` maps the error to kind 61 without the wire. `clients/live/src/machines.rs::a_host_behind_the_app_stops_the_add_with_its_platform_and_words` maps the kind to the outcome. Nothing runs the refusal through a real daemon call (Code, no test end to end).

4. **I do:** look at the dialog.  
   **I see:** an alert card, labelled "This Mac needs migrating", shows the daemon's words. Below them is the button **Migrate This Mac**. Name, host and login are still filled in.  
   **State and events:** `AddMachineState.migrate` is `HostMigrate { platform, what, stage: Blocked }`. The button is offered only while a waiting or failed Migrate item for This Mac's daemon exists.  
   **Status:** Code, no test for the card's drawing (`ui/src/overlay_add_machine.rs`, the `if let Some(migrate)` block near line 571). The state is covered by `features/hosts/tests/migrate.rs::a_blocked_add_stops_and_keeps_what_was_asked_for` (fakes).

5. **I do:** click **Migrate This Mac**.  
   **I see:** the card says "Migrating This Mac… the add runs again when it is done." The button is gone.  
   **State and events:** the stage becomes `Migrating(<item id>)`. The dialog sends the approval through its parent (`Delegate::Migrate`), and the Inbox marks the item as migrating. No second add is sent yet.  
   **Status:** Fakes only. `features/hosts/tests/migrate.rs::migrating_asks_the_parent_to_approve_then_the_same_add_runs_again` checks the stage and that the add has not run again. The drawn text is Code, no test.

6. **I do:** wait while This Mac's daemon restarts at the new release.  
   **I see:** the card still says "Migrating This Mac…". Nothing else on the screen changes.  
   **State and events:** the local daemon swaps the staged files in (the bundle last), restarts, and keeps its holders. The app waits up to 60 seconds for the socket to answer at the staged bundle (`features/inbox/src/lib.rs`, `LOCAL_DAEMON_WAIT`).  
   **Status:** Fakes only for the app's own item (`features/inbox/tests/migrate.rs::the_apps_own_item_waits_migrating_until_this_macs_daemon_is_back`, with a fake local daemon). The swap itself is Works (real daemon), with fake launchd: `daemon/humatad/tests/upgrade_local.rs::an_upgrade_swaps_the_staged_release_in_and_restarts_the_daemon_with_its_holders_untouched`.

7. **I do:** nothing; the app reconnects by itself.  
   **I see:** the Migrate item ends Migrated. The app reloads from the new daemon. The add runs again by itself with the same host, name and login, and the dialog's card goes away.  
   **State and events:** the app's `Answered(Ok)` for the own item moves it to Approved once the daemon is back. `after_migrations` sees `Approved` and calls `rerun`, which sends the same add request built from the dialog's state (`features/hosts/src/add.rs`, `rerun` and `request`).  
   **Status:** Fakes only. `features/app/tests/add_machine_migrate.rs::migrating_this_mac_from_a_blocked_add_runs_the_add_again_once_the_daemon_is_back` (two adds; the test asserts the count, not the form values). `features/app/tests/upgrade_reconnect.rs::the_app_reconnects_once_this_macs_daemon_is_back_and_its_item_ends_migrated`.

8. **I do:** watch the add finish.  
   **I see:** the dialog closes. `build-box` is selected on the Hosts page, with a toast.  
   **State and events:** `AddDelegate::Added` sets `state.hosts.selected` and switches to Hosts (`features/app/src/machines.rs`).  
   **Status:** Code, no test for the selection after a migrated add. The completing add is covered by fakes in `features/app/tests/add_machine_migrate.rs`, which does not assert the selection.

9. **I do:** look for any other sign that This Mac's tools are old: the Hosts list, the machine page, the Inbox, Settings.  
   **I see:** nothing about machine tools. The stale state shows only in this dialog, its refusal, and a log line the host writes. The Inbox item is shown only because This Mac's daemon really is behind.  
   **State and events:** `stale_platforms` is only inside `daemon/machines/src/bundle.rs` and the host's `Bundle`. No UI reads it.  
   **Status:** Code, no test. Checked by search: nothing in `ui/src`, `features/*/src` or `clients/*/src` reads `stale_platforms`, `machine-tools` or the stale state. No test asserts that absence.

10. **I do:** run `humata machine add box-1 --name box-1` in the terminal, then read the output when the Install step is refused.  
    **I see:** the command shows Connect and Check, then stops with the error kind `host_needs_migrate`. The message says the Linux tools for `linux-x86_64` are newer than the daemon on This Mac. The fix says to migrate This Mac first, using the Inbox item, then add the machine again, and that nothing was installed on the machine.  
    **State and events:** the operation ends Failed with `error.kind = HostNeedsMigrate { platform }`. The CLI exits 7 (`cli/src/exit.rs`, `Kind::HostNeedsMigrate` maps to PRECONDITION). Without a wait (for example in a script), the command returns the started operation and exits 0 (`cli/src/commands/hosts.rs`, `follow`, the `seconds == 0` branch), so the refusal shows only when the operation is watched or looked up.  
    **Status:** Works (real daemon) for a normal add to a fake box: `cli/tests/hosts_live.rs::a_machine_is_added_after_its_key_is_trusted_shown_reconnected_and_removed_by_plan`. The refusal's exit mapping is Fakes only (`cli/tests/exit_codes.rs`, the kind table, no shell). The printed text, the exit code from a real refused add, and the exit 0 without a wait are Code, no test.

11. **I do:** run `humata inbox` to find the Migrate item, then `humata inbox approve <id>` on it if it is listed.  
    **I see:** for a daemon that reports its build, one waiting item "Migrate the daemon on This Mac" (VIP). For an old daemon, nothing, because the app holds that item (D374). After approval the daemon swaps the staged release in, and the item moves to Migrated once the daemon answers at the new release.  
    **State and events:** the daemon posts its own Daemon item when a verified release is staged in its `bin` (D370). A Migrate item takes no answer but Approve (`cli/src/commands/inbox.rs`, the `Approve` branch).  
    **Status:** Code, no test for the CLI listing of the local item. Approve is Works (real daemon): `daemon/humatad/tests/upgrade_inbox.rs::a_migrate_item_takes_no_answer_but_migrate`. The swap is Works (real daemon): `daemon/humatad/tests/upgrade_local.rs::an_upgrade_swaps_the_staged_release_in_and_restarts_the_daemon_with_its_holders_untouched`. Remote posting (`upgrade_inbox.rs::a_machine_at_another_release_is_one_vip_migrate_item_with_no_requester`) is Works for remote machines only.

12. **I do:** run `humata machine add box-1 --name box-1` again.  
    **I see:** the add goes through, the operation ends Succeeded, and `box-1` is listed by `humata machine ls`.  
    **State and events:** This Mac's tools now match its daemon, so the Linux platform is not stale.  
    **Status:** Fakes only for the stale check clearing: `daemon/humatad/src/serve.rs::a_linux_add_with_current_machine_tools_does_not_ask_to_migrate` (unit). The full add after a migrate through the CLI is Code, no test.

13. **I do:** read the fix text for an old daemon and try to follow it from the CLI.  
    **I see:** the fix says "the Inbox item". For an old daemon that is the app's own item. The CLI cannot list or approve it, so `humata inbox` shows nothing to approve. The fix does not tell the CLI user to open the app.  
    **State and events:** kind 61 carries only the platform, so the CLI has no request id to show.  
    **Status:** Likely bug (dead end and wording). The fix text is in `daemon/humatad/src/machine_service.rs` (around line 1186) and `daemon/machines/src/install.rs` (around lines 168–171). To confirm: start a throwaway host with an old daemon that does not report its build, run the add, then run `humata inbox` and look for the item.

## Branches

### Migrate This Mac's daemon does not come back within a minute

- **When:** after step 6 the local daemon does not answer at the staged bundle within 60 seconds. I wait.
- **I see:** the Migrate item shows Failed with "This Mac's daemon did not come back at the new release." and a fix: quit any humata daemon in Activity Monitor, or run `launchctl kickstart -k` on it, then click Migrate again; if that fails, quit Humata and open it again. The dialog shows the same text in red and offers **Migrate This Mac** again.
- **Expected outcome:** nothing is half done. Clicking again starts a new migrate.
- **Status:** Fakes only. `features/app/tests/upgrade_reconnect.rs::a_daemon_that_never_comes_back_fails_the_item_and_the_app_does_not_reload`; `features/inbox/tests/migrate.rs::the_apps_own_item_fails_when_this_macs_daemon_does_not_come_back_in_time`; `features/hosts/tests/migrate.rs::a_failed_migrate_shows_its_failure_and_offers_the_action_again`. The red text is Code, no test.

### The local migrate is refused at once

- **When:** the staged files do not match the staged bundle, and I click Migrate This Mac.
- **I see:** the dialog shows the error's own text, then "Click Migrate again. If it fails again, quit Humata and open it again."
- **Expected outcome:** the add does not run. Migrate This Mac is offered again.
- **Status:** Fakes only. `features/inbox/tests/migrate.rs::a_local_migrate_that_fails_does_not_wait_for_the_daemon`. The red text is Code, no test.

### The daemon-posted item's answer call fails

- **When:** This Mac's daemon posts its own Migrate item (D370), and the app's approval call fails (for example, the socket drops). I click Migrate This Mac.
- **I see:** the dialog says "The migrate didn't start." and "Open the Migrate item in the Inbox to see why, then try again." The real error shows only in the Inbox panel's error line.
- **Expected outcome:** the dialog says what failed, with the daemon's reason.
- **Status:** Likely bug. `features/inbox/src/lib.rs` (`InboxAction::Answered`, about lines 893–900) sets only `state.error`. `features/hosts/src/add.rs` (`after_migrations`, about lines 407–412) turns the Waiting item into the generic failure. To confirm: reduce the add with a daemon item and a failing answer, then read `add_machine.migrate.stage`.

### The dialog is closed while the migrate runs

- **When:** after step 5 I press Cancel or close the dialog while the card says "Migrating This Mac…".
- **I see:** the dialog closes. Nothing else is said.
- **Expected outcome:** either the add runs again once the migrate ends, or the user is told it was dropped.
- **Status:** Likely bug. `features/hosts/src/add.rs` (`A::Cancel`, about lines 621–633) resets the whole state, and the app sends Migrations only while the dialog is open. To confirm: open the add, block it, click Migrate, cancel, then send the migrate's Ok; the add count stays at one.

### No Migrate item exists yet

- **When:** the add is refused, but This Mac's daemon has not posted or held its Migrate item.
- **I see:** the blocked card with its words, and no **Migrate This Mac** button. The words point to "the Inbox item". Clicking does nothing.
- **Expected outcome:** no button, and no add sent.
- **Status:** Fakes only. `features/hosts/tests/migrate.rs::without_an_item_the_action_does_nothing` and `the_action_shows_only_while_a_migrate_item_for_the_daemon_exists`.

### The Migrate item is approved from the Inbox instead

- **When:** the dialog is on the blocked card (stage Blocked, not Migrating), and I approve the same item in the Inbox.
- **I see:** the Inbox shows the item as migrated. The dialog still shows the blocked card with no button, and the add does not run.
- **Expected outcome:** the add runs again, or the card says the migrate is done and the add must be started again.
- **Status:** Likely bug, if the Inbox can be clicked while the dialog is open. `after_migrations` acts only in stage `Migrating`, and `migrate_offer` needs an unanswered item (`features/hosts/src/add.rs`). The overlay takes the keyboard (`ui/src/overlays.rs`); check whether the mouse reaches the Inbox behind it.

### This Mac's daemon restarts or the power is cut during the migrate

- **When:** the daemon or the Mac loses power after the upgrade is recorded (during step 6, or during the CLI's approve in step 11).
- **I see:** This Mac's daemon comes back on one release, never neither. The dialog starts fresh with no add in progress, and the CLI answers from one release. I start the add again by hand.
- **Expected outcome:** one whole release is running. The add is not resumed by itself.
- **Status:** Works (real daemon) for the daemon's part: `daemon/humatad/tests/upgrade_cut.rs::a_cut_after_the_upgrade_was_recorded_and_before_anything_was_swapped_leaves_release_a`, `...::a_new_build_cut_after_keeping_the_store_copy_is_rolled_back_to_release_a` and `...::a_new_build_cut_after_opening_the_store_at_the_new_schema_is_rolled_back_to_release_a`. The dialog starting fresh is not tested.

### The CLI on the old release runs a command during the migrate

- **When:** the old `humata` binary runs `humata ls` while This Mac is at the new release.
- **I see:** refused with `migration_needed` and exit 7, and the message names the item to open.
- **Expected outcome:** refused, with nothing run.
- **Status:** Works (real daemon). `daemon/humatad/tests/upgrade_refusals.rs::a_cli_at_another_release_is_refused_with_migration_needed`.

### The stale tools are older than the app's daemon

- **When:** the machine tools in `~/.humata2/machine-tools` are an older release than the host's `bin`, and I start a Linux add in the dialog or the CLI.
- **I see:** the refusal says the Linux tools "are newer than the daemon on This Mac" and to Migrate This Mac first.
- **Expected outcome:** the words match the direction. Humata never downgrades (SPEC UP-1), so a downgrade should say to open a newer app, not to migrate.
- **Status:** Likely bug (medium confidence). `with_machine_tools` in `daemon/humatad/src/serve.rs` (around line 1359) treats any difference in `bundle.json` as stale, and the words in `daemon/machines/src/install.rs` (around lines 168–171) do not depend on direction. To confirm: put an older `bundle.json` in the tools folder, start the host without `--tools`, and read the refusal and the Inbox.

### An agent runs the same add

- **When:** an agent calls `humata machine add` or the MCP add while the tools are stale.
- **I see:** the agent's add waits in my Inbox. If I approve it, it is refused the same way (kind `host_needs_migrate`), and the agent gets the same message and exit 7. The refusal does not invite the agent to approve the Migrate item.
- **Expected outcome:** the agent cannot migrate This Mac.
- **Status:** Code, no test for the stale variant. The agent's add waits in the Inbox first (`daemon/humatad/tests/machines.rs::an_agents_add_waits_in_the_inbox_and_runs_once_the_user_approves_it`).

### An agent tries to approve the Migrate item

- **When:** an agent runs `humata inbox approve <id>` on This Mac's Migrate item.
- **I see:** the call is refused, and the item stays where it was. The refusal is recorded as denied.
- **Expected outcome:** only the person upgrades a daemon.
- **Status:** Works (real daemon). `daemon/humatad/tests/upgrade_inbox.rs::an_agent_cannot_answer_a_migrate_item` and `daemon/humatad/tests/upgrade_local.rs::an_agent_or_a_script_cannot_upgrade_the_daemon`.

## Expected outcomes

- [ ] Add machine on stale tools stops at the Install step with kind 61 and the platform, and nothing is left on the box.
- [ ] The dialog shows the daemon's words and offers Migrate This Mac once a Migrate item for This Mac's daemon exists. Clicking sends one approval, and the card shows Migrating with no button.
- [ ] After This Mac's daemon is back at the new release, the Migrate item ends Migrated, and the add runs again with the same host, name and login, without re-entering the form.
- [ ] The CLI refusal exits 7 and says what failed and how to fix it. `humata inbox approve` finishes a daemon-posted Migrate item, and the same `humata machine add` goes through after it.
- [ ] A failed migrate shows what failed and how to fix it, and offers Migrate This Mac again.
- [ ] No screen other than this dialog and its refusal says the tools are stale.
- [ ] An agent cannot approve the Migrate item, and an agent's add is refused the same way.
- [ ] An old daemon's Migrate item is reachable from the CLI, or the fix says to use the app (not met today; see the gaps).
- [ ] Closing the dialog during the migrate does not silently drop the add (not met today; see the branch).

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Open Add machine | Fakes only | `desktop/tests/native/machines.rs::add_machine_failed_step_reference_image` |
| 2. Connect and Check run | Works (real daemon) | `daemon/humatad/tests/machines.rs::the_check_step_of_an_add_carries_what_it_read_from_the_machine` |
| 3. Install refused with kind 61 | Fakes only | `daemon/humatad/src/machine_service.rs::host_needs_migrate_tests::...`; `clients/live/src/machines.rs::a_host_behind_the_app_...` |
| 4. Blocked card with the button | Code, no test | `ui/src/overlay_add_machine.rs` (`if let Some(migrate)`); state: `features/hosts/tests/migrate.rs::a_blocked_add_stops_...` |
| 5. Click Migrate This Mac | Fakes only | `features/hosts/tests/migrate.rs::migrating_asks_the_parent_to_approve_...` |
| 6. Local daemon swaps and restarts | Fakes only (app item); Works (real daemon) for the swap | `features/inbox/tests/migrate.rs::the_apps_own_item_waits_...`; `daemon/humatad/tests/upgrade_local.rs::an_upgrade_swaps_...` |
| 7. Reconnect; add runs again | Fakes only | `features/app/tests/add_machine_migrate.rs::migrating_this_mac_from_a_blocked_add_...`; `features/app/tests/upgrade_reconnect.rs::the_app_reconnects_...` |
| 8. Machine selected after the add | Code, no test | `features/app/src/machines.rs` (`AddDelegate::Added`) |
| 9. No other stale status on screen | Code, no test | Search of `ui/src`, `features/*/src`, `clients/*/src` |
| 10. CLI add shows steps, refused with kind 61 | Works (real daemon) for a normal add; refusal exit mapping Fakes only; printed text and exit 0 without a wait Code, no test | `cli/tests/hosts_live.rs::a_machine_is_added_after_its_key_is_trusted_...`; `cli/tests/exit_codes.rs`; `cli/src/exit.rs` line 80 |
| 11. CLI lists and approves the Migrate item | Code, no test for the local listing; Approve Works (real daemon) | `daemon/humatad/tests/upgrade_inbox.rs::a_migrate_item_takes_no_answer_but_migrate`; `daemon/humatad/tests/upgrade_local.rs::an_upgrade_swaps_...` |
| 12. Add again after the migrate | Fakes only (stale check); full CLI add Code, no test | `daemon/humatad/src/serve.rs::a_linux_add_with_current_machine_tools_does_not_ask_to_migrate` |
| 13. Old daemon's item not reachable from the CLI | Likely bug | `daemon/humatad/src/machine_service.rs` ~1186; `daemon/machines/src/install.rs` ~168-171; D374 |
| B. Migrate does not come back in 60 s | Fakes only | `features/app/tests/upgrade_reconnect.rs::a_daemon_that_never_comes_back_fails_the_item_...` |
| B. Local migrate refused at once | Fakes only | `features/inbox/tests/migrate.rs::a_local_migrate_that_fails_does_not_wait_...` |
| B. Daemon item's answer call fails | Likely bug | `features/hosts/src/add.rs` ~407-412; `features/inbox/src/lib.rs` ~893-900 |
| B. Dialog closed while migrating | Likely bug | `features/hosts/src/add.rs` ~621-633; `features/app/src/lib.rs` (`add` block) |
| B. No item yet, no button | Fakes only | `features/hosts/tests/migrate.rs::without_an_item_the_action_does_nothing` |
| B. Approved in the Inbox, dialog stuck | Likely bug (if reachable) | `features/hosts/src/add.rs` (`after_migrations`, `migrate_offer`) |
| B. Daemon restart or power cut mid-migrate | Works (real daemon) for the daemon; dialog fresh start not tested | `daemon/humatad/tests/upgrade_cut.rs::...` |
| B. Old CLI during the migrate | Works (real daemon) | `daemon/humatad/tests/upgrade_refusals.rs::a_cli_at_another_release_is_refused_with_migration_needed` |
| B. Stale tools older than the app | Likely bug (medium) | `daemon/humatad/src/serve.rs` ~1359; `daemon/machines/src/install.rs` ~168-171 |
| B. Agent runs the same add | Code, no test for the stale variant | `daemon/humatad/tests/machines.rs::an_agents_add_waits_in_the_inbox_...` |
| B. Agent approves the Migrate item | Works (real daemon) | `daemon/humatad/tests/upgrade_inbox.rs::an_agent_cannot_answer_a_migrate_item`; `daemon/humatad/tests/upgrade_local.rs::an_agent_or_a_script_cannot_upgrade_the_daemon` |

## Gaps and likely bugs

Not built:

- The inline **Migrate This Mac** button is not named in the spec. The milestone that adds it is MACHINESCRIPTS w4c (`workflow/WORKLOG.md` line 68); `workflow/milestones/MACHINESCRIPTS.md` does not mention w4c, so the milestone file has no record of it.
- No CLI command leads from `host_needs_migrate` to the migrate. The fix is text only. No milestone names one.
- The dialog keeps no add in progress across an app restart. No spec asks for it, so there is no milestone.

Code, no test:

- The blocked card and its red and Migrating text have no native snapshot (`desktop/tests/__snapshots__` has no add-machine migrate image).
- The kind 61 refusal has no end-to-end test through the daemon's add call. The unit tests call the mapping directly.

Likely bugs:

- Step 13 (old daemon): the Migrate item lives only in the app (D374), but the fix says "the Inbox item" and the CLI has no id to approve. Confirm with a throwaway host whose daemon does not report its build.
- Branch "daemon-posted item's answer call fails": the dialog shows "The migrate didn't start." and not the reason.
- Branch "dialog closed while migrating": closing the dialog drops the add with no message.
- Branch "approved in the Inbox": the dialog cannot re-run the add if the item is approved from the Inbox while it sits on Blocked (if reachable).
- Branch "stale tools older than the app": the stale check and its words are direction-blind.

Doc notes (not bugs):

- `flows/cases/macro/15-upgrade-day.md` says `upgrade_holders.rs` and `src/upgrade/holders.rs` do not exist and that U6 is Not built. Both exist (`daemon/humatad/tests/upgrade_holders.rs`, 9 tests; `daemon/humatad/src/upgrade/holders.rs`). That case needs a fix, outside this area.
- `api/GRPC.md` line 796 says the app offers Migrate This Mac inline and says nothing about the CLI's path.

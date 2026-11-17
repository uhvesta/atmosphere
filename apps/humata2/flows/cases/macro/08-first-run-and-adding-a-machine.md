# First run: the app installs its host tools, I make my first channel, and I add box-1 over SSH with a new host key

Layer: macro. Platforms: macOS, Linux, web. (The keys shown are the macOS ones; Linux uses `Ctrl`.)

## Starting state

- This Mac has never run Humata 2. There is no Humata folder, no daemon and no `humata` command yet.
- I have downloaded the Humata 2 zip. It is in Downloads, and it has not been moved anywhere.
- box-1 is a Linux machine on my network. I can reach it over SSH with my key. I have never connected to it from This Mac, so This Mac has no record of its host key.
- box-1 has `git` installed but no `gh`.
- I hold the box-1 address (`box-1.internal`) in my head. I have not added it to any SSH config.

## Steps

1. **I do:** Unzip the download and open Humata 2 from Downloads, where it sits.
   **I see:** The app opens from Downloads. It does not move itself into Applications.
   **State and events:** The app runs from where it was opened. Nothing is copied into `/Applications` by the app.
   **Status:** Not built (no milestone). `desktop/src/main.rs` has no step that moves the app into Applications, and no SPEC section asks for one. The release tool `tools/release/install.sh` copies the app into Applications; the app does not.

2. **I do:** Wait for the starting window to finish.
   **I see:** A starting window shows progress while the host tools are installed: the daemon, the holder, and the `humata` command. When it finishes, the main window opens.
   **State and events:** The tools are copied into the Humata folder from the bundle. The `humata` command is linked into the local bin folder. A bundle record names the files that were installed. A launch with the same release changes nothing.
   **Status:** Fakes only. `desktop/install/tests/install.rs::first_install_copies_everything_and_registers` and `desktop/install/tests/install.rs::second_install_changes_nothing`. The tests use a fake launchd and a fake bundle, so the real copy on a Mac is unproven here.

3. **I do:** Log out and log in again.
   **I see:** After I log in, the daemon starts on its own. The main window opens with the app connected to it.
   **State and events:** A login item for the daemon is registered at install time. The daemon's start is recorded by the service.
   **Status:** Fakes only. `desktop/install/tests/install.rs::unloaded_agent_is_loaded_again`. The login item is run through a fake launchd, so the real login start is unproven.

4. **I do:** Look at Home on the first run.
   **I see:** Home is empty. It offers a Desk session and a new channel. Nothing is listed under Needs you.
   **State and events:** No channel, session or request exists yet.
   **Status:** Fakes only. `features/app/tests/desk.rs::empty_home_starts_a_desk_session_and_opens_it` covers the Desk offer. The empty Home itself is shown by the same reducer.

5. **I do:** Press New channel, name it `payments`, tick `acme/api` and press Create channel.
   **I see:** The channel opens. Its folder exists under the Humata folder, with a main clone of `acme/api`.
   **State and events:** The channel is recorded on this Mac's host. A repository that does not exist fails the setup and leaves no half clone.
   **Status:** Works (real daemon). `daemon/humatad/tests/channels.rs::a_new_channel_gets_its_folders_and_a_main_clone`. The form and its create button are Fakes only: `features/app/tests/new_channel.rs::the_create_button_says_how_many_repos_it_will_clone`.

6. **I do:** Open Hosts and press Add a machine…
   **I see:** The Add machine dialog opens. I can type a host, or pick one from the list the dialog offers.
   **State and events:** The dialog loads the choices it has. It picks the first one, or the one I typed.
   **Status:** Fakes only. `features/hosts/tests/add.rs::open_loads_the_choices_and_picks_the_first_host` and `features/hosts/tests/add.rs::a_typed_host_wins_over_the_popups_pick`. The list does not include the hosts in my SSH config: that is Not built (OVERVIEW.md line 299).

7. **I do:** Type `box-1.internal` as the host and press Add.
   **I see:** The dialog stops at the Connect step. It shows the new host key and asks me to confirm it. Nothing is installed on box-1.
   **State and events:** The add is recorded as waiting for a host key. Nothing is installed until I confirm the key.
   **Status:** Works (real daemon). `clients/live/tests/machines.rs::an_add_without_the_host_key_ends_asking_for_its_fingerprint` and `daemon/humatad/tests/machines.rs::a_machine_is_added_once_its_host_key_is_confirmed_then_removed_by_plan` (its first contact installs nothing, and the list stays empty).

8. **I do:** Compare the fingerprint with the one box-1 shows (`ssh-keygen -lf` on box-1), then press Confirm.
   **I see:** The dialog says the host's key is new to this computer, and shows the fingerprint. After Confirm, the steps run.
   **State and events:** The confirmed key is pinned for box-1. The add is run again with that key, under the same operation id.
   **Status:** Fakes only. `features/hosts/tests/add.rs::a_host_key_request_waits_for_confirmation_and_then_trusts_it` and `desktop/tests/machines_flow.rs::confirming_a_new_host_key_adds_trusting_it`.

9. **I do:** Nothing while the steps run: Connect, Check, Install and Start.
   **I see:** The dialog shows each step as it runs. When all are done, box-1 appears in Hosts as online.
   **State and events:** The daemon installs `humatad2`, the holder and the `humata` command on box-1, starts the daemon there, and opens the link. Its versions only go up.
   **Status:** Works (real daemon). `daemon/humatad/tests/machines.rs::a_machine_is_added_once_its_host_key_is_confirmed_then_removed_by_plan`, which checks each step is Done and every installed file is on the machine. The step list in the dialog is Fakes only: `features/hosts/tests/add.rs::a_successful_add_follows_the_steps_then_closes_and_delegates`.

10. **I do:** Nothing. The Check step reads box-1.
    **I see:** The step's output shows what it read: the platform, the free disk, and the versions of `git` and `gh`.
    **State and events:** The check's facts are kept on the add. The check does not install anything.
    **Status:** Works (real daemon). `daemon/humatad/tests/machines.rs::the_check_step_of_an_add_carries_what_it_read_from_the_machine`. The version parsing is covered in `daemon/machines/tests/check_facts.rs::the_check_reads_the_versions_of_git_and_gh_beside_the_platform_and_the_free_disk` (Fakes only: parsing the text a fake check prints).

11. **I do:** Nothing. box-1 has no `gh`.
    **I see:** The add goes on. The machine's page shows no gh version. Nothing says gh is missing as an error.
    **State and events:** `gh` is optional for a machine. The check records no version for it.
    **Status:** Works (real daemon). `daemon/humatad/tests/machines.rs::the_check_step_of_an_add_names_no_gh_version_when_gh_gives_none`.

12. **I do:** Remove `git` from box-1 and add it again.
    **I see:** The add stops at Check. The message says that `git` is not installed on box-1 and that it should be installed there, for example with the package manager, and then tried again.
    **State and events:** The add fails at Check. Nothing is installed.
    **Status:** Code, no test. The message is at `daemon/machines/src/error.rs:316`, and the refusal is in `daemon/machines/src/setup.rs:666`. No test removes `git` and checks the failure.

13. **I do:** Start a session in `payments` on box-1 with the message "Set up the test database".
    **I see:** The session starts on box-1 and its terminal shows up in the app through This Mac.
    **State and events:** The session is recorded on box-1 under the channel's id.
    **Status:** Works (real daemon). `daemon/humatad/tests/remote.rs::a_session_started_on_the_machine_runs_there_and_the_host_relays_its_terminal`.

14. **I do:** Restart This Mac.
    **I see:** After the restart, box-1 is linked again and online. The host key is still the one I confirmed.
    **State and events:** The link is brought up again from the pinned key. No second confirm is asked.
    **Status:** Works (real daemon). `daemon/humatad/tests/machines.rs::an_added_machine_holds_the_hosts_key_and_its_link_comes_up_again_after_a_restart`.

## Branches

### SSH cannot log in with my key

- **When:** box-1 asks for a password or a login that Humata cannot do with my key.
- **I do:** Read the failure in the dialog, fix the login on box-1, and press Try again.
- **I see:** The failed step is named, with its output. Try again runs the same add, under the same id. A login that needs me to type is offered as "Log in by hand" in the dialog.
- **Expected outcome:** The message says what failed and how to fix it. No half-made machine is listed.
- **Status:** Works (real daemon). `clients/live/tests/machines.rs::a_failed_add_names_its_step_and_trying_again_follows_the_same_operation` and `daemon/humatad/tests/machines.rs::a_failed_add_tried_again_runs_under_the_same_id_and_a_removal_cannot_be_cancelled`. The "Log in by hand" offer is covered by `clients/live/tests/machines.rs::a_provider_that_needs_a_login_is_logged_in_by_hand_and_becomes_a_machine` (a provider's login) and by `features/hosts/tests/add.rs::the_login_offer_keeps_the_daemons_reason_and_opening_it_keeps_why` (Fakes only). The fix text for a key login that fails (`daemon/machines` and `daemon/humatad/src/machine_service.rs`, near line 990: "make key login work without a password or passphrase prompt") is not checked by any test.

### box-1 cannot be reached

- **When:** box-1 is off, or its address is wrong.
- **I do:** Press Add, then Try again once box-1 is on.
- **I see:** The Connect step fails. The message says the machine could not be reached after several attempts, and says to try again if it is still booting.
- **Expected outcome:** A clear message with a fix. A machine that never answers can be deleted.
- **Status:** Fakes only. `daemon/machines/tests/setup.rs::a_machine_that_never_answers_is_unreachable_and_can_be_deleted` runs with a fake runner.

### The host key changed for a machine this Mac already knows

- **When:** box-1 was rebuilt, so its key differs from the one pinned on This Mac.
- **I do:** Compare the fingerprints on box-1 and decide.
- **I see:** The message shows both fingerprints. It says to trust the new key only if box-1 was rebuilt, and to check its fingerprint on the machine. The app offers no login for this case.
- **Expected outcome:** Nothing is trusted without my decision. The error names both keys.
- **Status:** Fakes only. `daemon/machines/tests/setup.rs::a_changed_host_key_shows_both_fingerprints_and_offers_no_login` runs with a fake runner.

### Trusting a changed key from the command line

- **When:** I want to trust the new key from `humata` on This Mac, instead of through the dialog.
- **I do:** Run `humata hosts trust-key box-1` with the fingerprint.
- **I see:** The command fails with a not-built answer.
- **Expected outcome:** Either the key is trusted after my confirm, or the message says where to do it.
- **Status:** Not built (MACH T10). `daemon/humatad/src/machine_service.rs:1400` answers `not_built` for `TrustNewHostKey`, and `cli/src/commands/hosts.rs:233` to `:244` sends that call.

### I decline the host key

- **When:** the fingerprint on the dialog does not match box-1's.
- **I do:** Press Don't add.
- **I see:** The dialog stays open. Nothing is added or installed.
- **Expected outcome:** No machine is recorded and no tool is put on box-1.
- **Status:** Fakes only. `features/hosts/tests/add.rs::declining_a_host_key_adds_nothing_and_keeps_the_dialog` and `desktop/tests/machines_flow.rs::declining_a_new_host_key_adds_nothing`. The daemon side also installs nothing before a confirm: `daemon/humatad/tests/machines.rs::a_machine_is_added_once_its_host_key_is_confirmed_then_removed_by_plan`.

### box-1 is a Mac

- **When:** I try to add a Mac as a remote machine.
- **I do:** Add the Mac over SSH.
- **I see:** The add is refused at Check, and the message says Humata 2 sets up Linux machines only, and to use a Linux machine.
- **Expected outcome:** Nothing is installed on the Mac. The reason is given.
- **Status:** Fakes only. `daemon/machines/tests/setup.rs::the_check_refuses_machines_setup_cant_use` runs with a fake runner. Running sessions on a Mac that is not This Mac needs launchd for macOS remotes, which is Not built (MACH T10; OVERVIEW.md line 559).

### box-1 is on another release

- **When:** box-1 has an older `humatad2` than This Mac's.
- **I do:** Start a session on box-1, or press Migrate in the Inbox when it appears.
- **I see:** The start is refused, and the item it was for says box-1 needs Migrate first. When I approve Migrate, the new release is staged on box-1 over SSH, and box-1 comes back at the new release.
- **Expected outcome:** No session is started on an older release. The migration finishes or is undone; nothing is left half installed.
- **Status:** Works (real daemon). `daemon/humatad/tests/upgrade_refusals.rs::starting_a_session_on_a_machine_at_another_release_is_refused_with_its_item` and `daemon/humatad/tests/upgrade_box.rs::approving_migrate_stages_the_release_over_ssh_and_the_machine_comes_back_at_it`.

### I cancel halfway through the add

- **When:** I press Cancel while the add is connecting.
- **I do:** Press Cancel in the dialog.
- **I see:** The dialog closes. The add ends as cancelled, and box-1 is not listed.
- **Expected outcome:** Nothing is left on box-1 or in the list.
- **Status:** Works (real daemon). `daemon/humatad/tests/machines.rs::an_add_cancelled_while_it_connects_ends_cancelled_and_leaves_nothing` and `clients/live/tests/machines.rs::a_cancelled_add_ends_without_a_machine`. The dialog's Cancel is Fakes only: `features/hosts/tests/add.rs::cancel_stops_a_running_add_and_closes`.

### The add fails at Install and I try again

- **When:** the Install step fails partway (for example, the disk is full on box-1).
- **I do:** Press Try again.
- **I see:** The same add runs again under the same id, and it finishes.
- **Expected outcome:** One machine, not two. The retry does not start a second add.
- **Status:** Works (real daemon). `daemon/humatad/tests/machines.rs::a_failed_add_tried_again_runs_under_the_same_id_and_a_removal_cannot_be_cancelled`. The dialog's retry is Fakes only: `desktop/tests/machines_flow.rs::an_add_over_ssh_fails_at_install_and_try_again_retries_the_same_add`.

### box-1 goes away in the middle of the add

- **When:** box-1 stops responding at some step of the add.
- **I do:** Nothing. When box-1 is back, I press Try again.
- **I see:** The add is undone on box-1 when it comes back, and the list is unchanged.
- **Expected outcome:** No half-installed machine is left behind.
- **Status:** Fakes only. `daemon/machines/tests/fake_ssh.rs::an_add_whose_machine_dies_at_any_step_is_undone_once_it_is_back` runs with a fake ssh.

### box-1 is an unsupported platform

- **When:** box-1 reports an OS or architecture that Humata 2 does not build for, for example FreeBSD.
- **I do:** Add it.
- **I see:** The add fails at Check and names the platform it found. Nothing is installed.
- **Expected outcome:** The refusal says what was found, and nothing is left on the machine.
- **Status:** Fakes only. `daemon/machines/tests/fake_ssh.rs::an_unsupported_platform_fails_with_its_uname_and_leaves_nothing`.

## Expected outcomes

- [ ] After the first launch, the daemon, the holder and the `humata` command are installed, and a second launch changes nothing.
- [ ] The first run shows an empty Home, and a new channel gets its folder and a main clone.
- [ ] Adding box-1 installs nothing until I confirm its key.
- [ ] After I confirm, the add runs Connect, Check, Install and Start, and box-1 has `humatad2` and the tools.
- [ ] A missing `gh` does not fail the add. A missing `git` does, and says what to install.
- [ ] A failed step names itself, and Try again runs the same add.
- [ ] A cancelled or declined add leaves no machine and nothing on box-1.
- [ ] A Mac or an unsupported platform is refused at Check, with its reason.
- [ ] A machine on an older release is not started until it is migrated.
- [ ] After a restart, box-1 links again with the key I confirmed, and no second confirm is asked.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Open from Downloads, no self-install | Not built (no milestone) | `desktop/src/main.rs` has no such step |
| 2. Host tools installed on first launch | Fakes only | `desktop/install/tests/install.rs::first_install_copies_everything_and_registers` |
| 3. Daemon starts at login | Fakes only | `desktop/install/tests/install.rs::unloaded_agent_is_loaded_again` |
| 4. Empty Home on first run | Fakes only | `features/app/tests/desk.rs::empty_home_starts_a_desk_session_and_opens_it` |
| 5. First channel with a main clone | Works (real daemon) | `daemon/humatad/tests/channels.rs::a_new_channel_gets_its_folders_and_a_main_clone` |
| 6. Add machine dialog and host choice | Fakes only | `features/hosts/tests/add.rs::open_loads_the_choices_and_picks_the_first_host` (SSH config list: Not built, OVERVIEW.md line 299) |
| 7. First contact asks for the key | Works (real daemon) | `clients/live/tests/machines.rs::an_add_without_the_host_key_ends_asking_for_its_fingerprint` |
| 8. Confirm the key in the dialog | Fakes only | `features/hosts/tests/add.rs::a_host_key_request_waits_for_confirmation_and_then_trusts_it` |
| 9. Steps run, tools installed | Works (real daemon) | `daemon/humatad/tests/machines.rs::a_machine_is_added_once_its_host_key_is_confirmed_then_removed_by_plan` |
| 10. Check reads the machine | Works (real daemon) | `daemon/humatad/tests/machines.rs::the_check_step_of_an_add_carries_what_it_read_from_the_machine` |
| 11. No gh, add goes on | Works (real daemon) | `daemon/humatad/tests/machines.rs::the_check_step_of_an_add_names_no_gh_version_when_gh_gives_none` |
| 12. No git, add stops with a fix | Code, no test | `daemon/machines/src/error.rs:316` |
| 13. First session on box-1 | Works (real daemon) | `daemon/humatad/tests/remote.rs::a_session_started_on_the_machine_runs_there_and_the_host_relays_its_terminal` |
| 14. Restart, link back, same key | Works (real daemon) | `daemon/humatad/tests/machines.rs::an_added_machine_holds_the_hosts_key_and_its_link_comes_up_again_after_a_restart` |
| Branch: SSH cannot log in | Works (real daemon) | `clients/live/tests/machines.rs::a_failed_add_names_its_step_and_trying_again_follows_the_same_operation` |
| Branch: key login fix text | Code, no test | `daemon/humatad/src/machine_service.rs` near line 990 |
| Branch: box-1 unreachable | Fakes only | `daemon/machines/tests/setup.rs::a_machine_that_never_answers_is_unreachable_and_can_be_deleted` |
| Branch: changed host key | Fakes only | `daemon/machines/tests/setup.rs::a_changed_host_key_shows_both_fingerprints_and_offers_no_login` |
| Branch: trust a changed key from the CLI | Not built | MACH T10; `daemon/humatad/src/machine_service.rs:1400` |
| Branch: decline the host key | Fakes only | `features/hosts/tests/add.rs::declining_a_host_key_adds_nothing_and_keeps_the_dialog` |
| Branch: box-1 is a Mac | Fakes only | `daemon/machines/tests/setup.rs::the_check_refuses_machines_setup_cant_use` |
| Branch: box-1 on another release | Works (real daemon) | `daemon/humatad/tests/upgrade_refusals.rs::starting_a_session_on_a_machine_at_another_release_is_refused_with_its_item` |
| Branch: cancel halfway | Works (real daemon) | `daemon/humatad/tests/machines.rs::an_add_cancelled_while_it_connects_ends_cancelled_and_leaves_nothing` |
| Branch: install fails, try again | Works (real daemon) | `daemon/humatad/tests/machines.rs::a_failed_add_tried_again_runs_under_the_same_id_and_a_removal_cannot_be_cancelled` |
| Branch: box-1 goes away mid-add | Fakes only | `daemon/machines/tests/fake_ssh.rs::an_add_whose_machine_dies_at_any_step_is_undone_once_it_is_back` |
| Branch: unsupported platform | Fakes only | `daemon/machines/tests/fake_ssh.rs::an_unsupported_platform_fails_with_its_uname_and_leaves_nothing` |

## Gaps and likely bugs

- **Not built (no milestone):** moving the app into Applications when it is opened from Downloads. The release tool does this, not the app.
- **Not built (OVERVIEW.md line 299):** listing the hosts in `~/.ssh/config` in the Add machine dialog. The dialog offers the hosts the daemon knows, and a typed host.
- **Not built (MACH T10, OVERVIEW.md line 299):** `TrustNewHostKey`, `daemon/humatad/src/machine_service.rs:1400`. The CLI's `humata hosts trust-key` sends it and gets not built.
- **Not built (MACH T10, OVERVIEW.md line 559):** launchd for macOS remotes.
- **Code, no test:** a `git` missing on box-1 stops the add with its fix (`daemon/machines/src/error.rs:316`, `daemon/machines/src/setup.rs:666`).
- **Code, no test:** the fix text for a failed key login (`daemon/humatad/src/machine_service.rs` near line 990).
- **Likely bug (to confirm live):** the changed-key message tells me to "trust the new key only if you know box-1 was rebuilt" (`daemon/machines/src/error.rs:275`). The only trust route in the app is the Add dialog's Confirm card, and that card comes from `HostKeyUnconfirmed` and `HostKeyMismatch` (`daemon/humatad/src/machine_service.rs:971` to `:990`). A changed pin found by the setup step is not mapped to that card, so the message may point to a control the dialog does not show. The CLI route it implies is Not built. Confirm by adding a machine whose pinned key differs.
- **Likely bug (low, to confirm):** the Confirm card says "This host's key is new to this computer" (`ui/src/overlay_add_machine.rs:582`). The outcome that drives it carries only the new fingerprint (`daemon/machines/src/manage.rs:948`, `clients/live/src/machines.rs:412`), so when a different key was pinned, the dialog cannot show the old one, and Confirm replaces the pin (`daemon/machines/src/known_hosts.rs:45`). A pin for a machine not yet added is rare, so the impact is small.
- **Correction to the brief:** the brief says the app installs itself into Applications when it is opened from Downloads. The code does not do that. The app installs host tools into the Humata folder on each launch (`desktop/install/src/lib.rs`).
- **Correction to the brief:** the brief names `features/hosts/tests/health.rs` and the other `features/hosts` suites. They are Fakes only, and this case does not rely on them.

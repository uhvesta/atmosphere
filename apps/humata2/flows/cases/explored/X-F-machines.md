# Machines: the Hosts list, the machine page, Add machine, and the `humata host` commands

Layer: explored · macOS · harness: `//apps/humata2/desktop:native_test` (module `hosts_settings`, `machines`), `//apps/humata2/desktop:machines_flow_test`, `//apps/humata2/desktop:live_machines_test`, `//apps/humata2/features/app:upgrade_reconnect_test`, `//apps/humata2/features/app:add_machine_migrate_test`, `//apps/humata2/features/hosts:migrate_test`, `//apps/humata2/features/hosts:upgrade_test`, `//apps/humata2/daemon/machines:setup_test`, `//apps/humata2/daemon/humatad:humatad2_unit_test`, `//apps/humata2/cli:hosts_live_test`

Status of this record: **phase 1 (map from code and existing snapshots).** Nothing was run.
Steps marked PENDING RUN need phase 2. Bazel is not run in phase 1.

## Setup

- Phase 1 only: no Bazel, cargo or git. Facts come from `features/hosts/src/{add,page,lib}.rs`,
  `ui/src/{hosts,machine_page,overlay_add_machine}.rs`, `clients/live/src/machines.rs`,
  `clients/src/machines.rs`, `daemon/machines/src/setup.rs`, `daemon/humatad/src/serve.rs`,
  `cli/src/{args,commands/hosts,exit}.rs`, the tests named per step, and the snapshots in
  `desktop/tests/__snapshots__/native/`.
- Headless harnesses only. The native and machines_flow targets run the real window on fake
  `Machines` (`FakeMachines`, scripted). The live targets start a throwaway `humatad2 serve --admin`
  with a fake machine behind a fake `ssh` (their own temp folder, never `~/.humata2`).
- Environment block: adding the real VM (`real_vm.rs`) fails `DiskLow` (1.3 GiB free, 2 GiB
  needed). Step 16 uses the daemon's own unit test instead.
- Phase 2 evidence folder: `verify/evidence/X-F/` in the session scratchpad (create it in phase 2).
  Bazel test logs are at `bazel-testlogs/<target>/test.log` and `test.outputs/` on a mismatch.
- Phase 2 preflight (once, from the repo root): `export PATH=$PWD/.tools/bin:$PATH`.

## Steps

### 1. Hosts list (frame 28)
- **Screen:** title "Hosts" with an "Add a machine…" button at the top right. One row per machine:
  status dot, name (a link to the machine page), a line of system or "last seen", session count
  ("none", "N sessions", or "Sessions stopped"), a disk bar with "N GB free", and an action button
  on the right. Rows: "This Mac" first, then "devbox-1" (selected), "gpu-box", "cloud-vm-3".
  The detail card on the right shows the machine name, the health line
  ("humatad 2.0.3 · up … · linked over ssh"), the tool chips, "Open a shell", "Free space…", and the
  "Bridge" card ("This Mac ⇄ devbox-1 (ssh, N ms)"). The layout stacks vertically when the window is narrower than 850 px.
- **Offered:** click a row (selects it, `host-row <id>`); click a name (opens the machine page); "Add a machine…" (`hosts-add`); "Open a shell" (`host-open-shell`); "Free space…" (`host-free`, disabled, see step 5); the row button (`host-connect`, "Reconnect" or "Start", or `host-start`, "Start sessions").
- **Action:** the Hosts route is opened and devbox-1 is selected.
- **Result:** the snapshot shows devbox-1 selected, its tools as "gh 2.83", "claude 2.4", "codex 0.98" (the workspace's values before the daemon answers), and "gpu-box" with "Reconnect".
- **Evidence:** `apps/humata2/desktop/tests/__snapshots__/native/28-hosts.png` and `28-hosts.view.txt`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=hosts_and_settings_reference_images` (passes on the snapshot; no new evidence needed).

### 2. Select a machine: the card shows the tools its daemon reports
- **Screen:** the detail card for the selected row. Its chips come from the machine's own daemon report (`MachineDetail.health`), not the workspace's cached values. A "stale" value is never shown beside the reported one.
- **Offered:** click a row (`ListItem <name>`).
- **Action:** click `devbox-1` in the Hosts list with the fake reporting gh 2.90, claude 2.7, codex 0.101.
- **Result:** the chips read "gh 2.90", "claude 2.7", "codex 0.101". The chips "gh 2.83", "claude 2.4", "codex 0.98" are gone.
- **Evidence:** existing snapshot `28-hosts.view.txt` shows the pre-report values; the assertion itself is in the test.
- **Replay check:** `bazel test //apps/humata2/desktop:machines_flow_test --test_filter=the_hosts_card_shows_the_selected_machines_tools`

### 3. An offline row: Reconnect, then Connecting…, then the row is online
- **Screen:** an offline machine row has a "Reconnect" button (`host-connect <name>`). A machine that was stopped shows "Start" instead. While connecting the button reads "Connecting…" and is disabled. The row's status is offline with "last seen <time>".
- **Offered:** Reconnect (`host-connect`); select the row; the name link.
- **Action:** click "Reconnect" on "gpu-box".
- **Result:** the machine's `online` flag becomes true in the workspace (`host_reconnect_and_shell_use_the_selected_machine`). The offline case keeps its last detail (`an_offline_machine_keeps_its_last_detail_and_reconnects`).
- **Evidence:** snapshot `28-hosts.view.txt` shows the offline "Reconnect" state for gpu-box.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=host_reconnect_and_shell_use_the_selected_machine` and `bazel test //apps/humata2/desktop:machines_flow_test --test_filter=an_offline_machine_keeps_its_last_detail_and_reconnects`

### 4. A machine whose sessions are stopped: Start sessions
- **Screen:** the row's button reads "Start sessions" and the row's session column reads "Sessions stopped". The detail card says "Sessions stopped by <who> at <time>; everything is kept.", and its first button is "Start sessions" in place of "Open a shell".
- **Offered:** "Start sessions" (`host-start`, `host-detail-start`), with "Starting…" while it runs and a disabled "Free space…".
- **Action:** none in phase 1 (state from the snapshot).
- **Result:** the snapshot shows the row "cloud-vm-3" and devbox-1 with "Start sessions" present. The stop and start confirms are in the machine page's own steps (step 19 and the frames 72 to 76 tests), not in this record.
- **Evidence:** `desktop/tests/__snapshots__/native/74-hosts-sessions-stopped.png` and `74-hosts-sessions-stopped.view.txt`.
- **Replay check:** `bazel test //apps/humata2/desktop:machines_flow_test --test_filter=start_sessions_asks_first_and_starts_only_when_confirmed`

### 5. A disk over 85% full: "Free space…" is a dead button
- **Screen:** the row's button and the detail card's button both read "Free space…", disabled. Above 85% used, the detail card adds a warning in orange: "N GB free: new clones stop at 5 GB." (the "5 GB" text is fixed in the code).
- **Offered:** nothing. The button is `.disabled(true)` in both places (`ui/src/hosts.rs`), and clicking it only selects the row.
- **Action:** click "Free space…" on devbox-1 (disk 97% used in the fixture).
- **Result:** nothing else happens in code: the row is selected, and no action or page opens. The machine page's Overview lists "Disk 97% used" but has no free-space action either.
- **Evidence:** `28-hosts.view.txt` (`Button "Free space…" disabled`) and `59-machine-page.view.txt`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=hosts_and_settings_reference_images` (the disabled state is in the snapshot).

### 6. Open a shell from a machine's detail
- **Screen:** the "Open a shell" button sits left of the disabled "Free space…" in the detail card. It is disabled when the machine is offline.
- **Offered:** "Open a shell" (`host-open-shell`).
- **Action:** click "Open a shell" on gpu-box after it reconnects.
- **Result:** the New session sheet opens with `machine == gpu-box` and `agent == Shell` (`NewSession` overlay).
- **Evidence:** not captured as an image; the sheet's state is asserted in the test.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=host_reconnect_and_shell_use_the_selected_machine`

### 7. Add a machine: the dialog's fields
- **Screen:** dialog "Add machine" with: a "Host" row (a pop-up button, "Choose a host" until one is picked, and a hint "from ~/.ssh/config, or type a host"), a text field "or type a host" on the right, a Cancel button, and an "Add" button (disabled until a host or a provider is chosen). A "Name" row appears only when a provider is chosen (see step 10). After Add starts, a "Steps" list shows.
- **Offered:** the "Host" pop-up (`add-machine-host-popup`); the typed host field; Cancel (`add-machine-cancel`); Add (`add-machine-start`); Esc closes.
- **Action:** click "Add a machine…" in the Hosts page.
- **Result:** the dialog opens, loads the choices, and selects the first listed host in the Host row. The snapshot shows the dialog over the Hosts page with "Button Host" and the typed-host field.
- **Evidence:** `apps/humata2/desktop/tests/__snapshots__/native/56-add-machine.png` and `56-add-machine.view.txt`.
- **Replay check:** `bazel test //apps/humata2/desktop:machines_flow_test --test_filter=an_add_over_ssh_fails_at_install_and_try_again_retries_the_same_add`
- **Note:** the view has no "Name" field for a plain SSH host (step 10).

### 8. The Host pop-up: hosts first, then providers, or the empty hint
- **Screen:** list box "Hosts and providers", with each `~/.ssh/config` host as an option (`add-machine-host <name>`), then each provider (`add-machine-provider <id>`). With no hosts: "No hosts in ~/.ssh/config. Type one instead."
- **Offered:** pick a host or a provider (closes the pop-up); click outside (scrim `add-machine-host-scrim`) closes it.
- **Action:** open the pop-up and pick "devbox-2".
- **Result:** the pick closes the pop-up and sets the choice.
- **Evidence:** `56-add-machine.view.txt` shows the pop-up is closed after the pick.
- **Replay check:** `bazel test //apps/humata2/desktop:machines_flow_test --test_filter=the_popup_lists_the_hosts_then_the_providers_and_a_provider_adds_with_its_params`

### 9. Type a host: a typed host wins over the pop-up's pick
- **Screen:** the field beside the Host row. Typed text is used as the host (`Choice::Host`), and "Add" enables only when the trimmed text is non-empty.
- **Offered:** type in "or type a host" (`TypeHost`); clearing the field returns to the pick.
- **Action:** pick devbox-2 in the pop-up, then type "box-9.lan".
- **Result:** the add targets box-9.lan (`AddTarget::Ssh`), not devbox-2.
- **Evidence:** none captured; asserted in the test.
- **Replay check:** `bazel test //apps/humata2/desktop:machines_flow_test --test_filter=a_typed_host_wins_over_the_popups_pick` and `--test_filter=a_host_can_be_typed_instead_of_picked`

### 10. A provider: its params and the Name field
- **Screen:** the provider's params each get a row labelled by the param (for example "Corporate SSH" params), then the "Name" row ("Name for the machine" placeholder, aria "Machine name"). Required params keep Add disabled until filled.
- **Offered:** the provider pop-up entry (`add-machine-provider corp-ssh`), the param inputs, the Name input.
- **Action:** pick corp-ssh, set the name to "corp-vm", click Add.
- **Result:** the add request carries the params and the name; the provider's own login flow may follow (step 14).
- **Evidence:** snapshot `57-add-machine-login.view.txt` (the corp-ssh flow, and the dialog after Add).
- **Replay check:** `bazel test //apps/humata2/desktop:live_machines_test --test_filter=the_popup_lists_a_provider_and_it_adds_a_machine_with_its_params`
- **Note:** for a plain SSH host the Name row is not drawn at all (`overlay_add_machine.rs` puts it inside the provider branch), so a host cannot be named in the dialog (Surprises, item 5).

### 11. Add: the steps run in order
- **Screen:** the "Steps" list: "Connect over SSH", "Check the system (arch, disk, git, gh)", "Install humatad", "Start it and open the link", "Find agents", and "Log in" when a login happens. Each row reads "Not run", "Running…", "done", or "failed", with a detail line.
- **Offered:** while running: Cancel (cancels the operation). No Add button.
- **Action:** click Add for devbox-2.
- **Result:** the steps move in order. A success closes the dialog and selects the new machine, with the toast "Added <name>."
- **Evidence:** `56-add-machine.view.txt` (done, done, failed, not run, not run).
- **Replay check:** `bazel test //apps/humata2/desktop:machines_flow_test --test_filter=an_add_over_ssh_fails_at_install_and_try_again_retries_the_same_add`
- **Live check:** `bazel test //apps/humata2/desktop:live_machines_test --test_filter=adding_a_machine_over_ssh_shows_its_host_key_and_lists_it_once_confirmed` (PENDING RUN for evidence)

### 12. A failed step: the red card with a fix, and Try again
- **Screen:** dialog alert "Install humatad failed on devbox-2", the words of the failure, "To fix it, run this on devbox-2, then Try again:", a code line "sudo chown -R $USER ~/.local/bin", and a "# run this on devbox-2, then Try again" line. Buttons: Cancel, "Try again".
- **Offered:** Try again (`add-machine-try-again`), which calls `retry` on the same operation (not a new add); Cancel.
- **Action:** click Try again after the fix.
- **Result:** the steps reset to "Not run" and the same operation runs again (`Call::Retry`).
- **Evidence:** `apps/humata2/desktop/tests/__snapshots__/native/56-add-machine.png` (fixture failure card).
- **Replay check:** `bazel test //apps/humata2/desktop:machines_flow_test --test_filter=an_add_over_ssh_fails_at_install_and_try_again_retries_the_same_add`
- **Note:** the live client never sets a fix (Surprises, item 6).

### 13. A new host key: Confirm or Don't add
- **Screen:** a card "Host key": "This host's key is new to this computer. Confirm only if the fingerprint matches what the host shows.", the fingerprint in mono, and buttons "Confirm" and "Don't add".
- **Offered:** Confirm (`add-machine-confirm-key`) adds again with the fingerprint trusted; "Don't add" (`add-machine-decline-key`) drops the key and shows the Add button again. A key that differs from one already pinned is a red error, not this card (the daemon's words).
- **Action:** add a host whose key is new; Confirm.
- **Result:** Confirm re-runs the add with `trusted_fingerprint` set; Don't add changes nothing.
- **Evidence:** none captured in phase 1.
- **Replay check:** `bazel test //apps/humata2/desktop:machines_flow_test --test_filter=confirming_a_new_host_key_adds_trusting_it` and `--test_filter=declining_a_new_host_key_adds_nothing`; live: `bazel test //apps/humata2/desktop:live_machines_test --test_filter=adding_a_machine_over_ssh_shows_its_host_key`

### 14. A login needed: Open interactively, then the terminal in the dialog
- **Screen:** banner "Login needed" with the line the connect stopped at ("Verification code:" or "The connect command didn't finish on its own…") and an "Open interactively" button. Then the "Login terminal" pane with the sentence "<who> is asking for a verification code, so Humata opened it here. Finish logging in, then press Done." Buttons: Cancel, "Done, I'm logged in".
- **Offered:** Open interactively (`add-machine-open-login`); keys typed into the terminal (`LoginInput`); terminal resize; "Done, I'm logged in" (`add-machine-login-done`); Cancel.
- **Action:** click "Open interactively" on the corp-ssh login, then "Done, I'm logged in" after the code.
- **Result:** the snapshot shows the dark terminal with "Reconnecting will need you" and both buttons. Done waits for the answer; on a login failure the reason shows next to Done.
- **Evidence:** `apps/humata2/desktop/tests/__snapshots__/native/57-add-machine-login.png` and `57-add-machine-login.view.txt`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=add_machine_login_reference_image` and `bazel test //apps/humata2/desktop:machines_flow_test --test_filter=a_login_shows_the_terminal_takes_keys_and_leads_to_the_unattended_offer`

### 15. After the login: the unattended offer, or it closes
- **Screen:** "Logged in to corp-vm · humatad 2.0.4 running" and the steps list with "Log in: done" and the rest done. Then "Reconnect without you?" with three radio choices: "Add a Humata key to authorized_keys" (selected), "Keep corp-ssh's cached token" (provider only), "Keep logging in by hand". Buttons: "Not now" (closes the dialog; the machine is kept interactive) and "Make it unattended" (shows "Making it unattended…" while it runs).
- **Offered:** the three choices (`add-machine-method-key`, `-token`, `-hand`); Not now (`add-machine-skip`); Make it unattended (`add-machine-make-unattended`). A plain SSH host has no token choice.
- **Action:** click "Login done"; Not now or Make it unattended.
- **Result:** Not now and a successful Make it unattended close the dialog and say "Added <name>." A login that cannot be made unattended closes when done.
- **Evidence:** `apps/humata2/desktop/tests/__snapshots__/native/58-add-machine-unattended.png` and `58-add-machine-unattended.view.txt` (radio buttons, Not now, Make it unattended).
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=add_machine_unattended_offer_reference_image` and `bazel test //apps/humata2/desktop:machines_flow_test --test_filter=keeping_the_login_by_hand_adds_the_machine_without_making_it_unattended`

### 16. DiskLow: a machine with under 2 GiB free is refused (PENDING RUN)
- **Screen:** expected: the Steps list stops at "Check the system (arch, disk, git, gh): failed" with a red card. The card's title is "Check the system (arch, disk, git, gh) failed" (`StepKind::label` + " failed", from `clients/live/src/machines.rs`), and its words are the daemon's sentence: "<machine> has <N> MiB free in its home folder and Humata needs 2048 MiB; free some space or grow the disk, then try again". Buttons: Cancel and "Try again".
- **Offered:** Try again (retries the same operation, after the person frees space on the machine); Cancel.
- **Action:** add a machine whose home folder has under 2048 MiB free (the real VM: 1.3 GiB free). In phase 2, the daemon's own test is used instead, since the VM is an environment block.
- **Result (expected):** the add stops at Check. Nothing is installed (the refusal is in `daemon/machines/src/setup.rs`, before the install). The check is `facts.disk_free_kib / 1024 < NEED_FREE_MIB` (2 GiB).
- **Evidence:** PENDING RUN: `bazel test //apps/humata2/daemon/machines:setup_test --test_filter=disk` and copy `bazel-testlogs/apps/humata2/daemon/machines/setup_test/test.log` to `verify/evidence/X-F/disk-low-setup.log`.
- **Replay check:** `bazel test //apps/humata2/daemon/machines:setup_test --test_filter=disk` (expected: pass; the test expects `SetupError::DiskLow`). The check measures `df -Pk $HOME` (`daemon/machines/src/check.rs`), so it is the home folder's free space.
- **Note:** if `setup_test` does not build or pass, record the failure output as evidence; do not skip it.

### 17. This Mac is behind: the add stops and offers "Migrate This Mac"
- **Screen:** the add is refused for a Linux machine because This Mac's machine tools are from another release. Card "This Mac needs migrating" (`add-machine-needs-migrate`) with the daemon's words (which say "use Migrate on This Mac, or open Humata again"), and a "Migrate This Mac" button (`add-machine-migrate`) only while a Migrate item for This Mac's daemon is waiting or failed. The Add button is hidden while the card shows. Expected text on the card: "Migrating This Mac… the add runs again when it is done." while migrating.
- **Offered:** Migrate This Mac (approves the same Migrate item the Inbox shows, through the parent). Cancel.
- **Action:** add a Linux SSH host while This Mac's `humatad2` is from release A and the tools are from release B.
- **Result:** the card appears; no machine is added. Source of the refusal: `with_machine_tools` in `daemon/humatad/src/serve.rs` marks the platform stale, and `Bundle::payloads` returns `HostNeedsMigrate`, which becomes `errors.proto` kind 61 (`host_needs_migrate`) and `AddOutcome::NeedsMigrate`.
- **Evidence:** not captured as an image; the reducer is covered in `features/hosts/tests/migrate.rs`.
- **Replay check:** `bazel test //apps/humata2/daemon/humatad:humatad2_unit_test --test_filter=a_linux_add_after_an_app_update_asks_to_migrate_this_mac_first` and `bazel test //apps/humata2/features/hosts:migrate_test`

### 18. Migrate This Mac: the add runs again once the daemon is back
- **Screen:** the card shows "Migrating This Mac… the add runs again when it is done." with no button. On a failed migrate: the failure's message and fix in red, and "Migrate This Mac" again.
- **Offered:** Migrate This Mac (only while the item is Waiting or Failed); Cancel.
- **Action:** click "Migrate This Mac", then the daemon comes back at the staged release.
- **Result:** the Migrate item is approved through the parent (`Delegate::Migrate`), the add runs again from the same inputs, and the dialog closes with "Added <name>." An item that goes back to waiting after the click is a failure, not a hang.
- **Evidence:** none captured.
- **Replay check:** `bazel test //apps/humata2/features/app:add_machine_migrate_test --test_filter=migrating_this_mac_from_a_blocked_add_runs_the_add_again_once_the_daemon_is_back` and `bazel test //apps/humata2/features/app:upgrade_reconnect_test --test_filter=the_app_reconnects_once_this_macs_daemon_is_back`

### 19. The machine page: Overview and the Storage tab (frame 59)
- **Screen:** heading "devbox-1" with buttons "Open a shell", "New session on devbox-1", "More" (the ⋯ menu), and "Sync skills". Tabs: "Overview" (selected) and "Storage". The Overview holds: a "Health" group (disk "Disk 97% used", the variables with "Set value…" and "Remove" for each, "New variable…"), the sessions as buttons, "Humata folders by channel" with "Open in Files" per folder, the agent-access rules ("Read files on other machines: Allow", "Push its branch or open a pull request: Ask me", and so on), and "Change in Agent access".
- **Offered:** Open a shell; New session; More; Sync skills; each tab; a session row (opens it); a variable's Set value… and Remove; New variable…; Open in Files; Change in Agent access.
- **Action:** open devbox-1 from the Hosts row name.
- **Result:** the page is the one in the snapshot. The Storage tab is an empty placeholder in code (`Tab::Storage => div().id("machine-storage")`, no content), and the spec's "Holders" tab does not exist.
- **Evidence:** `apps/humata2/desktop/tests/__snapshots__/native/59-machine-page.png` and `59-machine-page.view.txt`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=machine_page_reference_image`
- **PENDING RUN (Storage tab screenshot, optional):** no test clicks the Storage tab on this page; phase 2 may add a probe only if the coach asks. Otherwise rely on the code.

### 20. The machine page's ⋯ menu: Sync skills, Stop all sessions…, Reconnect now
- **Screen:** a card with "Sync skills", "Stop all sessions…" (only while sessions run), and "Reconnect now" (only while the link is down).
- **Offered:** each menu item; click outside closes (scrim `machine-menu-scrim`).
- **Action:** click "More".
- **Result:** the snapshot shows the card with `machine-sync-skills` present. There is no "Remove machine" and no "Forget machine" item, in the menu or anywhere on the page (`grep` finds neither in `ui/src/machine_page.rs`).
- **Evidence:** `apps/humata2/desktop/tests/__snapshots__/native/machine-menu-open.png` and `machine-menu-open.view.txt`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=machine_menu_open_reference_image`

### 21. A machine whose daemon has a waiting Migrate item: read-only page (PENDING RUN)
- **Screen:** the banner `machine-upgrade-banner` (role Status), "Migrate the daemon on <machine> first. Nothing was started or sent; its sessions keep running.", with an "Open in Inbox" link (`machine-open-in-inbox`). While read-only, New session and Open a shell are refused with a note (`machine-migration-refusal`). Migrating, failed and migrated states have their own text.
- **Offered:** Open in Inbox; opening a session still works; New session and Open a shell are refused.
- **Action:** open a machine whose daemon has a waiting Migrate item (fake `Migrate` request).
- **Result (from tests): the page is read-only; a session still opens; New session is refused.**
- **Evidence:** PENDING RUN: `bazel test //apps/humata2/features/hosts:upgrade_test` and copy the log to `verify/evidence/X-F/migrate-banner-upgrade.log`.
- **Replay check:** `bazel test //apps/humata2/features/hosts:upgrade_test --test_filter=a_waiting_daemon_migrate_makes_the_page_read_only`

### 22. Remove machine: CLI only, nothing in the app (PENDING RUN for the CLI)
- **Screen:** none in the app. The spec (LD-10) says Remove machine lives on the machine page; the code has no such item (see step 20).
- **Offered:** the CLI only: `humata host remove <machine>`. Without `--dry-run`, `--plan` or `--yes` it refuses (exit 2, USAGE) and changes nothing. `--dry-run` prints a plan and its token. `--plan <token>` runs it and `--wait` follows it.
- **Action (CLI):** `humata host remove box-1 --dry-run`, then `humata host remove box-1 --plan <token> --wait`.
- **Result (test):** the unconfirmed remove is refused with USAGE and the machine stays; the planned token removes it (`state == succeeded`, machine gone).
- **Evidence:** PENDING RUN: `bazel test //apps/humata2/cli:hosts_live_test --test_filter=a_machine_is_added_after_its_key_is_trusted_shown_reconnected_and_removed_by_plan` and copy the log to `verify/evidence/X-F/remove-cli.log`.
- **Replay check:** `bazel test //apps/humata2/cli:hosts_live_test --test_filter=a_machine_is_added_after_its_key_is_trusted_shown_reconnected_and_removed_by_plan`

### 23. Forget machine: CLI only, and no test (gap)
- **Screen:** none in the app. The spec (LD-11) says Forget machine is for a machine that is gone, and its confirm lists the lost work per session. Neither the app nor the tests show that list.
- **Offered:** CLI only: `humata host forget <machine>` with the same plan and token flow as remove (`PlanForgetMachine` then `ForgetMachine`).
- **Action (CLI, PENDING RUN):** `humata host forget <machine> --dry-run` on a machine that is unreachable.
- **Result (expected from the code):** the plan prints the lost work; `--plan` forgets it.
- **Evidence:** PENDING RUN: none of the existing tests exercise `forget`. Phase 2 may run `humata host forget box-1 --dry-run` against the throwaway daemon from `hosts_live` and record the output, or record the gap.
- **Replay check:** gap; no test exists. Replay by hand: `humata host forget <machine> --dry-run` expects `outcome` `planned` and exit 0.

### 24. The `humata host` commands (and the `machine` alias)
- **Screen:** CLI text output (JSON with `--json`). The command is `host` (`cli/src/args.rs`, `Command::Host`, with the visible alias `machine`). Subcommands: `ls`, `show`, `add <ssh-host> [--name] [--root] [--trust-host-key <fingerprint>]`, `reconnect`, `remove`, `forget`, `root [--default] [--move-channels]`, `vm-tier`, `storage`, `clear-build-cache`, and others (`humata host var`, `humata host stop|start`).
- **Offered:** the subcommands above; `humata machine ...` works as the alias of `host`.
- **Action:** `humata host ls` against a throwaway daemon (PENDING RUN), and `humata hosts ls` (the name in the brief).
- **Result (expected):** `host ls` prints the machines with "This Mac" first (exit 0). `hosts ls` is not a command: clap rejects it with a usage error, exit 2 (USAGE in `cli/src/exit.rs`). The tests use `host` throughout (`cli/tests/hosts_live.rs`).
- **Evidence:** PENDING RUN: build `//apps/humata2/cli:humata` and `//apps/humata2/daemon/humatad:humatad2_admin`, run `humatad2 serve --admin` with its own `--root`, `--state`, `--socket`, then `HUMATA_SOCKET=<socket> bazel-bin/apps/humata2/cli/humata host ls; echo exit=$?` and `... hosts ls; echo exit=$?`. Save to `verify/evidence/X-F/cli-host-ls.txt`.
- **Replay check:** `bazel test //apps/humata2/cli:hosts_live_test --test_filter=a_machine_is_added_after_its_key_is_trusted` (uses `host ls`); `humata hosts ls` expects exit 2.

### 25. An agent cannot add or remove a machine
- **Screen:** none. The CLI reports to an agent that it can't add or remove, and the agent sees no machines.
- **Offered:** none for agents.
- **Action:** an agent runs `humata host add` or `humata host remove` through its CLI.
- **Result (test):** refused, and the agent's list is empty (`an_agent_cannot_add_or_remove_a_machine_and_sees_none`). The spec (MP-24, the permission table) says "Remove, forget or delete machines: Never" for agents.
- **Evidence:** PENDING RUN: `bazel test //apps/humata2/cli:hosts_live_test --test_filter=an_agent_cannot_add_or_remove_a_machine_and_sees_none` and copy the log.
- **Replay check:** `bazel test //apps/humata2/cli:hosts_live_test --test_filter=an_agent_cannot_add_or_remove_a_machine_and_sees_none`

## Not reachable headless

- **The Add machine dialog's live login with a real terminal:** the terminal pane is drawn from the login stream; the fake login in `live_machines` covers the flow, but the terminal's typing and colour are not checked by a headless frame.
- **The Migrate This Mac card as a frame:** no native snapshot exists for `add-machine-needs-migrate` or the migrate banner; the reducer and the features tests cover it, and a snapshot would need a new `native` test and a record run (not allowed in phase 2 without the coach).
- **The DiskLow card as a frame:** the fake's failure card (frame 56) uses a fix; no native test scripts a DiskLow `AddFailure`. The real VM is an environment block (1.3 GiB free).
- **Remove and forget in the app:** not built; the CLI is the only way.
- **Storage tab on the machine page:** an empty placeholder; nothing to reach.
- **Real hosts over SSH:** `live_machines` uses a fake `ssh` and fake machine, not a real host; the real VM is blocked by DiskLow.

## Surprises

1. **`humata hosts` is not a command.** The CLI's subcommand is `host` (with the alias `machine`). `humata hosts ls` is a usage error (exit 2, USAGE). Facts from `cli/src/args.rs` (`#[command(subcommand, alias = "machine")] Host(HostCommand)`).
2. **No Remove or Forget in the app.** The spec (LD-10, LD-11) puts Remove machine on the machine page. The page's ⋯ menu has only Sync skills, Stop all sessions…, and Reconnect now. Removing or forgetting a machine is CLI only (`humata host remove|forget`). `forget` has no test in `cli/tests/`.
3. **The machine page has no "Holders" tab.** Tabs are "Overview" and "Storage" only. The Storage tab is an empty `div` (`ui/src/machine_page.rs`, `Tab::Storage`), so clicking it shows nothing.
4. **"Free space…" is always disabled.** In both the Hosts row and the detail card (`.disabled(true)` in `ui/src/hosts.rs`). The orange warning above 85% says "new clones stop at 5 GB" (fixed text), and nothing on the page frees space.
5. **No Name field for a plain SSH host.** The Name row is drawn only inside the provider branch of `overlay_add_machine.rs`. An SSH add takes the daemon's name (from the host). The CLI has `--name`; the dialog does not.
6. **The live failure card never has a fix.** `clients/live/src/machines.rs` `outcome_of_failure` sets `fix: None`, so a live failure (including DiskLow) shows the daemon's sentence and "Try again", with no "run this on <machine>" command. The fixture (frame 56) shows a fix, so the fixture and the live app differ.
7. **A blocked add with no Migrate item is a dead end (from the code; verify in phase 2).** While `state.migrate` is set, the primary button is hidden (`(None, None)` in `overlay_add_machine.rs`), and "Migrate This Mac" shows only if `migrate_offer()` finds a waiting or failed item. With no such item, the dialog offers only Cancel, under the card's words. The reducer test `without_an_item_the_action_does_nothing` (`features/hosts/tests/migrate.rs`) covers the reducer, not the dialog's buttons.
8. **An add that ends without an outcome may leave the dialog stuck (from the code; verify in phase 2).** In `features/hosts/src/add.rs`, `ProgressEnded` with a clean end and no outcome sets `running = false`, but `operation` stays set, so Add stays disabled (`ready` requires `operation.is_none()`), and Try again needs a failure card. Only Cancel remains, and no error is shown. A link that breaks with an error does set `error`. Phase 2 check: script an add stream that ends with no outcome in `machines_flow` and look at the buttons.
9. **Remove and forget are the same gap as the Storage tab:** the machine page offers no way to remove a machine from the app at all, and the Storage tab is empty.

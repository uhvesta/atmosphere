# The SSH VM is added as a machine in the real app and gets Humata's tools

Layer: real. Platforms: macOS (the VM is Linux arm64 under Lima on this Mac). Linux and web are not covered.

## Starting state

- This Mac runs a real Humata 2 desktop app and its own `humatad2`, on a throwaway home (see Test setup). Nothing is added yet.
- `h2-terminal-c1` is a Debian Lima VM: 1 CPU, 0.75 GiB RAM, 4 GiB disk, no host mounts, no agent forwarding. It is running and its OpenSSH answers on loopback.
- `~/.humata2` on the VM does not exist. `/tmp/h2-terminal-holder` may exist from an earlier `provision`.
- The Lima SSH config is at `$HOME/.lima/h2-terminal-c1/ssh.config`. It names the alias `lima-h2-terminal-c1`.
- I am on Home, with the Hosts page one click away.

## Test setup

- Start and check the VM: `./terminal-ssh-demo.sh start`, then `./terminal-ssh-demo.sh status`. For a read-only check: `bazel test //apps/humata2/terminal/platform_test:ssh_demo_status_test --test_output=all` (manual, no-sandbox, macOS only).
- Never use the shared VM's state for anything else while this runs. Do not stop, restart or delete it.
- Run a daemon with its own home so the owner's `~/.humata2`, `~/.ssh` and keys are never touched: set `HOME` to a temp folder and run `humatad2 serve --admin ... --ssh-config "$HOME_REAL/.lima/h2-terminal-c1/ssh.config" --tools <bundle>` (`daemon/humatad/src/serve.rs` lines 131-135 take `--ssh-config`; its default is `~/.ssh/config`). The bundle is `//apps/humata2/daemon/humatad:tools`, which holds the `linux-aarch64` set.
- The Lima config points at Lima's own key under `~/.lima/_config`. The run only reads it through that config. No key is copied into the temp home.
- Headless equivalent: the `humata` CLI against that daemon (`humata hosts add lima-h2-terminal-c1`, then again with `--trust-host-key <fingerprint>`).
- **Needs a free screen** for the app-window steps (the Add machine dialog and the machine page). The headless equivalent is the CLI above, and `desktop/tests/live_machines.rs` style tests (which use a fake `ssh`, not this VM).
- Needs no model login.

## Steps

1. **I do:** Run `./terminal-ssh-demo.sh start` and `ssh -F "$HOME/.lima/h2-terminal-c1/ssh.config" lima-h2-terminal-c1 'df -Pk $HOME; uname -sm; command -v git'`.
   **I see:** A shell answer on the VM: `Linux aarch64`, `git` found, and free space in KiB.
   **State and events:** The VM is up. Nothing of Humata is on it yet.
   **Status:** Code, no test. The helper is `terminal-ssh-demo.sh`; the read-only check is `terminal/platform_test:ssh_demo_status_test` (manual, not run for this case). No test adds the VM through Humata.

2. **I do:** Start the throwaway `humatad2` with `--ssh-config` set to the Lima config, open Hosts and press Add a machine…
   **I see:** The Add machine dialog opens with a host field.
   **State and events:** The dialog loads its choices. The VM's alias is not offered in the list, so I type it.
   **Status:** Fakes only. `features/hosts/tests/add.rs::open_loads_the_choices_and_picks_the_first_host` (see `macro/08-first-run-and-adding-a-machine.md` step 6). Listing hosts from an SSH config is Not built (OVERVIEW.md line 299).

3. **I do:** Type `lima-h2-terminal-c1` and press Add.
   **I see:** The dialog stops at Connect. It shows the VM's host key fingerprint and asks me to confirm. Nothing is installed.
   **State and events:** The add waits for a host key. A bare alias is resolved through the daemon's `--ssh-config` (`daemon/machines/src/manage.rs` `parse_ssh_host`, lines 1254-1283); a typed alias with no such file falls back to `/dev/null` and would not find the VM.
   **Status:** Works (real daemon) for the stop-at-key behaviour, against a fake `ssh`: `clients/live/tests/machines.rs::an_add_without_the_host_key_ends_asking_for_its_fingerprint`. Against this real VM it is Code, no test: the Lima config sets its own host key options, and no test runs Humata's pinned-key `UserKnownHostsFile` path (`daemon/machines/src/link.rs:381-398`) over it.

4. **I do:** Read the fingerprint on the VM with `ssh -F ... lima-h2-terminal-c1 'ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub'`, compare, and press Confirm.
   **I see:** The dialog runs Connect, Check, Install and Start, one after the other.
   **State and events:** The confirmed key is pinned in the throwaway daemon's `known_hosts`. The add reruns under the same operation id.
   **Status:** Fakes only. `features/hosts/tests/add.rs::a_host_key_request_waits_for_confirmation_and_then_trusts_it` and `desktop/tests/machines_flow.rs::confirming_a_new_host_key_adds_trusting_it`. The daemon half is Works (real daemon) against a fake `ssh`: `daemon/humatad/tests/machines.rs::a_machine_is_added_once_its_host_key_is_confirmed_then_removed_by_plan`.

5. **I do:** Nothing. The Check step reads the VM.
   **I see:** The step's output shows `Linux aarch64`, the free disk, and the `git` version. No `gh` version.
   **State and events:** The check refuses a Mac, an unsupported platform, a missing `git`, or less than 2 GiB free (`NEED_FREE_MIB`, `daemon/machines/src/setup.rs:29`). On a 4 GiB VM the 2 GiB rule can fail on a fresh Debian disk. Measure `df` in step 1 first.
   **Status:** Works (real daemon) for what the check reads, over a fake `ssh`: `daemon/humatad/tests/machines.rs::the_check_step_of_an_add_carries_what_it_read_from_the_machine`. The real VM's numbers are unproven.

6. **I do:** Nothing. Install puts the tools on the VM.
   **I see:** The Install step is Done. On the VM, `~/.humata2/bin` holds `humatad2`, `humata2_holder`, `humata` and `stack`, and a `bundle.json` beside them.
   **State and events:** The files are the `linux-aarch64` set, checked by SHA-256, swapped in atomically (`daemon/machines/src/install.rs`, `script`, line 328). The app embeds the Linux sets through `daemon/humatad/BUILD.bazel` `tools` (`//bazel/platforms:linux_arm64` is named). The app bundle carries only `tools_mac` (`app/BUILD.bazel:23`, `:65`), so a real app finds no Linux binaries to send.
   **Status:** Likely bug. `app/BUILD.bazel` passes `tools_mac` (darwin-arm64 only, `daemon/humatad/BUILD.bazel` lines 153-165) while TS-2 says the app embeds every machine platform. Confirm: run the real app, add the VM, and read the failure at Install or Check. The daemon half with fake binaries is Works (real daemon): `daemon/machines/tests/lima.rs::a_lima_vm_is_created_set_up_stopped_and_deleted` (its own VM `humata2-machines-e2e`, fake files), not this VM.

7. **I do:** Nothing. Start brings the daemon up on the VM.
   **I see:** The VM shows online in Hosts. Its machine page shows the platform, the release, and the link as open.
   **State and events:** `humatad2 serve --role machine` runs on the VM under systemd or `setsid`; the link opens over SSH.
   **Status:** Works (real daemon) with fake machine binaries over a fake `ssh`: `daemon/humatad/tests/machines.rs::a_machine_is_added_once_its_host_key_is_confirmed_then_removed_by_plan`. The real `humatad2` on a real 0.75 GiB VM is Code, no test.

8. **I do:** On the VM, run `~/.humata2/bin/humata --version` and `ls -l ~/.humata2/bin`.
   **I see:** The same release as this Mac, and four executable files.
   **State and events:** None beyond the files.
   **Status:** Code, no test. No test reads the installed files over a real SSH to a real Linux guest.

9. **I do:** Open a shell session on the VM from the new machine's page.
   **I see:** A shell on the VM, under the throwaway channel.
   **State and events:** The session is recorded on the VM.
   **Status:** Works (real daemon) over a fake machine: `daemon/humatad/tests/remote.rs::a_session_started_on_the_machine_runs_there_and_the_host_relays_its_terminal`. Terminal over the real VM is Works (real shell) for the terminal component only: `terminal/lab/tests/clipboard_ssh.rs::ssh_unicode_multiline_color_copy_pasteback_and_focus_are_exact`. The app path (app, `humatad2`, SSH, `humatad2 --role machine`, holder) is unproven.

## Branches

### The VM is stopped

- **When:** the VM was stopped before I press Add.
- **I do:** Press Add, read the message, start the VM with `./terminal-ssh-demo.sh start`, then Try again.
- **I see:** Connect fails and says the machine could not be reached, and to try again if it is booting. After start, the same add finishes.
- **Expected outcome:** One machine. Nothing half installed.
- **Status:** Fakes only. `daemon/machines/tests/setup.rs::a_machine_that_never_answers_is_unreachable_and_can_be_deleted`.

### SSH refuses the key

- **When:** the Lima config's identity file is missing or wrong, or I give `user@host:port` and no key.
- **I do:** Read the failed step, fix the config, Try again.
- **I see:** The step is named. The dialog may offer "Log in by hand".
- **Expected outcome:** The message says how to fix it. No machine is listed.
- **Status:** Works (real daemon) over a fake `ssh`: `clients/live/tests/machines.rs::a_failed_add_names_its_step_and_trying_again_follows_the_same_operation`. The key-login fix text near `daemon/humatad/src/machine_service.rs:990` is Code, no test.

### Too little disk on the VM

- **When:** the VM has less than 2 GiB free, or Install runs out of space.
- **I do:** Free space on the VM (remove `/tmp` leftovers), then Try again.
- **I see:** At Check: a message with the free MiB and the 2048 MiB needed. At Install: the failed step and its output.
- **Expected outcome:** The add stops at the step. A retry under the same id finishes.
- **Status:** Fakes only. `daemon/machines/tests/setup.rs` (the `disk` case, 512 MiB free, expects `SetupError::DiskLow`; line 396). The retry is Works (real daemon): `daemon/humatad/tests/machines.rs::a_failed_add_tried_again_runs_under_the_same_id_and_a_removal_cannot_be_cancelled`.

### Re-adding after `provision`

- **When:** `./terminal-ssh-demo.sh provision` has put a holder at `/tmp/h2-terminal-holder`, and I add the VM or add it again.
- **I do:** Add the VM.
- **I see:** The add finishes. The holder in `~/.humata2/bin` is Humata's own; `/tmp/h2-terminal-holder` is left alone and keeps serving the terminal experiment.
- **Expected outcome:** The two holders do not interfere. An already-added VM is not duplicated.
- **Status:** Code, no test. No test adds the VM beside a provisioned holder. Install never touches `/tmp` (`daemon/machines/src/install.rs`).

### I decline the host key

- **When:** the fingerprint does not match.
- **I do:** Press Don't add.
- **I see:** The dialog stays open. Nothing is installed.
- **Expected outcome:** No machine, nothing on the VM.
- **Status:** Fakes only. `features/hosts/tests/add.rs::declining_a_host_key_adds_nothing_and_keeps_the_dialog`.

### The throwaway daemon's `--ssh-config` is missing

- **When:** the daemon runs with its default `~/.ssh/config` and the alias is not in it.
- **I do:** Type `lima-h2-terminal-c1` and press Add.
- **I see:** Connect fails (the alias falls to `/dev/null`).
- **Expected outcome:** A message that names the alias and says where its SSH settings are read from.
- **Status:** Code, no test. `daemon/machines/src/manage.rs:1272-1279`. Whether the message says to pass `--ssh-config` or add the alias to `~/.ssh/config` is unchecked.

## Expected outcomes

- [ ] Nothing is installed on the VM until I confirm its host key.
- [ ] After Confirm, `~/.humata2/bin` on the VM holds `humatad2`, `humata2_holder`, `humata`, `stack` and `bundle.json`, all linux-aarch64.
- [ ] The VM shows online with its release, and a shell session opens on it.
- [ ] A stopped VM, a refused key and a full disk each fail at a named step with a fix, and Try again runs the same add.
- [ ] The owner's `~/.humata2`, `~/.ssh` and keys are not read or written.
- [ ] A provisioned `/tmp/h2-terminal-holder` and the VM's shared state are untouched.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. VM is up and probed | Code, no test | `terminal-ssh-demo.sh`; `ssh_demo_status_test` (manual) |
| 2. Add machine dialog | Fakes only | `features/hosts/tests/add.rs::open_loads_the_choices_and_picks_the_first_host` |
| 3. First contact asks for the key | Works (real daemon), fake ssh; real VM Code, no test | `clients/live/tests/machines.rs::an_add_without_the_host_key_ends_asking_for_its_fingerprint` |
| 4. Confirm the key | Fakes only | `desktop/tests/machines_flow.rs::confirming_a_new_host_key_adds_trusting_it` |
| 5. Check reads the VM | Works (real daemon), fake ssh | `daemon/humatad/tests/machines.rs::the_check_step_of_an_add_carries_what_it_read_from_the_machine` |
| 6. Install linux-aarch64 tools | Likely bug | `app/BUILD.bazel:23`, `daemon/humatad/BUILD.bazel:153-165` |
| 7. Start, link online | Works (real daemon), fake machine | `daemon/humatad/tests/machines.rs::a_machine_is_added_once_its_host_key_is_confirmed_then_removed_by_plan` |
| 8. Installed files read on the VM | Code, no test | `daemon/machines/src/install.rs` |
| 9. Shell on the VM | Works (real daemon), fake machine | `daemon/humatad/tests/remote.rs::a_session_started_on_the_machine_runs_there_and_the_host_relays_its_terminal` |
| Branch: VM stopped | Fakes only | `daemon/machines/tests/setup.rs::a_machine_that_never_answers_is_unreachable_and_can_be_deleted` |
| Branch: key refused | Works (real daemon), fake ssh | `clients/live/tests/machines.rs::a_failed_add_names_its_step_and_trying_again_follows_the_same_operation` |
| Branch: too little disk | Fakes only | `daemon/machines/tests/setup.rs` (`disk` case) |
| Branch: re-add after provision | Code, no test | `daemon/machines/src/install.rs` |
| Branch: decline the key | Fakes only | `features/hosts/tests/add.rs::declining_a_host_key_adds_nothing_and_keeps_the_dialog` |
| Branch: missing `--ssh-config` | Code, no test | `daemon/machines/src/manage.rs:1272-1279` |

## Gaps and likely bugs

- **Not built (OVERVIEW.md line 299):** the Add dialog does not list the hosts in an SSH config, so the alias must be typed, and the daemon must be run with `--ssh-config`.
- **Not built (MACH T10):** `humata hosts trust-key` (see `macro/08-first-run-and-adding-a-machine.md`).
- **Likely bug:** the app bundle carries `tools_mac` (darwin-arm64 only), so a real app may have no Linux binaries to put on the VM, against TS-2. Confirm by adding the VM from the real app. The daemon's own `tools` bundle does have `linux-aarch64`.
- **Code, no test:** the whole add against a real Linux guest with real `humatad2`, holder and `humata`. The only real-VM add test (`lima.rs`) uses fake files in its own VM, and needs `limactl` and network.
- **Risk to measure:** a 4 GiB disk and a 2 GiB free rule (`NEED_FREE_MIB`). Read `df -Pk $HOME` on the VM before the run.
- **Unproven:** the real app path, and its desktop-to-VM terminal, are proven nowhere (see `terminal/lab/HANDOFF.md`).

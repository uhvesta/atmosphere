# Adding the real test VM stops at the disk check, 1.3 GiB free against 2 GiB needed, and that is an environment block, not a product bug

Layer: macro (the real-VM add, the owner's own app). Platforms: macOS for this Mac and the app; the VM is Linux (arm64).

## Starting state

- The shared Lima VM `h2-terminal-c1` is running. It has a 4 GiB root disk. It is never stopped, restarted or resized by these steps.
- Its free space reads about 1.3 GiB (1304 MiB) in the home folder that the add will use. After a cleanup of test leftovers the owner measured 1651 MiB, which is still short.
- The VM is reached over SSH by its alias (kept in the owner's own SSH config, not in these notes).
- This Mac's host daemon is at the current release and its tools match it, so the Migrate blocker from F-1 does not apply.
- Hosts is open and Add machine is not yet open.
- I have not run the real-VM tests in this session.

## Reproduce headless

- The product check is unit-tested with fakes: `//apps/humata2/daemon/machines:machines_unit_test` covers the Check step, including the disk case in `daemon/machines/tests/setup.rs` (a 512 MiB machine is refused with `DiskLow`).
- The add's behaviour after a failed step is tested with a real daemon and a fake box: `//apps/humata2/daemon/humatad:machines_test`, with `a_failed_add_tried_again_runs_under_the_same_id_and_a_removal_cannot_be_cancelled`.
- The real VM is a manual, gated target. It runs only when `TERMINAL_LAB_SSH_CONFIG` names the VM's Lima `ssh.config`, and it is run by hand:
  - start the VM with the terminal demo helper (`terminal-ssh-demo.sh start`, as in `terminal/platform_test/SSH_DEMO.md`);
  - `bazel test //apps/humata2/desktop:live_flows_ssh_test --test_env=TERMINAL_LAB_SSH_CONFIG=<the VM's ssh.config> --test_output=streamed`.
- The real-VM test does not reproduce the app's add. It gives its throwaway user a sparse 3 GiB home on a loop image (D691), so the 2 GiB check passes there. The check the real-VM test makes first (`preflight`, 600 MiB on `/var/tmp`) passes at 1.3 GiB.
- To see the real app's add refused, run the owner's own add from the app against the VM with no loop image. Capture the Check step's text in the dialog and the Inbox, and `df -Pk "$HOME"` on the VM.
- What cannot be done headless: the real VM's disk and the app's own add against it. Do not stop or restart the VM to test this.

## Steps

1. **I do:** Open Add machine in the app and type the VM's SSH alias as the host. Press Start.  
   **I see:** The dialog runs Connect, then Check.  
   **State and events:** The daemon connects over the real SSH path and reads the machine's facts (`daemon/machines/src/check.rs`).  
   **Status:** Works (real daemon). `daemon/humatad/tests/machines.rs::the_check_step_of_an_add_carries_what_it_read_from_the_machine` (real daemon, fake box, reads the free disk). The real VM's own reading is not checked by any test run here.

2. **I do:** Watch the Check step.  
   **I see:** Check fails. The step's text says the VM has 1304 MiB free in its home folder and Humata needs 2048 MiB, then "free some space or grow the disk, then try again".  
   **State and events:** `daemon/machines/src/setup.rs` (around lines 674-681) compares `df -Pk "$HOME"` with `NEED_FREE_MIB` (2 GiB) and returns `SetupError::DiskLow`. The add is undone at once (`daemon/machines/src/manage.rs`).
   **Expected outcome:** Nothing is installed on the VM. The dialog shows the step as failed with the words above.  
   **Status:** Fakes only. `daemon/machines/tests/setup.rs` (the `disk` case, 512 MiB free, expects `DiskLow`). The real VM's failing run is recorded in D694, not by a test run here.

3. **I do:** Read the dialog's failure card.  
   **I see:** The Check step is red. The message is the one in step 2. No Migrate This Mac button shows, because this is not a migrate block.  
   **State and events:** `AddOutcome::Failed` with the step Check (`clients/live/src/machines.rs`).  
   **Status:** Code, no test for the dialog's drawing. The mapping of a failed step to the outcome is unit-tested in `clients/live/src/machines.rs` (`outcome_of_failure` tests, fakes).

4. **I do:** Read the Inbox and check that nothing is left on the VM.  
   **I see:** No Inbox item for this. On the VM, `humatad2` is not installed and no service is registered.  
   **State and events:** The undo removes what the add made. For a failure at Check, nothing was made.  
   **Status:** Works (real daemon) for the undo of a failed add, with a fake box: `daemon/humatad/tests/machines.rs::a_failed_add_tried_again_runs_under_the_same_id_and_a_removal_cannot_be_cancelled`. The VM-side check is Code, no test.

5. **I do:** Free space on the VM by hand, or have the owner grow its disk. Then press Try again in the dialog.  
   **I see:** The Check step passes and the add continues to Install.  
   **State and events:** The retry runs under the same operation id; the free disk is read again.  
   **Status:** Fakes only for the Check passing with enough space (`daemon/machines/tests/setup.rs`, the other cases with enough space). The retry under the same id is Works (real daemon) with a fake box (`daemon/humatad/tests/machines.rs::a_failed_add_tried_again_runs_under_the_same_id_and_a_removal_cannot_be_cancelled`).

6. **I do:** Run the owner's real-VM test with the VM's config, after the start helper is up.  
   **I see:** The test checks the VM's architecture and its `/var/tmp` free space (at least 600 MiB), then makes its own user, mounts its sparse home and runs the add. The 2 GiB check passes there.  
   **State and events:** The test's home is a 3 GiB sparse loop image, so `df` on `$HOME` reads the image, not the 4 GiB root disk (D691).  
   **Status:** Works (real daemon, manual gated target). `desktop/tests/live_flows/real_vm.rs` (`preflight`, `add_machine`), run as `//apps/humata2/desktop:live_flows_ssh_test`. It runs only when the VM's config is given.

7. **I do:** Read D694 and the owner's list.  
   **I see:** The decision: the product check stays at 2 GiB. The fix is to grow the VM's disk to 6 GiB, on the owner's list.  
   **State and events:** No product change. The VM is not resized by any test.  
   **Status:** Not built (environment). D694 is an owner action on the VM's disk, not a product feature, so it has no milestone. Nothing in the product needs to change for this step.

## Branches

### The owner frees space and tries again

- **When:** After step 2, I free space on the VM's home (or the owner grows the root disk) and press Try again.
- **I do:** Press Try again in the dialog.
- **I see:** The add restarts under the same operation and passes Check, once the free space is at or above 2 GiB.
- **State and events:** The retry keeps the same id, and a removal cannot be cancelled (`daemon/humatad/tests/machines.rs::a_failed_add_tried_again_runs_under_the_same_id_and_a_removal_cannot_be_cancelled`).
- **Expected outcome:** The add goes on to Install.
- **Status:** Works (real daemon, fake box).

### Free space is 1.9 GiB and the owner stops there

- **When:** The owner frees space but stays below 2 GiB (for example 1651 MiB after a cleanup).
- **I do:** Press Try again.
- **I see:** The same DiskLow words, with the new free figure.
- **State and events:** The check reads `$HOME` again.
- **Expected outcome:** The message shows the new number, so the owner knows how much more to free.
- **Status:** Fakes only. `daemon/machines/tests/setup.rs` (the `disk` case). The live reading of 1651 MiB is recorded in D694.

### The preflight passes but the add still fails

- **When:** The real-VM test's preflight reads enough free space on `/var/tmp` (600 MiB), but the app's add checks `$HOME`.
- **I do:** Run the real-VM test (passes) and then the app's add against the VM with no loop home (fails).
- **I see:** The test passes. The app's add fails with DiskLow.
- **State and events:** The two checks look at different paths: the test's preflight uses `df /var/tmp`, the product check uses `df $HOME`.
- **Expected outcome:** A green real-VM test does not prove the app's add passes on the shared VM. This is the environment block, not a product bug.
- **Status:** Works (real daemon) for the test; the app's add against the shared VM is not run here. Recorded in D694 (`DECISIONS.md` around line 3454).

### The VM is stopped or rebooted during the add

- **When:** Someone stops or reboots the shared VM during the add.
- **I do:** Nothing; this must not happen.
- **I see:** The link drops. The add ends with a link error.
- **State and events:** The link drops and the add is undone or left for the next recover (`daemon/machines/src/manage.rs`).
- **Expected outcome:** Nothing is left half installed. This is not tested against the VM, and the rules say the VM is never stopped by these steps.
- **Status:** Code, no test for the real VM. The undo of an add cut short is covered for fake boxes by `daemon/humatad/tests/machines.rs::an_add_cancelled_while_it_connects_ends_cancelled_and_leaves_nothing`.

### The disk reading is wrong for the real folder

- **When:** `df -Pk "$HOME"` on the machine returns no number (for example, no `df`).
- **I do:** Start the add.
- **I see:** The Check passes without a disk check (no DiskLow).
- **State and events:** `daemon/machines/src/check.rs` (the fact reads `disk_free_kib` as optional, and `setup.rs` skips the check when it is missing).
- **Expected outcome:** The add goes on, and the disk is not checked. This is the documented behaviour for a missing number.
- **Status:** Fakes only. `daemon/machines/src/check.rs` tests (`disk_free_kib` is None when the reading is missing).

## Expected outcomes

- [ ] The real app's add to the shared VM stops at Check with DiskLow, when the VM has less than 2 GiB free in the home folder.
- [ ] The words say how much is free, how much is needed, and what to do (free space or grow the disk, then try again).
- [ ] Nothing is installed on the VM after a DiskLow refusal.
- [ ] After enough space is free, Try again runs the add under the same id.
- [ ] The real-VM test does not change the VM. It passes with its own sparse home, and that proves nothing about the shared VM's root disk.
- [ ] The VM is never stopped, restarted or resized by these steps.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Connect and Check run | Works (real daemon, fake box) | `daemon/humatad/tests/machines.rs::the_check_step_of_an_add_carries_what_it_read_from_the_machine` |
| 2. DiskLow words at Check | Fakes only | `daemon/machines/tests/setup.rs` (`disk` case); real VM result recorded in D694 |
| 3. Dialog shows the failed step | Code, no test | `clients/live/src/machines.rs` (`outcome_of_failure`, unit tests only) |
| 4. Nothing left on the VM | Works (real daemon, fake box) | `daemon/humatad/tests/machines.rs::a_failed_add_tried_again_runs_under_the_same_id_and_a_removal_cannot_be_cancelled` |
| 5. Try again after space is freed | Works (real daemon, fake box) | `daemon/humatad/tests/machines.rs::a_failed_add_tried_again_runs_under_the_same_id_and_a_removal_cannot_be_cancelled` |
| 6. Real-VM test with its sparse home | Works (real daemon, manual gated target) | `desktop/tests/live_flows/real_vm.rs`, `//apps/humata2/desktop:live_flows_ssh_test` |
| 7. D694: grow the VM disk | Not built (environment, owner action) | `DECISIONS.md` D694 (line 3452) |
| B1. Freed space, Try again | Works (real daemon, fake box) | `daemon/humatad/tests/machines.rs::a_failed_add_tried_again_runs_under_the_same_id_and_a_removal_cannot_be_cancelled` |
| B2. Still under 2 GiB | Fakes only | `daemon/machines/tests/setup.rs` (`disk` case) |
| B3. Preflight passes, app add fails | Works (real daemon, manual) for the test; app add not run here | `desktop/tests/live_flows/real_vm.rs` (`preflight`); D694 |
| B4. VM stopped mid-add | Code, no test for the real VM | `daemon/machines/src/manage.rs`; `daemon/humatad/tests/machines.rs::an_add_cancelled_while_it_connects_ends_cancelled_and_leaves_nothing` |
| B5. No disk reading | Fakes only | `daemon/machines/src/check.rs` tests |

## Gaps and likely bugs

- **Environment block, not a product bug:** The shared VM has a 4 GiB root disk and too little free space in the home folder for the 2 GiB rule (D694). The product check is right to refuse. The owner's fix is to grow the disk to 6 GiB; the product check stays as it is.
- **Not built (environment):** There is no test that runs the app's add against the shared VM with its real home. That is by design: the real-VM test uses its own sparse home (D691), so it passes the check and cannot show the refusal.
- **Code, no test (step 3):** The dialog's drawing of a DiskLow failure has no native snapshot.
- **Likely bug:** None found in the DiskLow path. The message names the folder ("home folder") and the fix, and the undo runs for every failed step.
- **Doc note:** `flows/cases/real/01-the-vm-becomes-a-machine.md` (line 94) and `flows/cases/real/04-agent-clis-on-the-vm-for-a-test.md` (line 92) cite the same DiskLow test. They do not say that the real VM's own add is refused, so a reader may take the real-VM pass as proof that the app's add passes on the shared VM.

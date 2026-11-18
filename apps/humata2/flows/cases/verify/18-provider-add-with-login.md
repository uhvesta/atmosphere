# Adding a machine through a provider, logging in by hand, and making it unattended leaves a machine that starts without me

Layer: macro, verify layer (flow 05, step 9). Platforms: macOS (the app and this Mac's daemon). The provider here is a fake login program, not a real Lima VM; Linux is not covered.

## Starting state

- This Mac's Humata is running, with its daemon and `humata` tools installed.
- `#ledger-v2` exists with `acme/ledger`, on This Mac only. No other machine is linked yet.
- Settings has one provider, "Corp", with a create step and a connect step. Its connect step asks for a login code, so the add stops and asks me to log in. (The flow's Lima provider has the same shape; the test uses a fake login program.)
- The Hosts page is open, with no machine selected.
- I hold nothing in a terminal. No machine named `corp-vm` exists yet.

## Reproduce headless

- Real daemon, real app model, fake provider login: `//apps/humata2/desktop:live_machines_test`, file `desktop/tests/live_machines.rs`. Run `the_popup_lists_a_provider_and_it_adds_a_machine_with_its_params` and `a_login_by_hand_in_the_dialog_leads_to_the_unattended_offer_and_the_machine`. The second test is the whole login path: it types a code into the login terminal, clicks **Done, I'm logged in**, then **Make it unattended**.
- Real daemon, client level: `//apps/humata2/clients/live:machines_test`, file `clients/live/tests/machines.rs`. Run `a_provider_that_needs_a_login_is_logged_in_by_hand_and_becomes_a_machine`, `a_trusted_add_runs_every_step_and_ends_with_the_machine` and `a_first_skills_sync_is_previewed_then_applied_with_its_plan`.
- Daemon only: `//apps/humata2/daemon/humatad:providers_test` (file `daemon/humatad/tests/providers.rs`) for the provider's machine creation, and `//apps/humata2/daemon/humatad:login_test` (file `daemon/humatad/tests/login.rs`) for the login that the daemon runs.
- Headless CLI route: a throwaway `humatad2` with its own `--root` and `HOME`, a providers file with one provider whose connect step is a fake login script, then `humata hosts add` against it. Capture the operation's steps and the exit code (0 on success, 7 when a precondition such as a login is still needed).
- Evidence to capture: the test log, the operation's final state (Succeeded), and the machine list from `humata machines list --json`.
- Cannot be done headless: a real login in a real terminal window, and a real Lima VM (its disk is short, so Add machine fails with DiskLow until the owner grows it, D694). The nearest headless equivalent is the fake login program above.

## Steps

1. **I do:** on Hosts, click **Add a machine…**. In the pop-up, pick **Host**, then pick "Corp". Type the name `corp-vm` and click **Add**.  
   **I see:** the dialog moves to the add steps. The first steps tick off in order.  
   **State and events:** an Operation is created for the add. The machine is not recorded yet.  
   **Status:** Works (real daemon). `desktop/tests/live_machines.rs::the_popup_lists_a_provider_and_it_adds_a_machine_with_its_params` (the pop-up lists the provider and the typed parameter and default reach the provider). The pop-up order (hosts, then providers) is **Fakes only**: `desktop/tests/machines_flow.rs::the_popup_lists_the_hosts_then_the_providers_and_a_provider_adds_with_its_params`.

2. **I do:** wait.  
   **I see:** the add stops at a login. A banner names the line the connect stopped at, and the button **Open interactively** appears.  
   **State and events:** the add waits for the login. Nothing is recorded: `corp-vm` is not in the machine list yet.  
   **Status:** Works (real daemon). `desktop/tests/live_machines.rs::a_login_by_hand_in_the_dialog_leads_to_the_unattended_offer_and_the_machine` (it asserts that the add asks for a login and that the daemon does not list the machine until the login is done). Client level: `clients/live/tests/machines.rs::a_provider_that_needs_a_login_is_logged_in_by_hand_and_becomes_a_machine`.

3. **I do:** click **Open interactively**. In the login terminal, type the code `123456` and press Return. When the machine's shell prompt shows, click **Done, I'm logged in**.  
   **I see:** the terminal shows "Verification code", then the shell prompt. After **Done, I'm logged in**, the dialog offers to make the machine unattended.  
   **State and events:** the login is its own step, shown as "Log in: done". Keys typed into the terminal reach the login program; a wrong code would be denied there.  
   **Status:** Works (real daemon, fake login program). The same live test as step 2 types the code and checks the prompt. Keys reaching the program, and a wrong code being denied: `daemon/humatad/tests/login.rs` (the daemon's login session). The real login in a real terminal window is not run headless.

4. **I do:** on the offer, keep the first choice and click **Make it unattended**.  
   **I see:** two choices: "Add a Humata key to authorized_keys" and "Keep corp's cached token" (the provider's name is filled in). The buttons are **Make it unattended** and **Not now**. After **Make it unattended**, the dialog closes and the new machine is selected in the list.  
   **State and events:** the daemon records the machine and its unattended method (`UnattendedMethod`). The machine is listed. The app's selection is set to it.  
   **Status:** Works (real daemon). `desktop/tests/live_machines.rs::a_login_by_hand_in_the_dialog_leads_to_the_unattended_offer_and_the_machine`. The live test picks "Keep corp's cached token" before **Make it unattended**; the authorized-keys choice is shown but not clicked there. The choice is **Fakes only** in `desktop/tests/machines_flow.rs::a_login_shows_the_terminal_takes_keys_and_leads_to_the_unattended_offer`.

5. **I do:** (on the other path) click **Not now** on the offer instead of **Make it unattended**.  
   **I see:** the dialog closes and `corp-vm` is listed. It still needs my login when it connects.  
   **State and events:** the machine is recorded without an unattended method.  
   **Status:** Fakes only: `desktop/tests/machines_flow.rs::keeping_the_login_by_hand_adds_the_machine_without_making_it_unattended`. Live, Not run.

6. **I do:** nothing more; check the machine page.  
   **I see:** the machine is online, with Humata's tools installed. The first skills sync shows its plan as a preview, and I confirm it before anything is applied.  
   **State and events:** Operation updated per step, then Succeeded; Machine created; the skills preview is applied only after the confirm.  
   **Status:** Works (real daemon, fake `ssh` and fake machine). `clients/live/tests/machines.rs::a_trusted_add_runs_every_step_and_ends_with_the_machine` and `::a_first_skills_sync_is_previewed_then_applied_with_its_plan`. The daemon side: `daemon/humatad/tests/providers.rs::a_machine_is_created_through_its_provider_then_run_and_deleted_by_the_provider`. The preview's confirm in the dialog is **Fakes only**, not checked in this run.

## Branches

### The login is not done in time, and the add fails at a step

- **When:** the provider's login (or a machine step) fails before I click **Done, I'm logged in**.
- **I do:** let the login fail (close the login terminal, or type a wrong code).
- **I see:** the failed step says what failed and what to run on the machine. **Try again** appears.
- **Expected outcome:** the add does not record a machine. **Try again** finishes the same operation with no second **Add**.
- **Status:** Works (real daemon) for the SSH add, not the provider login: `desktop/tests/live_machines.rs::an_add_over_ssh_fails_at_install_and_try_again_finishes_it` and `clients/live/tests/machines.rs::a_failed_add_names_its_step_and_trying_again_follows_the_same_operation`. For the provider's login, a wrong code is denied by the login program (step 3). The "Try again" on a login failure is **Code, no test**.

### The machine cannot be made unattended

- **When:** the login is done, but the daemon cannot add Humata's key to the machine (for example, the machine asks for a person every time).
- **I do:** click **Make it unattended**.
- **I see:** the error says the machine must be reachable without a person now, and to log in again and then make it unattended. The dialog closes when done.
- **Expected outcome:** the machine stays listed, still logged in by hand; the error names the fix.
- **Status:** Fakes only: `desktop/tests/machines_flow.rs::a_login_that_cannot_be_made_unattended_closes_when_done`. The error text is in `daemon/machines/src/login.rs:187`; no daemon test asserts that text, so I check only the behaviour here.

### The provider's connect has no SSH address to read

- **When:** the provider's connect command is a provider tool and it prints no SSH details, and I click **Make it unattended**.
- **I do:** click **Make it unattended** on the offer.
- **I see:** the error says Humata can't tell which SSH address the machine has, and names the two fixes: an `identity` command that prints SSH details (with the host key), or `host` and `user` params on the provider.
- **Expected outcome:** the machine is not made unattended; the error names both fixes and the offer stays.
- **Status:** Code, no test. The error is `NoSshAddress` in `daemon/machines/src/login.rs:167`; no test in `daemon/humatad/tests/providers.rs` or `login.rs` reaches it.

### Host variables and the real Lima VM

- **When:** I try to set a host variable on the new machine, or add a real Lima VM.
- **I do:** open the machine page's variables, or add the Lima provider's VM.
- **I see:** host variables answer "not served yet". The real VM add fails at its disk check (DiskLow) on the VM here.
- **Expected outcome:** host variables are not built; the VM add is blocked by the environment, not by a bug.
- **Status:** Not built, no milestone for host variables (MACH T10 follow-up). The client answer is not in `clients/live/tests/machines.rs`: the flow cites `host_variables_answer_not_served_yet`, which does not exist. Real Lima: **Not built: LINUX** (blocked; `real/01` records the DiskLow block).

## Expected outcomes

- [ ] The provider appears in the add pop-up, and its typed parameter reaches it.
- [ ] The add stops at the login and records no machine until I finish the login.
- [ ] Keys typed into the login terminal reach the login program.
- [ ] **Make it unattended** records the machine with its chosen method and selects it in the list.
- [ ] **Not now** records the machine as needing my login again.
- [ ] Each step of the add ends in Succeeded; the skills sync is previewed before it is applied.
- [ ] A login that cannot be made unattended keeps the machine and names the fix.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Add a machine through the pop-up with a provider | Works (real daemon) | `desktop/tests/live_machines.rs::the_popup_lists_a_provider_and_it_adds_a_machine_with_its_params` |
| 2. Add stops at the login; nothing recorded | Works (real daemon) | `desktop/tests/live_machines.rs::a_login_by_hand_in_the_dialog_leads_to_the_unattended_offer_and_the_machine`; `clients/live/tests/machines.rs::a_provider_that_needs_a_login_is_logged_in_by_hand_and_becomes_a_machine` |
| 3. Login terminal takes keys; Done, I'm logged in | Works (real daemon, fake login) | `desktop/tests/live_machines.rs::a_login_by_hand_in_the_dialog_leads_to_the_unattended_offer_and_the_machine`; `daemon/humatad/tests/login.rs` |
| 4. Make it unattended with a chosen method | Works (real daemon) for the cached-token choice; Fakes only for the choice list | `desktop/tests/live_machines.rs::a_login_by_hand_in_the_dialog_leads_to_the_unattended_offer_and_the_machine`; `desktop/tests/machines_flow.rs::a_login_shows_the_terminal_takes_keys_and_leads_to_the_unattended_offer` |
| 5. Not now keeps the login by hand | Fakes only | `desktop/tests/machines_flow.rs::keeping_the_login_by_hand_adds_the_machine_without_making_it_unattended` |
| 6. Steps run; skills sync previewed then applied | Works (real daemon, fake ssh) | `clients/live/tests/machines.rs::a_trusted_add_runs_every_step_and_ends_with_the_machine`; `::a_first_skills_sync_is_previewed_then_applied_with_its_plan` |
| Branch: login or step fails, Try again | Works (real daemon) for SSH; Code, no test for a provider login | `desktop/tests/live_machines.rs::an_add_over_ssh_fails_at_install_and_try_again_finishes_it` |
| Branch: cannot make unattended | Fakes only | `desktop/tests/machines_flow.rs::a_login_that_cannot_be_made_unattended_closes_when_done` |
| Branch: provider gives no SSH address | Code, no test | `daemon/machines/src/login.rs:167` |
| Branch: host variables; real Lima VM | Not built (host variables, no milestone); Not built: LINUX (Lima) | `flows/cases/real/01-the-vm-becomes-a-machine.md` |

## Gaps and likely bugs

- Not built, no milestone: host variables. The flow cites `clients/live/tests/machines.rs::host_variables_answer_not_served_yet`, which does not exist. Fix the citation or add the test.
- Not built: LINUX. A real Lima VM add over real SSH is blocked by the VM's disk (DiskLow until the owner grows it, D694).
- Code, no test: a failed provider login followed by **Try again** (the SSH variant is tested, the login variant is not).
- Code, no test: the "no SSH address" failure from a provider's connect command (`daemon/machines/src/login.rs:167`).
- Likely bugs: none confirmed in this path. The unattended choice is a required radio in the dialog (`ui/src/overlay_add_machine.rs:914-925`); the flow's "Make it unattended" alone does not say that a choice comes first. That is a flow wording gap, not a code bug.

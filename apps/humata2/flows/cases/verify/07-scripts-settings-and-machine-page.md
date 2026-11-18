# Machine scripts are listed in Settings and on the machine page, and Run now updates the row by push

Layer: verify (scripts-ui). Platforms: macOS (the app). The Linux and web clients do not show Scripts.

## Starting state

- Humata 2 is running on this Mac. The app is open on the main window, with the Settings window closed.
- The machines are `This Mac` and `devbox-1`. `devbox-1` is connected over SSH and its link is up.
- The owner has a rules file at the Humata home with two rules:
  - `set-up-tools`, `on = "added"`, `once = true`, one step `run = "setup.sh"`.
  - `after-a-move`, `on = "session-arrived"`, `machines = ["devbox-1"]`, one step `run = "after-move.sh"`.
- The scripts folder holds `setup.sh` and `after-move.sh`.
- Nothing has run yet on `devbox-1`, except that `set-up-tools` ran once at its add (its last outcome is Passed).
- The owner is holding no path. The UI never asks for one.

## Reproduce headless

- No desktop test covers Scripts (`desktop/tests/native` and `desktop/tests/live_flows` have nothing on it), so use the feature crates.
- Feature tests (fakes, run headless, no window):
  - `bazel test //apps/humata2/features/settings:machine_scripts_test`
  - `bazel test //apps/humata2/features/hosts:scripts_test`
  - `bazel test //apps/humata2/features/app:machine_script_events_test`
- Daemon data source (a throwaway `humatad2`, own `HOME` and `--state`; see case D-2 for the start-up command): `HUMATA_SOCKET=<throwaway socket> humata machine script list devbox-1` prints the same rules and last runs the machine page draws. Capture the output as evidence.
- What cannot be done headless: the real window, the rendered Scripts block (no `.new.png` render exists for it), the editor that `open -t` starts, Finder for Reveal scripts, and a real `devbox-1` whose link can run a script. The nearest equivalent is the fake `Machines` client with `set_scripts` and `set_script_outcome`.

## Steps

1. **I do:** open Settings, then click **Machines** in its list.  
   **I see:** each machine with its Humata folder row. Under the list, a **Scripts** block with a short description, then the rule rows. Each row shows the rule name, its triggers joined with " · ", and "1 step". Below the rules: "Scripts: setup.sh, after-move.sh". Two buttons: **Edit rules** and **Reveal scripts**.  
   **State and events:** the app reads the rules and script names once, when the section opens (one `list_machine_scripts` call with no machine). Nothing else in Settings asks for a path.  
   **Status:** Fakes only. `features/settings/tests/machine_scripts.rs::opening_machines_reads_the_rules_and_the_script_names`. The rows and buttons are drawn in `ui/src/settings.rs` (`machine_scripts_block`), which no test renders: Code, no test for the drawing.

2. **I do:** look for a folder or path field anywhere in the Scripts block.  
   **I see:** none. There is no picker, no path text field and no Change button in the block.  
   **State and events:** no RPC sends a path from Settings.  
   **Status:** Code, no test. The block in `ui/src/settings.rs` (around lines 113–185) has only the two buttons; no test asserts it.

3. **I do:** click **Edit rules**.  
   **I see:** the rules file opens in the owner's text editor. If the file is missing, Humata first writes the commented sample there (the starter text from `humata2_script_sample`). If the file exists, it is not changed.  
   **State and events:** `SettingsAction::EditScriptRules` calls `Reveal::edit` with the rules path and the starter. No rule is written by an RPC.  
   **Status:** Fakes only. `features/settings/tests/machine_scripts.rs::edit_rules_opens_the_rules_file_in_the_editor` (FakeReveal records the path). The file creation is `make_file_if_missing` in `clients/src/reveal.rs`, which has unit tests at lines 279 and 290 on a real temp file. The `open -t` call itself is not tested: Code, no test.

4. **I do:** click **Reveal scripts**.  
   **I see:** Finder opens the scripts folder. If the folder was missing, Humata makes it first.  
   **State and events:** `Reveal::reveal_folder` runs `create_dir_all` and `open` on `~/.humata2/scripts`. No sample script file is written.  
   **Status:** Fakes only for the reducer (`features/settings/tests/machine_scripts.rs::reveal_scripts_shows_the_scripts_folder`). The folder creation is Code, no test. The sample file is Not built: see Gaps.

5. **I do:** save a third rule in the editor, close the editor, and return to the app's Settings, still on Machines.  
   **I see:** the new rule is not in the list yet. It appears only after I leave the Machines section and come back.  
   **State and events:** the Settings reducer reads the rules only when the section is opened, or when a run changes on the machine page or Settings (the feed event). Nothing watches the rules file or reads it on activation.  
   **Status:** Likely bug. See Gaps (stale rules). The branch that does re-read is covered by `features/app/tests/machine_script_events.rs::settings_machines_reads_the_rules_again_when_it_is_the_page_shown`, which proves only the feed case.

6. **I do:** open `devbox-1` from the sidebar (its machine page, Overview tab). Scroll to the **Scripts** section.  
   **I see:** a card titled "Scripts" with one row per rule. Each row shows the name, the triggers, and the last outcome as "Passed · <time>" in green, or "Never run" in grey. Each row has a grey **Run now** button on the right. `after-a-move` shows "Never run".  
   **State and events:** the page sends `ListMachineScripts` with `devbox-1`. The page reads the last run of each rule for this machine (`ScriptsLoaded`).  
   **Status:** Fakes only. `features/hosts/tests/scripts.rs::showing_the_page_lists_the_rules_with_their_last_run`. Gap: the daemon returns every rule, not only the rules for this machine. Expected for this case: a rule whose `machines` leaves out `devbox-1` does not show. See Gaps.

7. **I do:** click the `set-up-tools` row (on its name, not on Run now).  
   **I see:** the row opens. Below its header there is one chip per step (kind `run`, then the script label `setup.sh`), then the last run's steps: "Step 1: setup.sh · Passed", and the last 40 lines of its output in a mono face, or "No output." Clicking again closes the row.  
   **State and events:** `ToggleScript(name)` sets the open rule. Nothing is sent to the host.  
   **Status:** Code, no test for what the owner sees. The toggle alone is Fakes only: `features/hosts/tests/scripts.rs::opening_a_rule_toggles_its_steps`. The drawing of the chips and tails (`ui/src/machine_page.rs`, `scripts_section` and `script_tail`) is Code, no test. The 40-line cap is `TAIL_LINES` in `daemon/humatad/src/machine_scripts/live.rs:27`, which no test asserts on.

8. **I do:** click **Run now** on `set-up-tools`.  
   **I see:** the button changes to "Running…" and is disabled. The outcome text on the row reads "Running…" in orange. The row's click does not open or close the row, because the button stops the click.  
   **State and events:** `RunScript(rule)` sends `RunMachineScript` with the rule and the machine, no session, and a fresh idempotency key. A second click while it runs sends nothing.  
   **Status:** Fakes only: `features/hosts/tests/scripts.rs::run_now_is_running_until_the_run_ends_and_then_shows_its_outcome` and `run_now_while_the_rule_runs_sends_nothing_more`. The stop-propagation is in `ui/src/widgets.rs` (`push_quiet`), which has no test: Code, no test.

9. **I do:** wait for the run to end.  
   **I see:** the row shows "Passed · <time>" (or "Failed · <time>") with the new time. The button reads **Run now** again and is enabled. No timer or poll redraws it.  
   **State and events:** the daemon writes the run to the host's log and then sends one `MachineScriptRun` feed event for `devbox-1` (`machine_scripts/live.rs::changed`, called from `engine.rs` `finish`). The app reads the runs again on the open page (`ScriptRunChanged`). The Run now reply (`ScriptRan`) also reads them again.  
   **Status:** Fakes only. The reducer reads are proven by `run_now_is_running_until_the_run_ends_and_then_shows_its_outcome`. The event routing is proven by `features/app/tests/machine_script_events.rs::a_changed_run_on_the_open_machine_reads_its_runs_again`. The daemon side only has an in-process unit test of the feed payload, `daemon/humatad/src/machine_scripts/live.rs` (`a_change_goes_on_the_feed_with_the_machine_and_its_last_run`). No test runs the real daemon, a real script, and the feed end to end: Code, no test for that path.

10. **I do:** open a second window or use `humata machine script run set-up-tools devbox-1` while the page is open, and watch the row.
    **I see:** the row does not show "Running…" while the run goes on. It changes only when the run ends, to that run's outcome.
    **State and events:** the host never records a Running state. See Gaps.
    **Status:** Likely bug. See Gaps (no Running state from the host).

## Branches

### A run the host refuses shows the reason on its row

- **When:** `devbox-1` is waiting for a Migrate item, or the rule is not for this machine.
- **I do:** click **Run now**.
- **I see:** no run starts. The row shows the reason in red under its header, with `role = Alert`. If the Migrate item blocks it, the reason says that the machine is waiting for its upgrade.
- **Expected outcome:** nothing runs. The reason stays until the next run of this rule, and the button is enabled again.
- **Status:** Fakes only for the host refusal: `features/hosts/tests/scripts.rs::a_run_the_host_refuses_says_why_on_its_row`. The Migrate-blocked refusal (`Refused::MigrationNeeded`) has no test: Code, no test.

### A failed run keeps the end of the failed step's output

- **When:** the script exits non-zero.
- **I see:** the row's outcome reads "Failed · <time>" in red. Under the header, in red: "Step 1: setup.sh (exit 1)", then the last lines of the failed step's output. The tail shows without opening the row.
- **Expected outcome:** the user can see the cause without opening the log.
- **Status:** Fakes only: `features/hosts/tests/scripts.rs::a_failed_run_keeps_the_end_of_the_failed_steps_output`. The exit code shown comes from the step's `exit` field, which the fake fills in.

### A rules file that can't be used

- **When:** the rules file has a problem, for example a step names a script that is not there.
- **I see:** the Scripts section on the machine page shows the problem in red (`machine-scripts-problem`). In Settings, the same problem shows above the rule rows (`settings-scripts-problem`). The section does not list any rules.
- **Expected outcome:** the page says what to fix. The fix text names the rules file and `humata machine script check`.
- **Status:** Fakes only for the page: `features/hosts/tests/scripts.rs::a_rules_file_that_cannot_be_used_shows_its_problem_and_no_rules`. The Settings drawing is untested.

### No rules for this machine hides the section

- **When:** no rule applies to `devbox-1`, and the file is valid.
- **I see:** no Scripts section at all on the machine page.
- **Expected outcome:** a quiet page stays quiet.
- **Status:** Fakes only: `features/hosts/tests/scripts.rs::a_machine_with_no_rules_shows_no_scripts`.

### A failed read keeps the rules last read

- **When:** the host can't answer the read after a run ends.
- **I see:** the rules and their last runs stay on screen. A red line "machine-scripts-error" appears under the section title.
- **Expected outcome:** nothing disappears because one read failed.
- **Status:** Fakes only: `features/hosts/tests/scripts.rs::a_failed_read_keeps_the_rules_last_read`. The Settings version: `features/settings/tests/machine_scripts.rs::a_failed_read_keeps_what_was_read_and_says_why`.

### A change on another machine, or while Settings is closed, changes nothing

- **When:** the feed says a run on `other` changed while `devbox-1`'s page is open, or says a run changed while Settings is closed.
- **I see:** nothing changes, and no read is sent.
- **Status:** Fakes only: `features/app/tests/machine_script_events.rs::a_changed_run_on_another_machine_reads_nothing`, `settings_not_shown_reads_nothing_on_the_event`, and `features/hosts/tests/scripts.rs::the_feed_about_another_machine_changes_nothing`.

### Run now while the machine is offline

- **When:** the link to `devbox-1` is down.
- **I see:** the **Run now** button is disabled, and the last run and its time still show.
- **Expected outcome:** the page sends nothing, and the row says nothing is running.
- **Status:** Code, no test. The `.disabled(... || !context.online)` is in `ui/src/machine_page.rs` (`scripts_section`). The reducer itself does not check `online`.

### The app restarts during a run

- **When:** the app quits while a run on `devbox-1` is going.
- **I see (expected):** when the app opens the page again, the rule shows the last run the host's log holds. The app does not keep a stale "Running…".
- **Expected outcome:** the run that finishes is in the log, and the page shows it on the next read. Whether the daemon writes the run when the app is gone is not shown by any test.
- **Status:** Code, no test. The log is `daemon/humatad/src/machine_scripts/log.rs` (per machine, last 50 runs). Its unit tests cover keeping 50 runs and filtering by rule, not a restart.

### Edit rules when the editor can't open

- **When:** the system has no editor for the file type, or `open` fails.
- **I see:** a red line "settings-scripts-error" under the Scripts block with the reason.
- **Expected outcome:** the error says what failed. The rules file is unchanged.
- **Status:** Code, no test. The error is set in `features/settings/src/lib.rs` (`EditScriptRules`), but FakeReveal never fails.

## Expected outcomes

- [ ] Settings › Machines › Scripts lists each rule by name with its triggers and step count, and the script names.
- [ ] Settings › Machines › Scripts has Edit rules and Reveal scripts, and no path field or folder picker.
- [ ] Edit rules opens the rules file at its fixed path, making it from the commented starter first when it is missing.
- [ ] Reveal scripts opens the scripts folder, making it first when it is missing.
- [ ] The machine page's Scripts section lists the rules that apply to that machine, each with its last outcome and time.
- [ ] A row opens to show its steps, and its last run's step outputs (the last 40 lines).
- [ ] A failed step's exit and output tail show without opening the row.
- [ ] Run now sends one run, shows Running… until the run ends, then shows its outcome.
- [ ] The row changes when the host's feed says the run changed, with no timer and no poll.
- [ ] The feed about another machine, or while Settings is closed, causes no read.
- [ ] A refused run shows the reason on its row.
- [ ] A rules file that can't be used shows its problem and no rules.

## Status summary

| Step or branch | Status | Evidence |
| --- | --- | --- |
| 1. Settings lists rules and script names | Fakes only | `features/settings/tests/machine_scripts.rs::opening_machines_reads_the_rules_and_the_script_names`; drawing: Code, no test (`ui/src/settings.rs`) |
| 2. No folder picker in the Scripts block | Code, no test | `ui/src/settings.rs` lines 113–185 |
| 3. Edit rules opens the rules file | Fakes only | `features/settings/tests/machine_scripts.rs::edit_rules_opens_the_rules_file_in_the_editor`; `clients/src/reveal.rs` unit tests (file made); `open -t`: Code, no test |
| 4. Reveal scripts opens the folder | Fakes only | `features/settings/tests/machine_scripts.rs::reveal_scripts_shows_the_scripts_folder`; sample file: Not built |
| 5. Edited rules appear without reopening Machines | Likely bug | `features/settings/src/lib.rs` (reads only on section open or feed) |
| 6. Machine page lists rules and last runs | Fakes only | `features/hosts/tests/scripts.rs::showing_the_page_lists_the_rules_with_their_last_run`; rules not filtered by machine: Likely bug |
| 7. Row opens to steps and output tails | Code, no test | Toggle only: `features/hosts/tests/scripts.rs::opening_a_rule_toggles_its_steps` (Fakes only); drawing untested |
| 8. Run now sends one run and shows Running… | Fakes only | `features/hosts/tests/scripts.rs::run_now_is_running_until_the_run_ends_and_then_shows_its_outcome` |
| 9. Row updates by push when the run ends | Fakes only | `features/app/tests/machine_script_events.rs::a_changed_run_on_the_open_machine_reads_its_runs_again`; daemon feed: `daemon/humatad/src/machine_scripts/live.rs` unit test |
| 10. Row shows Running… for a run started elsewhere | Likely bug | `daemon/humatad/src/machine_scripts/mod.rs` Outcome has no Running; `engine.rs` `finish` |
| Branch: refused run shows reason | Fakes only | `features/hosts/tests/scripts.rs::a_run_the_host_refuses_says_why_on_its_row` |
| Branch: Migrate-blocked run refused | Code, no test | `features/hosts/src/page.rs` `RunScript` |
| Branch: failed run shows exit and tail | Fakes only | `features/hosts/tests/scripts.rs::a_failed_run_keeps_the_end_of_the_failed_steps_output` |
| Branch: rules file cannot be used | Fakes only | `features/hosts/tests/scripts.rs::a_rules_file_that_cannot_be_used_shows_its_problem_and_no_rules`; Settings drawing untested |
| Branch: no rules hides section | Fakes only | `features/hosts/tests/scripts.rs::a_machine_with_no_rules_shows_no_scripts` |
| Branch: failed read keeps rules | Fakes only | `features/hosts/tests/scripts.rs::a_failed_read_keeps_the_rules_last_read` |
| Branch: feed about another machine or Settings closed | Fakes only | `features/app/tests/machine_script_events.rs::a_changed_run_on_another_machine_reads_nothing`, `settings_not_shown_reads_nothing_on_the_event` |
| Branch: offline disables Run now | Code, no test | `ui/src/machine_page.rs` `scripts_section` |
| Branch: app restarts during a run | Code, no test | `daemon/humatad/src/machine_scripts/log.rs` |
| Branch: editor cannot open | Code, no test | `features/settings/src/lib.rs` `EditScriptRules` |


## Gaps and likely bugs

Not built:
- The sample script file: Reveal scripts makes the folder (`clients/src/reveal.rs` `reveal_folder`) but no commented sample script. Only the rules file gets a starter (Edit rules). D680 says Humata makes "a commented sample file" with the folder; only the rules half is built.
- Live refresh of the rules file: nothing re-reads the rules after Edit rules, on window activation, or on a file change (`features/settings/src/lib.rs`, `features/app/src/events.rs`).
- A real run on a machine is never tested. `daemon/machines/tests/machine_scripts.rs::a_bash_script_goes_to_the_machine_on_stdin` only builds the invocation, and the engine tests in `daemon/humatad/src/machine_scripts/engine.rs` use `FakeSteps`. No test runs real `bash -s`.

Likely bugs:
- The machine page lists every rule, not only those for this machine. `list()` in `daemon/humatad/src/machine_service/script_rpc.rs` sets `response.rules` from all rules and filters only the last runs by machine. The fake `clients/src/machines/fake.rs` (`scripts`, around lines 473–482) filters only last runs too, so no test catches it. Confirm: a rule with `machines = ["other"]` appears on `devbox-1`'s page as "Never run", and Run now on it is refused with "isn't for devbox-1".
- No Running state from the host. `ScriptOutcome::Running` exists (`clients/src/machines.rs` around line 461) and the page draws it, but the daemon's `Outcome` (`daemon/humatad/src/machine_scripts/mod.rs` line 55) has no Running variant. The engine logs and sends the feed event only at `finish` (`daemon/humatad/src/machine_scripts/engine.rs` around line 301). Confirm: start a run from the CLI and watch the page before it ends. It shows the previous outcome, not Running.
- Stale rules in Settings and on the machine page after Edit rules, until the section is reopened. Confirm: edit the rules file, save, return to Settings > Machines. The list is unchanged.
- Edit rules and Reveal scripts use fixed `~/.humata2` paths (`SCRIPT_RULES_PATH` and `SCRIPTS_FOLDER` in `clients/src/machines.rs`, lines 598–601), but the daemon's home is the parent of its `--state` folder (`daemon/humatad/src/serve.rs` around lines 976–987). They open the wrong files only when the daemon runs with a custom `--state`. Confirm with a throwaway daemon.

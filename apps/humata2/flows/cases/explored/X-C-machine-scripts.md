# Machine scripts: rules and scripts from Settings, the machine page, the Inbox and the CLI

Layer: explored · macOS · harness: `features/settings/tests/machine_scripts.rs`, `features/hosts/tests/scripts.rs`, `features/app/tests/machine_script_events.rs`, `cli/tests/machine_scripts.rs` (fake daemon), `daemon/humatad/src/machine_service/script_rpc.rs` and `daemon/humatad/src/machine_scripts/live.rs` (unit tests), and a throwaway `humatad2` for the CLI steps

Phase 1 (map from code, no Bazel). Steps marked **PENDING RUN** are filled in phase 2. No desktop native test drives any Scripts screen today, and no snapshot shows one (the machine page fixture and the Settings default section have no rules), so every GPUI screen step below is from the view code, not a rendered frame.

## Setup

- Rules file: `~/.humata2/config/machine-scripts.toml` (`SCRIPT_RULES_PATH`, `clients/src/machines.rs`). Missing file means no rules and no problem (`daemon/machines/src/machine_scripts.rs` `rules_path` read).
- Scripts folder: `~/.humata2/scripts` (`SCRIPTS_FOLDER`). Names are listed by `script_names`, sorted; a missing folder is an empty list.
- Commented starter for Edit rules: `daemon/machines/sample/machine-scripts.toml` (`SAMPLE_RULES`). Every line is a comment, so it parses to zero rules.
- Phase 2 uses a throwaway HOME under the scratchpad (`$S/home-c`), never the owner's `~/.humata2`. The daemon is started from `bazel-bin` with its own `--root`, `--state` and `--socket`; see PENDING RUN.
- Fixtures for the GPUI steps are the fakes: `FakeMachines::set_scripts`, `set_script_outcome` (`clients/src/machines/fake.rs`), `FakeReveal` (`clients/src/reveal.rs`).

## Steps

### 1. Settings › Machines, no rules file yet
- **Screen:** Settings tab "Machines" → a block titled "Scripts", with the line "What Humata runs on a machine when it is added, reconnects, or a session arrives or leaves. The rules and scripts are your own files; Humata never changes them." Below it, with no rules: "No scripts in the scripts folder yet." Then the buttons "Edit rules" and "Reveal scripts". No rules card, no error (`ui/src/settings.rs` `machine_scripts_block`).
- **Offered:** Edit rules, Reveal scripts (both `settings-scripts-*`).
- **Action:** Open Settings, then Machines (`SettingsAction::Section(Machines)`, `features/settings/src/lib.rs`).
- **Result:** Fires `read_scripts` (`clients.machines.scripts(None)`). On `Ok` it stores `scripts` and clears `scripts_error`. Nothing is written.
- **Evidence:** none yet. The existing `29-settings` snapshot is the default Settings section, not Machines. Not rendered in phase 1.
- **Replay check:** `bazel test //apps/humata2/features/settings:machine_scripts_test --test_filter=opening_machines_reads` (reducer: the read is made and the names stored). The screen itself is not reachable headless, see below.

### 2. Settings › Machines with rules, script names and warnings
- **Screen:** Under the description, one card with a row per rule: the rule's name as the row title, and as its subtitle `"<triggers joined by ' · '> · <N step|N steps>"`, for example `added · connected · 2 steps`. Each row's right-hand control is empty (`div().into_any_element()`). Then a dim line `Scripts: a.sh, b.sh`, or "No scripts in the scripts folder yet." Each warning from the host is a dim line (for example "rule setup names a machine that isn't added"). Then the two buttons.
- **Offered:** Edit rules, Reveal scripts. No per-rule action; a row does nothing when clicked.
- **Action:** None (the state is read-only). Driven by the fake: `set_scripts(Ok(MachineScripts { rules, script_names, warnings, .. }))`.
- **Result:** The block shows only what the rule list carries: name, triggers, step count. The machines list, `once`, and each step's kind and label are not shown here. They are on the CLI and the machine page.
- **Evidence:** none; not rendered in phase 1.
- **Replay check:** `bazel test //apps/humata2/features/settings:machine_scripts_test` (all five tests, including `a_failed_read_keeps_what_was_read_and_says_why`).

### 3. Settings › Machines › Edit rules, file missing
- **Screen:** The block as in step 1. Nothing changes on screen while the editor opens.
- **Offered:** Edit rules.
- **Action:** Click Edit rules (`SettingsAction::EditScriptRules`).
- **Result:** `RealReveal::edit` (`clients/src/reveal.rs`) creates `~/.humata2/config/machine-scripts.toml` with the commented starter if it is missing (a file that exists is never changed), then runs `open -t` on it, which opens the default text editor. An error is shown as a red alert `settings-scripts-error` until the next Machines read or the next Edit rules; success clears it and gives no other feedback. The list is not reloaded when the editor closes: Settings reads the rules only when the Machines tab is selected again.
- **Evidence:** the starter text is `daemon/machines/sample/machine-scripts.toml`. Not run: `open -t` opens a window on the screen, which the brief and the owner forbid for explorers.
- **Replay check:** `bazel test //apps/humata2/features/settings:machine_scripts_test --test_filter=edit_rules` (the fake reveal records the path). The starter itself is checked in step 14 with `humata machine script check`.

### 4. Settings › Machines › Reveal scripts
- **Screen:** The block as in step 1.
- **Offered:** Reveal scripts.
- **Action:** Click Reveal scripts (`SettingsAction::RevealScripts`).
- **Result:** `RealReveal::reveal_folder` creates `~/.humata2/scripts` if missing and runs `open` on it, which shows it in Finder. It writes no file: no sample script and no rules file. The spec says Humata makes "the folder and a commented sample file the first time"; in the code only the folder is made here, and the sample file is the rules starter from step 3.
- **Evidence:** Not run: `open` shows a Finder window on screen.
- **Replay check:** `bazel test //apps/humata2/features/settings:machine_scripts_test --test_filter=reveal_scripts` (the fake records the folder). After a real click, `humata machine script list` shows `scripts:` with no names (phase 2 step 16 covers the empty folder).

### 5. Settings › Machines when the rules file cannot be used, or the read fails
- **Screen:** A red alert `settings-scripts-problem` with the problem text (for example the TOML error "fix it and run `humata machine script check`"). A failed read shows a red alert `settings-scripts-error` with the error text. The block keeps the rules last read.
- **Offered:** Edit rules and Reveal scripts, which are the only ways to fix the file.
- **Action:** Open Machines with a bad file (fake: `set_scripts(Ok(problem set, rules empty))`), or a failed read (`set_scripts(Err)`).
- **Result:** Problem text is drawn and rules are empty; the failed read keeps `scripts` (`ScriptsShown(Err)` sets only `scripts_error`).
- **Evidence:** none.
- **Replay check:** `bazel test //apps/humata2/features/settings:machine_scripts_test --test_filter=a_failed_read` (the rules last read are kept).

### 6. Machine page, a machine with no rules that apply
- **Screen:** The page as in `59-machine-page` (overview, sessions, folders). No "Scripts" label, no card, no empty state: `scripts_section` returns `None` when `visible()` is false (no rules for this machine, no problem, no error).
- **Offered:** Nothing about scripts. The page gives no path to the rules (Settings is the only entry).
- **Action:** Open the machine page of a machine that no rule names (`machines = ["other"]`), or with no rules file.
- **Result:** The machine page shows no sign of the Scripts feature.
- **Evidence:** `desktop/tests/__snapshots__/native/59-machine-page.view.txt` has no "Scripts" text (fixture has no rules).
- **Replay check:** `bazel test //apps/humata2/features/hosts:scripts_test --test_filter=a_machine_with_no_rules_shows_no_scripts`.

### 7. Machine page, rules apply: the Scripts section
- **Screen:** Label "Scripts", then one bordered card. Each rule is a 36px row: the name (bold, 190px), its triggers dim, its last outcome in colour (green Passed, red Failed or Timed out, orange Running, dim Skipped) with `"<Outcome> · <clock time>"` or "Never run", and a quiet button "Run now" (or "Running…"). The row is clickable as a whole.
- **Offered:** Run now (`machine-script-run`, disabled while the rule runs or the machine is offline), and a click on the row, which opens the rule's steps.
- **Action:** Open the machine page of a machine that a rule applies to.
- **Result:** `load_scripts` reads `scripts(Some(machine))`; rows come from `ScriptsState::rows()` in the rules' file order. A machine that is offline shows Run now disabled, with no line saying why.
- **Evidence:** none; rendered from view code only.
- **Replay check:** `bazel test //apps/humata2/features/hosts:scripts_test --test_filter=showing_the_page_lists_the_rules_with_their_last_run`.

### 8. Machine page, a rule's steps open
- **Screen:** Under the row, each step as a grey chip with its kind (`run`, `humata`, `start`, `send`) and its label. Below them, the last run's steps: `Step n: <label> · <outcome>`, then each step's last output lines in mono (or "No output.").
- **Offered:** Click the row again to close it.
- **Action:** Click the rule row (`MachinePageAction::ToggleScript`).
- **Result:** `open` is set to the rule's name, and cleared on a second click. Only one rule is open at a time.
- **Evidence:** none.
- **Replay check:** `bazel test //apps/humata2/features/hosts:scripts_test --test_filter=opening_a_rule_toggles_its_steps`.

### 9. Machine page, Run now runs the rule and the row follows the host
- **Screen:** The button reads "Running…" and is disabled; the row's outcome reads "Running…" in orange.
- **Offered:** Nothing else on the row while it runs.
- **Action:** Click Run now on an online machine with no migration pending (`RunScript`).
- **Result:** `run_script(machine, rule, None)` is sent. The run is an Operation of kind `MACHINE_SCRIPT` (`begin_run`, `rpc_run_machine_script`). When it returns, the page reads the host's runs again (`ScriptRan` → `load_scripts`). The row then shows the run's outcome and time. A second click while it runs sends nothing (`running` set).
- **Evidence:** none for the GPUI row. The CLI twin is step 17.
- **Replay check:** `bazel test //apps/humata2/features/hosts:scripts_test --test_filter=run_now_is_running_until_the_run_ends` and `--test_filter=run_now_while_the_rule_runs_sends_nothing_more`.

### 10. Machine page, Run now refused
- **Screen:** A red line under the row with the refusal text (`machine-script-refused`), or, for a machine waiting for its Migrate item, the page-level refusal `Refused::MigrationNeeded` (not on the row).
- **Offered:** Run now again. The refusal line goes away on the next Run now, or once a later run passes or fails.
- **Action:** Run now on a rule the host refuses (for example an unknown rule or a machine it does not apply to), or Run now while the machine has a pending Migrate.
- **Result:** A host refusal is kept on the row (`scripts.refused`). The Migrate case runs nothing, and the refusal is set on the page, not beside the rule, so the row gives no reason.
- **Evidence:** none.
- **Replay check:** `bazel test //apps/humata2/features/hosts:scripts_test --test_filter=a_run_the_host_refuses_says_why_on_its_row` and `--test_filter=run_now_of_a_rule_the_machine_does_not_have_sends_nothing`.

### 11. Machine page, a failed run shows the failed step and the end of its output
- **Screen:** The row's outcome in red, "Failed · <time>" (or "Timed out · <time>"). Under the row: red `Step n: <label> (exit <code>)`, then the failed step's last output lines in mono. Clicking the row also lists every step with its outcome.
- **Offered:** Run now (enabled again), row toggle.
- **Action:** Fail a rule, then open the machine page (the last run is from the host's log).
- **Result:** `failure_tail` takes the first step whose outcome is a problem and shows its tail. A step with no exit shows no "(exit …)".
- **Evidence:** none in phase 1.
- **Replay check:** `bazel test //apps/humata2/features/hosts:scripts_test --test_filter=a_failed_run_keeps_the_end_of_the_failed_steps_output`.

### 12. Inbox, "Script <rule> failed on <machine>"
- **Screen:** A VIP request (host ask) titled `Script <rule> failed on <machine>`. Its reason says: "The rule <rule> ran on <machine> because of <trigger> and stopped at step n (<kind> <label>) (exit n): <why>. Approve this to run the rule again, or deny it to leave things as they are. The next pass of the rule clears this request." The preview is the failed command (`<kind> <label>`) and the output tail as its notes (`daemon/humatad/src/machine_scripts/live.rs` `failure_card`).
- **Offered:** "Allow" and "Deny". Host asks are not questions and have no copy card, so `ui/src/request_card.rs` draws the Allow/Deny branch, not "Approve" (which the reason text uses), and no Retry or Dismiss.
- **Action:** Not run (the Inbox only gets this item on a failure). Phase 2 covers it headless through the daemon's unit test.
- **Result (code):** `failed` posts the card under the key `machine-script/<machine>/<rule>`, so one item per pair; a second failure replaces the first in place (same request id, revision bumped). Allow runs `run_again` with the same trigger and the session id kept (channel, folder, from, to left empty). Deny drops the call; the request is denied and its call never runs. A later pass withdraws the item with reason Resolved (`passed`).
- **Evidence:** none yet.
- **Replay check:** `bazel test //apps/humata2/daemon/humatad:<lib unit test target, see PENDING RUN> --test_filter=a_failed_run_posts_one_request_a_second_failure_replaces_and_a_pass_withdraws`.

### 13. Inbox, Allow reruns the rule; Deny leaves it; a pass clears it
- **Screen:** The request stays until answered or withdrawn. An approved call runs in its own task while the request is marked running (`inbox.rs` `held.running`); the request's final state after the rerun is a phase-2 check.
- **Offered:** Allow, Deny.
- **Action:** Allow on a rule that now fails again, then on one that now passes.
- **Result (code):** Allow on a still-failing rule replaces the request with a new failure (`LiveReport::failed`). Allow on an offline machine or a stopped engine returns FailedPrecondition with the engine's text (see Surprises). A passing run withdraws the item.
- **Evidence:** none.
- **Replay check:** `bazel test //apps/humata2/daemon/humatad:<lib unit test target> --test_filter=a_failed_run_posts_one_request` (covers the replace and the withdraw).

### 14. CLI: `humata machine script check` on the starter file
- **Screen:** Terminal output.
- **Offered:** Flags only: `--json`.
- **Action:** `humata machine script check` with the commented starter as `~/.humata2/config/machine-scripts.toml` and an empty scripts folder.
- **Result (code):** `check` returns `valid: true`, no problems, no warnings; exit 0. The reply goes through `Done::reply(as_json::typed(...))`; whether text mode prints that JSON without `--json` is a phase-2 check.
- **Evidence:** PENDING RUN transcript at `$S/verify/evidence/X-C/14-check-sample.txt`.
- **Replay check:** `bazel test //apps/humata2/cli:machine_scripts_test --test_filter=check_of_a_valid_file_exits_ok_and_carries_the_warnings`; real run: `humata machine script check; echo $?` prints `0`.

### 15. CLI: `humata machine script check` on a bad rules file
- **Screen:** Terminal output: each problem as text, then a fix line.
- **Offered:** `--json`.
- **Action:** Write a rule with `on = "nope"`, a `run` script that is missing, and a `send` with `keys = "maybe"`, then run `check`.
- **Result (code):** Exit 2 (`USAGE`), `kind: invalid_field`, `details.problems` lists every problem (`check_of_an_invalid_file_exits_usage_and_lists_every_problem`). The unknown-trigger text says "use added, connected, session-arrived or session-leaving". That list omits `upgraded`, which is valid (`UnknownTrigger` and `NoTrigger` in `daemon/machines/src/machine_scripts.rs`). The missing-script text points to Settings › Machines › Scripts › Reveal scripts.
- **Evidence:** PENDING RUN at `$S/verify/evidence/X-C/15-check-bad.txt`.
- **Replay check:** `bazel test //apps/humata2/cli:machine_scripts_test --test_filter=check_of_an_invalid_file_exits_usage`; real run: `humata --json machine script check; echo $?` prints `2`.

### 16. CLI: `humata machine script list`
- **Screen:** Terminal output: a table RULE, ON, MACHINES, ONCE, STEPS; then, when a run exists, a table RULE, MACHINE, LAST RUN, STARTED, TOOK; then `scripts: <names>`.
- **Offered:** optional machine name; `--json`.
- **Action:** With one rule (`on = ["added","connected"]`), an empty `scripts` folder and no runs, then after one run (step 17).
- **Result (code):** A missing rules file lists nothing and no problem. A problem is printed with its fix line after the table. `--json` returns `rules`, `last_runs`, `scripts`, `problem`.
- **Evidence:** PENDING RUN at `$S/verify/evidence/X-C/16-list-*.txt`.
- **Replay check:** `bazel test //apps/humata2/cli:machine_scripts_test --test_filter=list_in_json_carries` and `--test_filter=list_without_a_machine`.

### 17. CLI: `humata machine script run <rule> <machine> [--session <s>]`
- **Screen:** Terminal output of the Operation (the follow output).
- **Offered:** `--session`, `--json`, and the global `--wait` (see Surprises).
- **Action:** Run a rule that exists and a rule that does not, against the host or a machine.
- **Result (code):** A rule that doesn't exist is NOT_FOUND ("there is no rule named …"). A machine name that does not resolve is NOT_FOUND and nothing is sent. A refused caller is DENIED. In a terminal the command waits to the end (`follow`: interactive means wait forever unless `--wait`); when piped it returns the started Operation at once. The run's id is the Operation's.
- **Evidence:** PENDING RUN at `$S/verify/evidence/X-C/17-run-*.txt`.
- **Replay check:** `bazel test //apps/humata2/cli:machine_scripts_test --test_filter=run_sends_the_rule_machine_and_session_with_a_key` and `--test_filter=run_on_an_unknown_machine_is_not_found`.

### 18. CLI: `humata machine script log <machine> [--rule] [--limit]`
- **Screen:** Terminal output: a table RUN, RULE, TRIGGER, STARTED, OUTCOME, TOOK; under it, each run's steps that did not pass: `<run> step n (<kind> <label>): <outcome>, exit n: <why>` and the tail lines indented.
- **Offered:** `--rule`, `--limit` (default: the server's, sent as 0), `--json`.
- **Action:** `log` after step 17, with a failing run and a passing one.
- **Result (code):** Newest first, as the help says. Passing steps are not printed under the table.
- **Evidence:** PENDING RUN at `$S/verify/evidence/X-C/18-log.txt`.
- **Replay check:** `bazel test //apps/humata2/cli:machine_scripts_test --test_filter=log_sends_the_machine_rule_and_limit` and `--test_filter=log_in_json_carries_each_run_and_its_steps`.

## Not reachable headless
- **Settings › Machines › Scripts visuals (steps 2, 5):** no harness opens the Machines tab with rules. The native test `hosts_settings.rs` visits Settings on its default section, and `desktop/tests/native/machines.rs` never sets scripts. Only the reducer is covered (features/settings tests). Needs a test that selects `Section::Machines` and calls `set_scripts`; that is a repo change, so the coach decides.
- **Machine page Scripts section visuals (steps 7 to 11):** the fixture has no scripts, so no frame shows the section. The reducer and the row view model are covered by `features/hosts/tests/scripts.rs`. Same needed change as above.
- **Edit rules and Reveal scripts (steps 3, 4):** both open an editor or Finder on screen. Explorers may not do that. Their file effects are checked in phase 2 by the CLI and filesystem only where they don't open a window.
- **Inbox card visuals (step 12):** no native frame of a host ask with the `machine.script` operation. The card's text and buttons are from the code above.
- **Triggers on a real machine (`added`, `connected`, `session-arrived`, `session-leaving`):** the scripted runs need a remote machine over SSH. The writer flows `flows/cases/real/10-a-script-runs-when-a-machine-is-registered.md` and `11-scripts-on-reconnect-and-session-arrival.md` cover them on the dedicated VM (`h2-terminal-c1`, see AGENTS.md). Not in this explorer's phase 1 or the headless run.

## Surprises
- **Inbox buttons:** the failure request offers "Allow" and "Deny" (the host-ask branch of `ui/src/request_card.rs`), but its reason text says "Approve this to run the rule again, or deny it". The words disagree, and neither is Retry or Dismiss as the milestone doc says. Deny does not remove the request; it denies it.
- **Retry drops the session context:** Allow on a failed session-trigger run keeps only the session id. The channel, folder, from and to variables are blank in the rerun (`run_again` in `live.rs`); a manual `run --session` does the same (`begin_run` in `script_rpc.rs`, with `to` set to the machine). A `session-arrived` rule rerun from the Inbox may not see the variables it saw the first time.
- **Trigger error text is out of date:** `UnknownTrigger` and `NoTrigger` list four triggers and leave out `upgraded`, which the parser accepts and the sample file documents.
- **Reveal scripts makes only the folder:** the milestone says it makes "a commented sample file the first time". The sample is the rules starter, written by Edit rules only; Reveal scripts writes nothing.
- **Settings rule rows are thin:** the row shows name, triggers and a step count. The right side of each row is empty, so there is no way to see a rule's machines, once flag, or steps from Settings.
- **Settings does not reload after Edit rules:** the list reads only when the Machines tab is opened again, and Edit rules gives no confirmation when the editor closes.
- **Machine page has no door to rules:** with no applicable rule the Scripts section is absent, so nothing on the page says scripts exist or where the file is.
- **Run now offline has no reason:** the button is disabled when the machine is offline, with no line saying why.
- **Migration refusal is page-wide:** Run now on a machine with a pending Migrate sets the page-level refusal, not the row's refusal line, so the rule row says nothing.
- **Run time is a clock time:** outcome lines show "<outcome> · <time>" from the local clock formatter (`clients/live/src/local_time.rs`), with no date for older runs unless the formatter adds a day word.
- **CLI run help:** the help text says "`--wait` follows it to the end", but `--wait` is a global flag, not an option of `run`. In a terminal `run` waits to the end anyway; piped, it returns at once.
- **Check output mode:** `check` replies through `Done::reply(as_json::typed(...))`, so whether text mode prints JSON is for phase 2 to confirm (step 14).

## PENDING RUN (phase 2, one explorer, coach's go)

Scratchpad: `S=<scratchpad>`. Evidence goes to `$S/verify/evidence/X-C/`. From the repo root, `export PATH=$PWD/.tools/bin:$PATH` first.

1. `bazel build --jobs=4 //apps/humata2/cli:humata //apps/humata2/daemon/humatad:humatad2 //apps/humata2/daemon/holder:humata2_holder`
2. `bazel test --jobs=4 //apps/humata2/features/settings:machine_scripts_test`
3. `bazel test --jobs=4 //apps/humata2/features/hosts:scripts_test`
4. `bazel test --jobs=4 //apps/humata2/features/app:machine_script_events_test`
5. `bazel test --jobs=4 //apps/humata2/cli:machine_scripts_test`
6. `bazel query 'tests(//apps/humata2/daemon/humatad:all)'` (find the lib unit-test target that holds `machine_scripts::live` and `machine_service::script_rpc` tests), then `bazel test --jobs=4 <that target> --test_filter=a_failed_run_posts_one_request` and `bazel test --jobs=4 <that target> --test_filter=script_rpc`
7. Throwaway HOME: `H=$S/home-c; rm -rf $H; mkdir -p $H/.humata2/config $H/.humata2/scripts; cp apps/humata2/daemon/machines/sample/machine-scripts.toml $H/.humata2/config/machine-scripts.toml`
8. Check the starter (step 14): `HUMATA_SOCKET=$S/sock HOME=$H` plus the daemon from step 9 running, then `$PWD/bazel-bin/apps/humata2/cli/humata machine script check; echo $?`
9. Start the daemon: `HOME=$H $PWD/bazel-bin/apps/humata2/daemon/humatad/humatad2 serve --socket $S/sock --state $S/state --root $S/root --holder $PWD/bazel-bin/apps/humata2/daemon/holder/humata2_holder --shell /bin/sh --shell-arg=-i &` (keep its PID; it is stopped at the end)
10. Write a real rule and script into $H (step 16 and 17): `$H/.humata2/scripts/hello.sh` (`echo hello; exit 0`) and a rule `[[rule]] name="hello" on=["added"] [[rule.step]] run="hello.sh"`; then `humata machine script list` (step 16), and `humata machine script list --json` to compare.
11. Bad file (step 15): copy the rule with `on = "nope"`, a missing script name, and `keys = "maybe"` into $H, then `humata --json machine script check; echo $?` (expect 2).
12. Run now (step 17): `humata machine script run hello <host-name>`, with the host name from `humata hosts list` or `humata --help` (the machine name may be "This Mac"; record what the CLI says). Then `humata machine script log <host-name>` (step 18).
13. Step 12 and 13 (Inbox) are headless via the unit test in step 6; `humata inbox --help` to find how to list the Inbox if a real failure is made.
14. Stop the daemon started in step 9 (`kill <pid>`) and remove nothing outside `$S`.

Note: Settings and machine-page screens (steps 1 to 11) are not run. The reducer tests in steps 2 to 4 are the headless proof for them.

## Phase 1 totals
- Steps: 18 (14 with code-only expectations, 4 with a replay check only).
- PENDING RUN commands: 14 numbered lines above; each maps to steps 6, 12 to 18 and the replay checks.

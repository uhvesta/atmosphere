# A rule on added, connected or upgraded runs its steps once per link-up, and a failed step stops its rule

Layer: macro (daemon with fake machines, the CLI against a throwaway `humatad2`). Platforms: macOS. The fake machine stands in for a Linux arm64 machine.

## Starting state

- A throwaway `HOME` holds `~/.humata2/state`, `config/` and `scripts/`. No owner config is read. Channel `setup-lab` exists. No window is open on the owner's screen.
- No machine is added yet: `box-1` is a fake from the test rig, and This Mac is the host. `~/.humata2/config/machine-scripts.toml` holds these rules, in order:
  - `hello`: `on = ["added", "connected"]`, `once = false`, one step `run = "hello.sh"`.
  - `restart-copilot`: `on = "added"`, `once = true`, steps `start = { agent = "copilot", channel = "setup-lab", title = "Setup" }` then `send = "/restart"`.
  - `after-upgrade`: `on = "upgraded"`, one step `run = "hello.sh"`.
  - `broken`: `on = "connected"`, steps `run = "fail.sh"`, then `run = "never.sh"`. This is the rule that proves the first failure stops it.
  - `tidy`: `on = "connected"`, one step `run = "hello.sh"`, after `broken`, to show the next rule still runs.
- `~/.humata2/scripts/` holds `hello.sh` (writes `registered $HUMATA_MACHINE $HUMATA_TRIGGER` to a file under the throwaway home and exits 0), `fail.sh` (prints a line and exits 3), and `never.sh` (writes a marker file, so a run that wrongly reaches it is visible).

## Reproduce headless

- Rules and checks: `bazel test //apps/humata2/daemon/machines:machine_scripts_test //apps/humata2/daemon/machines:upgraded_trigger_test`. The CLI on a fake daemon: `bazel test //apps/humata2/cli:machine_scripts_test`.
- The run engine on fakes, Inbox card and the link-up choice: `bazel test //apps/humata2/daemon/humatad:humatad2_unit_test`, narrowed with `--test_arg=machine_scripts` for `machine_scripts::engine` and `machine_scripts::live`.
- Against a throwaway `humatad2`, for the file and the CLI only: `export HOME=<throwaway>`, then `humatad2 serve` (its state defaults to `$HOME/.humata2/state`, so the rules file it reads is `$HOME/.humata2/config/machine-scripts.toml`). Then run `humata machine script check; echo $?` and `humata machine script list`. Capture the output and the exit code as evidence. A valid file exits 0 and an invalid one exits 2.
- Not headless here: adding a real machine (the Lima VM disk fails Add with DiskLow, D694), the real Copilot login, and the machine page and Inbox card. The fake-ssh rig in `daemon/humatad/tests/machines.rs` is the nearest; no test there fires a trigger yet.

## Steps

1. **I do:** Write the scripts and the rules file above under the throwaway home. No secret goes in either file.  
   **I see:** Two files exist. Nothing has run.  
   **State and events:** No RPC writes rules or scripts. The rules name each script by file name only (`run = "hello.sh"`), never by a path.  
   **Status:** Fakes only. `daemon/machines/tests/machine_scripts.rs::a_valid_file_parses_to_its_rules_and_steps` parses a file of this shape. `daemon/machines/tests/upgraded_trigger.rs::an_upgraded_rule_applies_only_after_an_upgrade` covers `upgraded`.

2. **I do:** Run `humata machine script check`, then `humata machine script list`.  
   **I see:** Check prints valid and exits 0. List shows five rules, their triggers, their steps, and the script names. No machine has a last run yet.  
   **State and events:** Check reads the file and the script names only. It writes nothing.  
   **Status:** Fakes only. `cli/tests/machine_scripts.rs::check_of_a_valid_file_exits_ok_and_carries_the_warnings` and `machine_service/script_rpc.rs::tests::check_says_valid_for_a_good_file_and_lists_problems_for_a_bad_one` cover the reply shape. A run of the real daemon is the headless step above, and it is not yet recorded.

3. **I do:** Add `box-1` with Add machine. The first link-up after the add is the one that counts.  
   **I see:** The machine comes online. Its page would show `hello` and `restart-copilot` as Passed, once the steps have run.  
   **State and events:** `Work::note_added` marks `box-1` in the add (`machine_service.rs` around line 906), then `link_up` runs. `fire_link_up` (`machine_service.rs` around line 583) takes the mark once and fires `added`. Every later link-up fires `connected`. `hello` (`added` and `connected`) runs once at `added`. `restart-copilot` runs at `added`. `after-upgrade` does not run.  
   **Status:** Code, no test. The choice of `added` over `connected` is tested as a unit: `machine_service/scripts.rs::tests::only_the_first_link_up_after_an_add_is_the_added_one`. The link-up wiring that calls it has no test.

4. **I do:** Read `hello`'s output on the fake machine.  
   **I see:** The file says `registered box-1 added`.  
   **State and events:** The bytes are streamed to `bash -l -s --` with `HUMATA_TRIGGER=added` and `HUMATA_MACHINE=box-1`. No session variable is set, and host variables are not passed.  
   **Status:** Fakes only. `machine_scripts/engine.rs::tests::a_machine_trigger_has_no_session_variables` checks the variables. `daemon/machines/tests/machine_scripts.rs::a_bash_script_goes_to_the_machine_on_stdin` checks the streaming.

5. **I do:** Wait for `restart-copilot`. It starts copilot in `setup-lab` and then types `/restart`.  
   **I see:** One copilot session appears in `setup-lab` on `box-1`. `/restart` is typed once, after the session is idle, and Enter is pressed. If the copilot trust prompt is still showing, the wait holds until it is answered.  
   **State and events:** The `start` step runs first and keeps the session id. The `send` step waits for the session to be Idle (`wait = "idle"` by default), then types `/restart` with Enter as its own keyed request. The key is `<run id>/<step index>`, so this run never types it twice.  
   **Status:** Fakes only for the order: `machine_scripts/engine.rs::tests::send_types_into_the_session_a_start_step_made` and `the_wait_for_idle_comes_before_a_send_that_asks_for_it`. The live glue (`start_for_script`, `wait_idle`, `type_for_script` in `machine_scripts/live.rs`, `session_service/script_steps.rs`, `terminal.rs`) is Code, no test. See the wait-for-idle gap below.

6. **I do:** Flap the link: cut it twice in quick succession while `hello` is still running (make `hello.sh` sleep for a few seconds in this run).  
   **I see:** `hello` finishes, then runs once more. The two runs do not overlap, and the log has two `hello` rows in order.  
   **State and events:** A link-up while a `connected` run is queued folds into it. One running is queued behind it. One run executes per machine at a time. `peer_client.rs::supervise` reports one `Up` per connect.  
   **Status:** Fakes only. `machine_scripts/engine.rs::tests::fires_for_one_machine_never_overlap_and_waiting_connects_fold_into_one` checks the fold and the single running step. The one `Up` per connect is Code, no test.

7. **I do:** Set `hello` to `once = true` and fire `connected` twice more.  
   **I see:** The second and third link-ups log nothing for `hello`. `humata machine script run hello box-1` runs it again on purpose.  
   **State and events:** A passed `once` rule is recorded per machine and rule in `once.json`. A failed `once` rule is not recorded, so it runs at the next trigger. `run` ignores `once`.  
   **Status:** Fakes only. `machine_scripts/engine.rs::tests::once_runs_once_and_a_failure_is_retried` and `run_now_ignores_once_and_records_the_pass`.

8. **I do:** Let `broken` run on the next link-up. Its first step is `fail.sh`.  
   **I see:** `broken` shows Failed. Its first step has exit 3 and the output line. `never.sh` shows Skipped, and its marker file is absent. `tidy` still runs and passes.  
   **State and events:** The first step that doesn't pass stops its rule. The next rule in file order still runs. Both runs go to the log.  
   **Status:** Fakes only. `machine_scripts/engine.rs::tests::the_first_failed_step_stops_its_rule_and_the_next_rule_runs`.

9. **I do:** Open the Inbox after step 8.  
   **I see:** One request, "Script broken failed on box-1", with the failed step, its exit, the tail and the trigger. The buttons are Approve and Deny.  
   **State and events:** A VIP host ask keyed by machine and rule (`machine-script/box-1/broken`).  
   **Status:** Likely bug. The request, its key and its single item are covered by `machine_scripts/live.rs::tests::a_failed_run_posts_one_request_a_second_failure_replaces_and_a_pass_withdraws` (Fakes only). But the card offers Approve and Deny only. `inbox/host_asks.rs` line 124 says a host ask "takes Approve or Deny", and the spec (D686) asks for Retry and Dismiss.

10. **I do:** Make `fail.sh` pass, then fire `connected` again.
    **I see:** `broken` shows Passed. Its request leaves the Inbox as withdrawn. A second failure while the request is still pending would replace it, not add a second one.
    **State and events:** `finish` on a pass calls `withdraw_host_ask` for that machine and rule. A failure while pending takes the card in place.
    **Status:** Fakes only. `machine_scripts/engine.rs::tests::a_failure_is_reported_and_a_later_pass_withdraws_it` and the live test in step 9.

11. **I do:** Run `humata machine script log box-1 --limit 10`.
    **I see:** One row per run, newest first: rule, trigger (`connected` or `added`), started time, outcome, took. The failed rows print the failing step, its exit and the tail below. Passed runs print no detail.
    **State and events:** The log keeps the last 50 runs per machine in `machine-scripts/box-1.jsonl` under the daemon's state folder. The CLI prints one table and the detail of each failed step.
    **Status:** Fakes only. `machine_service/script_rpc.rs::tests::a_run_is_in_the_log_with_its_steps` and `cli/tests/machine_scripts.rs::log_in_json_carries_each_run_and_its_steps`.

12. **I do:** Migrate `box-1` (the Migrate item in the Inbox, approved). When the Migrate finishes, the machine is upgraded.
    **I see:** `after-upgrade` runs once on `box-1`. Migrating This Mac runs nothing, because This Mac has no triggers.
    **State and events:** A finished Migrate of a machine other than the host calls `on_migrated`, which calls `fire_upgraded` (`upgrade/items.rs` around line 539, wired in `serve.rs` around line 992). The Desk folder migrate does not count. `upgraded` is queued and folded like `connected`.
    **Status:** Code, no test. Parsing is Fakes only (`upgraded_trigger.rs`). D689 still reads "Open", but the code wires it (re-check).

13. **I do:** Press Approve on `broken`'s request, which is Retry in the spec.
    **I see:** `broken` runs again on `box-1`. If the rule passes, the request is withdrawn. A second failure posts a new request.
    **State and events:** Approve calls `run_now` for the rule with the failed run's trigger and session id. The other session fields are left empty. The engine gives the new run a new id.
    **Status:** Likely bug. Each execution gets a new run id (`machine_scripts/engine.rs` around line 308), and every keyed step uses `<run id>/<index>` (line 378). So a retry does not reuse the earlier keys. A rule that failed after its `start` or `send` step will start a second session or type `/restart` a second time. See the gap below.

## Branches

### A step runs past its time limit

- **When:** `slow.sh` is set to run longer than the rule's `timeout = "10s"`.
- **I do:** Fire `connected` and wait.
- **I see:** The run shows Timed out, later steps show Skipped, and the Inbox gets one request.
- **Expected outcome:** The step is killed and no `sleep` is left on the machine. The step limit is the smaller of five minutes and the rule's timeout (D684).
- **Status:** Fakes only for the outcome: `machine_scripts/engine.rs::tests::a_step_past_its_limit_is_timed_out`. The process-group kill is Code, no test (`daemon/machines/src/system.rs` ~166 to 188).

### The link drops in the middle of a run

- **When:** I cut the link while `hello` is running.
- **I do:** Cut the link, then let it come back.
- **I see:** The run shows Failed with the step's error "couldn't run on box-1: ... Check that the machine is connected, then run the rule again". When the link returns, `connected` fires once more and runs the rule from its first step.
- **Expected outcome:** The interrupted run is not counted as passed. It is not resumed; the next trigger runs it from the start.
- **Status:** Code, no test. The error mapping is `StepError::Shell` in `machine_scripts/mod.rs` and `live.rs`.

### The daemon restarts in the middle of a run

- **When:** I stop and start `humatad2` while `hello` is running.
- **I do:** Restart the daemon.
- **I see:** The run is not in the log, because a run is written only when it ends. When the daemon links to `box-1` again, `connected` fires and the rule runs once.
- **Expected outcome:** No run is recorded for the interrupted attempt. The machine is not left with a second copy running, and a `once` rule that did not pass runs again.
- **Status:** Code, no test. The engine keeps runs in memory until `finish` (`machine_scripts/engine.rs`).

### The start step names a kind that is not there

- **When:** `restart-copilot` names an agent kind that is not enabled on the machine.
- **I do:** Add `box-1`.
- **I see:** The run shows Failed at step 1 with the start error, which names the kind and the fix. Step 2 shows Skipped. Nothing is typed.
- **Expected outcome:** No half-started session is left, and no text goes into another session.
- **Status:** Code, no test. The error is `StepError::Start` (`machine_scripts/mod.rs`).

### The rules file cannot be read

- **When:** The file has a bad TOML line.
- **I do:** Fire `connected`, then run `humata machine script check`.
- **I see:** Check exits 2 and lists the problem. The link-up runs nothing, and the daemon logs one line.
- **Expected outcome:** A broken file never blocks the link, and nothing runs from it.
- **Status:** Fakes only. `machine_scripts/engine.rs::tests::a_rules_file_that_does_not_parse_runs_nothing`; the exit 2 is `cli/tests/machine_scripts.rs::check_of_an_invalid_file_exits_usage_and_lists_every_problem`.

### A send step with no session

- **When:** A `connected` rule has `send = "/restart"` but no `start` before it.
- **I do:** Fire `connected`.
- **I see:** The run shows Failed with "there is no session to type into". Nothing is typed.
- **Expected outcome:** A `send` never types into a session it cannot name.
- **Status:** Fakes only. `machine_scripts/engine.rs::tests::send_with_no_session_fails`.

### An agent asks to run a rule now

- **When:** An agent session in a channel calls `humata machine script run`.
- **I do:** From an agent session, run the command.
- **I see:** Refused with the access reason, unless the channel's `machine_script_run` rule is Allow. Ask puts the run in the Inbox.
- **Expected outcome:** No script runs from an agent by default. Triggered runs need no check.
- **Status:** Fakes only. `daemon/access/tests/machine_script_run.rs::a_new_channel_never_lets_an_agent_run_a_machine_script` and `machine_service/script_rpc.rs::tests::an_agent_without_the_switch_is_refused_and_the_user_never_is`.

### A session wait on a stopped or offline agent

- **When:** A `restart-copilot`-style `send` waits for an agent that has stopped, or whose machine is out of reach.
- **I do:** Stop the agent, then fire the rule.
- **I see:** The wait returns at once if the session shows Idle, and the text is typed into a stopped session. If the machine is offline, the wait runs for two minutes and then says "still busy".
- **Expected outcome:** A stopped or offline session is a failure with a plain reason, and nothing is typed.
- **Status:** Likely bug. `machine_scripts/live.rs` `wait_idle` (around lines 210 and 215) returns Ok on Idle or ProbablyIdle without checking the agent runs (B6). Offline is not in the list of statuses that fail, so it waits out the two minutes.
## Expected outcomes

- [ ] `check` exits 0 for the file above and 2 for a file with a bad step; a bad file lists every problem at once.
- [ ] `added` rules run once, at the first link-up after the add; `connected` rules run once per link-up. A link-up while a run is going queues one more run, and two runs never overlap on one machine.
- [ ] A script sees `HUMATA_TRIGGER` and `HUMATA_MACHINE`, and no session variable on a machine trigger.
- [ ] `once = true` runs a rule once per machine until it passes, and `humata machine script run` runs it anyway.
- [ ] A failing step stops its rule, the rest of that rule shows Skipped, and the next rule still runs.
- [ ] A failed run gives one Inbox request per machine and rule, and a later pass withdraws it.
- [ ] `upgraded` runs after a Migrate of a machine, and never for This Mac.
- [ ] A retry of a failed rule does not repeat a `start` or a `send` that already happened.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Write the rules and scripts | Fakes only | `machines/tests/machine_scripts.rs::a_valid_file_parses_to_its_rules_and_steps`; `machines/tests/upgraded_trigger.rs` |
| 2. `check` and `list` | Fakes only | `cli/tests/machine_scripts.rs::check_of_a_valid_file_exits_ok_and_carries_the_warnings`; `script_rpc.rs` check test |
| 3. `added` fires at the first link-up | Code, no test | `machine_service.rs` ~906 and ~583; `machine_service/scripts.rs` unit test on the choice |
| 4. Script sees its variables, streamed to bash | Fakes only | `engine.rs::a_machine_trigger_has_no_session_variables`; `machines/tests/machine_scripts.rs::a_bash_script_goes_to_the_machine_on_stdin` |
| 5. Start copilot, wait idle, send `/restart` | Fakes only (order); live glue Code, no test | `engine.rs::send_types_into_the_session_a_start_step_made`, `the_wait_for_idle_comes_before...` |
| 6. Flapping link runs `connected` once per link-up | Fakes only | `engine.rs::fires_for_one_machine_never_overlap_and_waiting_connects_fold_into_one` |
| 7. `once = true` | Fakes only | `engine.rs::once_runs_once_and_a_failure_is_retried`; `run_now_ignores_once...` |
| 8. Failing step stops its rule; next rule runs | Fakes only | `engine.rs::the_first_failed_step_stops_its_rule_and_the_next_rule_runs` |
| 9. One Inbox item with Retry | Likely bug (card offers Approve and Deny) | `live.rs::a_failed_run_posts_one_request...`; `inbox/host_asks.rs` line 124 |
| 10. A later pass withdraws it | Fakes only | `engine.rs::a_failure_is_reported_and_a_later_pass_withdraws_it` |
| 11. Log shows runs and failed detail | Fakes only | `script_rpc.rs::a_run_is_in_the_log_with_its_steps`; `cli/tests/machine_scripts.rs::log_in_json...` |
| 12. `upgraded` after a Migrate | Code, no test (trigger wiring) | `upgrade/items.rs` ~539; `serve.rs` ~992; parse Fakes only |
| 13. Retry from the Inbox | Likely bug (keys change per run, so a retry repeats steps) | `engine.rs` ~308 and ~378 |
| Branch: step times out | Fakes only (outcome); kill Code, no test | `engine.rs::a_step_past_its_limit_is_timed_out` |
| Branch: link drops mid-run | Code, no test | `machine_scripts/mod.rs` `StepError::Shell` |
| Branch: daemon restarts mid-run | Code, no test | `engine.rs` in-memory until `finish` |
| Branch: start names a missing kind | Code, no test | `StepError::Start` |
| Branch: rules file cannot be read | Fakes only | `engine.rs::a_rules_file_that_does_not_parse_runs_nothing` |
| Branch: send with no session | Fakes only | `engine.rs::send_with_no_session_fails` |
| Branch: agent runs a rule now | Fakes only | `daemon/access/tests/machine_script_run.rs`; `script_rpc.rs` agent test |
| Branch: wait on stopped or offline agent | Likely bug | `live.rs` `wait_idle` ~210 |
## Gaps and likely bugs

- **Not built:** the Retry and Dismiss labels on the Inbox card (D686). The card has Approve and Deny only.
- **Likely bug: a retry repeats earlier steps.** Keys are `<run id>/<index>` and a new run id is made per execution (`machine_scripts/engine.rs` ~308, ~378). A Retry, or the next trigger after a failure, re-runs `start` (a new session) and `send` (a second `/restart`). Confirm by running a rule with `start`, `send` and a failing last step, then Retry.
- **Likely bug: `start` on `connected` makes a new session on every link-up.** Nothing reuses an existing session in the channel. Confirm by firing `connected` twice with a `start` step; two sessions appear.
- **Likely bug: Retry drops the session context.** `machine_scripts/live.rs` `run_again` (around line 314) keeps only the session id, so `HUMATA_CHANNEL`, `HUMATA_SESSION_DIR`, `HUMATA_FROM` and `HUMATA_TO` are empty and left out of a retried `session-*` run. A manual run with `--session` sets `HUMATA_TO` to the machine's own name (`machine_service/script_rpc.rs` `begin_run`).
- **Likely bug: wait for idle trusts Idle.** `live.rs` `wait_idle` (~210) returns on Idle without checking that the agent runs (B6). Offline is not a failure, so the wait runs out after two minutes with "still busy".
- **Likely bug (spec): the Inbox card has Approve and Deny, not Retry and Dismiss** (`inbox/host_asks.rs` line 124; `live.rs` `failure_card`).
- **Likely bug (spec): `list`, `check` and `log` answer agents with nothing or a refusal, though the spec says reads follow `machine_read`** (`script_rpc.rs` `sees()`, line 50). D689 still reads "Open" although `upgraded` is wired (re-check).

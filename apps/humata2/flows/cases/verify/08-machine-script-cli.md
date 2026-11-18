# `humata machine script` checks the rules, runs a rule and reads its log, and an agent can't run one by default

Layer: verify (scripts-ui, the CLI half). Platforms: macOS (the daemon and the `humata` CLI). The machine page is in case D-1.

## Starting state

- A throwaway `humatad2` is running. It has its own `HOME`, its own `--state` folder (under that `HOME`, so the rules home is the throwaway `.humata2`), its own `--root` and its own socket. Nothing it does touches the owner's `~/.humata2`.
- `HUMATA_SOCKET` points at that throwaway socket for every `humata` call in this case. `HOME` is the throwaway one too.
- The machine list has one machine, this Mac, named `this-mac` below. It is the only machine.
- The rules file at `<throwaway home>/.humata2/config/machine-scripts.toml` holds one valid rule, `setup`:
  - `on = ["added", "connected"]`, `once = false`, `timeout = 300`.
  - One step, `run = "setup-tools.sh"`.
- The scripts folder beside it holds `setup-tools.sh`, which prints a line and exits 0.
- No run has happened yet. The run log for `this-mac` is empty.
- The tester holds a terminal with the throwaway environment exported. No agent session exists.

## Reproduce headless

- The feature under test is the daemon and the CLI together. Two harnesses exist, both with fakes:
  - `bazel test //apps/humata2/cli:machine_scripts_test` runs the CLI against `FakeDaemon` (its exit codes and `--json` shapes).
  - `bazel test //apps/humata2/daemon/access:machine_script_run_test` checks the Never / Allow / Ask policy for `machine_script_run`.
  - `bazel test //apps/humata2/daemon/machines:machine_scripts_test` checks the rules parser, every problem at once, and warnings.
  - `bazel test //apps/humata2/daemon/humatad:humatad2_unit_test --test_arg=machine_scripts` runs the engine on `FakeSteps` (no real shell).
- The throwaway daemon, run by the tester (not the owner), with the built binaries from `bazel build //apps/humata2/daemon/humatad:humatad2 //apps/humata2/cli:humata`:

  ```
  D=$(mktemp -d)   # the throwaway root
  mkdir -p "$D/home/.humata2/config" "$D/home/.humata2/scripts" "$D/root"
  HOME="$D/home" humatad2 serve --socket "$D/h.sock" --state "$D/home/.humata2/state" --root "$D/root" --holder <the holder path the daemon tests use> --ready-fd <fd>
  ```

  Then `export HUMATA_SOCKET="$D/h.sock" HOME="$D/home"` and run `humata machine script ...`. The `--ready-fd` and `--holder` values follow `daemon/humatad/src/serve.rs` (the test start-up near line 1960). Capture the stdout, stderr and exit code of every command below as evidence.
- Agent identity cannot be made headless without a live agent session. The nearest equivalent is a script token: `humata token create --name t --capability machine_script_run --days 1`, then `HUMATA_TOKEN=<token>` for the call. A token without the capability is the closest test of the refusal.
- What cannot be done headless: a real agent asking for the run, the Inbox card a channel's Ask rule makes (it needs the app open), and a real `setup-tools.sh` run on a second machine.

## Steps

1. **I do:** `humata machine script check` with the valid file above.  
   **I see:** no problems. Exit 0. With `--json`, the result has `valid: true`, no problems and no warnings.  
   **State and events:** `CheckMachineScripts` is sent. The daemon reads the rules file and checks each step's script name against the scripts folder.  
   **Status:** Fakes only. `cli/tests/machine_scripts.rs::check_of_a_valid_file_exits_ok_and_carries_the_warnings` (FakeDaemon). The daemon's check is not run by any test: Code, no test for the RPC itself.

2. **I do:** edit the file to add two broken rules: a rule named `Bad_Name` (capitals and an underscore), and a rule whose step is `run = "missing.sh"`, a script that isn't in the folder. Then run `humata machine script check`.  
   **I see:** exit 2. stderr says the rules have 2 problems and lists both, one per line. Each line names the rule and what to change. The fix line says to fix each problem in the rules file or the scripts folder, then run `check` again.  
   **State and events:** the daemon collects every problem in one pass (`parse_machine_scripts` in `daemon/machines/src/machine_scripts.rs`), so both show together.  
   **Status:** Fakes only for the CLI: `cli/tests/machine_scripts.rs::check_of_an_invalid_file_exits_usage_and_lists_every_problem`. The "every problem at once" rule is a parser test: `daemon/machines/tests/machine_scripts.rs::every_problem_is_reported_at_once`. Neither runs the real daemon.

3. **I do:** run `humata --json machine script check` on the same file.  
   **I see:** exit 2. The JSON error has `kind: "invalid_field"`, and `details.problems` holds the two problems as strings. `details.warnings` is empty, because the daemon drops warnings when the file is invalid.  
   **Status:** Fakes only: `cli/tests/machine_scripts.rs::check_of_an_invalid_file_exits_usage_and_lists_every_problem`. The warnings drop is Likely bug, see Gaps.

4. **I do:** replace the file with broken TOML (an unclosed string), then run `check`.  
   **I see:** exit 2 with one problem: the file isn't valid TOML of the expected shape, and the text says to fix it and run `check` again.  
   **State and events:** `MachineScriptsError::Syntax` becomes one problem in the check reply.  
   **Status:** Code, no test. The parser maps the syntax error (`daemon/machines/src/machine_scripts.rs` `Syntax`), and `daemon/machines/tests/machine_scripts.rs::unknown_keys_and_bad_toml_are_syntax_errors` covers the parser. No test covers the CLI exit for it.

5. **I do:** restore the valid file, then run `humata machine script list`.  
   **I see:** a table with the header RULE, ON, MACHINES, ONCE, STEPS. The `setup` row shows `added,connected`, `all`, `no`, `run setup-tools.sh`. Then a table of last runs (empty for now, since nothing ran), then `scripts: setup-tools.sh`. Exit 0.  
   **State and events:** `ListMachineScripts` with no machine. The daemon returns every rule, the script names, and the last run per rule per machine that the rule applies to.  
   **Status:** Fakes only: `cli/tests/machine_scripts.rs::list_without_a_machine_asks_for_every_machine`. The real list is Code, no test.

6. **I do:** run `humata machine script run setup this-mac` as the owner, with no session.  
   **I see:** exit 0 and the Operation the daemon started, printed as it is (the subcommand does not wait; see Gaps). The Operation's kind is `MACHINE_SCRIPT`.  
   **State and events:** `RunMachineScript` with the rule, the machine, no session and a fresh idempotency key (the CLI makes the key itself). The trigger is `manual`. The daemon runs the rule in the background, then logs the run and sends one feed event.  
   **Status:** Fakes only: `cli/tests/machine_scripts.rs::run_sends_the_rule_machine_and_session_with_a_key`. The real run on `this-mac` is not tested anywhere: whether the daemon's machine path reaches this Mac's own shell is not shown by any test. Code, no test.

7. **I do:** after the run ends, run `humata machine script log this-mac --rule setup`.  
   **I see:** a table: RUN, RULE, TRIGGER (`manual`), STARTED, OUTCOME (`passed`), TOOK. Nothing else for a passed run.  
   **State and events:** `ListMachineScriptRuns` with the rule and no limit (0 means the whole log, up to the last 50 the daemon keeps).  
   **Status:** Fakes only: `cli/tests/machine_scripts.rs::log_sends_the_machine_rule_and_limit`. The log file and its 50-run cap: `daemon/humatad/src/machine_scripts/log.rs` unit tests. Code, no test at the CLI level.

8. **I do:** change `setup-tools.sh` to `echo "E: no space left"; exit 1`, run the rule again, then run `humata --json machine script log this-mac`.  
   **I see:** the JSON has the new run with `outcome: "failed"`, and its failed step has `exit: 1`, the step label, and the output tail (the last lines). In text mode, the log prints one line per failed step under the table, with the exit and the output lines indented.  
   **Status:** Fakes only: `cli/tests/machine_scripts.rs::log_in_json_carries_each_run_and_its_steps` (a passed run). The failed-step text is in `cli/src/commands/machine_scripts.rs` (`log_text`), which no test asserts on. Code, no test for the failed text.

9. **I do:** run `humata machine script run no-such-rule this-mac`.  
   **I see:** exit 3 (not found). The message names the rule and says to list the rules or add it to the rules file. No Operation is started.  
   **State and events:** `find_rule` answers NotFound before any run starts.  
   **Status:** Code, no test. The CLI maps NotFound to exit 3 (`cli/src/exit.rs`). No test sends an unknown rule.

10. **I do:** add a rule with `machines = ["box-9"]` and run it on `this-mac`.
    **I see:** exit 2 (invalid field). The message says the rule is not for `this-mac`, because its `machines` list leaves it out.
    **Status:** Code, no test: `find_rule` in `daemon/humatad/src/machine_service/script_rpc.rs`. The engine's own check (`run_now_names_the_rule_and_the_machine_it_is_wrong_for`) uses `FakeSteps`.

11. **I do:** run `humata machine script run setup this-mac --wait`.
    **I see:** `run` has no `--wait` flag of its own. The subcommand help nonetheless says `--wait` follows the run to the end. The wait the CLI does have is the global `--wait <seconds>`, and whether clap accepts it after the subcommand needs a live check (exit 2 if not).
    **Status:** Not built (the flag named in the help does not exist). See Gaps.

12. **I do:** run `humata machine script edit setup`, or any other write-looking word.
    **I see:** a usage error, exit 2. No rule or script changes. The subcommands are list, check, run and log only.
    **State and events:** no RPC writes rules or scripts; the four `MachineService` RPCs are list, check, run and read the log.
    **Status:** Code, no test. Checked by reading `cli/src/args.rs` (`ScriptCommand`) and `api/proto/humata/v1/hosts.proto`. No test asserts that nothing writes the file.

## Branches

### An agent's run is refused by default

- **When:** an agent session (its own session token) runs `humata machine script run setup this-mac`. Its channel has no rule for `machine_script_run`, which is Never by default.
- **I do:** run the command from the agent's shell.
- **I see:** exit 4 (denied). The error names the capability, `machine_script_run`, and the channel, and says how the owner can allow it in the channel's agent access.
- **State and events:** no Operation starts. The run log does not change. The Inbox gets no card, since a Never rule asks nothing.
- **Expected outcome:** the agent's run does nothing.
- **Status:** Code, no test at the RPC. The policy is proven by `daemon/access/tests/machine_script_run.rs::a_new_channel_never_lets_an_agent_run_a_machine_script` (a policy check, not a daemon call). The RPC gate is `begin_run` → `run_gate` in `script_rpc.rs`, which no test reaches.

### An agent's run asks when the channel says Ask

- **When:** the channel's rule for `machine_script_run` is Ask.
- **I see:** the agent's run returns an answer that it waits on. An Inbox request appears in the app for the owner to approve. Approving runs the rule once, through the same call with `approved` set.
- **Expected outcome:** the owner decides. Denying leaves nothing run.
- **Status:** Code, no test. The policy's Ask verdict is covered by `daemon/access/tests/machine_script_run.rs::a_channel_whose_rule_says_ask_puts_the_run_in_the_inbox`. The Inbox card and the approved re-run are not tested.

### An agent's run goes ahead when the channel allows it

- **When:** the channel's rule is Allow.
- **I see:** the run starts and the Operation returns, as for the owner.
- **Status:** Code, no test. The policy is covered by `a_channel_whose_rule_allows_it_lets_its_agents_run_one` (`daemon/access/tests/machine_script_run.rs`), not the RPC.

### A script token without the capability is refused

- **When:** the headless stand-in for an agent: `HUMATA_TOKEN` from `humata token create` without `--capability machine_script_run`.
- **I see:** exit 4 (denied), as for the agent case.
- **Status:** Code, no test. Nearest headless equivalent only. The code path is the same `run_gate` with a `Caller::Script`, but this check is not run here.

### An agent lists or reads the log

- **When:** an agent runs `humata machine script list`, `log` or `check`.
- **I see:** `list` and `log` print nothing and exit 0 (empty results). `check` exits 4 (denied, user only).
- **Expected outcome:** the spec says reads follow `machine_read`. That capability does not exist, so agents get no rules and no runs, and the empty answer does not say why.
- **Status:** Not built. See Gaps.

### No rules file at all

- **When:** the rules file was never made.
- **I see:** `list` shows no rules and no runs; `check` exits 0 with no problems and no warnings; `run` says there is no rule of that name.
- **Status:** Fakes only at the parser level: `daemon/machines/tests/machine_scripts.rs::a_missing_file_is_no_rules_and_a_present_one_is_checked_against_the_scripts_folder`. The CLI and daemon path are Code, no test.

### A rule names a machine that isn't added yet

- **When:** `machines = ["box-9"]` and no machine by that name exists.
- **I see:** `check` exits 0 and prints the warning that the rule applies once a machine of that name is added. The rule is still valid.
- **Status:** Fakes only: `cli/tests/machine_scripts.rs::check_of_a_valid_file_exits_ok_and_carries_the_warnings` (the warning text from FakeDaemon) and `daemon/machines/tests/machine_scripts.rs::an_unknown_machine_is_only_a_warning_and_star_is_everything`.

### The daemon restarts during a run

- **When:** `humatad2` stops while `setup-tools.sh` runs on `this-mac`.
- **I see:** the Operation's state after the restart, and whether the run appears in the log, are not shown by any test.
- **Expected outcome:** the run shows as ended, not as running forever, and the log holds whatever finished.
- **Status:** Code, no test.

## Expected outcomes

- [ ] `check` exits 0 for a valid file and prints any warnings.
- [ ] `check` exits 2 for an invalid file, and lists every problem, in text and in `--json` (`details.problems`).
- [ ] A broken TOML file gives exit 2 with the syntax problem.
- [ ] `list` prints the rules, the last run per machine, and the script names, and needs no machine.
- [ ] `run` as the owner starts one Operation, with trigger `manual`, and the run appears in `log`.
- [ ] `log` shows newest first, with the outcome, and the failed steps with exit and output.
- [ ] `run` of an unknown rule exits 3 and starts nothing.
- [ ] `run` of a rule that leaves the machine out exits 2 and starts nothing.
- [ ] An agent's `run` is refused (exit 4) unless its channel allows `machine_script_run`; Ask puts it in the Inbox.
- [ ] No command or RPC writes the rules file or the scripts folder.
- [ ] Reads by an agent say why they are empty (`machine_read` does not exist; see Gaps). A peer machine's call is refused.

## Status summary

| Step or branch | Status | Evidence |
| --- | --- | --- |
| 1. check valid exits 0 | Fakes only | `cli/tests/machine_scripts.rs::check_of_a_valid_file_exits_ok_and_carries_the_warnings` |
| 2. check invalid lists every problem | Fakes only | `cli/tests/machine_scripts.rs::check_of_an_invalid_file_exits_usage_and_lists_every_problem`; `daemon/machines/tests/machine_scripts.rs::every_problem_is_reported_at_once` |
| 3. check --json carries problems | Fakes only | same CLI test; warnings dropped: Likely bug |
| 4. broken TOML exits 2 | Code, no test | `daemon/machines/tests/machine_scripts.rs::unknown_keys_and_bad_toml_are_syntax_errors` (parser only) |
| 5. list shows rules, runs, scripts | Fakes only | `cli/tests/machine_scripts.rs::list_without_a_machine_asks_for_every_machine` |
| 6. run as owner starts an Operation | Fakes only | `cli/tests/machine_scripts.rs::run_sends_the_rule_machine_and_session_with_a_key`; real run: Code, no test |
| 7. log shows a passed run | Fakes only | `cli/tests/machine_scripts.rs::log_sends_the_machine_rule_and_limit` |
| 8. log shows a failed step | Code, no test | `cli/src/commands/machine_scripts.rs` `log_text` |
| 9. unknown rule exits 3 | Code, no test | `daemon/humatad/src/machine_service/script_rpc.rs` `find_rule` |
| 10. rule for another machine exits 2 | Code, no test | `script_rpc.rs` `find_rule` |
| 11. `run --wait` | Not built | `cli/src/args.rs` line 843 says `--wait`; no such flag |
| 12. no write subcommand | Code, no test | `cli/src/args.rs` `ScriptCommand`; `hosts.proto` |
| Branch: agent refused by default | Code, no test | policy: `daemon/access/tests/machine_script_run.rs::a_new_channel_never_lets_an_agent_run_a_machine_script` |
| Branch: agent asks under Ask | Code, no test | policy: `..::a_channel_whose_rule_says_ask_puts_the_run_in_the_inbox` |
| Branch: agent allowed under Allow | Code, no test | policy: `..::a_channel_whose_rule_allows_it_lets_its_agents_run_one` |
| Branch: script token without capability | Code, no test | nearest headless; `run_gate` |
| Branch: agent reads | Not built | `machine_read` absent from `daemon/access/src/capability.rs` |
| Branch: no rules file | Fakes only (parser); CLI Code, no test | `daemon/machines/tests/machine_scripts.rs::a_missing_file_is_no_rules_...` |
| Branch: unknown machine is a warning | Fakes only | `daemon/machines/tests/machine_scripts.rs::an_unknown_machine_is_only_a_warning_and_star_is_everything` |
| Branch: daemon restart mid-run | Code, no test | none |

## Gaps and likely bugs

Not built:
- `humata machine script run --wait`: the help in `cli/src/args.rs` (line 843) says `--wait` follows the run, but the subcommand has no such flag. The only wait is the global `--wait <seconds>` (`follow` in `cli/src/commands/hosts.rs`), which outside a terminal defaults to 0, so a script's run returns at once with the Operation.
- Agent reads follow `machine_read`: no such capability exists in `daemon/access/src/capability.rs`. `list` and `log` return empty results to an agent, and `check` refuses it, with no reason in the empty case (`sees` in `script_rpc.rs`).
- No test runs a real `bash -s` script on a machine. `daemon/machines/tests/machine_scripts.rs::a_bash_script_goes_to_the_machine_on_stdin` only builds the invocation, and the engine tests use `FakeSteps`.

Likely bugs:
- `check` never shows warnings for an invalid file: `parse_machine_scripts` returns `Invalid` without its `warnings`, and `check()` in `script_rpc.rs` sends `warnings: Vec::new()` on that path. A file with one bad rule and one rule for `box-9` shows only the problem. Confirm with that file.

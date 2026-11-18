# A script I wrote runs on a new machine as soon as Humata's tools are on it

Layer: real. Platforms: macOS host, Linux arm64 VM. Spec: `workflow/milestones/MACHINESCRIPTS.md` (D680-D689, Provisional). Nothing here is built: every MACHINESCRIPTS step is Not built (MACHINESCRIPTS). Steps that are Humata's existing add flow carry their tags from `real/01-the-vm-becomes-a-machine.md`.

## Starting state

- This Mac runs a real Humata 2 app and its own `humatad2` on a throwaway home. The VM `h2-terminal-c1` is running and is not added (see `real/01`).
- `~/.humata2/config/machine-scripts.toml` does not exist, so nothing runs (D680: no compiled-in rules).
- `~/.humata2/scripts/` does not exist. Humata makes it and a commented sample file the first time Settings › Machines › Scripts opens.
- The pinned Copilot CLI is on the VM under a test folder with an isolated `COPILOT_HOME`, and the VM has a `copilot` kind from an `--agents` file (see `real/04`). A channel `setup-lab` exists on this Mac.
- I am on Home.

## Test setup

- Everything from `real/01` Test setup: throwaway `HOME`, `--ssh-config` set to the Lima config, the `linux-aarch64` tools bundle. Never stop or restart the shared VM.
- Rules and scripts are plain files under the throwaway home. No secret goes in them: the test script reads nothing secret, and a Copilot login stays in the isolated `COPILOT_HOME` (`real/04`).
- The scripts: `hello.sh` writes `registered $HUMATA_MACHINE` to `/var/tmp/h2-real-clis/hello.txt` and exits 0. `fail.sh` prints a line and exits 3. `slow.sh` sleeps 600.
- Needs the owner's login for the Copilot agent step (see `real/04`). The bash-only steps need none.
- Needs a free screen for the machine page. Headless equivalent: `humata machine script check|run|log` (D687).
- Cleanup: remove `/var/tmp/h2-real-clis/hello.txt`, the rules file and the scripts from the throwaway home.

## Steps

1. **I do:** Write `~/.humata2/scripts/hello.sh`, then write `~/.humata2/config/machine-scripts.toml` with a rule `name = "hello"`, `on = "added"`, `machines = ["lima-h2-terminal-c1"]`, `once = true`, and one step `run = "hello.sh"`. I add a second rule `name = "restart-copilot"`, `on = "added"`, with `start = { agent = "copilot", channel = "setup-lab" }` and then `send = "/restart"`.
   **I see:** Nothing yet. Both files are mine, in my editor. No secret is in either.
   **State and events:** No RPC writes rules or scripts (D688). The rules name a script by file name, never a path (D680).
   **Status:** Not built (MACHINESCRIPTS T1).

2. **I do:** Run `humata machine script check`.
   **I see:** Exit 0. If I mistype (`run = "../x.sh"`, `start` on a session trigger, a missing script), exit 2 with every problem listed at once.
   **State and events:** The check reads the file and the script names only. A machine name that does not exist yet is a warning, not an error, since the machine may be added later (D683).
   **Status:** Not built (MACHINESCRIPTS T1, T3).

3. **I do:** Run `humata machine script list`.
   **I see:** Both rules, their trigger `added`, and "never run" for the VM, which is not a machine yet.
   **State and events:** None.
   **Status:** Not built (MACHINESCRIPTS T3).

4. **I do:** Open Hosts, press Add a machine…, type `lima-h2-terminal-c1`, press Add, check the fingerprint and press Confirm.
   **I see:** Connect, Check, Install and Start run and finish, as in `real/01`. No script has run yet.
   **State and events:** The `added` trigger fires at the first link-up after Add succeeds (D681), not during Install. The rules file was shipped to `~/.humata2/config/machine-scripts.toml` on the VM with the other config (D680).
   **Status:** Works (real daemon) for the add, over a fake `ssh`: `daemon/humatad/tests/machines.rs::a_machine_is_added_once_its_host_key_is_confirmed_then_removed_by_plan` (see `real/01` steps 3-7). The real VM add is Code, no test. The trigger and the file ship are Not built (MACHINESCRIPTS T2).

5. **I do:** On the VM, read `/var/tmp/h2-real-clis/hello.txt` and `ls ~/.humata2/bin`.
   **I see:** The file says `registered lima-h2-terminal-c1`. `~/.humata2/bin` already held `humatad2`, `humata2_holder`, `humata` and `stack` when the script ran.
   **State and events:** The script's bytes were streamed to `bash -s` over the link (D680). No copy of `hello.sh` is left on the VM. The script saw `HUMATA_TRIGGER=added` and `HUMATA_MACHINE`.
   **Status:** Not built (MACHINESCRIPTS T2).

6. **I do:** Open the machine page for the VM, Overview, Scripts.
   **I see:** Two rows, `hello` and `restart-copilot`, each with its outcome and time. Opening `hello` shows the step, its exit 0 and the last lines of output.
   **State and events:** The run is in the host's run log for this machine (last 50 runs, D686). The page reads the log and the feed says when it changes. Nothing is polled.
   **Status:** Not built (MACHINESCRIPTS T2, T5).

7. **I do:** Wait for the agent rule. I do not touch the VM.
   **I see:** A Copilot session starts on the VM in `setup-lab`. Copilot's folder-trust prompt comes first and is answered by the session's own start (`real/05` step on trust). The step then waits.
   **State and events:** `start` goes through the same StartSession every client uses (D682). The rule's `send` step waits for the session to be idle (SE-21). It does not type while the trust prompt is showing.
   **Status:** Not built (MACHINESCRIPTS T2). Copilot start itself: Works (real CLI, manual REALFLOW run), no test pins it; its trust prompt first is seen in that run. Agent kind `copilot` is not built in (AGENTKINDS; `--agents` file needed).

8. **I do:** Wait until the Copilot prompt is idle.
   **I see:** `/restart` appears typed once in the session's terminal and Copilot restarts. The session shows Idle again.
   **State and events:** "Ready" means the folder-trust prompt is answered and the turn state says the agent waits for input (SE-21, the same signal turn ends use). `send` presses Enter by default (`keys = "enter"`), as `humata session type` now does too (B12 fixed, D754). The send is keyed, so a retry cannot type it twice (D682).
   **Status:** Not built (MACHINESCRIPTS T2). Idle detection for Copilot: Works (real CLI, manual REALFLOW run). B12 is Fixed (B12, D754): `humata session type` presses Enter by default.

9. **I do:** Open the `restart-copilot` row.
   **I see:** Two steps, `start` and `send`, both passed, with timings. The rule shows Passed.
   **State and events:** The Inbox has no item. A passing run never asks anything (D686, and the owner's rule that status shows only where it blocks an action).
   **Status:** Not built (MACHINESCRIPTS T2, T5).

## Branches

### The script exits non-zero

- **When:** `hello` is `fail.sh`, which exits 3.
- **I do:** Add the VM. Read the Inbox, press Retry after fixing the script.
- **I see:** The `hello` rule shows Failed on the machine page, with exit 3 and the output tail. One Inbox item, "Script hello failed on lima-h2-terminal-c1", carries the failed step, its exit, the tail and the trigger, with Retry and Dismiss (D686, D105 full decision context). The add itself still ends Succeeded and the machine is online.
- **Expected outcome:** The rule stopped at the failed step. The next rule still ran. A second failure replaces the item. A pass withdraws it.
- **Status:** Not built (MACHINESCRIPTS T2).

### The script times out

- **When:** `slow.sh` with `timeout = "10s"` on the rule.
- **I do:** Add the VM and wait.
- **I see:** After 10 seconds the step is killed and the run shows Timed out. The Inbox item says so and says to raise `timeout` or fix the script.
- **Expected outcome:** No `sleep` is left running on the VM. The default step cap is 5 minutes, a rule's cap is at most 1 hour, and a wait for idle defaults to 2 minutes (D684).
- **Status:** Not built (MACHINESCRIPTS T2).

### The machine goes offline mid-script

- **When:** `slow.sh` is running and I cut the link (kill the tunnel; never restart the shared VM).
- **I do:** Wait for the link to come back.
- **I see:** The run shows Failed with a cause that says the machine was out of reach, not a script error. After the link returns, `connected` rules run (see `real/11`), but `added` does not run again.
- **Expected outcome:** The interrupted run is not silently lost or counted as passed. With `once = true` the failure is retried at the next trigger or by Retry (D684).
- **Status:** Not built (MACHINESCRIPTS T2).

### The agent CLI is not on the VM

- **When:** `copilot` is not installed or has no kind.
- **I do:** Add the VM and read the Inbox.
- **I see:** The `start` step fails and names the kind `copilot`. It says to install the CLI on the machine or add the kind in Settings › Agents. The `send` step does not run.
- **Expected outcome:** Nothing is typed into any other session. No half-started session is left.
- **Status:** Not built (MACHINESCRIPTS T2; AGENTKINDS for built-in `copilot`).

### An agent kind that cannot say when it is ready

- **When:** the kind's turn end is unknown (`TurnEnd::None`).
- **I do:** Add the VM.
- **I see:** The `send` step fails with "can't tell when it is ready; set `wait = "none"`". Nothing is typed.
- **Expected outcome:** Humata never types blind by default (D682).
- **Status:** Not built (MACHINESCRIPTS T2).

### The script writes into Humata's folders

- **When:** the script runs `touch ~/.humata2/bin/x` or writes under `~/.humata2/scripts`.
- **I do:** Add the VM.
- **I see:** Unknown. The spec refuses a rule that names such a path (TS-4 for sync rules, D680 for script names), but a bash script can write anywhere the login shell can. See open question 3.
- **Expected outcome:** At least `humata machine script check` refuses a script name or argument that points at `~/.humata2`; whether the run is also confined is not decided.
- **Status:** Not built (MACHINESCRIPTS; undecided).

### I add the machine again

- **When:** I remove the VM and add it again, or Try again after a failed add.
- **I do:** Add it a second time.
- **I see:** With `once = true` on `hello`, the second add does not run it again and the row shows "ran earlier". Without `once`, it runs again on the new `added`.
- **Expected outcome:** A re-add counts as a new `added`. Whether `once` survives a remove is an open question (4). `humata machine script run hello lima-h2-terminal-c1` runs it on purpose either way (D684, D687).
- **Status:** Not built (MACHINESCRIPTS T2).

### I preview before adding

- **When:** I want to see what would run.
- **I do:** `humata machine script list` and `check`, then `humata machine script run hello lima-h2-terminal-c1 --wait` on an existing machine.
- **I see:** `list` and `check` show the plan without running anything. There is no dry run that shows a script's effects.
- **Expected outcome:** Preview is limited to listing and checking (D687). A true dry run is an open question (5).
- **Status:** Not built (MACHINESCRIPTS T3).

### An agent tries Run now

- **When:** an agent session calls `humata machine script run`.
- **I do:** Read the Inbox.
- **I see:** Refused unless the channel's access allows `machine_script_run` (NeverByDefault, D688). The owner's own CLI is not gated.
- **Expected outcome:** No script ran from the agent's request.
- **Status:** Not built (MACHINESCRIPTS T4).

## Expected outcomes

- [ ] With no rules file, adding a machine runs nothing.
- [ ] `check` lists every problem at once and exits 2 on a bad file; a good file exits 0.
- [ ] The `added` rule runs once, after the tools are installed and the link is up, and sees `HUMATA_MACHINE` and `HUMATA_TRIGGER`.
- [ ] The script is streamed, not stored, on the VM.
- [ ] `/restart` is typed once, only after the trust prompt is answered and Copilot is idle, and Enter is pressed.
- [ ] A pass shows on the machine page and nowhere else. A failure also gives one Inbox item with Retry.
- [ ] A failing script never fails the add or takes the machine offline.
- [ ] Rules and scripts hold no secrets and no RPC writes them.
- [ ] The owner's real `~/.humata2`, `~/.copilot` and the VM's shared state are untouched.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Write rules and scripts | Not built (MACHINESCRIPTS T1) | `workflow/milestones/MACHINESCRIPTS.md` |
| 2. `check` | Not built (MACHINESCRIPTS T1, T3) | same |
| 3. `list` | Not built (MACHINESCRIPTS T3) | same |
| 4. Add the VM | Works (real daemon), fake ssh; trigger Not built | `daemon/humatad/tests/machines.rs::a_machine_is_added_once_its_host_key_is_confirmed_then_removed_by_plan` |
| 5. Script ran after install | Not built (MACHINESCRIPTS T2) | none |
| 6. Machine page Scripts | Not built (MACHINESCRIPTS T2, T5) | none |
| 7. Copilot starts, trust prompt first | Not built (MACHINESCRIPTS T2) | REALFLOW (manual) for Copilot itself |
| 8. Idle, then `/restart` + Enter | Not built (MACHINESCRIPTS T2) | B12 fixed (D754) |
| 9. Passed run, no Inbox item | Not built (MACHINESCRIPTS T2, T5) | none |
| Branch: non-zero exit | Not built (MACHINESCRIPTS T2) | none |
| Branch: timeout | Not built (MACHINESCRIPTS T2) | none |
| Branch: offline mid-script | Not built (MACHINESCRIPTS T2) | none |
| Branch: agent not installed | Not built (MACHINESCRIPTS T2, AGENTKINDS) | none |
| Branch: readiness unknown | Not built (MACHINESCRIPTS T2) | none |
| Branch: writes Humata's folders | Not built (undecided) | none |
| Branch: add again | Not built (MACHINESCRIPTS T2) | none |
| Branch: preview | Not built (MACHINESCRIPTS T3) | none |
| Branch: agent Run now | Not built (MACHINESCRIPTS T4) | none |

## Gaps and likely bugs

- **Not built:** the whole feature (MACHINESCRIPTS T1-T5). Nearest code is `daemon/machines/src/sync_rules.rs`, which "only parses and checks" TS-3 rules; nothing in `daemon/humatad/src` runs a sync rule's `before` or `after`.
- **Not built:** built-in `copilot` and `codex` kinds (AGENTKINDS). A real run needs an `--agents` file.
- **Fixed (B12, D754): `humata session type` presses Enter by default.** The `send` step still presses its own Enter.
- **Likely bug (real/01 step 6):** the app bundle carries `tools_mac` only, so a real app may have no Linux tools to install before any script can run.
- **Risk:** the VM has 0.75 GiB RAM. `humatad2`, a holder and Copilot at once may be killed by the kernel (see `real/04`).
- **Open questions the spec must answer:**
  1. What exactly is "ready" for an agent: the trust prompt gone plus SE-21 idle, or a kind-specific check?
  2. Does a script's non-zero exit on `added` mark the machine as needing attention anywhere besides the Inbox item?
  3. Can a script be confined from writing `~/.humata2` (TS-4), or is only the rule's names checked?
  4. Does `once` survive removing and re-adding the same machine?
  5. Is there a real dry run (for example `--plan` that prints the steps and variables), beyond `list` and `check`?

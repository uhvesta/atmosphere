# A dropped link leaves every agent running on the VM, and reconnect re-attaches each with no lost and no replayed input

Real. Platforms: macOS (the app and the link). The VM is Debian on arm64. Paths are relative to `apps/humata2`.

This is a disconnect, not a shutdown. Nothing on the VM stops. The link between this Mac and the VM breaks (the SSH process dies, the network is cut, this Mac sleeps, or Wi-Fi changes), and later it comes back. For a power-off or reboot of the VM or this Mac, see [real/13](13-the-vm-and-this-mac-reboot.md). For one shell over a cut link, see [real/03](03-the-vm-link-drops-and-the-app-restarts.md), which this case reuses. Agent kinds and the cheapest-model keystrokes are in [real/05](05-copilot-on-the-cheapest-model-on-this-mac.md) and [real/07](07-codex-on-the-cheapest-model-on-this-mac.md). Related: [agents/20](../agents/20-an-agent-on-a-box-while-the-host-is-away.md), [agents/19](../agents/19-an-agent-across-a-restart.md).

This case is for a run by hand or a later test. No test cuts a link under a real agent CLI through `humatad2`. Each step says which half is proven.

## Starting state

- The VM `h2-terminal-c1` is running and is a machine in the app, with its link up (real/01). Channel `acme/api` has this Mac and the VM, with a clone on each. The repo folder on the VM is `hello/`.
- Three sessions run on the VM, one per agent kind, each on its cheapest model:
  - `claude-s` (Claude Code, its smallest model),
  - `codex-s` (Codex, GPT-6 Luna, low reasoning),
  - `copilot-s` (Copilot, Claude Haiku 5.5, low reasoning).
  Codex and Copilot run as custom kinds from an `--agents` JSON file (REALFLOW B1: no built-in kinds).
- Every task prompt names the repo folder. Humata does not tell an agent where the repo is; the person or the test prompt does.
- `copilot-s` is mid-turn: it was told "In the hello/ folder, write notes.txt with ten lines, one line at a time", so it keeps working for a minute.
- `claude-s` has asked for permission to run one command in `hello/`. Its permission hook on the VM is holding, and the card shows in the Inbox.
- `codex-s` is idle after one finished turn ("In the hello/ folder, add the line `Tested.` to README.md").
- I wrote down each session's conversation id, the last line of each tab, and the VM process ids of the three agents.

## Test setup

- VM: `./terminal-ssh-demo.sh start`, then `status`. Never stop, restart or delete the shared VM, and never stop its network (`terminal/platform_test/SSH_DEMO.md`). The VM has 0.75 GiB of RAM. Three agent CLIs at once, one of them Node-based, may run out of memory. If one is killed by the VM's memory limit, the run is invalid. Run two kinds, then add the third.
- The agent CLIs on the VM are put there as in real/04, each in its own home (`COPILOT_HOME`, `CODEX_HOME`, `CLAUDE_CONFIG_DIR` under a temp folder on the VM). Never the owner's `~/.copilot`, `~/.codex`, `~/.claude`, `~/.humata2` or SSH keys.
- This Mac's `humatad2` has its own temp state folder and `--ssh-config "$HOME/.lima/h2-terminal-c1/ssh.config"` (`daemon/humatad/src/serve.rs:131-134`).
- Cutting the link, the safe way: kill only the `ssh` the daemon started for the VM (its command line has `lima-h2-terminal-c1` and `--role machine`; the provider uses `ControlPath=none`, `daemon/machines/src/link.rs:390`). Never kill Lima's own `limactl` agent. Other cuts, for the branches: close this Mac's lid; switch Wi-Fi networks. These need the owner at the Mac.
- **Needs the owner's login** for Copilot (device login), Codex (`login`) and Claude. Tokens land only in the run's temp homes. Alternative with no login: the loopback model server in `terminal/provider/tests/*_resume.py`. It proves resume without a real model, and not through `humatad2`.
- **Needs a free screen** for every step that looks at the app window. Headless equivalent: the `humata` CLI to list sessions, read tabs and answer the Inbox, plus a `desktop/tests/live_flows/*` style test.

## Steps

1. **I do:** start `claude-s`, `codex-s` and `copilot-s` on the VM and bring each to the state above. Choose each model with its own `/model` keystrokes.
   **I see:** three sessions on the machine, each with an agent tab.
   **State and events:** each session's launch record names the conversation id and its resume command. Humata keeps no model setting (real/05 step 4).
   **Status:** Works (real daemon), fake agent: `daemon/humatad/tests/remote.rs::a_session_started_on_the_machine_runs_there_and_the_host_relays_its_terminal` and `daemon/humatad/tests/two_machines.rs::a_session_started_on_box_2_runs_there_with_its_conversation_on_box_2`. Real CLIs on this Mac only: Works (real CLI, manual REALFLOW run), Copilot, no test pins it. Real CLIs on the VM: Code, no test. **Needs the owner's login.**

2. **I do:** make `claude-s` ask. Let `copilot-s` start its slow turn. Write down the last printed line of each tab.
   **I see:** `claude-s` shows Needs input and a card in the Inbox with Allow and Deny. `copilot-s` shows Working.
   **State and events:** the agent's permission hook runs `humata hook` on the VM, against the VM's own `humatad2`. The hook holds up to an hour (`cli/src/commands/hook.rs:22`).
   **Status:** Works (real daemon) for the hold: `daemon/humatad/tests/hooks.rs::the_daemon_writes_a_blocking_claude_hook_that_holds_for_an_hour`. The card from a hook on a machine, shown on the host: Code, no test. `daemon/humatad/src/away/journal.rs` says every Inbox change is journaled for the host, but no test raises a hook-held card on a machine and shows it on the host.

3. **I do:** cut the link. Kill the daemon's `ssh` for the VM. **Needs a free screen.**
   **I see:** within seconds the machine shows Away or offline, and each of the three sessions shows offline. A terminal tab says "Lost the connection to" the machine.
   **State and events:** the host marks the machine's sessions offline. It does not stop, restart or start any session.
   **Status:** Works (real daemon), fake `ssh` and fake agents: `daemon/humatad/tests/two_machines.rs::a_lost_link_comes_back_and_the_session_is_the_same` and `daemon/humatad/tests/remote.rs::a_machine_that_goes_away_and_returns_reattaches_its_session_without_starting_it_twice` (stronger: it also kills the machine's daemon). App tab text: `desktop/tests/live_flows/link_drop.rs::a_dropped_link_refuses_typing_and_the_same_shell_takes_it_again_once_back` (stand-in `ssh`, a shell tab). Over real SSH through the app: Code, no test.

4. **I do:** from a second SSH session of my own, list the three agents' process ids on the VM.
   **I see:** the same three pids as before the cut, with elapsed time still counting. `copilot-s` finishes its turn on the VM during the outage.
   **State and events:** the agents run under the VM's holders, which do not depend on this Mac's `ssh`.
   **Status:** Works (real shell), component only, for a shell: `terminal/host/src/roaming_tests.rs::ssh_outage_and_route_change_reattach_existing_holder_without_input_replay` and `terminal/session/tests/ssh.rs::killing_the_ssh_frontend_reaps_its_tunnel_and_keeps_the_remote_shell` (manual, real VM). Agents: Works (real daemon), fake agent: the same holder pid before and after in `remote.rs::a_machine_that_goes_away_and_returns_reattaches_its_session_without_starting_it_twice`. A real Claude, Codex or Copilot over a cut: Code, no test.

5. **I do:** while the link is down, try Stop on `codex-s`, start a second session on the VM, and type a line into `claude-s`'s tab.
   **I see:** Stop and Start are refused at once as machine offline, and no second session appears. The typed line is refused with a toast and is not echoed.
   **State and events:** nothing is queued for later. A refused start leaves no record.
   **Status:** Works (real daemon): `remote.rs::a_machine_that_goes_away_and_returns_reattaches_its_session_without_starting_it_twice` asserts both refusals and one stored session. Typing: Works (real daemon), stand-in `ssh`: `link_drop.rs::a_dropped_link_refuses_typing_and_the_same_shell_takes_it_again_once_back`; also Fakes only: `clients/live/src/link.rs::input_while_reattaching_fails_and_is_never_queued`.

6. **I do:** during the outage, with the Inbox card for `claude-s` on screen, press Allow.
   **I see:** the answer is refused. The message says the request is held on the machine, which is not linked to this Mac right now, so nothing was sent, and says to reconnect and answer again, or to answer on the machine with `humata login` and the approval passphrase. The card stays Pending.
   **State and events:** the host does not queue the answer. It fails with machine offline, and the hook on the VM keeps holding.
   **Status:** Works (real daemon), unit level: `daemon/humatad/src/inbox.rs::an_answer_to_a_machine_whose_link_dropped_fails_at_once_as_machine_offline` and `::no_verdict_in_time_is_machine_offline_and_the_request_stays_pending`. The app's card in this state: Code, no test. A card that says "unreachable" before I press Allow: Not built, no milestone. The refusal only comes after the press.

7. **I do:** wait. I press nothing. The daemon redials.
   **I see:** the machine returns online. The three sessions show live again. Each tab drops its lost banner and shows the same prompt.
   **State and events:** each tab re-attaches to the same holder from its last offset. No second agent starts.
   **Status:** Works (real daemon), fake `ssh`: the same two daemon tests as step 3 assert the same record, the same holder pid, and a Disconnected then Reconnected notice. Real shell, component only: `roaming_tests.rs` waits for at least five failed dials, then re-attaches. The back-off is 1 to 30 seconds (`terminal/lab/HANDOFF.md`), so allow up to a minute. A Reconnect now button is Not built (HO-6).

8. **I do:** read the end of `copilot-s`'s tab.
   **I see:** the lines it printed during the outage appear once each, in order, after the last line I wrote down. Its turn ended, and the session shows Idle.
   **State and events:** the replica gets only output newer than its cursor. The turn end happened on the VM while the host was away, and the host learns it after reconnect.
   **Status:** Output once: Works (real daemon), `daemon/humatad/tests/terminal.rs::reattaching_from_the_cursor_sends_only_newer_output` and the printed-meanwhile check in `link_drop.rs`. The turn end learned after reconnect: Code, no test. I searched `daemon/humatad/tests/turns.rs` and `remote.rs`: the turn tests run on one daemon, and none drives a hook report on a machine while its link is down. A "caught up N lines" line over the relayed path is unproven (real/03 step 7).

9. **I do:** look at `claude-s`'s card in the Inbox, then press Allow.
   **I see:** the card is still there and answerable. After Allow, Claude runs the command in its own tab, once.
   **State and events:** the answer travels host to machine as one relayed answer. The machine settles the card and the hook prints Allow once. A second press or a repeat after reconnect changes nothing.
   **Status:** Works (real daemon), fake agent: `daemon/humatad/tests/away_replay.rs::an_answer_relayed_by_the_host_settles_the_request_on_the_machine` and `::a_relay_for_a_request_the_machine_already_answered_changes_nothing`; hook side `daemon/humatad/tests/hooks.rs::an_inbox_approval_reaches_the_waiting_hook` and `::the_same_ask_held_twice_is_one_card_and_one_answer_reaches_both`. A hook held across a real link cut and answered from the host: Code, no test.

10. **I do:** in `codex-s`, type "In the hello/ folder, add the line `After the cut.` to README.md" with `humata session type`, which presses Enter.
    **I see:** Codex answers once. Nothing typed during the outage appears.
    **State and events:** keyed input types once, and a repeat types nothing.
    **Status:** Works (real daemon): `daemon/holder/tests/keyed_input.rs::a_keyed_input_types_once_and_a_repeat_types_nothing`. Fixed (B12, D754): `humata session type` presses Enter by default, so no separate Return is sent. Codex's real answer on the VM: Code, no test.

11. **I do:** compare each session's conversation id with the list from the start, and read each conversation.
    **I see:** the same three ids. The history has each turn once. No fresh conversation was made.
    **State and events:** a link drop never runs a resume. The session records are unchanged.
    **Status:** Works (real daemon), fake agent: the one-record check in `remote.rs::a_machine_that_goes_away_and_returns_reattaches_its_session_without_starting_it_twice`. Reading the conversation: Likely bug (seen live, B3/B4): `humata session read` was empty for Copilot. Codex: Code, no test (B9: its real id is never learned).

## Branches

### The cut comes while `copilot-s` is still printing
When: I cut at the first line of the turn. The turn ends on the VM, and I look after reconnect (step 8).
**Status:** Code, no test for the host learning the turn end. The output half is Works (real daemon), as in step 8.

### The Mac sleeps
When: I close the lid for five minutes with the agents running, then open it.
I see: on wake, the machine shows Away, then online by itself, and steps 7 to 11 hold.
**Status:** Code, no test. Needs the owner at the Mac, and **Needs a free screen**. The redial is covered only by the fake-`ssh` tests above.

### Wi-Fi changes
When: this Mac moves to another network, so its address changes. The old connection hangs and never reports a close.
I see: the link is lost once the SSH keepalive runs out, then it re-attaches on the new route.
**Status:** Works (real shell), component only: `roaming_tests.rs::ssh_outage_and_route_change_reattach_existing_holder_without_input_replay` reroutes an owned proxy. Through the app: Code, no test.

### Attach is refused during a resume
When: the link comes back while Humata is resuming a session, and I open the tab.
I see: the attach is refused while the agent resumes.
**Status:** Likely bug (seen live, B10). It was seen on Copilot after a stop and resume on this Mac, not after a link cut. Confirm by cutting the link while a session is resuming, then opening its tab.

### The link stays down
When: I leave it down for ten minutes.
I see: the sessions stay offline. Nothing restarts them. The hook on the VM keeps waiting, up to an hour, then prints nothing and Claude shows its own prompt.
**Status:** Works (real daemon) for the sessions staying offline and not starting twice (`remote.rs` test above). The hook's one-hour end: Code, no test.

### The answer is sent, and the link drops before the machine confirms
When: I press Allow at the moment the link drops.
I see: a refusal saying the answer was sent but not confirmed, and that the card shows how it ended once the machine is linked.
**Status:** Works (real daemon), unit level: `daemon/humatad/src/inbox.rs::a_link_that_drops_while_waiting_ends_the_wait_at_once` and `::a_timed_out_answer_leaves_no_record_so_a_second_answer_after_the_machine_returns_works`.

### An agent's own call to the host is waiting when the link drops
When: `claude-s` waits on a request the Mac holds, and the link drops.
I see: the agent's call ends as host unreachable and retryable. The request is still on the host.
**Status:** Works (real daemon): `daemon/humatad/tests/away_forward.rs::a_link_that_drops_while_the_agent_waits_is_host_unreachable_and_the_wait_can_be_made_again`.

### The answer is given on the VM while the Mac is away
When: I log in on the VM with `humata login` and the approval passphrase and answer there.
I see: after reconnect the Mac shows the card settled, in order, once.
**Status:** Works (real daemon): `daemon/humatad/tests/away_inbox.rs::a_request_answered_on_the_machine_while_away_reaches_the_host_in_order` and `::entries_the_host_never_acked_are_sent_again_after_a_restart`.

## Expected outcomes

- [ ] Cutting the link leaves all three agents running on the VM, with the same process ids.
- [ ] While down, the sessions show offline. Stop, start and typing are refused, not queued, and no second session or agent starts.
- [ ] The link returns without a button press, and each tab re-attaches to the same holder.
- [ ] Output printed during the outage shows once, in order, after catch-up.
- [ ] `copilot-s`'s turn end, which happened during the outage, shows after reconnect.
- [ ] An answer given while down is refused as machine offline, with a fix, and the card stays Pending.
- [ ] After reconnect, one answer reaches the held hook once. A repeat changes nothing.
- [ ] Conversation ids are unchanged and no history is repeated or lost.
- [ ] No step touched the shared VM's power or network, or the owner's own agent homes.

## Status summary

| Step or branch | Status | Evidence |
| --- | --- | --- |
| 1. Three sessions on the VM | Works (real daemon), fake agent; real CLI on the VM Code, no test | `daemon/humatad/tests/remote.rs::a_session_started_on_the_machine_runs_there_and_the_host_relays_its_terminal` |
| 2. Claude's ask, Copilot mid-turn | Works (real daemon) for the hold; card on host Code, no test | `daemon/humatad/tests/hooks.rs::the_daemon_writes_a_blocking_claude_hook_that_holds_for_an_hour` |
| 3. Link cut, sessions offline | Works (real daemon), fake ssh | `daemon/humatad/tests/two_machines.rs::a_lost_link_comes_back_and_the_session_is_the_same` |
| 4. Agents keep running | Works (real shell), component only; agents Works (real daemon), fake agent | `terminal/host/src/roaming_tests.rs::ssh_outage_and_route_change_reattach_existing_holder_without_input_replay` |
| 5. Stop, start, typing refused | Works (real daemon) | `daemon/humatad/tests/remote.rs::a_machine_that_goes_away_and_returns_reattaches_its_session_without_starting_it_twice` |
| 6. Allow while down is refused | Works (real daemon), unit level | `daemon/humatad/src/inbox.rs::an_answer_to_a_machine_whose_link_dropped_fails_at_once_as_machine_offline` |
| 7. Link returns, same holders | Works (real daemon), fake ssh | same tests as step 3 |
| 8. Output once; turn end after reconnect | Output Works (real daemon); turn end Code, no test | `daemon/humatad/tests/terminal.rs::reattaching_from_the_cursor_sends_only_newer_output` |
| 9. Held ask answered once | Works (real daemon), fake agent; across a real cut Code, no test | `daemon/humatad/tests/away_replay.rs::a_relay_for_a_request_the_machine_already_answered_changes_nothing` |
| 10. Typing after reconnect | Works (real daemon); Fixed (B12, D754): `humata session type` presses Enter by default | `daemon/holder/tests/keyed_input.rs::a_keyed_input_types_once_and_a_repeat_types_nothing` |
| 11. Same conversation ids | Works (real daemon), fake agent; Likely bug (seen live, B3/B4) | `remote.rs` same test |
| Branch: cut mid-turn | Code, no test | none |
| Branch: Mac sleeps | Code, no test | none |
| Branch: Wi-Fi changes | Works (real shell), component only | `roaming_tests.rs` |
| Branch: attach refused while resuming | Likely bug (seen live, B10) | REALFLOW run |
| Branch: link stays down | Works (real daemon); hook's hour end Code, no test | `remote.rs` same test |
| Branch: answer sent, no confirm | Works (real daemon), unit level | `daemon/humatad/src/inbox.rs::a_timed_out_answer_leaves_no_record_so_a_second_answer_after_the_machine_returns_works` |
| Branch: agent call waits on the host | Works (real daemon) | `daemon/humatad/tests/away_forward.rs::a_link_that_drops_while_the_agent_waits_is_host_unreachable_and_the_wait_can_be_made_again` |
| Branch: answered on the VM | Works (real daemon) | `daemon/humatad/tests/away_inbox.rs::a_request_answered_on_the_machine_while_away_reaches_the_host_in_order` |

## Gaps and likely bugs

Not built:
- A card that says "unreachable" before I press Allow, with the answer disabled (no milestone). Today the press is refused with a fix.
- A queue for an answer given while the machine is unreachable (none by design: the refusal tells me to answer again).
- A Reconnect now button (HO-6).
- No test cuts the app's own path (app, `humatad2`, `ssh`, VM holder) under a real agent CLI. No milestone owns it.

Likely bugs:
- B10: attach refused while a session resumes. Confirm as in the branch above.
- Fixed (B12, D754): `humata session type` presses Enter by default, so step 10 needs no separate Return.
- B3/B4: `humata session read` is empty for Copilot, so step 11 can't read the history that way. Codex's real session id is never learned (B9), so its conversation can't be named.
- Unconfirmed: no test shows the host learning a turn end, or a hook-held card, that happened on a machine while its link was down. The code journals Inbox changes, but the turn path is not covered.

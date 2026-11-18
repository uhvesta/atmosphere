# `humata attach` picks the agent's tab, and an attach made during a resume waits for it

Layer: macro (the CLI, as the person runs it). Platforms: macOS (this Mac). Linux (`box-1`) only for the remote variant, which is not covered here.

## Starting state

- Humata 2 runs on this Mac with a throwaway `--root` and its own `HOME`, so nothing touches the owner's agent logins or real folders.
- The channel `acme/api` has one session, `Fix login` (`se_1`). Its agent is a fake agent (a script that prints `agent up` and reads lines), not a real agent CLI.
- `Fix login` has two tabs: its agent tab (`tb_agent`) and one shell tab (`tb_shell`). Its status is Working.
- No attach is open. The app is not running, so nothing is focused.
- The daemon's test fixture for this is `daemon/humatad/tests/sessions.rs` (`Fixture::new`, which waits for Working before it returns).

## Reproduce headless

- Harness: a `daemon/humatad/tests/*.rs` target for the daemon side (`sessions.rs`, which already has the fake agent, `agent_tab`, `wait_status` and `kill_holders`), and the `humata` binary against that daemon for the CLI side. `cli/tests/hosts_live.rs` shows the pattern: `env_clear`, `PATH=/usr/bin:/bin`, `HOME` set to a temp folder, `HUMATA_SOCKET` pointing at the throwaway daemon, and `binary("HUMATA")`.
- Commands to run, against the throwaway daemon only:
  1. `humata --json session tabs se_1` and keep the output. Expect two tabs, one `kind` agent and one `kind` shell.
  2. `humata session stop se_1`, then `humata session resume se_1` (this returns an operation id at once), then `humata attach se_1` straight away. Keep the exit code and the first lines of output.
  3. Repeat step 2 with `humata attach se_1 --tab tb_agent`.
- Evidence to keep: the JSON of step 1, the exit code and stderr of each attach, and the `attach.wait_for("agent up --resume")` result from the daemon test.
- `humata attach` needs an interactive keyboard. When stdin is not a terminal the CLI may refuse before it attaches. If it does, the side you can prove headless is the daemon's `TerminalService.Attach` through `daemon/humatad/tests/common` `Attach::start` (the same RPC the CLI calls after it picks the tab), with the tab id the CLI would have picked.
- Not possible headless: seeing the agent's real screen in a window, and a real agent CLI's startup. The nearest equivalent is the fake agent's `agent up --resume` line.

## Steps

1. **I do:** I run `humata --json session tabs se_1`.

   **I see:** JSON with `tabs` listing `tb_agent` (kind agent) and `tb_shell` (kind shell), each with its id.

   **State and events:** The CLI sends `ListTabs` with the session id. Nothing is typed and nothing is attached.

   **Status:** Fakes only. `cli/tests/sessions.rs::session_tabs_prints_the_tabs` checks the printed ids against a fake daemon. The daemon's own `ListTabs` for a local session is not asserted by a test that names the kinds.

2. **I do:** I run `humata attach se_1`, with no `--tab`, on a Working session.

   **I see:** `Attached. Ctrl-] detaches.` on stderr, then the agent tab's screen. The shell tab's screen does not appear.

   **State and events:** The CLI calls `ListTabs`, then `default_tab` picks the agent tab over the shells (`cli/src/commands/sessions.rs:43-62`). It opens `TerminalService.Attach` with `tab` set to `tb_agent` and `read_only` false.

   **Status:** Fakes only. `cli/tests/sessions.rs::attach_without_a_tab_attaches_when_the_session_has_an_agent_tab` only checks that `ATTACH` was called, not which tab it named. The choice itself is checked by the pure unit test `cli/src/commands/sessions.rs::the_agent_tab_is_chosen_over_shells`. No real-daemon test attaches without `--tab`.

3. **I do:** I run `humata session stop se_1`, then `humata session resume se_1`, then `humata attach se_1` at once, before the session says Working.

   **I see:** `stop` returns. `resume` returns an operation id (it runs in the background, `session_service.rs` around line 1090). The attach either shows the agent's screen after a short wait, with the banner `[Humata] Resuming the ... conversation`, or it exits with an error. It must not show the shell tab.

   **State and events:** The record is Resuming. The old agent tab is still in the store, with the path of its holder socket. The new holder starts before verify, and the tab is saved with the new holder only after verify passes (`daemon/revive/src/agent.rs` around lines 300-313, `running()`). The attach's holder link waits for that save for at most 5 s (`daemon/humatad/src/holders.rs:483-520`, `SAVE_WAIT`, using the store's commit notice). Then it links and attaches. A link that is still not saved with the new holder is refused with `NotSaved`.

   **Status:** Fakes only. The wait is `daemon/humatad/src/holders.rs::a_link_waits_for_the_tab_to_be_saved_with_the_new_holder` (line 996), which drives the generic `wait_until` with a fake saved tab and a commit channel. No test attaches to a real session during a real resume. The resume itself is proven: `daemon/humatad/tests/sessions.rs::stopping_ends_the_agent_and_resuming_brings_it_back_in_its_conversation`.

4. **I do:** I wait until `humata --json session show se_1` says Working, then run `humata attach se_1`.

   **I see:** The agent's screen, with the resumed conversation. The `Fix login` conversation still holds what it had before the stop.

   **State and events:** Same as step 2, now with no wait. The attach names the agent tab, and the holder it names is the one that answers.

   **Status:** Works (real daemon). `daemon/humatad/tests/sessions.rs::a_lost_holder_is_resumed_in_its_conversation_after_a_restart` attaches after the status is Working and waits for `agent up --resume` on the screen. That test uses an interrupted resume, not a user's `session resume`, and it attaches through the daemon's `Attach` RPC, not the CLI's tab choice.

5. **I do:** I read the agent's conversation after a user-asked resume (step 3).

   **I see:** The agent is not told it was interrupted. The conversation gets no `[Humata] This terminal stopped` note. The terminal shows the `Resuming the ... conversation` banner only.

   **State and events:** For an asked resume the record gets no interruption (`daemon/revive/src/agent.rs` `Why::Asked`, and `tell` is false at line 302, so `verify` writes no note).

   **Status:** Code, no test. No test checks the absence of the note on a user's resume. The interrupted case checks the note is there (`sessions.rs::a_lost_holder_is_resumed...`, which asserts the conversation contains `one`, not the note).

## Branches

### The resume fails while the attach waits

**When:** During step 3, the agent does not verify (its conversation is not taken up), so the record goes to ResumeFailed and the tab is never saved with the new holder.

**I do:** Run `humata attach se_1` during the resume, as in step 3, with a fake agent set to fail its verify.

**I see:** The attach waits about 5 s, then exits. The error is not a not_found or a precondition. It is shown as an internal error about the holder answering at the socket "isn't the one its tab names", with the fix "Try again in a moment; if it keeps happening, restart humatad2".

**Expected:** The attach ends with an error that says the agent did not resume, and what to do (look at the session's status, then resume again or start fresh). It never attaches to the old holder, and it never picks a shell.

**Status:** Likely bug. `daemon/humatad/src/attach.rs:940-955` (`holder_status`) sends every `HolderError` other than Connect and Lost to `internal` (exit 70). `NotSaved` falls there. Its text (`daemon/holder/src/error.rs:58`) describes a fresh holder, not a failed resume, so the fix is wrong here. To confirm: make the fake agent fail verify, run the attach during the resume, and read the exit code and `error.kind`.

### The attach names its tab with `--tab`

**When:** During a resume, the person runs `humata attach se_1 --tab tb_agent`.

**I do:** Run the command as in step 3.

**I see:** The same wait as step 3, with no `ListTabs` call first. Then the agent's screen.

**Expected:** The same as step 3. `--tab` skips the lookup, not the wait.

**Status:** Fakes only. The wait is the holder link's unit test (see step 3). The CLI skips `ListTabs` when `--tab` is given (`cli/src/commands/sessions.rs:17-19`), and no test covers that path with a real daemon.

### Two shells and no agent tab

**When:** The session has tb_shell and a second shell tab, and no agent tab (for example, its agent tab was removed).

**I do:** Run `humata attach se_1`.

**I see:** Exit 7, error kind `no_agent_tab`. The message lists `tb_shell` and the other id, and tells the person to pick one with `--tab`.

**Expected:** Nothing is attached. The person chooses a tab.

**Status:** Fakes only. `cli/tests/sessions.rs::attach_without_a_tab_stops_when_the_tab_is_ambiguous` (fake daemon, exit 7, no ATTACH call).

### One shell tab and no agent tab yet

**When:** The session has exactly one shell tab and no agent tab in the store. The agent tab is saved only when its holder starts (`daemon/revive/src/agent.rs` around line 139), so this is a short window during a first launch, or a session whose agent tab was never saved.

**I do:** Run `humata attach se_1` with no `--tab`.

**I see:** The attach opens the shell tab, with no error. The person asked for the agent.

**Expected:** The documented rule (CLI.md line 99) is that a session with no agent tab opens its only tab. The person should be told which tab they got, or the attach should wait for the agent tab. Silently opening the shell is not what the person asked for.

**Status:** Likely bug, to confirm. `cli/src/commands/sessions.rs:43-62` (`default_tab`) gives `(None, [only])` the only tab with no check of its kind. It does not wait for an agent tab that is still launching. To confirm: read `session tabs` while a fresh launch is in progress, with one shell open.

### Attach to a stopped session, without a resume

**When:** `Fix login` is Stopped (step 3 without the resume), so the agent tab stays in the store with a holder that is gone.

**I do:** Run `humata attach se_1`.

**I see:** An error of kind `not_found` for the tab, saying its shell isn't running any more, and the fix says to close the tab and open a new one.

**Expected:** The attach is refused. The fix tells the person to resume the session, which brings the agent back in its conversation. Closing the tab is the wrong fix here.

**Status:** Likely bug. `daemon/humatad/src/attach.rs:940-948` maps `HolderError::Connect` to `not_found` with "close the tab and open a new one; what it showed can't be recovered". For a Stopped session the tab is not gone, and the agent can be resumed. To confirm: stop the session and attach; read the error text.

### The daemon is killed while the session resumes

**When:** The daemon is killed while `Fix login` is Resuming, and started again.

**I do:** Restart `humatad2` during the resume, then run `humata attach se_1`.

**I see:** The resume finishes after the restart, and the attach shows the agent's screen once the tab is saved with the new holder.

**Expected:** The same result as step 3, with no wrong tab.

**Status:** Code, no test for the attach part. `daemon/revive/tests/kill_matrix.rs::a_daemon_killed_while_resuming_finishes_the_resume_when_it_starts_again` (line 462) proves the resume finishes. No test attaches while it does.

## Expected outcomes

- [ ] `session tabs` lists the agent tab and each shell tab, with their ids and kinds.
- [ ] `attach` without `--tab` opens the agent tab, not a shell tab, when the session has an agent tab.
- [ ] An attach made while the session is resuming waits for the agent tab to be saved with its new holder, then attaches. It does not refuse at once.
- [ ] An attach made during a resume that fails refuses with a message that says the agent did not resume, and does not suggest closing the tab.
- [ ] An attach to a Stopped session is refused, and the fix tells the person to resume it.
- [ ] A session with two shells and no agent tab stops with exit 7 and lists the tabs.
- [ ] A user's resume records no interruption and writes no note into the conversation.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. `session tabs` lists agent and shell | Fakes only | `cli/tests/sessions.rs::session_tabs_prints_the_tabs` |
| 2. `attach` without `--tab` picks the agent tab | Fakes only | `cli/tests/sessions.rs::attach_without_a_tab_attaches_when_the_session_has_an_agent_tab`, `cli/src/commands/sessions.rs::the_agent_tab_is_chosen_over_shells` |
| 3. Attach during resume waits, then attaches | Fakes only | `daemon/humatad/src/holders.rs::a_link_waits_for_the_tab_to_be_saved_with_the_new_holder` |
| 4. Attach after Working shows the agent | Works (real daemon) | `daemon/humatad/tests/sessions.rs::a_lost_holder_is_resumed_in_its_conversation_after_a_restart` |
| 5. User's resume writes no note | Code, no test | `daemon/revive/src/agent.rs:302` |
| Branch: resume fails during attach | Likely bug | `daemon/humatad/src/attach.rs:940-955` |
| Branch: `--tab` during resume | Fakes only | `cli/src/commands/sessions.rs:17-19`, holder link unit test |
| Branch: two shells, no agent tab | Fakes only | `cli/tests/sessions.rs::attach_without_a_tab_stops_when_the_tab_is_ambiguous` |
| Branch: one shell, no agent tab | Likely bug | `cli/src/commands/sessions.rs:43-62` |
| Branch: attach to Stopped session | Likely bug | `daemon/humatad/src/attach.rs:940-948` |
| Branch: daemon killed mid-resume | Code, no test | `daemon/revive/tests/kill_matrix.rs::a_daemon_killed_while_resuming_finishes_the_resume_when_it_starts_again` (resume only) |

## Gaps and likely bugs

- Not built: nothing. The look-again is built as a notice-driven wait (D672 ruling), in `holders.rs` `Link::start`.
- Likely bug: `attach::holder_status` (`daemon/humatad/src/attach.rs:940-955`) maps `NotSaved` to internal, exit 70. A failed resume reads as a transient internal fault, with a fix that says to restart humatad2. To confirm: step 3 with a failing verify.
- Likely bug: `attach::holder_status` (`daemon/humatad/src/attach.rs:940-948`) tells the person to close the tab for a Stopped session, when resuming is the fix. To confirm: attach to a Stopped session and read the message.
- Likely bug: `cli/src/commands/sessions.rs:43-62` attaches to the only tab when there is no agent tab yet, so a launching session can open a shell. To confirm: `session tabs` during a fresh launch.
- Gap: no real-daemon test attaches during a real resume. The only proof of the wait is a unit test of the generic wait with a fake saved tab. To close it, drive `session stop`, `session resume` and `attach` against the fake agent and assert the attach names the new holder's screen.
- Gap: `cli/tests/sessions.rs::attach_without_a_tab_attaches_when_the_session_has_an_agent_tab` does not assert which tab was sent. It would still pass if the CLI sent a shell.
- Gap: no test that a user's resume writes no note (D671).

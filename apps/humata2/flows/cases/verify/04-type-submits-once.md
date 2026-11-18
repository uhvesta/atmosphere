# `humata session type` submits its text once, including after a retry and right after a resume

Layer: macro (the CLI, as the person runs it). Platforms: macOS (this Mac). Linux (`box-1`) is not covered here.

## Starting state

- Humata 2 runs on this Mac with a throwaway `--root` and its own `HOME`.
- The session `Review docs` (`se_1`) has one shell tab (`tb_shell`) running a real `sh`, at a prompt. Nothing is typed into it yet. The session's agent is `sh`, as in the daemon's typing fixture.
- The folder of the session holds an empty file `marks`. Each command in this case appends a letter to it, so a count of letters is a count of runs.
- The person's shell has the throwaway `HOME` and `HUMATA_SOCKET`. No app window is open.

## Reproduce headless

- Harness: `daemon/humatad/tests/terminal.rs`. Its `session_shell()` fixture starts a real daemon with a real `sh` shell tab. `type_keyed`, `add_a_mark` and `ran_up_to_now` are the helpers for the marks file. The CLI side runs the `humata` binary against the same throwaway daemon, with the pattern of `cli/tests/hosts_live.rs` (`env_clear`, `PATH=/usr/bin:/bin`, `HOME` in a temp folder, `HUMATA_SOCKET`).
- Commands to run:
  1. `humata --json session type se_1 $'printf x >> marks\n'`. Keep the JSON and the exit code. Then read `marks`.
  2. `humata --json session type se_1 $'printf y >> marks' --no-submit`. Then read `marks`.
  3. Stop the daemon during a keyed type with the fault point `terminal.type.held` or `terminal.type.typed` (`HUMATA2_STOP_AT`, as in the daemon test), then re-run with the same `--key`.
  4. `humata session stop se_1`, then `humata session resume se_1`, then `humata --json --key k2 session type se_1 $'printf r >> marks\n'` straight away. Read `marks` and the exit code.
- Evidence to keep: the JSON replies, exit codes, the content of `marks` after each step, and the time between the body write and the return write (a timestamp from the holder log at `logs/<tab>.log`, if the holder logs it; otherwise say it was not seen).
- Not possible headless: seeing an agent's TUI take a return as a submit. A `sh` prompt shows only that a line ran. The nearest proof is the `marks` count.

## Steps

1. **I do:** I run `humata --json session type se_1 $'printf x >> marks\n'`. The text ends in a newline, which is the submit.

   **I see:** `typed` and `submitted` are true and the exit code is 0. `marks` holds one `x`, and the shell prompt is back. Without the trailing newline the CLI would add one, as it presses Enter by default (D754).

   **State and events:** The CLI sends `Type` with a key it made for this run. The daemon splits the trailing newline (`daemon/humatad/src/terminal.rs:309-318`, `split_submit`). The body is typed under the key, then 400 ms later the return is sent as its own write under `<key>/submit` (`terminal.rs:893-948`, `then_submit`).

   **Status:** Works (real daemon). `daemon/humatad/tests/terminal.rs::a_keyed_type_retried_after_a_restart_is_not_typed_again` runs a `printf x >>` command with a trailing newline through a real `sh` and asserts the marks file is `x`. `each_text_typed_into_a_session_reaches_its_shell` (line 578) proves the text reaches the shell. Neither counts return presses.

2. **I do:** I count the returns that reached the shell for step 1.

   **I see:** One return, 400 ms after the body.

   **State and events:** Two writes reach the holder: the body, then one `\r` under `<key>/submit`. The return is not sent twice.

   **Status:** Code, no test. The gap is that a stray empty line at a `sh` prompt does not change `marks`. Nothing in the daemon tests counts the return presses. The split and the step decision have unit tests (`terminal.rs` `submit_tests`: `a_trailing_newline_is_split_off_as_the_submit`, `a_retry_whose_return_arrived_sends_nothing`), but those prove the decision, not the writes.

3. **I do:** I run `humata --json session type se_1 $'printf y >> marks' --no-submit`.

   **I see:** `typed` is true and `submitted` is false. `marks` gets no `y` yet, because the command sits at the prompt with no Enter. The person must press Enter or run the command again without `--no-submit`.

   **State and events:** No submit. `--no-submit` sends the text as it is, with no trailing newline, so the body is typed with no return write (`split_submit` returns `submit` false) and the daemon answers `submitted` false.

   **Status:** Works (real daemon). `daemon/humatad/tests/terminal.rs::typing_says_it_submitted_only_text_that_ends_in_a_newline` types text with no newline into a real `sh`, checks `submitted` is false, then shows the line ran only once the newline came. `cli/tests/sessions.rs::type_with_no_submit_types_the_text_as_it_is` checks `--no-submit` sends the text unchanged.

4. **I do:** I stop the daemon during a keyed type after the shell has the text (fault point `terminal.type.typed`), restart it, and run the same command again with the same `--key`.

   **I see:** The first call has no reply, because the daemon died. The retry answers `typed` and `submitted` true. `marks` holds one `x`.

   **State and events:** The key is noted in the store before typing. The retry asks the tab's holder, which typed both the body and the return, so the retry types nothing.

   **Status:** Works (real daemon). `daemon/humatad/tests/terminal.rs::a_keyed_type_cut_short_after_the_shell_has_it_answers_typed` (line 668).

5. **I do:** I stop the daemon before the shell has the text (fault point `terminal.type.held`), restart it, and run the same command with the same `--key`.

   **I see:** The retry types the command once, and `marks` holds one `x`.

   **State and events:** The holder says nothing arrived, so the retry types the body and its return once.

   **Status:** Works (real daemon). `daemon/humatad/tests/terminal.rs::a_keyed_type_cut_short_before_the_shell_has_it_types_on_retry` (line 690).

6. **I do:** I run `humata session type se_1 $'printf z >> marks\n'` twice, each time without `--key`.

   **I see:** Each run types the command and runs it. `marks` gets two `z`s.

   **State and events:** The CLI makes a new key per run (`cli/src/call.rs:82-92`), so the two runs are two intents. This matches `api/CLI.md:62`: "Default: a new key per run".

   **Status:** Code, no test for the outcome. The CLI test `cli/tests/sessions.rs::key_reuses_the_given_idempotency_key` checks that `--key` is sent, using `session stop`, not `session type`.

7. **I do:** I run `humata session stop se_1`, then `humata session resume se_1`, then at once `humata --json --key k2 session type se_1 $'printf r >> marks\n'`.

   **I see:** Expected: the call waits until the agent is back and its conversation is taken up, then types the command once, and `marks` gets one `r`. Or it refuses with nothing typed.

   **State and events:** The resume is in the background. The keyed path does not go through the holder link's wait for the tab to be saved with the new holder (see the branch "Type with a key right after a resume"), so it may reach the new holder before verify passes.

   **Status:** Likely bug, to confirm live. See the branch below and the Gaps list.

8. **I do:** I run `humata session type se_1 "..." --at-turn-end`.

   **I see:** Exit 2, error kind `invalid_field` on `when`, with the message that typing at the end of a turn is not something the daemon does, and that only the app holds text until a turn ends. The fix says to type now or to ask the user with `inbox_ask`, and that updating Humata does not add the mode.

   **State and events:** The CLI sends `When::TurnEnds`. The daemon refuses it in `type_in` (`daemon/humatad/src/terminal.rs:1347-1353`). Nothing is queued.

   **Status:** Not built. SE-23 and D126 make the wait mode optional. `api/CLI.md:120` says `--at-turn-end` queues the text "waiting: turn end not certain", which the daemon does not do. The CLI test `cli/tests/sessions.rs::type_at_turn_end_waits_for_the_turn` checks only the request it sends.

## Branches

### The daemon is stopped between the body and the return

**When:** The body is typed and the holder has it, but the return is not yet sent (between `terminal.type.typed` and the return write). A fault point for this moment does not exist.

**I do:** Stop the daemon there, restart it, and run the same command with the same `--key`.

**I see:** The retry sends only the return. `marks` holds one `x`.

**Expected:** Only the missing return is sent. The body is not typed again.

**Status:** Code, no test. The decision is a pure function with unit tests (`submit_step`, `a_retry_whose_return_did_not_arrive_sends_it`). No daemon test stops the daemon between the two writes, so the write-level behaviour is not proven. The fault points are only `terminal.type.held` and `terminal.type.typed` (`daemon/humatad/src/terminal.rs:1014`, `:1407`).

### The retry comes after a resume (a new holder)

**When:** The first `Type` with key `k1` typed its body and stopped before the return. Then the session was stopped and resumed, so the holder has changed. The retry comes with the same `k1`.

**I do:** Run `humata --json --key k1 session type se_1 $'printf w >> marks\n'` after the resume.

**I see:** An error (`internal`, with the message that an earlier Type call with this key started and did not finish, so some or all of its text may have been typed; nothing was typed again; look at the session's terminal and type what is missing with a new key). `marks` gets no new `w`.

**Expected:** Nothing is typed again, and the person is told to look at the terminal and use a new key. This is the intended refusal (D426): only the holder that held the key can say what arrived.

**Status:** Code, no test. `daemon/humatad/src/terminal.rs:988` (`type_keyed`, the `(_, Some(_))` arm) and `maybe_typed_before` (line 1878). No test covers a retry across a resume.

### A `Type` with a key right after a resume

**When:** Step 7. The keyed path runs during the resume, and the agent's tab is not yet saved with its new holder.

**I do:** Run the keyed type straight after `session resume`.

**I see:** The text may be typed before the agent's conversation is verified. If verify then fails, the resume is stopped, but the Type call may already have answered `typed` true. The person would then see a `typed` reply for text the agent never took.

**Expected:** The same as an un-keyed type: the call waits for the tab to be saved with the new holder (up to 5 s), then types once, or it refuses with nothing typed.

**Status:** Likely bug, to confirm. The keyed path does not use the holder link's wait. `type_keyed` calls `humata2_holder::key_arrived` and `keyed_input` straight on `tab.holder_socket` (`daemon/humatad/src/terminal.rs:979` and `:1017`), and `then_submit` does the same for the return (`:911` and `:936`). Only the plain path (`type_into`, line 841) goes through `self.holders.link`, which waits (`daemon/humatad/src/holders.rs:483-520`). The CLI always sends a key, so every `session type` takes the keyed path. To confirm: resume a session whose verify is slow (for example, a fake agent that waits 3 s before taking its conversation), run the keyed type right after `resume`, and compare the time the text reaches the shell with the time the tab is saved (`attach` or the `session show` status).

### The shell has exited

**When:** The shell tab of `se_1` has run `exit 3`.

**I do:** Run `humata session type se_1 $'printf q >> marks\n'`.

**I see:** An error of kind `not_found`, saying the tab's shell exited before the text reached it, and the fix says to open a new tab and type again. Nothing is typed.

**Expected:** Nothing is typed.

**Status:** Works (real daemon). `daemon/humatad/tests/terminal.rs::typing_into_a_shell_that_exited_is_refused` (line 592).

### The session is moving to another machine

**When:** The session is being moved (MV-4) when the person types.

**I do:** Run `humata session type se_1 ...` during the move.

**I see:** The call is refused before it reaches any box, and nothing is typed. After the agent resumes on the new machine, the same text typed with a new key goes through once.

**Expected:** Nothing typed while the session moves. Typed once afterwards.

**Status:** Works (real daemon). `daemon/humatad/tests/move_typing.rs::typing_into_a_moving_session_is_refused_and_works_once_it_resumes_on_the_new_machine` (line 91).

### The person types with no key, right after a resume

**When:** No `--key` is passed, which the CLI does not do, but the MCP or another client may send a Type without a key.

**I do:** Send a Type with no key right after the resume.

**I see:** The text goes through the holder link, which waits for the tab to be saved with the new holder.

**Expected:** Typed once, after the wait.

**Status:** Code, no test. `type_into` (`daemon/humatad/src/terminal.rs:841`) uses `holders.link`. The wait itself has only a fake-based unit test (`holders.rs::a_link_waits_for_the_tab_to_be_saved_with_the_new_holder`).

## Expected outcomes

- [ ] A command typed with a trailing newline runs once in a real shell.
- [ ] Exactly one return is pressed, after the body, and is not sent twice.
- [ ] Text sent with `--no-submit` is typed and not submitted, and the reply says `submitted` false.
- [ ] A retry with the same key after the daemon stopped once the shell had the text types nothing.
- [ ] A retry with the same key after the daemon stopped before the shell had the text types the text once.
- [ ] Two runs without `--key` are two intents, and each is typed (the person must pass `--key` to retry safely).
- [ ] A retry that cannot be sure what arrived (a new holder after a resume) types nothing and says so.
- [ ] A keyed type right after a resume waits until the tab is saved with the new holder, or refuses with nothing typed.
- [ ] Typing into an exited shell, or into a moving session, types nothing.
- [ ] `--at-turn-end` is refused as not built, with exit 2.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Trailing newline runs once | Works (real daemon) | `daemon/humatad/tests/terminal.rs::a_keyed_type_retried_after_a_restart_is_not_typed_again` |
| 2. Exactly one return pressed | Code, no test | `terminal.rs` `submit_tests` (unit only) |
| 3. `--no-submit`, no submit | Works (real daemon) | `daemon/humatad/tests/terminal.rs::typing_says_it_submitted_only_text_that_ends_in_a_newline`; `cli/tests/sessions.rs::type_with_no_submit_types_the_text_as_it_is` |
| 4. Retry after cut, shell had it | Works (real daemon) | `daemon/humatad/tests/terminal.rs::a_keyed_type_cut_short_after_the_shell_has_it_answers_typed` |
| 5. Retry after cut, shell did not have it | Works (real daemon) | `daemon/humatad/tests/terminal.rs::a_keyed_type_cut_short_before_the_shell_has_it_types_on_retry` |
| 6. Two runs without `--key` | Code, no test | `cli/src/call.rs:82-92`, `api/CLI.md:62` |
| 7. Keyed type right after resume | Likely bug | `daemon/humatad/src/terminal.rs:979`, `:1017`, `:911`, `:936` |
| 8. `--at-turn-end` | Not built | `daemon/humatad/src/terminal.rs:1347-1353`, SE-23, D126 |
| Branch: stop between body and return | Code, no test | `terminal.rs` `submit_step` unit tests; fault points at `:1014`, `:1407` |
| Branch: retry after a resume (new holder) | Code, no test | `terminal.rs:988`, `maybe_typed_before` at `:1878` |
| Branch: keyed type right after resume | Likely bug | as step 7 |
| Branch: shell exited | Works (real daemon) | `daemon/humatad/tests/terminal.rs::typing_into_a_shell_that_exited_is_refused` |
| Branch: session moving | Works (real daemon) | `daemon/humatad/tests/move_typing.rs::typing_into_a_moving_session_is_refused_and_works_once_it_resumes_on_the_new_machine` |
| Branch: no key, right after resume | Code, no test | `terminal.rs:841` (`type_into`) |

## Gaps and likely bugs

- Not built: `--at-turn-end` (`session type --at-turn-end`). The daemon refuses it (`daemon/humatad/src/terminal.rs:1347-1353`), though `api/CLI.md:120` says it queues. SE-23 and D126 (optional). No milestone.
- Not built: the CLI's queue commands, `ListQueued`, `CancelQueuedTyping` and `SendQueuedNow`, answer `not_built` for a local session (`daemon/humatad/src/terminal.rs`, the handlers at lines 1690-1710). Only the remote path works.
- Not built: a check that the agent is ready for input before typing. D671 gives a fixed 400 ms pause, marked Provisional, and nothing else. A TUI that is still starting can swallow the return.
- Likely bug: the keyed path skips the holder link's wait for a resumed tab (`daemon/humatad/src/terminal.rs:979`, `:1017`, `:911`, `:936` against `:841` and `daemon/humatad/src/holders.rs:483-520`). A `session type` right after `session resume` may type before verify passes, and report `typed` true for text the agent never took. To confirm: the slow-verify run in the branch above.
- Gap: no test counts return presses. The marks-file tests prove a command ran once, but a stray empty line at a `sh` prompt is invisible to them. A double submit to a TUI would not show up. To close it, the test should read the bytes the holder receives (`keyed_input` in `daemon/holder`) and count `\r`.
- Gap: no daemon test stops between the body and the return, so the `<key>/submit` step is only proven by unit tests of the decision.

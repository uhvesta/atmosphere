# A message queued for a turn end waits, then goes once the turn ends

Layer: agents. Platforms: macOS, Linux, web.

## Starting state

- Humata 2 is open on this Mac. Session `ledger` runs Claude Code on this Mac and is in the Working state.
- Session `brin` runs another agent (agent B) on this Mac. Agent B holds its own session token and can call the Humata MCP tools.
- In `ledger`, the composer is empty and the queue bar is hidden. Nothing is held.
- The app has the `ledger` page open with the agent tab focused.
- The daemon (`humatad2`) runs on this Mac. The app and the daemon are connected.

## Steps

1. **I do:** Type `After the tests pass, update the changelog.` in the `ledger` composer and press Return.

   **I see:** The composer clears. A bar above the composer reads "1 message waiting to send". The transcript gets no new line.

   **State and events:** The app adds one text entry to the `ledger` queue. No `Type` request leaves the app, because the turn is Running (`features/session/src/lib.rs:1195`, then `flush` at `:663`, which sends only when the turn is Ended). The queue is a structural change, so the app saves it at once (`features/app/src/saved.rs`).

   **Status:** Fakes only. `features/session/tests/queue.rs::a_hook_turn_end_delivers_waiting_messages_but_running_and_uncertain_do_not` shows that a Running Submit does not send. The save timing has no test (see the restart branch).

2. **I do:** Nothing. The agent finishes its turn, and Claude's Stop hook runs `humata turn idle`.

   **I see:** The `ledger` status in the sidebar changes from Working to Idle.

   **State and events:** The daemon records the hook report through `SessionService.ReportTurn` and publishes a Session event. Only the caller's own session can report its turn (D169).

   **Status:** Works (real daemon). `daemon/humatad/tests/turns.rs::the_hooks_move_a_session_between_idle_working_and_needing_input`.

3. **I do:** Nothing. The app gets the Idle snapshot.

   **I see:** The text `After the tests pass, update the changelog.` appears in the `ledger` transcript as typed input, and the bar goes away.

   **State and events:** `clients/live/src/map.rs:289-292` maps Idle to `Turn::Ended`. `flush` (`features/session/src/lib.rs:673`) sends the first queued text for a Claude session at Ended. The app calls `send` (`clients/live/src/daemon.rs:702`), which sends one `Type` request with `When::Now`, the text, a `\r`, and a new idempotency key (`connection.rs:323`). On `Ok`, the `Sent` handler removes the entry (`lib.rs:1253`).

   **Status:** Fakes only. The state change is proved by `features/session/tests/queue.rs::a_hook_turn_end_delivers_waiting_messages_but_running_and_uncertain_do_not`. The live `send` path has no test. The daemon side of typing is covered by `daemon/humatad/tests/terminal.rs::each_text_typed_into_a_session_reaches_its_shell`, which uses a real `sh`.

4. **I do:** Agent B calls the MCP tool `session_type` with `session: "ledger"`, `text: "Also run the linter."`, and `when: "turn_end"`.

   **I see:** Agent B gets an error that says the turn-end typing "isn't built into this humatad2 yet". Nothing is typed into `ledger`.

   **State and events:** The daemon's `Type` handler returns `not_built` before it types anything (`daemon/humatad/src/terminal.rs:911-916`). No queue entry is made on the daemon. The MCP tool passes `when` through as `When::TurnEnds` (`daemon/humatad/src/mcp.rs:985-988`).

   **Status:** Not built (no milestone; D169 defers it: "`When::TurnEnds` typing (REVIEW, D126) gets nothing yet"). No test pins this refusal.

5. **I do:** Agent B calls `session_type` again with the same session and text, and `when: "now"`.

   **I see:** The result is `typed: true`, `queued_id: null` and `turn_end_uncertain: false`.

   **State and events:** The text is typed at once. The MCP result always has `queued_id` null and `turn_end_uncertain` false (`daemon/humatad/src/mcp.rs:1005-1009`).

   **Status:** Works (real daemon). `daemon/humatad/tests/mcp.rs::an_agent_types_into_its_own_session_through_mcp` checks `typed` and the `applied` outcome. It does not check `queued_id`.

6. **I do:** In a shell, run `humata session type ledger "Run the tests." --at-turn-end`.

   **I see:** The CLI prints the daemon's "isn't built" message and exits 7.

   **State and events:** The CLI sends `When::TurnEnds` (`cli/src/commands/sessions.rs:320-325`). The daemon refuses it as in step 4. Exit 7 comes from `cli/src/exit.rs:98` (Unimplemented maps to PRECONDITION).

   **Status:** Not built (no milestone; D169). `cli/tests/sessions.rs::type_at_turn_end_waits_for_the_turn` runs against a fake daemon and checks only that the request carries `When::TurnEnds`. Its name says it waits, but it does not check that anything waits. Fakes only.

7. **I do:** In a shell, run `humata session queue ledger`.

   **I see:** An error. The waiting messages are not listed.

   **State and events:** The CLI sends `ListQueued` (`cli/src/commands/sessions.rs:73`). The daemon answers `not_built` for a local session (`daemon/humatad/src/terminal.rs:1198-1207`). The waiting messages exist only in the app. Exit 7.

   **Status:** Not built (no milestone). No CLI or daemon test covers `session queue`.

8. **I do:** In the app, click **Show** on the bar.

   **I see:** The list shows two messages. Each has **Send now** and **Remove**.

   **State and events:** `ShowQueue(true)` opens the list. Nothing is sent.

   **Status:** Fakes only. `desktop/tests/native/queue.rs::queued_messages_can_be_shown_removed_and_sent_and_presence_follows_the_tab`.

9. **I do:** Click **Remove** on the first message, `Keep both indexes until the backfill ends.`

   **I see:** Only the second message is left in the list.

   **State and events:** `RemoveQueued(0)` drops the entry in the app. No daemon call is made. No client calls `CancelQueuedTyping`, so the daemon's `cancel_queued_typing` (`daemon/humatad/src/terminal.rs:1184-1196`) is never reached from the app.

   **Status:** Fakes only (same test as step 8). The daemon's `CancelQueuedTyping` is Not built (no milestone), and no CLI command calls it.

10. **I do:** Click **Send now** on the remaining message.

    **I see:** The bar goes away. The message appears in the transcript.

    **State and events:** `SendQueued(0)` sends one `Type` request with `When::Now`, then removes the entry on `Ok`.

    **Status:** Fakes only (same test as step 8).

11. **I do:** The host Mac sleeps, so `ledger` goes Offline. I queue `Keep this for later.` and press **Send now**.

    **I see:** An error banner reads "This session cannot take a message. Reconnect its machine or resume the agent, then send again." The message stays in the queue.

    **State and events:** `send_queued` checks `accepts_input` before it sends (`features/session/src/lib.rs:681-700`). Nothing is sent. The queue is unchanged.

    **Status:** Fakes only. `features/session/tests/queue.rs::offline_send_preserves_the_queue_and_remove_only_changes_the_open_session`.

12. **I do:** Send a message while the connection to the host drops during the send.

    **I see:** An error banner shows the daemon's message. The message stays in the queue, and the bar offers **Send next**. Nothing is sent again on its own.

    **State and events:** The `Sent` handler with `Err` adds the session to `failed_queues` and sets the error (`lib.rs:1253` onward). `flush` does not retry a session in `failed_queues`.

    **Status:** Fakes only. `features/session/tests/queue.rs::a_failed_selected_message_returns_to_its_original_session_after_navigation` and `desktop/tests/native/queue.rs::a_failed_queued_message_stays_visible_for_retry`.

## Branches

### The agent asks for approval while a message waits

**When:** Claude raises a permission prompt while a message is queued. Its hook reports needs-input (D169), so the session shows Needs you.

**I do:** Nothing. The agent is at its prompt.

**I see:** The session shows Needs you, and the queued message stays in the bar.

**Expected outcome:** The queued text is not typed. It goes only when the turn ends or when I send it (SE-23).

**What the code does:** The live client maps every status except Working to `Turn::Ended` (`clients/live/src/map.rs:289-292`). Needs you accepts input, so `flush` (`features/session/src/lib.rs:673-678`) sends the queued text for Claude. The text is typed into the permission prompt.

**Status:** Likely bug. Confirm by queueing text during a turn, letting Claude ask for a permission, and checking whether the text reaches the prompt. No test covers it.

### OpenCode with a queued message

**When:** An OpenCode session finishes a turn.

**I do:** Nothing.

**I see:** The message stays in the bar until I press **Send now**.

**Expected outcome:** The built-in agents table says OpenCode's turns come from hooks (`daemon/sessions/src/agents.rs:116`), and the comment on `flush` says hook-backed agents send at turn end. So the expected outcome is an automatic send at turn end.

**What the code does:** `flush` matches only Claude and Copilot (`features/session/src/lib.rs:673-674`). SPEC SE-21 says OpenCode uses the output fallback. The spec and the table disagree, and the code follows neither.

**Status:** Likely bug. Decide which is right, then align the code, the table and SE-21.

### Copilot with the output fallback

**When:** A Copilot session has no hook report, so its turn end comes from the output fallback. The status becomes Probably idle after 3 s without output.

**I do:** Nothing.

**I see:** The queued message is typed about 3 s after the output stops.

**Expected outcome:** SE-23 says fallback detection needs an explicit Send.

**What the code does:** Probably idle maps to `Turn::Ended` (`clients/live/src/map.rs:289-292`), and `flush` sends for Copilot at Ended (`features/session/src/lib.rs:673-674`). Copilot is not in the built-in agents table (D169). The default `turn_end` is output (`daemon/sessions/src/agents.rs:406`), so Copilot is on the fallback unless its agent entry says `hooks`.

**Status:** Likely bug.

### A turn end the app is unsure of

**When:** A session's status is Probably idle, so the app is not sure the turn ended.

**I do:** Nothing.

**I see:** Expected: the bar shows "Send next" and the held text waits. The fake tests show this (`desktop/tests/native/queue.rs::uncertain_turn_end_keeps_the_queue_visible_until_explicit_send`).

**Expected outcome:** Held text waits, and the app says it is unsure.

**What the code does:** The live client never makes `Turn::Uncertain`. It yields only `Running` or `Ended` (`clients/live/src/map.rs:289-292`). `Uncertain` is set only in fixtures (`clients/src/fixture/sessions.rs:45`). In the live app, this branch is unreachable.

**Status:** Likely bug. The uncertain branch is tested only with fakes.

### Two-line message sent to a terminal

**When:** I queue `one whole message` and then `with two lines` in the same text, and send it.

**I do:** Press **Send now**.

**I see:** Each line is submitted on its own, if the agent reads the newline as Enter.

**Expected outcome:** SE-23 says whole messages use serialized bracketed paste, never raw bytes.

**What the code does:** The live client sends the text plus `\r` as one `Type` request (`clients/live/src/daemon.rs:702-716`). The daemon types raw bytes with no bracketed paste (`daemon/humatad/src/terminal.rs:1158-1160`). `DECISIONS.md` item 12 records this as a recommendation to switch.

**Status:** Likely bug. `features/session/tests/queue.rs::fallback_and_plain_shell_messages_wait_for_an_explicit_send` uses a two-line message but checks only the queue, not the bytes.

### The app closes while a send is in flight

**When:** I press **Send now**. The daemon types the text, and the app quits before the reply arrives.

**I do:** Quit the app, then reopen it.

**I see:** The message is back in the bar.

**Expected outcome:** The message is not sent twice.

**What the code does:** The app keeps a sent message in its saved queue until the reply arrives (`features/session/src/lib.rs:705`). Each send gets a new idempotency key (`clients/live/src/connection.rs:323-327`), so the daemon cannot match the retry. On relaunch, the Claude session at Ended sends the message again, and the text is typed twice.

**Status:** Likely bug. Confirm by killing the app between the `Type` request and its reply, then counting the lines the shell receives.

### The app restarts with messages waiting

**When:** I quit and reopen Humata 2 with two messages waiting.

**I do:** Reopen the app.

**I see:** The bar shows 2 messages. If the session is at Ended, the first one is sent at once.

**State and events:** The queue is saved in `humata2-ui.json`, under `SavedUi.sessions` (`features/app/src/saved.rs`), and restored at `features/app/src/lib.rs:227`. The daemon holds nothing. What is lost: nothing from the app's side. The daemon, `humata session queue` and any other app instance cannot see the messages.

**Expected outcome:** The messages come back in the bar in the same order. They are not sent without a turn end or an explicit send.

**Status:** Code, no test for the save and reopen round trip. `features/app/tests/messages.rs::string_queues_saved_before_comment_messages_still_load` loads an older saved queue, against fake storage. It does not save a queue and reopen the app.

## Expected outcomes

- [ ] A message sent while the turn is Running is held, not typed.
- [ ] The daemon records a turn end from the Stop hook, and the session shows Idle.
- [ ] The held message is typed once when the turn ends, for a Claude session.
- [ ] `session_type` with `when: "turn_end"` is refused with a not-built error, and nothing is typed.
- [ ] `session_type` with `when: "now"` types at once and returns `queued_id: null`.
- [ ] `session queue` and `session send-now` answer not-built and exit 7.
- [ ] Remove drops a message from the app only. No daemon call is made.
- [ ] Send now types one message and removes it only after the daemon acknowledges it.
- [ ] An offline session keeps its messages, and a send refuses with a message that says how to resume.
- [ ] A failed send keeps the message, shows an error, and does not retry on its own.
- [ ] A held message is not typed into a permission prompt (Likely bug; see the branch).
- [ ] A held message is typed once, even if the app quits during a send (Likely bug; see the branch).

## Status summary

| Step or branch | Status | Evidence |
| --- | --- | --- |
| 1. Submit while Running holds the message | Fakes only | `features/session/tests/queue.rs::a_hook_turn_end_delivers_waiting_messages_but_running_and_uncertain_do_not` |
| 2. Stop hook sets the turn to Idle | Works (real daemon) | `daemon/humatad/tests/turns.rs::the_hooks_move_a_session_between_idle_working_and_needing_input` |
| 3. The app sends the held message at turn end | Fakes only | `features/session/tests/queue.rs::a_hook_turn_end_delivers_waiting_messages_but_running_and_uncertain_do_not`; daemon typing: `daemon/humatad/tests/terminal.rs::each_text_typed_into_a_session_reaches_its_shell` |
| 4. MCP `session_type` with `turn_end` | Not built | D169; `daemon/humatad/src/terminal.rs:911-916`; no test |
| 5. MCP `session_type` with `now` | Works (real daemon) | `daemon/humatad/tests/mcp.rs::an_agent_types_into_its_own_session_through_mcp` |
| 6. CLI `session type --at-turn-end` | Not built | D169; `cli/tests/sessions.rs::type_at_turn_end_waits_for_the_turn` (Fakes only, request shape) |
| 7. CLI `session queue` | Not built | `cli/src/commands/sessions.rs:73`; `daemon/humatad/src/terminal.rs:1198-1207`; no test |
| 8. Show the queue | Fakes only | `desktop/tests/native/queue.rs::queued_messages_can_be_shown_removed_and_sent_and_presence_follows_the_tab` |
| 9. Remove a held message | Fakes only | same test; daemon `CancelQueuedTyping` Not built (`daemon/humatad/src/terminal.rs:1184-1196`) |
| 10. Send now a held message | Fakes only | same test |
| 11. Offline send keeps the queue | Fakes only | `features/session/tests/queue.rs::offline_send_preserves_the_queue_and_remove_only_changes_the_open_session` |
| 12. Failed send keeps the queue | Fakes only | `features/session/tests/queue.rs::a_failed_selected_message_returns_to_its_original_session_after_navigation`; `desktop/tests/native/queue.rs::a_failed_queued_message_stays_visible_for_retry` |
| Branch: agent asks for approval | Likely bug | `clients/live/src/map.rs:289-292`; `features/session/src/lib.rs:673-678` |
| Branch: OpenCode | Likely bug | `features/session/src/lib.rs:673-674`; `daemon/sessions/src/agents.rs:116` |
| Branch: Copilot on the fallback | Likely bug | `clients/live/src/map.rs:289-292`; `daemon/sessions/src/agents.rs:406` |
| Branch: uncertain turn end | Likely bug | `clients/live/src/map.rs:289-292`; `clients/src/fixture/sessions.rs:45` |
| Branch: two-line message | Likely bug | `clients/live/src/daemon.rs:702-716`; `daemon/humatad/src/terminal.rs:1158-1160` |
| Branch: app closes mid-send | Likely bug | `features/session/src/lib.rs:705`; `clients/live/src/connection.rs:323-327` |
| Branch: app restarts with messages | Code, no test | `features/app/src/saved.rs`; `features/app/src/lib.rs:227`; `features/app/tests/messages.rs::string_queues_saved_before_comment_messages_still_load` (Fakes only) |

## Gaps and likely bugs

**Not built**

- Turn-end typing through `Type` with `When::TurnEnds`: `daemon/humatad/src/terminal.rs:911-916`. No milestone; D169 defers it.
- `ListQueued` (`daemon/humatad/src/terminal.rs:1198-1207`) and the CLI `humata session queue`. No milestone.
- `SendQueuedNow` (`daemon/humatad/src/terminal.rs:1210-1221`) and the CLI `humata session send-now`. No milestone.
- `CancelQueuedTyping` (`daemon/humatad/src/terminal.rs:1184-1196`). No milestone; no client or CLI calls it.
- The daemon never holds the waiting messages, so the app queue is invisible to agents, the CLI and other app instances. No milestone.

**Likely bugs**

- A queued message is typed into a Claude permission prompt (Needs you counts as Ended in `clients/live/src/map.rs:289-292`, and `flush` sends for Claude at `features/session/src/lib.rs:673`).
- OpenCode never auto-sends, although the built-in table makes it hook-backed (`daemon/sessions/src/agents.rs:116`). SPEC SE-21 says otherwise.
- Copilot on the output fallback auto-sends after 3 s of quiet, which SE-23 forbids.
- `Turn::Uncertain` never reaches the live app, so the uncertain hold runs only in fakes (`clients/live/src/map.rs:289-292`).
- Multi-line messages are typed raw, not as bracketed paste, which SE-23 requires (`clients/live/src/daemon.rs:702-716`; `daemon/humatad/src/terminal.rs:1158-1160`).
- An app that quits during a send can type the message twice, because each send gets a new idempotency key (`clients/live/src/connection.rs:323-327`).
- The MCP refusal for `turn_end` says "update Humata on this machine; until then, do this from the app" (`daemon/humatad/src/status.rs:131-140`). No update adds this feature, and an agent cannot do it from the app. The message should say to send now or ask the user, as AGENTS.md asks.
- The comment on `Submit` says "send now if the turn ended, else queue" (`features/session/src/lib.rs:432`). The code queues at turn end for every agent except Claude and Copilot.
- `api/MCP.md:251` documents `session_type` as returning `{typed, queued_id?}`. The code also returns `turn_end_uncertain` (`daemon/humatad/src/mcp.rs:1008`). The doc should list it, or the field should go.

**Answers to the brief's questions**

- What the app keeps across a restart: the queue, in `humata2-ui.json`. What the daemon does not keep: any of it.
- Can `queued_id` or `turn_end_uncertain` ever be set? No. The only success path of `Type` sets `queued: None` and `turn_end_uncertain: false` (`daemon/humatad/src/terminal.rs:920-924`). The `TurnEnds` path returns before that. A remote machine runs the same code. The MCP result always writes null and false.

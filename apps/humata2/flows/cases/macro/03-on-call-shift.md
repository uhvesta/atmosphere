# On call: triage a page in a Desk session, dig in on box-1, answer its asks, stop at shift end

Layer: macro. Platforms: macOS, Linux, web. (The keys shown are the macOS ones; Linux uses `Ctrl`.)

## Starting state

- Humata 2 is open on This Mac. The daemon is running. The Home page is showing.
- Machines: This Mac (online) and box-1 (online, linked to This Mac).
- A channel named `oncall` exists, with `acme/api` and `acme/web` as repos. It is pinned, so it sits at the top of the sidebar's Pinned section.
- The clock reads 02:10. The page's text and a screenshot of the alert are on the clipboard.
- No sessions are running. The Inbox is empty.

## Steps

1. **I do:** Click `oncall` in the sidebar's Pinned section.
   **I see:** The channel page opens. `oncall` is the first row of the sidebar's Pinned section.
   **State and events:** The route becomes the channel. The pin is kept on the host.
   **Status:** Works (real daemon). `desktop/tests/live_flows/channel_page.rs::the_palette_opens_a_channel_and_its_pin_is_kept_on_the_host` pins the channel from its page, sees it under Pinned, and opens it from the Pinned row on another page. Fake-client version: `features/app/tests/marks.rs::vip_and_channel_pins_from_the_palette_reach_the_host`.

2. **I do:** Open the Desk and start a session titled "Triage 02:10 page".
   **I see:** The Desk page's form, then the new Desk session with its agent starting. The session is listed under the Desk, not under a channel.
   **State and events:** A Desk session row is recorded. Its folder sits under the Desk folder.
   **Status:** Works (real daemon). `desktop/tests/live_flows/desk.rs::a_desk_session_starts_from_the_page_gets_a_repo_and_its_document_is_copied_to_a_channel` types the title, clicks Start, and the session page opens. Fake-client version: `features/app/tests/desk.rs::empty_home_starts_a_desk_session_and_opens_it`. The daemon side is in step 3.

3. **I do:** Nothing. The daemon makes the folder and starts the agent.
   **I see:** The folder is named from the title (a slug of it). The agent's terminal appears.
   **State and events:** The session's folder is the title's slug under the Desk root. A title that is already taken gets the next suffix.
   **Status:** Works (real daemon). `daemon/humatad/tests/desk.rs::a_scratch_session_runs_its_agent_in_a_folder_named_from_its_title`.

4. **I do:** Click the session's terminal and press ⌘V with the alert text on the clipboard.
   **I see:** The alert text appears at the agent's prompt as one paste.
   **State and events:** The text goes to the agent's terminal as one bracketed paste. It is not typed key by key.
   **Status:** Works (real shell). `terminal/lab/tests/clipboard_ssh.rs::ssh_unicode_multiline_color_copy_pasteback_and_focus_are_exact` pastes back into a real shell over SSH. That test does not cover the confirmation for a multi-line paste, which lives in `terminal/pane` and is not checked here.

5. **I do:** Press ⌘V with the screenshot on the clipboard.
   **I see:** The screenshot is not typed as text. The image is saved, and the terminal gets the saved file's path.
   **State and events:** The clipboard image goes to the host, which saves it privately and returns the path. What the terminal does with that path after the save is not covered by a test.
   **Status:** Fakes only. `terminal/view/tests/view.rs::pasted_text_and_images_reach_the_host` checks that an image reaches the host, against a fake host. The daemon's save is proven: `daemon/humatad/tests/paste.rs::a_pasted_image_is_saved_privately_where_the_path_says`.

6. **I do:** Press New session in `oncall`. Choose machine `box-1`. Type "dig into the 02:10 errors" as the first message.
   **I see:** The new session's tab opens. The machine picker shows box-1 as the place it runs.
   **State and events:** A channel session is recorded on box-1. The start is sent to the host, which relays it to box-1.
   **Status:** Works (real daemon) for the picker: `desktop/tests/live_machines.rs::the_new_session_picker_lists_an_added_machine_in_its_channel` lists an added box in the channel's machines and picks it. Fake-client version: `features/app/tests/new_session.rs::changing_channel_selects_an_online_host_in_that_channel`. The start Works from the sheet too: `desktop/tests/live_machines.rs::a_session_started_on_box_1_from_the_sheet_runs_there` picks box-1, types the first message and clicks Start session; the session is listed on box-1 and the agent there answers it.

7. **I do:** Nothing. Box-1 starts the agent and the host relays its terminal.
   **I see:** The session shows as running on box-1. Its terminal is attached through this Mac.
   **State and events:** The session is recorded on box-1 under the channel's id. Its status is live.
   **Status:** Works (real daemon). `daemon/humatad/tests/remote.rs::a_session_started_on_the_machine_runs_there_and_the_host_relays_its_terminal`. Box-1 is a fake machine daemon in the test. From the app: `desktop/tests/live_machines.rs::a_session_started_on_box_1_from_the_sheet_runs_there` shows the box-1 agent's answer in the session's terminal, relayed through this Mac, and the session is live.

8. **I do:** Nothing. The agent asks to run a command that needs permission.
   **I see:** An Inbox card with the requester's title, the command shown in full, and the session it came from. Buttons: Allow, Deny and Reply…. A tick box: "Allow this for the rest of the session". The agent's hook waits.
   **State and events:** A pending request is recorded against the session. The Inbox counts it.
   **Status:** Works (real daemon). `desktop/tests/live_inbox.rs::an_agents_permission_ask_waits_in_the_inbox_and_allow_reaches_its_hook`. The agent is a fake script.

9. **I do:** Go to Home.
   **I see:** The session is listed under "Needs you", with its request shown there.
   **State and events:** Home reads the pending request on the session.
   **Status:** Works (real daemon). `desktop/tests/live_inbox.rs::a_session_waiting_on_its_own_ask_is_under_needs_you_on_home`. Fake-client version: `features/app/tests/home_needs_you.rs::a_session_waiting_on_its_own_request_needs_you_with_that_request`.

10. **I do:** Press Allow on the first ask.
    **I see:** The card goes away. The agent carries on.
    **State and events:** The request is answered Approved. The hook gets the answer and the agent's call returns.
    **Status:** Works (real daemon). `desktop/tests/live_inbox.rs::an_agents_permission_ask_waits_in_the_inbox_and_allow_reaches_its_hook` clicks Allow on a real daemon.

11. **I do:** On the second ask, tick "Allow this for the rest of the session" and press Allow.
    **I see:** The tick is marked. After Allow, the grant is listed with the rule it came from (IN-19), and it covers that target only.
    **State and events:** The answer is Approved with the rule. A grant is made for this session and this target. Other targets still ask.
    **Status:** Works (real daemon). `desktop/tests/live_inbox.rs::allowing_with_always_ticked_makes_a_grant_and_the_next_ask_for_that_target_does_not_wait` ticks the box and clicks Allow, and the grant is listed in the app and held by the host. The next ask for that target doesn't wait. Always allow is offered on an ask-ahead (`humata access ask-ahead`), not on a one-off tool permission (`daemon/humatad/src/inbox/permission.rs`). Fake-client versions: `features/inbox/tests/answers.rs::a_ticked_allow_sends_always_with_the_rule` and `features/app/tests/inbox_flow.rs::allowing_with_always_ticked_records_a_grant_for_the_requesting_session`. The grant's scope is proven on the daemon: `daemon/humatad/tests/inbox.rs::always_let_grants_only_the_target_the_card_offered`.

12. **I do:** On a third ask, press Reply…, type "wrong cluster, use prod-2", and press Send reply.
    **I see:** A text box holding what I typed. Send reply denies the request with my note.
    **State and events:** `InboxAction::SendReply` answers Denied with the text as the reply. The hook gets the note.
    **Status:** Works (real daemon): `clients/live/tests/inbox.rs::deny_with_a_reply_ends_the_agents_wait_as_denied` denies with a note from the app's client and the agent's wait ends with the note. Typing into the box is `features/inbox/tests/answers.rs::a_typed_reply_is_what_send_reply_sends`. (Was a likely bug: the box took no typing; fixed by BUGS, `ui/src/request_card.rs` sends `ReplyText`.)

13. **I do:** Look for a Not now button on the permission card.
    **I see:** Allow, Deny, Reply… and the always tick. There is no Not now.
    **State and events:** None.
    **Status:** Not built (no milestone). SPEC's "Not now" is for view offers only (VW-5, `SPEC.md:2094`). The brief named it for the Inbox, and it is not in the SPEC for permission cards.

14. **I do:** At shift end, choose Stop all sessions… on box-1, and then confirm.
    **I see:** A confirmation. Until I confirm, nothing is sent. After confirming, box-1's sessions show Stopped.
    **State and events:** Nothing is sent until the confirm. Then the stop is sent to box-1.
    **Status:** Fakes only. `features/hosts/tests/stop.rs::stop_all_sessions_asks_first_and_sends_nothing_until_confirmed`. Left to STOP.

15. **I do:** Nothing. Box-1 ends its agents and shells.
    **I see:** Box-1's sessions are Stopped. Starting them again later shows a restart report that names the stop.
    **State and events:** The stop is recorded on box-1, and a report says it was a stop, not a crash.
    **Status:** Works (real daemon). `daemon/humatad/tests/stop_box.rs::a_host_stops_a_linked_boxs_sessions_and_starts_them_again_with_a_report_naming_the_stop`.

## Branches

### box-1 is offline when I start the session

- **When:** At step 6, when box-1's link is down.
- **I do:** Press New session and choose box-1.
- **I see:** The start is refused at once. The message says box-1 is offline, that its sessions keep running, and to use Reconnect now on the machine's page.
- **Expected:** No session record is made. Box-1's running sessions are not touched. A stop to box-1 is refused the same way.
- **Status:** Works (real daemon). `daemon/humatad/tests/remote.rs::a_machine_that_goes_away_and_returns_reattaches_its_session_without_starting_it_twice` refuses a start and a stop with `MachineOffline` and leaves one session. The wording is in `daemon/humatad/src/host_relay.rs:55-73`.

### An ask arrives while I am typing

- **When:** An agent asks for permission while I am writing in a composer.
- **I do:** Keep typing.
- **I see:** The Inbox count goes up. If permission alerts are on, a toast reads "Permission requested: <title>". Alerts are off by default.
- **Expected:** Focus stays where it is, and my typing is not interrupted.
- **Status:** Code, no test. `features/app/src/events.rs:33-34` sets the toast text and `:63-64` shows it. Nothing in that path moves focus. The shell's focus target (`ui/src/shell.rs:283-299`) follows the route and tab, not requests. No test asserts focus. The default is covered by `features/app/tests/permission_alerts.rs::alert_preference_survives_restart_and_old_saves_default_to_off`.

### I deny with a note

- **When:** At step 12.
- **I do:** Press Reply…, type a note, and press Send reply.
- **I see:** The note is typed into a box and the request is denied.
- **Expected:** The request is denied with my note, and the agent's hook gets the note.
- **Status:** Works (real daemon): `clients/live/tests/inbox.rs::deny_with_a_reply_ends_the_agents_wait_as_denied`; the hook side is `daemon/humatad/tests/hooks.rs::an_inbox_deny_reaches_the_hook_with_its_note`; typing the note is `features/inbox/tests/answers.rs::a_typed_reply_is_what_send_reply_sends` (fakes). (Was a likely bug; fixed by BUGS.)

### The agent asks to read another session's turns

- **When:** The agent lists another session's turns. Its rule for that is Ask.
- **I do:** Allow or Deny in the Inbox.
- **I see:** A card that names the other session. The agent's call waits.
- **Expected:** The call waits until I answer. Allow returns the turns. Deny returns a refusal, and nothing is read.
- **Status:** Works (real daemon). `daemon/humatad/tests/inbox.rs::an_agent_listing_another_sessions_turns_waits_in_the_inbox_when_the_rule_asks`.

### Stop with a terminal attached

- **When:** At step 15, with a terminal open on one of the sessions.
- **I do:** Stop the sessions.
- **I see:** The attached terminal says the session is stopped and to connect again after a start.
- **Expected:** The stop finishes. Nothing is typed into the terminal after it stops, and the terminal is told to reconnect.
- **Status:** Works (real daemon). `daemon/humatad/tests/stop_sessions.rs::a_terminal_attached_when_the_sessions_stop_is_told_to_connect_again`.

### Hand the session to the next person at shift end

- **When:** The shift ends and I want the next on-call person to pick the session up.
- **I do:** Look for a handover to another person.
- **I see:** Nothing. Humata has no flow for handing a session to another person. SPEC FK-6 is a handover between agents, not people. The nearest things are a fork (`daemon/humatad/tests/fork.rs`) and a copy of a Desk document to the channel's shared folder.
- **Expected:** Not applicable until the spec has a handover between people.
- **Status:** Not built (no milestone).

## Expected outcomes

- [ ] `oncall` opens from the sidebar's Pinned section.
- [ ] A Desk session's folder is named from its title, and its agent starts.
- [ ] Pasted text reaches the agent's terminal in one paste, and a pasted image is saved privately with a path.
- [ ] A session starts on box-1 and its terminal is relayed through This Mac.
- [ ] An agent's ask waits in the Inbox with its command shown, and Allow reaches the waiting hook.
- [ ] "Allow this for the rest of the session" grants only the target the card offered.
- [ ] A typed note with Deny reaches the agent.
- [ ] Starting a session on an offline box is refused with the offline message, and nothing is recorded.
- [ ] An ask does not take focus from what I am typing.
- [ ] Stop all sessions on box-1 waits for my confirm, then stops the sessions, and the next start names the stop.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Click oncall in Pinned | Works (real daemon) | `desktop/tests/live_flows/channel_page.rs::the_palette_opens_a_channel_and_its_pin_is_kept_on_the_host`; fakes: `features/app/tests/marks.rs::vip_and_channel_pins_from_the_palette_reach_the_host` |
| 2. Start a Desk session | Works (real daemon) | `desktop/tests/live_flows/desk.rs::a_desk_session_starts_from_the_page_gets_a_repo_and_its_document_is_copied_to_a_channel` |
| 3. Desk folder named from title | Works (real daemon) | `daemon/humatad/tests/desk.rs::a_scratch_session_runs_its_agent_in_a_folder_named_from_its_title` |
| 4. Paste alert text | Works (real shell) | `terminal/lab/tests/clipboard_ssh.rs::ssh_unicode_multiline_color_copy_pasteback_and_focus_are_exact` |
| 5. Paste screenshot | Fakes only | `terminal/view/tests/view.rs::pasted_text_and_images_reach_the_host` (save: `daemon/humatad/tests/paste.rs::a_pasted_image_is_saved_privately_where_the_path_says`) |
| 6. New session on box-1 (picker) | Works (real daemon) | `desktop/tests/live_machines.rs::the_new_session_picker_lists_an_added_machine_in_its_channel`, `desktop/tests/live_machines.rs::a_session_started_on_box_1_from_the_sheet_runs_there` |
| 7. Session runs on box-1, terminal relayed | Works (real daemon) | `daemon/humatad/tests/remote.rs::a_session_started_on_the_machine_runs_there_and_the_host_relays_its_terminal` |
| 8. Ask waits in Inbox | Works (real daemon) | `desktop/tests/live_inbox.rs::an_agents_permission_ask_waits_in_the_inbox_and_allow_reaches_its_hook` |
| 9. Home shows Needs you | Works (real daemon) | `desktop/tests/live_inbox.rs::a_session_waiting_on_its_own_ask_is_under_needs_you_on_home`; fakes: `features/app/tests/home_needs_you.rs::a_session_waiting_on_its_own_request_needs_you_with_that_request` |
| 10. Allow reaches the hook | Works (real daemon) | `desktop/tests/live_inbox.rs::an_agents_permission_ask_waits_in_the_inbox_and_allow_reaches_its_hook` |
| 11. Always tick, grant for this target | Works (real daemon) | `desktop/tests/live_inbox.rs::allowing_with_always_ticked_makes_a_grant_and_the_next_ask_for_that_target_does_not_wait`; fakes: `features/inbox/tests/answers.rs::a_ticked_allow_sends_always_with_the_rule` (grant scope: `daemon/humatad/tests/inbox.rs::always_let_grants_only_the_target_the_card_offered`) |
| 12. Type a note and Send reply | Works (real daemon) | `clients/live/tests/inbox.rs::deny_with_a_reply_ends_the_agents_wait_as_denied`; `features/inbox/tests/answers.rs::a_typed_reply_is_what_send_reply_sends` |
| 13. Not now on the card | Not built | no milestone; `SPEC.md:2094` (VW-5, views only) |
| 14. Stop all sessions, confirm first | Fakes only (STOP) | `features/hosts/tests/stop.rs::stop_all_sessions_asks_first_and_sends_nothing_until_confirmed` |
| 15. Box stops, report names the stop | Works (real daemon) | `daemon/humatad/tests/stop_box.rs::a_host_stops_a_linked_boxs_sessions_and_starts_them_again_with_a_report_naming_the_stop` |
| Branch: box-1 offline | Works (real daemon) | `daemon/humatad/tests/remote.rs::a_machine_that_goes_away_and_returns_reattaches_its_session_without_starting_it_twice` |
| Branch: ask while typing | Code, no test | `features/app/src/events.rs:33-34, 63-64` |
| Branch: deny with a note | Works (real daemon) | `clients/live/tests/inbox.rs::deny_with_a_reply_ends_the_agents_wait_as_denied`; `daemon/humatad/tests/hooks.rs::an_inbox_deny_reaches_the_hook_with_its_note`; `features/inbox/tests/answers.rs::a_typed_reply_is_what_send_reply_sends` |
| Branch: agent asks to read another session's turns | Works (real daemon) | `daemon/humatad/tests/inbox.rs::an_agent_listing_another_sessions_turns_waits_in_the_inbox_when_the_rule_asks` |
| Branch: stop with a terminal attached | Works (real daemon) | `daemon/humatad/tests/stop_sessions.rs::a_terminal_attached_when_the_sessions_stop_is_told_to_connect_again` |
| Branch: hand over to the next person | Not built | no milestone; SPEC FK-6 is agent-to-agent |

## Gaps and likely bugs

- **Not built:** "Not now" on a permission card. The SPEC has it only for view offers (VW-5). No milestone found.
- **Not built:** a handover of a session to another person. No milestone found.
- **Fixed (was a likely bug):** the reply box takes typed text and Send reply sends it (`features/inbox/tests/answers.rs::a_typed_reply_is_what_send_reply_sends`, `clients/live/tests/inbox.rs::deny_with_a_reply_ends_the_agents_wait_as_denied`).
- **Note, not a bug:** the brief said box-1 "has the prod tools". Nothing in the code checks or names tools on a machine for this flow, so this case treats box-1 as an ordinary machine.

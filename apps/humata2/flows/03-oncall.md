# 3. On call: open the channel, paste the alert, start a session

> "Hey, I'm on call. Let me open the oncall channel, paste something into the desk and make a session to work on something."

My team keeps an `#oncall` channel with the service repos already cloned. When an alert fires, I want a quick place to paste it and have an agent triage it against those clones. If it needs a real fix, that becomes a proper session with its own branch.

How to read this file, and what the tags mean: [README](README.md).

## Starting state

- `#oncall` exists, with `acme/checkout` and `acme/payments` cloned on This Mac and devbox-1. It's pinned in the sidebar.
- `#oncall` has no Desk sessions and no running sessions.
- An alert's text is on my clipboard.

## Steps

1. **Do:** press ⌘K, type "oncall" and press Return. (Or click it under Pinned.)
   **See:** the `#oncall` page on **Sessions**.
   **Expect:** nothing changes on the daemon.
   **Status:**
   - Opening a channel from the palette: **Fakes only**, `features/palette/tests/hits_cursor.rs::running_a_hit_closes_the_palette_and_hands_its_place_to_the_app`. The live palette flow opens a session, not a channel (`desktop/tests/live_flows/find.rs::from_the_palette_a_session_moves_a_turn_is_found_a_pr_reviews_settings_open_and_a_pin_holds`).
   - The pin **Works**: `clients/live/tests/marks.rs::pins_and_vip_marks_survive_a_restart`, and the live find flow checks a pin holds. The page's **Pin** is **Fakes only**: `desktop/tests/native/marks.rs::the_channel_pages_pin_pins_and_unpins_its_channel`.

2. **Do:** click the **Desk · 0** tab, after Sessions.
   **See:** an empty state with **New desk session**.
   **Status: Not built: ROAM** (CD-2, frame 91). There's no tab and no session kind. Today's stand-in is the global Desk (flow 1), whose agent can't read `#oncall`'s clones and isn't tied to the channel.

3. **Do:** click **New desk session**. Title it "Checkout 5xx alert", leave the target as the channel and the machine as devbox-1, and leave the switches as they are. Click **Start**.
   **See:** the switches **Read its files** and **Send notes and questions** are on. **Type into its terminal**, **Start and fork sessions**, **Stop and archive sessions** and **Change anyone's items on the channel board** are off.
   **Expect:**
   - A scratch folder `#oncall/desk/checkout-5xx-alert` exists on devbox-1.
   - The agent can read the channel's main clones and `shared/`, read-only.
   - The tab reads **Desk · 1**, and the sidebar shows a Desk row under `#oncall`.
   - Event: Session created.
   **Status: Not built: ROAM** (CD-1, CD-3, CD-6, frame 92).

4. **Do:** click into the agent's terminal and press ⌘V, then Return. Then paste a screenshot of the error graph.
   **See:** the multi-line paste asks first. The agent reads the alert and starts looking through the clones. The image arrives as a path the agent can open.
   **Expect:**
   - The text reaches the agent once.
   - The image is saved privately on the session's machine, and its path is typed.
   - Event: Session updated (Working).
   **Status:**
   - Typing: **Works**, `clients/live/tests/parity.rs::typed_text_reaches_the_agent_on_real`.
   - Paste: **Fakes only**, in the terminal component only (`terminal/pane/tests/pane.rs::a_multi_line_paste_waits_for_the_user`). No app test pastes into a session tab (TERM).
   - Image paste: **Not built, no milestone.** The daemon saves it (`daemon/humatad/tests/paste.rs::a_pasted_image_is_saved_privately_where_the_path_says`), but no client calls it.

5. **Do:** the agent finds the cause in `acme/checkout`. On the channel page, click **New session**. Title it "Retry budget for checkout", leave only `acme/checkout` in (the other gets "Leave this repo out"), paste the agent's summary into **First message**, and click **Start session**.
   **See:** the new session's page, with its agent on my message.
   **Expect:**
   - One clone, of `acme/checkout`, on `you/retry-budget-for-checkout`.
   - Event: Session created, then updated (Working).
   **Status: Works.** `desktop/tests/live_flows.rs::a_session_started_from_new_session_runs_and_answers_its_first_message` (real daemon and holder, scripted agent). Having the Desk agent start it (with **Start and fork sessions** on) is **Not built: ROAM** (CD-3).

6. **Do:** the fix session's agent wants to run a migration script. Open the Inbox and click **Allow** on its ask.
   **See:** the card names the command and the session.
   **Expect:**
   - The agent's permission hook gets "allow", and the agent goes on.
   - Event: InboxRequest created (Pending), then updated (Approved).
   **Status: Works.** `desktop/tests/live_inbox.rs::an_agents_permission_ask_waits_in_the_inbox_and_allow_reaches_its_hook` and `::a_request_allowed_from_the_requesters_session_returns_the_agents_waiting_call`.

7. **Do:** on the next ask for the same target, tick **Always allow …** before clicking Allow. Then on the channel's **Agent access** tab, set **Start or fork sessions (same machine)** to Allow.
   **See:** the grant is listed and can be removed. The rule's radio shows Allow.
   **Expect:**
   - That session doesn't ask again for that target.
   - Agents in `#oncall` can start sessions on their machine without asking.
   - Events: Grant created; ChannelRules updated.
   **Status:**
   - Always allow **Works** in the client: `clients/live/tests/inbox.rs::always_allow_makes_a_grant_the_app_lists_and_can_remove`. No app test ticks it against a real daemon.
   - Agent access **Works**: `desktop/tests/live_inbox.rs::the_agent_access_tab_sets_the_channels_rule_on_the_daemon` clicks the tab and the radio, and the daemon keeps the rule. Also `clients/live/tests/inbox.rs::an_agents_call_under_an_allow_rule_runs_without_asking`.

8. **Do:** mark the fix session VIP from the palette.
   **See:** it stays at the top of Home while it runs.
   **Expect:** Event: Marks updated. The mark survives a restart.
   **Status:**
   - **Works** in the marks client: `clients/live/tests/marks.rs::vip_is_set_apart_from_the_pin_and_reads_back`. The palette entry is **Fakes only**: `features/palette/tests/marks.rs::an_unpinned_session_offers_pin_and_vip`.
   - The older daemon client still answers "not in this release" for pin, VIP and mark-read (`clients/live/src/daemon.rs`). Any screen that uses it instead of the marks client would fail live.

9. **Do:** before the hand-off, the Desk agent writes `handoff.md`. Pick **Close session** on its Desk session.
   **See:** a confirmation saying the folder goes to the trash now, listing `handoff.md` with **Copy to shared/**.
   **Expect:**
   - `#oncall/shared/handoff.md` exists.
   - The scratch folder is in the trash right away, with no 7-day wait.
   - The tab reads **Desk · 0**.
   **Status: Not built: ROAM** (CD-4, frame 94). The global Desk's **Copy to a channel…** works (`daemon/humatad/tests/desk.rs::markdown_in_the_folder_is_listed_and_copies_into_a_channels_shared_folder`).

10. **Do:** the next person on call opens `#oncall` › **Sessions**.
    **See:** "Retry budget for checkout" with its state (Working, Needs input or Idle) and whether it has an unread turn.
    **Status:**
    - **Fakes only**: `desktop/tests/native/channel.rs::channel_sessions_filter_fold_and_open_their_own_inbox`. The state from hooks **Works** in the daemon: `daemon/humatad/tests/turns.rs::the_hooks_move_a_session_between_idle_working_and_needing_input`.
    - Handing the channel to another person: **Not built, no milestone.** Humata is one person's app, so the next on-call only sees what I push or copy out.

## Expected outcome

The alert was triaged in a short-lived channel Desk session that's now in the trash. Its hand-off note is in `#oncall/shared/`. A fix session is running with its own branch, marked VIP, with a grant that stops repeat asks.

## Gaps this flow shows

- The channel's Desk is the heart of this flow, and none of it is built (ROAM CD-1–CD-6). Without it, triage happens either in the global Desk, which can't see the channel, or in a full session with a branch I don't need.
- Pasting text into a session tab has no app test. Pasting an image has no app wiring at all.
- Pin, VIP and mark-read go through two clients. The marks client works live; the older daemon calls don't.
- There's no team hand-off. That's by design, but on call is where it will be felt.

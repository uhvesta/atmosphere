# Inbox: every kind of request, answered from the wide Inbox, the session panel and the cards

Layer: explored · macOS · harness: desktop:native_test, desktop:inbox_copy_test, desktop:live_inbox_test, desktop:panels_test, features/inbox and features/app tests

Phase 1 (map from code and snapshots, no run). Steps marked PENDING RUN need the phase 2 commands in the report.

## Setup

- Wide Inbox snapshots use `fixture::design` (7 waiting requests, one selected: "Move PAY-3 to Done"). Frames 26 (Waiting) and 27 (All) are the same fixture.
- Copy cards, Migrate cards and the ask-ahead grant flows have no snapshot. Their evidence comes from the named tests in phase 2.
- Live tests (`live_inbox_test`) start a real `humatad2_admin` and holder, and an agent script that asks through the hook. They never touch the owner's `~/.humata2`, `~/.claude`, `~/.codex` or `~/.copilot`.
- Snapshot text: `apps/humata2/desktop/tests/__snapshots__/native/26-inbox.view.txt` and `27-inbox-dark.view.txt` (PNG beside each). The `.view.txt` lists rows and buttons; group headings and layout are only in the PNG, which phase 2 should view.

## Steps

### 1. Open the global Inbox from the rail

- **Screen:** Sidebar rail "Inbox" selected. Heading "Inbox", "7 waiting" beside it. Segmented control "Waiting" (selected), "Answered", "All". Bulk strip "Allow all 7" and "Deny all". Left list of 7 rows (second line names the agent and session, e.g. "Claude Code · Ledger schema migration"), first row "Move PAY-3 to Done" selected. Right detail is that request's card.
- **Offered:** Waiting / Answered / All tabs; Allow all 7; Deny all; each row (select); on the card: Open session ↗, the grant checkbox "Allow this for the rest of the session", Allow, Deny, Reply….
- **Action:** Open the Inbox from the rail.
- **Result:** The first waiting request is selected (IN-3). Waiting is the default history (`show_answered` false).
- **Evidence:** existing snapshot `apps/humata2/desktop/tests/__snapshots__/native/26-inbox.view.txt` (and `26-inbox.png`); design golden `desktop/tests/__snapshots__/design/26-inbox.png`.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=home_and_global_inbox_reference_images` exits 0 (it compares the frame to the golden).

### 2. Select a different request in the list

- **Screen:** Same as step 1. Row "Push try-brin to origin" is in the list, not selected.
- **Offered:** Click any row.
- **Action:** Click the row "Push try-brin to origin" (`global-request push-brin`).
- **Result:** The detail pane shows that request's card; the list row shows the selected fill. Only the selected request's card renders in full (IN-3).
- **Evidence:** none yet. PENDING RUN: phase 2 captures the view text after the click.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=the_global_inbox_answers_only_the_selected_request` exits 0 (asserts `inbox.selected` changes to `push-brin`).

### 3. Deny a request from its card

- **Screen:** Card for "Push try-brin to origin": title, requester line, reason, preview, "What it was doing" steps, "Asks you because" line with "Change this rule", then the answers row Allow / Deny / Reply….
- **Offered:** Allow, Deny, Reply…, grant checkbox (only when the request has a rule), Open session ↗, Change this rule.
- **Action:** Click Deny.
- **Result:** The card shows "Answering…" while the call is out. The daemon records Denied with no note. On success the request leaves the Waiting list (the row shows "denied" once the Answered history is shown), and the selection moves to the first waiting row, because the global list keeps only the selected request while it is still shown.
- **Evidence:** none yet. PENDING RUN.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=the_global_inbox_answers_only_the_selected_request` exits 0 (asserts the request's answer is `Denied { reply: None }`).

### 4. Allow with the grant checkbox ticked (Always)

- **Screen:** The checkbox "Allow this for the rest of the session" (role CheckBox) sits above the answers, only on a waiting request with a non-empty rule. It is absent for questions and for requests with no rule.
- **Offered:** Tick or untick the checkbox; Allow; Deny.
- **Action:** Tick the checkbox, then click Allow.
- **Result:** The tick sends `Answer::Always { rule }` in place of `Approved`. A grant is made for that session and target. The next ask for the same target does not wait (IN-14, IN-22). The tick clears after any answer.
- **Evidence:** none yet. PENDING RUN.
- **Replay check:** `bazel test //apps/humata2/desktop:live_inbox_test --test_filter=allowing_with_always_ticked_makes_a_grant_and_the_next_ask_for_that_target_does_not_wait` exits 0. Also `bazel test //apps/humata2/features/inbox:answers_test --test_filter=a_ticked_allow_sends_always_with_the_rule` and `--test_filter=the_tick_is_ignored_without_a_rule`.

### 5. Allow an agent's permission prompt that waits on its hook

- **Screen:** A Command or Network card: "Command" preview with the exact command (IN-10), "Asks you because" with the rule, Allow / Deny / Reply….
- **Offered:** Allow, Deny, Reply….
- **Action:** Click Allow.
- **Result:** The agent's hook receives the approval and the agent's waiting call proceeds. The item leaves Waiting.
- **Evidence:** none yet. PENDING RUN.
- **Replay check:** `bazel test //apps/humata2/desktop:live_inbox_test --test_filter=an_agents_permission_ask_waits_in_the_inbox_and_allow_reaches_its_hook` exits 0.

### 6. Deny with a reply typed on a permission card

- **Screen:** After Reply…, a text box (ReplyBox) replaces the answers, with "Send reply" and "Cancel". Empty text after trim sends a plain denial.
- **Offered:** Type; Send reply; Cancel.
- **Action:** Click Reply…, type a note, click Send reply.
- **Result:** A non-question request gets `Denied { reply: Some(text) }`. The hook gets the note back. Cancel drops the draft and sends nothing.
- **Evidence:** none yet. PENDING RUN.
- **Replay check:** `bazel test //apps/humata2/desktop:live_inbox_test --test_filter=a_reply_typed_on_a_permission_card_denies_it_and_the_hook_gets_the_note` exits 0. Also `bazel test //apps/humata2/features/inbox:answers_test --test_filter=a_typed_reply_is_what_send_reply_sends`.

### 7. Answer a question with one of its choices

- **Screen:** A question card has no Allow and no Deny. Its choices are buttons (the first is primary), then Reply….
- **Offered:** Each choice; Reply….
- **Action:** Click a choice button.
- **Result:** The answer is `Replied { text: <choice> }`, sent as the question's reply. The request leaves Waiting.
- **Evidence:** none yet. PENDING RUN.
- **Replay check:** `bazel test //apps/humata2/features/inbox:answers_test --test_filter=choosing_sends_the_choice_as_a_reply` exits 0.

### 8. Read a cross-machine copy card (what, where, size)

- **Screen:** Title line "<session> wants to copy <what> (<size>) from <machine> → <machine> · <place>". A "Details" toggle below. Actions: Approve, Somewhere else…, Deny. No Allow, no Reply… and no grant checkbox on a copy.
- **Offered:** Approve; Somewhere else…; Deny; Details (toggle).
- **Action:** Click Details.
- **Result:** The From and To paths open, each with Copy path. Reveal in Finder appears only on the side whose machine is This Mac and only for an absolute path. "Holds" shows the file count and size.
- **Evidence:** none yet (no snapshot). PENDING RUN.
- **Replay check:** `bazel test //apps/humata2/desktop:inbox_copy_test --test_filter=a_copy_card_shows_what_where_and_the_three_answers_with_paths_behind_details` exits 0. Element-level checks: `bazel test //apps/humata2/features/inbox:copy_test --test_filter=details_open_and_close_on_a_copy_and_never_on_another_request`.

### 9. Approve a copy card

- **Screen:** As step 8.
- **Offered:** Approve.
- **Action:** Click Approve.
- **Result:** `Answer::Approved` goes through the request client; the card shows "Answering…" until the daemon answers.
- **Evidence:** none yet. PENDING RUN.
- **Replay check:** `bazel test //apps/humata2/desktop:inbox_copy_test --test_filter=approving_a_copy_card_answers_the_request_through_the_client` exits 0.

### 10. Somewhere else… on a copy card

- **Screen:** As step 8.
- **Offered:** Somewhere else….
- **Action:** Click Somewhere else….
- **Result:** Opens the Files page on the destination machine (`OpenDestination`). The request stays waiting. There is no way to approve the copy to a different place: the person copies by hand and then denies the request (see Surprises).
- **Evidence:** none yet. PENDING RUN.
- **Replay check:** `bazel test //apps/humata2/desktop:inbox_copy_test --test_filter=somewhere_else_opens_the_files_page_and_keeps_the_request_waiting` exits 0.

### 11. Allow all / Deny all over the list

- **Screen:** The bulk strip shows only when more than one waiting request is shown ("Allow all N", "Deny all"). Migrate items are left out of both counts.
- **Offered:** Allow all N; Deny all; when a confirmation is open: a question line, "Allow all" or "Deny all", and Cancel.
- **Action:** Click Allow all 7 (with some undoable and some not), then Allow all in the confirmation. Separately, click Deny all.
- **Result:** Allow all with any waiting request that is not undoable first shows "Allow all 7? Some of them can't be undone once allowed." and waits for "Allow all". Allow all with only undoable requests sends at once. Deny all never asks and sends at once (see Surprises). Each request is a separate answer call.
- **Evidence:** none yet. PENDING RUN.
- **Replay check:** No test drives the confirmation (ConfirmBulk). `bazel test //apps/humata2/features/inbox:migrate_test --test_filter=allow_all_skips_the_migrate_item_and_its_count_leaves_it_out` exits 0 covers the count and the Migrate skip only.

### 12. Waiting, Answered and All history

- **Screen:** Tabs "Waiting" / "Answered" / "All" (role Tab). Answered rows dim to about 55% opacity and their second line says the answer ("approved", "denied", "denied: <note>", "replied: <text>", "always allowed").
- **Offered:** Waiting; Answered; All.
- **Action:** Click All (frame 27).
- **Result:** Answered requests show with their answers. The selected detail for an answered request says "Answered" in place of answer buttons. The "Allowed without asking" log (IN-8) does not appear in the live Inbox (see Surprises).
- **Evidence:** `apps/humata2/desktop/tests/__snapshots__/native/27-inbox-dark.view.txt` (and `27-inbox-dark.png`), dark, All.
- **Replay check:** `bazel test //apps/humata2/desktop:native_test --test_filter=home_and_global_inbox_reference_images` exits 0 (its second and third frames use All).

### 13. A Migrate item (VIP, under the "Humata" heading)

- **Screen:** The first group is "Humata". The row carries the VIP badge and "Humata · <machine name>". The card shows the release change ("from → to"), "What changes" folded to 3 lines with "+N more", and the only answer, Migrate. No Allow, Deny, Reply or grant checkbox. Status line shows Waiting, Migrating, Failed (with "Migrate again") or Migrated (the Answered history).
- **Offered:** Migrate; Migrate again (after a failure); "What changes" toggle.
- **Action:** Click Migrate.
- **Result:** The item shows "Migrating" until the daemon reports. Migrate is sent as an approval to the daemon, except the app's own old-daemon item, which migrates on This Mac and waits up to 60 s for the daemon to return at the new release, then reconnects. Allow all and Deny all leave it out.
- **Evidence:** none yet (no snapshot). PENDING RUN.
- **Replay check:** `bazel test //apps/humata2/features/inbox:migrate_test --test_filter=a_migrate_item_is_vip_in_the_humata_group_with_migrate_its_only_answer` exits 0. Also `--test_filter=migrate_sends_approve_for_that_item_and_shows_migrating` and `--test_filter=a_failed_migrate_shows_the_failure_with_migrate_again`. The fold: `bazel test //apps/humata2/features/inbox:changes_fold_test` (three tests named `more_than_three_change_lines...`, `toggle_changes_unfolds...`).

### 14. Session Inbox panel (narrow, beside the terminal)

- **Screen:** Panel header "Inbox", subtitle "this session and its forks". The open request is a full narrow card (no "What it was doing" block). Other waiting requests are folded rows (status dot, title, age, requester). With nothing waiting: "Nothing waiting for you".
- **Offered:** Click a folded row to open it; Allow, Deny, Reply… on the open card; Open session ↗ is not on the narrow card.
- **Action:** Open the Inbox panel of a session that has a waiting request, then click a folded row.
- **Result:** The clicked request opens in place (one at a time); the other folds. A failed answer shows its message under the list as an alert "Answer failed" (`panel-error`).
- **Evidence:** existing snapshot `apps/humata2/desktop/tests/__snapshots__/panels/inbox-light.txt` (and `inbox-dark.txt`).
- **Replay check:** `bazel test //apps/humata2/desktop:panels_test --test_filter=inbox_answers_the_selected_sessions_request_through_the_client` exits 0 (the file's own IN-3 test).

### 15. Open session ↗ from a wide card

- **Screen:** "Open session ↗" beside the requester line (wide cards only).
- **Offered:** Open session ↗.
- **Action:** Click Open session ↗.
- **Result:** Opens the requester's session on its Inbox panel with that request selected (`OpenRequest` → `Delegate::OpenRequest`).
- **Evidence:** none yet. PENDING RUN.
- **Replay check:** `bazel test //apps/humata2/features/app:inbox_flow_test --test_filter=open_session_lands_on_the_requesters_inbox_panel_with_the_request_selected` exits 0.

## Not reachable headless

- Real OS Finder reveal (Reveal in Finder): the `reveal` client is faked in the tests; the real Finder step needs a screen and is not run here.
- The Migrate of the app's own daemon on This Mac against a real staged release: only through the fake local daemon in `features/inbox/tests/migrate.rs`; a real restart needs a live Mac with a second release.
- The Always rule's effect on other sessions' asks is only checked through the live test's agent script.

## Surprises

1. **Deny all never asks, Allow all does.** The Inbox header comment says "Approve all and Deny all ask first when something can't be undone (IN-9)". `features/inbox/src/lib.rs` (`Bulk` arm) asks only for `ApproveAll`. `ui/src/lists.rs` has the same asymmetry, and no test drives the confirmation.
2. **Wide Inbox shows no error for a failed answer.** `InboxAction::Answered(Err)` writes `state.inbox.error`. Only `ui/src/panel_inbox.rs` (session panel) renders it; `ui/src/lists.rs` (global Inbox) does not read it. A failed Allow on the wide Inbox leaves the card as it was, with no message.
3. **The "Allowed without asking" log (IN-8) is not in the live Inbox.** The phrase appears only in the design golden drawings (`desktop/tests/design/session.rs`, `design/pages.rs`), not in `ui/src`.
4. **Copy cards have no Reply, no grant and no Allow.** The only way to answer a copy with another place is to copy by hand and deny (`features/inbox/src/lib.rs`, `SomewhereElse` doc comment). Denying a copy has no note field.
5. **Bulk Allow and the daemon's `answer_all` differ.** The UI calls the per-request answer for each shown row, so it can approve permission cards one by one and ask-ahead grants. The daemon's `answer_all` (`daemon/humatad/src/inbox.rs`) skips permission cards, ask-aheads, host asks and Migrate items, and approves only `Once`. The CLI `inbox approve-all` uses the daemon's rule, the wide Inbox's "Allow all" does not.
6. **Prepared view offers are not in the Inbox list.** "Open it / Later / Not now" for a prepared view render as a bar in the session tab (`ui/src/view_tab.rs`, `bars`), not as a row in the Inbox. The brief's "prepared views Open/Later" item is therefore in the session strip, not here.
7. **Script failures are not an Inbox kind I could find.** Grep of `features/inbox`, `ui/src` and `daemon/humatad/src/inbox*` shows no script-failure request. Not verified further; phase 2 should grep `machine_scripts` for requests.
8. **The Question card and the copy card show different controls from the rest.** A question has no Deny at all, so an unwanted question can only be answered with a choice or a reply.
9. **Offer::AlwaysAllow and Offer::Deny are computed but not drawn.** `features/inbox/src/lib.rs` `offers()` lists them, but `ui/src/request_card.rs` draws its own buttons. The grant checkbox is the only "always" control.

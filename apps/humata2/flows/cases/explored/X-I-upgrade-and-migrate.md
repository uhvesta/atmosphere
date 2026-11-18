# Upgrade and Migrate: the VIP Inbox "Migrate X" items, the machine page while it waits, and the upgrade steps

Layer: explored · macOS · harness: features/inbox/tests/migrate.rs (Inbox store, headless), features/hosts/tests/upgrade.rs (machine page store, headless), desktop/tests/design/upgrade.rs (drafts 77 to 83, design goldens, PNG only, not yet recorded), desktop/tests/native/hosts_settings.rs and the machine snapshots 56 to 59

Phase 1 (map from code and snapshots). Steps marked PENDING RUN are filled in phase 2.

## Setup

- No native or live test in this area renders a Migrate card. The Inbox and page behaviour is tested with TestStore reducers (features/inbox/tests/migrate.rs, features/hosts/tests/upgrade.rs). The screens for frames 77 to 83 are design goldens in desktop/tests/design/upgrade.rs and have no PNG yet (desktop/tests/__snapshots__/design/ stops at 71 for numbered frames). Frames 77 to 83 are marked draft in design.rs:258-270 "stay drafts until the owner signs them off (milestone UPGRADE)".
- The CLI has no migrate command. Migrate is answered in the Inbox (or from This Mac's machine page). A stale daemon refuses a call with kind `migration_needed` and exit 7 (cli/src/exit.rs:79, PRECONDITION).
- Throwaway daemon setup: see the restart record (X-I-restart-and-link-drop.md, Setup) for the build and serve command.

## Steps

### 1. A daemon at an old release is found; the Inbox gets a Migrate item
- **Screen:** The Inbox has a group "Humata" for VIP items. The card (ui/src/request_card.rs:495-520, 560-610) reads "From Humata · found when {machine} linked", with the title "Migrate the daemon on {machine}" (or the Migrate item's thing label, such as "Migrate holders on gpu-box" in the draft frame 83). Body: "{machine} runs {from}; this app is {to}." and a "What keeps running" section.
- **Offered:** Migrate (only). The card has no Allow, Always, Deny, or reply (migrate.rs:158-210 `a_migrate_item_is_vip_in_the_humata_group_with_migrate_its_only_answer`, `a_migrate_item_takes_no_deny_always_reply_or_choice`). Allow all and Deny all leave it out (migrate.rs:210-250). Open machine link on wide windows ("Open {machine} ↗").
- **Action:** A machine links with an old daemon (the fixture gives a daemon item).
- **Result:** The item waits in the Humata group; its count is not part of Allow all.
- **Evidence:** PENDING RUN for a drawn card (none exists). The draft is desktop/tests/design/upgrade.rs `inbox_migrate` (frame 77). Store-level: features/inbox/tests/migrate.rs.
- **Replay check:** `bazel test --jobs=4 //apps/humata2/features/inbox:migrate_test --test_filter=a_migrate_item_is_vip_in_the_humata_group`

### 2. Migrate on the card
- **Screen:** The card's Migrate button. While it runs: "From Humata · migrating…" and "Migrating {machine} to {to}… Sessions keep running; the link drops for a few seconds." (request_card.rs:604).
- **Offered:** Migrate (disabled or refused while migrating: a second click sends nothing more).
- **Action:** Click Migrate.
- **Result:** The daemon gets Approve for that item only (migrate.rs:253-290 `migrate_sends_approve_for_that_item_and_shows_migrating`, `a_second_click_while_migrating_sends_nothing_more`).
- **Evidence:** Store-level; draft frame 79 (desktop/tests/design/upgrade.rs `migrating`), no PNG.
- **Replay check:** `bazel test --jobs=4 //apps/humata2/features/inbox:migrate_test --test_filter=a_second_click_while_migrating_sends_nothing_more`

### 3. The migrate succeeds
- **Screen:** "From Humata · migrated" and "Migrated to {release} ({build})" with a check glyph (request_card.rs:575). It moves to the answered list.
- **Offered:** None on the card.
- **Action:** The daemon reports the update.
- **Result:** The item shows Migrated and moves to answered (migrate.rs:325-350 `an_approved_item_shows_migrated_and_moves_to_answered`).
- **Evidence:** Store-level; draft frame 81 (design/upgrade.rs `migrated`), no PNG.
- **Replay check:** `bazel test --jobs=4 //apps/humata2/features/inbox:migrate_test --test_filter=an_approved_item_shows_migrated`

### 4. The migrate fails
- **Screen:** "From Humata · couldn't migrate", the failure line "Couldn't migrate the daemon on {machine}", and a "Migrate again" button. The draft text says: "Fix what the log names, then Migrate again. Migrating again without a fix fails the same way." The draft also says "If a step fails, 2.0.4 keeps running and nothing is lost." (desktop/tests/design/upgrade.rs; the code's failure wording is in request_card.rs and features/hosts/src/add.rs:410 "Open the Migrate item in the Inbox to see why, then try again.")
- **Offered:** Migrate again.
- **Action:** Click Migrate again.
- **Result:** The item moves to migrating and asks the daemon again (migrate.rs:291-325 `a_failed_migrate_shows_the_failure_with_migrate_again`).
- **Evidence:** Store-level; draft frame 80 (design/upgrade.rs `failed`), no PNG.
- **Replay check:** `bazel test --jobs=4 //apps/humata2/features/inbox:migrate_test --test_filter=a_failed_migrate_shows_the_failure_with_migrate_again`

### 5. A withdrawn item leaves the list
- **Screen:** The card is gone.
- **Offered:** None.
- **Action:** The mismatch goes away (the daemon withdraws the item).
- **Result:** The item leaves the waiting list (migrate.rs:350-370 `a_withdrawn_item_leaves_the_waiting_list`; upgrade.rs:258 `a_withdrawn_item_leaves_the_page_current`).
- **Evidence:** Store-level.
- **Replay check:** `bazel test --jobs=4 //apps/humata2/features/inbox:migrate_test --test_filter=a_withdrawn_item_leaves_the_waiting_list`

### 6. This Mac's own daemon is old: the app holds its own Migrate item
- **Screen:** The Inbox shows "Migrate the daemon on This Mac" as the app's own item, until the local daemon can hold one (D374). Its Migrate runs through the local daemon client, not as an answer (migrate.rs:370-411 `an_old_local_daemon_with_a_staged_release_shows_the_apps_own_migrate_item`, `the_apps_own_item_migrates_on_this_mac_not_through_an_answer`).
- **Offered:** Migrate.
- **Action:** Click Migrate on the app's own item.
- **Result:** Shows Migrating, then the Inbox waits (bounded) for This Mac's daemon at the staged release. When it is back, the app asks to reconnect and the item ends Migrated. Past the bound it fails (D376; migrate.rs:510-580).
- **Evidence:** Store-level only.
- **Replay check:** `bazel test --jobs=4 //apps/humata2/features/inbox:migrate_test --test_filter=the_apps_own_item_ends_migrated_and_asks_for_a_reconnect`

### 7. The machine page while its daemon waits for Migrate
- **Screen:** The machine page's Upgrade banner (ui/src/machine_page.rs:1510-1565): "Migrate the daemon on {machine} first. Nothing was started or sent; its sessions keep running." with "Open in Inbox". The page is read-only: New session and Open a shell are refused.
- **Offered:** Open in Inbox (hands the item to the parent), Open a session (still works, upgrade.rs:166 `opening_a_session_still_works_while_read_only`).
- **Action:** Open a machine whose daemon waits.
- **Result:** The page is read only (upgrade.rs:143-180).
- **Evidence:** Store-level; draft frame 80 (design/upgrade.rs `machine_read_only`, "80-machine-read-only" in design.rs:265), no PNG.
- **Replay check:** `bazel test --jobs=4 //apps/humata2/features/hosts:upgrade_test --test_filter=while_read_only_new_session_and_open_shell_are_refused`

### 8. A session tab's typing while the daemon waits
- **Screen:** The session tab says "Migrate the daemon on {machine} first. What you type isn't sent until then; the agent keeps running." (ui/src/session_tabs.rs:565). Typing is held, not sent.
- **Offered:** Open in Inbox.
- **Action:** Type into a session on the machine whose daemon waits.
- **Result:** Typing is not sent until the migrate is done; the agent keeps running.
- **Evidence:** PENDING RUN (no native test renders the tab with a Migrate item; the tab's text is in ui/src/session_tabs.rs:565 only).
- **Replay check:** none exists. Phase 2 should ask the coach for a focused desktop test, or record this as untested.

### 9. An action refused with migration_needed
- **Screen:** The refusal routes to the Migrate item (upgrade.rs:276-317 `a_migration_needed_refusal_routes_to_its_item`, `a_refusal_that_names_no_item_routes_to_the_machines_item`).
- **Offered:** Open in Inbox.
- **Action:** Start or type on a machine at an old release.
- **Result:** The refusal says the machine must be migrated first and points to the item. With no item named, it points to the machine's item (upgrade.rs:298-317). With no item at all, Open in Inbox hands nothing on (upgrade.rs:317).
- **Evidence:** Store-level.
- **Replay check:** `bazel test --jobs=4 //apps/humata2/features/hosts:upgrade_test --test_filter=a_refusal_that_names_no_item_routes_to_the_machines_item`

### 10. Migrating and migrated on the machine page
- **Screen:** While migrating: Status "Migrating" with "Migrating to {to}… Sessions keep running; the link drops for a few seconds." (machine_page.rs:1510-1516). After approval: "Migrated to {release} · {time}" (machine_page.rs:1602).
- **Offered:** The page stays read only while migrating; after the migrate it works again (upgrade.rs:215-258).
- **Action:** Migrate from the Inbox (step 2).
- **Result:** The page shows Migrated, then works again.
- **Evidence:** Store-level; draft frames 79 and 81 (`machine_migrating`, `machine_migrated`), no PNG.
- **Replay check:** `bazel test --jobs=4 //apps/humata2/features/hosts:upgrade_test --test_filter=after_approval_the_page_shows_migrated_and_works_again`

### 11. After a failed migrate on the page
- **Screen:** The page shows the failure and stays read only (upgrade.rs:225-242 `after_a_failure_the_page_shows_it_and_stays_read_only`).
- **Offered:** Open in Inbox (to Migrate again).
- **Action:** None.
- **Result:** Read only, failure shown.
- **Evidence:** Store-level.
- **Replay check:** `bazel test --jobs=4 //apps/humata2/features/hosts:upgrade_test --test_filter=after_a_failure_the_page_shows_it_and_stays_read_only`

### 12. Upgrade steps for the sessions on the holders
- **Screen:** Draft frame 83 "Migrate holders on gpu-box" with the "Host tools" and "Installs" lines and "Restarting humatad on gpu-box…" (design/upgrade.rs `holders`), no PNG.
- **Offered:** Migrate (the holder item).
- **Action:** A holder item is waiting.
- **Result:** Per the draft, the holders restart with the daemon; the README records that sessions on an old holder do not move to the new one when idle (item 77, UPGRADE U6).
- **Evidence:** Not possible without a golden; code-read only.
- **Replay check:** none for the holders card. The nearest store test is `a_daemon_held_item_for_the_same_daemon_replaces_the_apps_own` (migrate.rs:433), which checks the daemon's item replacing the app's own item, not the holders card.

## Not reachable headless

- Cards and page banners in frames 77 to 83 have no PNG and no native test. They are drafts; the design test renders them only with a golden. Do not run `design_test_record` (flows rules). The coach decides whether a phase 2 run of `//apps/humata2/desktop:design_test --test_filter=77-inbox-migrate` (without record) is allowed to write a `.new.png` under `bazel-testlogs/`.
- The real daemon migrate (a second humatad at another release) is not in any live test in this record.
- The machine-level "Upgrade" from This Mac's own daemon in the real app, with the app's restart, is in the migrate tests as a store flow only.

## Surprises

- Gap 34 (flows/cases/README.md): writes and moves to a machine at another release are not refused. Only start, type and writable attach are. The page's "Nothing was started or sent" and UPGRADE.md:48 say otherwise. Facts: file forwarding (files/forward.rs:471-480) and moves (move_session.rs:162) do not call `machine_ready`.
- The migrate card's body says "Sessions keep running; the link drops for a few seconds." The machine page's refusal text says "Nothing was started or sent". A person reads "sessions keep running" and "nothing was sent" in two places; both are true for start and type but not for file writes (gap 34).
- The app's own Migrate item is not an answer: it is a button that migrates This Mac locally, so a person who looks for Allow or Deny for it finds only Migrate. That is intended (migrate.rs:370-411), but the card's group is still "Humata".
- The Inbox waits "bounded" for This Mac's daemon to come back (D376). Whether the card says how long it waits is not checked in phase 1; phase 2 should look at the migrating text for the bound.
- `humata` has no migrate command. A stale daemon's CLI refusal is `migration_needed` with exit 7 and points to the Inbox; nothing in the CLI migrates it.
- The draft failure text says "Migrating again without a fix fails the same way." That is a claim about the daemon's retry; the code's retry (migrate.rs:291) only sends Approve again.
- The test files say frames 77 to 81 (migrate.rs header), 80 to 82 (upgrade.rs header), and 77 to 83 (design.rs). The numbering differs by file; the design goldens are the drafts.

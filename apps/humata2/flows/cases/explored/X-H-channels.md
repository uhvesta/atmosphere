# Channels: the New channel sheet, the repo library, the channel page, adding a repo, a channel's Desk

Layer: explored · macOS · harness: `//apps/humata2/desktop:native_test` (new_channel::, channel::, channel_desk::, channel_desk_target::, overlays for the error frame) and `//apps/humata2/desktop:live_flows_test` (new_channel, repo_library::, channel_add_repo::, session_add_repo::, channel_page::, channel_desk::)

Phase 1 (map from code and snapshots, no Bazel run). Every step marked PENDING RUN is filled in phase 2.

## Setup

- Native: the design workspace (`fixture::design()`); the New channel sheet is over Home with the stopped cloud VM (`cloud-vm-3`) removed (`sheet_over_home()` in `desktop/tests/native/new_channel.rs`). The fresh channel `web-revamp` (`fixture::fresh_channel()`) is the empty channel.
- Live: a real `humatad2` with a fake `gh` that sees every repo the world makes (`World::repo`), This Mac online. `World::start_holding_clones()` holds the daemon's `git clone` so the progress sheet can be seen.
- Snapshots used: `desktop/tests/__snapshots__/native/{44-new-channel,18-channel-sessions,19-channel-activity,20-channel-repos-machines,35-empty-channel,new-channel-error-light}.view.txt` (and PNGs). The error frame is produced by `desktop/tests/native/overlays.rs`.

## Steps

### 1. Open the New channel sheet
- **Screen:** the sheet "New channel" over Home. Fields: "Channel name" (placeholder "web") and "Search repos". Repo check boxes (`acme/ledger`, `acme/api`, `acme/web`, `acme/infra`, `acme/docs`, and `octo/notes` in the error frame). Machine check boxes: "This Mac" (ticked by default), "devbox-1", "gpu-box" (offline, not ticked), "cloud-vm-3" in the error frame. Buttons: Cancel, and the create button.
- **Offered:** Channel name, Search repos, each repo check box, each machine check box (an offline one does nothing), Cancel, Create.
- **Action:** ⇧⌘N, or New channel in the sidebar, or the Home "New session" path.
- **Result:** The sheet opens over Home with This Mac ticked. The folder the name gives is never shown on the sheet (PL-9).
- **Evidence:** `desktop/tests/__snapshots__/native/44-new-channel.view.txt` (and `44-new-channel.png`).
- **Replay check:** `bazel test --jobs=4 //apps/humata2/desktop:native_test --test_filter=new_channel::` (includes `new_channel_frame_44`, `an_offline_machine_does_not_tick`, `create_with_no_repos_opens_the_channel`); expected: pass.

### 2. Search the repos and tick them
- **Screen:** after typing "acme" in Search repos, the list narrows to the matches. A ticked repo stays at the top when the search changes. The Create button reads "Create channel and clone 2 repos" with two repos ticked (the label counts ticked repos: 0 gives "Create channel", 1 gives "... clone 1 repo", n gives "... clone n repos", from `features/app/src/new_channel.rs`).
- **Offered:** the search field, each check box, Create.
- **Action:** Type "acme"; tick `acme/ledger` and `acme/api`; tick `devbox-1`.
- **Result:** The sheet holds those two repos and two machines. The offline `gpu-box` does not tick.
- **Evidence:** `44-new-channel.view.txt`.
- **Replay check:** `--test_filter=new_channel::` (frame 44 above); live: `bazel test --jobs=4 //apps/humata2/desktop:live_flows_test --test_filter=new_channel` (includes `a_channel_made_from_the_new_channel_sheet_clones_its_repos_and_opens_empty`); expected: pass.

### 3. Whole-word matches come first
- **Screen:** search "es": `octo/notes` comes before `acme/docs` (a whole-word match before a scattered one). With no search, the list keeps the GitHub order.
- **Offered:** the same list.
- **Action:** Type "es" in Search repos.
- **Result:** The row order changes; the window draws the whole match first.
- **Evidence:** none.
- **Replay check:** `bazel test --jobs=4 //apps/humata2/desktop:live_flows_test --test_filter=new_channel_sheet` (includes `whole_word_matches_come_before_scattered_ones_in_the_repo_search`); expected: pass.

### 4. Create with no name: the error
- **Screen:** the sheet with an alert "Give the channel a name." and a Dismiss button. Name field empty (placeholder "web").
- **Offered:** Dismiss, Cancel, Create channel.
- **Action:** Create with an empty name (the name is trimmed, and a leading "#" is removed).
- **Result:** The sheet stays open with the error; nothing is created.
- **Evidence:** `desktop/tests/__snapshots__/native/new-channel-error-light.view.txt` (produced by `desktop/tests/native/overlays.rs`).
- **Replay check:** `bazel test --jobs=4 //apps/humata2/desktop:native_test --test_filter=new_channel_error_light` (test `new_channel_error_light` in `desktop/tests/native/overlays.rs`, line 149); expected: pass.

### 5. Create the channel with repos (progress, then the channel opens)
- **Screen:** after Create, the sheet becomes the progress sheet: one clone row per repo, none done, and the note "The channel opens when every clone is done." The channel page opens when the clones finish.
- **Offered:** Cancel (if it is still there); nothing else while clones run.
- **Action:** Name "billing-exports", tick `owner/api` and `owner/ledger`, "Create channel and clone 2 repos".
- **Result:** Progress sheet while the clones run. The channel is listed with its two repos, and the disk holds `main/api/.git` and `main/ledger/.git`, not `main/web`.
- **Evidence:** none for the progress sheet (the live test holds the clone on a pipe).
- **Replay check:** `bazel test --jobs=4 //apps/humata2/desktop:live_flows_test --test_filter=new_channel_sheet` (includes `the_progress_sheet_shows_while_the_clones_run_then_the_channel_opens`); expected: pass.

### 6. Create with no repos
- **Screen:** the button reads "Create channel".
- **Offered:** Create channel.
- **Action:** Name "Notes", Create.
- **Result:** The channel "notes" is created with no repos (the name is lower-cased in the channel), and the sheet closes.
- **Evidence:** none.
- **Replay check:** `--test_filter=new_channel::` (includes `create_with_no_repos_opens_the_channel`); expected: pass.

### 7. The repo library: a repo added once is first
- **Screen:** the sheet has a "Library" group first, then GitHub's group. A repo the library holds is listed even with GitHub signed out.
- **Offered:** tick a library repo.
- **Action:** Create a first channel with `owner/api` (puts it in the library), sign gh out, open a second New channel.
- **Result:** `owner/api` is listed under Library. A repo never added (`owner/ledger`) is not listed while GitHub is out of reach. Ticking `owner/api` and creating the second channel clones it again in `second/main/api/.git`.
- **Evidence:** none.
- **Replay check:** `bazel test --jobs=4 //apps/humata2/desktop:live_flows_test --test_filter=repo_library::` (includes `a_repo_added_once_is_listed_first_in_the_next_new_channel_even_with_github_out_of_reach`); expected: pass.

### 8. Signed-out gh: Retry, not an empty list
- **Screen:** the sheet says why repos can't be listed and offers Retry. It shows no list, not an empty one.
- **Offered:** Retry, Cancel.
- **Action:** Open the sheet with gh signed out.
- **Result:** The error text is shown, no repo is listed.
- **Evidence:** none.
- **Replay check:** `bazel test --jobs=4 //apps/humata2/desktop:live_flows_test --test_filter=new_channel` (includes `a_signed_out_gh_shows_the_sheets_signed_out_state_not_an_empty_list`); expected: pass.

### 9. The channel page: the tabs and the Sessions tab
- **Screen (`18-channel-sessions`):** header with "New session", "Pin this channel", and a disabled "More channel actions". Tabs: Sessions (selected), Inbox, Board, Repos and machines, Agent access, Activity. The Sessions list has a filter ("Filter sessions") and the segments Open (selected), Archived, All, plus "Expand all" and "Collapse all". Rows: session names (Ledger schema migration, Try BRIN index, Partial BRIN on posted_at, Review the schema, Refund retries, API rate limiter), each with a fork-count button ("2", "1") described as "1 from this session, 1 from forks", and "Fold session forks".
- **Offered:** New session, Pin this channel, the tabs, the filter, Open/Archived/All, Expand all, Collapse all, a fork count, Fold session forks, a row (opens the session).
- **Action:** Open the channel from the sidebar; type in the filter; fold `ledger`; expand all; click the Inbox count on `ledger`.
- **Result:** The filter narrows the rows; fold removes the forks; Expand all shows them; the Inbox count opens that session on its Inbox panel.
- **Evidence:** `desktop/tests/__snapshots__/native/18-channel-sessions.view.txt` (and PNG).
- **Replay check:** `bazel test --jobs=4 //apps/humata2/desktop:native_test --test_filter=channel::` (includes `channel_sessions_filter_fold_and_open_their_own_inbox`, `channel_page_reference_images`); expected: pass.

### 10. Pin a channel from the palette
- **Screen:** ⌘K palette lists the channel; the channel page has "Pin this channel". Pinned channels appear in the sidebar's Pinned section.
- **Offered:** the palette entry (Return runs it), Pin this channel, and from Home the Pinned row.
- **Action:** ⌘K, type the channel name, Return; then Pin; then from Home click its Pinned row.
- **Result:** The palette closes when its entry runs; the channel's page opens on Sessions; the pin is kept on the host; the Pinned row opens the channel.
- **Evidence:** none.
- **Replay check:** `bazel test --jobs=4 //apps/humata2/desktop:live_flows_test --test_filter=channel_page::` (includes `the_palette_opens_a_channel_and_its_pin_is_kept_on_the_host`, `a_channels_sessions_tab_lists_its_session_with_its_state`); expected: pass.

### 11. Repos and machines tab
- **Screen (`20-channel-repos-machines`):** a clone table: rows of machines (This Mac, devbox-1) and cells "2 clones", "4 clones", "2 clones", "3 clones". Buttons: "Add a GitHub repo…" (shown disabled in this snapshot, see Surprises) and "Add a machine…".
- **Offered:** each clone cell (filters the Sessions tab to that clone), Add a GitHub repo…, Add a machine….
- **Action:** Click the `acme/ledger` · devbox-1 cell.
- **Result:** The Sessions tab opens filtered to that clone (only its sessions show); The clear control (`channel-clear-clones`) restores the full list.
- **Evidence:** `desktop/tests/__snapshots__/native/20-channel-repos-machines.view.txt`.
- **Replay check:** `--test_filter=channel::` (includes `channel_clone_cells_and_activity_are_actionable`, `channel_page_reference_images`); expected: pass.

### 12. Activity tab
- **Screen (`19-channel-activity`):** events with actions: "Asks to move PAY-3 to Done" (Review), "Forked “Try BRIN index” → “Partial BRIN on posted_at” (code only)", "Finished a turn: +184 −24 across 2 repos" (Diff), "Created PAY-9 “Backfill progress bar”", "gpu-box went offline", "Pushed rate-limiter and opened PR #491" (Open PR).
- **Offered:** Review, Diff, Open PR, and each event row (opens its session).
- **Action:** Click the first event row.
- **Result:** The session opens on its Inbox panel.
- **Evidence:** `desktop/tests/__snapshots__/native/19-channel-activity.view.txt`.
- **Replay check:** `--test_filter=channel::` (includes `channel_clone_cells_and_activity_are_actionable`); expected: pass.

### 13. An empty channel: three steps
- **Screen (`35-empty-channel`):** the Sessions tab lists three steps: "Add a repo" ("Sessions get their own clone of each repo you add." with "Add a GitHub repo…"), "Pick machines" (with "Add a machine…"), "Start a session" (with "New session"). The header's "More channel actions" is disabled. The "Add a GitHub repo…" button is shown disabled in the snapshot (see Surprises).
- **Offered:** Add a GitHub repo…, Add a machine…, New session.
- **Action:** Click New session, then Escape; click Add a machine….
- **Result:** New session opens the New session overlay (`new-session-create` is present). Add a machine opens the add-machine sheet (`add-machine-start`).
- **Evidence:** `desktop/tests/__snapshots__/native/35-empty-channel.view.txt`.
- **Replay check:** `--test_filter=channel::` (includes `an_empty_channel_offers_a_session_and_a_machine`); expected: pass. The test does not click "Add a GitHub repo…" (see Surprises).

### 14. Add a repo to a channel from its Repos tab
- **Screen (`ui/src/overlay_channel_add_repo.rs`):** sheet "Add a GitHub repo" with the one repo search: the Library group first, then GitHub. The sheet shows no path and no local folder.
- **Offered:** the search field, each repo (a click adds it), Cancel.
- **Action:** On a second channel, Repos and machines, "Add a GitHub repo…", click `owner/ledger` (in the Library group).
- **Result:** The sheet closes; the channel lists `owner/ledger`; its clone is at `second/main/ledger/.git`.
- **Evidence:** none.
- **Replay check:** `bazel test --jobs=4 //apps/humata2/desktop:live_flows_test --test_filter=channel_add_repo::` (includes `a_repo_from_the_library_is_added_to_an_existing_channel_from_its_repos_tab`); expected: pass.

### 15. Add a repo to a running session (session menu)
- **Screen (`ui/src/overlay_session_add_repo.rs`):** sheet "Add repo to this session" with the repo search and an "Add repo" button ("Adding…" while it runs).
- **Offered:** the search, Add repo, Cancel.
- **Action:** A running channel session's menu, "Add repo to this session…", pick a repo the channel lacks, Add repo.
- **Result:** The repo is cloned in the session on the session's branch, added to the channel, the agent is told, and the repo is listed among the session's clones.
- **Evidence:** none.
- **Replay check:** `bazel test --jobs=4 //apps/humata2/desktop:live_flows_test --test_filter=session_add_repo::` (includes `a_repo_added_to_a_running_session_is_cloned_in_it_told_to_the_agent_and_joins_the_channel`); expected: pass.

### 16. A channel's own Desk: the tab and New desk session
- **Screen (`ui/src/channel_desk.rs`, `overlay_desk_channel.rs`):** the channel's Desk tab lists its open Desk sessions under the "New desk session" button; the empty state has its own button. The New desk session dialog "New desk session" with subtitle "in # <channel>", rows: Target ("This channel" or "A session", the second with a picker, "Covers that session and its forks, including forks made later."), Machine, and "May do": six switches (read files, send notes, type into the terminal, start and fork, stop and archive, change board items), each with a rule. A switch the channel's Agent access blocks is marked and links to the channel's Agent access tab.
- **Offered:** Target choice, machine menu, each switch, Start, Cancel, and on a session's info panel a link to the channel's Agent access for a blocked switch.
- **Action:** Channel's Desk tab, New desk session, type a title, Start; then Close session on it.
- **Result:** The session is listed on the channel's Desk tab and under the channel in the sidebar; Close session asks first, then the session is gone from both.
- **Evidence:** none (no channel-desk snapshot in the native set; frames 91 to 95 are not among them).
- **Replay check:** native `--test_filter=desk::` (covers `the_desk_tab_lists_the_sessions_and_the_sidebar_has_its_row`, `new_desk_session_offers_the_switches_and_marks_a_blocked_one`, `close_session_asks_first_then_the_session_is_gone`, `a_blocked_switch_in_the_info_panel_links_to_the_channels_agent_access`); live `bazel test --jobs=4 //apps/humata2/desktop:live_flows_test --test_filter=channel_desk::`; expected: pass.

### 17. A channel's Desk session: closing says where the folder goes
- **Screen:** the close question for a channel's Desk session always asks (CD-4). It says the folder stays under Closed when it holds a version (button `channel-desk-close-kept`), or goes to the trash when it has none (button `channel-desk-close-trash`).
- **Offered:** the two confirm buttons, Cancel.
- **Action:** Close a channel Desk session with a version, then one with none.
- **Result:** Kept with a version, trash with none.
- **Evidence:** none.
- **Replay check:** `--test_filter=desk::` (covers `docv_closing_a_session_with_a_version_says_the_folder_is_kept`, `docv_closing_a_session_with_no_version_says_the_folder_goes_to_the_trash`); expected: pass.

## Not reachable headless

- Reopen of a closed channel Desk session: the fake daemon cannot reopen one (`desktop/tests/native/channel_desk.rs`). Its Reopen row is in `channel_desk_reopen.rs`, which drives only the click through the Desk tab.
- A repo the library never held while GitHub is signed in: the GitHub group needs gh; the live tests cover it with the fake gh only.
- The progress sheet's "failed" row (a clone that fails) has a retry path; no test drives a failing clone here.

## Surprises

- **"Add a GitHub repo…" shows as disabled in the snapshots.** `20-channel-repos-machines.view.txt` and `35-empty-channel.view.txt` both mark the button `disabled`. The code (`ui/src/channel.rs` at the Repos tab and the empty step) sends `ChannelAddRepo::Open` with no disabled flag, and the empty-channel test never clicks it. Phase 2 should click it once to see if it opens the sheet (the live `channel_add_repo::` test opens it from the Repos tab and clicks a library repo, so it can open).
- **The New channel Create label counts repos.** "Create channel" with no repos, "Create channel and clone 1 repo", "... clone N repos". A person who ticks one repo sees "clone 1 repo".
- **Create with an empty name shows an alert that names the fix.** It says only "Give the channel a name." and Dismiss, with the sheet still open. The name is trimmed and a leading "#" removed before the check.
- **Channel names are lower-cased.** "Notes" becomes the channel "notes" (live test `create_with_no_repos_opens_the_channel`). The sheet does not show that.
- **Two Add-a-repo sheets with different titles.** The channel sheet is "Add a GitHub repo"; the session sheet is "Add repo to this session"; the Desk folder bar's sheet is a plain "Repo to add" field (`X-H-desk-page.md`, step 12). The three take repos three ways.
- **A channel's Desk sessions are in no channel's Sessions tab and not on the Desk page.** They appear on the channel's Desk tab, under the channel in the sidebar, and in the info panel of a session they look after (`features/desk/src/channel.rs` header; `desktop/tests/native/channel_desk.rs` `the_info_panel_lists_the_desk_sessions_looking_after_a_session`). Facts only.
- **Channel Desk close words differ from the Desk page.** Channel: "stays under Closed" or "goes to the trash". Desk page: "deleted in N days". The channel's close question does not give the day count.
- **No way to move a Desk session into a channel from the UI.** The only related actions are the channel's own "New desk session" and the session menu's "New desk session for this session" (`ui/src/pages.rs`), which start a new session.

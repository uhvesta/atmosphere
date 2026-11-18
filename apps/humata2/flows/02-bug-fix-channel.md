# 2. A bug to fix: find the repos and make a channel

> "Hey, I got pinged to fix this bug. Let's find the related repos and maybe make a channel."

The bug touches billing exports. I think it spans `acme/api` and `acme/ledger`, and I'll want a branch per repo, a review, and pull requests at the end. That calls for a channel: a home for the repos, with one session per piece of work.

How to read this file, and what the tags mean: [README](README.md).

## Starting state

- Humata is open. `gh` on This Mac is signed in and sees `acme/api`, `acme/ledger`, `acme/web` and others.
- There's no `billing-exports` channel and no `~/Humata/billing-exports/` folder.
- This Mac is online. gpu-box is a linked machine, but offline.

## Steps

1. **Do:** click the **+** beside Channels in the sidebar (or press ⇧⌘N).
   **See:** **New channel** (frame 44), with **Name**, **Repos** and **Machines**.
   **Expect:** nothing changes yet.
   **Status: Works.** `desktop/tests/live_flows/new_channel.rs::a_channel_made_from_the_new_channel_sheet_clones_its_repos_and_opens_empty` presses ⇧⌘N on a real daemon and sees the sheet with This Mac ticked. Fake-client version: `desktop/tests/native/new_channel.rs::new_channel_frame_44`.

2. **Do:** type `billing-exports` under Name.
   **See:** no folder line; the dialog names the machines the channel goes on (PL-9).
   **Status: Works.** `desktop/tests/live_flows/new_channel.rs::a_channel_made_from_the_new_channel_sheet_clones_its_repos_and_opens_empty`. Fake-client version: `new_channel.rs::new_channel_frame_44`.

3. **Do:** under **Repos**, type "export". Tick `acme/api` and `acme/ledger`. Scroll down.
   **See:** the field starts with my scope as chips (`acme ✕`, `you ✕`), and before I type, the list shows my repo library, most recently used first (RL-3, RL-8). As I type, the list narrows to two groups: **Library**, then **GitHub · acme, you**, with whole-word matches first (RL-6). More repos page in as I scroll. If `gh` is signed out, the GitHub group says so and how to fix it, and the library still answers (RL-10).
   **Branch:** the fix needs `acme-labs/rounding`, which is outside my scope. I click **+ Org**, type `acme-labs`, and tick it. Closing the sheet drops the `acme-labs` chip unless I click **Save as default** (RL-8).
   **Expect:** creating the channel puts all three repos in the library, so on devbox-1 or in the browser they are offered without being typed again (RL-1, RL-2). **Works** in the daemon: `daemon/humatad/tests/repo_library_adds.rs::a_channel_created_with_a_repo_leaves_the_repo_in_the_library`; the next picker offering it **Works** live: `desktop/tests/live_flows/repo_library.rs::a_repo_added_once_is_listed_first_in_the_next_new_channel_even_with_github_out_of_reach`. An agent's search on devbox-1 reaches the host's library and **Works** in the daemon: `daemon/humatad/tests/away_forward.rs::an_agents_repo_search_on_a_machine_goes_up_and_the_hosts_answer_is_the_agents` and `::away_an_agents_repo_search_is_host_unreachable_and_answers_nothing` (D568). The browser is cut.
   **Expect:** the repo list is cached for a minute, so typing again doesn't run `gh` again.
   **Status:**
   - **Works** in the client and daemon, with a scripted `gh`: `clients/live/tests/github_repos.rs::every_repo_gh_sees_comes_back_in_order_across_pages` and `::a_query_narrows_the_repos`; `daemon/humatad/tests/gh_repos.rs::a_query_keeps_the_fuzzy_matches_whole_ones_first`, `::a_listing_within_a_minute_does_not_run_gh_again` and `::a_signed_out_gh_is_a_failed_precondition_and_is_not_remembered`.
   - The picker **Works**: `desktop/tests/live_flows/new_channel.rs::a_channel_made_from_the_new_channel_sheet_clones_its_repos_and_opens_empty` lists the repos `gh` sees, narrows them as I type and keeps a ticked repo at the top. A signed-out `gh` shows the sheet's signed-out state with **Retry**, not an empty list: `desktop/tests/live_flows/new_channel.rs::a_signed_out_gh_shows_the_sheets_signed_out_state_not_an_empty_list`. Paging on scroll and the one-minute cache aren't driven from the app live (**Fakes only**, `new_channel.rs::new_channel_frame_44`). Fake-client version: `new_channel.rs::new_channel_frame_44`.
   - Suggesting the repos related to the bug: **Not built, no milestone.** Search is by name. An agent can propose a repo later (step 7).

4. **Do:** under **Machines**, leave This Mac ticked. gpu-box is listed but can't be ticked. Click **Create channel and clone 2 repos**.
   **See:** "Creating # billing-exports" (frame 45), one row per clone with its progress, and "The channel opens when every clone is done." A failed clone says why and offers **Retry**.
   **Expect:**
   - `~/Humata/billing-exports/` has `main/api` and `main/ledger`, `sessions/` and `shared/`.
   - A repo that doesn't exist fails the setup and leaves no half clone.
   - Events: Operation created, then updated per step, then Succeeded; Channel created.
   **Status: Works.** `clients/live/tests/parity.rs::new_channel_is_listed_on_real`; `daemon/humatad/tests/channels.rs::a_new_channel_gets_its_folders_and_a_main_clone` and `::a_repository_that_does_not_exist_fails_the_setup_and_leaves_no_half_clone`. The dialog **Works**: `desktop/tests/live_flows/new_channel.rs::a_channel_made_from_the_new_channel_sheet_clones_its_repos_and_opens_empty` clicks **Create channel and clone 2 repos** and finds real main clones of those two repos, with `sessions/` and `shared/`. The clone progress sheet **Works**: `desktop/tests/live_flows/new_channel_sheet.rs::the_progress_sheet_shows_while_the_clones_run_then_the_channel_opens`. Still **Fakes only**: an offline machine that can't be ticked (the rig has one machine; WIRE-A later): `new_channel.rs::an_offline_machine_does_not_tick` and `::create_with_no_repos_opens_the_channel`.

5. **Do:** wait for the channel page.
   **See:** `#billing-exports` on **Sessions**, empty, offering **New session** and **Add a machine…**.
   **Status: Works.** `desktop/tests/live_flows/new_channel.rs::a_channel_made_from_the_new_channel_sheet_clones_its_repos_and_opens_empty` opens the empty `#billing-exports` page on Sessions and finds **New session**. **Add a machine…** isn't checked live (**Fakes only**: `desktop/tests/native/channel.rs::an_empty_channel_offers_a_session_and_a_machine`).

6. **Do:** click **New session** (frame 31). Type the title "Fix export rounding", leave Runs on Claude Code and Machine on This Mac, and paste the bug report into **First message**. Click **Start session**.
   **See:**
   - Both repos under **Repos** with "Own clone, new branch from main", and "Branch: you/fix-export-rounding".
   - Then the session page, with the agent already on my message.
   **Expect:**
   - Each repo is cloned into `sessions/fix-export-rounding/` on branch `you/fix-export-rounding`.
   - The agent gets the first message once.
   - Events: Session created (Starting), then updated (Working).
   **Status: Works.** `desktop/tests/live_flows.rs::a_session_started_from_new_session_runs_and_answers_its_first_message` (real daemon and holder, scripted agent); `daemon/humatad/tests/sessions.rs::a_started_session_has_its_clone_on_a_branch_named_from_its_title`.

7. **Do:** the agent finds the bug also lives in `acme/web` and asks to add it to the channel. Open the Inbox and click **Allow** on its card (frame 87).
   **See:** a card naming `acme/web` and the channel, with **Allow** and **Deny**.
   **Expect:**
   - Event: InboxRequest created (Pending), then updated (Approved).
   - The repo's main clone is made, and the agent's call returns.
   - Event: Channel updated.
   **Status:** the daemon side **Works**: `daemon/humatad/tests/channel_repo_propose.rs::an_agents_add_repo_becomes_an_inbox_request_under_the_repo_add_rule` and `::approving_the_proposal_adds_the_repo`. Answering Inbox requests in the app works live for other kinds (`desktop/tests/live_inbox.rs::a_request_allowed_from_the_requesters_session_returns_the_agents_waiting_call`). The repo-proposal card: **Not built: ROAM.**

8. **Do (instead of 7):** on **Repos and machines**, click **Add a GitHub repo…** and type `web`. `acme/web` is under **Library** if I've used it anywhere before, and under **GitHub** otherwise (RL-5). Pick it.
   **See:** an `acme/web` row whose cell per machine fills in as each clone finishes.
   **Expect:** the main clone is made. Existing sessions don't change. Event: Channel updated.
   **Status:** **Works**: `desktop/tests/live_flows/channel_add_repo.rs::a_repo_from_the_library_is_added_to_an_existing_channel_from_its_repos_tab`, with the daemon's main clone in `daemon/humatad/tests/channel_add.rs::a_repo_added_to_a_channel_gets_a_main_clone_and_the_first_is_untouched` and the library record in `daemon/humatad/tests/repo_library_adds.rs::a_repo_added_to_a_channel_is_recorded_and_a_repo_that_fails_is_not`. No test watches the per-machine cell fill in, since the live rig has one machine.

9. **Do:** in the session's menu, pick **Add repo to this session** › `acme/web`.
   **See:** a `web` group in Diff and Files.
   **Expect:** `acme/web` is cloned on the session's branch, and the agent is told nothing about it (D743). Event: Session updated (repos).
   **Status: Not built, no milestone** (CH-21, §26.21 step 3). The daemon answers "not built" for AddRepoToSession. ROAM only waits for it.

10. **Do:** when the agent says it's done, open **Diff**.
    **See:** changes grouped by repo, then by file.
    **Status: Works.** `desktop/tests/live_flows/review.rs::a_three_repo_diff_is_reviewed_commented_on_and_marked_reviewed` (real daemon, real git clones, steps sent as app actions rather than clicks). The clicks **Work** too: `desktop/tests/live_flows/review.rs::the_diff_panel_base_menu_line_comments_and_push_work_by_clicking` clicks the base menu's choices and **Mark reviewed**. Fake-client version: `desktop/tests/native/review.rs::the_base_menu_picks_what_the_diff_is_against_and_mark_reviewed_starts_it_over`.

11. **Do:** click a line and type "round half to even here". Add two more comments, then click **Send 3 comments**.
    **See:** each comment as a draft under its line, then sent.
    **Expect:**
    - All three are typed into the agent's terminal as one paste.
    - Events: Comment created (Draft), then updated (Sent).
    **Status: Works.** Same live test. The clicks **Work** too: `desktop/tests/live_flows/review.rs::the_diff_panel_base_menu_line_comments_and_push_work_by_clicking` clicks two lines' gutters, types each comment and clicks **Send 2 comments**. Fake-client version: `desktop/tests/native/review.rs::clicking_a_diff_line_opens_the_box_under_it_and_comment_types_it_at_once` and `::drafted_comments_show_in_the_panel_and_send_together`.

12. **Do:** click **Mark reviewed**.
    **See:** the diff is empty until the agent changes something. Then only the new change shows.
    **Expect:** the review base is now "since last reviewed".
    **Status: Works.** Same live test.

13. **Do:** click **Push** on the `api` group.
    **See:** the group shows its pull request number. `ledger` stays unpushed. A failed push shows on its group with **Try again**.
    **Expect:**
    - The branch is committed and pushed, and a pull request is opened for `api` only.
    - Events: Operation Running, then Succeeded; Session updated.
    **Status: Works.** Same live test, step 8 (a local bare remote and a scripted `gh`). The button **Works** by click on a one-repo session: `desktop/tests/live_flows/review.rs::the_diff_panel_base_menu_line_comments_and_push_work_by_clicking` clicks **Push & PR** with the remote gone (the group shows the failure), then again once it is back (the PR number shows). With several repos it **Works** by click too: each group's button is named for its repo (D388), and `desktop/tests/live_flows/review.rs::each_repo_group_pushes_its_own_branch_by_clicking_its_button` pushes one repo's branch with the other's remote untouched, then the other. Fake-client version: `desktop/tests/native/push.rs::each_repo_group_pushes_its_own_branch_and_names_its_pull_request` and `::a_failed_push_shows_on_its_group_with_try_again`.

14. **Do:** after the pull requests merge, pick **Archive session**. Then click **Undo** on the toast.
    **See:** with unpushed work, the dialog asks first and offers "Commit, push and open a PR, then archive" (frame 65). After archiving, a toast with **Undo**.
    **Expect:**
    - Plain archive is refused while work is unpushed.
    - Undo brings the session back exactly.
    - Events: Session updated (Archived), then back to its last status.
    **Status:**
    - Archive **Works** in the daemon: `daemon/humatad/tests/archive.rs::plain_archive_is_refused_while_there_is_unpushed_work` and `daemon/humatad/tests/push.rs::commit_push_pr_then_archive_archives_after_the_pull_request`. The menu, the session leaving the sidebar and the toast **Work** against a real daemon: `desktop/tests/live_flows/lifecycle.rs::archiving_a_session_from_its_menu_and_undoing_brings_it_back`. The unpushed-work dialog with "Commit, push and open a PR, then archive" **Works** against a real daemon: `desktop/tests/live_flows/review.rs::archiving_unpushed_work_asks_first_and_undo_brings_the_session_back` cancels it once, then commits, pushes, archives and clicks **Undo**. Fake-client version: `desktop/tests/native/fork_move.rs::archiving_unpushed_work_asks_first_as_in_frame_65` and `native/overlays.rs::archiving_offers_a_working_undo`. The live test takes the dialog's "Archive and delete the work" choice only if it appears.
    - Undo **Works**: the toast's **Undo** brings the session back, in `desktop/tests/live_flows/lifecycle.rs::archiving_a_session_from_its_menu_and_undoing_brings_it_back` and `clients/live/tests/parity.rs::an_archived_session_is_restored_on_real`. The daemon restores exactly (`archive.rs::undo_brings_the_session_back_exactly`).

## Expected outcome

`#billing-exports` exists with its repos. "Fix export rounding" has pushed branches and open pull requests, then is archived. Nothing unpushed was lost.

## Gaps this flow shows

- Adding a repo to a channel that exists works live; adding one to a single session works in the daemon (`daemon/humatad/tests/session_add_repo.rs`). No test watches the per-machine cells fill in, because the live rig has one machine.
- Undo after Archive now works against a real daemon. The Restore… page isn't tested live.
- There's no card for an agent's repo proposal (ROAM).
- Finding the related repos is a name search only.
- The repo library and scope (REPOS, RL-1 to RL-12) work, and a machine's agents search the host's library (D568). The user's own `humata repos search` on a machine is refused with a fix that points at the host: AWAY cut MachineUser forwarding, and no milestone brings it back.
- The New channel dialog runs against a real daemon, except its clone progress sheet and an offline machine (the rig has one machine).

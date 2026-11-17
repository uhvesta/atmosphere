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
   **Status: Fakes only.** `desktop/tests/native/new_channel.rs::new_channel_frame_44`.

2. **Do:** type `billing-exports` under Name.
   **See:** "Folder: ~/Humata/billing-exports/".
   **Status: Fakes only.** Same test.

3. **Do:** under **Repos** ("From gh on This Mac. Zero or more."), type "export". Tick `acme/api` and `acme/ledger`. Scroll down.
   **See:** the list narrows as I type, with whole-word matches first. More repos page in as I scroll. If `gh` is signed out, the list says so and how to fix it.
   **Expect:** the repo list is cached for a minute, so typing again doesn't run `gh` again.
   **Status:**
   - **Works** in the client and daemon, with a scripted `gh`: `clients/live/tests/github_repos.rs::every_repo_gh_sees_comes_back_in_order_across_pages` and `::a_query_narrows_the_repos`; `daemon/humatad/tests/gh_repos.rs::a_query_keeps_the_fuzzy_matches_whole_ones_first`, `::a_listing_within_a_minute_does_not_run_gh_again` and `::a_signed_out_gh_is_a_failed_precondition_and_is_not_remembered`.
   - The picker is **Fakes only** (`new_channel.rs::new_channel_frame_44`).
   - Suggesting the repos related to the bug: **Not built, no milestone.** Search is by name. An agent can propose a repo later (step 7).

4. **Do:** under **Machines**, leave This Mac ticked. gpu-box is listed but can't be ticked. Click **Create channel and clone 2 repos**.
   **See:** "Creating # billing-exports" (frame 45), one row per clone with its progress, and "The channel opens when every clone is done." A failed clone says why and offers **Retry**.
   **Expect:**
   - `~/Humata/billing-exports/` has `main/api` and `main/ledger`, `sessions/` and `shared/`.
   - A repo that doesn't exist fails the setup and leaves no half clone.
   - Events: Operation created, then updated per step, then Succeeded; Channel created.
   **Status: Works.** `clients/live/tests/parity.rs::new_channel_is_listed_on_real`; `daemon/humatad/tests/channels.rs::a_new_channel_gets_its_folders_and_a_main_clone` and `::a_repository_that_does_not_exist_fails_the_setup_and_leaves_no_half_clone`. The dialog is **Fakes only**: `new_channel.rs::an_offline_machine_does_not_tick` and `::create_with_no_repos_opens_the_channel`. Live app tests make their channel through the client, never this dialog.

5. **Do:** wait for the channel page.
   **See:** `#billing-exports` on **Sessions**, empty, offering **New session** and **Add a machine…**.
   **Status: Fakes only.** `desktop/tests/native/channel.rs::an_empty_channel_offers_a_session_and_a_machine`.

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

8. **Do (instead of 7):** on **Repos and machines**, click **Add a GitHub repo…** and paste `acme/web`.
   **See:** an `acme/web` row whose cell per machine fills in as each clone finishes.
   **Expect:** the main clone is made. Existing sessions don't change. Event: Channel updated.
   **Status: Not built, no milestone.** The button is drawn disabled (`ui/src/channel.rs`, "No action adds a repo to a channel yet"). The daemon does it (`daemon/humatad/tests/channel_add.rs::a_repo_added_to_a_channel_gets_a_main_clone_and_the_first_is_untouched`). NEWCH left CH-6 out of scope, and nothing else wires it.

9. **Do:** in the session's menu, pick **Add repo to this session** › `acme/web`.
   **See:** a `web` group in Diff and Files.
   **Expect:** `acme/web` is cloned on the session's branch, and the agent is told the repo and its path in one message. Event: Session updated (repos).
   **Status: Not built, no milestone** (CH-21, §26.21 step 3). The daemon answers "not built" for AddRepoToSession. ROAM only waits for it.

10. **Do:** when the agent says it's done, open **Diff**.
    **See:** changes grouped by repo, then by file.
    **Status: Works.** `desktop/tests/live_flows/review.rs::a_three_repo_diff_is_reviewed_commented_on_and_marked_reviewed` (real daemon, real git clones, steps sent as app actions rather than clicks). The clicks are **Fakes only**: `desktop/tests/native/review.rs::the_base_menu_picks_what_the_diff_is_against_and_mark_reviewed_starts_it_over`.

11. **Do:** click a line and type "round half to even here". Add two more comments, then click **Send 3 comments**.
    **See:** each comment as a draft under its line, then sent.
    **Expect:**
    - All three are typed into the agent's terminal as one paste.
    - Events: Comment created (Draft), then updated (Sent).
    **Status: Works.** Same live test. The clicks are **Fakes only**: `review.rs::clicking_a_diff_line_opens_the_box_under_it_and_comment_types_it_at_once` and `::drafted_comments_show_in_the_panel_and_send_together`.

12. **Do:** click **Mark reviewed**.
    **See:** the diff is empty until the agent changes something. Then only the new change shows.
    **Expect:** the review base is now "since last reviewed".
    **Status: Works.** Same live test.

13. **Do:** click **Push** on the `api` group.
    **See:** the group shows its pull request number. `ledger` stays unpushed. A failed push shows on its group with **Try again**.
    **Expect:**
    - The branch is committed and pushed, and a pull request is opened for `api` only.
    - Events: Operation Running, then Succeeded; Session updated.
    **Status: Works.** Same live test, step 8 (a local bare remote and a scripted `gh`). The buttons are **Fakes only**: `desktop/tests/native/push.rs::each_repo_group_pushes_its_own_branch_and_names_its_pull_request` and `::a_failed_push_shows_on_its_group_with_try_again`.

14. **Do:** after the pull requests merge, pick **Archive session**. Then click **Undo** on the toast.
    **See:** with unpushed work, the dialog asks first and offers "Commit, push and open a PR, then archive" (frame 65). After archiving, a toast with **Undo**.
    **Expect:**
    - Plain archive is refused while work is unpushed.
    - Undo brings the session back exactly.
    - Events: Session updated (Archived), then back to its last status.
    **Status:**
    - Archive **Works** in the daemon: `daemon/humatad/tests/archive.rs::plain_archive_is_refused_while_there_is_unpushed_work` and `daemon/humatad/tests/push.rs::commit_push_pr_then_archive_archives_after_the_pull_request`. The dialog is **Fakes only**: `desktop/tests/native/fork_move.rs::archiving_unpushed_work_asks_first_as_in_frame_65` and `native/overlays.rs::archiving_offers_a_working_undo`.
    - Undo and **Restore…**: **Not built, no milestone.** The daemon restores (`archive.rs::undo_brings_the_session_back_exactly`), but the live client answers "not in this release" for restoring a session.

## Expected outcome

`#billing-exports` exists with its repos. "Fix export rounding" has pushed branches and open pull requests, then is archived. Nothing unpushed was lost.

## Gaps this flow shows

- Once a channel exists, the app can't add a repo to it, either for the channel or for one session. A bug often grows past its first repos. Neither has a milestone.
- Undo after Archive, and Restore, fail against a real daemon. The daemon is ready; only the live client is missing.
- There's no card for an agent's repo proposal (ROAM).
- Finding the related repos is a name search only.
- The New channel dialog has never run against a real daemon in a test.

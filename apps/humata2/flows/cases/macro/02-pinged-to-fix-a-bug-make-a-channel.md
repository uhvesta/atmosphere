# A bug report becomes a channel over three repos, reviewed and pushed

Layer: macro. Platforms: macOS, Linux, web. (The keys shown are the macOS ones; Linux uses `Ctrl`.)

## Starting state

- Humata 2 is open on This Mac. Its daemon is running and the Home page is showing.
- `gh` is signed in on This Mac. It sees `acme/web`, `acme/api`, `acme/payments` and a few other repos.
- Machines: This Mac (online) and box-1 (online).
- No channel named `eu-checkout` exists. The sidebar's Channels section is visible and no sheet is open.
- I hold the report text: "checkout fails for EU users since this morning".

## Steps

1. **I do:** Press ⇧⌘N.
   **I see:** The New channel sheet opens over Home.
   **State and events:** The overlay stack gets a New channel sheet, and the app asks the daemon for the repos `gh` sees.
   **Status:** Works (real daemon). `desktop/tests/live_flows/new_channel.rs::a_channel_made_from_the_new_channel_sheet_clones_its_repos_and_opens_empty` presses ⇧⌘N in the headless app and the sheet opens. The binding is `ui/src/keys.rs:96`, and `ui/src/shell.rs:499` sends `StartNewChannel`.

2. **I do:** Nothing more. Look at the sheet.
   **I see:** Three rows labelled Name, Repos and Machines. This Mac is ticked. Under Repos it says "Listing repos from gh…", then the list appears, with the hint "From gh on This Mac. Zero or more."
   **State and events:** The sheet holds `machines = [This Mac]` and `listed = Some(repos)` once the daemon answers.
   **Status:** Works (real daemon). `desktop/tests/live_flows/new_channel.rs::a_channel_made_from_the_new_channel_sheet_clones_its_repos_and_opens_empty` sees This Mac ticked and the repos a scripted `gh` lists. A signed-out `gh` shows the sheet's signed-out state and Retry, not an empty list: `desktop/tests/live_flows/new_channel.rs::a_signed_out_gh_shows_the_sheets_signed_out_state_not_an_empty_list`. Fake-client version: `features/app/tests/new_channel.rs::opening_the_sheet_lists_the_repos_gh_sees_and_ticks_this_mac`. Not proven live: the real sheet on a real `gh`.

3. **I do:** Type `eu-checkout` in Channel name.
   **I see:** No folder line; the dialog names the machines the channel goes on (PL-9).
   **State and events:** The folder is derived from the name (lowercase, runs of other characters become one `-`).
   **Status:** Works (real daemon). `desktop/tests/live_flows/new_channel.rs::a_channel_made_from_the_new_channel_sheet_clones_its_repos_and_opens_empty`. Fake-client version: `features/app/tests/new_channel.rs::the_folder_follows_the_name`.

4. **I do:** Type `web` in Search repos.
   **I see:** The list narrows. Whole matches (`acme/web`) come before scattered ones. Repos I have already ticked stay at the top whether or not they match.
   **State and events:** `query` changes. `shown_repos()` keeps ticked repos first, then the matches.
   **Status:** Works (real daemon) for narrowing and for a ticked repo staying at the top: `desktop/tests/live_flows/new_channel.rs::a_channel_made_from_the_new_channel_sheet_clones_its_repos_and_opens_empty`. Whole matches before scattered ones Works (real daemon): `desktop/tests/live_flows/new_channel_sheet.rs::whole_word_matches_come_before_scattered_ones_in_the_repo_search`. Fake-client version: `features/app/tests/new_channel.rs::typing_narrows_the_list_by_fuzzy_match`, `whole_word_matches_come_before_scattered_ones`, `ticked_repos_show_first_even_when_they_do_not_match`. The ranking rule itself is also proven on the daemon side (see step 6).

5. **I do:** Tick `acme/web`, `acme/api` and `acme/payments`.
   **I see:** Three ticked rows. The primary button reads "Create channel and clone 3 repos".
   **State and events:** `repos` holds the three names in tick order.
   **Status:** Works (real daemon). `desktop/tests/live_flows/new_channel.rs::a_channel_made_from_the_new_channel_sheet_clones_its_repos_and_opens_empty` ticks two repos and the button reads "Create channel and clone 2 repos". Fake-client version: `features/app/tests/new_channel.rs::the_create_button_says_how_many_repos_it_will_clone`.

6. **I do:** Nothing. This is what the picker draws from.
   **I see:** The sheet shows at most 50 matches and, when there are more, says "and N more; type to narrow". It does not page. Paging happens between the client and the daemon.
   **State and events:** The daemon lists the repos for the signed-in user with `gh api --paginate user/repos?...affiliation=owner,collaborator,organization_member`, most recently pushed first, at most 1000. The live client reads 100 at a time and follows the page tokens.
   **Status:** Works (real daemon). `daemon/humatad/tests/gh_repos.rs::an_empty_query_lists_every_repo_gh_sees_most_recently_pushed_first` and `pages_follow_on_from_the_token`. `clients/live/tests/github_repos.rs::every_repo_gh_sees_comes_back_in_order_across_pages` runs against a real humatad2. Still unproven: `gh` is a fake script in these tests, so the real `gh` and GitHub are untested.

7. **I do:** Look for repos I used before, or pick from GitHub users and orgs saved in Settings.
   **I see:** before I type, a **Library** group of the repos I used before, most recent first, then **GitHub** narrowed to the scope chips from Settings › Repositories (RL-3, RL-8).
   **State and events:** None.
   **Status:** **Works** (REPOS): `desktop/tests/live_flows/repo_library.rs::a_repo_added_once_is_listed_first_in_the_next_new_channel_even_with_github_out_of_reach` shows the library before anything is typed. The Settings owners are **Fakes only**: `features/app/tests/repo_picker_owners.rs::org_suggestions_list_the_library_owners_then_the_github_orgs`. The spec item is CH-18.

8. **I do:** Look at the machines list. Tick box-1 as well.
   **I see:** This Mac and box-1 are listed with online or offline. This Mac is ticked. Box-1 ticks when online. An offline machine shows "offline" and does not tick.
   **State and events:** `machines` holds the ticked ids. An offline machine is refused by the reducer (CH-4).
   **Status:** Works (real daemon). `desktop/tests/live_machines.rs::an_offline_machine_cannot_be_ticked_in_the_new_channel_sheet` ticks and unticks an added box, takes its link down, and the box no longer ticks. Fake-client versions: `features/app/tests/new_channel.rs::an_offline_machine_cannot_be_ticked`; `desktop/tests/native/new_channel.rs::an_offline_machine_does_not_tick`.

9. **I do:** Press Create channel and clone 3 repos.
   **I see:** The sheet changes to "Creating # eu-checkout".
   **State and events:** The daemon makes the channel folder and the main clones of the three repos on each ticked machine.
   **Status:** Works (real daemon). `daemon/humatad/tests/channels.rs::a_new_channel_gets_its_folders_and_a_main_clone`. The button, on a real daemon in the app: `desktop/tests/live_flows/new_channel.rs::a_channel_made_from_the_new_channel_sheet_clones_its_repos_and_opens_empty`.

10. **I do:** Watch the sheet.
    **I see:** A group per machine, with "Folders made" and one bar per clone. The footer says "The channel opens when every clone is done." When every clone is done, the sheet closes and the channel opens.
    **State and events:** `Event::ChannelSetup` updates `workspace.setups` and the open sheet. When every step is `Done`, the app opens `Route::Channel`.
    **Status:** The sheet closing and the empty channel opening Work (real daemon): `desktop/tests/live_flows/new_channel.rs::a_channel_made_from_the_new_channel_sheet_clones_its_repos_and_opens_empty`. The per-clone progress sheet Works (real daemon): `desktop/tests/live_flows/new_channel_sheet.rs::the_progress_sheet_shows_while_the_clones_run_then_the_channel_opens` holds the daemon's clones until the test releases them. Fake-client version: `features/app/tests/new_channel_setup.rs::progress_events_land_in_the_open_sheet_and_in_the_workspace` and `a_clone_that_works_opens_the_channel_without_a_failed_row`.

11. **I do:** Open the channel's New session. Pick the three repos, leave the machine on This Mac, and type the report text as the first message.
    **I see:** A session tab opens and the agent starts.
    **State and events:** A session row is recorded with the channel, the agent, the machine and the repos. The first message is sent to the agent's terminal.
    **Status:** Works (real daemon). `desktop/tests/live_flows.rs::a_session_started_from_new_session_runs_and_answers_its_first_message`. The agent is a fake.

12. **I do:** Look at the branch the session uses.
    **I see:** The session has one branch name. Each repo's clone is checked out on it.
    **State and events:** `daemon/sessions/src/start.rs:329` and `:563` set the same `branch.name` on every repo of the session. The brief said each repo gets its own branch. The code does not do that. The name is per session.
    **Status:** Works (real daemon) for one repo: `daemon/humatad/tests/branch_names.rs::an_edited_branch_is_used_as_typed_and_a_taken_one_is_refused` checks that the clone's HEAD is the branch. The same name across three repos is in the code only, with no test.

13. **I do:** Watch the agent's terminal.
    **I see:** The agent's output streams in the session's terminal tab and the turn ends.
    **State and events:** The terminal tab shows the agent's output. The session's status moves with the turn.
    **Status:** Works (real daemon). `desktop/tests/live_flows.rs::a_session_started_from_new_session_runs_and_answers_its_first_message`, with a fake agent.

14. **I do:** Open Fork on the session to try a second approach.
    **I see:** The Fork session dialog. Its kind choices are named "Code + context", "Context only" and "Code only" (the brief's wording differs; the code's names win). It also has "Fork from" (Now, or before an earlier turn), Agent and Machine rows.
    **State and events:** `ForkDraft` holds the kind, the turn and the agent. Nothing is forked yet.
    **Status:** Works (real daemon) for opening the dialog: `desktop/tests/live_flows/lifecycle.rs::forking_from_the_fork_dialog_starts_a_child_session_on_the_daemon`. The kind choices and Fork from Work (real daemon): `desktop/tests/live_flows/fork_kinds.rs::a_fork_from_before_a_turn_carries_the_conversation_up_to_it` and the Code only and Context only tests in that file. The failed fork keeps its title and kind, and a retry makes the child: `fork_kinds.rs::a_refused_fork_keeps_the_dialog_and_a_retry_makes_the_child`. Fake-client versions: `features/session/tests/fork.rs::pending_forks_ignore_repeated_create_and_edits` and `a_failed_fork_keeps_its_title_and_kind_for_retry`.

15. **I do:** Choose Code + context and press Fork.
    **I see:** A new session named after the fork appears under the parent.
    **State and events:** The daemon copies the working files, uncommitted changes included, and the conversation into a new folder with its own branch.
    **Status:** Works (real daemon). `daemon/humatad/tests/fork.rs::code_and_context_copies_the_work_and_the_conversation`. The app's Fork button on the default Code + context: `desktop/tests/live_flows/lifecycle.rs::forking_from_the_fork_dialog_starts_a_child_session_on_the_daemon`.

16. **I do:** Fork again with Code only.
    **I see:** A fork with copies of the files and a fresh conversation.
    **State and events:** The conversation is not copied.
    **Status:** Works (real daemon). `daemon/humatad/tests/fork.rs::code_only_copies_the_work_into_a_fresh_conversation`; from the dialog: `desktop/tests/live_flows/fork_kinds.rs::a_code_only_fork_copies_the_files_into_a_fresh_conversation`.

17. **I do:** Fork again with Context only.
    **I see:** A fork that knows the conversation and works in the parent's files, with no copies.
    **State and events:** The fork has a handover in the parent's folder. Its edits land in the parent.
    **Status:** Works (real daemon). `daemon/humatad/tests/context_fork.rs::a_context_only_fork_carries_the_conversation_and_works_in_the_parents_code`; from the dialog: `desktop/tests/live_flows/fork_kinds.rs::a_context_only_fork_carries_the_conversation_and_shares_the_parents_files`.

18. **I do:** Open the Diff panel on the session. Choose "Since I last reviewed".
    **I see:** The changes are grouped by repo, with "3 files · 3 repos". Full screen and back work.
    **State and events:** The diff base is set on the review. Each repo group is drawn from that repo's own files.
    **Status:** Works (real daemon). `desktop/tests/live_flows/review.rs::a_three_repo_diff_is_reviewed_commented_on_and_marked_reviewed`.

19. **I do:** Read a file from each repo in the diff.
    **I see:** Each file is compared with its own repo's base, not one shared base.
    **State and events:** The spec names the base per repo. The daemon uses the spec's base for each repo, or the repo's default branch when the spec leaves it out.
    **Status:** Works (real daemon). `daemon/humatad/tests/review_per_repo.rs::each_file_shows_against_its_own_repos_base`, and `a_repo_the_spec_leaves_out_shows_against_its_default_branch`.

20. **I do:** Press Push. Tick `acme/api` and leave "Open a pull request" on. Confirm the plan.
    **I see:** The plan lists each repo with its branch and totals. After the push, the result names the pull request for each pushed repo.
    **State and events:** A plan token is issued. The push commits the uncommitted files in that repo's clone, pushes the branch, and runs `gh pr create`. Only the ticked repo is pushed.
    **Status:** Works (real daemon). `daemon/humatad/tests/push.rs::pushing_one_repo_commits_pushes_and_opens_its_pull_request` and `push_many.rs::a_push_of_two_repos_reports_both_pull_requests_and_the_open_ones_note`. The native group UI is `desktop/tests/native/push.rs::each_repo_group_pushes_its_own_branch_and_names_its_pull_request` (fakes). Still unproven: a real GitHub remote and a real `gh`. The tests use bare repos and a fake `gh`.

21. **I do:** Nothing. The push is my own action, so the app does not ask me.
    **I see:** The push runs without an Inbox request.
    **State and events:** `check_user` in `daemon/access/src/policy.rs` lets the app's own calls through. An agent's push is different: its rule asks, and the request waits in the Inbox.
    **Status:** Works (real daemon). `daemon/humatad/tests/access.rs::explain_follows_the_rule_as_it_stands` (the default for `GitPush` is Ask for agents), and `daemon/humatad/tests/away_boundary.rs::row_6_while_the_host_is_linked_a_push_is_asked_of_the_user`.

## Branches

### gh is not signed in

- **When:** On step 2, or on Retry, when `gh` is signed out.
- **I do:** Open New channel.
- **I see:** The repo list is replaced by a red error with the reason, and a Retry button. The message says to run `gh auth login` in a terminal. The channel can still be made without repos.
- **Expected:** The daemon's error names the fix. The sheet still lets me create a channel with no repos (CH-10). Retry lists the repos again.
- **Status:** Works (real daemon). `daemon/humatad/tests/gh_repos.rs::a_signed_out_gh_is_a_failed_precondition_and_is_not_remembered` (a failure is not cached). The sheet's error and Retry button Work (real daemon): `desktop/tests/live_flows/new_channel.rs::a_signed_out_gh_shows_the_sheets_signed_out_state_not_an_empty_list` (no empty list is shown). Still Fakes only: Retry listing again, and making the channel with no repos after the failure: `features/app/tests/new_channel.rs::when_gh_cannot_list_repos_the_sheet_says_why_and_still_makes_a_channel` and `retry_lists_the_repos_again_after_a_failure`.

### gh is rate limited

- **When:** On step 2, when GitHub has no requests left for the user.
- **I do:** Open New channel.
- **I see:** The error shown is the sign-in message: "run `gh auth login` in a terminal if gh is signed out, then try again".
- **Expected:** The message says to wait for the limit to reset. It must not send the user to sign in again (AGENTS.md, Errors).
- **Status:** Works (real daemon): `clients/live/tests/github_repos.rs::a_rate_limited_gh_is_refused_as_a_wait_and_not_a_sign_in` lists repos through a real `humatad2` whose fake `gh` is rate-limited, and the refusal does not say to sign in. The mapping is `daemon/humatad/src/channel_service/gh_repos.rs::a_rate_limited_gh_is_a_wait_and_not_a_sign_in`. (Was a likely bug: every `gh` failure got the sign-in fix; fixed by BUGS.)

### A repo does not exist

- **When:** On create, when a ticked repo is missing on GitHub or has no access.
- **I do:** Press Create.
- **I see:** The sheet shows the group for that machine with the clone row marked "Clone failed", the reason, and a Retry button. Other clones keep going.
- **Expected:** The channel is kept, the failed repo is marked failed, no half clone is left in `main/`, and Retry runs the failed clone again.
- **Status:** Works (real daemon). `daemon/humatad/tests/channels.rs::a_repository_that_does_not_exist_fails_the_setup_and_leaves_no_half_clone`. The Retry row is fakes: `features/app/tests/new_channel_setup.rs::a_failed_clone_shows_failed_and_retry_runs_it_again_then_the_channel_opens`.

### A repo is added to the channel later, and the agent proposes it

- **When:** Mid-work. The agent of a channel session asks for another repo.
- **I do:** Approve the request in the Inbox.
- **I see:** An Inbox card with the repo and the reason. After Allow, the repo's main clone is made and it shows in the channel.
- **Expected:** The repo is added once. An agent cannot propose for another channel.
- **Status:** Works (real daemon). `daemon/humatad/tests/channel_repo_propose.rs::approving_the_proposal_adds_the_repo`, `an_agents_add_repo_becomes_an_inbox_request_under_the_repo_add_rule`, and `a_channel_agent_asking_for_another_channel_is_denied_and_adds_nothing_to_the_inbox`.

### Remove a repo from the channel

- **When:** After the channel exists, I want to drop a repo from it.
- **I do:** Look for a remove control on the channel's Repos and machines tab. There is none in the UI, and the channel client has no remove call. Only the Desk can remove a repo from a session.
- **I see:** Nothing to press. A daemon call for the channel-level remove would answer "not built".
- **Expected (CH-22):** Removing a repo no session uses deletes its main clones at once. When a session uses it, the dialog lists the sessions and offers to drop the repo from each one, or archive it.
- **Status:** Not built. `daemon/humatad/src/channel_service.rs:1121-1133` returns `not_built` for `PlanRemoveRepo` and `RemoveRepo`. No milestone found in `workflow/milestones/`; the spec item is CH-22.

### The push is denied

- **When:** An agent asks to push, and I answer Deny in the Inbox.
- **I do:** Deny the request.
- **I see:** The card shows as answered Denied. The push does not run.
- **Expected:** No commit, no push, no `gh pr create`. The agent's call is not left waiting.
- **Status:** Code, no test. `daemon/humatad/src/session_service/push.rs:819-835` (`ask_for_pushes`) and `daemon/humatad/src/session_service.rs:2412-2440` (`answer_gated`). The generic deny test, `daemon/humatad/tests/inbox.rs::a_denied_deferred_call_never_runs`, covers a different call, not a push.

### The session's branch name is taken

- **When:** On New session, I type a branch name that another session in the channel (or a remote branch) already uses.
- **I do:** Type `team/refunds` and start.
- **I see:** The start is refused with a "name taken" message and a suggested free name. No session is made.
- **Expected:** The first session keeps its branch. The second start makes nothing.
- **Status:** Works (real daemon). `daemon/humatad/tests/branch_names.rs::an_edited_branch_is_used_as_typed_and_a_taken_one_is_refused`.

### A repo I used before, in another channel or on another machine

- **When:** on step 2, after `acme/api` was used in an earlier channel or on the Desk from any machine.
- **I do:** open New channel from the browser on devbox-1 and don't type.
- **I see:** `acme/api` under **Library**, with when and where it was last used, before anything is typed.
- **Expected:** the library comes from the host (RL-1, RL-3). Nothing was added on this machine.
- **Status:** **Works** on the Mac: `desktop/tests/live_flows/repo_library.rs::a_repo_added_once_is_listed_first_in_the_next_new_channel_even_with_github_out_of_reach`. From devbox-1 the library comes from the host for its agents: `daemon/humatad/tests/away_forward.rs::an_agents_repo_search_on_a_machine_goes_up_and_the_hosts_answer_is_the_agents` and `::away_an_agents_repo_search_is_host_unreachable_and_answers_nothing` (D568). The browser is cut.

### The repo is outside my default scope

- **When:** `repos.toml` has `owners = ["acme"]`, and the repo is `acme-labs/rounding`.
- **I do:** type `round`, see no GitHub hit, click **+ Org**, type `acme-labs`, tick the repo, then create.
- **I see:** the chips `acme ✕ you ✕` gain `acme-labs ✕`, the GitHub heading names all three, and **Save as default** appears.
- **Expected:** the widened scope covers this search only. The next picker shows `acme ✕ you ✕` again, and `repos.toml` is unchanged unless I clicked Save as default (RL-8). The repo is in the library from then on (RL-2).
- **Status:** **Fakes only**: `features/app/tests/repo_search.rs::widening_with_an_owner_keeps_the_scope_and_adds_the_owner_to_it`, `::removing_a_chip_narrows_this_search_and_offers_save_as_default` and `::save_as_default_writes_the_scope_and_is_no_longer_offered`. The daemon part **Works**: `daemon/humatad/tests/repos.rs::a_scope_for_one_search_is_used_once_and_never_saved`.

### GitHub search is rate limited

- **When:** GitHub has no searches left for the host's gh login.
- **I do:** type in the repo field.
- **I see:** library matches as usual. In place of the GitHub group, one line says GitHub is limiting searches, until when, and that signing in again won't help.
- **Expected:** the search runs again after the reset. The sheet still creates channels from library repos (RL-10, D478).
- **Status:** **Works** in the daemon: `daemon/humatad/tests/repos.rs::a_signed_out_gh_and_a_rate_limit_are_problems_and_the_library_still_answers`. The sheet's line is **Fakes only**: `features/app/tests/repo_search.rs::a_failing_github_search_keeps_the_library_group_and_says_so`.

## Expected outcomes

- [ ] ⇧⌘N opens the New channel sheet with This Mac ticked and the repos gh sees listed.
- [ ] The folder is `~/Humata/eu-checkout/` for the name "eu-checkout".
- [ ] Typing narrows the list, whole matches first, and ticked repos stay at the top.
- [ ] Create says "clone 3 repos" and makes the channel with three main clones on each ticked machine.
- [ ] The daemon lists repos across pages, up to 1000, most recently pushed first.
- [ ] An offline machine cannot be ticked.
- [ ] A session starts with the three repos and the report text as its first message.
- [ ] Each fork kind copies what its name says: code and conversation, code only, or conversation only.
- [ ] The diff groups changes by repo and compares each repo with its own base.
- [ ] Push ships only the ticked repo, and an agent's push waits in the Inbox under the rule.
- [ ] A signed-out or rate-limited `gh` says what to do, and a rate limit says to wait.
- [ ] A repo that does not exist fails its clone, leaves no half clone, and offers Retry.
- [ ] A taken branch name is refused with a free suggestion.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Press ⇧⌘N | Works (real daemon) | `desktop/tests/live_flows/new_channel.rs::a_channel_made_from_the_new_channel_sheet_clones_its_repos_and_opens_empty` |
| 2. Sheet opens, This Mac ticked | Works (real daemon) | `desktop/tests/live_flows/new_channel.rs::a_channel_made_from_the_new_channel_sheet_clones_its_repos_and_opens_empty`; fakes: `features/app/tests/new_channel.rs::opening_the_sheet_lists_the_repos_gh_sees_and_ticks_this_mac` |
| 3. Folder follows the name | Works (real daemon) | `desktop/tests/live_flows/new_channel.rs::a_channel_made_from_the_new_channel_sheet_clones_its_repos_and_opens_empty`; fakes: `features/app/tests/new_channel.rs::the_folder_follows_the_name` |
| 4. Search narrows, ticked first | Works (real daemon) | `desktop/tests/live_flows/new_channel.rs::a_channel_made_from_the_new_channel_sheet_clones_its_repos_and_opens_empty`; order: `desktop/tests/live_flows/new_channel_sheet.rs::whole_word_matches_come_before_scattered_ones_in_the_repo_search`; fakes: `features/app/tests/new_channel.rs::typing_narrows_the_list_by_fuzzy_match` |
| 5. Tick repos, button says how many | Works (real daemon) | `desktop/tests/live_flows/new_channel.rs::a_channel_made_from_the_new_channel_sheet_clones_its_repos_and_opens_empty`; fakes: `features/app/tests/new_channel.rs::the_create_button_says_how_many_repos_it_will_clone` |
| 6. Daemon lists repos in pages | Works (real daemon) | `daemon/humatad/tests/gh_repos.rs::pages_follow_on_from_the_token` (fake gh) |
| 7. Used-before and Settings sources | Works (real daemon) | `desktop/tests/live_flows/repo_library.rs::a_repo_added_once_is_listed_first_in_the_next_new_channel_even_with_github_out_of_reach`; Settings owners: `features/app/tests/repo_picker_owners.rs::org_suggestions_list_the_library_owners_then_the_github_orgs` (fakes) |
| 8. Pick machines, offline cannot tick | Works (real daemon) | `desktop/tests/live_machines.rs::an_offline_machine_cannot_be_ticked_in_the_new_channel_sheet` |
| 9. Create makes folders and clones | Works (real daemon) | `daemon/humatad/tests/channels.rs::a_new_channel_gets_its_folders_and_a_main_clone` |
| 10. Progress sheet, then channel opens | Works (real daemon) | `desktop/tests/live_flows/new_channel_sheet.rs::the_progress_sheet_shows_while_the_clones_run_then_the_channel_opens`; `desktop/tests/live_flows/new_channel.rs::a_channel_made_from_the_new_channel_sheet_clones_its_repos_and_opens_empty`; fakes: `features/app/tests/new_channel_setup.rs::progress_events_land_in_the_open_sheet_and_in_the_workspace` |
| 11. Start session with the bug text | Works (real daemon) | `desktop/tests/live_flows.rs::a_session_started_from_new_session_runs_and_answers_its_first_message` |
| 12. One branch name across repos | Works (real daemon) | `daemon/humatad/tests/branch_names.rs::an_edited_branch_is_used_as_typed_and_a_taken_one_is_refused` (one repo only) |
| 13. Watch the agent | Works (real daemon) | `desktop/tests/live_flows.rs::a_session_started_from_new_session_runs_and_answers_its_first_message` |
| 14. Fork dialog and its kinds | Works (real daemon) | `desktop/tests/live_flows/fork_kinds.rs::a_fork_from_before_a_turn_carries_the_conversation_up_to_it`, `::a_refused_fork_keeps_the_dialog_and_a_retry_makes_the_child`; `lifecycle.rs::forking_from_the_fork_dialog_starts_a_child_session_on_the_daemon` |
| 15. Fork Code + context | Works (real daemon) | `daemon/humatad/tests/fork.rs::code_and_context_copies_the_work_and_the_conversation` |
| 16. Fork Code only | Works (real daemon) | `daemon/humatad/tests/fork.rs::code_only_copies_the_work_into_a_fresh_conversation` |
| 17. Fork Context only | Works (real daemon) | `daemon/humatad/tests/context_fork.rs::a_context_only_fork_carries_the_conversation_and_works_in_the_parents_code` |
| 18. Diff across three repos | Works (real daemon) | `desktop/tests/live_flows/review.rs::a_three_repo_diff_is_reviewed_commented_on_and_marked_reviewed` |
| 19. Each repo against its own base | Works (real daemon) | `daemon/humatad/tests/review_per_repo.rs::each_file_shows_against_its_own_repos_base` |
| 20. Push one repo and open its PR | Works (real daemon) | `daemon/humatad/tests/push.rs::pushing_one_repo_commits_pushes_and_opens_its_pull_request` (fake gh) |
| 21. My push is not asked; an agent's is | Works (real daemon) | `daemon/humatad/tests/access.rs::explain_follows_the_rule_as_it_stands` |
| Branch: gh signed out | Works (real daemon) | `daemon/humatad/tests/gh_repos.rs::a_signed_out_gh_is_a_failed_precondition_and_is_not_remembered` |
| Branch: gh rate limited | Works (real daemon) | `clients/live/tests/github_repos.rs::a_rate_limited_gh_is_refused_as_a_wait_and_not_a_sign_in`; `daemon/humatad/src/channel_service/gh_repos.rs::a_rate_limited_gh_is_a_wait_and_not_a_sign_in` |
| Branch: repo does not exist | Works (real daemon) | `daemon/humatad/tests/channels.rs::a_repository_that_does_not_exist_fails_the_setup_and_leaves_no_half_clone` |
| Branch: repo added later, approved | Works (real daemon) | `daemon/humatad/tests/channel_repo_propose.rs::approving_the_proposal_adds_the_repo` |
| Branch: remove a repo from the channel | Not built | `daemon/humatad/src/channel_service.rs:1121-1133`; CH-22 |
| Branch: push denied | Code, no test | `daemon/humatad/src/session_service/push.rs:819-835` |
| Branch: session branch name taken | Works (real daemon) | `daemon/humatad/tests/branch_names.rs::an_edited_branch_is_used_as_typed_and_a_taken_one_is_refused` |

## Gaps and likely bugs

- **Not built:** `PlanRemoveRepo` and `RemoveRepo` for a channel (`daemon/humatad/src/channel_service.rs:1121-1133`; CH-22). No milestone found.
- **Built (daemon and CLI):** `GetGrid` (`daemon/humatad/src/channel_service/grid.rs`; CH-9, D764) and `humata channel grid <c>`. **Not built:** the app's Repos and machines tab that shows it.
- **Fixed (was a likely bug):** a rate-limited `gh` is told as a wait, not a sign-in (`clients/live/tests/github_repos.rs::a_rate_limited_gh_is_refused_as_a_wait_and_not_a_sign_in`).
- **Note, not a bug:** the brief said each repo gets its own branch. The code gives one branch name per session, used in every repo (`daemon/sessions/src/start.rs:329`, `:563`). Only one repo is tested.
- **Note, not a bug:** the brief said the picker pages. The daemon pages, but the sheet does not. It shows 50 and asks the user to type to narrow.

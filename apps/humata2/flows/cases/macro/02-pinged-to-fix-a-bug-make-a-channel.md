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
   **Status:** Code, no test. `ui/src/keys.rs:96` binds `shift-cmd-n` to `NewChannel`, and `ui/src/shell.rs:499` sends `StartNewChannel`. `features/app/tests/every_action.rs` reads `keys.rs` as text, but only to check that the palette can reach each action. No test presses the key.

2. **I do:** Nothing more. Look at the sheet.
   **I see:** Three rows labelled Name, Repos and Machines. This Mac is ticked. Under Repos it says "Listing repos from gh…", then the list appears, with the hint "From gh on This Mac. Zero or more."
   **State and events:** The sheet holds `machines = [This Mac]` and `listed = Some(repos)` once the daemon answers.
   **Status:** Fakes only. `features/app/tests/new_channel.rs::opening_the_sheet_lists_the_repos_gh_sees_and_ticks_this_mac`. Not proven live: the real sheet on a real `gh`.

3. **I do:** Type `eu-checkout` in Channel name.
   **I see:** The folder line reads `Folder: ~/Humata/eu-checkout/`.
   **State and events:** The folder is derived from the name (lowercase, runs of other characters become one `-`).
   **Status:** Fakes only. `features/app/tests/new_channel.rs::the_folder_follows_the_name`.

4. **I do:** Type `web` in Search repos.
   **I see:** The list narrows. Whole matches (`acme/web`) come before scattered ones. Repos I have already ticked stay at the top whether or not they match.
   **State and events:** `query` changes. `shown_repos()` keeps ticked repos first, then the matches.
   **Status:** Fakes only. `features/app/tests/new_channel.rs::typing_narrows_the_list_by_fuzzy_match`, `whole_word_matches_come_before_scattered_ones`, `ticked_repos_show_first_even_when_they_do_not_match`. The ranking rule itself is also proven on the daemon side (see step 6).

5. **I do:** Tick `acme/web`, `acme/api` and `acme/payments`.
   **I see:** Three ticked rows. The primary button reads "Create channel and clone 3 repos".
   **State and events:** `repos` holds the three names in tick order.
   **Status:** Fakes only. `features/app/tests/new_channel.rs::the_create_button_says_how_many_repos_it_will_clone`.

6. **I do:** Nothing. This is what the picker draws from.
   **I see:** The sheet shows at most 50 matches and, when there are more, says "and N more; type to narrow". It does not page. Paging happens between the client and the daemon.
   **State and events:** The daemon lists the repos for the signed-in user with `gh api --paginate user/repos?...affiliation=owner,collaborator,organization_member`, most recently pushed first, at most 1000. The live client reads 100 at a time and follows the page tokens.
   **Status:** Works (real daemon). `daemon/humatad/tests/gh_repos.rs::an_empty_query_lists_every_repo_gh_sees_most_recently_pushed_first` and `pages_follow_on_from_the_token`. `clients/live/tests/github_repos.rs::every_repo_gh_sees_comes_back_in_order_across_pages` runs against a real humatad2. Still unproven: `gh` is a fake script in these tests, so the real `gh` and GitHub are untested.

7. **I do:** Look for repos I used before, or pick from GitHub users and orgs saved in Settings.
   **I see:** Nothing of the kind. The only source is what `gh` lists on This Mac. Org repos come in through the affiliation list in step 6, but there is no "used before" list and no Settings source.
   **State and events:** None.
   **Status:** Not built. NEWCH (`workflow/milestones/NEWCH.md`, line 124: "No saved-repositories source existed, so none was reused"). The spec item is CH-18.

8. **I do:** Look at the machines list. Tick box-1 as well.
   **I see:** This Mac and box-1 are listed with online or offline. This Mac is ticked. Box-1 ticks when online. An offline machine shows "offline" and does not tick.
   **State and events:** `machines` holds the ticked ids. An offline machine is refused by the reducer (CH-4).
   **Status:** Fakes only. `features/app/tests/new_channel.rs::an_offline_machine_cannot_be_ticked`; `desktop/tests/native/new_channel.rs::an_offline_machine_does_not_tick`.

9. **I do:** Press Create channel and clone 3 repos.
   **I see:** The sheet changes to "Creating # eu-checkout".
   **State and events:** The daemon makes the channel folder and the main clones of the three repos on each ticked machine.
   **Status:** Works (real daemon). `daemon/humatad/tests/channels.rs::a_new_channel_gets_its_folders_and_a_main_clone`.

10. **I do:** Watch the sheet.
    **I see:** A group per machine, with "Folders made" and one bar per clone. The footer says "The channel opens when every clone is done." When every clone is done, the sheet closes and the channel opens.
    **State and events:** `Event::ChannelSetup` updates `workspace.setups` and the open sheet. When every step is `Done`, the app opens `Route::Channel`.
    **Status:** Fakes only. `features/app/tests/new_channel_setup.rs::progress_events_land_in_the_open_sheet_and_in_the_workspace` and `a_clone_that_works_opens_the_channel_without_a_failed_row`.

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
    **Status:** Fakes only. `features/session/tests/fork.rs::pending_forks_ignore_repeated_create_and_edits` and `a_failed_fork_keeps_its_title_and_kind_for_retry`.

15. **I do:** Choose Code + context and press Fork.
    **I see:** A new session named after the fork appears under the parent.
    **State and events:** The daemon copies the working files, uncommitted changes included, and the conversation into a new folder with its own branch.
    **Status:** Works (real daemon). `daemon/humatad/tests/fork.rs::code_and_context_copies_the_work_and_the_conversation`.

16. **I do:** Fork again with Code only.
    **I see:** A fork with copies of the files and a fresh conversation.
    **State and events:** The conversation is not copied.
    **Status:** Works (real daemon). `daemon/humatad/tests/fork.rs::code_only_copies_the_work_into_a_fresh_conversation`.

17. **I do:** Fork again with Context only.
    **I see:** A fork that knows the conversation and works in the parent's files, with no copies.
    **State and events:** The fork has a handover in the parent's folder. Its edits land in the parent.
    **Status:** Works (real daemon). `daemon/humatad/tests/context_fork.rs::a_context_only_fork_carries_the_conversation_and_works_in_the_parents_code`.

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
- **Status:** Works (real daemon). `daemon/humatad/tests/gh_repos.rs::a_signed_out_gh_is_a_failed_precondition_and_is_not_remembered` (a failure is not cached). The sheet's error and Retry are fakes: `features/app/tests/new_channel.rs::when_gh_cannot_list_repos_the_sheet_says_why_and_still_makes_a_channel` and `retry_lists_the_repos_again_after_a_failure`.

### gh is rate limited

- **When:** On step 2, when GitHub has no requests left for the user.
- **I do:** Open New channel.
- **I see:** The error shown is the sign-in message: "run `gh auth login` in a terminal if gh is signed out, then try again".
- **Expected:** The message says to wait for the limit to reset. It must not send the user to sign in again (AGENTS.md, Errors).
- **Status:** Likely bug. `daemon/humatad/src/channel_service/gh_repos.rs:71-73`. Every `gh` failure gets the same fix. The rate limit is not detected here, although `daemon/humatad/src/review/pull_requests/github.rs:74-75` does detect it for pull requests. To confirm: make the fake `gh` print "API rate limit exceeded" to stderr with exit 1 and read the error.

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
| 1. Press ⇧⌘N | Code, no test | `ui/src/keys.rs:96`, `ui/src/shell.rs:499` |
| 2. Sheet opens, This Mac ticked | Fakes only | `features/app/tests/new_channel.rs::opening_the_sheet_lists_the_repos_gh_sees_and_ticks_this_mac` |
| 3. Folder follows the name | Fakes only | `features/app/tests/new_channel.rs::the_folder_follows_the_name` |
| 4. Search narrows, ticked first | Fakes only | `features/app/tests/new_channel.rs::typing_narrows_the_list_by_fuzzy_match` |
| 5. Tick repos, button says how many | Fakes only | `features/app/tests/new_channel.rs::the_create_button_says_how_many_repos_it_will_clone` |
| 6. Daemon lists repos in pages | Works (real daemon) | `daemon/humatad/tests/gh_repos.rs::pages_follow_on_from_the_token` (fake gh) |
| 7. Used-before and Settings sources | Not built | NEWCH; CH-18 |
| 8. Pick machines, offline cannot tick | Fakes only | `features/app/tests/new_channel.rs::an_offline_machine_cannot_be_ticked` |
| 9. Create makes folders and clones | Works (real daemon) | `daemon/humatad/tests/channels.rs::a_new_channel_gets_its_folders_and_a_main_clone` |
| 10. Progress sheet, then channel opens | Fakes only | `features/app/tests/new_channel_setup.rs::progress_events_land_in_the_open_sheet_and_in_the_workspace` |
| 11. Start session with the bug text | Works (real daemon) | `desktop/tests/live_flows.rs::a_session_started_from_new_session_runs_and_answers_its_first_message` |
| 12. One branch name across repos | Works (real daemon) | `daemon/humatad/tests/branch_names.rs::an_edited_branch_is_used_as_typed_and_a_taken_one_is_refused` (one repo only) |
| 13. Watch the agent | Works (real daemon) | `desktop/tests/live_flows.rs::a_session_started_from_new_session_runs_and_answers_its_first_message` |
| 14. Fork dialog and its kinds | Fakes only | `features/session/tests/fork.rs::pending_forks_ignore_repeated_create_and_edits` |
| 15. Fork Code + context | Works (real daemon) | `daemon/humatad/tests/fork.rs::code_and_context_copies_the_work_and_the_conversation` |
| 16. Fork Code only | Works (real daemon) | `daemon/humatad/tests/fork.rs::code_only_copies_the_work_into_a_fresh_conversation` |
| 17. Fork Context only | Works (real daemon) | `daemon/humatad/tests/context_fork.rs::a_context_only_fork_carries_the_conversation_and_works_in_the_parents_code` |
| 18. Diff across three repos | Works (real daemon) | `desktop/tests/live_flows/review.rs::a_three_repo_diff_is_reviewed_commented_on_and_marked_reviewed` |
| 19. Each repo against its own base | Works (real daemon) | `daemon/humatad/tests/review_per_repo.rs::each_file_shows_against_its_own_repos_base` |
| 20. Push one repo and open its PR | Works (real daemon) | `daemon/humatad/tests/push.rs::pushing_one_repo_commits_pushes_and_opens_its_pull_request` (fake gh) |
| 21. My push is not asked; an agent's is | Works (real daemon) | `daemon/humatad/tests/access.rs::explain_follows_the_rule_as_it_stands` |
| Branch: gh signed out | Works (real daemon) | `daemon/humatad/tests/gh_repos.rs::a_signed_out_gh_is_a_failed_precondition_and_is_not_remembered` |
| Branch: gh rate limited | Likely bug | `daemon/humatad/src/channel_service/gh_repos.rs:71-73` |
| Branch: repo does not exist | Works (real daemon) | `daemon/humatad/tests/channels.rs::a_repository_that_does_not_exist_fails_the_setup_and_leaves_no_half_clone` |
| Branch: repo added later, approved | Works (real daemon) | `daemon/humatad/tests/channel_repo_propose.rs::approving_the_proposal_adds_the_repo` |
| Branch: remove a repo from the channel | Not built | `daemon/humatad/src/channel_service.rs:1121-1133`; CH-22 |
| Branch: push denied | Code, no test | `daemon/humatad/src/session_service/push.rs:819-835` |
| Branch: session branch name taken | Works (real daemon) | `daemon/humatad/tests/branch_names.rs::an_edited_branch_is_used_as_typed_and_a_taken_one_is_refused` |

## Gaps and likely bugs

- **Not built:** a "used before" repo source and a Settings source for GitHub users and orgs (NEWCH, CH-18). The gh listing covers org repos, but nothing remembers repos across sessions.
- **Not built:** `PlanRemoveRepo` and `RemoveRepo` for a channel (`daemon/humatad/src/channel_service.rs:1121-1133`; CH-22). No milestone found.
- **Not built:** `GetGrid` (`daemon/humatad/src/channel_service.rs:1205-1209`; CH-9), which the Repos and machines grid needs.
- **Likely bug:** a rate-limited `gh` gets the sign-in fix instead of a "wait" message (`daemon/humatad/src/channel_service/gh_repos.rs:71-73`). Confirm with a fake `gh` that prints "API rate limit exceeded".
- **Note, not a bug:** the brief said each repo gets its own branch. The code gives one branch name per session, used in every repo (`daemon/sessions/src/start.rs:329`, `:563`). Only one repo is tested.
- **Note, not a bug:** the brief said the picker pages. The daemon pages, but the sheet does not. It shows 50 and asks the user to type to narrow.

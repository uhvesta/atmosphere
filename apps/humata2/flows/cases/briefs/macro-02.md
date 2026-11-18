# Brief macro-02: Pinged to fix a bug, I find the repos and make a channel
Output: `apps/humata2/flows/cases/macro/02-pinged-to-fix-a-bug-make-a-channel.md`

Scenario: "checkout fails for EU users since this morning". I press ⇧⌘N for New channel, name it "eu-checkout", pick repos with the fuzzy repo picker (acme/web, acme/api, acme/payments), pick machines, start a session with the bug text, watch the agent, fork to try a second approach, review the diff across three repos, and push.

Steps: New channel setup flow; repo picker sources (used before, GitHub users/orgs through the gh token, pages of results); a session clones each repo on its own branch; the agent's terminal; Fork dialog (code and context / code only / context only); the diff review across repos, each against its own base; push asked by the rules.

Branches: gh not logged in or rate limited (message says wait); a repo that doesn't exist; a repo added later to the channel (agent proposes; user approves); PlanRemoveRepo/RemoveRepo for the channel (not built: ChannelService); a push denied; the session's branch name taken.

Pointers: features/app/tests/new_channel*.rs, desktop/tests/native/new_channel.rs, daemon/humatad/tests/channel_add.rs, channels.rs, gh_repos.rs, branch_names.rs, push*.rs, review_per_repo.rs, desktop/tests/live_flows/review.rs (three-repo diff review; real daemon), daemon/humatad/src/channel_service.rs (not_built PlanRemoveRepo/RemoveRepo; GetGrid built in channel_service/grid.rs), workflow/milestones/NEWCH.md, FORK.md.

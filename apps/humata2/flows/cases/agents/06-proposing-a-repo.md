# An agent's repo proposal becomes an Inbox request, and the repo is cloned only after I approve

Layer: agents. Platforms: macOS, Linux, web (the Inbox card).

## Starting state

- This Mac runs humatad2. This Mac is the only machine.
- Channel `eu-checkout` holds `acme/api`. Its main clone is on this Mac.
- A session "Fix checkout" runs in `eu-checkout`, and its agent is mine. The agent is fixing a bug in `acme/api`.
- The repo `acme/shared-lib` exists on GitHub, and this Mac's `gh` can see it. It is not in the channel.
- The rule "Add a repo to a channel" (`channel_repo_add`) is at its default, Ask me.
- The Inbox is empty. The app is open on `eu-checkout`.

## Steps

1. **I do:** the agent runs `humata channel repo add eu-checkout acme/shared-lib`, because it needs the shared library for the fix.
   **I see:** no clone yet. The CLI reports the call as waiting for an answer and gives the request id. With `--json` it returns at once with the pending exit code 12, the same as any ask (see the agents-05 case).
   **State and events:** the call is held. The channel record still lists only `acme/api`.
   **Status:** Code, no test. The CLI path is `cli/src/commands/looking.rs:110-116` (`ChannelRepoCommand::Add` sends `ADD_REPO`). No CLI test covers `channel repo add`. The daemon side is covered by step 2, and it is called over gRPC there.

2. **I do:** nothing. I look at the Inbox.
   **I see:** one request on `eu-checkout`, for the capability "Add a repo to a channel", with `acme/shared-lib` on its card.
   **State and events:** one Pending request with capability ChannelRepoAdd on channel `eu-checkout`. Nothing else is in the Inbox.
   **Status:** Works (real daemon). `daemon/humatad/tests/channel_repo_propose.rs::an_agents_add_repo_becomes_an_inbox_request_under_the_repo_add_rule`. The test checks the capability and that the Inbox holds one request. It does not check the card's buttons.

3. **I do:** I press Allow.
   **I see:** the card leaves the Inbox. The channel lists `acme/api` and `acme/shared-lib`. The clone is at `main/shared-lib` and has a `.git` folder. The agent's wait returns an operation, and the operation finishes as Succeeded.
   **State and events:** the request is Approved, with result type `humata.v1.Operation`. The operation kind is AddRepo.
   **Status:** Works (real daemon). `daemon/humatad/tests/channel_repo_propose.rs::approving_the_proposal_adds_the_repo`. The GitHub side is a fake remote in the test.

4. **I do:** before I answer the card, I add `acme/shared-lib` myself from the app. Then I press Allow on the old card.
   **I see:** the approval succeeds. The clone is not made again. A marker file I had put in the clone is still there, and the clone's HEAD is unchanged.
   **State and events:** the channel record lists the repo once. The operation succeeds.
   **Status:** Works (real daemon). `daemon/humatad/tests/channel_repo_propose.rs::an_approval_after_the_user_added_the_repo_succeeds_without_a_second_clone`.

## Branches

### The agent asks for another channel

- **When:** the agent in `eu-checkout` runs `humata channel repo add billing acme/shared-lib`.
- **I do:** nothing.
- **I see:** the call is refused at once with a permission error. No card appears.
- **Expected outcome:** the Inbox gets no request. `billing` is unchanged, and no clone is made.
- **Status:** Works (real daemon). `daemon/humatad/tests/channel_repo_propose.rs::a_channel_agent_asking_for_another_channel_is_denied_and_adds_nothing_to_the_inbox`.

### A Desk agent proposes for any channel

- **When:** an agent in a Desk session, which has no channel of its own, proposes `acme/shared-lib` for `billing`.
- **I do:** I look at the Inbox.
- **I see:** a card asks about the repo for `billing`. Nothing is added until I approve.
- **Expected outcome:** one Pending request on `billing`. The record is unchanged until Allow.
- **Status:** Works (real daemon). `daemon/humatad/tests/channel_repo_propose.rs::a_desk_agents_proposal_for_any_channel_is_asked`.

### The agent names a local folder

- **When:** the agent runs `humata channel repo add eu-checkout ~/code/shared-lib`, or uses a name with a leading slash or `../`.
- **I do:** nothing.
- **I see:** the call is refused at once with an invalid-argument error. The error says a folder on this machine can't be proposed, and it tells the agent to ask me to add the folder from the app.
- **Expected outcome:** no card, no clone, and no change to the record.
- **Status:** Works (real daemon). `daemon/humatad/tests/channel_repo_propose.rs::an_agent_naming_a_local_path_is_refused_and_asks_nothing`. The test covers a leading slash, a bare `~`, and a name with `../`.

### The rule is set to Never

- **When:** I set "Add a repo to a channel" to Never, and the agent proposes a repo.
- **I do:** set the rule in Agent access.
- **I see:** the agent's call is refused at once with a permission error. No card appears.
- **Expected outcome:** no Inbox request, and no clone folder is made.
- **Status:** Works (real daemon). `daemon/humatad/tests/channel_add.rs::an_agents_add_repo_is_refused_when_the_repo_add_rule_is_never`.

### The earlier clone failed, and the approval runs again

- **When:** `acme/shared-lib` was added before, but its clone failed. Then the agent's proposal is approved.
- **I do:** Allow on the card.
- **I see:** the clone is made again. The channel lists the repo once.
- **Expected outcome:** the operation succeeds, and the repo is not listed twice.
- **Status:** Works (real daemon). `daemon/humatad/tests/channel_add.rs::a_repo_the_channel_has_runs_as_a_retry_and_is_listed_once`.

### I deny the proposal

- **When:** after step 2, I press Deny.
- **I do:** Deny on the card.
- **I see:** the agent's wait ends with a denied error. The channel does not gain the repo, and no clone is made.
- **Expected outcome:** the request is Denied, and the record and folders are unchanged.
- **Status:** Code, no test. The deny path for `add_repo_to_channel` (`daemon/humatad/src/channel_service/add.rs:44-100`) has no test. The generic deny is tested for a different call, `daemon/humatad/tests/inbox.rs::a_denied_deferred_call_never_runs`.

### The agent tries to drop a repo

- **When:** the agent runs `humata channel repo rm eu-checkout acme/api`.
- **I do:** nothing.
- **I see:** an error that says the removal "isn't built into this humatad2 yet", with a fix line that says to update Humata.
- **Expected outcome:** nothing is removed.
- **Status:** Not built. See the gaps below.

### The agent tries to rename the channel or add a machine

- **When:** the agent calls the channel rename or the channel machine add, with its own token.
- **I do:** nothing.
- **I see:** both calls are refused with a permission error. No card appears.
- **Expected outcome:** the channel name and machines are unchanged, and the Inbox is empty.
- **Status:** Works (real daemon). `daemon/humatad/tests/channel_repo_propose.rs::other_channel_changes_by_an_agent_are_still_never`. The test runs at the default rule. The tool text says the user can change the rule, so this is "never by default", not "never".

## Expected outcomes

- [ ] A proposal from the agent's CLI becomes one Inbox request on its own channel, under the repo-add rule.
- [ ] Allow adds the repo to the channel and clones it into `main/<repo>`. The agent's wait returns a succeeded operation.
- [ ] Approving after I added the repo succeeds, and makes no second clone.
- [ ] An agent proposal for another channel is refused at once. The Inbox gets nothing.
- [ ] A Desk agent's proposal for any channel is asked.
- [ ] A local folder is refused at once, and the refusal says what to do instead.
- [ ] A rule set to Never refuses the proposal, with no card and no clone.
- [ ] A repo whose clone failed is cloned again on approval, and is listed once.
- [ ] A denied proposal adds nothing.
- [ ] An agent cannot drop a repo, which is not built.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. The agent runs `channel repo add` | Code, no test | `cli/src/commands/looking.rs:110-116` |
| 2. The request appears in the Inbox | Works (real daemon) | `daemon/humatad/tests/channel_repo_propose.rs::an_agents_add_repo_becomes_an_inbox_request_under_the_repo_add_rule` |
| 3. Allow adds and clones | Works (real daemon) | `daemon/humatad/tests/channel_repo_propose.rs::approving_the_proposal_adds_the_repo` |
| 4. Approval after I added it: no second clone | Works (real daemon) | `daemon/humatad/tests/channel_repo_propose.rs::an_approval_after_the_user_added_the_repo_succeeds_without_a_second_clone` |
| Branch: another channel | Works (real daemon) | `daemon/humatad/tests/channel_repo_propose.rs::a_channel_agent_asking_for_another_channel_is_denied_and_adds_nothing_to_the_inbox` |
| Branch: Desk agent, any channel | Works (real daemon) | `daemon/humatad/tests/channel_repo_propose.rs::a_desk_agents_proposal_for_any_channel_is_asked` |
| Branch: local folder refused | Works (real daemon) | `daemon/humatad/tests/channel_repo_propose.rs::an_agent_naming_a_local_path_is_refused_and_asks_nothing` |
| Branch: rule Never | Works (real daemon) | `daemon/humatad/tests/channel_add.rs::an_agents_add_repo_is_refused_when_the_repo_add_rule_is_never` |
| Branch: failed clone is retried | Works (real daemon) | `daemon/humatad/tests/channel_add.rs::a_repo_the_channel_has_runs_as_a_retry_and_is_listed_once` |
| Branch: I deny | Code, no test | `daemon/humatad/src/channel_service/add.rs:44-100` |
| Branch: agent drops a repo | Not built | `daemon/humatad/src/channel_service.rs:1137-1142` |
| Branch: agent renames or adds a machine | Works (real daemon) | `daemon/humatad/tests/channel_repo_propose.rs::other_channel_changes_by_an_agent_are_still_never` |

## Gaps and likely bugs

Not built:

- `RemoveRepo` (`humata channel repo rm`, MCP `channel_remove_repo`). `daemon/humatad/src/channel_service.rs:1134-1142` returns not_built for both `PlanRemoveRepo` and `RemoveRepo`, and `daemon/humatad/src/mcp.rs` has no arm for it. Milestone: DESK, Batch D4 (MV-9) says "until then UNIMPLEMENTED naming the method".
- `AddRepoToSession` (`humata session repo add`, MCP `session_add_repo`). `daemon/humatad/src/session_service.rs:2366-2372` returns not_built, and `daemon/humatad/src/mcp.rs` has no arm for it. `workflow/milestones/ROAM.md:67` waits on it. No milestone owns it.
- MCP `channel_add_repo`. The tool is in `daemon/mcp/src/tools.rs:1251-1255`, but `daemon/humatad/src/mcp.rs:1020-1064` has no arm for it, so an MCP call answers not_built. The CLI path works. No milestone.

Likely bugs:

- The reason is dropped. The MCP description says "The user is asked first, so give a reason" (`daemon/mcp/src/tools.rs:1252-1253`). The schema adds an optional `reason` to every ask tool (`daemon/mcp/src/tools.rs:372-376`). But `AddRepoRequest` has no reason field (`api/proto/humata/v1/channels.proto:118-122`), `add_repo_to_channel` takes none (`daemon/humatad/src/channel_service/add.rs:44`), and the CLI has no reason flag (`cli/src/args.rs:297-300`). So the card cannot say why the agent wants the repo. To confirm: propose from the CLI with a reason written in the description, and see that the card has no reason text. The MCP route is not built, so it cannot carry a reason at all.
- `tools/list` advertises every operation, including the ones that answer not_built. `daemon/mcp/src/tools.rs:274-281` lists `Operation::ALL`. Of the 81 operations, 38 have no arm in `daemon/humatad/src/mcp.rs:1020-1064` (InboxWait is served by the wait path). An agent sees tools that cannot work. To confirm: call `channel_add_repo` over MCP and read the not_built answer.

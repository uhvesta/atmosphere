# An agent forks itself onto box-1 only after I approve, and a fork that fails or is cut short leaves nothing behind

Layer: agents. Platforms: macOS, Linux, web (the Inbox card).

## Starting state

- This Mac runs humatad2 as the host. Two boxes, `box-1` and `box-2`, are linked and online. Channel `payments` has all three machines.
- Channel `payments` holds `acme/api`. My parent session "Cache warmup" runs on this Mac. Its clone has one commit, one uncommitted change to `README.md`, and one untracked file, `notes.txt`.
- A session "Retry queue" runs on `box-1`, for the context-only case.
- The rule "Start or fork sessions on another machine" (SessionStartOtherMachine) is at its default, Ask me.
- The Inbox is empty.

## Steps

1. **I do:** the agent asks for the machine list with the MCP tool `topology_get`.
   **I see:** this machine first, then the channel's machines (`box-1` among them), each with `reachability` (`online`, `offline`, or `unknown` when the channel has no repos) and the channel's sessions on it (id, title, agent, status).
   **State and events:** nothing changes.
   **Status:** Works (real daemon, D772): `daemon/humatad/tests/mcp.rs::topology_get_names_this_machine_and_the_channels_sessions_on_it`; order and reachability in `daemon/humatad/src/mcp/topology.rs` unit tests. Reachability comes from `ChannelService.GetGrid` (D764). No test yet has a second machine in the channel.

2. **I do:** the agent calls MCP `session_fork` with `machine: "box-1"`, `kind: "code_and_context"`, `title: "Try it"` and a `reason`.
   **I see:** the call returns outcome `pending`, with a request id. Nothing forks yet. The parent's `forks` list is empty.
   **State and events:** one Pending request for SessionStartOtherMachine. No operation has started.
   **Status:** Works (real daemon). `daemon/humatad/tests/fork_remote.rs::an_agent_forks_itself_onto_another_machine_once_the_user_approves`. The test checks that nothing forks before the answer.

3. **I do:** I open the card in the Inbox.
   **I see:** the card names `box-1` as the machine and the fork kind. It shows the repo with its copy mode (branch and uncommitted) and its clone size. It shows a usage note for the agent's plan. It does not name the branch the fork will use.
   **State and events:** the card has a NewSession preview with those fields.
   **Status:** Works (real daemon) for the machine, kind, copy mode, size and usage note: `daemon/humatad/tests/fork_remote.rs::an_agent_forks_itself_onto_another_machine_once_the_user_approves`. The branch name is not on the card. See the gaps.

4. **I do:** I press Allow.
   **I see:** the agent's wait returns an operation id, and the operation Succeeds. `box-1` gets a clone of `acme/api` on a new branch, with the parent's commit, uncommitted change and untracked file. `box-1` starts the fork's agent. The parent keeps running on this Mac.
   **State and events:** the fork's origin names the parent. The parent's agent is unchanged.
   **Status:** Works (real daemon). `daemon/humatad/tests/fork_remote.rs::an_agent_forks_itself_onto_another_machine_once_the_user_approves` checks the approval, the success, the session live on `box-1`, the parent's commit in the fork, and the parent still live. `daemon/humatad/tests/fork_remote.rs::code_and_context_fork_onto_box_1_has_the_parents_work_on_a_new_branch_and_a_copied_conversation` checks the new branch and the copied conversation. That test drives the fork as the user.

5. **I do:** as the user, I fork the "Retry queue" session on `box-1` to `box-2`, with code only.
   **I see:** `box-2` has the parent's files, and its conversation starts fresh. `box-1`'s agent keeps running on the same holder.
   **State and events:** the new session is live on `box-2`. The parent's holder is the same as before.
   **Status:** Works (real daemon). `daemon/humatad/tests/fork_remote.rs::code_only_fork_from_box_1_onto_box_2_is_a_fresh_agent_and_the_parent_keeps_running`. This test drives the fork as the user, not as an agent.

6. **I do:** I preview a fork onto `box-1` from the CLI: `humata --dry-run session fork <parent> --kind code-and-context --machine box-1`.
   **I see:** the preview names a branch for each repo, a branch the parent is not on. It gives the copy size in bytes. Nothing is made.
   **State and events:** no session and no folder are created on `box-1`.
   **Status:** Works (real daemon). `daemon/humatad/tests/fork_remote.rs::previewing_a_fork_onto_or_from_a_box_names_its_branch_and_size_and_makes_nothing`. The test calls PreviewFork over gRPC. The CLI wrapper has no test.

## Branches

### box-1 is offline

- **When:** the link to `box-1` is lost, and the agent asks for a fork onto it.
- **I do:** nothing.
- **I see:** the request is refused at once as a failed precondition. No card appears.
- **Expected outcome:** nothing is staged on either side, the host lists only the parent, and `box-1` has no folder for the fork.
- **Status:** Works (real daemon). `daemon/humatad/tests/fork_remote.rs::a_fork_onto_an_offline_box_is_refused_and_leaves_nothing`. The test uses gRPC as the user. The agent's MCP path is not tested.

### A typed branch is bad or taken

- **When:** the typed branch is a name git rejects (`bad..name`), or the parent's own branch.
- **I do:** ask for the fork with that branch.
- **I see:** the call is refused before anything is copied. A bad name is an invalid argument, and a taken name is "already exists".
- **Expected outcome:** no operation starts, and `box-1` has no fork folder.
- **Status:** Works (real daemon), over gRPC. `daemon/humatad/tests/fork_remote.rs::a_bad_or_taken_typed_branch_is_refused_when_the_fork_is_asked_for`. The agent cannot type a branch over MCP. See the gaps.

### A context-only fork asked for onto another machine

- **When:** a context-only fork is asked for onto `box-1` from this Mac, or onto this Mac from `box-1`.
- **I do:** nothing.
- **I see:** the call is refused at once as a failed precondition. The fix says to fork with code and context, or to fork context-only onto the parent's machine. The preview is refused the same way.
- **Expected outcome:** nothing is made on either machine.
- **Status:** Works (real daemon). `daemon/humatad/tests/fork_remote.rs::a_context_only_fork_onto_another_machine_is_refused`.

### A context-only fork of a box session, onto that box

- **When:** the "Retry queue" session on `box-1` is forked context-only, onto `box-1`.
- **I do:** ask for the fork.
- **I see:** the fork runs on `box-1` in the parent's clones. No new branch is made, and no code is copied. Its conversation is a copy under its own id. The parent's agent keeps running.
- **Expected outcome:** the host lists the fork on `box-1`, with the parent as its origin, and `box-1` runs two agents.
- **Status:** Works (real daemon). `daemon/humatad/tests/fork_remote.rs::a_context_only_fork_of_a_box_session_onto_that_box_works_in_the_parents_clones_there`.

### The host restarts before the fork's record is written

- **When:** the host is killed during the fork, at a point before the record is written: `fork-packed`, `fork-copied`, `fork-unpacked`, `fork-started` or `fork-copying`.
- **I do:** restart the host.
- **I see:** the fork is undone. The parent runs on. The host lists only the parent. `box-1` keeps no record, agent or folder of the fork. Nothing is staged on either side.
- **Expected outcome:** forking again with a new key works.
- **Status:** Works (real daemon). `daemon/humatad/tests/fork_remote.rs::a_fork_cut_short_by_a_host_restart_is_undone_or_finished` (the packed, copied and unpacked cases), `daemon/humatad/tests/fork_remote.rs::a_fork_cut_short_as_it_starts_is_undone_at_restart` and `daemon/humatad/tests/fork_remote.rs::a_fork_cut_short_as_its_copy_begins_is_undone_at_restart`.

### The host restarts after the fork's record is written

- **When:** the host is killed at `fork-recorded`, once the record is written.
- **I do:** restart the host.
- **I see:** the fork finishes on `box-1` with the parent's work.
- **Expected outcome:** the fork is live on `box-1`, and the parent's commit is in it.
- **Status:** Works (real daemon). `daemon/humatad/tests/fork_remote.rs::a_fork_cut_short_by_a_host_restart_is_undone_or_finished` (the recorded case).

### A context-only fork is cut after box-1 took it

- **When:** the host is killed at `fork-context-asked`, after `box-1` has taken a context-only fork of its own session.
- **I do:** restart the host, once `box-1`'s link is up.
- **I see:** the host lists the fork once, on `box-1`, with the parent as its origin. `box-1` made the fork once.
- **Expected outcome:** one fork, not two. Its journal is done with.
- **Status:** Works (real daemon). `daemon/humatad/tests/fork_remote.rs::a_context_only_fork_on_a_box_cut_short_by_a_host_restart_is_listed_once_the_box_is_linked`.

### The agent asks for a different agent on another machine

- **When:** a fork onto `box-1` names an agent that is not the parent's.
- **I do:** ask for the fork with that agent.
- **I see:** the call is refused as not built.
- **Expected outcome:** nothing is made.
- **Status:** Not built. `daemon/humatad/src/session_service/fork_remote.rs:270-276`. Neither MCP nor the CLI sends an agent on a fork (`daemon/humatad/src/mcp.rs:698-737`, `cli/src/commands/sessions.rs:132-137`), so only a gRPC caller can reach this refusal.

### I deny the box card

- **When:** after step 2, I press Deny.
- **I do:** Deny on the card.
- **I see:** the agent's call ends with a denied error. Nothing is copied to `box-1`.
- **Expected outcome:** the request is Denied, the host lists only the parent, and `box-1` has no fork folder.
- **Status:** Code, no test. Every box-fork test approves the card, and none denies it. The generic deny is tested for another call, `daemon/humatad/tests/inbox.rs::a_denied_deferred_call_never_runs`.

### The fork names a machine this host does not know

- **When:** the agent asks for a fork onto `box-9`, which is not linked to this host, and this host has no boxes.
- **I do:** nothing.
- **I see:** the refusal says the fork "isn't built into this humatad2 yet", and the fix says to update Humata. An update will not fix an unknown machine.
- **Expected outcome:** a refusal that names the machine as unknown, with nothing made.
- **Status:** Works (real daemon): `daemon/humatad/tests/fork.rs::a_fork_onto_a_machine_this_host_does_not_know_is_refused_as_an_unknown_machine`. The refusal names the machine as unknown and nothing is made. (Was a likely bug: it said "not built"; fixed by BUGS.)

## Expected outcomes

- [ ] An agent's fork onto another machine waits as an Inbox card, and makes nothing until I approve.
- [ ] The card names the machine, the kind, each repo's copy mode and size, and a usage note.
- [ ] An approved code-and-context fork gives `box-1` a clone on a new branch, with the parent's commit, uncommitted change and untracked file, and a copy of the conversation under its own id.
- [ ] The parent keeps running on this Mac, with the same agent.
- [ ] A code-only fork from `box-1` to `box-2` starts a fresh conversation, and `box-1`'s agent keeps running.
- [ ] A preview names a branch for each repo and the copy size, and makes nothing.
- [ ] A fork onto an offline box is refused at once, and leaves nothing on either side.
- [ ] A bad or taken typed branch is refused before anything is copied.
- [ ] A context-only fork across machines is refused. A context-only fork onto its own box works in the parent's clones.
- [ ] A host restart before the record undoes the fork. A restart after the record finishes it.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. `topology_get` (machine list) | Works (D772) | `daemon/humatad/tests/mcp.rs::topology_get_names_this_machine_and_the_channels_sessions_on_it` |
| 2. Agent asks: pending, nothing forks | Works (real daemon) | `daemon/humatad/tests/fork_remote.rs::an_agent_forks_itself_onto_another_machine_once_the_user_approves` |
| 3. Card: machine, kind, copy, size, usage note | Works (real daemon) | `daemon/humatad/tests/fork_remote.rs::an_agent_forks_itself_onto_another_machine_once_the_user_approves` |
| 4. Approve: code and context onto box-1 | Works (real daemon) | `daemon/humatad/tests/fork_remote.rs::code_and_context_fork_onto_box_1_has_the_parents_work_on_a_new_branch_and_a_copied_conversation` |
| 5. Code only from box-1 to box-2 | Works (real daemon) | `daemon/humatad/tests/fork_remote.rs::code_only_fork_from_box_1_onto_box_2_is_a_fresh_agent_and_the_parent_keeps_running` |
| 6. Preview names branch and size, makes nothing | Works (real daemon) | `daemon/humatad/tests/fork_remote.rs::previewing_a_fork_onto_or_from_a_box_names_its_branch_and_size_and_makes_nothing` |
| Branch: box offline | Works (real daemon) | `daemon/humatad/tests/fork_remote.rs::a_fork_onto_an_offline_box_is_refused_and_leaves_nothing` |
| Branch: bad or taken typed branch | Works (real daemon) | `daemon/humatad/tests/fork_remote.rs::a_bad_or_taken_typed_branch_is_refused_when_the_fork_is_asked_for` |
| Branch: context-only across machines refused | Works (real daemon) | `daemon/humatad/tests/fork_remote.rs::a_context_only_fork_onto_another_machine_is_refused` |
| Branch: context-only onto its own box | Works (real daemon) | `daemon/humatad/tests/fork_remote.rs::a_context_only_fork_of_a_box_session_onto_that_box_works_in_the_parents_clones_there` |
| Branch: host restart before record, undone | Works (real daemon) | `daemon/humatad/tests/fork_remote.rs::a_fork_cut_short_by_a_host_restart_is_undone_or_finished` |
| Branch: host restart after record, finished | Works (real daemon) | `daemon/humatad/tests/fork_remote.rs::a_fork_cut_short_by_a_host_restart_is_undone_or_finished` |
| Branch: context-only cut, listed once | Works (real daemon) | `daemon/humatad/tests/fork_remote.rs::a_context_only_fork_on_a_box_cut_short_by_a_host_restart_is_listed_once_the_box_is_linked` |
| Branch: a different agent on another machine | Not built | `daemon/humatad/src/session_service/fork_remote.rs:270-276` |
| Branch: I deny the box card | Code, no test | Generic deny only: `daemon/humatad/tests/inbox.rs::a_denied_deferred_call_never_runs` |
| Branch: unknown machine | Works (real daemon) | `daemon/humatad/tests/fork.rs::a_fork_onto_a_machine_this_host_does_not_know_is_refused_as_an_unknown_machine` |

## Gaps and likely bugs

Not built:

- A typed branch over MCP. The `session_fork` schema has no `branch` field (`daemon/mcp/src/tools.rs:1097-1116`), though `ForkSessionRequest.branch` exists (`api/proto/humata/v1/sessions.proto:356-358`). The CLI has `--branch` (`cli/src/args.rs:372-377`). No milestone.
- The box card names no branch. `NewSessionPreview.Repo` has the repo, the copy mode and the size, but no branch name (`api/proto/humata/v1/inbox.proto:293-297`). The brief's preview names the branch, so the card falls short here. No milestone.
- A fork onto another machine that runs a different agent (`daemon/humatad/src/session_service/fork_remote.rs:270-276`). Only a gRPC caller can ask for it.

Likely bugs:

- Fixed (was a likely bug): an unknown or unlinked machine is refused as unknown, not as not built (`daemon/humatad/tests/fork.rs::a_fork_onto_a_machine_this_host_does_not_know_is_refused_as_an_unknown_machine`).

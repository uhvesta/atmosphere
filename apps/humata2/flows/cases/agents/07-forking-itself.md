# An agent forks itself, and each fork kind carries what it says: code and context, code only, or context only

Layer: agents. Platforms: macOS, Linux, web (the Inbox card).

## Starting state

- This Mac runs humatad2. This Mac is the only machine.
- Channel `payments` holds `acme/api`. My session "Fix login" is running, and its agent is mine. The agent has its session token in its shell.
- The session's clone has one commit (`login`), one uncommitted change to `README.md`, and one untracked file, `notes.txt`.
- The session's conversation has three turns.
- The rule "Start or fork sessions on this machine" (SessionStartSameMachine) is at its default, Ask me.
- The app is open on `payments`, with my session selected.

## Steps

1. **I do:** the agent runs `humata --dry-run session fork <its session> --kind code-and-context --title "Try a retry"`.
   **I see:** a preview. It gives the text, the branch the fork's clone would get (named from the title, and not the parent's branch), the copy size, and whether a summary would seed the fork.
   **State and events:** no session is created. The list still shows one session.
   **Status:** Works (real daemon). `daemon/humatad/tests/fork.rs::the_preview_names_the_forks_branch_and_what_it_copies`. The test calls PreviewFork as the user, not as the agent. The `--dry-run` wiring in `cli/src/commands/sessions.rs:132-137` has no test.

2. **I do:** the agent asks for the same fork: `humata session fork <its session> --kind code-and-context --title "Try a retry"`. Over MCP, the same is `session_fork` with `kind`, `machine: ""` and `reason`.
   **I see:** nothing forks yet, and the call waits. An Inbox card appears. It says a new agent will start, and it shows the kind, the repo with its copy mode and size, and the usage note. The card shows no reason, even if the agent gave one.
   **State and events:** one Pending request with a NewSession preview. The parent's `forks` list is still empty.
   **Status:** Code, no test. The gate is `daemon/humatad/src/session_service/fork.rs:370-390` (SessionStartSameMachine). The card is built in `daemon/humatad/src/session_service/fork_card.rs:15-80`. The only agent-side fork test is for a box: `daemon/humatad/tests/fork_remote.rs::an_agent_forks_itself_onto_another_machine_once_the_user_approves`.

3. **I do:** I press Allow on the card.
   **I see:** the card leaves the Inbox. The fork appears under `payments`, with its own clone and branch. Its clone has the parent's commit, uncommitted change and untracked file. Its conversation is a copy under its own id, with the parent's turns.
   **State and events:** the operation Succeeds with the fork's session id. The parent's `forks` lists the fork. The fork's origin names the parent and the kind.
   **Status:** Works (real daemon). `daemon/humatad/tests/fork.rs::code_and_context_copies_the_work_and_the_conversation`. The test drives the fork as the user. The agent's waiting call and its approval on this machine are not tested.

4. **I do:** the agent forks with `--kind code-only`.
   **I see:** the fork has the parent's work on a new branch, and its conversation starts fresh. The parent's turns are not in it.
   **State and events:** the fork's origin kind is CodeOnly.
   **Status:** Works (real daemon). `daemon/humatad/tests/fork.rs::code_only_copies_the_work_into_a_fresh_conversation`.

5. **I do:** the agent forks with `--kind context-only`.
   **I see:** the fork carries the parent's conversation and works in the parent's clone, on the parent's branch. No new branch is made. The preview says no code is copied.
   **State and events:** the fork's origin kind is ContextOnly. Its conversation id differs from the parent's.
   **Status:** Works (real daemon). `daemon/humatad/tests/context_fork.rs::a_context_only_fork_carries_the_conversation_and_works_in_the_parents_code`.

6. **I do:** later, the context-only fork asks for its own copy of the code. Over MCP this is `session_own_copy`.
   **I see:** (expected) the fork gets its own clones on a new branch, and its conversation goes on.
   **State and events:** (expected) the fork's clones move onto their own folders.
   **Status:** Not built on the agent surface. The tool is listed in `daemon/mcp/src/tools.rs:1139-1143`, but `daemon/humatad/src/mcp.rs:1020-1064` has no arm for `SessionOwnCopy`, so the call answers not_built. The daemon's own-copy works over gRPC: `daemon/humatad/tests/context_fork.rs::giving_a_context_only_fork_its_own_copy_moves_it_onto_its_own_clones`.

7. **I do:** the agent forks at an earlier turn, before turn 2. It tries `humata session fork <its session> --kind code-and-context --before-turn 2`.
   **I see:** the CLI does not accept the flag. Over MCP there is no `before_turn` field, so the agent cannot ask for a fork at a turn.
   **State and events:** nothing forks at turn 2.
   **Status:** Not built on the agent surface. The daemon supports it: `daemon/humatad/tests/context_fork.rs::a_fork_before_a_turn_carries_the_conversation_up_to_it_and_the_code_as_now` (called as the user). The CLI and MCP never set `before_turn`: `cli/src/commands/sessions.rs:130` sets it to 0, and `daemon/humatad/src/mcp.rs:698-737` does not send it. No milestone names the agent surface.

8. **I do:** the agent forks its own fork, so the fork has a child.
   **I see:** the child names its parent (the fork). The parent lists the child. The three sessions have three different branches.
   **State and events:** the child's origin names the fork. The fork's `forks` lists the child.
   **Status:** Works (real daemon). `daemon/humatad/tests/fork.rs::forks_nest_and_each_names_its_parent`. The test drives the forks as the user.

## Branches

### A fork with another agent

- **When:** the agent asks for the fork with a different agent (the same as the case in agents-10).
- **I do:** I approve the card.
- **I see:** the fork runs the other agent. Its conversation is handed over as a file beside the clones, and the first message names that file.
- **Expected outcome:** the fork's agent is the other one. The handover file exists and holds the conversation. The first message is typed once.
- **Status:** Works (real daemon). `daemon/humatad/tests/fork.rs::a_fork_with_another_agent_is_handed_the_conversation_and_a_summary`. The full case is in the agents-10 brief.

### An agent with no conversation format

- **When:** the parent's agent has no conversation format.
- **I do:** the agent asks for a fork before a turn, and then a fork from now.
- **I see:** the list of turns is empty. The fork before a turn is refused as a failed precondition. The fork from now works.
- **Expected outcome:** no turns are listed, and only the fork from now is made.
- **Status:** Works (real daemon). `daemon/humatad/tests/context_fork.rs::an_agent_without_a_conversation_format_has_no_turns_to_fork_at`.

### A fork before a turn that is not there

- **When:** a fork before turn 4 of a three-turn conversation, or before turn 1.
- **I do:** the fork is asked for over gRPC. The agent surface cannot ask for it (step 7).
- **I see:** the call is refused with an invalid argument.
- **Expected outcome:** no fork is made.
- **Status:** Works (real daemon). `daemon/humatad/tests/context_fork.rs::a_fork_before_a_turn_that_is_not_there_is_refused`.

### The MCP call asks for some repos or a first message

- **When:** the agent passes `repos: ["acme/api"]`, or `first_message: "start with the tests"`.
- **I do:** nothing.
- **I see:** the call is refused as an invalid field. The message says that forking some repos, or with a first message, is not built yet. The fix says to type the message after the fork, with `session_type`. Nothing forks.
- **Expected outcome:** no fork, and the session list is unchanged.
- **Status:** Works (real daemon). `daemon/humatad/tests/mcp.rs::a_fork_of_some_repos_or_with_a_first_message_is_refused_as_an_invalid_field`. The refusal is tested. The features behind it are Not built (see the gaps).

### A fork cut before its agent starts is undone at restart

- **When:** the host is killed during the fork, at one of the points `write_record`, `copy_code:api`, `copy_conversation` or `launch`.
- **I do:** restart the host.
- **I see:** the fork is gone. Its session, folder and conversation copy are removed. My session is intact, with its work.
- **Expected outcome:** the parent lists no fork, and the parent's clone keeps its commit, change and file.
- **Status:** Works (real daemon). `daemon/humatad/tests/fork.rs::a_fork_cut_before_its_agent_starts_is_undone_at_restart`. Also `daemon/humatad/tests/fork.rs::a_fork_cut_when_its_records_are_saved_is_undone_at_restart`, `...::a_fork_cut_after_its_conversation_is_copied_is_undone_at_restart`, `...::a_fork_cut_before_its_agent_launches_is_undone_at_restart`, and `daemon/humatad/tests/context_fork.rs::a_context_only_fork_cut_when_its_records_are_saved_is_undone_at_restart`. These cut forks the user started.

### A fork cut after its agent started is kept at restart

- **When:** the host is killed at `fork.verify`, after the agent has started.
- **I do:** restart the host.
- **I see:** the fork is still there. It has its origin and its conversation copy.
- **Expected outcome:** the fork is listed under the parent and works.
- **Status:** Works (real daemon). `daemon/humatad/tests/fork.rs::a_fork_cut_after_its_agent_started_is_kept_at_restart`.

### I deny the fork card

- **When:** after step 2, I press Deny.
- **I do:** Deny on the card.
- **I see:** the agent's call ends with a denied error. No fork is made.
- **Expected outcome:** the request is Denied, and the parent is unchanged.
- **Status:** Code, no test for the same-machine fork card. The generic deny is tested for a different call: `daemon/humatad/tests/inbox.rs::a_denied_deferred_call_never_runs`.

### The MCP call names an unknown kind

- **When:** the agent passes `kind: "copy_everything"`.
- **I do:** nothing.
- **I see:** the call is refused as an invalid field. The fix lists the three kinds. Nothing forks.
- **Expected outcome:** no fork.
- **Status:** Code, no test. The refusal is `daemon/humatad/src/mcp.rs:700-705` (`fork_unsupported`).

## Expected outcomes

- [ ] The preview names the fork's branch and the copy size, and makes nothing.
- [ ] A same-machine fork asked by the agent waits as an Inbox card, under the default rule of Ask me.
- [ ] Code and context: the fork has the parent's commit, uncommitted change and untracked file, on its own branch. Its conversation is a copy under its own id.
- [ ] Code only: the fork has the code and a fresh conversation.
- [ ] Context only: the fork shares the parent's clone and branch, and copies no code.
- [ ] A fork of a fork names its parent, and the parent lists the fork.
- [ ] A fork before a turn that is not there is refused.
- [ ] An agent without a conversation format has no turns, and can fork from now only.
- [ ] A fork with some repos, or with a first message, is refused as an invalid field, and nothing forks.
- [ ] A fork cut before its agent starts is gone after a restart. A fork cut after its agent started is kept.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Preview names branch and copy size | Works (real daemon) | `daemon/humatad/tests/fork.rs::the_preview_names_the_forks_branch_and_what_it_copies` |
| 2. Agent asks: Inbox card, nothing forks | Code, no test | `daemon/humatad/src/session_service/fork.rs:370-390`; `daemon/humatad/src/session_service/fork_card.rs:15-80` |
| 3. Approve: code and conversation copied | Works (real daemon) | `daemon/humatad/tests/fork.rs::code_and_context_copies_the_work_and_the_conversation` |
| 4. Code only: fresh conversation | Works (real daemon) | `daemon/humatad/tests/fork.rs::code_only_copies_the_work_into_a_fresh_conversation` |
| 5. Context only: parent's clone | Works (real daemon) | `daemon/humatad/tests/context_fork.rs::a_context_only_fork_carries_the_conversation_and_works_in_the_parents_code` |
| 6. Context-only fork asks for its own copy (MCP) | Not built | `daemon/mcp/src/tools.rs:1139-1143`; no arm in `daemon/humatad/src/mcp.rs:1020-1064` |
| 7. Fork before a turn (agent surface) | Not built | `cli/src/commands/sessions.rs:130`; `daemon/humatad/src/mcp.rs:698-737` |
| 8. Fork of a fork names its parent | Works (real daemon) | `daemon/humatad/tests/fork.rs::forks_nest_and_each_names_its_parent` |
| Branch: another agent | Works (real daemon) | `daemon/humatad/tests/fork.rs::a_fork_with_another_agent_is_handed_the_conversation_and_a_summary` |
| Branch: no conversation format | Works (real daemon) | `daemon/humatad/tests/context_fork.rs::an_agent_without_a_conversation_format_has_no_turns_to_fork_at` |
| Branch: before a turn not there | Works (real daemon) | `daemon/humatad/tests/context_fork.rs::a_fork_before_a_turn_that_is_not_there_is_refused` |
| Branch: MCP refuses repos or first message | Works (real daemon) | `daemon/humatad/tests/mcp.rs::a_fork_of_some_repos_or_with_a_first_message_is_refused_as_an_invalid_field` |
| Branch: cut before agent starts | Works (real daemon) | `daemon/humatad/tests/fork.rs::a_fork_cut_before_its_agent_starts_is_undone_at_restart` |
| Branch: cut after agent started | Works (real daemon) | `daemon/humatad/tests/fork.rs::a_fork_cut_after_its_agent_started_is_kept_at_restart` |
| Branch: I deny the card | Code, no test | Generic deny only: `daemon/humatad/tests/inbox.rs::a_denied_deferred_call_never_runs` |
| Branch: unknown kind over MCP | Code, no test | `daemon/humatad/src/mcp.rs:700-705` |

## Gaps and likely bugs

Not built:

- Context-only fork gets its own copy from the agent (step 6). The MCP arm is missing. The daemon's GiveOwnCopy is built.
- Fork at an earlier turn from the agent (step 7). The daemon supports it. The CLI and MCP never send `before_turn`. No milestone names the agent surface.
- Fork with some repos, or with a first message (branch "MCP refuses repos or first message"). The refusal is tested, but no milestone was found for the feature. The refusal says to type the message after the fork, with `session_type`.
- The same-machine Inbox card names no branch. `NewSessionPreview.Repo` has only the repo, the copy mode and the size (`api/proto/humata/v1/inbox.proto:293-297`). No milestone.

Likely bugs:

- The reason is dropped. The MCP schema requires `reason` (`daemon/mcp/src/tools.rs:1111`). The handler never reads it (`daemon/humatad/src/mcp.rs:698-737`), and the fork card is built with an empty reason (`daemon/humatad/src/session_service/fork_card.rs:75`). The user never sees why the agent wants the fork. To confirm: call `session_fork` with a reason, then read the card's reason, which is empty.
- The own-copy milestone says the MCP side is done. `workflow/milestones/FORK.md:47` says own-copy "takes no repo in the CLI and MCP", but `daemon/humatad/src/mcp.rs` has no `SessionOwnCopy` arm. To confirm: call `session_own_copy` over MCP and expect not_built.
- The machine argument is required, but an empty value means this machine. The schema requires `machine` (`daemon/mcp/src/tools.rs:1101`), and a missing required field is rejected (`daemon/mcp/src/schema.rs:388`). The handler treats `""` as this machine (`daemon/humatad/src/mcp.rs:698-737`). The description does not say to send `""`. To confirm: call `session_fork` without `machine`, and expect "machine is required".

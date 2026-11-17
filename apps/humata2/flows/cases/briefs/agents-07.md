# Brief agents-07: An agent forks itself
Output: `apps/humata2/flows/cases/agents/07-forking-itself.md`

Scenario: an agent wants to try two approaches. It forks itself code-and-context, then code-only, then context-only, then forks at an earlier turn. The user also forks from the Fork dialog.

Steps: preview names the fork's branch and what it copies; code and context copies the work and conversation; code only gives a fresh conversation; context only works in the parent's folder (and can get its own copy later); fork before a turn carries the conversation up to it; forks nest and name their parent.

Branches: a fork with another agent (handed the conversation and a summary — see agents-10); an agent with no conversation format has no turns; a fork of some repos or with a first message via MCP is refused as invalid; cuts at each step undone or kept at restart.

Pointers: daemon/humatad/tests/fork.rs, context_fork.rs, mcp.rs (`a_fork_of_some_repos_or_with_a_first_message_is_refused_as_an_invalid_field`), desktop/tests/native/session.rs `the_fork_dialog_creates_the_selected_kind_and_can_cancel`, desktop/tests/native/fork_move.rs, daemon/humatad/src/session_service/fork.rs (not_built parts), workflow/milestones/FORK.md.

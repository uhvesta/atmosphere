# Brief agents-02: An agent types into sessions
Output: `apps/humata2/flows/cases/agents/02-typing-into-sessions.md`

Scenario: an agent types a command into its own session's shell tab via MCP `session_type`, then tries another session's terminal, then tries to open a shell or paste (user-only), then types into an exited shell.

Steps: own session: typed at once; another session: waits for the user's Ask approval in the Inbox; open a shell / paste: refused as the user's alone; exited shell: refused.

Branches: the user denies; the approval arrives after the agent stopped waiting (withdrawn); a keyed input sent twice types once; when TURN_ENDS (queued) — not_built (terminal.rs ~912); TerminalInput access op.

Pointers: daemon/humatad/tests/mcp.rs (`an_agent_types_into_its_own_session_through_mcp`, `opening_a_shell_or_pasting_is_the_users_alone`, `a_question_the_agent_stops_waiting_on_is_withdrawn`), daemon/humatad/tests/terminal.rs (an agent's typing into another session waits for Ask approval; typing into an exited shell is refused), daemon/holder/tests/keyed_input.rs, daemon/humatad/src/terminal.rs.

# Brief agents-05: Reading other sessions and channels under the rules
Output: `apps/humata2/flows/cases/agents/05-reading-under-the-rules.md`

Scenario: an agent lists channels, reads another session's turns, and searches sessions. I change the channel's agent-access rule from the default to Ask, then to Never, then Allow, from the agent access tab.

Steps: default rule: list channels at once; Ask: the call is deferred, a card with a preview appears, approve runs it and the result is waited for; Never: denied; the user lists without asking; deferred card offers no Always without a target.

Branches: a denied deferred call never runs; after a restart a pending deferred request is withdrawn; `access explain` says why; ReadConversation not_built (session_service.rs) — what does session_read do instead?

Pointers: daemon/humatad/tests/inbox.rs (deferred tests, `an_agent_listing_another_sessions_turns_waits_in_the_inbox_when_the_rule_asks`), mcp.rs `an_agent_lists_the_channels_once_the_user_approves`, desktop/tests/live_inbox.rs (the agent access tab sets the channel's rule; real daemon), daemon/humatad/tests/access.rs, history_tools.rs, daemon/access/.

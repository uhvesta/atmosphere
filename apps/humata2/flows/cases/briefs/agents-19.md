# Brief agents-19: An agent across a restart
Output: `apps/humata2/flows/cases/agents/19-an-agent-across-a-restart.md`

Scenario: an agent has an ask-ahead pending, a deferred call pending, a hook holding, and a terminal token, when the daemon restarts (upgrade, crash) and then when the whole Mac reboots.

Steps: the tab's token still works after a daemon restart; a pending deferred request is withdrawn and an ask-ahead still waits; a hook with the daemon gone prints nothing and exits 0; the Inbox and grants survive; the agent's conversation resumes after reboot; a fork or move cut midway is undone or finished.

Branches: the agent was mid-turn at reboot (resumes in its conversation); its conversation is lost ("Couldn't resume"); a handover cut after launch is typed once.

Pointers: daemon/humatad/tests/mcp.rs `a_tab_that_outlives_a_restart_keeps_its_token`, inbox.rs (`after_a_restart_a_pending_deferred_request_is_withdrawn_and_an_ask_ahead_still_waits`, `a_restart_keeps_the_inbox_and_the_grants_still_standing`), hooks.rs `with_the_daemon_gone_the_hook_prints_nothing_and_exits_0`, desktop/tests/live_flows.rs reboot tests, fork.rs cut tests.

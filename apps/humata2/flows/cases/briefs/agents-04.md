# Brief agents-04: Ask ahead and grants
Output: `apps/humata2/flows/cases/agents/04-ask-ahead-and-grants.md`

Scenario: before a long job the agent asks ahead for write access to `docs/`. I approve it. It writes under docs/ without asking; it tries the parent and is asked again; I revoke the grant; the next write waits for me.

Steps: an ask-ahead card; approval makes a grant; a folder grant covers what is under it and not its parent; an approved ask-ahead covers the same ask and nothing wider; revoke makes the next ask wait; the feed tells of each grant and revoke; the agent lists only its own session's grants.

Branches: Approve all never approves an ask-ahead; approving a deferred call once makes no grant; a restart keeps grants; revoking a grant that isn't there is not found; session-only grants and the Reply editor are disabled in the UI (OVERVIEW gap).

Pointers: daemon/humatad/tests/inbox.rs (many), daemon/humatad/tests/mcp.rs `an_agent_asks_ahead_for_access_and_gets_the_grant_or_the_users_no`, daemon/access/tests/*, features/inbox/tests/answers.rs, ui/src (grep "Always" "Not now").

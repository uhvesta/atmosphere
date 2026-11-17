# Brief agents-15: An agent prepares a view for the user
Output: `apps/humata2/flows/cases/agents/15-preparing-a-view.md`

Scenario: an agent finishes a migration and wants to show me the three files that matter, in order. It prepares a view, revises it, marks it ready; I see an offer to Open it or Later; I pick Later, come back, and it is offered again; another time I dismiss it.

Steps: view_prepare / view_revise / view_mark_ready; a draft is the agent's until offered, then the user's; a draft is checked before it is kept; prepared views wait for Open and Later returns on the next visit; a dismissed one never returns; the user lists every offered view newest first; the feed tells the agent of drafts and the user of offers only; a restart keeps it.

Branches: ViewsPrepare (plural) op not in dispatch; a view draft while the host is away is host_unreachable; VIEWS milestone (planning) — what remains?; ViewStateService not built (check not_built.rs).

Pointers: daemon/humatad/tests/mcp.rs `an_agent_prepares_revises_and_offers_a_view`, review.rs (draft view tests), features/views/tests/offers.rs, desktop/tests/native/session.rs `prepared_views_wait_for_open_and_later_returns_on_the_next_visit`, workflow/milestones/VIEWS.md, daemon/humatad/src/not_built.rs.

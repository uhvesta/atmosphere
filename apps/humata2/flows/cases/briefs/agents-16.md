# Brief agents-16: The review chat drives the view
Output: `apps/humata2/flows/cases/agents/16-review-chat-drives-the-view.md`

Scenario: in a review, I ask the review chat "where is the retry logic?". It focuses the view on that file and line, leaves notes, orders the review with a plan; I undo a move it made and later turn its view control off.

Steps: review_context, review_focus, review_note(/edit/drop), review_prepare; the chat never reviews on its own; the agent may read and focus but not change my review state; notes are the agent's and I can turn them off; a later diff marks notes outdated; list comments by file; undo and a repeated key act the same after a restart.

Branches: notes Resolve/Reopen/Reattach not_built (review.rs); the agent hears of notes on every session it may list; review order (ReviewOrder op) built?

Pointers: daemon/humatad/tests/review.rs, mcp.rs (`the_review_chats_agent_shows_the_user_a_place_and_reads_it_back`, `the_review_chats_agent_leaves_notes_and_orders_the_review`), features/review/tests/*, desktop/tests/live_flows/review.rs, SPEC PR-22–26, D59.

# Brief macro-04: Write a proposal doc
Output: `apps/humata2/flows/cases/macro/04-write-a-proposal-doc.md`

Scenario: I need a two-page proposal for moving the queue to a new store. I start a desk session "queue proposal", ask the agent to draft PROPOSAL.md from three repos' code, open the doc in the app, leave comments on paragraphs, the agent answers and revises, I edit a paragraph myself while it writes, then copy the final doc into the channel's shared folder.

Steps: document view of markdown; document comments (review flows have "document comments"); agent replies and resolves (MCP comment_reply / comment_resolve); concurrent edit by me and the agent (who wins? is it detected?); copy to channel shared folder.

Branches: the agent edits a line I commented on (comment outdated or moved?); ReattachComment/ResolveComment/ReopenComment for a note answer not_built (daemon/humatad/src/review.rs); Files "Open in editor" (OVERVIEW gap); restart while comments are open.

Pointers: desktop/tests/live_flows/review.rs (document comments; real daemon), desktop/tests/native/document.rs, editor.rs, editor_comments.rs, daemon/humatad/tests/comments.rs, comment_tools.rs, daemon/humatad/tests/desk.rs (copy into channel's shared folder).

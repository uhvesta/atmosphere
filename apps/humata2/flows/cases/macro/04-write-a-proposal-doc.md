# A proposal doc, written by the agent and commented on, ends up in the channel's shared folder

Layer: macro. Platforms: macOS, Linux, web. (The keys shown are the macOS ones; Linux uses `Ctrl`.)

## Starting state

- Humata 2 is open on This Mac. The daemon is running.
- Machines: This Mac (online). Box-1 is not used here.
- A channel named `queue` exists, with `acme/api`, `acme/ledger` and `acme/web` as repos, and a shared folder.
- No Desk session named "queue proposal" exists yet.
- I hold the request: "a two-page proposal for moving the queue to a new store, drafted from the code in the three repos".

## Steps

1. **I do:** Open the Desk and start a session titled "queue proposal".
   **I see:** The Desk form, then the session opens with its agent starting. It is listed on the Desk, not in a channel.
   **State and events:** A Desk session row is recorded. Its folder is named from the title.
   **Status:** Works (real daemon). `desktop/tests/live_flows/desk.rs::a_desk_session_starts_from_the_page_gets_a_repo_and_its_document_is_copied_to_a_channel` starts a Desk session from the Desk page, and it is listed on the Desk, not in a channel. Fake-client version: `features/app/tests/desk.rs::empty_home_starts_a_desk_session_and_opens_it`. The daemon side is step 2.

2. **I do:** Add the three repos to the session when I start it (or add them afterwards).
   **I see:** Each repo is cloned into the session's folder.
   **State and events:** One clone per repo is recorded under the session. Adding a repo again changes nothing.
   **Status:** Works (real daemon). `daemon/humatad/tests/desk.rs::a_start_with_repos_clones_each_into_the_folder` and `an_added_repo_is_cloned_and_listed_and_a_second_add_changes_nothing`.

3. **I do:** Ask the agent in the terminal to write `PROPOSAL.md` from the three repos' code.
   **I see:** The agent works. `PROPOSAL.md` appears in the session's folder and is listed among the session's documents.
   **State and events:** The file is recorded. The Diff panel lists it under the session.
   **Status:** Works (real daemon). `desktop/tests/live_flows/review.rs::a_document_is_commented_on_rewritten_and_its_comments_followed` (steps 1 and 2) has the agent write a markdown file and sees it in the session's changes. The agent is a fake that writes the file.

4. **I do:** Open `PROPOSAL.md`.
   **I see:** The file opens as a tab beside the terminal, showing the rendered page. The modes are Preview, Source and Side by side.
   **State and events:** The tab's group is split from the terminal's group. Side by side puts the page and its source together.
   **Status:** Works (real daemon). `desktop/tests/live_flows/review.rs::a_document_is_commented_on_rewritten_and_its_comments_followed` opens the file as a tab, moves it into a second group and switches to Side by side. The page's look is also checked in `desktop/tests/native/document.rs` (fakes).

5. **I do:** Click the source line of paragraph 7 and type "Where are the sizes from?". Press Comment.
   **I see:** A comment box under line 7. The comment shows as sent to the agent.
   **State and events:** The comment is typed into the session's agent as one message with the line `[comment on <path>:7]`. Its status moves from draft to sent.
   **Status:** Works (real daemon). The same live test (`desktop/tests/live_flows/review.rs::a_document_is_commented_on_rewritten_and_its_comments_followed`) opens the box, types, and sends. It drives the app's actions on a real daemon, not a mouse.

6. **I do:** Leave two more comments on lines 3 and 12 as drafts. Press Send 2 comments.
   **I see:** Two drafts, then both are sent with one paste holding both lines.
   **State and events:** Two draft comments wait. Send all types them as one bracketed paste. Each comment is typed once.
   **Status:** Works (real daemon). Same test, step 4 of the live flow: "one paste with both", and the count of `[comment on` lines is checked.

7. **I do:** Nothing. The agent replies in the thread with `comment_reply`, then resolves the comment with `comment_resolve` and a note.
   **I see:** The reply shows under the comment, marked with the agent's name. The comment is Resolved with the note.
   **State and events:** `comment_reply` adds a reply. `comment_resolve` needs a reply and a comment that was sent, and it is idempotent per key.
   **Status:** Works (real daemon). `daemon/humatad/tests/comment_tools.rs::an_agent_finds_replies_to_and_resolves_a_comment_sent_to_its_session` and `resolving_needs_a_reply_and_a_comment_that_was_sent`, and `daemon/humatad/tests/comments.rs::the_agents_reply_and_resolve_happen_once_per_idempotency_key`. The MCP client is a test harness. How the reply looks in the thread is checked only by design goldens (`desktop/tests/design/comments.rs`).

8. **I do:** Nothing. The agent rewrites the file. Line 7 changes and the diagram paragraph is gone.
   **I see:** The sizes comment shows as addressed, with its passage before and after the change. The arrows comment shows as detached, with Re-attach and Drop offered.
   **State and events:** A sent comment whose passage changed becomes addressed. A sent comment whose passage is gone becomes a detached draft (SM-7, SM-16).
   **Status:** Works (real daemon) for the daemon's rule. The live test checks the sizes comment is addressed and the arrows comment is detached. The daemon's watcher is `daemon/humatad/src/review/comments.rs:655-660`, and the rule is `daemon/review_diff/src/comments.rs`. A second copy of the rule in the app's review model gives a different result for the gone case. See the likely bug under the branch "the agent edits a line I commented on".

9. **I do:** Press Resolve on the addressed comment. Press Re-attach on the detached one, select paragraph 10, and Send.
   **I see:** The addressed comment is resolved and hidden. The re-attached comment is a draft on line 10, and after Send the agent gets it.
   **State and events:** Resolved stays resolved, even after a later change (SM-17). A re-attached comment is a draft until it is sent again.
   **Status:** Works (real daemon). Same live test, step 6: `CommentsAction::Resolve` and `CommentsAction::Reattach`, then `SendAll`, and the count of comments in the agent's text is checked as four.

10. **I do:** Edit a paragraph of `PROPOSAL.md` in my own editor while the agent is writing it.
    **I see:** Nothing in Humata lets me edit a Markdown file. The Files panel's "Open in editor" is a disabled button. The app shows the file as it is on disk.
    **State and events:** The file on disk changes. Comments on the changed lines are re-checked (see step 8).
    **Status:** Not built. SE-17 is "Open in editor". The Files and Diff panels draw it disabled (`ui/src/panel_files.rs:306`, `ui/src/panel_changes.rs:289`), and OVERVIEW.md lists it as a known gap (line 459).

11. **I do:** Press Copy to channel on `PROPOSAL.md` and choose `queue`.
    **I see:** A confirmation that the document was copied to the channel's shared folder.
    **State and events:** The copy request goes to the daemon. The Desk's own copy stays.
    **Status:** Works (real daemon). `desktop/tests/live_flows/desk.rs::a_desk_session_starts_from_the_page_gets_a_repo_and_its_document_is_copied_to_a_channel` clicks Copy to a channel…, picks the channel, and sees the note saying where it went. Fake-client version: `desktop/tests/native/desk.rs::a_document_is_copied_into_a_channels_shared_folder`.

12. **I do:** Nothing. The file is in the channel.
    **I see:** `shared/PROPOSAL.md` exists in the `queue` channel's folder, with the same text. An existing file of that name is never overwritten.
    **State and events:** The file is copied. A copy never overwrites a file of the same name that the channel already has.
    **Status:** Works (real daemon). `daemon/humatad/tests/desk.rs::markdown_in_the_folder_is_listed_and_copies_into_a_channels_shared_folder` and `a_copy_never_overwrites_the_channels_file`.

## Branches

### The agent edits a line I commented on

- **When:** At step 8, the agent changes a passage with a sent comment on it.
- **I do:** Look at the comment.
- **I see:** If the passage changed, the comment is addressed and shows the passage before and after. If the passage is gone, the comment is a detached draft and offers Re-attach or Drop. An untouched passage stays sent.
- **Expected (SM-7, SM-16):** A changed passage addresses the comment. A gone passage returns a sent comment to a detached draft, so it can be sent again once re-attached. Whitespace-only changes do not count.
- **Status:** Works (real daemon) for the daemon's rule: `daemon/humatad/tests/comments.rs::a_sent_comment_is_addressed_when_its_lines_change_and_a_draft_detaches`, and the live test in step 8.

### The agent removes the passage of a comment I sent, in the app's copy of the rule

- **When:** At step 8, the passage of a sent comment is gone.
- **I do:** Look at the comment in the Diff panel.
- **I see:** Per the spec, a detached draft with Re-attach and Drop.
- **Expected (SM-16, D190):** A sent comment whose passage is gone returns to a detached draft.
- **Status:** Fakes only: `review/tests/model.rs::a_sent_comment_whose_passage_is_gone_is_a_detached_draft_again` proves the app's review model makes it a detached draft again, as the daemon's copy does (`daemon/review_diff/tests/gone.rs::a_sent_comment_whose_passage_is_gone_is_a_detached_draft_again`). The rule runs inside the app's model, so no step crosses the daemon. (Was a likely bug: the app's copy set it Addressed; fixed by BUGS.)

### Someone changes the file while I have a sent comment on it

- **When:** I edit the file outside Humata, or the agent writes it while I also write it.
- **I do:** Save my change to the same passage.
- **I see:** The comment changes state as if the agent had changed the passage.
- **Expected (SM-16):** "Changes the user makes never address a comment."
- **Status:** Works (real daemon): `daemon/humatad/tests/comments.rs::a_sent_comment_stays_sent_when_the_user_writes_its_lines_through_files` writes a commented passage through Files and the comment stays sent (SM-16). Two writers are told apart by the base: `daemon/humatad/tests/files.rs::a_replace_made_from_a_stale_read_is_refused_and_writes_nothing` refuses a replace made from a stale read and writes nothing. (Was a likely bug; fixed by BUGS.)

### Re-attach or Drop a comment whose passage is gone

- **When:** After step 8, on a detached comment.
- **I do:** Press Re-attach and select a new passage, or Drop.
- **I see:** The comment is a draft on the new passage, or it is removed.
- **Expected:** A re-attached comment is a draft until sent again. A dropped comment is gone from the list and never typed.
- **Status:** Works (real daemon). The live test covers Re-attach (step 9). Drop is covered by `daemon/humatad/tests/comments.rs::the_user_drafts_edits_lists_and_drops_comments`.

### Re-attach, resolve or reopen a note

- **When:** A note (a comment with a note destination, `note-<n>` ids) is resolved or reopened, or re-attached.
- **I do:** Press Resolve, Reopen or Re-attach on the note.
- **I see:** The daemon refuses with "not built".
- **Expected:** These actions work on notes, as they do on comments.
- **Status:** Not built (no milestone). `daemon/humatad/src/review.rs:981`, `:998` and `:1007` return `not_built` for ReattachComment, ResolveComment and ReopenComment on notes. `workflow/milestones/REVIEW.md` line 60 keeps the user's ReplyToComment as not built as well (`daemon/humatad/src/review/comments.rs:344`).

### Open the file in my own editor from Humata

- **When:** I want to edit the proposal from the Files or Diff panel.
- **I do:** Press Open in editor.
- **I see:** The button is disabled.
- **Expected (SE-17):** The file opens in my own editor.
- **Status:** Not built. `ui/src/panel_files.rs:306` and `ui/src/panel_changes.rs:289`. OVERVIEW.md line 459 lists it as a known gap.

### The app restarts while comments are open

- **When:** The app or the daemon restarts between steps 5 and 9, with drafts, sent comments and an archived one.
- **I do:** Restart, then open the document.
- **I see:** Drafts, sent comments and archived comments are all there with their states. Nothing is typed into the agent again.
- **Expected:** Drafts survive restarts (PER-25). A sent comment is not typed twice. A comment waiting to send is typed once after the restart.
- **Status:** Works (real daemon). `daemon/humatad/tests/comments.rs::drafts_sent_and_archived_comments_survive_a_restart_and_nothing_is_typed_again` and `comments_wait_when_there_is_no_terminal_and_are_typed_once_when_there_is`.

### The agent tries to resolve without replying

- **When:** The agent calls `comment_resolve` with no reply, or on a comment that was never sent.
- **I do:** Nothing. This is the agent's call.
- **I see:** The call is refused.
- **Expected (SM-19):** Resolve needs a reply, and only a comment that was sent can be resolved.
- **Status:** Works (real daemon). `daemon/humatad/tests/comment_tools.rs::resolving_needs_a_reply_and_a_comment_that_was_sent`.

## Expected outcomes

- [ ] The Desk session's folder is named from its title, and the three repos are cloned into it.
- [ ] `PROPOSAL.md` opens as a tab beside the terminal, in Side by side mode.
- [ ] A comment on a source line is typed into the agent as `[comment on <path>:<line>]`, once.
- [ ] Send N comments types all drafts in one paste.
- [ ] The agent's reply shows under its comment, and comment_resolve needs a reply.
- [ ] A changed passage addresses a sent comment, and a gone passage makes it a detached draft.
- [ ] Resolved comments stay resolved after the agent rewrites the file.
- [ ] A re-attached comment is a draft until it is sent again.
- [ ] Drafts and sent comments survive a restart, and nothing is typed twice.
- [ ] Copy to channel puts the file in the channel's shared folder and never overwrites a file there.
- [ ] A user change to a passage does not address a sent comment.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Start the Desk session | Works (real daemon) | `desktop/tests/live_flows/desk.rs::a_desk_session_starts_from_the_page_gets_a_repo_and_its_document_is_copied_to_a_channel`; fakes: `features/app/tests/desk.rs::empty_home_starts_a_desk_session_and_opens_it` |
| 2. Repos cloned into the folder | Works (real daemon) | `daemon/humatad/tests/desk.rs::a_start_with_repos_clones_each_into_the_folder` |
| 3. Agent writes PROPOSAL.md | Works (real daemon) | `desktop/tests/live_flows/review.rs::a_document_is_commented_on_rewritten_and_its_comments_followed` (fake agent) |
| 4. Open as tab, Side by side | Works (real daemon) | `desktop/tests/live_flows/review.rs::a_document_is_commented_on_rewritten_and_its_comments_followed` |
| 5. Comment on a source line, sent | Works (real daemon) | `desktop/tests/live_flows/review.rs::a_document_is_commented_on_rewritten_and_its_comments_followed` |
| 6. Two drafts, Send 2 in one paste | Works (real daemon) | `desktop/tests/live_flows/review.rs::a_document_is_commented_on_rewritten_and_its_comments_followed` |
| 7. Agent replies and resolves | Works (real daemon) | `daemon/humatad/tests/comment_tools.rs::an_agent_finds_replies_to_and_resolves_a_comment_sent_to_its_session` |
| 8. Agent rewrites; addressed and detached | Works (real daemon) | `desktop/tests/live_flows/review.rs::a_document_is_commented_on_rewritten_and_its_comments_followed` |
| 9. Resolve, re-attach, send again | Works (real daemon) | `desktop/tests/live_flows/review.rs::a_document_is_commented_on_rewritten_and_its_comments_followed` |
| 10. Edit in my own editor | Not built | SE-17; `ui/src/panel_files.rs:306`; OVERVIEW.md:459 |
| 11. Copy to channel, button | Works (real daemon) | `desktop/tests/live_flows/desk.rs::a_desk_session_starts_from_the_page_gets_a_repo_and_its_document_is_copied_to_a_channel`; fakes: `desktop/tests/native/desk.rs::a_document_is_copied_into_a_channels_shared_folder` |
| 12. File lands in shared folder | Works (real daemon) | `daemon/humatad/tests/desk.rs::markdown_in_the_folder_is_listed_and_copies_into_a_channels_shared_folder` |
| Branch: agent edits a commented line | Works (real daemon) | `daemon/humatad/tests/comments.rs::a_sent_comment_is_addressed_when_its_lines_change_and_a_draft_detaches` |
| Branch: passage gone, app's copy of the rule | Fakes only (fixed) | `review/tests/model.rs::a_sent_comment_whose_passage_is_gone_is_a_detached_draft_again`; `daemon/review_diff/tests/gone.rs::a_sent_comment_whose_passage_is_gone_is_a_detached_draft_again` |
| Branch: user or concurrent change | Works (real daemon) | `daemon/humatad/tests/comments.rs::a_sent_comment_stays_sent_when_the_user_writes_its_lines_through_files`; `daemon/humatad/tests/files.rs::a_replace_made_from_a_stale_read_is_refused_and_writes_nothing` |
| Branch: re-attach or drop | Works (real daemon) | `daemon/humatad/tests/comments.rs::the_user_drafts_edits_lists_and_drops_comments` |
| Branch: resolve, reopen or re-attach a note | Not built | `daemon/humatad/src/review.rs:981, 998, 1007`; REVIEW.md:60 |
| Branch: Open in editor | Not built | SE-17; `ui/src/panel_changes.rs:289`; OVERVIEW.md:459 |
| Branch: restart with comments open | Works (real daemon) | `daemon/humatad/tests/comments.rs::drafts_sent_and_archived_comments_survive_a_restart_and_nothing_is_typed_again` |
| Branch: agent resolves without a reply | Works (real daemon) | `daemon/humatad/tests/comment_tools.rs::resolving_needs_a_reply_and_a_comment_that_was_sent` |

## Gaps and likely bugs

- **Not built:** editing a Markdown file in Humata, and "Open in editor" (SE-17). The brief's step 10 (I edit a paragraph myself while the agent writes) can only happen in my own editor, outside Humata.
- **Not built:** Re-attach, Resolve and Reopen for notes (`daemon/humatad/src/review.rs:981, 998, 1007`). No milestone; REVIEW.md keeps the user's ReplyToComment not built too.
- **Fixed (was a likely bug):** a sent comment whose passage is gone is a detached draft again in the app's model too (`review/tests/model.rs::a_sent_comment_whose_passage_is_gone_is_a_detached_draft_again`).
- **Fixed (was a likely bug):** a change the user makes leaves a sent comment sent (`daemon/humatad/tests/comments.rs::a_sent_comment_stays_sent_when_the_user_writes_its_lines_through_files`).
- **Fixed (was a likely bug):** a write made from a stale read is refused and writes nothing (`daemon/humatad/tests/files.rs::a_replace_made_from_a_stale_read_is_refused_and_writes_nothing`).

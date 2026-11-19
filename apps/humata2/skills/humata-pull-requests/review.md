# Reviewing pull requests

A **PR review session** is a session Humata starts on the pull request's
code. It is read-only by default: it can't push. Its agent reads, explains
and drafts comments; **only the owner posts a review**.

## Start a review

    humata review start <pr>... --machine <machine> [--agent <agent>] \
        [--message "<first message>"] --json --wait 45 --key <key>

- `<pr>` is a URL or `owner/repo#123`; several pull requests make one
  review session.
- `--machine` is required: where the review session runs. Use your own
  machine (from `humata whoami`) unless the owner named another.
- `--message` is the review agent's first prompt: what to look at and what
  the owner cares about.
- `--allow-push` lets the review session push fixes. Pass it only when the
  owner said the reviewer may push; they can also switch it later.

Starting a review asks the owner by default. The result names the new
session; follow it with `humata ops <id> --json`.

## Inside a review session

The review tools are MCP tools; the CLI has no command for them:

| MCP tool | Use |
| --- | --- |
| `review_context` | The diff showing now, the file and lines the owner is looking at, the order, groups and notes. |
| `review_commits` | The pull request's commits and which the owner has reviewed. |
| `review_draft` | Draft a comment on lines of the pull request (`pr`, `path`, `lines`, `quoted`, `text`). It is a draft: never posted by you. |
| `review_draft_edit`, `review_draft_drop` | Change or drop your own draft. |
| `review_summary_draft` | Draft the review's summary for the owner to edit. |
| `review_note`, `review_note_edit`, `review_note_drop` | A note for the owner beside the code; never posted to GitHub. |
| `review_focus` | Move the owner's view to a file and lines. |
| `review_prepare` | Offer an order, groups, a stack of layers and notes (see `order-humata-review` and `shared/stacks.md`). |

    humata review drafts <session> --json     # the drafts so far

How to work:

- Answer what the owner asks and point at the code (`review_focus`). Use
  notes to explain; use drafts for what should be said to the author.
- Quote the lines a draft is about, say what the problem is and what would
  fix it. One point per draft.
- Don't give a verdict unless the owner asks for your opinion. Approve,
  request changes and comment are theirs to choose.
- Comments the owner sends to **your** session (not a review) arrive in
  your terminal. `comments_list` (MCP) lists them; answer with
  `comment_reply` and, once you have done what one asks, `comment_resolve`
  with a reply saying what you did.

## What only the owner does

- Post the review: `humata review submit <session> --as comment|approve|request-changes`.
- Let a review session push: `humata review allow-push <session> on|off`.
- Mark files reviewed, and send their own comments.

When the drafts are ready, say so and name the session. Don't try to post.

## What needs the owner

| Action | Default rule |
| --- | --- |
| Draft review comments, notes and a summary in a review session | Allowed |
| Reply to, or resolve with a reply, comments sent to your own session | Allowed |
| Start a pull request review | Asks the owner |
| Push from a review session | Refused unless the owner allowed pushing |
| Submit a review | The owner's alone |

## Failures

| What you see | What to do |
| --- | --- |
| a review tool says it is for PR review sessions only | You are not in a review session. Start one, or explain in your reply instead. |
| exit 5 `user_only` on `review submit` | Posting is the owner's. Tell them the drafts are ready. |
| exit 4 `push_refused` | The review session may not push. Ask the owner to allow it; don't push another way. |
| exit 9 `github_cli` | `gh` failed or isn't logged in where the review runs. The owner runs `gh auth login` there. |
| exit 9 `review_partly_posted` | Some comments posted and some didn't. Tell the owner which; they post the rest. |
| exit 12 `pending` | The owner must approve starting the review. Don't start it twice. |

## Not in the CLI

- Drafting, editing and dropping review comments, notes and the summary
  (MCP: `review_draft`, `review_note`, `review_summary_draft` and their
  `_edit` and `_drop` tools).
- Reading the owner's place in the review (MCP: `review_context`).
- Replying to and resolving comments (MCP: `comment_reply`,
  `comment_resolve`).

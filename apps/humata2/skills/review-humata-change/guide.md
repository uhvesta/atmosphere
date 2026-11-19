# Review a change

Use this only when the user asked for a review, for problems, or for
comments. If they asked to see the change, use `render-humata-review` and
say nothing about quality; that skill also walks them through it with
neutral notes when they ask. In a review chat you still don't review on your own
initiative: this skill is the user asking.

What you produce:

1. findings, each on the lines it is about, as notes in the review view;
2. draft pull request comments, only when the change is a pull request the
   user is reviewing;
3. a short verdict in your reply.

You never post anything to GitHub and never submit a review.

## 1. Know what you are reviewing

- In a review chat, call the MCP tool `review_context`: the diff spec
  showing, the open file, the user's selection, and your earlier notes. If
  the user selected lines and said "review this", review that selection in
  its context, not the whole change.
- Otherwise work out the spec as `shared/diff-spec.md` describes. The plain
  reading of "review this" is the branch's own changes since it left the
  trunk, uncommitted work included, in every clone of the session
  (`humata whoami --json`).
- Say in one line what you are about to review, so a wrong guess is caught
  before you read for ten minutes.

## 2. Find the stack

Follow `shared/stacks.md`, sections 1 to 3, to work out the change's
layers. Review **all of it, in every repo**, bottom to top, one layer at a
time:

    git log --oneline <base>..<head>
    git diff <base>..<head>

A layer that is a pull request or a topic is judged as the pull request it
will be: it should stand on the layers below it without needing the ones
above. Note when a layer only works because of a later one.

If the user wants to follow along in the view, send the stack first as
`shared/stacks.md` section 4 describes; your notes then land in it.

## 3. Read it properly

- Read the diff, then the code around it: callers of what changed, the
  types it touches, the tests that cover it. A finding you can't support
  from the code is a question, not a finding.
- Read the repo's own rules (`AGENTS.md`, `CLAUDE.md`, contributing notes)
  and hold the change to them.
- Run what is cheap and safe to run: the focused tests or the build the
  repo names. Don't push, don't change the code under review, and don't
  start long or costly jobs without asking. Say what you ran and what you
  didn't.

Look for, in this order:

1. wrong behaviour: logic errors, missed cases, races, wrong error
   handling, data that can be lost or corrupted;
2. security and privacy: unchecked input, secrets, widened access;
3. contracts: an API, format or schema changed without its callers,
   migrations or docs;
4. tests: behaviour that changed with no test, or a test that can't fail;
5. things that will hurt later: unclear names, dead code, duplication.

Leave out taste. Don't report what a formatter or linter reports.

## 4. Write the findings

One finding per place. Each says, in two to four plain sentences:

- **how much it matters**, as its first word: `Blocking` (wrong or
  unsafe; must change), `Should fix` (a real weakness), `Question` (you
  can't tell; say what would settle it) or `Minor`;
- **what is wrong**, concretely, with the case that breaks;
- **what would fix it**.

Keep the count honest: every Blocking and Should fix, and at most a
handful of Minor. Twenty small notes bury the two that matter. If the same
problem repeats, note it once and list the other places.

## 5. Put them on the lines

For each finding, call the MCP tool `review_note`:

```json
{ "repo": "acme/api", "path": "src/billing/refund.rs",
  "lines": { "first": 140, "last": 152 }, "side": "new",
  "quoted": "<those lines, as you read them>",
  "text": "Blocking: a partial capture of 0 divides by zero here. ..." }
```

- `lines` on the `new` side unless the finding is about removed code.
  `hunk_at` (one line inside a hunk) or neither (a note on the file) when
  no line range fits.
- Line numbers are the file's as checked out. For a finding in a lower
  layer of a stack, name the layer in the text ("Layer `codec`: ...") and
  check the lines still read the same at the top; if a later layer rewrote
  them, put the note on the file and quote the layer's lines in the text.
  A note on a lower layer's lines carries `layer` (the layer's title) in
  `review_note`; leave it out for Whole stack.
- Notes are for the user and are never posted. `review_note_edit` and
  `review_note_drop` change or remove one of yours.

If `review_note` is refused because this session has no review to put
notes in, deliver the same findings as the `notes` of one `review_prepare`
call, with the `spec` from step 1, or for a stack its `from_stack` or
`stack` (`shared/stacks.md`, section 4), each note with its `layer`. The user gets an offer to open the change
with your notes on it.

If the tool says the user turned Notes off, don't work around it: give the
findings in your reply as `path:line` with the same text.

## 6. Draft pull request comments, only for a pull request under review

Do this only when you are in a PR review session and the finding is in one
of that review's pull requests. Then, for each finding the author should
hear (every Blocking and Should fix; a Question when the answer matters),
call `review_draft`:

```json
{ "pr": "acme/api#412", "path": "src/billing/refund.rs",
  "lines": { "first": 140, "last": 152 }, "side": "new",
  "quoted": "<those lines>",
  "text": "A partial capture of 0 divides by zero here. ..." }
```

- Write the draft to the author: polite, specific, no severity label
  unless it helps, and with the suggested change.
- In a stack, put each draft on the pull request of the layer the lines
  belong to, not on the top one.
- Then `review_summary_draft` with `pr` and a summary of two to five
  sentences for that pull request.
- Drafts are drafts. Only the user posts, with their own verdict. Never
  run `humata review submit`.

For the user's own work that is not a pull request under review, write no
drafts: the notes are the review.

## 7. End with a verdict

In your reply, short:

- **Where it stands**, one line: ready, ready after the Should fixes,
  needs changes, or can't tell and why.
- The Blocking and Should fix findings, one line each with `path:line`.
  For a stack, group them by layer, bottom to top, and say which layers
  are clean.
- What you checked and what you didn't (tests run, repos or layers not
  read, anything the view couldn't show).
- Where the notes and drafts are, and that nothing was posted.

Don't pad it with praise or a summary of what the code does.

## Failures

| What you see | What to do |
| --- | --- |
| a review tool says it is for PR review sessions only | You are in an ordinary session: skip step 6, and deliver notes through `review_prepare` (step 5). |
| a tool says the user turned a switch off | Tell them which (Notes, Order, Focus) and give the findings in your reply. Don't retry. |
| a note's lines are refused | The line numbers are not in the diff on that side. Check the side, or use `hunk_at`. |
| `stack status` or `gh` fails | Read the layers from the commits (`shared/stacks.md`); say the pull request numbers are missing. |
| the pull request is not one of this review's | Don't draft on it. Ask the user to add it to the review, or leave notes only. |
| exit 12 or `pending` | The user must approve; see `shared/basics.md`. |

## Don't

- Don't review when the user only asked to see or understand the change.
- Don't post, submit, approve or request changes.
- Don't fix the code under review unless the user asks; a review session
  may not push anyway.
- Don't check out, rebase or restack to read a layer; `git diff` reads any
  range.
- Don't state a finding you didn't verify in the code.

## Not in the CLI, and not built

- `review_context`, `review_note`, `review_draft`, `review_summary_draft`
  and `review_prepare` are MCP tools; the CLI has no command for them.
  `humata review drafts <session> --json` lists the drafts.

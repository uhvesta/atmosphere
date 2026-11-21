# Prepare a change for the user to look at

The user wants to **look at code**. You prepare the change so they can
find their way through it: you work out its layers, across every repo it
touches, and send them to the review view. They step through the stack;
you stay out of the way.

You don't judge the change. If they want problems found, that is the
`review-humata-change` skill; don't drift into it. Explain how it works
or put notes on lines only when they asked to be walked through it or for
notes: then `with-notes.md` adds a reading order and neutral notes to what
you send in step 3 or 4.

## 1. Find out what they want to see

In a review chat, call the MCP tool `review_context`: it gives the diff
spec showing now, the open file and the user's selection. Otherwise, in
each clone (`humata whoami --json` lists them):

- `git status --short` for staged, unstaged and untracked files;
- `git rev-parse --abbrev-ref HEAD` and `git rev-parse --abbrev-ref @{upstream}`
  for the branch and what it tracks;
- `git log --oneline -5` when the user names a commit loosely ("my last
  commit", "before the refactor").

Two kinds of ask:

- **The change as a whole** ("show me what changed", "show me this PR",
  "show the stack", "let me look at this branch"): prepare it as a stack,
  steps 2 and 3.
- **Something narrow they named** ("just my unstaged changes", "since
  commit abc", "since I last looked", "without the lockfile"): one diff
  spec, no layers. Go to step 4.

If their words could mean two different diffs and the difference matters,
ask one short question. Otherwise pick the plain reading and say which.

## 2. Work out the layers

Follow `shared/stacks.md`, sections 1 to 3, for every clone that has
changes. It picks layers in this order:

1. a stack that already exists: `stack` topics, or chained pull requests;
2. otherwise one layer per commit;
3. otherwise a logical split of the uncommitted work, and you say that you
   chose it.

The result is one ordered list, bottom to top. Each layer has a title, the
repos it touches with a range in each, its pull request if any, and at
most a one-line purpose that names the step without judging it.

## 3. Send the stack to the view

Follow `shared/stacks.md`, section 4: one `review_prepare` call with
`from_stack: {repo}` when the repo has a `tools/stack` stack (send `tip`
if it forks), otherwise `stack.layers` with a range per repo, or a worktree
layer for uncommitted work on top. Add `importance` only for the key and
skim files you know of (paths and a tier). No `groups`, no `notes`, unless
the user asked for notes (`with-notes.md`).

The owner gets the layers as a list at the top of the file list: Whole
stack first, then each layer. Stepping through each layer's own diff is
being built (STACKVIEW); `shared/stacks.md` says what the view shows.

## 4. A narrow ask: one diff spec

`shared/diff-spec.md` has every field and worked examples. Call
`review_prepare` with a short `title` and the `spec`, and nothing else: no
`order`, no `groups`, no `notes`, unless the user asked for notes
(`with-notes.md`).

## 5. When the tool answers

- In an open review chat it applies at once (the answer carries an `undo_id`
  and the user can Undo it). Elsewhere — or in a PR review before the review
  view has reported itself — `review_prepare` creates a draft view (`id` with
  no `undo_id`); call `view_mark_ready` with that `id` so the user gets an
  offer to open it.
- If the tool says the user turned a switch off (Order, for the layers),
  tell them which one and that it is in the review's Info panel. Send the
  `spec` alone and put the layer list in your reply. Don't retry.
- If the spec is refused (a base that doesn't exist, a repo the session
  doesn't have), fix it from the error; don't guess a second base.
- If a file in `order`, an `importance` entry or a group's `first` is
  refused, it isn't in the diff: check the path and the repo.
- If `from_stack` is refused, the error says why (a fork needs `tip`, a
  commit needs a `Topic:` trailer, a restack must finish, the stack tool
  is missing). Fix that, or send explicit layers; don't guess them.

## 6. Say what you sent, and stop

A few lines, no more:

- what is compared, in which repos, and what is left out ("committed work
  only; 2 uncommitted files aren't included");
- the layer list, bottom to top, each with its pull request, repos and
  file count (`shared/stacks.md` has the form), and how you picked the
  layers (an existing stack, per commit, or your own split);
- for a stack, one plain line on the view: "You can pick Whole stack or
  any layer in the file list. Stepping through each layer's own diff is
  being built."

Then stop. Don't summarise what the code does, don't point out problems,
and don't offer opinions, unless the user asks.

## Don't

- Don't add notes on files or lines unless the user asked for them.
- Don't say whether the change is good, safe or ready, in a purpose line
  or anywhere else.
- Don't check out, rebase, restack or commit anything to make a layer.
- Don't post or draft anything.

## Not in the CLI, and not built

- `review_prepare` and `review_context` are MCP tools; the CLI has no
  command for them. `humata diff <session> --json` prints a session's
  changes as data but opens nothing.
- The stepped stack view (`shared/stacks.md`, "Not built yet"): one
  layer's own diff, and stepping to a layer with `review_focus`.

# Stacked pull requests with `stack`

`stack` is Humata's own tool. It turns the commits on your branch into a
chain of GitHub pull requests: each commit names its pull request with a
`Topic:` trailer, and a `Relative:` trailer says which topic it builds on.
It is installed beside `humata`, so it is on your `PATH` in a session.

`stack` carries its own manual. Read it before you tag a commit; it is the
reference, and it matches the version installed:

    stack guide              # the model, the trailers, titles and descriptions, the loop, restacking, exit codes
    stack guide --template   # the pull request template in use, with its required sections
    stack --help             # every command
    stack <command> --help   # one command's flags
    stack commands --json    # the same, as data
    stack config show        # each setting and where it came from

## The loop, in short

    stack check              # local checks only; fix every error it reports
    stack upload --dry-run   # what would be pushed and opened
    stack upload             # push the topic branches, open or update the pull requests
    stack status             # the stack as a tree; each should say `up to date`

`stack` also matches `gh stack` command for command (`init`, `add`, `view`,
`up`, `down`, `top`, `bottom`, `trunk`, `submit`, `push`, `sync`, `rebase`,
`link`). A stack created by `gh stack` or `stack` is understood by both.

## Importing a remote stack & interleaving local commits

To copy or import a remote stack locally (by PR URL/number, a `gh stack` stack,
or a chain of PRs whose bases point at each other):

    stack import <pr-url-or-number>        # fetches branches, records topics/relatives/PRs locally
    stack status                           # inspects the imported stack layer by layer
    stack diff <topic>                     # diffs an imported or pinned layer
    stack import <pr-url-or-number>        # re-importing updates the local stack in place

You can interleave local commits between, on top of, or below imported layers
using `Topic:` and `Relative:` trailers (or `stack add` / `stack restack`).
When you `stack upload`, `stack submit`, `stack push`, or `stack sync`:

- Your own local topics are pushed and their PRs are opened with the right bases.
- Imported PRs' bases on GitHub are updated when a local layer is inserted below them.
- Foreign branches (other authors' branches) are **never** force-pushed unless
  you explicitly pass `--force-foreign`.

## Native GitHub stack mode vs comment fallback

The model has two levels: a PR is a stack of commits (one topic), and a stacked
PR is a stack of those PRs.

- **Native mode (default when `gh stack` is installed):** `stack upload` /
  `stack submit` links the chain as a native GitHub stack and writes **zero**
  comments (existing comments are left alone, never deleted).
- **Fallback mode (no `gh stack` extension, repo refusal, or `--no-native-stack`):**
  `stack` maintains one history comment per PR with a revision table and a
  **Stack Update Summary** table containing clickable compare/diff links for
  every repushed layer.

To change an earlier topic after review:

    stack restack edit --topic <topic>     # stop at that topic's last commit
    git commit --amend                     # or add commits with the same Topic trailer
    stack restack continue                 # replay the later topics onto it
    stack upload --note "addressed review: <what changed>"
    stack diff <topic>                     # what changed since the last upload

Every `stack` error ends with a `Next:` line: the command to run next.

## Rules

- Run `stack check` before `stack upload`.
- Never push the `<prefix>/stack/<topic>` branches or `refs/stack/` pins by
  hand, and never use a hand-run `git rebase -i` where `stack restack` does
  the job.
- Never edit a pull request description on GitHub. Change the commit body
  and upload.
- Never close or delete a pull request unless the owner asks. `stack status`
  lists pull requests no local topic uses: ask before closing any.
- A conflict (exit 3) or a failed `--exec` (exit 5) stops a restack where
  it is. Resolve it and `stack restack continue`, or `stack restack abort`
  to put the branch back. Don't leave one half done: `stack status` shows
  it.

## What needs the owner

`stack upload` pushes with the GitHub CLI's login on the machine, outside
Humata's push rule. Treat it as a push all the same: upload only when the
owner asked for pull requests, or ask first (the `humata-inbox` skill).
Closing pull requests, renaming a topic whose pull request was closed, and
`stack upload --overwrite-body` over an edit someone made on GitHub are the
owner's calls.

## Failures

| What you see | What to do |
| --- | --- |
| `stack` is not found | It isn't installed beside `humata` on this machine. Tell the owner; don't build pull requests by hand in its place unless they say so. |
| exit 1 | git or `gh` couldn't run. `gh auth status` must show a login on this machine; the owner logs in. |
| exit 2 | A trailer, plan or template problem. The message names the commit. Reword it and run `stack check` again. |
| exit 3 | A commit doesn't apply where its topic puts it. Follow the `Next:` line. |
| exit 4 | GitHub refused, or a pull request is closed. For a closed one, ask the owner: reopen it or rename the Topic. |
| exit 5 | The restack's `--exec` command failed. Fix that topic, then `stack restack continue`. |

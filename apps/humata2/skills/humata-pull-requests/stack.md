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

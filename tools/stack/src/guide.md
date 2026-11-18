# Writing commits that become pull requests

`stack` turns the commits on your branch into a stack of GitHub pull requests.
Run it as `bazel run //tools/stack -- <command>`. Read this before you tag
commits or write their messages.

## The model

- One pull request per topic. A topic is a name you give commits with a
  `Topic:` trailer. All commits with the same topic make one pull request.
- Each topic is pushed to its own branch: `<prefix>/stack/<topic>`. The prefix
  is `--prefix`, else `prefix` in stack.toml, else your GitHub login. A topic
  whose pull request is open keeps the branch it was opened on, even after the
  prefix changes. Branch names never say where a topic sits in the chain, so
  reordering topics never renames a branch or closes a pull request.
- A topic can name a `Relative:` topic. Its pull request then targets that
  topic's branch, so the pull requests form a chain (or a tree). A topic with
  no `Relative:` targets the trunk: the repository's default branch, or
  `--trunk` (`--base` still works), or `trunk` in stack.toml.
- Commits without a `Topic:` stay local. A relative topic needs commits on this branch.

## Tagging commits

Put the trailers in the last paragraph of the message, one `Key: value` per line:

    storage: compress the journal

    ## Summary
    Journal segments are written with zstd.

    ## Why
    Journals were 40% of the disk.

    ## Testing
    bazel test //daemon/store/...

    Topic: compress-journal
    Relative: zstd-codec

The trailers:

- `Topic: <name>` (a-z, 0-9, `.`, `_`, `-`) puts the commit in a pull request.
  One Topic per commit.
- `Relative: <topic>` names the topic this one builds on. Use one value across
  the topic's commits.
- `Reviewers: <login>, <login>` and `Labels: <label>, <label>` (commas or spaces).
- `Draft: true` (or `false`, `yes`, `no`) opens the pull request as a draft.
  One `true` commit makes the whole topic a draft.

The trailers must be the last paragraph, and every line must be `Key: value`.
Several commits share a topic by repeating the same `Topic:` line. They need
not be adjacent.

## Titles and descriptions

- The title is the subject line of the topic's first commit (the oldest one).
- The description comes from the body of that same commit. The body's `## `
  headings are laid out under the sections of the PR template (run
  `stack guide --template` to see it). The template is built in; stack.toml's
  `template = "<file>"` names a file next to stack.toml instead. stack never
  reads the repository's `.github` template.
- Text before the body's first heading counts as Summary. Each other heading
  goes under the template section with the same name (names ignore case).
  Empty sections are left out.
- Sections marked `<!-- required -->` in the template must have text. Here
  Summary, Why and Testing are required; Risks is optional. A missing one fails
  `check` and `upload` (exit 2). Reword that commit (`git rebase -i`, `reword`).
- A topic with several commits also gets `## Commits`, one line per commit subject.
- When the stack has more than one pull request, the description ends with
  `## Stack`: every pull request of the tree, this one marked.
- stack ends each description with a hidden marker, so it can tell when someone
  edited the description on GitHub.

Never edit a description on GitHub. The edit is kept, later uploads stop
updating it, and `upload` warns about that pull request. Only
`upload --overwrite-body` replaces it. Change the commit body and upload instead.

## Revisions

Each upload that changes a topic's head adds one row to that pull request's
history comment, the one comment stack keeps up to date. The Diff column says
what the row compares:

- `full`: the first revision, the whole change against its base.
- `since N`: same base as revision N; the change since N.
- `rebase`: a new base, the same patch.
- `rebase + changes`: a new base and a changed patch.

`--note "text"` puts text in the row's Summary cell for every pull request the
upload changed, for example `--note "addressed review: renamed Codec"`. An
upload that changes no head adds no row.

Every upload also pins each moved head and its base on the remote, as
`refs/stack/<prefix>/<topic>/r<N>` and `r<N>-base` (N is the row's number).
Those refs are not branches, and they keep old revisions fetchable after the
force-push. `stack diff <topic>` shows what changed between the last two
revisions with `git range-diff`, so a rebase's base changes stay out; `stack
diff <topic> 2 4` compares any two. A `rebase` row names the exact command.
stack.toml's `pin_revisions = false` turns pinning off, and `keep_revisions =
N` keeps only the newest N.

## The loop

1. `stack check`: local checks only (trailers, topics, required sections, and
   whether each topic rebuilds onto its relative). Fix every error it reports.
2. `stack upload --dry-run` (optional): shows what would be pushed and opened.
3. `stack upload`: pushes the changed topic branches in one push and opens or
   updates each pull request.
4. Address review in the topic it belongs to, with a restack (below).
5. `stack upload --note "addressed review: ..."`.
6. `stack status`: the stack as a tree; each pull request should say
   `up to date`. It also lists stack pull requests no local topic uses. Ask
   before closing any of them.

## Restacking: changing an earlier topic

`stack restack` drives `git rebase -i` for you, one topic at a time:

1. `stack restack edit --topic <topic>` (or `--commit <sha>`) stops at that
   topic's last commit. Change it: `git commit --amend`, or add commits with
   the same `Topic:` trailer.
2. `stack restack continue` replays every later topic onto it. Add `--step` to
   stop after each later topic, and `continue` again to go on (without
   `--step`, it runs to the end). `--exec "<command>"` runs a command after
   each topic, such as `--exec "bazel test //tools/stack/..."`; when it fails
   the restack stops there (exit 5) so you can fix that topic.
3. On a conflict the restack stops (exit 3) and names the topic, the commit
   and the files. Resolve them, `git add` each one, then `stack restack
   continue`. `stack restack skip` drops that commit; `stack restack abort`
   puts the branch back as it was before `edit`.
4. `stack upload` once the restack is done. Upload refuses while one is in
   progress; `stack status` shows where it stopped and what to run next.

The restack's state lives in the git folder, so it survives a closed terminal:
run `stack status` and the command it names.

## Settings

Every setting has a compiled-in default. `~/.humata2/config/stack.toml` (or
`$HUMATA2_HOME/config/stack.toml`) can change them; Humata copies that folder
to each machine, so a copied `stack` binary plus the folder behave the same
anywhere. `stack config show` (or `--json`) prints each setting and where it
came from. The keys:

- `prefix`: the branch prefix (default: your GitHub login).
- `trunk`: the branch pull requests target (default: the repository's default).
- `remote`: the git remote (default `origin`).
- `template`: a PR template file in the same folder (default: built in).
- `draft`, `overwrite_body`: like `--draft` and `--overwrite-body` on every upload.
- `native_stack`: link chains as GitHub native stacks (default true).
- `history_comment`: keep the revision table comment (default true).
- `pin_revisions`, `keep_revisions`: revision pins (default on, keep all: 0).

An unknown key, a wrong type or bad TOML stops every command (exit 2) with the
file and line. Secrets never go in stack.toml: a key that looks like one is
refused. stack uses the GitHub CLI's login, so run `gh auth login` on each
machine.

## Finding things

- `stack --help` and `stack <command> --help`: every command and flag.
- `stack commands --json`: the same, as data.
- Every error ends with a `Next:` line: the command to run next.

## Rules for agents

- Never push the `<prefix>/stack/<topic>` branches or `refs/stack/` pins by
  hand. `stack upload` does that.
- Use `stack restack` rather than a hand-run `git rebase -i` to change an
  earlier topic.
- Never edit pull request descriptions on GitHub.
- Never close or delete pull requests unless the person asks.
- Run `stack check` before `stack upload`.
- If unsure what an upload will do, use `stack upload --dry-run`.

## Errors and exit codes

- 0: done.
- 1: git or gh could not run, or failed. Check that `gh auth status` shows a
  login and that the remote (`--remote`, default `origin`) exists. Then run again.
  If a temporary file could not be written, check that the temp folder is writable.
- 2: a usage, trailer, plan or template problem. The message names the commit
  and the fix. Fix the commits, then run again. Examples: a bad Topic name, two
  Topics on one commit, a bad `Draft:` value, two Relative values, a Relative
  no commit on this branch names, topics that loop, a missing required section.
  A bad stack.toml is exit 2 too.
- 3: a commit does not apply where its topic puts it (a rebuild conflict), or
  a restack stopped on a conflict. For a rebuild, reorder or fix up the
  commits, then run `stack check` again; for a restack, follow its `Next:`.
- 4: GitHub refused the request, or a pull request's state blocks the upload.
  A closed pull request stops its topic. Ask the person: reopen it on GitHub
  (keeps its review) or rename the Topic (new pull request). For any other
  refusal, read GitHub's message and fix that. Never push the branches by hand
  instead.
- 5: a restack's `--exec` command failed. Fix the topic it stopped after,
  then `stack restack continue`.

A merged topic is not an error: upload skips it with a note and retargets the
topics on it to the trunk. Rebase onto the trunk to drop its commits.

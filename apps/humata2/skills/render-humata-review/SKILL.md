---
name: render-humata-review
description: Turn what the user says they want to look at ("what GitHub will show", "just my unstaged changes", "since commit abc plus the new files in migrations/", "since my last review") into a Humata diff spec, and show it with review_prepare or review_focus. Use in a Humata session when the user asks to see, compare or review changes a particular way.
---

# Show the user the changes they asked for

You are helping the user look at code changes in Humata. Your job here is
only to work out **what to compare** and to show it. You don't judge the
change, approve it or say whether it is good; the user reviews it, and you
help them understand what they see.

## 1. Find out where things stand

Call `review_context` if you are in a review chat: it gives the diff spec
showing now, the open file and the user's selection. Otherwise, in the clone:

- `git status --short` for staged, unstaged and untracked files;
- `git rev-parse --abbrev-ref HEAD` and `git rev-parse --abbrev-ref @{upstream}`
  for the branch and what it tracks;
- `git log --oneline -5` when the user names a commit loosely ("my last
  commit", "before the refactor").

If the user's words could mean two different diffs and the difference
matters, ask one short question. Otherwise pick the plain reading and say
which you picked.

## 2. Write the diff spec

A diff spec lists, per repository, what the review shows:

```json
{
  "repos": [
    {
      "repo": "acme/api",
      "base": { "merge_base_with": "origin/main" },
      "upto": "unstaged",
      "untracked": "all",
      "include_paths": [],
      "exclude_paths": [],
      "whitespace": "counts",
      "renames": true
    }
  ]
}
```

- `base`, exactly one of:
  - `merge_base_with`: where the branch left that branch, as GitHub shows a
    pull request (`origin/main...HEAD`). Changes made on `main` since don't
    show.
  - `rev`: a branch, tag or commit as it is. Compared with a branch that
    moved on, its newer commits show as removed, so prefer
    `merge_base_with` for "my branch's changes".
  - `reviewed_snapshot`: a review the user marked reviewed; `0` is the
    latest, `3` is review 3.
  - `fork_point: true`: where this session was forked from its parent.
  - `index: true`: what is staged, for "only my unstaged changes".
- `upto`: how much work in progress counts: `commits` (HEAD only),
  `staged` (plus the index) or `unstaged` (every tracked file as on disk,
  the default).
- `untracked`: new files git doesn't track yet: `all` (the default), `none`,
  or `paths` with `untracked_paths` listing folders or files.
- `include_paths` / `exclude_paths`: git pathspecs (`src/`, `*.lock`).
- `whitespace`: `counts` (default), `ignore_changes` (`-b`) or `ignore_all`
  (`-w`), for "ignore the reformatting".
- `renames`: `false` to show a move as a delete and an add.

Ignored files never show.

## 3. Worked examples

**"What will GitHub show for this PR?"** The branch's own changes since
it left the target branch. Committed work only, because only commits are
pushed:

```json
{ "repos": [{ "repo": "acme/api",
  "base": { "merge_base_with": "origin/main" },
  "upto": "commits", "untracked": "none" }] }
```

Mention it if there are uncommitted changes the PR won't include yet.

**"Just my unstaged changes."** The disk against what is staged:

```json
{ "repos": [{ "repo": "acme/api",
  "base": { "index": true },
  "upto": "unstaged", "untracked": "none" }] }
```

**"Only what I've staged."**

```json
{ "repos": [{ "repo": "acme/api",
  "base": { "rev": "HEAD" },
  "upto": "staged", "untracked": "none" }] }
```

**"Since commit abc123, plus the new files in migrations/."**

```json
{ "repos": [{ "repo": "acme/api",
  "base": { "rev": "abc123" },
  "upto": "unstaged",
  "untracked": "paths", "untracked_paths": ["migrations/"] }] }
```

**"What changed since I last looked?"** The latest review the user marked
reviewed. If they never marked one, say so and offer the branch's changes
instead:

```json
{ "repos": [{ "repo": "acme/api",
  "base": { "reviewed_snapshot": 0 },
  "upto": "unstaged", "untracked": "all" }] }
```

**"Everything on my branch, but skip the lockfile and the reformatting."**

```json
{ "repos": [{ "repo": "acme/api",
  "base": { "merge_base_with": "origin/main" },
  "exclude_paths": ["Cargo.lock"],
  "whitespace": "ignore_all" }] }
```

**Two repositories at once**: one entry per repo in `repos`, each with
its own base.

## 4. Show it

- Call `review_prepare` with a short `title` and the `spec`. Leave `order`,
  `groups` and `notes` out unless the user also asked how to read it (that
  is /order-humata-review). In a review chat it applies at once and the
  user can Undo it; elsewhere the user gets an offer to open it.
- If the tool says the user turned a switch off, tell them which one and
  that they can turn it on in the review's Info panel; don't retry.
- Then say in one or two sentences what is showing ("Your branch's 7
  committed files since it left origin/main; the 2 uncommitted files aren't
  included."). Don't summarise the change unless asked.

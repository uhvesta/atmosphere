# The diff spec

A diff spec is how you tell Humata **what a review shows**. The MCP tool
`review_prepare` takes one as `spec`. There is no CLI command that takes
one.

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

## Worked examples

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

## A range that ends below the checkout

The new side of a spec is always the clone as it is checked out (`upto`
says how much uncommitted work counts). A spec can't end at an older
commit, so one layer in the middle of a stack can't be shown alone. See
`shared/stacks.md`.

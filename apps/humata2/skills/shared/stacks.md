# A change as a stack of layers

The owner reads a change as a **stack**: an ordered list of layers, bottom
to top, each a step of the work that builds on the ones below. A layer can
touch one repo or several. Your job is to work out the layers and hand
them over so the owner can go through the change one layer at a time, and
also see all of it at once.

A layer has:

- a **title**: a few words naming the step;
- per repo it touches, a **range**: `<base>..<head>` for committed work, or
  a set of paths for work that is not committed;
- its pull request, when it has one;
- optionally a **purpose**: one line saying what the step is for. It is
  there to find your way around ("Adds the zstd codec"), never to judge
  ("Adds the codec, but misses the error path").

## 1. Find the repos

    humata whoami --json

lists your session's clones (repo, path, branch). Check every clone that
has changes, not only the one you are in.

## 2. Pick the layers

Use the first of these that applies, per clone, then merge the clones'
layers into one list (step 3). Say which one you used.

### a. A stack that already exists

**A `stack` tool stack.** `stack` is Humata's own tool; commits name their
layer with a `Topic:` trailer and the layer below with `Relative:`.

    git log --format='%h %s%n%(trailers:key=Topic,key=Relative)' <trunk>..HEAD
    stack status --json

- The `git log` works offline and shows whether any commit has a `Topic:`.
  `<trunk>` is the repo's default branch as fetched, such as `origin/main`.
- `stack status --json` (it asks GitHub, so it needs `gh` logged in) gives
  `trunk` and `topics`, each with `topic`, `parent`, `depth`, `branch`,
  `base`, the pull request `number`, `url` and `state`, and `needs_upload`.
  A topic with no `parent` sits on the trunk. Order layers by `depth`.
- If `stack status` can't reach GitHub, `stack check` prints the planned
  stack from the commits alone.
- A restack in progress shows in `restack`. Say so: the layers are moving.

**Chained pull requests.** A pull request whose base branch is another
open pull request's head branch sits on top of it.

    gh pr list --author @me --state open --json number,title,headRefName,baseRefName,url

Follow `baseRefName` → `headRefName` from the current branch down to the
trunk. In a PR review session, the review's own pull requests are the
layers when they chain this way.

One layer per topic or pull request. Its title is the topic's name or the
pull request's title.

### b. Otherwise, one layer per commit

    git log --reverse --format='%h %s' <trunk>..HEAD

Each commit is a layer: its range is `<commit>^..<commit>`, its title the
commit's subject. Uncommitted work, if any, is one more layer on top,
titled "Not committed yet" (or split as in c when it is large).

A single commit with nothing else is a stack of one layer: just show it.

### c. Otherwise, a logical split of work that is not committed

When nothing is committed in layers (everything is in the working tree, or
in one large commit the owner asks you to break down), group the changed
files into steps yourself:

    git status --short
    git diff --stat

- Group by what belongs together: a schema and its migration; the code
  that uses it; the callers; the tests; generated files and lockfiles
  last.
- Order the groups so that each one makes sense after the ones before it.
- Three to seven layers. A file belongs to one layer.
- **Say that you chose this split**, and that it is a way to read the
  change, not how it is committed.

## 3. One list, across repos

- Layers with the same topic, branch name or (for a split) the same step
  in two repos are **one layer** that spans both.
- Order bottom to top: what sits on the trunk first.
- A stack that forks (two layers on the same parent) is a tree. List each
  branch of it bottom to top and say where they split.

For each layer and repo:

    git log --oneline <base>..<head>        # its commits
    git diff --stat <base>..<head>          # its files

The layer list you give the owner looks like this:

    1. codec        acme/api #412, acme/web #88     6 files
    2. journal      acme/api #413                    4 files
    3. settings-ui  acme/web #89  (needs upload)     9 files

## 4. Send it to the view: what works today

The review view has no layers of its own yet (see "Not built"). The
closest thing it has is **groups**: titled sections over the files, in an
order you give. So today you send one `review_prepare` (an MCP tool) with:

- `spec`: per repo, the whole stack as one diff. The base is the merge
  base with the trunk; the new side is the checkout
  (`shared/diff-spec.md`).
- `order`: every changed file, layer by layer from the bottom, each file
  under the **lowest layer that changes it**.
- `groups`: one per layer, in order: `title` is "1. codec" (the number,
  then the layer's title), `note` is the layer's purpose or its pull
  request, and `first` is the layer's first file in the order.
- no `notes`.

```json
{ "title": "Journal compression: 3 layers, 2 repos",
  "spec": { "repos": [
    { "repo": "acme/api", "base": { "merge_base_with": "origin/main" }, "upto": "commits", "untracked": "none" },
    { "repo": "acme/web", "base": { "merge_base_with": "origin/main" }, "upto": "commits", "untracked": "none" } ] },
  "order": [
    { "repo": "acme/api", "path": "src/codec/zstd.rs" },
    { "repo": "acme/web", "path": "src/api/codec.ts" },
    { "repo": "acme/api", "path": "src/journal/segment.rs" },
    { "repo": "acme/web", "path": "src/settings/Compression.tsx" } ],
  "groups": [
    { "title": "1. codec", "note": "Adds the zstd codec. acme/api #412, acme/web #88",
      "first": { "repo": "acme/api", "path": "src/codec/zstd.rs" } },
    { "title": "2. journal", "note": "Journal segments use it. acme/api #413",
      "first": { "repo": "acme/api", "path": "src/journal/segment.rs" } },
    { "title": "3. settings-ui", "note": "A setting to turn it on. acme/web #89",
      "first": { "repo": "acme/web", "path": "src/settings/Compression.tsx" } } ] }
```

Use `"upto": "unstaged"` when the stack includes work that is not
committed (always, for a split as in 2c).

What this does and doesn't give the owner, which you tell them in a line:

- They see the whole stack, in every repo, and can go through it layer by
  layer as sections.
- A file that more than one layer changes appears once, under its lowest
  layer, showing all its changes together. Name those files.
- The view can't yet show one layer's own diff, or step between layers as
  separate diffs. That stepped view is being built.

To show the **top layer alone** today: a spec with
`"base": { "rev": "<head of the layer below>" }` per repo. Any other
single layer can't be shown alone; `git diff <base>..<head>` reads it.

If the clone is not checked out at the top of the stack, the view ends
where the checkout is. Say which layers are above it and not shown. Never
check out another commit, restack or rebase to show a layer.

## Not built

The stepped stack view (milestone STACKVIEW). The payload it will take is
the list this file has you work out:

```json
{ "stack": { "layers": [
  { "title": "codec", "purpose": "Adds the zstd codec",
    "repos": [
      { "repo": "acme/api", "base": "<rev>", "head": "<rev>", "pull_request": "acme/api#412" },
      { "repo": "acme/web", "base": "<rev>", "head": "<rev>", "pull_request": "acme/web#88" } ] },
  { "title": "Not committed yet",
    "repos": [ { "repo": "acme/api", "worktree_paths": ["src/journal/"] } ] } ] } }
```

With it the view lists the layers, shows the whole stack or one layer's
own diff across its repos, and `review_focus` steps to a layer. Until then
`review_prepare` takes no `stack`: send groups as above.

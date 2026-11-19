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

The daemon reads this same metadata itself when you send `from_stack`
(section 4a), so you don't need to list the topics for the view.

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

## 4. Send it to the view

Send one `review_prepare` (an MCP tool) per review. Use the cheapest form
that fits:

- **a. `from_stack`**: the repo has a `tools/stack` stack. Send only its
  repo. Use this whenever section 2a applies.
- **b. `stack.layers`**: spell the layers out. Use this for a cross-repo
  stack, chained pull requests, one layer per commit, a split of
  uncommitted work, or uncommitted work on top of a stack.

Send `spec` only for a narrow ask (`render-humata-review`, step 4). Never
send `spec` with `stack` or `from_stack`.

### a. A `tools/stack` stack: send the repo

    { "title": "Journal compression",
      "from_stack": { "repo": "acme/api" } }

The daemon reads the stack in your checkout of that repo (`stack status
--json --offline`, no network and no GitHub login). It makes one layer per
topic, bottom first, titled with the topic's first commit's subject. The
topic's name shows beside the title. You name nothing else.

- **Forks.** When the stack forks, the daemon refuses and names the leaf
  topics. Add `tip` with the topic you mean; the layers are then the chain
  from the root to that tip. Send one review per branch.
- **Titles and purposes.** Add `topics` only to give a layer a better title
  or a purpose:

      { "from_stack": { "repo": "acme/api",
          "topics": [ { "topic": "codec", "title": "zstd codec",
                        "purpose": "Adds the zstd codec" } ] } }

  An unknown topic name is refused, and the error lists the real ones.
- **Commits need a `Topic:` trailer.** If no commit between trunk and HEAD
  has one, the daemon refuses. Send explicit layers instead (section 4b).
- **A restack in progress** is refused: finish or abort it, then send again.
- **A missing or old stack tool** is refused. Don't guess the layers from
  git in its place; tell the owner that the tool needs upgrading.
- **Refresh is automatic.** When the stack moves (a rebase, a new topic), the
  daemon re-reads it and updates the review. Don't send it again for that.

### b. Explicit `stack.layers`

    { "title": "Journal compression: 3 layers, 2 repos",
      "stack": { "layers": [
        { "title": "codec", "purpose": "Adds the zstd codec",
          "repos": [
            { "repo": "acme/api", "range": { "base": "origin/main", "head": "9f2c1ab" },
              "pull_request": "acme/api#412" },
            { "repo": "acme/web", "range": { "base": "origin/main", "head": "51de07e" },
              "pull_request": "acme/web#88" } ] },
        { "title": "journal", "purpose": "Journal segments use it",
          "repos": [
            { "repo": "acme/api", "range": { "base": "9f2c1ab", "head": "c04e7b2" },
              "pull_request": "acme/api#413" } ] },
        { "title": "Not committed yet",
          "repos": [
            { "repo": "acme/api", "worktree": { "include_paths": ["src/settings/"] } } ] } ] } }

Rules the daemon checks, all at once:

- Layers are bottom first, at most 50. A layer's title is unique (ignoring
  case), at most 60 characters. A purpose is one line, at most 120.
- Each repo in a layer has a `range` or a `worktree`, not both.
- A `range` is `base..head`. Both must resolve, and base must be an
  ancestor of head.
- In one repo, each range layer's base is the previous range layer's head.
  That is how a chain shows. One layer per commit gives a chain for free:
  each commit's `^` is the commit below it.
- A `worktree` layer goes on top. It takes `include_paths` (git pathspecs;
  `"."` for everything) and HEAD must be the top range head. An empty
  worktree layer is accepted and shows "No changes".
- A forked stack is refused: send one review per branch.
- Send commit ids rather than branch names. A branch that moves makes the
  layer drift.

**Mixing.** Explicit layers may follow a `from_stack`: other repos, or
uncommitted work on top. The same checks run over the whole list. An explicit
range layer in the `from_stack` repo is refused: make that step a topic, or
send that repo as explicit layers.

**Drift.** When a kept commit moves so the chain breaks, or HEAD leaves the
top, the view shows that layer's repo as "Moved since it was sent". Send the
stack again. A wrong diff is never shown.

## 5. Key and skim files

Mark only the files that matter to the reading, and say nothing else. Paths
and a tier, no prose:

    "importance": [
      { "repo": "acme/api", "path": "src/codec/zstd.rs", "tier": "key" },
      { "repo": "acme/web", "path": "src/api/codec.generated.ts", "tier": "skim" } ]

- `key`: read first. It gets a dot in the list and keeps its place.
- `skim`: drawn last in its repo as one folded row. Opening it shows the
  file dimmed.
- `normal`: un-skims a file the daemon would skim by itself.
- Lockfiles (Cargo.lock, MODULE.bazel.lock, package-lock.json, yarn.lock,
  pnpm-lock.yaml, go.sum, Podfile.lock and similar), files that
  `.gitattributes` marks `linguist-generated`, and renames with no content
  change are skimmed already. **Don't list them.**
- Each file once, and only paths that are in the diff. A file listed twice
  is refused.
- Put `importance` at the top level for the whole review. A layer can carry
  its own `importance` for that layer's view. Whole stack uses the top-level
  list; with none sent, a file is key if any layer marks it key, and skim
  only if every layer that touches it marks it skim.
- `order` is optional. Send it only when you want a reading order; the rest
  follow. A key file is not moved when you send an order.

## Not built yet

- Ordering the hunks inside a file. Hunks stay in file order; point at one
  with a note (`hunk_at`).
- The stepped stack view is being built (milestone STACKVIEW): each layer's
  own diff, and `review_focus` stepping to a layer. Send the stack in the
  shape above either way, and say to the owner what the view shows today.

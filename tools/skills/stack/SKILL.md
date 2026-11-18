---
name: stack
description: Use when opening or updating GitHub pull requests from commits, stacked PRs, changing an earlier PR in a stack, or addressing review on them. Runs the repo's `stack` CLI.
---

# stack

Turns the trailer-tagged commits on your branch into a linked stack of GitHub
pull requests. This is only an index: the CLI is the source of truth, so do
not work from memory. Run it as `bazel run //tools/stack -- <command>`.

1. Read `stack guide` first: trailers, descriptions, the restack loop,
   settings, revisions and exit codes.
2. `stack commands --json` lists every command and flag; `stack <command>
   --help` explains one.
3. Every error ends with `Next:`, the command to run next. Follow it.

The everyday commands: `check`, `upload` (`--dry-run`, `--note`), `status`,
`restack edit|continue|skip|abort`, `diff <topic>`, `config show`.

Rules:

- Never push the `<prefix>/stack/<topic>` branches or `refs/stack/` pins by hand.
- Never edit PR descriptions on GitHub; never close or delete PRs unless asked.
- Run `check` before `upload`. Use `upload --dry-run` when unsure.
- Never put a token in `stack.toml`; stack uses `gh auth login`.

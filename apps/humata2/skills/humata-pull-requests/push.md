# Pushing and opening pull requests

Push through Humata, not with `git push` or `gh pr create`. Humata's push
goes through the owner's rule, shows them the repos, branch, commits and
pull request on the request card, and records the pull request on the
session.

## See what would go

    humata diff <session> --json                          # the session's changes against main
    humata diff <session> --repo <owner/name> --against uncommitted --json
    humata session push <session> --dry-run               # the push plan: repos, branches, commits

`--against` is `main` (the default), `uncommitted`, `last-reviewed`, `fork`
(since the fork point), `pr` (the pull request's base) or another session's
name. `<session>` is yours, from `humata whoami`.

Before you push: the tests the repo asks for have run, the commits say
what and why, and nothing unrelated or secret is in the diff.

## Push

    humata session push <session> [--repo <owner/name>]... [--pr] \
        [--message "<commit message>"] --json --wait 45 --key <key> --yes

- Without `--repo` every repo in the push plan goes. Repeat `--repo` to
  pick.
- `--pr` opens a pull request for each repo pushed.
- `--message` commits the uncommitted files first with that message.
  Without it, uncommitted files stay local. Commit yourself when the work
  needs more than one commit.
- `--yes` skips the CLI's own "Continue?" after it prints the plan. It
  never skips the owner's approval. Or run with `--dry-run`, read the plan
  and its token, then run again with `--plan <token>`.
- `--from-view <view>` pushes with the title, body and base of a pull
  request view you prepared and the owner opened (see below).

Pushing asks the owner by default. The command exits 12 with a request id
until they answer; approval runs the push. Don't push another way while it
waits. Afterwards `humata ops <id> --json` shows each step, and the result
names the pull requests.

## Title, description and base branch

The CLI's push takes no title, body or base branch. Three ways to set them:

- Write commits whose subject and body are what the pull request should
  say, and let the owner edit it in the app.
- Use the MCP tool `session_push` (`repos`, `open_pr`, `title`, `body`).
- Prepare a pull request view the owner can read and edit before anything
  is pushed: MCP `view_prepare` with `pull_request` content (per repo:
  `title`, `body`, `base_branch`, `draft`), then `view_mark_ready`. Nothing
  is pushed by preparing. `humata views --json` lists your views and their
  state.

For several dependent pull requests from one branch, use `stack`
(`stack.md`).

## What needs the owner

| Action | Default rule |
| --- | --- |
| Read your session's diff | Allowed |
| Prepare a view for the owner | Allowed |
| Push a branch or open a pull request | Asks the owner |
| Read another session's diff | Asks the owner (as reading its files) |

Only the owner: marking files reviewed (`humata diff mark-reviewed`),
opening or dismissing a view (`humata view ...`), and writing or sending
their own comments (`humata comment add|send|rm`).

## Failures

| What you see | What to do |
| --- | --- |
| exit 12 `pending` | The owner must approve the push. Don't push by hand. |
| exit 4 `push_refused` | The remote refused (a protected branch, no rights, a review session that may not push). Read the message. Never force-push to get past it. |
| exit 9 `git_failed` or `github_cli` | git or `gh` failed on the session's machine. The message says which. If `gh` is not logged in, the owner runs `gh auth login` there. |
| exit 6 `plan_stale` | The branch changed after the plan. Run `--dry-run` again and read the new plan. |
| exit 7 `unpushed_work` on an archive | Push first (`humata session archive <session> --push-first`), or ask the owner. |
| exit 8 `host_unreachable` | GitHub is reached through the owner's Mac, which is away. The push waits or fails by the away rule. |

## Not in the CLI

- A pull request title, body, base branch or draft flag on
  `session push` (MCP: `session_push`, or a prepared pull request view).
- Preparing a view (MCP: `view_prepare`, `view_revise`, `view_mark_ready`).

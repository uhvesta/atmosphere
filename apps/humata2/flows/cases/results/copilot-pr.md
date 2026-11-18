# VERIFY results: lane pr (Copilot 1.0.113, claude-haiku-5.5), PR review flows

Lane `pr` ran against real GitHub (`LANE_REMOTE=github`) on the owner's machine (lane daemon, isolated homes). Test PR: uhvesta/atmosphere#33 "VERIFY test PR (dummy, will be closed)", branch `verify/pr-flows` off origin/main, 2 files +4 -0. The PR, its branch and the local worktree were closed and removed at the end (see Cleanup).

Evidence root: `V/evidence/pr/` (V = `<scratchpad>/verify`).

## 1. An agent reviews its own diff, from its own session

- **Agent:** copilot 1.0.113, claude-haiku-5.5. **Machine:** this computer (`ma_c16c…`, lane). **Channel:** `pr-reviews` (repo uhvesta/atmosphere, clone done in about 10 s).
- **Steps:**
  1. `humata --json channel create pr-reviews --repo uhvesta/atmosphere`.
  2. `humata --json session start --channel pr-reviews --agent copilot --model claude-haiku-5.5 --title "flow1 self-review"`; op succeeded.
  3. `humata session type <s> "...create review-note.txt..."`: text landed in the prompt but was not submitted (BUG-T1 again). Enter sent with tmux.
  4. The agent asked for permission to edit. Approved with `humata inbox approve <rq>`.
  5. Agent wrote the file in the session folder, not in the clone. Asked it to move it into `atmosphere/` and to run `humata diff <s> --against uncommitted`. Approved two more requests (a shell `cd` and the diff).
  6. Checked `humata --json diff <s> --against uncommitted`: 1 added file, `review-note.txt`, +1 -0.
- **What happened:** the agent ran the diff on its own change and answered "The diff is right: it shows exactly one added file, review-note.txt (1 line added, 0 removed)". The first write went outside the git clone, so the diff showed no changes until the file was moved. The agent said the environment "isn't a git repo".
- **Result:** PASS (with findings F1, F2, F3).
- **Evidence:** `01-channel-create.json`, `02-session-start-f1.json`, `03-type-f1.json`, `04-inbox-f1.json`, `05-approve-f1.json`, `06-diff-f1.json` (empty diff, first write), `07-diff-f1-after.json`, `08-screen-f1.txt`.

## 2. An agent prepares a review for the owner, which the owner opens, questions, edits, drops and submits

- **Agent:** copilot 1.0.113, claude-haiku-5.5. **Machine:** this computer. **Session:** `review-uhvesta-atmosphere-33`, started by `review start` (flow 4).
- **Steps:**
  1. Asked the review chat to read the PR diff, group the changes, leave two suggested comments as `review_draft` drafts (exit.rs line 1, .verify-dummy.md line 1) and hand the review over as a prepared view.
  2. The agent grouped the changes correctly. It said it could not post drafts: "If you can enable the review-draft tool or give me another way to post drafts...". No draft and no view were made.
  3. Asked it to list its Humata tools. Its reply: bash, view, create, edit, grep, glob, sql, session_store_sql, task, read_agent, list_agents, write_agent, web_fetch, fetch_copilot_cli_documentation, search_code_subagent, semantic_code_search, ask_user, skill, dynamic_workflows_manage, run_dynamic_workflow, image_generation, store_memory, vote_memory, tool_search_tool. No Humata tools.
  4. Asked it to tool-search `review_draft` and `view_prepare`: "No tools found matching pattern". Confirmed.
  5. Owner side (CLI): `humata --json views --session <s>` returns none. `humata view open no-such-view` returns `not_found`.
  6. Owner added a comment with `humata comment add <s> "uhvesta/atmosphere:apps/humata2/cli/src/exit.rs:1-1" "..."`, listed it with `comment ls`, then dropped it with `comment rm`. The CLI has no edit command for a comment, so "edit one draft" could not be done from the CLI.
  7. Submitted the review (see flow 6).
- **What happened:** the agent could not prepare drafts or a view, because the Copilot agent has no Humata MCP tools in this lane. The owner-side open, edit and drop steps had nothing to act on. The flow's edit step has no CLI command.
- **Result:** BLOCKED (env: Copilot 1.0.113 gets no Humata MCP tools in this lane, so review_draft and view_prepare do not exist for the agent). Owner-side submit and drop were exercised through flow 6 and this flow.
- **Evidence:** `12-type-f2.json`, `13-type-tools.json`, `14-type-toolsearch.json`, `15-comment-add.json`, `23-comment-rm.json`, `22-view-open-missing.json`, `24-screen-review-tools.txt`.

## 3. Review a local diff

- **Steps:** `humata --json diff <s1> --against uncommitted` (before and after the file was moved; see flow 1) and `humata --json diff <s1> --against main`.
- **What happened:** against uncommitted, the diff is empty while the file sits outside the clone and shows `review-note.txt` once it is inside. Against main the label is `origin/main...HEAD + staged + unstaged + untracked`, 1 added file, 0 removed.
- **Result:** PASS.
- **Evidence:** `06-diff-f1.json`, `07-diff-f1-after.json`, `21-diff-f3-main.json`.

## 4. Review a PR by number

- **Steps:**
  1. `humata --json review start https://github.com/uhvesta/atmosphere/pull/33 --channel pr-reviews --agent copilot --dry-run` returned `this command needs --machine <name|id>`.
  2. Same command with `--machine ma_c16c…`. Its `--dry-run` did not print a plan: it created a session "Review uhvesta/atmosphere#33" and started it (op `start_session` applied).
  3. Session ready. The chat's bootstrap prompt was pasted but not submitted; Enter sent it. The agent answered "I'm ready for your questions on PR #33".
- **What happened:** the session was started from the URL, with the PR's 2 files and +4 -0 in the prompt. `--dry-run` was not a dry run.
- **Result:** PASS (with F4 about `--dry-run`).
- **Evidence:** `09-review-start-dryrun.json` (the op and its state). No screen capture was saved for the start step.

## 5. Ask questions about the PR in the review chat

- **Steps:** `humata session type <review-s> "Question for the review: which files does PR 33 change, and what does the line added to ... exit.rs do?"`, then Enter. Approved the agent's `gh pr diff 33` bash request (`humata inbox approve`).
- **What happened:** answer "PR 33 changes two files: apps/humata2/cli/src/exit.rs (modified) and a new apps/humata2/flows/cases/results/.verify-dummy.md ... It has no effect on behaviour". Both claims are right.
- **Result:** PASS.
- **Evidence:** `10-type-f5.json`, `11-approve-f5.json`.

## 6. Submit a PR review and check it on GitHub

- **Steps:**
  1. `humata --json review submit <review-s> --as comment --summary "VERIFY flow 6 test review (dummy PR)" --dry-run`: planned, drafts `[]`, token printed (value not kept).
  2. Same command with `--plan <token> -y`: `review.submit` applied, review URL `https://github.com/uhvesta/atmosphere/pull/33#pullrequestreview-5947783342`.
  3. `gh pr view 33 --repo uhvesta/atmosphere --json reviews,comments`: one review, `state: COMMENTED`, body matches, `comments: []`.
- **What happened:** the review was posted as COMMENTED with no inline comments, because the review had no PR-destined drafts (`review drafts` was empty, see F3).
- **Result:** PASS.
- **Evidence:** `16-submit-dryrun.json`, `17-submit-f6.json`.

## 7. Inline feedback sent to the agent's TUI

- **Steps:**
  1. `humata --json comment add <flow1-s> "uhvesta/atmosphere:review-note.txt:1-1" "Please change this line to say: hello from flow seven."`. The comment's destination is `terminal`.
  2. `humata --json comment send <flow1-s>`: status `sent`.
  3. `screen.sh` on the flow1 session: the agent's screen showed `[comment on review-note.txt:1]` and the text, then "Edit review-note.txt".
  4. Approved the edit request. The file now reads `hello from flow seven`.
- **What happened:** the comment reached the agent's TUI and the agent acted on it.
- **Result:** PASS.
- **Evidence:** `18-comment-add-f7.json`, `19-comment-send-f7.json`, `20-screen-f7.txt`, and the approval in the inbox (approved `rq_531e…`).

## Summary

| flow | result |
| --- | --- |
| 1. Agent reviews its own diff | PASS (findings F1, F2, F3) |
| 2. Agent prepares a review, owner opens, questions, edits, drops, submits | BLOCKED (env: no Humata MCP tools for Copilot; no CLI edit for a comment) |
| 3. Review a local diff | PASS |
| 4. Review a PR by number | PASS (F4) |
| 5. Questions in the review chat | PASS |
| 6. Submit a PR review and check on GitHub | PASS |
| 7. Inline feedback to the agent's TUI | PASS |

Pass count: 6 of 7 PASS, with 1 BLOCKED. Flow 2 is BLOCKED, not FAIL, because the blocker is the agent's missing tools, not a Humata defect that the flow reached.

## Findings

- **F1. Session `session type` does not submit (BUG-T1 again).** Repro: `humata session type <s> "hi"`; the text sits in Copilot's prompt. Expected: the text is submitted as a turn. Actual: not submitted; Enter pressed through tmux submits it. Affects flows 1, 4 and 5.
- **F2. The agent's working folder is the session folder, not the clone.** Repro: start a Copilot session in a channel with a repo, ask it to create a file in "the repo". Expected: the file goes into the clone (`sessions/<title>/atmosphere/`). Actual: the file goes into `sessions/<title>/` and the agent says "it isn't a git repo". `humata diff` shows no changes. Code location not found; the agent's start folder appears in the screen header.
- **F3. Copilot agents get no Humata MCP tools in this lane.** Repro: start a Copilot session in a review, ask for `review_draft` and `view_prepare`. Expected: the review tools exist (channel rules allow `review_draft` and `views_prepare`). Actual: the agent's tool list has no humata tools, and tool search finds none. `apps/humata2/api/MCP.md` §1 says the holder writes the agent's MCP config. The only Copilot MCP reference found is `apps/humata2/daemon/contain/src/home.rs:147` (`.copilot/mcp-config.json`, a containment path); `apps/humata2/terminal/provider/src/resume.rs:317` writes `--mcp-config` for Claude only. No `mcp-config.json` appeared in the lane's Copilot home or session folders. Likely a missing Copilot MCP writer.
- **F4. `humata review start --dry-run` is not a dry run.** Repro: `humata --json review start https://github.com/uhvesta/atmosphere/pull/33 --channel pr-reviews --machine <m> --agent copilot --dry-run`. Expected: a plan and token, nothing changed (as `review submit --dry-run` does). Actual: a `start_session` op ran and a review session started. `review submit --dry-run` returns a plan correctly.
- **F5. `review start` needs `--machine` but the error only names the flag.** Expected: the message says the machine to review on, or it defaults to this computer as `session start` does. Actual: `this command needs --machine <name|id>`. Minor.
- **F6. The `view open` error points to MCP tool names.** Repro: `humata --json view open no-such-view`. Actual `fix`: "list the session's views (views_list or ListViews)". The CLI command is `humata views`. Expected: the CLI command name. Minor.
- **F7. No CLI way to edit a review comment, and no PR-destined draft from the CLI.** `humata comment` has add, send, ls and rm, but no edit. `comment add` on a review session creates a `terminal`-destined comment, not a PR draft, so `review submit` lists no drafts (`drafts: []`) after an owner comment. Flow 2's "edit one draft" cannot be done from the CLI, and the owner cannot put an inline PR comment through the CLI. Ask: is this intended (PR-drafts come only from the agent)?
- **F8. Permission requests for read-only steps.** The review agent's `gh pr diff` and the self-review's `humata diff` each waited for approval in the inbox. Expected: maybe a standing allow for read-only commands. Not a bug; noted for the UX pass.
- **F9. Session-start bootstrap is pasted, not submitted.** The review chat's first prompt sat in the input until Enter, same class as F1. The owner sees the prompt without knowing it needs Enter.

## Cleanup

- Both agent sessions stopped; lane daemon stopped (pid 26720); PR #33 closed with `--delete-branch` (the first close attempt failed because the lane env had redirected HOME, so the keyring token was not found; the retry from a clean shell succeeded); local branch `verify/pr-flows` deleted; worktree `V/pr-worktree` removed; `git ls-remote origin verify/pr-flows` returns nothing. No VM users were made (no machine other than this computer was used). No token was written to any file.
- A notice from another session said `V/bin` briefly held Linux binaries (about 14:36 onward), so `humata` failed with exec format errors. None of the `humata` calls in this lane hit that error; every call returned a normal JSON result.

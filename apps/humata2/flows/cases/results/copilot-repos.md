# VERIFY results: lane `repos` (Copilot 1.0.113, claude-haiku-5.5, real GitHub clones)

Scope: flows 06 (proposing a repo), run with `LANE_REMOTE=github`. Repos used: `uhvesta/nvim-reviewtool`,
`uhvesta/rust-benchmarks`, `uhvesta/caveman`, `uhvesta/steel`, `uhvesta/rules_jvm` (all public, read and clone only;
nothing pushed, no issues or PRs, no repo settings touched). Clones were checked on disk. Evidence lives under
`<scratchpad>/verify/evidence/repos/`
(abbreviated below as `E/`).

## 1. Add a repo to a new channel, and to an existing channel

Outcome: a repo added at channel creation, and a second repo added later, both end up cloned and listed.

- Agent: copilot 1.0.113, claude-haiku-5.5 (no agent involved in this flow; CLI as the user)
- Machine: this Mac (the only machine in the lane)
- Steps:
  1. `humata --json channel create repos-a --repo uhvesta/nvim-reviewtool --wait 300`
  2. Polled `humata --json ops op_398d8e9e565d4fbaa7fe6e4dfc2abf44` until `succeeded`.
  3. `humata --json channel repo add repos-a uhvesta/rust-benchmarks --wait 60`, then polled its operation.
  4. `humata --json channel grid repos-a`.
  5. Checked `Humata/repos-a/main/` on disk and `git remote -v` / `git log -1` in each clone.
- What happened: the create returned at once with the operation `running` (`--wait 300` did not wait for the clone;
  it waits for an answer, so the operation had to be polled). Both clones finished as `succeeded`. Both folders exist
  under `main/` with `.git`, `origin` pointing at `https://github.com/uhvesta/<repo>.git`, and HEAD set. `channel list`
  shows both repos once. `channel grid` answers `unimplemented` ("GetGrid isn't built into this humatad2 yet").
- Result: PARTIAL (grid not built in this humatad2; the rest passes)
- Evidence: `E/f1-create-channel.json`, `E/f1-op.json`, `E/f1b-channel-repo-add.json`, `E/f1b-op.json`,
  `E/f1b-channels.json`, `E/f1-grid-after-create.json`, `E/f1-repos-list.json`

## 2. Add a second repo to a running session

Outcome: a second repo is cloned into a session that is already running, and the agent is told.

- Agent: copilot 1.0.113, claude-haiku-5.5, session `repos-s1` in channel `repos-a`
- Machine: this Mac
- Steps:
  1. `humata --json session start --channel repos-a --agent copilot --model claude-haiku-5.5 --title repos-s1 --wait 60`
     and polled the start operation to `succeeded` (clones of `nvim-reviewtool` and `rust-benchmarks` into the session).
  2. Waited for the agent's first turn to end.
  3. `humata --json session repo add <session> uhvesta/caveman --wait 30`, then polled the operation.
  4. Read the session turns with `humata --json session read <session>`; checked the session folder on disk.
- What happened: the operation ran three steps (`add_to_channel`, `clone:caveman`, `tell_agent`) and finished as
  `succeeded`. `caveman` is cloned at `Humata/repos-a/sessions/repos-s1/caveman` on branch `uhvesta/repos-s1`. The agent
  got a user turn saying the repo was added, and then it inspected the repo by itself. The channel gained `caveman`
  once. (Note: I also ran `channel repo add repos-a uhvesta/caveman` at the same time by mistake; both operations
  succeeded and the channel lists `caveman` once.) The gap doc (`apps/humata2/flows/cases/agents/06-proposing-a-repo.md`
  and `workflow/milestones/ROAM.md:67`) says `session repo add` is not built; on this build it works.
- Result: PASS
- Evidence: `E/f2-session-start.json`, `E/f2-session-repo-add.json`, `E/f2-session-add-op.json`,
  `E/f2-channel-add-caveman.json`, `E/f1-repos-list.json`

## 3. An agent asks to add a repo: a Copilot session adds one through humata

Outcome: the agent, told in plain words, adds `uhvesta/steel` through humata; the add waits in the Inbox; I approve it
with the CLI; the channel gets the repo.

- Agent: copilot 1.0.113, claude-haiku-5.5, session `repos-s1`
- Machine: this Mac
- Steps:
  1. `humata --json session type <session> "Please add the GitHub repo uhvesta/steel to this channel using the humata command line, then tell me what it said."`
     The text reached the prompt and was not submitted (BUG-T1 again). I pressed Enter in the headless tmux attach
     (`tmux -L v-repos send-keys -t s-<last 8> Enter`).
  2. The agent ran `which humata`, `humata --help`, `humata channel --help`, `humata channel repo --help`,
     `humata channel repo add --help`, `humata status`, `humata channels`. Each was a Copilot tool call that waited for
     Manual Approval in the Inbox. I approved each one with `humata inbox approve <id>`. Read-only commands only.
  3. The agent ran `humata channel repo add repos-a uhvesta/steel -y --wait 60`. The call did not run. An Inbox
     request appeared: `rq_2cfadb3e6aab4bc1951e94a23ed7f05c`, "repos-s1 asks to add a repo to a channel",
     capability `channel_repo_add`, rule `ask`, pending.
  4. Read the request with `humata --json inbox show`; approved with `humata inbox approve rq_2cfadb3e6aab4bc1951e94a23ed7f05c`.
  5. The agent polled the operation and reported it. Channel `repos-a` lists `steel`. `Humata/repos-a/main/steel` exists.
- What happened: the agent found the command on its own (it read `humata channel --help`, then `channel repo --help`).
  The proposal landed as one Inbox request on the right channel. After approval the operation `op_ee9e1157193041c5a8d6d1af54c0a881`
  succeeded and the agent's final reply said so. Problems: (a) the request card has no repo name. Its preview is
  `access_ahead` with `target: null`, `reason` is empty, and the title is only "repos-s1 asks to add a repo to a channel".
  An approver cannot tell which repo is being added from the Inbox. The name appears only in the CLI args. (b) The
  Copilot tool approvals took about 6 round trips for read-only help calls, and the agent's shell call sat blocked
  (up to 2m 26s) until I approved it. Nothing on screen said why it was waiting.
- Result: PASS (with findings 1, 2, 3, 6)
- Evidence: `E/f3-session-type.json`, `E/f3-inbox-1.json` (the first bash request), `E/f3-approve-tool.json`,
  `E/f3-inbox-channel-repo-add.json` (the card with no repo name), `E/f3-approve-channel-repo.json`,
  `E/f3-screen-end.txt`, `E/f3-inbox-all.txt`

## 4. An agent adds a repo directly, if access rules allow it

Outcome: with the rule set to allow, the agent adds a repo with no Inbox request for the add itself.

- Agent: copilot 1.0.113, claude-haiku-5.5, session `repos-s1`
- Machine: this Mac
- Steps:
  1. `humata --json access --channel repos-a` (rules as they stood): `channel_repo_add` is `ask`. Others that matter:
     `channel_change` never, `session_type` ask, `session_start_same_machine` ask, `own_session_end` ask.
     `humata --json access why <session> channel_repo_add` says `ask` (source `rule`).
  2. I meant to preview with `humata --json access set channel_repo_add allow --channel repos-a --dry-run`. It did
     not preview: the rule was written (revision 0 -> 1, `channel_repo_add` = `allow`). See finding 1. The rule was
     then `allow`, which this flow needed, so I kept it.
  3. Told the agent in plain words to add `uhvesta/rules_jvm` with `humata channel repo add ... -y`. Approved its bash calls
     as before.
  4. The agent ran `humata channel repo add repos-a uhvesta/rules_jvm -y --wait 60`. No `channel_repo_add` request
     appeared in the Inbox; the only requests were the agent's bash approvals.
  5. Operation `op_5388407d86f34bd292023d9fd686cf42` succeeded. `rules_jvm` is in `channel repos-a` and cloned at `main/rules_jvm`.
     The agent confirmed it.
  6. `humata --json access why <session> channel_repo_add` now says `allow` (source `rule`).
- Recorded rules (what the access rules say): the add is `ask` by default. The Inbox card is the path for an
  agent under `ask`, and no add happens until the owner approves. Under `allow` the add is direct, with no card.
  Under `never` it is refused (not run; the code path is in the gap doc, `channel_add` tests).
- Result: PASS
- Evidence: `E/f3-access-before.json` (rules before the change; the first call without `--channel` was refused with
  "needs --channel"), `E/f4-access-why-now.json`, `E/f4-screen-end.txt`, `E/inbox-all-final.json`

## 5. A repo that does not exist or is not reachable

Outcome: the error says what failed and how to fix it.

- Agent: none (CLI as the user)
- Machine: this Mac
- Steps:
  1. `humata --json channel repo add repos-a uhvesta/no-such-repo-zz --wait 30`, then polled the operation.
  2. The same with an owner that does not exist: `nobodyhere-zz9/whatever`.
  3. `humata --json session repo add <session> uhvesta/no-such-repo-zz --wait 30`, then polled the operation.
  4. Malformed names: `justname`, `uhvesta/`, `/abs/path/repo`, `../rel/x`, `uhvesta/a/b`.
  5. Tried `humata --json channel repo rm repos-a uhvesta/no-such-repo-zz` to clear the failed repo.
  6. Ran `humata --json channel repo add repos-a uhvesta/no-such-repo-zz` again (retry).
- What happened:
  - Missing repo, channel add: the operation `op_f5c8757b3cd6438e82e46aa16970ec3c` failed at `clone_main:uhvesta/no-such-repo-zz`.
    The message: "couldn't clone uhvesta/no-such-repo-zz: git clone ... failed (exit status: 128) ... fatal: could not read
    Username ... terminal prompts disabled; check the repository address, that this machine can reach it, and that it has
    access (git never asks for a password here)". That part is useful. The `fix` says "the channel and its folders were
    kept, with that repository marked as failed; ... then create the channel again under another name". For an existing
    channel that is the wrong advice (finding 4). `retry` is null, and the repo is not marked as failed anywhere in
    `channels` (finding 5).
  - Missing owner: the same message and fix, with `nobodyhere-zz9/whatever`.
  - Missing repo, session add: operation `op_ee3092fca7644812b1f03adab1641a44` failed with `the channel's clone of
    uhvesta/no-such-repo-zz failed: ...`. The fix is "check the repo's owner and name ... then add it to the session again".
    That one is right for a session.
  - Malformed names: refused at once, `invalid_field`, "repo can't be \"justname\": expected owner/name". Clear.
  - `channel repo rm` on the failed repo: `unimplemented`, "PlanRemoveRepo isn't built into this humatad2 yet", fix "update
    Humata on this machine; until then, do this from the app". So the failed repo can't be removed from the CLI (finding 5).
  - Retry: the same add ran again and failed again with the same message. It was listed once.
  - Not tested: a repo that exists but that this login cannot read (a private repo of someone else). I had no safe one.
- Result: PARTIAL (missing repo: error names the cause and the check, but the fix text is wrong for an existing channel,
  and the failed repo stays in the channel with no way to remove it from the CLI; malformed names: PASS)
- Evidence: `E/f5-missing-repo-add.json`, `E/f5-missing-owner-add.json`, `E/f5-session-missing.json`,
  `E/f5-op-*.json`, `E/f5-malformed.txt`, `E/f5-channels-after-missing.json`, `E/f5-channel-repo-rm-missing.json`

## Findings

1. BUG: `humata access set --dry-run` applies the change instead of printing a plan. Steps: `humata --json access set channel_repo_add allow --channel repos-a --dry-run`.
   Expected: prints the plan and a token, changes nothing (the global flag says "Print the plan and its token; change nothing").
   Actual: the rule is written (revision 0 -> 1). Cause: `cli/src/commands/inbox.rs:237-250` sends `SET_CHANNEL_RULE` with
   `ctx.present_json`, not the planned path. `cli/src/call.rs:275` (`if self.flags.dry_run`) is never reached for this call.
   Evidence: `E/f3-access-before.json` (before), `E/f4-access-why-now.json` (after).
2. LIKELY BUG (code read, not run): `humata channel repo add --dry-run` probably applies too. `cli/src/commands/looking.rs:110-116`
   sends `ADD_REPO` with `reply(...)`, not `ctx.planned(...)`, so the dry-run branch in `call.rs:275` is skipped. I did not run it, because
   it would clone a repo.
3. UX/BUG: the Inbox card for an agent's repo add names no repo and no reason. Steps: flow 3. The request
   `rq_2cfadb3e6aab4bc1951e94a23ed7f05c` has `preview.access_ahead.target = null` and `reason = ""`; the title is only
   "repos-s1 asks to add a repo to a channel". Expected (per `flows/cases/agents/06-proposing-a-repo.md` step 2): the card names
   `acme/shared-lib`. Actual: the repo appears only in the operation args. Code: `daemon/humatad/src/inbox/deferred.rs:142-147`
   (`AccessAhead` with `target` from `offered`, which is None for this capability; `reason` comes only from a board request, line ~133).
   The agent's `--reason`, if any, is dropped (the MCP path is not built, see the gap doc).
4. BUG (wrong text): the fix for a failed clone tells an owner who added a repo to an existing channel to "create the channel again
   under another name". Steps: flow 5, `channel repo add repos-a uhvesta/no-such-repo-zz`. Expected: advice for the existing channel
   (remove the repo, or fix its name and add it again). Actual: `daemon/humatad/src/channel_service.rs:874`.
5. BUG (state): a repo whose clone failed stays in the channel's repo list with no marker, and `channel repo rm` is not built.
   Steps: flow 5. After the failed add, `humata --json channels` lists `no-such-repo-zz` and `whatever` as normal repos. The fix text
   says "marked as failed", which is not shown. Next session starts for this channel will try to clone them too. `channel repo rm`
   answers `unimplemented` (the gap doc already lists this as RemoveRepo, not built). The CLI has no way to clean this up.
6. NOT BUILT: `humata channel grid` answers `unimplemented` (GetGrid). Flow 1's grid check cannot be done.
7. UX: `--wait` on `channel create`, `channel repo add` and `session start` returns the operation as `running` at once, even
   with `--wait 300`. The help says the wait is "for an answer", so an operation has to be polled with `humata ops <id>`. The
   docs do not say so, and a user would think the clone finished.
8. BUG-T1 reproduced: `humata session type <s> "<text>"` puts the text in Copilot's prompt and does not submit it. Enter
   pressed in the tmux attach submits it. Evidence: `E/f3-session-type.json`.
9. UX: every Copilot tool call from the agent waits for Manual Approval through the Inbox. Read-only `humata --help` calls each
   needed one approval, and the agent's shell call sat blocked for up to 2m 26s before I answered it. The screen does not say
   the call is waiting for an approval. (`E/f3-inbox-1.json`.)
10. DOC drift: the gap doc says `session repo add` (`AddRepoToSession`) is not built (`daemon/humatad/src/session_service.rs:2366-2372`,
    `workflow/milestones/ROAM.md:67`). On this build it works (flow 2). The doc needs updating.

Side effects in this lane (not a product issue): flow 4 `access set` (finding 1) left `channel_repo_add = allow` on `repos-a`.
Also one stray channel `repos-x` was created with `uhvesta/rules_jvm` while I checked the create command. Both live only in the lane's own daemon.

## Summary

| flow | result |
|---|---|
| 1. Add a repo to a new channel and to an existing channel | PARTIAL (grid not built) |
| 2. Add a second repo to a running session | PASS |
| 3. Agent asks, Inbox request, approve with the CLI | PASS (findings 3, 8, 9) |
| 4. Agent adds directly when the rule allows it | PASS (rule `ask` by default; `allow` skips the card) |
| 5. Repo missing or unreachable: the error | PARTIAL (wrong fix text for channels; failed repo stays; no way to remove it) |

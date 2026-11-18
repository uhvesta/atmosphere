# VERIFY lane `host`: Copilot on this Mac, five flows

Agent: copilot 1.0.113-1, claude-haiku-5.5. Machine: This Mac (one machine only; no VM was used). Lane daemon: `humatad2` admin, own state and HOME. Evidence: `<scratchpad>/verify/evidence/host/`.

Harness notes (not product): `verify/bin` was overwritten with Linux aarch64 binaries from 14:36 until the other session restored it at 14:37. Any `humata` call in that window failed with `exec format error`, and a Desk start failed with `holder cannot execute binary file`. Those errors are recorded below only where they touched the flow. A second Desk start after the restore worked.

---

## 1. A session on this Mac writes a file in its folder, and its transcript shows it

Result: PARTIAL

Agent: copilot 1.0.113-1, claude-haiku-5.5. Machine: This Mac.

Steps:
1. `humata --json session start --channel host-ch1 --agent copilot --model claude-haiku-5.5 --title "host flow 1 hello"` gave session `se_52dcc155f9b34c4a9b6971a755e8e023`.
2. `humata --json session type <s> "Create a file named hello.txt ... containing exactly: hi from copilot. Then stop."` returned `typed: true`, but the text stayed in Copilot's prompt unsubmitted (BUG-T1 again).
3. Pressed Enter in the agent tab through tmux (`evidence/host/enter.sh`). Copilot asked to run `create`, which is the Manual Approval inbox request `rq_74c202a9…`.
4. `humata --json inbox approve rq_74c202a9…`. Copilot wrote the file.
5. Checked the file and `humata session read` and `humata conversation`.
6. Second prompt, "Remember the word amber. Reply with only OK.", answered `OK`.

What happened:
- The file was created in the session folder, not in the repo clone: `.../host-ch1/sessions/host-flow-1-hello/hello.txt`, content `hi from copilot` (15 bytes, no newline), mode 600.
- Immediately after the approval, `session read` showed the user prompt and two tool entries, but not the agent's final reply ("I created hello.txt ..."). `humata conversation` showed the reply. About 1.5 minutes later `session read` still lacked it. It appeared after the next prompt. Mirror lag, not reproduced with a shorter wait.
- `humata ls` showed `probably_idle` before the first prompt, with `unread: true` and `turn_end: hooks`, then `working`, then `needs_input` after the approval (though the turn had already ended), then `idle`.

Evidence: `evidence/host/01-start.json`, `01-type.json`, `01-inbox-pending.json`, `01-approve.json`, `01-read.txt`, `01-conversation.txt`, `01-read.json`.

---

## 2. A new session in an existing channel

Result: PASS

Agent: copilot 1.0.113-1, claude-haiku-5.5. Machine: This Mac.

Steps:
1. `humata --json session start --channel host-ch1 --agent copilot --model claude-haiku-5.5 --title "host flow 2 second"` gave `se_18d9abc2ddd14986aaffff39d6b97e00`.
2. Sent "Remember the word cobalt. Reply with only OK." and got `OK`.

What happened: the session started in `host-ch1` on its own branch `uhvesta/host-flow-2-second` and folder `host-flow-2-second`. The first Enter was lost because the agent tab did not exist yet, which is my harness timing, not the product. It was submitted on the second try.

Evidence: `evidence/host/02-start.json`, `02-read.txt`, `02-topology.txt`.

---

## 3. Grow a channel: more sessions, a second repo, channels and topology

Result: PASS (with findings)

Agent: copilot 1.0.113-1, claude-haiku-5.5. Machine: This Mac.

Steps:
1. Seeded a second repo with plain git under `verify/remotes/verify/second.git`, from `verify/seed/second`.
2. `humata --json channel repo add host-ch1 verify/second`. The `add_repo` operation succeeded.
3. `humata --json session start --channel host-ch1 ... --repo verify/second --title "host flow 3 second repo"` gave `se_57048dc1013d40d5a2311f5099981d9b`. Its branch is `uhvesta/host-flow-3-second-repo`.
4. Asked it to list its folder. It ran a shell command, which needed approval. I approved it through the inbox (`rq_6cdff50e…`). It listed `second/` with `.git`, `README.md`, `util.py`.
5. `humata channels`, `humata topology --channel host-ch1`, `humata ls`, `humata channel grid host-ch1`.

What happened:
- `host-ch1` shows repos `[demo, second]` and 3 live sessions. Topology lists all three with their ids, status and branches.
- `channel grid` fails: `humata: humata.v1.ChannelService.GetGrid isn't built into this humatad2 yet`, exit 7. It is in `--help` but not built.
- While the bash approval was pending, the session status was `working`, not `needs_input`. The request only showed in the inbox.

Evidence: `evidence/host/03-repo-add.json`, `03-start-second-repo.json`, `03-topology.txt`, `03-read-s3.txt`, `03-approve-bash.json`, `06-channels-final.json`.

---

## 4. Move a session between channels, and between sessions, so its folder moves and it resumes and remembers

Result: FAIL (no channel move the CLI can do; the carry path that exists refuses Copilot conversations)

Agent: copilot 1.0.113-1, claude-haiku-5.5. Machine: This Mac.

Steps:
1. `humata session move --help`: `session move <SESSION> --to <TO>`, where `--to` is a machine. `humata session move <s> --to host-ch2 --dry-run` gave `no machine is named "host-ch2"`. `--to "This Mac"` also fails, see findings.
2. `humata session continue <s> --channel host-ch2 --dry-run`, the nearest "carry the conversation to another channel" path. Refused with `agent_lacks_feature`, exit 7, "nothing was changed".
3. `humata --json session fork <s> --kind context-only --channel host-ch2 --dry-run` and `--kind code-and-context ...`. Both refused with the same error.
4. `humata --json session fork <s> --kind code-only --channel host-ch2 --title "host fork code only" --agent copilot` ran. It copies code and starts a fresh conversation. This is a copy, not a move.
5. Asked the fork "What word did I ask you to remember earlier?". It answered that it saw no such request. That is the expected result for a fresh conversation.

What happened:
- No session moved between channels. The conversation-carrying operations (`continue`, fork with context, machine move) are refused for every Copilot session, with: "Humata can't read this Copilot conversation (CLI version 1.0.113-1) yet: this version of Humata doesn't know its format or some of its records". Root cause in the findings.
- The code-only fork landed in `host-ch1`, not `host-ch2`, although I passed `--channel host-ch2` (finding F2).
- Machine move between machines was not tested live, because I did not set up a second machine. The code path is the same carry check (`daemon/humatad/src/carry_service.rs:284-296`), so I expect the same refusal. Not verified.

Evidence: `evidence/host/04-continue-dryrun.json`, `04-fork-code-only.json`, `04-fork-read.txt`, the `session move` dry-run outputs (copied above).

---

## 5. Move a session from the Desk into a channel's session

Result: PARTIAL (Desk start and work pass; the move fails at `pack` with the same Copilot refusal)

Agent: copilot 1.0.113-1, claude-haiku-5.5. Machine: This Mac.

The CLI cannot start a Desk session: `session start` needs `--channel`, and `humata desk` only has `keep`. I started the Desk session with a raw gRPC call to the lane daemon (`DeskService/StartDeskSession`, a helper at `evidence/host/grpc_call.py`).

Steps:
1. First Desk start, `host-desk-start-1`, failed during the `verify/bin` outage (`holder cannot execute binary file`). It left a Desk record stuck in `starting` (`se_2fbd1f5f…`, folder `Humata/desk/host-desk-one`).
2. Retried after the restore: `StartDeskSession` gave `se_40cc89030f254f22bfa5e9f4ca1ce1df` at `Humata/desk/host-desk-one-2`, Copilot with claude-haiku-5.5.
3. Sent "Create a file named notes.md containing exactly one line: desk note from copilot. Then reply with only DONE." Approved the write request in the inbox. The reply was `DONE`. `notes.md` contains `desk note from copilot`.
4. `DeskService/PlanDeskMove` (session, `host-ch2`) gave a plan with branch `uhvesta/host-desk-one`.
5. `DeskService/MoveDeskSessionToChannel` with that plan (token `2b0a34f7…`) started operation `op_d2789899…`.
6. Polled the operation until it ended.

What happened:
- The move ran `add_repos` (done, no repos were added), then `pack` failed. The error: "couldn't carry the session's copilot conversation: Humata can't read this Copilot conversation (CLI version 1.0.113-1) yet … Nothing was copied or deleted", with `agent_lacks_feature: conversation_move`.
- Afterwards the session is still a Desk session, `idle`, and its `notes.md` is in place. `host-ch2` still has only `demo`. That matches "a failure leaves it a Desk session, still running".
- The proto says a Copilot move is refused up front, before anything changes (`api/proto/humata/v1/desk.proto` around lines 125-128). The actual refusal came after `add_repos` and was worded about unreadable records. The doc and the behaviour disagree.

Evidence: `evidence/host/05-desk-start.txt` (first attempt, failed), `05-plan-desk-move.txt`, `05-desk-move.txt`, `05-ops-latest.json`, `grpc_call.py` and `grpc_raw.py` in `evidence/host/`.

---

## Findings

Each candidate bug or UX problem, with steps, expected and actual, and the code where I found it.

**F1 (bug, high). Every Copilot 1.0.113-1 conversation is refused by every carry operation.** Continue, fork with context, Desk-to-channel move and (per the code) machine move all fail.
- Steps: any Copilot session with at least one prompt. Run `humata session continue <s> --channel host-ch2 --dry-run`, or `humata session fork <s> --kind context-only --dry-run`.
- Expected: the conversation carries, as `X1` claims for 1.0.113-1.
- Actual: refused. The unknown record is `instruction.update_delivery`, written by the CLI on every session, not in the strict table `DATA_FIELDS` (`daemon/conversation/src/copilot.rs:231`). `is_known` (about line 887) returns false, and the adapter reports `Unsupported` (about line 874). I checked this against the real `events.jsonl` files of all three sessions: `instruction.update_delivery` is the only unknown type in sessions 2 and 3. Session 1 also has nine `model.*` types written by Copilot's own side model calls (for example `model.turn_started`, `model.messages_snapshot`). The `X1` table in the file header (line 86 onward) claims 1.0.113-1 was covered by fixture `copilot_roam2.py`, but that fixture did not include `instruction.update_delivery`.
- The message also says "update Humata" and names no version that knows this CLI. Expected: an actionable fix or a working carry.

**F2 (bug, medium). `session fork --channel <other>` ignores the channel.** The code-only fork went into `host-ch1`, the parent's channel, without any error. The dry-run plan does not name the target channel.
- Steps: `humata --json session fork <s> --kind code-only --channel host-ch2 --title x --agent copilot`, then `humata ls`.
- Expected: the fork is in `host-ch2`. Actual: `channel` is `ch_5488…` (host-ch1). Evidence: `04-fork-code-only.json`, `06-ls-final.txt`.

**F3 (gap). The CLI has no session move between channels, and no way to start a Desk session.** `session move --to` takes a machine only. `humata desk` has only `keep`. `session start` needs `--channel`. Only raw gRPC (`DeskService/StartDeskSession`, `MoveDeskSessionToChannel`) can do Desk work.
- Expected for a human flow 5 with the CLI: a `humata desk start` and a `session move --to-channel`. Actual: neither exists in the CLI (`cli/src/args.rs` around 230-340 and 488).

**F4 (UX). Machine names with spaces can't be used.** `host ls` prints `name: This Mac`. `--to "This Mac"`, `host show "This Mac"`, and `host show "this mac"` all fail with `no machine is named ...`. `host show this-mac` works (a slug). The error says only "Check the name" and does not suggest the slug or the id.

**F5 (bug, low). `humata channel grid` is in `--help` but not built.** It fails with exit 7: `humata.v1.ChannelService.GetGrid isn't built into this humatad2 yet; update Humata on this machine`. The "update" advice does not apply to a fresh build.

**F6 (bug, medium). Session status is `working` while a Manual Approval request is pending.** Session 3's bash request sat in the inbox with the session shown as `working`, not `needs_input`. Session 1 showed `needs_input` after I approved the file write, although its turn had already ended. Expected: `needs_input` whenever an inbox request is pending. Actual: the status only tracks the agent's turn.

**F7 (UX). `probably_idle` (`Turn::Uncertain`) shows on a fresh session before any prompt.** Sessions 1 and 3, and the code-only fork right after start, all showed `probably_idle` with no turn yet. It settles to `idle` after the first turn ends by hooks. The status mapping is at `clients/live/src/map.rs:199` and `:314`. Expected: a clear "not started yet" state rather than "uncertain idle".

**F8 (bug, low-medium). `session read` lacks the agent's reply for a while.** Session 1's last reply was missing from `session read` for at least 1.5 minutes. `humata conversation` showed it at once. It appeared after the next prompt. Expected: `session read` shows the reply when the turn ends.

**F9 (cosmetic). Approval preview diff header is malformed.** It shows `--- a/dev/null` and `diff --git a/private/tmp/...` with the leading slash dropped from the absolute path. Expected: `/dev/null` and a normal `a/` path for a new file.

**F10 (bug, medium). A failed Desk start leaves a record stuck in `starting`.** The first Desk start failed when the holder could not start. The record `host desk one` (`se_2fbd1f5f…`, folder `Humata/desk/host-desk-one`) stayed in `starting` for the rest of the run, while the retry was `idle`. Expected: a failed start is removed or shows failed. This was triggered by the harness outage, but the leftover is the product's problem.

**F11 (doc vs behaviour). The Desk move doc says Copilot is refused before anything changes.** `api/proto/humata/v1/desk.proto` around lines 125-128 says a Copilot move fails with `AgentLacksFeature FEATURE_CONVERSATION_MOVE` before anything changes. The actual refusal came after `add_repos` had run, and its text is about unreadable records (F1). Both refusals should be checked against each other.

**F12 (observation). Copilot makes its own model calls.** Session 1's `events.jsonl` shows `model.*` records from a `gpt-5.4-nano` call per prompt, with a system prompt that classifies "frustration with an AI coding assistant". Humata does not read these records, so it does not matter for carry except through F1. Recorded because it's an unexpected model call on the user's account.

**F13 (mirror lag, note).** The mirror of `session read` is lagging (F8). Not a bug on its own.

**F14 (harness, not product).** `verify/bin` was overwritten by Linux aarch64 binaries from 14:36 until 14:37 (the other session reported it fixed). Calls in that window failed with `exec format error` and `holder cannot execute binary file`. I used my own copies in `lanes/host/bin` for a while and removed them after the restore. Not counted as a product failure.

## Result table

| flow | result |
|---|---|
| 1. Session on this Mac writes a file, read shows it | PARTIAL |
| 2. New session in an existing channel | PASS |
| 3. Grow a channel: sessions, second repo, channels, topology | PASS |
| 4. Move a session between channels or sessions | FAIL |
| 5. Desk session moved into a channel's session | PARTIAL |

Pass count: 2 of 5 PASS, 2 PARTIAL, 1 FAIL. Machine move between machines: not run (no second machine).

## Cleanup

- Desk record `host desk one` (`se_2fbd…`) left in `starting` (F10). Lane daemon stopped at the end. No VM users were created.
- Stray channel `host-ch3` was created by mistake and deleted (`trash_item` kept).

# VERIFY results, lane fork: forks of a Copilot session on this Mac

Lane `fork`. Evidence folder: `<scratchpad>/verify/evidence/fork/` (short names below are relative to it).

Agent: Copilot CLI 1.0.113-1, model claude-haiku-5.5.
Machine: this Mac (`ma_50c4185ff865438f8b5b36a5b502fa9c`). Channel `forkch` on repo `verify/demo` (seed commit `2494c0b`). Isolated daemon `humatad2_admin` under the lane's own HOME, COPILOT_HOME and socket. No VM used, no VM users made.

Parent session: `se_eeda17e00add4d82ae38f415bd370222` ("Words A"), conversation `bcc58200-9e89-41d6-9951-434b1b62f84d`.

## Remember two words, then two turns

Agent: Copilot 1.0.113-1, claude-haiku-5.5. Machine: this Mac.

Steps:
1. `humata --json session start --channel forkch --agent copilot --model claude-haiku-5.5 --title "Words A"`.
2. `humata session type <parent> "Remember the word APPLE. Reply only with OK."`. Reported `typed: true`.
3. Pressed Enter in the session's tmux tab, which submits it (`say.sh`).
4. Repeated for "Remember the word PEAR too. Reply only with OK."

What happened: `session type` put the text in Copilot's prompt and did not submit it (BUG-T1 reproduced). Enter submitted it. Both turns answered `OK`. `session read` lists turns 1 and 2.

Result: PASS (with the BUG-T1 workaround).
Evidence: `01-start.json`, `02-type-turn1.txt`, `03-read-after-2-turns.json`.

## Fork from the start (empty fork)

Agent: Copilot 1.0.113-1, claude-haiku-5.5. Machine: this Mac.

Steps:
1. `humata --json session fork <parent> --kind code-only --title "Empty fork" --wait 120`.
2. Waited for `probably_idle`.
3. Asked it which words it was asked to remember.

What happened: the fork started (new branch `uhvesta/empty-fork`, new conversation `e433c0ac-0dd1-473d-8b4c-174aee39c557`). Its `fork_origin` is `{parent, kind: code_only, before_turn: 0}`. Its answer: "NONE. There is no earlier conversation in this session, and you haven't asked me to remember any words." The clone is the seed commit `2494c0b` with `calc.py` and `README.md`. The parent made no file changes, so the clone check does not show carried work.

Result: PASS.
Evidence: `05-fork-code-only.json`, `06-ls-poll.json`, `07-empty-fork-read.json`, `09-agents-ls.json`.

## Fork before a turn (before turn 2)

Agent: Copilot 1.0.113-1, claude-haiku-5.5. Machine: this Mac.

Steps:
1. `humata --json session fork <parent> --kind code-and-context --before-turn 2 --title "Before turn 2"`.
2. Same with `--dry-run`.
3. `--before-turn 1` (CLI check, no daemon call).

What happened:
- `--before-turn 1` is a CLI usage error: `1 is not in 2..=4294967295`. Nothing is forked.
- `--before-turn 2` gets past the CLI and is refused by the daemon with `kind: agent_lacks_feature`, `details.feature: context_fork`: "the conversation holds records this version of Humata doesn't know ... Copying the files as they are isn't offered". Nothing is made. Refusal in `08-fork-before-turn2-attempt.txt`.
- The parent's `events.jsonl` and `workspace.yaml` were not written by the refused attempts (mtimes 14:25:33 and 14:25:16, before the attempts). Hashes are in `13-parent-hash-after-refused-forks.txt`.

Result: FAIL. The fork is refused for a Copilot conversation that Humata should be able to cut. See Finding F1.
Evidence: `08-fork-before-turn2-attempt.txt`, `13-parent-hash-after-refused-forks.txt`.

## Fork from now (context fork)

Agent: Copilot 1.0.113-1, claude-haiku-5.5. Machine: this Mac.

Steps:
1. `humata session fork <parent> --kind code-and-context --dry-run`, and the same against the empty fork (`se_fd3e4d89187e4f9f85515643079461f6`).
2. `--kind context-only --dry-run` on the parent.
3. `humata session continue <parent> --channel forkch --dry-run` (refused as "already in that channel", not a conversation check, so it is not evidence of the bug).

What happened: the CLI offers the kind. The daemon refuses it for the parent and for the empty fork, with the same `agent_lacks_feature` error as before-turn. The message says "Until then, fork it from now or start fresh with a summary" (Finding F2). `--kind code-only` dry-runs succeed against both sessions.

Result: FAIL. The context fork that the first two turns should give ("it should know both") could not run. Knowledge of APPLE and PEAR in a fork was not checked.
Evidence: `08b-fork-agent-flag-attempt.txt`, `21-topology-after.json`.

## Fork with a different agent

Agent: Copilot 1.0.113-1 as the source. Targets: codex, claude, zsh (`humata --json agents ls`, `09-agents-ls.json`).

Steps:
1. `humata session fork <parent> --kind code-only --agent zsh --dry-run`, then the real run `--wait 120`.
2. `--agent codex --kind code-only --dry-run` and `--agent claude --kind code-only --dry-run`. These are dry-runs only (plan printed, nothing changed, no agent started), run before I applied the deferral. I did not run a real fork to codex or claude.

What happened:
- zsh (plain shell kind, `supports_context_fork: false`, `supports_resume: false`): the code-only fork to zsh worked. The session `se_6b7dd00785674bb398bff8f384deab46` ("Zsh fork") started and reached `running_unknown_turn`. It is a shell, so it has no conversation and no turns. Stopped afterwards with `session stop` (rc 0).
- codex and claude dry-runs: the plan says "Forks 'Words A' into a new session on branch uhvesta/fork-of-words-a ... starts a fresh conversation." It does not say which agent will run it (Finding F3). Nothing was run.
- `--agent shell` (not an agent kind here) is refused: "this machine has no agent named shell to run the fork ... pick one of the agents the new-session dialog lists".

Result: codex: DEFERRED (owner). claude: DEFERRED (owner). zsh: PASS (code-only fork to a shell kind).
Evidence: `10-zsh-dryrun.txt`, `11-fork-zsh.json`, `12-ls-after-zsh.json`, `22-stop-zsh-fork.json`, `09-agents-ls.json`.

## Stop and resume a forked session

Agent: Copilot 1.0.113-1, claude-haiku-5.5 (the empty fork). Machine: this Mac.

Steps:
1. Asked the empty fork to remember KIWI. Reply `OK`.
2. `humata --json session stop <empty fork> --wait 60`. Status `exited`.
3. `humata --json session resume <empty fork> --wait 90`. Status `probably_idle`.
4. Asked "What word did I ask you to remember?"
5. Asked the parent "What two words did I ask you to remember?" after the refused forks.

What happened: after the resume the fork answered `KIWI`, and its turns (1 NONE, 2 KIWI, 3 the question) are unchanged. The parent still answers `APPLE PEAR` and keeps its three turns. Note a sequencing gap: a `session read` about 5 s after the KIWI prompt showed no agent reply yet, and I stopped the session after that read without waiting for the turn to end. A later read shows the reply `OK` recorded, so the stop did not lose it here, but I did not check whether a stop mid-turn would.

Result: PASS.
Evidence: `14-parent-read-after-forks.json`, `15-empty-fork-read-kiwi.json`, `16-stop-empty-fork.json`, `18-resume-empty-fork.json`, `20-empty-fork-read-after-resume.json`.

## Topology

`humata --json topology --channel forkch`, before the forks (`04-topology-before-forks.json`) and after (`21-topology-after.json`).

| session | title | agent | status | fork_origin | forks | conversation | branch |
|---|---|---|---|---|---|---|---|
| se_eeda17e0... | Words A (parent) | copilot | idle | none | empty fork, zsh fork | bcc58200 | uhvesta/words-a |
| se_fd3e4d89... | Empty fork | copilot | idle | code_only, before_turn 0 | none | e433c0ac | uhvesta/empty-fork |
| se_6b7dd007... | Zsh fork | zsh | running_unknown_turn | code_only, before_turn 0 | none | 28ca26d9 | uhvesta/zsh-fork |

Before the forks the parent had `forks: []`.

## Parent conversation hashes

After the refused forks, before the parent was asked again (`13-parent-hash-after-refused-forks.txt`):
- `events.jsonl` sha256 `2f669459423f2ba88a537f646b12e4ec43bd95e2e4b604e3611f6b7d41a12bdc`
- `workspace.yaml` sha256 `0669781f4f870e343b94bb4a770dbe29d44c8347c9cf5964ed9559f305689c72`

No hash was taken before the first fork. The parent's mtimes show the refused forks wrote nothing. Hashes after the later parent question are not comparable, since that turn appended to the file.

## Results

| flow | result |
|---|---|
| 1. Two turns (APPLE, PEAR) | PASS (BUG-T1 workaround needed) |
| 2. Empty fork (code-only) starts fresh | PASS |
| 3. Fork before turn 2 knows APPLE, not PEAR | FAIL (refused, F1) |
| 4. Fork from now knows both | FAIL (refused, F1) |
| 5a. Different agent: codex | DEFERRED (owner) |
| 5b. Different agent: claude | DEFERRED (owner) |
| 5c. Different agent: zsh (shell kind), code-only | PASS |
| 6. Stop and resume a fork keeps its conversation | PASS |

## Findings

**F1. Copilot conversations with records Humata doesn't know refuse every context fork (high).**
- Steps: start a Copilot session, send one or two turns, then `humata session fork <s> --kind code-and-context` (also `--kind context-only` and `--before-turn 2`).
- Expected: the fork is made from the copy of the conversation, as the flow expects for Copilot.
- Actual: `agent_lacks_feature`, "this version of Humata doesn't know its format or some of its records". Nothing is made.
- Cause: the parent's `events.jsonl` (written by the real CLI 1.0.113-1) holds record types the strict parse does not list: `model.turn_started`, `model.model_call_started`, `model.captured_assignment_context`, `model.model_call_success`, `model.message`, `model.response`, `model.turn_ended`, `model.messages_snapshot`, and `instruction.update_delivery`. The known table `DATA_FIELDS` in `apps/humata2/daemon/conversation/src/copilot.rs` (about lines 228–500) has no such entry, and `is_known` (about line 887) returns false for an unlisted type. The module doc (about line 84–90) says new types are added "only once the real CLI shows what it holds". The real CLI now shows them, in this run.
- Also: the same error shows for the empty fork (which has two turns).

**F2. The refusal tells the user to do the refused thing (medium).**
- The message says "Until then, fork it from now or start fresh with a summary". "Fork it from now" is the code-and-context fork that the same message refuses. Only the `code-only` hint ("fork with CODE_ONLY for a fresh agent") is correct.
- Expected: the message names only options that work (code-only, or a summary-based start).

**F3. The plan does not name the agent that will run a fork (low).**
- `humata session fork <s> --kind code-only --agent codex --dry-run` and `--agent claude` both print "starts a fresh conversation" and no agent name. The `--agent zsh` plan has the same text, though a shell has no conversation.
- Expected: the plan names the agent and says "no conversation" for a shell kind.

**F4. Refusal and reads disagree (low).**
- The refusal says the parent's "turns can't be listed", but `humata session read <parent>` lists all three turns from the same session at the same time. Possibly `session read` reads Humata's mirror and not the file. Check which, and make the message say what is really unavailable.

**F5. BUG-T1 is still present (known).**
- `humata session type` put text in the prompt and did not submit it. Enter in the tmux tab submitted it. Reproduced on Copilot 1.0.113-1 at flow 1.

**Harness note (not a product finding).** Outside the lane env, `humata` on PATH is `~/.local/bin/humata`, an older build whose `session fork --help` does not list `--agent` or `--before-turn`. Under `. lanes/fork/env.sh` the harness binary is used and lists both. Use the lane env for every command.

**Not tested.** The MCP `session_fork` and `session_continue_in_channel` tools (`before_turn` handling). Codex and Claude forks (owner defer). The `code-only` clone check with carried work (the parent made no file changes).

## Rerun 2026-11-17 (build after D758)

Agent copilot 1.0.113, claude-haiku-5.5; This Mac; lane fork, channel forkr. Evidence: V/evidence/fork/r2-*.json, model-all.txt.
Model seen: claude-haiku-5.5 for the parent, empty fork, before-2 fork, from-now fork and the code-only `--channel` fork (currentModel in each events.jsonl under the lane's .copilot/session-state). Zsh fork has no model.

1. Two turns (APPLE, PEAR) via `session type` (submitted:true both times, BUG-T1 fixed): PASS.
2. Empty fork (code-only): asked for remembered words, answered NONE: PASS.
3. Fork before turn 2 (code-and-context): answered APPLE only: PASS (F1 fixed).
4. Fork from now: answered "APPLE PEAR": PASS.
5a/5b. codex, claude: DEFERRED (owner).
5c. zsh fork (code-only, --agent zsh): created, running; `type "echo hi"` submitted: PASS.
5d. `fork --channel other` (different channel) refused with pointer to `session continue --channel other`: PASS (VF-46). `--channel` naming the parent's own channel is accepted (fine).
6. Stop and resume the before-2 fork: it resumed, and on screen answered APPLE to a new question: PASS. See finding N1 on the mirror.
7. Model carried: PASS (all sessions claude-haiku-5.5 in Copilot's records).

| flow | result |
|---|---|
| 1 two turns | PASS |
| 2 empty fork | PASS |
| 3 before turn 2 | PASS |
| 4 from now | PASS |
| 5c zsh | PASS |
| 5d fork --channel refused | PASS |
| 6 stop/resume | PASS (N1) |
| 7 model carried | PASS |
| 5a/5b codex, claude | DEFERRED |

Findings (new)
- N1 (low): after stop + resume of a fork, the new turn's user prompt reached the mirror (`session read` turn 3 user) but the agent's answer ("APPLE", visible on the screen) was still absent from `session read` ~60 s later. Steps: fork before turn 2, stop, resume, `session type`, then compare screen.sh with `session read`. Expected both turns; actual the agent entry missing.
- N2 (low): `humata session continue` has no `--title`, while fork does; not a bug, just asymmetric.

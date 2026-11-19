# Claude Code and Codex pass the Copilot cases, and Humata reads each through its own structured channel

Layer: agents. Platforms: macOS (This Mac; the remote cases use the `h2-terminal-c1` VM). Web is not covered.

This case runs the agent cases that VERIFY ran with Copilot (`results/copilot-*.md`) once per agent kind. `AGENT` is one of `copilot`, `claude` and `codex`. Copilot is the baseline, and the other two must match it (AGENTS2, D795–D799). Write results to `results/<AGENT>-<lane>.md`, in the same shape as the Copilot files.

## Starting state

- This Mac is the host. Channel `Payments` has this Mac, and repo `acme/api`.
- The kind under test is on in Settings › Agents. Its CLI is already installed: Humata never installs an agent CLI here.
- Models: Claude Code on Claude Haiku 5.5 (low effort), Codex on its cheapest model, Copilot on Claude Haiku 5.5.
- The agents run in their own scratch homes (`HOME`, `CLAUDE_CONFIG_DIR`, `CODEX_HOME`). Nothing reads or writes `~/.claude`, `~/.codex` or `~/.copilot`. Copying a login into a scratch home needs the owner's say-so at the time.

## Reproduce headless

- Structured channels, in process with recorded payloads (no CLI runs):
  - `bazel test //apps/humata2/daemon/agent_hooks:conversations_test //apps/humata2/daemon/agent_hooks:questions_test //apps/humata2/daemon/agent_hooks:agent_questions_test`: Claude's SessionStart (every source) and Codex's thread name the conversation, a subagent's payload names none, and AskUserQuestion and requestUserInput round-trip.
  - `bazel test //apps/humata2/daemon/codex:events_test //apps/humata2/daemon/codex:link_test //apps/humata2/daemon/codex:wrapper_test`: the app-server link (handshake, `/new`, turns, approvals, questions, withdrawal, reconnect) and the `humata codex` wrapper (private socket, `--remote`, exit status, the hooks fallback).
  - `bazel test //apps/humata2/cli:codex_test`: `humata codex` against a fake daemon and a fake app-server: a turn, the conversation, an approval and a question each reach the daemon, and the answer reaches Codex.
- The daemon, with a script agent standing in for each CLI:
  - `bazel test //apps/humata2/daemon/humatad:conversation_follows_test`: `/clear` (Claude) and `/new` (Codex), then Stop and Resume, take up the newest conversation, as Copilot's case does.
  - `bazel test //apps/humata2/daemon/humatad:ready_gate_test`: the first message waits for the structured signal, the 20 s "waiting in its terminal" card and its "Send it now", and the screen fallback staying quiet for a heard session.
- Not headless: the real CLIs with a login, and the drive on the owner's screen. The live checks O1–O3 in `workflow/milestones/AGENTS2.md` › Owed belong here.

## Steps

Run each Copilot case below with `--agent $AGENT`. The expected result is what the Copilot result file records, plus any fix made since.

| Copilot result | Cases to run per kind |
|---|---|
| `copilot-e1.md`, `copilot-e3.md` | 1 Stop and resume (agent half), 2 End for good, 3 Keep running |
| `copilot-e2.md` | 4 Discovery and adopt, 5 Ask in the Inbox switch, 6 Inbox terminal |
| `copilot-t1.md` | 1 start `--message`, 2 type, 3 no-submit and Enter (This Mac and the VM) |
| `copilot-remote.md` | 1 trust card, 2 held start message typed once, 3 unsent start message reaches the host Inbox |
| `copilot-fork.md` | 1 two turns, 2 empty fork, 3 before turn 2, 4 from now |

Then the cases only these two kinds add:

1. **I do:** start a session, wait for the agent's trust screen, and send a composer message before answering it.  
   **I see:** the message waits, and nothing is typed into the trust screen. After 20 s the Inbox shows "<Agent> is waiting in its terminal" with the held message and "Send it now". When I answer the trust screen, the card goes away and the message is sent once.  
   **State and events:** the signal is a `conversation` hook: Claude's SessionStart, or the wrapper's `thread/started` for Codex (D797).

2. **I do:** have two turns, then `/clear` (Claude) or `/new` (Codex), have one more turn, then Stop and Resume.  
   **I see:** the resumed agent has the conversation after `/clear` or `/new`, not the first one.  
   **State and events:** the session's conversation id changes at the `conversation` hook (D799). For Codex, the rollout is found by its id in its day folder.

3. **I do:** ask the agent to run a command that needs approval, then to ask me a multiple-choice question.  
   **I see:** both arrive in the Inbox. Answering there lets the command run and gives the agent the picked option. Answering in the terminal first withdraws the card.  
   **State and events:** the cards come from the Hook stream (PermissionRequest or AskUserQuestion for Claude, the app-server's requestApproval or requestUserInput for Codex), not from `screen_ask`. The screen fallback raises nothing for a heard session (D798).

4. **I do (Codex only):** run with a Codex whose app-server can't start (for example, an older one).  
   **I see:** Codex still runs in the tab. Turns and approvals come through its hooks once they are trusted in its `/hooks`.  
   **State and events:** the wrapper runs plain `codex -c hooks=…` (D795).

# AGENTS2: Claude Code and Codex are first-class agents, read through their own structured channels

- **Status:** next (owner, 2026-11-18: "after ur current roadmap is done begin working on claude code and codex integration")
- **Depends on:** RELEASE3: starts after 0.3 ships
- **Unblocks:** nothing yet
- **Layer:** L13 · **Lane:** daemon + agent_hooks + clients + UI

## Outcome
Claude Code and Codex do everything Copilot does in Humata 2: start, resume,
stop, fork and move, End for good, their own questions in the Inbox, and
conversation tracking that survives `/clear` (Claude Code) and `/new` (Codex).
Each is opt-in per agent in Settings › Agents (AGENTSET, D720–729).

The owner's direction: "I don't like the fact you parse the terminal screen.
Not saying to remove it but just keep that in mind." So Humata reads each
agent's state and questions through the agent's own structured channel first:
- hooks: Claude Code's hooks, Copilot's plugin hooks;
- JSON or stream output;
- Codex's app-server protocol;
- session files.

Screen parsing (`screen_ask`, D786) stays only as a fallback.

## Exit criterion
- With each agent switched on, the VERIFY Copilot cases pass with Claude Code and with Codex in place of Copilot.
- `/clear` (Claude Code) and `/new` (Codex), followed by Stop and Resume, resume the newest conversation. This mirrors the Copilot test in `daemon/humatad/tests/conversation_follows.rs`.
- An approval the agent asks for arrives in the Inbox from its structured channel, not from the screen.
- The fast tier passes. A live drive on the owner's screen shows each agent end to end, with screenshots.

## Read these (pointers, nothing else)
- `daemon/agent_hooks/src/lib.rs`:
  - `parse_conversation`;
  - the Copilot plugin manifest, which shows how a hook reports `conversation`.
- `daemon/sessions/src/agents.rs`: `Agents::follow_conversation`.
- `daemon/humatad/src/session_service/hooks.rs`.
- AGENTKINDS.md (D670–675) and HOOKS.md for the existing kinds and hooks.
- D786 (`screen_ask`) for the fallback being replaced.
- Memory: never install an agent CLI unless that agent's opt-in is on; never modify `~/.claude`, `~/.codex` or `~/.copilot`.

## Out of scope
- Removing screen parsing.
- Agents other than Claude Code, Codex and Copilot.

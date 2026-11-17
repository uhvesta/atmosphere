# Brief agents-17: Queueing a message for turn end
Output: `apps/humata2/flows/cases/agents/17-queueing-a-message-for-turn-end.md`

Scenario: while an agent is mid-turn I type a follow-up and send it; it should wait until the turn ends. Another agent sends a message into this session with "when the turn ends". I look at the queue, send one now, cancel another.

Steps: the app's queue (features/session queue tests): fallback and plain shell messages wait for an explicit send; a hook turn-end delivers waiting messages but running and uncertain do not; offline send preserves the queue; a failed message returns to its session.

Branches: the daemon side: TerminalService.Type with when TURN_ENDS, ListQueued, SendQueuedNow, CancelQueuedTyping answer not_built (daemon/humatad/src/terminal.rs ~912, 1193–1221). So the app queue lives only in the app (fakes). Say what is lost across an app restart. The MCP `session_type` result has `queued_id` and `turn_end_uncertain` fields — report whether they can ever be set.

Pointers: features/session/tests/queue.rs (4), desktop/tests/native/queue.rs, daemon/humatad/src/terminal.rs, daemon/humatad/src/mcp.rs session_type, `humata turn` (cli/src/commands/turn.rs).

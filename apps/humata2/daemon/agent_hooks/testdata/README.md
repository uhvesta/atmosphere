Hook payloads each agent writes to its hook's stdin, as their docs give them
(HOOKS-research.md, "Verified from primary sources"). Replace one with a
live capture when it is taken; keep the file name.

`copilot/ask_user_*.json` are Copilot's `preToolUse` payload for its
`ask_user` form (D761). The arguments
are the ones Copilot 1.0.113 recorded in its own events.jsonl
(`tool.execution_start` of `ask_user`: `message` and an elicitation
`requestedSchema`); `toolArgs` is an object in one and a JSON string in the
other, since the docs give a string and the live postToolUse an object.

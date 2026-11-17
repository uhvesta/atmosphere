# Brief agents-13: An agent on a stopped machine
Output: `apps/humata2/flows/cases/agents/13-working-on-a-stopped-machine.md`

Scenario: the user stopped this Mac's sessions while an agent had a pending Inbox request and a script was polling. What do the agent, the script, and a terminal reply waiting on its holder see; what happens on Start.

Steps: the agent's process ends; its pending request is withdrawn as stopped with its machine; scripts are refused; nothing can be typed, attached or started; a daemon killed and restarted while stopped stays stopped; Start brings sessions back with a report naming the stop; the agent resumes its conversation.

Branches: a stop finishes while a terminal reply waits on its holder; under the default rule an agent cannot start the machine; PR reviews stop following; a box stop (stop_box.rs).

Pointers: daemon/humatad/tests/stop_sessions.rs, stop_box.rs, stop_access.rs, pr_stopped.rs, cli/src/commands/terminal_replies.rs.

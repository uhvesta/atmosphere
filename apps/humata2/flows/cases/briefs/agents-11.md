# Brief agents-11: An agent tries to stop the machine
Output: `apps/humata2/flows/cases/agents/11-trying-to-stop-the-machine.md`

Scenario: an agent decides the machine should be stopped and runs `humata stop`. Under the default rule it is denied; I set the rule to Ask and it lands in the Inbox; I set it to Allow and it stops the machine (itself included) and is named as the stopper. It then tries `humata start`.

Steps and branches: default rule denies at once with a fix (exit 4); Ask puts it in the Inbox and stops nothing meanwhile; Allow stops, named as stopper; it still can't start the user's stop; a script token lacking the capability is refused; `humata stop` typed outside every session with no login stops the machine; inside a session without its token is refused; an unusable session token is refused, not taken as a terminal; an agent cannot stop another session (MCP).

Pointers: daemon/humatad/tests/stop_access.rs (12), mcp.rs `an_agent_cannot_stop_another_session`, cli/src/commands/mod.rs Command::Stop.

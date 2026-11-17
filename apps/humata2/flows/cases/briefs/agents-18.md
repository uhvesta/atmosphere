# Brief agents-18: Scripts with a token
Output: `apps/humata2/flows/cases/agents/18-scripts-with-a-token.md`

Scenario: I write a cron script that lists sessions every hour and pins the ones that need me. I mint a script token (`humata token ...`), export HUMATA_TOKEN, and run it; later it tries to stop the machine.

Steps: token minting and its capabilities; whoami says script; the agent stays the agent with a script token too; a capability missing is refused; a stop without the capability is refused and nothing stops.

Branches: SEC-14 script tokens (OVERVIEW lists "script tokens (SEC-14)" as a gap — check what `humata token` does in cli/src/commands/inbox.rs `token` and on the daemon); token revoked; token used on another machine.

Pointers: cli/tests/actor.rs (`a_script_token_makes_a_script`, `the_agent_stays_the_agent_with_a_script_token_too`, `whoami_says_script_for_a_script_token`), daemon/humatad/tests/stop_access.rs `a_script_whose_token_lacks_the_capability_is_refused_a_stop_and_nothing_stops`, daemon/humatad/tests/auth.rs, SPEC SEC-14.

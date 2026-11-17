# Brief agents-01: Who am I, and who may act
Output: `apps/humata2/flows/cases/agents/01-who-am-i-and-who-may-act.md`

Scenario: the same `humata whoami` and `humata ls` are run by four callers: an agent inside a session (session token in its env), a script with HUMATA_TOKEN, me in a plain terminal after `humata login`, and me in a plain terminal with no login. Then an agent runs with a token the daemon never gave out.

Steps: whoami names the session for an agent; "script" for a script token; "user" for the person; with no one only whoami and login work; a login lasts up to 12 h in this terminal only and never prints its token; an agent's token wins over the terminal's login; logout forgets it.

Branches: an unknown token is refused; a closed tab's token stops working; a tab that outlives a restart keeps its token; HUMATA_SOCKET / HUMATA_ROOT choose the socket; no socket lists every place tried; MCP whoami (daemon/humatad/tests/mcp.rs).

Pointers: cli/tests/actor.rs, cli/tests/login.rs, daemon/humatad/tests/login.rs, cli_login.rs, auth.rs, mcp.rs (`whoami_names_the_session_the_token_belongs_to`, `a_token_the_daemon_never_gave_out_is_refused`, `a_closed_tabs_token_stops_working`, `a_tab_that_outlives_a_restart_keeps_its_token`), api/CLI.md principals section.

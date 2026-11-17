# Each caller learns who it is, and only the right credential acts

Layer: agents. Platforms: macOS, Linux.

## Starting state

- Humata 2 runs on this Mac. The channel `acme/api` has one session, `Fix login`, with a running agent tab.
- The agent tab was given `HUMATA_SESSION_TOKEN` when it opened. The token is never printed.
- A script token for the channel exists. Terminal T1 has it in `HUMATA_TOKEN`, and no session token.
- Terminal T2 is a plain terminal of this Mac user, with no saved login and no `HUMATA_*` variables. `HUMATA_SOCKET` and `HUMATA_ROOT` are unset. Nothing is held.

## Steps

1. **I do:** In the agent tab, run `humata --json whoami`.

   **I see:** `tier` is `agent`. The actor is an agent that names this session and its channel, by id.

   **State and events:** The daemon checks the session token against the tab's process tree (SEC-8, SEC-15). No Inbox event.

   **Status:** Works (real daemon). `daemon/humatad/tests/auth.rs::a_session_token_used_outside_its_tab_is_refused_and_works_inside_it` runs `humata whoami` inside the tab and expects tier `agent`. The MCP form is `daemon/humatad/tests/mcp.rs::whoami_names_the_session_the_token_belongs_to`.

2. **I do:** In the agent tab, run `humata --json ls`.

   **I see:** The sessions of `acme/api`, this one among them.

   **State and events:** The daemon answers for the agent's channel only (CLI.md §2).

   **Status:** Code, no test. No test in `daemon/humatad/tests/` runs `ls` as an agent. The matrix runs `channels` and `topology` as an agent.

3. **I do:** In T1, run `humata --json whoami`.

   **I see:** `tier` is `script`. The daemon does not name it an agent.

   **State and events:** The call carries the script token in `humata-script-token`, and the daemon judges it by that token's capabilities (SEC-14).

   **Status:** Fakes only. `cli/tests/actor.rs::whoami_says_script_for_a_script_token` uses a fake daemon. The real daemon judges script calls in `daemon/humatad/tests/matrix.rs::the_matrix_holds_for_a_script_token`, but that test runs `topology`, not `whoami`.

4. **I do:** In T2, run `humata --json whoami`.

   **I see:** `tier` is `user`, with no actor and `presence_possible` false.

   **State and events:** No credential is sent. The release daemon answers WhoAmI with no actor (SEC-9), and answers nothing else.

   **Status:** Works (real daemon). `daemon/humatad/tests/auth.rs::whoami_with_no_credential_answers_with_no_actor`. `daemon/humatad/tests/cli_login.rs::a_confirmed_login_makes_its_terminal_the_user_until_logout` runs the real `humata whoami` in a pty before login.

5. **I do:** In T2, run `humata --json logins`, a user-only call.

   **I see:** Exit 5, which means the person must act. The error kind is `unauthenticated` or `presence_required`.

   **State and events:** The daemon refuses the call. Nothing is listed.

   **Status:** Works (real daemon). `daemon/humatad/tests/cli_login.rs::a_confirmed_login_makes_its_terminal_the_user_until_logout` asserts exit 5 before login.

6. **I do:** In T2, run `humata login` and the person confirms the request in the app.

   **I see:** A login that lasts 12 hours. The token is not printed.

   **State and events:** The token is saved for this terminal session only, in `~/.humata2/cli-logins/<session>` (folder 0700, file 0600, `cli/src/login.rs`). The daemon binds the login to this terminal's process tree (SEC-11).

   **Status:** Works (real daemon). `daemon/humatad/tests/cli_login.rs::a_confirmed_login_makes_its_terminal_the_user_until_logout` checks the token text is not in the JSON. The file modes are checked only by the unit test `cli/src/login.rs::a_saved_login_reads_back_and_only_the_user_can_read_it` (Fakes only).

7. **I do:** In T2, run `humata --json whoami` again, then `humata logout`, then `whoami` once more.

   **I see:** After login, `tier` is `user` with `presence_possible` true. After logout, it is `user` with `presence_possible` false, and the saved file is gone.

   **State and events:** The login is sent as `humata-cli-login`. Logout ends the login in the daemon and deletes the file.

   **Status:** Works (real daemon). `daemon/humatad/tests/cli_login.rs::a_confirmed_login_makes_its_terminal_the_user_until_logout`.

8. **I do:** In T2, run `humata login --hours 13`.

   **I see:** Refused with `invalid_field`: a login lasts at most 12 hours, not 13. The fix says to ask for 12 or fewer.

   **State and events:** The daemon refuses before it asks the app (`daemon/humatad/src/cli_login.rs:367-377`).

   **Status:** Code, no test. The default of 12 is `cli/src/args.rs:171`. The only hours test uses 3 hours, with a fake daemon (`cli/tests/login.rs::a_login_is_kept_for_its_terminal_session_and_its_token_never_printed`).

9. **I do:** In a terminal with a saved login and `HUMATA_SESSION_TOKEN` also set, run `humata --json whoami`.

   **I see:** `tier` is `agent`. The login is not used for this call.

   **State and events:** A session token wins over a script token, and both win over a login (`cli/src/principal.rs`, `principal()`).

   **Status:** Fakes only. `cli/tests/login.rs::an_agents_token_wins_over_the_terminals_login` uses a fake daemon, and `cli/tests/actor.rs::the_agent_stays_the_agent_with_a_script_token_too` is a pure unit test. No real-daemon run mixes the two.

## Branches

### An unknown token is refused

**When:** A process sends a 32-hex-character token that the daemon never gave out.

**I do:** Call MCP `whoami` with that token.

**I see:** The gRPC call fails as unauthenticated. The MCP tool reports `identity_mismatch`.

**Expected:** No actor is named. Nothing runs as the agent.

**Status:** Works (real daemon). `daemon/humatad/tests/mcp.rs::a_token_the_daemon_never_gave_out_is_refused`.

### A session token used outside its tab is refused, and revoked

**When:** An agent's token is copied to a shell outside the tab's process tree.

**I do:** Run `humata whoami` there with the copied token.

**I see:** An identity mismatch, with a fix. The session's own shell is also refused until the session resumes and gets a new token.

**Status:** Works (real daemon). `daemon/humatad/tests/auth.rs::a_session_token_used_outside_its_tab_is_refused_and_works_inside_it`.

### A closed tab's token stops working, and a restart keeps a live tab's token

**When:** The user closes the agent's tab, or the daemon restarts while the tab's holder keeps the shell alive.

**I see:** A closed tab's token gets Unauthenticated. After a restart, the same token still names the same agent and session.

**Status:** Works (real daemon). `daemon/humatad/tests/mcp.rs::a_closed_tabs_token_stops_working` (gRPC WhoAmI, not the CLI) and `daemon/humatad/tests/mcp.rs::a_tab_that_outlives_a_restart_keeps_its_token`. `daemon/humatad/tests/auth.rs::a_session_token_works_on_the_release_daemon` also restarts, as the release build.

### A script token that is revoked or expired is refused

**When:** The user revokes the script token, or its lifetime ends.

**I do:** Run `humata topology --channel acme/api` as the script.

**I see:** Refused. The Inbox gets no request.

**Status:** Works (real daemon). `daemon/humatad/tests/matrix.rs::a_revoked_token_stops_at_once_and_a_kept_one_survives_a_restart` and `daemon/humatad/tests/matrix.rs::an_expired_token_is_refused`.

### A saved login is read from another terminal

**When:** The saved login file is copied to another terminal session, or the session id is reused.

**I do:** Run `humata logins` there.

**I see:** Exit 5. Nothing is listed.

**Status:** Works (real daemon). `daemon/humatad/tests/cli_login.rs::a_logins_token_is_refused_outside_its_terminal_session` and `daemon/humatad/tests/cli_login.rs::a_saved_login_is_refused_in_another_terminal`.

### A login expires, is denied, or is asked for in a session's tab

**When:** The lifetime passes (the test uses `--cli-login-seconds=1`), the person denies the request, or someone runs `humata login` in an agent's tab.

**I see:** After expiry, `humata logins` exits 5, and logout forgets the file. A denied login saves nothing and exits 4. A session's tab gets no login, because anything typed there may come from its agent.

**Status:** Works (real daemon). `daemon/humatad/tests/cli_login.rs::a_login_ends_when_it_expires`, `daemon/humatad/tests/cli_login.rs::a_login_denied_in_the_app_leaves_the_terminal_as_it_was` and `daemon/humatad/tests/cli_login.rs::a_sessions_tab_cant_log_in`.

### Logout with no saved login asks the daemon nothing

**I do:** Run `humata logout` in T2, which has no login.

**I see:** "This terminal isn't logged in." No daemon call is made.

**Status:** Fakes only. `cli/tests/login.rs::logout_without_a_login_asks_nothing_of_the_daemon`.

### The socket is chosen by `HUMATA_SOCKET`, `HUMATA_ROOT` or the defaults

**When:** `HUMATA_SOCKET` is set, `HUMATA_ROOT` moves this machine's socket, or no socket exists.

**I do:** Run `humata whoami` in each case.

**I see:** With `HUMATA_SOCKET`, only that socket is tried. Otherwise the Mac's socket is tried first, then this machine's socket under its root, then the default under the home folder. If none exists, the error lists every place tried.

**Status:** Fakes only. `cli/tests/actor.rs::humata_socket_is_the_only_socket_tried`, `cli/tests/actor.rs::humata_root_moves_this_machines_socket`, `cli/tests/actor.rs::the_macs_socket_comes_before_this_machines` and `cli/tests/actor.rs::no_socket_lists_every_place_tried`. These test the pure functions in `cli/src/socket.rs`.

## Expected outcomes

- [ ] An agent's `whoami` names its session and channel (tier `agent`). A script's reports tier `script`.
- [ ] With no credential, `whoami` reports the user with no actor, and each user-only call exits 5.
- [ ] `humata login` makes the terminal the user for the login's lifetime, and never prints the token. `logout` ends it.
- [ ] A login is refused in another terminal session, from a session's tab, and after it expires.
- [ ] A token the daemon never gave out, or a session token used outside its tab, is refused. The latter is revoked.
- [ ] A closed tab's token stops working. A token survives a restart while its tab does.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Agent runs `whoami` | Works (real daemon) | `daemon/humatad/tests/auth.rs::a_session_token_used_outside_its_tab_is_refused_and_works_inside_it` |
| 2. Agent runs `ls` | Code, no test | `daemon/humatad/src/mcp.rs` (no test) |
| 3. Script runs `whoami` | Fakes only | `cli/tests/actor.rs::whoami_says_script_for_a_script_token` |
| 4. Plain terminal runs `whoami` | Works (real daemon) | `daemon/humatad/tests/auth.rs::whoami_with_no_credential_answers_with_no_actor` |
| 5. User-only call refused | Works (real daemon) | `daemon/humatad/tests/cli_login.rs::a_confirmed_login_makes_its_terminal_the_user_until_logout` |
| 6. `humata login` and confirm | Works (real daemon) | `daemon/humatad/tests/cli_login.rs::a_confirmed_login_makes_its_terminal_the_user_until_logout` |
| 7. Whoami and logout | Works (real daemon) | `daemon/humatad/tests/cli_login.rs::a_confirmed_login_makes_its_terminal_the_user_until_logout` |
| 8. `login --hours 13` refused | Code, no test | `daemon/humatad/src/cli_login.rs:367-377` |
| 9. Agent token beats a login | Fakes only | `cli/tests/login.rs::an_agents_token_wins_over_the_terminals_login` |
| Branch: unknown token | Works (real daemon) | `daemon/humatad/tests/mcp.rs::a_token_the_daemon_never_gave_out_is_refused` |
| Branch: token used outside its tab | Works (real daemon) | `daemon/humatad/tests/auth.rs::a_session_token_used_outside_its_tab_is_refused_and_works_inside_it` |
| Branch: script token revoked or expired | Works (real daemon) | `daemon/humatad/tests/matrix.rs::a_revoked_token_stops_at_once_and_a_kept_one_survives_a_restart` |
| Branch: login used from another terminal | Works (real daemon) | `daemon/humatad/tests/cli_login.rs::a_logins_token_is_refused_outside_its_terminal_session` |
| Branch: closed tab, restart, login expiry, login denied, tab cannot log in | Works (real daemon) | `daemon/humatad/tests/mcp.rs::a_closed_tabs_token_stops_working`; `daemon/humatad/tests/cli_login.rs::a_login_ends_when_it_expires` |
| Branch: logout with no login | Fakes only | `cli/tests/login.rs::logout_without_a_login_asks_nothing_of_the_daemon` |
| Branch: socket choice | Fakes only | `cli/tests/actor.rs::no_socket_lists_every_place_tried` |

## Gaps and likely bugs

- Code, no test: `humata ls` as an agent (step 2), and the 12-hour cap (step 8).
- Fakes only: the script identity in the CLI (step 3), the agent-beats-login rule (step 9), and the socket choice. None runs against a real daemon.
- Not built: nothing in this case is missing as a feature.
- Likely bug: none found. Logout as an agent or script deletes the saved file without ending the daemon's login (`cli/src/commands/login.rs`, `logout`). The token is gone from disk, but the daemon keeps the login until it expires. The code comment says this is by design; confirm the intent before relying on it.

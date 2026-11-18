# A script with its own token sees and does only what the token lists

Layer: agents. Platforms: macOS, Linux, web.

## Starting state

- Humata 2 runs on this Mac. Its daemon holds channel `Payments`, with sessions `ledger` (Claude, Needs you) and `brin` (Claude, Working).
- Box `box-1` runs its own `humatad2` with its own store. It has no tokens from this Mac.
- I am in a plain terminal on this Mac, outside every session tab. I ran `humata login` here, so this terminal holds a CLI login. No `HUMATA_SESSION_TOKEN` is set.
- My cron script lives in my scripts folder. It runs `humata` with `HUMATA_TOKEN` set in its environment. The script has no session of its own.
- Nothing is stopped. No script token exists yet.

## Steps

1. **I do:** Run `humata --json token create --name hourly-triage --capability channel_read --channel Payments --days 30`.

   **I see:** JSON with a `token_id` and a `token`. The clear token is shown now and never again.

   **State and events:** The daemon checks that the caller is the user (`daemon/humatad/src/cli_login.rs:489-493`). It stores a hash of the token, its name, capabilities, channels and expiry. The clear text is never saved (`daemon/humatad/src/script_tokens.rs`, test `the_clear_token_is_never_saved`). The CLI sends the request with no presence proof, and the daemon never asks for one (see the branch on presence).

   **Status:** Code, no test for the CLI command. The daemon side of a mint is exercised through gRPC by `daemon/humatad/tests/matrix.rs::the_matrix_holds_for_a_script_token`, which creates its tokens with the test helper.

2. **I do:** `export HUMATA_TOKEN=<token>`, then run `humata --json whoami`.

   **I see:** The result has `tier: "script"`. The daemon's reply carries a script actor with the name `hourly-triage`.

   **State and events:** The CLI picks the script principal, since no session token is set (`cli/src/principal.rs:86-88`). It sends the token in the `humata-script-token` header. The daemon resolves a `Caller::Script` (`daemon/humatad/src/identity.rs:951-954`) and answers with `Actor::Script` (`identity.rs:395-397`). The CLI sets `tier` from its own principal (`cli/src/commands/looking.rs:44`), not from the daemon's answer.

   **Status:** Fakes only. `cli/tests/actor.rs::whoami_says_script_for_a_script_token` runs against a fake daemon, so the tier it checks comes from the CLI. The daemon's script actor in the reply has no test.

3. **I do:** Run `humata --json ls --channel Payments`.

   **I see:** The two sessions in `Payments`.

   **State and events:** The daemon lists the sessions the caller may read. Each session is checked as `Call::ReadSession`, which needs `channel_read` on that session's channel (`daemon/access/src/policy.rs:1152-1198`).

   **Status:** Works (real daemon). `daemon/humatad/tests/matrix.rs::the_matrix_holds_for_a_script_token`, row "listed, its channel". That row runs `topology --channel`, which calls `GetChannel` and `ListSessions` (`cli/src/commands/looking.rs:225-231`). Plain `ls` is not a row of that matrix.

4. **I do:** Run `humata --json topology --channel <a channel the token does not list>`.

   **I see:** A refusal. The CLI exits 4 (denied) and says the token is not for that channel, with a fix that names `humata token create`.

   **State and events:** `ScriptTokenOtherChannel` (`daemon/access/src/verdict.rs:163-165`). Nothing is read.

   **Status:** Works (real daemon). `matrix.rs::the_matrix_holds_for_a_script_token`, row "listed, other channel".

5. **I do:** In the cron script, run `humata --json pin <session>` for a session that needs me.

   **I see:** The CLI refuses the call as user-only. It exits 5.

   **State and events:** Pinning needs `MarksChange`, which is user-only (`daemon/access/src/capability.rs`, the `UserOnly` group). The mark service refuses an agent or a script (`daemon/humatad/src/mark_service.rs:6`). So the script cannot pin. It can only read the list and report.

   **Status:** Code, no test. No test runs a script's pin. The matrix covers only "user-only token ls" and "user-only rule".

6. **I do:** The script runs `humata --json stop --reason "cron"`, with the token from step 1. That token lists no machine capability.

   **I see:** A refusal. The CLI exits 4 (denied). The message says the token does not list the capability and tells me to make a new token that lists it.

   **State and events:** The stop service sends a script to the gate (`daemon/humatad/src/stop/service.rs:184-189`). The policy refuses it as `ScriptTokenLacks` (`daemon/access/src/policy.rs:1187-1189`). No stop id is recorded, and no session stops.

   **Status:** Works (real daemon). `daemon/humatad/tests/stop_access.rs::a_script_whose_token_lacks_the_capability_is_refused_a_stop_and_nothing_stops` (gRPC `StopSessions`, `PermissionDenied`, and nothing stopped).

7. **I do:** Make a second token that lists the machine capability, with no channels: `humata --json token create --name ci-stop --capability machine_sessions_stop --days 1`. Run the script with it: `humata --json stop --reason "cron"`.

   **I see:** This machine's sessions stop. The stop record names the stopper as a script, with its token.

   **State and events:** `ByKind::Script` is recorded for a script (`daemon/humatad/src/stop/service.rs`), the proto's `ScriptActor` (`api/proto/humata/v1/common.proto`). The policy skips the channel check for a machine target (`daemon/access/src/policy.rs`; D523).

   **Status:** Works (real daemon): `daemon/humatad/tests/stop_access.rs::a_script_whose_token_lists_stopping_sessions_stops_the_machine_and_is_named_a_script`. (Was a likely bug: the stopper was an agent; fixed by BUGS.)

8. **I do:** Start a Humata session tab, so that the tab has `HUMATA_SESSION_TOKEN`. Run the same script there, with `HUMATA_TOKEN` also set.

   **I see:** The call acts as the session's agent, not as the script. The agent's rules apply.

   **State and events:** The CLI picks the agent principal and sends only the session token header (`cli/src/principal.rs:76-82`, `:110`). The daemon checks the session token first (`daemon/humatad/src/identity.rs:910-912`).

   **Status:** Fakes only. `cli/tests/actor.rs::the_agent_stays_the_agent_with_a_script_token_too` tests the CLI's choice as a pure function, with no daemon. No daemon test sends both headers.

9. **I do:** Run `humata token ls` from the script.

   **I see:** A refusal, exit 5 (user only).

   **State and events:** `user_only` refuses a script (`daemon/humatad/src/cli_login.rs:593-598`).

   **Status:** Works (real daemon). `matrix.rs::the_matrix_holds_for_a_script_token`, row "user-only token ls". The matrix also covers a script that tries `token create` (row in `a_token_cannot_be_made_for_a_user_only_action`).

## Branches

### The token is revoked

**When:** I run `humata token revoke <token_id>` from my logged-in terminal. The script runs again later.

**I do:** Revoke the token. Run `humata --json ls --channel Payments` from the script.

**I see:** The call is refused, exit 5. The message says the token "was revoked or made by another daemon", and the fix says to list this daemon's tokens with `humata token ls` and make a new one.

**State and events:** The revoke removes the token from the store and from memory (`daemon/humatad/src/script_tokens.rs:272-287`). A second revoke says the token does not exist. A token that was kept stays valid after a daemon restart.

**Expected outcome:** The revoked token stops at once. Another token keeps working, including after a restart.

**Status:** Works (real daemon). `daemon/humatad/tests/matrix.rs::a_revoked_token_stops_at_once_and_a_kept_one_survives_a_restart`. The CLI command `humata token revoke` has no test of its own, so it is Code, no test for that part.

### The token has expired

**When:** The token's `--days` have run out. The test daemon's `--script-token-seconds` makes this quick.

**I do:** Run the script.

**I see:** A refusal, exit 5. The message says a new token is needed, because an expired token cannot be renewed.

**State and events:** `ScriptTokenUnusable::Expired` (`daemon/humatad/src/script_tokens.rs:127-137`). The token stays listed until it is revoked (D132).

**Expected outcome:** The call is refused. The token stays in the list until I revoke it.

**Status:** Works (real daemon). `daemon/humatad/tests/matrix.rs::an_expired_token_is_refused`.

### The token is used on another machine

**When:** I copy the token to a script on `box-1`, and run `humata` there with `HUMATA_TOKEN` set.

**I do:** Run `humata --json ls` on `box-1`.

**I see:** A refusal, exit 5. The message says the token was revoked or made by another daemon, and the fix says to make a new token on this daemon.

**State and events:** Each daemon loads only its own store. `box-1` does not have the token, so it answers `Unknown` (`daemon/humatad/src/identity.rs:951`, `status.rs:255-257`). No code sends the script-token header to another machine. The only senders are the CLI and the daemon's test helper.

**Expected outcome:** Refused. The token does not work on any machine but the one that made it.

**Status:** Code, no test. `daemon/humatad/src/script_tokens.rs::a_made_up_token_is_unknown` is an in-process test of the same `Unknown` answer. No test runs two daemons.

### The token lists a user-only capability

**When:** I try to make a token that holds `access_change` or `peer_report`.

**I do:** Run `humata token create --name x --capability access_change`.

**I see:** A refusal that names the capability field, and says a script token cannot hold that action.

**State and events:** The daemon refuses the capability (`daemon/humatad/src/cli_login.rs:507-530`). No token is made.

**Expected outcome:** Refused, with the field named. The user-only capability is never saved in a token.

**Status:** Works (real daemon). `daemon/humatad/tests/matrix.rs::a_token_cannot_be_made_for_a_user_only_action`.

### Presence is not asked when I make or revoke a token

**When:** I run `humata token create` or `humata token revoke` from a logged-in terminal, with the app closed.

**I do:** Run the command.

**I see:** The token is made, with no prompt to confirm in the app and no approval passphrase.

**State and events:** The CLI's `present` sends the request first with no proof, and builds a proof only when the daemon answers `PresenceRequired` (`cli/src/call.rs:185-199`). The daemon never answers that for `CreateScriptToken` or `RevokeScriptToken`. Its handlers check only `user_only` (`daemon/humatad/src/cli_login.rs:489-493`, `:603-607`). The request's `presence` field is read nowhere for these two methods.

**Expected outcome:** If presence is meant to gate tokens, as the `presence` field and the CLI's `present_json` suggest, the command asks for confirmation before it makes or revokes a token.

**Status:** Code, no test. Owner decision, Provisional D522: making or revoking a token needs the user (the app, or a terminal whose login the user confirmed), with no second presence step. No test runs an agent's or a script's refused create.

### Token listed for machine stop but no channels

**When:** A token lists `machine_sessions_stop` and no channels, and runs `humata stop`.

**I do:** Run the stop.

**I see:** This machine's sessions stop.

**State and events:** `check_script` skips the channel check when the target is a machine (`daemon/access/src/policy.rs:1190`). A token's channel list does not limit a machine stop.

**Expected outcome:** SEC-14 says a token lists "which channels". The owner should say whether a machine stop should also be limited by the channels, or whether the list only limits channel actions.

**Status:** Works (real daemon): `daemon/humatad/tests/stop_access.rs::a_script_whose_token_lists_stopping_sessions_stops_the_machine_and_is_named_a_script` stops the machine with a token that lists no channels. Owner decision, Provisional D523: a token's channels limit channel actions; a machine stop is limited by its capability alone.

## Expected outcomes

- [ ] Making a token shows its clear text once. The store keeps only a hash.
- [ ] `humata whoami` with `HUMATA_TOKEN` reports a script, and the daemon knows the script by its token id and name.
- [ ] A script lists and reads only the sessions in its own channels.
- [ ] A script cannot pin, read the token list, make a token, or revoke one.
- [ ] A script without the machine capability is refused a stop, and nothing stops.
- [x] A script with the machine capability stops the machine, and the stop names a script (`daemon/humatad/tests/stop_access.rs::a_script_whose_token_lists_stopping_sessions_stops_the_machine_and_is_named_a_script`).
- [ ] A session tab's agent identity wins over the script token, in the daemon as well as in the CLI.
- [ ] A revoked token stops at once. A kept token survives a restart.
- [ ] An expired token is refused and says a new one is needed.
- [ ] A token made on one daemon is refused by another.
- [ ] Making or revoking a token needs the user, with no second confirmation (Provisional D522; no test).

## Status summary

| Step or branch | Status | Evidence |
| --- | --- | --- |
| 1. `token create` (CLI) | Code, no test | `cli/src/commands/inbox.rs:373-400`; daemon mint via `matrix.rs::the_matrix_holds_for_a_script_token` |
| 2. `whoami` with a script token | Fakes only | `cli/tests/actor.rs::whoami_says_script_for_a_script_token`; daemon script actor has no test |
| 3. List sessions in own channel | Works (real daemon) | `daemon/humatad/tests/matrix.rs::the_matrix_holds_for_a_script_token` (topology row) |
| 4. Read a channel not listed | Works (real daemon) | `matrix.rs::the_matrix_holds_for_a_script_token` (other channel row) |
| 5. Pin as a script | Code, no test | `daemon/humatad/src/mark_service.rs:6`; `capability.rs` UserOnly group |
| 6. Stop without the capability | Works (real daemon) | `daemon/humatad/tests/stop_access.rs::a_script_whose_token_lacks_the_capability_is_refused_a_stop_and_nothing_stops` |
| 7. Stop with the capability | Works (real daemon) | `daemon/humatad/tests/stop_access.rs::a_script_whose_token_lists_stopping_sessions_stops_the_machine_and_is_named_a_script` |
| 8. Session tab wins over script token | Fakes only | `cli/tests/actor.rs::the_agent_stays_the_agent_with_a_script_token_too`; daemon `identity.rs:910-912` has no test |
| 9. `token ls` as a script | Works (real daemon) | `matrix.rs::the_matrix_holds_for_a_script_token` (user-only token ls row) |
| Branch: revoked | Works (real daemon) | `matrix.rs::a_revoked_token_stops_at_once_and_a_kept_one_survives_a_restart` |
| Branch: expired | Works (real daemon) | `matrix.rs::an_expired_token_is_refused` |
| Branch: another machine | Code, no test | `script_tokens.rs::a_made_up_token_is_unknown` (in-process only) |
| Branch: user-only capability in a token | Works (real daemon) | `matrix.rs::a_token_cannot_be_made_for_a_user_only_action` |
| Branch: presence on create or revoke | Code, no test (owner decision, Provisional D522) | `daemon/humatad/src/cli_login.rs` (`create_script_token`, `revoke_script_token`: `user_only`) |
| Branch: channels and machine stop | Works (real daemon) (as Provisional D523 picks) | `daemon/humatad/tests/stop_access.rs::a_script_whose_token_lists_stopping_sessions_stops_the_machine_and_is_named_a_script`; `daemon/access/src/policy.rs` |

## Gaps and likely bugs

**Not built**

- Settings › Security, where the user lists and revokes script tokens (ST-17, SEC-14). No app code exists in `ui/` or `features/`. The ASK milestone left it out of its exit (`workflow/milestones/ASK.md:133`). No milestone is named for it. Today the user does this from the CLI.
- The OVERVIEW line "script tokens (SEC-14)" (`OVERVIEW.md:576`) is stale for the daemon side. ASK batch 4 (commit `21cbef1d`) built the daemon side. The UI half is still open.

**Likely bugs**

- Fixed (was a likely bug): a script's machine stop names a script (`daemon/humatad/tests/stop_access.rs::a_script_whose_token_lists_stopping_sessions_stops_the_machine_and_is_named_a_script`).
- Owner decision (Provisional D522): making or revoking a token needs the user and no second presence step. Reverse it to have the daemon answer `PresenceRequired`, which the CLI already handles.
- Owner decision (Provisional D523): a token's channel list limits channel actions, not a machine stop.
- Fixed: a real-daemon test runs an allowed script stop (`daemon/humatad/tests/stop_access.rs::a_script_whose_token_lists_stopping_sessions_stops_the_machine_and_is_named_a_script`).
- The refusal for an unusable token says "made by another daemon" (`status.rs:256`). That is accurate for another machine, but a revoked token gets the same words, so the fix does not say which case it is.

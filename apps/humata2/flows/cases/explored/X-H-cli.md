# CLI: humata desk, humata channel, humata repos (and the session commands the Desk uses)

Layer: explored · macOS · harness: the throwaway `humatad2` and the `humata` CLI built from `//apps/humata2/cli:humata` (no test target; the CLI commands are driven from a shell)

Phase 1 (map from `cli/src/args.rs`, `cli/src/commands/{desk,repos,looking,mod}.rs`, `cli/src/exit.rs`, `cli/src/error.rs`, and the daemon test rig, no Bazel run). Every step is PENDING RUN.

## Setup

- Build (phase 2, once): `export PATH=$PWD/.tools/bin:$PATH; bazel build --jobs=4 //apps/humata2/cli:humata //apps/humata2/daemon/humatad:humatad2 //apps/humata2/daemon/humatad:holder_for_clients`. The live rig also builds `//apps/humata2/daemon/humatad:holder_for_clients` next to the daemon, since the test rig passes a holder.
- Throwaway daemon, with its own root and a temp HOME under the scratchpad (never the owner's `~/.humata2`, `~/.claude`, `~/.codex`, `~/.copilot`). The flags the test rig passes (`desktop/tests/`'s `daemon/humatad/tests/common/mod.rs` lines around 236 to 244) are `--socket`, `--state`, `--root`, `--holder`, `--shell /bin/sh`, and `--shell-arg=-i`. The `humatad2 serve` flag list has not been read in full in phase 1; PENDING RUN step 0 reads it with `--help` before the first start.
- Point the CLI at it with `HUMATA_SOCKET=<scratch>/xh.sock`.
- Scratchpad root: `<scratchpad>`. Transcripts go in `verify/evidence/X-H/cli-*.txt`.

## Steps

### 1. `humata desk --help`: what the Desk command offers
- **Screen (text):** the subcommand list for `desk`: only `keep [<session>]` ("Keep a Desk session's scratch folder after it closes, so it is never deleted after its 7 days. Inside a Desk session, leave out the session to keep this one's own folder.").
- **Offered:** keep.
- **Action:** `humata desk --help`.
- **Result:** Prints the help; exit 0. There is no `move`, `add`, `list` or `new` under `desk`.
- **Evidence:** `verify/evidence/X-H/cli-desk-help.txt` (PENDING RUN).
- **Replay check:** `$HUMATA desk --help; echo "exit $?"` expected: exit 0, one subcommand `keep`.

### 2. `humata session move` and `humata session repo add`: where the moves live
- **Screen (text):** `session` has `move <session> --to <machine>` (`SessionCommand::Move`) and `repo add|drop <session> <repo>` (`SessionRepoCommand`).
- **Action:** `humata session move --help`, `humata session repo --help`.
- **Result:** Usage text; exit 0.
- **Evidence:** `verify/evidence/X-H/cli-session-move-help.txt` (PENDING RUN).
- **Replay check:** `$HUMATA session move --help; echo "exit $?"` expected: exit 0, `--to` is required.

### 3. `humata desk keep` with no session outside a Desk session
- **Screen (text):** the command takes the caller's own session; outside a Desk session the daemon has no session to keep.
- **Offered:** keep.
- **Action:** From the shell (not inside a Desk session): `$HUMATA desk keep; echo "exit $?"`.
- **Result:** An error (not verified in phase 1; the code calls `reply(... KEEP_DESK_FOLDER)` with no session, so the daemon decides). Record the exit code and the message.
- **Evidence:** `verify/evidence/X-H/cli-desk-keep-no-session.txt` (PENDING RUN).
- **Replay check:** `$HUMATA desk keep; echo "exit $?"` expected: non-zero; record the code. The message should say how to resolve it (the owner keeps a folder by name or id).

### 4. `humata desk keep <unknown>`
- **Screen (text):** the session name is resolved through `ctx.session(&name)`.
- **Action:** `$HUMATA desk keep nosuch; echo "exit $?"`.
- **Result:** Expected not_found (exit 3 from `cli/src/exit.rs`, `NOT_FOUND`), with the name. Verify.
- **Evidence:** `verify/evidence/X-H/cli-desk-keep-unknown.txt` (PENDING RUN).
- **Replay check:** `$HUMATA desk keep nosuch; echo "exit $?"` expected: exit 3.

### 5. `humata repos list` on a fresh library
- **Screen (text):** the library, most recently used first, with where each was last used. Empty on a fresh daemon.
- **Action:** `$HUMATA repos list; echo "exit $?"`.
- **Result:** Empty list; exit 0.
- **Evidence:** `verify/evidence/X-H/cli-repos-list-empty.txt` (PENDING RUN).
- **Replay check:** `$HUMATA repos list; echo "exit $?"` expected: exit 0.

### 6. `humata channel create` with repos, then `humata repos list`
- **Screen (text):** `channel create <name> [--repo owner/name]...` (`ChannelCommand::Create`) makes the channel and clones; machines come from `ctx.machines_flag()`.
- **Action:** Needs a fake gh or a repo the library knows. In phase 2 use `$HUMATA channel create xh-web; echo "exit $?"` (no repos, machines default), then `$HUMATA channel repo add xh-web owner/api; echo "exit $?"` (needs gh; record the outcome either way), then `$HUMATA repos list`.
- **Result:** The channel exists with no repos; the add either clones (exit 0) or returns a gh error (record it). The library lists `owner/api` only when the add succeeded.
- **Evidence:** `verify/evidence/X-H/cli-channel-create.txt` (PENDING RUN).
- **Replay check:** `$HUMATA channel create xh-web; echo "exit $?"` expected: exit 0 and the channel shows in `$HUMATA channels`.

### 7. `humata repos search` with no words
- **Screen (text):** the search needs words; no words is a usage error (`CliError::BadValue`, field "search words").
- **Action:** `$HUMATA repos search; echo "exit $?"`.
- **Result:** Usage error; exit 2 (`USAGE`) expected. Verify; the message should say to give at least one word.
- **Evidence:** `verify/evidence/X-H/cli-repos-search-empty.txt` (PENDING RUN).
- **Replay check:** `$HUMATA repos search; echo "exit $?"` expected: exit 2.

### 8. `humata repos search <words>` with the scope flags
- **Screen (text):** a flag changes the scope for this search only (never saved). `--owner` (repeatable, a user or org, not `owner/name`), `--all-of-github`, `--your-repos`; the three flags conflict.
- **Action:** `$HUMATA repos search api --owner acme --all-of-github; echo "exit $?"` (conflict), then `$HUMATA repos search api --owner acme/x; echo "exit $?"` (bad owner).
- **Result:** Clap rejects the conflict (exit 2). The bad owner is a BadValue (exit 2, "a GitHub user or organization such as acme").
- **Evidence:** `verify/evidence/X-H/cli-repos-search-flags.txt` (PENDING RUN).
- **Replay check:** the two commands above; expected: both exit 2.

### 9. `humata repos scope set` and `show`
- **Screen (text):** `scope show` prints the saved scope; `scope set --owner <o>` saves owners plus own repos unless `--no-include-me`. `scope set --no-include-me` with no owner is refused.
- **Action:** `$HUMATA repos scope show`, `$HUMATA repos scope set --no-include-me; echo "exit $?"`, `$HUMATA repos scope set --owner acme; echo "exit $?"`, then `$HUMATA repos scope show`.
- **Result:** The first set is refused (BadValue, exit 2, "at least one --owner ... when your own repos are left out"). The second saves; show prints it.
- **Evidence:** `verify/evidence/X-H/cli-repos-scope.txt` (PENDING RUN).
- **Replay check:** `$HUMATA repos scope set --no-include-me; echo "exit $?"` expected: exit 2.

### 10. `humata repos forget-all`: dry run, then the confirmation
- **Screen (text):** `forget-all` lists the plan: `{"forget": <n>, "kept": "every channel, session and clone; nothing is deleted on disk or on GitHub"}`. `--dry-run` prints it and stops.
- **Action:** `$HUMATA repos forget-all --dry-run; echo "exit $?"`, then `$HUMATA repos forget-all; echo "exit $?"` with no terminal (non-interactive), then `$HUMATA repos forget-all --yes; echo "exit $?"`.
- **Result:** The dry run prints the plan (`Outcome::Planned`), exit 0 (verify). Without --yes and not interactive, the CLI refuses with the message "Run it with --dry-run to see the plan, then with --plan <token> to do exactly that, or add --yes to skip the check." (exit 2, `invalid_field`). With --yes the library is emptied.
- **Evidence:** `verify/evidence/X-H/cli-repos-forget-all.txt` (PENDING RUN).
- **Replay check:** `$HUMATA repos forget-all --dry-run; echo "exit $?"` expected: exit 0 and a plan; `$HUMATA repos forget-all </dev/null; echo "exit $?"` expected: exit 2.

### 11. `humata repos remove <owner/name>`
- **Screen (text):** "Take one repo out of the library. Channels, sessions and clones that use it are untouched. For the user only." The daemon enforces "user only": an agent's call gets `user_only` (exit 5, `NEEDS_USER`).
- **Action:** `$HUMATA repos remove owner/api; echo "exit $?"` (from the user's shell, the daemon accepts), and an agent-side call (not run in phase 2 unless a session is at hand).
- **Result:** From the user's shell the repo leaves the library (exit 0); `repos list` no longer shows it, and the channel keeps its repo.
- **Evidence:** `verify/evidence/X-H/cli-repos-remove.txt` (PENDING RUN).
- **Replay check:** `$HUMATA repos remove owner/api; echo "exit $?"` then `$HUMATA repos list` expected: exit 0; the repo is gone from the list.

### 12. `humata channel repo add` and `rm`
- **Screen (text):** `channel repo add <channel> <repo>` and `rm`; the channel is resolved by name or id.
- **Action:** Covered in step 6 for add; `$HUMATA channel repo rm xh-web owner/api; echo "exit $?"`.
- **Result:** Removes the repo from the channel's list; clones are not removed by this command in the tests (verify in phase 2 by looking at the channel's grid: `$HUMATA channel grid xh-web`).
- **Evidence:** `verify/evidence/X-H/cli-channel-repo-rm.txt` (PENDING RUN).
- **Replay check:** `$HUMATA channel repo rm xh-web owner/api; echo "exit $?"` expected: exit 0 (if the repo was added in step 6).

### 13. `humata session move` with an unknown session
- **Screen (text):** `session move <session> --to <machine>`.
- **Action:** `$HUMATA session move nosuch --to this-mac; echo "exit $?"`.
- **Result:** not_found, exit 3 expected.
- **Evidence:** `verify/evidence/X-H/cli-session-move-unknown.txt` (PENDING RUN).
- **Replay check:** the command above; expected: exit 3.

### 14. `humata desk move` (the command the snapshot and hint use)
- **Screen (text):** the command does not exist. clap prints an unrecognized-subcommand usage error.
- **Action:** `$HUMATA desk move devbox-1; echo "exit $?"`.
- **Result:** Usage error, exit 2 expected.
- **Evidence:** `verify/evidence/X-H/cli-desk-move-missing.txt` (PENDING RUN).
- **Replay check:** the command above; expected: exit 2. This is the check behind the Surprises entries in `X-H-desk-page.md`.

## Not reachable headless

- `humata desk keep` from inside a Desk session (the "leave out the session" form): needs a live Desk session with `$HUMATA` in its terminal. The live desk test drives the same keep through the app's Keep button, not the CLI.
- `repos search` results: needs gh (the fake gh only exists in the live rig, `World::repo`). Phase 2 can run `repos search` against the throwaway daemon with no gh and record the error; the page of results is not reachable from a plain shell.
- Agent-side `repos remove` (user only): the daemon's refusal needs an agent's identity, which the throwaway daemon does not give a plain shell.

## Surprises

- **The transcripts in the Desk snapshots use commands that do not exist.** `38-desk-move.view.txt` shows `humata desk move` and `humata desk add`; `37-desk-board.view.txt` shows `humata channel_create ... repos=...`. The real forms are `humata session move <s> --to <m>`, `humata session repo add <s> <repo>`, `humata channel create <name> --repo <owner/name>`. Step 14 checks the first.
- **The New desk session hint names `humata desk move`.** `ui/src/overlay_desk_new.rs` prints "Move it later with humata desk move", which is the same missing command.
- **`forget-all` names a flag the reader may not know.** The confirmation says "--plan <token>". The flag exists (`args.rs`, global `--plan`); a person who runs the command without a terminal sees the three choices together.
- **Two removes with different reach.** `repos remove` leaves channels and clones alone (the library entry only). `channel repo rm` changes a channel's repo list. The help text of each says so; a person who types the wrong one would not see the other's effect. Facts only.
- **Usage errors and user-only errors share the same shape.** Exit 2 covers a bad owner and an unconfirmed `forget-all`; exit 5 covers a user-only call. An agent that gets 5 is told the call is for the user (the daemon's message), not what to do instead.

# A machine at another release refuses starts, typing and writable attaches until I migrate it, and reads still work

Layer: agents. Platforms: macOS and Linux. Web is not covered.

## Starting state

- This Mac is the host at release `a`. Channel `Payments` has this Mac and `box-1`, with repo `acme/api`.
- `box-1` ran release `a` and has a session `Fit model` with its agent up. I have a read-only attach open on its tab.
- Release `b` has been staged for `box-1`. `box-1`'s daemon is now at release `b`, and the host is at `a`.
- The Inbox holds one VIP item: "Migrate the daemon on box-1".
- The agent in `Fit model` is on `box-1`. Its `humata` is the one from release `a`, and it holds a session token.

## Steps

1. **I do:** from the host, start a new session on `box-1` (for example, from the channel's page).
   **I see:** the start is refused. The message says to migrate the daemon on `box-1` first, and points to the item. Its fix says to open "Migrate the daemon on box-1" in the Inbox and click Migrate. No session appears on `box-1`.
   **State and events:** the error kind is `migration_needed`, with thing `daemon`, the machine, and the item's request id. The refusal is FailedPrecondition (CLI exit 7). No tab starts on `box-1`.
   **Status:** Works (real daemon): `daemon/humatad/tests/upgrade_refusals.rs::starting_a_session_on_a_machine_at_another_release_is_refused_with_its_item`. The gate is on the host's call (`daemon/humatad/src/session_service.rs:1854`).

2. **I do:** from the host, type `hello` into `Fit model`.
   **I see:** the typing is refused, with the same item named.
   **State and events:** no bytes reach the agent. The error carries `migration_needed` with the item.
   **Status:** Works (real daemon): `daemon/humatad/tests/upgrade_refusals.rs::typing_into_a_session_on_a_machine_at_another_release_is_refused_with_its_item` (gate at `daemon/humatad/src/terminal.rs:888`).

3. **I do:** open a writable attach to `Fit model` from the host, and type.
   **I see:** the attach ends with the refusal, naming the item. A read-only attach to the same tab works.
   **State and events:** no writable attach is open.
   **Status:** Works (real daemon): `upgrade_refusals.rs::a_writable_attach_to_a_machine_at_another_release_is_refused_with_its_item`. The read-only attach is in `upgrade_refusals.rs::a_machine_at_another_release_can_still_be_read`.

4. **I do:** look at `box-1` from the host: its session, its tabs and the machine list.
   **I see:** the session, the tab (read-only) and `box-1` in the machine list all show.
   **State and events:** reads go through. The host still lists the session.
   **Status:** Works (real daemon): `upgrade_refusals.rs::a_machine_at_another_release_can_still_be_read`.

5. **I do:** the agent in `Fit model` runs `humata whoami` with its release-`a` CLI, against `box-1`'s release-`b` daemon.
   **I see:** the CLI fails with `migration_needed` for the CLI (thing `cli`). The fix says to run the `humata` that comes with the daemon's release, or to click Migrate on the CLI if it waits in the Inbox.
   **State and events:** the refusal names the CLI and the machine. The exit code is 7 (precondition).
   **Status:** Works (real daemon) for the refusal: `daemon/humatad/tests/upgrade_refusals.rs::a_cli_at_another_release_is_refused_with_migration_needed`. The test sends the CLI's release as a header, not a real `humata` binary. The exit code 7 is Fakes only: `cli/tests/exit_codes.rs::each_error_kind_exits_with_its_code_and_name` (the fake daemon sends every error kind).

6. **I do:** the same agent's CLI call returns the error. I check that `ServerInfo` still answers, so the CLI can learn which release to be.
   **I see:** `ServerInfo` answers with `box-1`'s bundle.
   **State and events:** only ServerInfo and Upgrade pass a mismatched CLI.
   **Status:** Works (real daemon): `upgrade_refusals.rs::a_cli_at_another_release_still_gets_server_info`.

7. **I do:** run the release-`b` `humata` that comes with the daemon, from the same agent's shell.
   **I see:** the command is served. The agent is recognised as the agent.
   **State and events:** the CLI's release matches the daemon's.
   **Status:** Works (real daemon): `upgrade_refusals.rs::an_agents_cli_at_the_daemons_release_is_served_as_the_agent`.

8. **I do:** run a hand-built `humata`, one that names no release, with the agent's token.
   **I see:** refused with `migration_needed` for the CLI. The fix says a hand-built humata names no release.
   **State and events:** refused whatever the call.
   **Status:** Works (real daemon): `upgrade_refusals.rs::an_agents_cli_that_names_no_release_is_refused`.

9. **I do:** the agent reads a file in `Fit model`'s folder.
   **I see:** the read works.
   **State and events:** reads are never refused by the release gate.
   **Status:** Code, no test. The gate exempts reads by its design (`daemon/humatad/src/upgrade/gate.rs` header). No test reads a file from a mismatched box as an agent through MCP. The host-side read of a session is tested in step 4.

10. **I do:** the agent in `Fit model` tries to answer the Migrate item (Approve) with its token.
    **I see:** the answer is refused as user-only, with permission denied. The item stays pending.
    **State and events:** the agent's call is refused for being an agent, not for the release. The item is unchanged.
    **Status:** Works (real daemon): `daemon/humatad/tests/upgrade_inbox.rs::an_agent_cannot_answer_a_migrate_item`. This test runs the agent on this Mac, not on `box-1`, so the agent-on-`box-1` route is not tested.

11. **I do:** on the item, try Deny, Deny with a note, Always, and Reply. Then I press Answer all (deny all, approve all, or with the item selected).
    **I see:** each of the four answers is refused. Answer all skips the item. The item is still pending.
    **State and events:** only Migrate (Approve) can answer the item.
    **Status:** Works (real daemon): `upgrade_inbox.rs::a_migrate_item_takes_no_answer_but_migrate` and `upgrade_inbox.rs::answering_all_leaves_a_migrate_item_waiting`.

12. **I do:** click Migrate on the item.
    **I see:** the release is staged onto `box-1`, the daemon swaps and restarts, and the item ends Approved. The machine's daemon reports release `b`.
    **State and events:** the old daemon is kept as `humatad2.previous`, and nothing is left staged.
    **Status:** Works (real daemon, fake box over SSH): `daemon/humatad/tests/upgrade_box.rs::approving_migrate_stages_the_release_over_ssh_and_the_machine_comes_back_at_it`. The machine is a fake box in the rig.

13. **I do:** after the migration, start a session on `box-1`, type into `Fit model`, and open a writable attach.
    **I see:** all three work.
    **State and events:** the gate passes, because no mismatch remains.
    **Status:** Code, no test. `machine_ready` returns Ok without a mismatch (`daemon/humatad/src/upgrade/gate.rs:23-35`), but no test runs these calls after a migration. `upgrade_box.rs` and `upgrade_local.rs` check the release and the files only.

## Branches

### B1. The upgrade is cut midway on this Mac

**When:** the local daemon is killed at a point in its own upgrade, and starts again.
**I do:** restart.
**I see:** the daemon comes back whole on release `a`, and the record says the upgrade did not take. A retry upgrades.
**Status:** Works (real daemon): `daemon/humatad/tests/upgrade_cut.rs::a_cut_between_the_daemon_and_the_bundle_being_swapped_leaves_release_a_whole` and `upgrade_cut.rs::a_new_build_cut_after_opening_the_store_at_the_new_schema_is_rolled_back_to_release_a`.

### B2. The upgrade is cut midway on `box-1` over SSH

**When:** the connection drops or the box restarts while `box-1`'s files are being swapped.
**I do:** restart the box.
**I see:** expected: the box comes back whole on one release, and the item shows what it is now.
**Status:** Code, no test. `upgrade_box.rs` tests only the successful SSH path. No test cuts the SSH swap.

### B3. Mixed versions are never a fallback

**When:** the host at `a` and `box-1` at `b` are asked for a start, a type or an attach.
**I do:** make the call.
**I see:** the call is refused, not run on an older protocol.
**Expected:** UPGRADE.md:47 says "Refusals instead of fallbacks". D281 (provisional, 2026-11-16) and D290 l say the same.
**Status:** Works (real daemon): `upgrade_refusals.rs::a_cli_at_another_release_is_refused_with_migration_needed` and step 1 above.

### B4. The machine's identity does not match its bundle

**When:** `box-1`'s daemon binary differs from its entry in `bundle.json`.
**I do:** the host reads `box-1`'s build.
**I see:** the daemon names no bundle. The host treats `box-1` as at another release, so it posts the Migrate item, and refuses the start.
**Status:** Works (real daemon): `daemon/humatad/tests/upgrade_identity.rs::a_daemon_whose_binary_differs_from_its_entry_names_no_bundle`, `upgrade_identity.rs::the_hello_reply_of_a_machine_off_its_bundle_names_no_bundle`, and `upgrade_inbox.rs::a_machine_whose_binary_is_off_its_bundle_counts_as_another_release`.

### B5. A machine changes release again while its item is pending

**When:** `box-1` moves to another release while the item waits.
**I do:** change the release again.
**I see:** the one item stays, and says what the machine is now. No second item appears.
**Status:** Works (real daemon): `upgrade_inbox.rs::a_machine_that_changes_release_again_keeps_its_one_item_showing_what_it_is_now`.

### B6. The machine is back at the host's release

**When:** `box-1` returns to release `a` while the item waits.
**I do:** check the Inbox.
**I see:** the item is withdrawn as Resolved.
**Status:** Works (real daemon): `upgrade_inbox.rs::a_machine_back_at_the_hosts_release_withdraws_its_item_as_resolved`.

### B7. Writes and moves to a machine at another release

**When:** I save a file into `box-1`'s folder, or move the session to `box-1`, while `box-1` is at another release.
**I do:** write a file (or start a move) to `box-1`.
**I see:** expected by UPGRADE.md:48 and by the header of `upgrade_refusals.rs` (line 3): refused with `migration_needed`. Actual: the write is forwarded to the machine.
**Status:** Likely bug. `daemon/humatad/src/files.rs:245-262` sends a remote write to `forward.write_file`, and `daemon/humatad/src/files/forward.rs:471-480` relays it with no `machine_ready` call. `machine_ready` is called only from `session_service.rs:1854` (start), `terminal.rs:888`, `:1109` and `:1133` (type and attach). `move_session_of` (`session_service/move_session.rs:162`) has no call either. The test header and the spec disagree with the code. To confirm: write a file to a box at another release and watch it arrive.

### B8. An agent on `box-1` calls its own daemon

**When:** the agent in `Fit model` runs `humata session start` on `box-1`, with `box-1`'s own CLI.
**I do:** run the command.
**I see:** unknown. The gate is on the host's calls, and no test shows what `box-1`'s own daemon does with its own agent.
**Status:** Code, no test. Step 1's test drives the start from the host.

### B9. Agent answers the Migrate item from `box-1`

**When:** the agent on `box-1` (not on this Mac) sends an Approve to the item.
**I do:** send the answer.
**I see:** expected: refused as user-only.
**Status:** Code, no test. Step 10's test runs the agent on this Mac.

## Expected outcomes

- [ ] A start, a typed line and a writable attach to a machine at another release are refused, each naming the Migrate item.
- [ ] The refusal is `migration_needed`, with thing, machine, request and both releases. The CLI exits 7.
- [ ] Reads, the read-only attach and ServerInfo still work.
- [ ] A CLI at another release is refused for everything but ServerInfo and Upgrade.
- [ ] A hand-built CLI that names no release is refused.
- [ ] An agent cannot answer the Migrate item. Deny, Always, Reply and Answer all leave it pending.
- [ ] Migrate swaps the release in, keeps the old daemon, and resolves the item.
- [ ] A local upgrade cut at any point comes back whole on one release.
- [ ] No mixed-version fallback runs.
- [ ] Writes and moves to a mismatched machine are refused. (Likely bug: they are not.)
- [ ] After the migration, the same start, type and attach work. (Code, no test.)

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Start refused with item | Works (real daemon) | `daemon/humatad/tests/upgrade_refusals.rs::starting_a_session_on_a_machine_at_another_release_is_refused_with_its_item` |
| 2. Typing refused with item | Works (real daemon) | `upgrade_refusals.rs::typing_into_a_session_on_a_machine_at_another_release_is_refused_with_its_item` |
| 3. Writable attach refused; read-only works | Works (real daemon) | `upgrade_refusals.rs::a_writable_attach_to_a_machine_at_another_release_is_refused_with_its_item` |
| 4. Reads work from the host | Works (real daemon) | `upgrade_refusals.rs::a_machine_at_another_release_can_still_be_read` |
| 5. Old CLI refused, exit 7 | Works (real daemon) for refusal; Fakes only for exit 7 | `upgrade_refusals.rs::a_cli_at_another_release_is_refused_with_migration_needed`; `cli/tests/exit_codes.rs::each_error_kind_exits_with_its_code_and_name` |
| 6. ServerInfo still answers | Works (real daemon) | `upgrade_refusals.rs::a_cli_at_another_release_still_gets_server_info` |
| 7. CLI at daemon's release served | Works (real daemon) | `upgrade_refusals.rs::an_agents_cli_at_the_daemons_release_is_served_as_the_agent` |
| 8. Hand-built CLI refused | Works (real daemon) | `upgrade_refusals.rs::an_agents_cli_that_names_no_release_is_refused` |
| 9. Agent reads its files | Code, no test | `daemon/humatad/src/upgrade/gate.rs` header (exempt reads); no agent read test |
| 10. Agent cannot answer the item | Works (real daemon) | `daemon/humatad/tests/upgrade_inbox.rs::an_agent_cannot_answer_a_migrate_item` (agent on this Mac) |
| 11. Deny, Always, Reply, Answer all leave it | Works (real daemon) | `upgrade_inbox.rs::a_migrate_item_takes_no_answer_but_migrate`; `upgrade_inbox.rs::answering_all_leaves_a_migrate_item_waiting` |
| 12. Migrate swaps the release | Works (real daemon, fake box) | `daemon/humatad/tests/upgrade_box.rs::approving_migrate_stages_the_release_over_ssh_and_the_machine_comes_back_at_it` |
| 13. Same calls work after migrating | Code, no test | `daemon/humatad/src/upgrade/gate.rs:23-35` |
| B1. Local upgrade cut midway | Works (real daemon) | `daemon/humatad/tests/upgrade_cut.rs::a_cut_between_the_daemon_and_the_bundle_being_swapped_leaves_release_a_whole`; `upgrade_cut.rs::a_new_build_cut_after_opening_the_store_at_the_new_schema_is_rolled_back_to_release_a` |
| B2. SSH upgrade of a box cut midway | Code, no test | `daemon/humatad/tests/upgrade_box.rs` tests the success path only |
| B3. No mixed-version fallback | Works (real daemon) | `upgrade_refusals.rs::a_cli_at_another_release_is_refused_with_migration_needed`; `workflow/milestones/UPGRADE.md:47`; D281 |
| B4. Identity mismatch | Works (real daemon) | `daemon/humatad/tests/upgrade_identity.rs::a_daemon_whose_binary_differs_from_its_entry_names_no_bundle`; `upgrade_inbox.rs::a_machine_whose_binary_is_off_its_bundle_counts_as_another_release` |
| B5. Changes release again, one item | Works (real daemon) | `upgrade_inbox.rs::a_machine_that_changes_release_again_keeps_its_one_item_showing_what_it_is_now` |
| B6. Back at host's release, item withdrawn | Works (real daemon) | `upgrade_inbox.rs::a_machine_back_at_the_hosts_release_withdraws_its_item_as_resolved` |
| B7. Writes and moves refused | Likely bug | `daemon/humatad/src/files/forward.rs:471-480`; `daemon/humatad/src/session_service/move_session.rs:162`; spec `UPGRADE.md:48` |
| B8. Agent on box calls its own daemon | Code, no test | `session_service.rs:1854` (host side only) |
| B9. Agent on box answers the item | Code, no test | `upgrade_inbox.rs::an_agent_cannot_answer_a_migrate_item` runs on this Mac |

## Gaps and likely bugs

- Likely bug: writes and moves to a machine at another release are not refused. Only start, type and writable attach call `machine_ready`. `daemon/humatad/src/files/forward.rs:471-480` relays a remote write, and `daemon/humatad/src/session_service/move_session.rs:162` starts a move, with no gate. The spec (`workflow/milestones/UPGRADE.md:48`) and the header of `daemon/humatad/tests/upgrade_refusals.rs` both say writes and moves are refused.
- Test gap: no test runs a start, a type or an attach after a migration (step 13), so the "same calls work" outcome is unproven.
- Test gap: no test cuts the SSH upgrade of a box (B2).
- Test gap: no test covers an agent on the box itself (B8, B9). The gate and the answer tests run from this Mac.
- Test gap: the CLI refusal tests send the release as a header, not through a real `humata` binary. The exit code 7 is checked only against a fake daemon.
- Not verified: the Migrate item for the CLI (thing `cli`) is named in the refusal, but no test here checks that such an item is posted.

## Corrections to the brief

- The brief's scenario has the agent on `box-1` try the start, the type and the attach. The gate is on the host's calls (`session_service.rs:1854`, `terminal.rs:888`). The tests drive them from the host. The agent-on-box route is Code, no test.
- The brief asks "which code?" for `migration_needed`. It is exit 7 (precondition), per `cli/src/exit.rs:77` and `cli/tests/exit_codes.rs:64`.
- The brief's mixed-version question: the rule is in `workflow/milestones/UPGRADE.md:47` ("Refusals instead of fallbacks") and in D281 and D290 l in `DECISIONS.md`. No D-number is named for "mixed" alone.
- The brief's "agent on box-1 reads its files fine" has no test. Only the host's read is tested (step 4).
- The brief lists `upgrade_refusals.rs` as covering writes and moves, which its header claims. The code does not refuse them (B7).

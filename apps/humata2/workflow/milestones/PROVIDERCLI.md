# PROVIDERCLI: An agent can write a machine provider from the humata CLI alone

- **Status:** queued — opens after FUSEFSWIRE closes, ships as **0.10.1** (owner, 2026-11-20); NEWCHCLOSE moves to 0.10.2
- **Depends on:** FUSEFSWIRE (owner's order), 0.10.0
- **Unblocks:** NEWCHCLOSE's release slot

## The owner's words (2026-11-20)
"add a helper CLI set of commands for making sure an agent can understand the remote machine providers stuff … so that an agent could write the integrations for us just based on this CLI stuff alone."

## Outcome
An agent with no repo access, given only `humata` and `humata provider --help`, writes a working provider for a new cloud, VM tool or internal system. It learns the model, checks its file, sees the exact commands Humata would run, and hands the file to the owner through the existing `humata provider import`, which the owner must approve (MP-23).

## Commands (all read-only; nothing is run, imported or changed; no approval needed)
- `humata provider guide`: the whole model in plain text. It covers:
  - what a provider is (MP-1, MP-2);
  - every action and when Humata calls it (create, start, stop, status, delete, discover; MP-21);
  - the three ways to reach a machine (connect, identity, plain SSH; MP-7 to MP-10);
  - templates `{name}` and `{cmd}` and the `HUMATA_<PARAM>` variables (MP-3, MP-4);
  - where commands run (MP-5);
  - interactive logins and takeover (MP-12 to MP-18);
  - the SSH steps after identity (MP-19);
  - what a failure looks like (MP-20);
  - how to hand the file to the owner (MP-23).
- `humata provider schema [--json]`: every TOML field with its type, whether it is required, its default and what it means, plus the identity JSON and status/discover output formats. **It is generated from the same Rust types that parse providers**, so it can't drift.
- `humata provider example [<kind>]`: complete, valid example files. The built-in Lima provider is one (MP-22). Add a generic cloud CLI, a Terraform workspace and an SSH-only fleet. Every example passes `check`, and a test proves it.
- `humata provider check <file>`: parses and validates with the real parser, offline, and names each bad field with how to fix it (MP-11 style).
- `humata provider render <file> <action> --name <m> [--param k=v]…`: prints the exact command lines, environment and target machine Humata would use for that action. Nothing is run.
- `humata provider parse <identity|status|discover> [<file>|-]`: validates output the agent got by running its own command, against what Humata expects, and names each missing or bad field.
- MCP twins of all of these as read-only tools. The `humata-remote` skill points agents at `provider guide` first.

## Who can run it (owner, 2026-11-20)
"anyone should be able to run it but I can approve it or deny it."
- The helper commands are open to every caller: the owner, any agent session, and sessions on remote machines. Remote callers' requests reach the host.
- Any of them can send an import request (`humata provider request-import <file> --reason …` and the MCP `provider_request_import`). Each one becomes a host Inbox card that only the owner approves or denies (D792 rules unchanged).

## Make sure it works end to end
- These daemon tests already exist: `daemon/humatad:provider_import_test` (an agent's import waits in the Inbox, approve adds it, deny adds nothing, a stale request adds nothing, a Never rule refuses). That level is covered.
- Still to add: a full-path check.
  1. A session agent runs the CLI, and separately the MCP tool, from the host and from a remote machine.
  2. The card appears in the desktop Inbox, through a native test of the card's text, adds and commands.
  3. Approve: `humata provider ls` lists it.
  4. Deny: it doesn't.
  5. Any gap this finds, such as remote asks being refused as "host only", gets fixed here.

## Exit criterion
- Each command has behavioural tests, given these inputs, this result or this error:
  - `check` and `parse` reject each class of bad input;
  - `render` matches what the daemon would actually invoke;
  - the examples pass `check`.
- A one-shot agent check, recorded in this file: a fresh agent, given only the CLI (no repo), writes a provider for a made-up tool that passes `check` and `render` on the first or second try.
- api/CLI.md, api/MCP.md and the skill are updated, with a GAPS_REVIEW entry. Main's gate passes, and release 0.10.1 ships.

## Read these (pointers, nothing else)
- SPEC.md MP-1 to MP-24, HO-1.
- daemon/machines/src/provider.rs: the parser and the Action, Reach, Param and StatusReport types.
- api/CLI.md §5.6; the existing `humata provider` commands.
- The `humata-remote` skill.

## Out of scope
- Letting agents import or change providers. MP-23 stands: only the owner imports.
- Running provider commands from these helpers.

## Lane
- **Status:** core offline engine, CLI commands, any-caller `RequestProviderImport`, docs/skill, and fast behavioural tests implemented (pending main build/test + seam wiring).
- **Files touched:**
  - `apps/humata2/daemon/machines/BUILD.bazel` (added `//apps/humata2/cli:__pkg__` visibility)
  - `apps/humata2/daemon/machines/src/identity.rs` (shared `IDENTITY_KEYS`, `SSH_KEYS`, `KEY_ALGORITHMS` with `spec`)
  - `apps/humata2/daemon/machines/src/provider.rs` (shared `RAW_PROVIDER_KEYS`, `RAW_PARAM_KEYS`, `STATUS_WORD_GROUPS` + compile-time exhaustiveness witness)
  - `apps/humata2/daemon/machines/src/spec.rs` (new: `provider_guide`, `provider_schema`, `list_examples`, `provider_example`, `check_providers`, `render_provider_action`, `parse_provider_output`)
  - `apps/humata2/daemon/machines/src/lib.rs` (exported `pub mod spec`)
  - `apps/humata2/daemon/machines/tests/provider_spec.rs` (new: fast leaf behavioural tests)
  - `apps/humata2/cli/BUILD.bazel` (added `//apps/humata2/daemon/machines` dep)
  - `apps/humata2/cli/src/args.rs` (`ProviderCommand::{Guide, Schema, Example, Check, Render, Parse, RequestImport}` + `ProviderParseKind`)
  - `apps/humata2/cli/src/error.rs` (`CliError::ProviderSpec`)
  - `apps/humata2/cli/src/lib.rs` (short-circuit `works_offline` before daemon connect)
  - `apps/humata2/cli/src/commands/mod.rs` (`works_offline` + `operation` arms)
  - `apps/humata2/cli/src/commands/hosts.rs` (offline helper handlers + `request-import` routing to `REQUEST_PROVIDER_IMPORT`)
  - `apps/humata2/cli/tests/provider_cli.rs` (new: offline CLI + `request-import` behavioural tests)
  - `apps/humata2/daemon/store/src/schema.rs` (Step 34 migration: allow non-session deferred Inbox requests with `session = NULL, migrate_thing = NULL`)
  - `apps/humata2/daemon/store/src/inbox.rs` (`put_inbox_request` accepts empty `session` as `NULL`; `inbox_requests()` selects `WHERE migrate_thing IS NULL` with `COALESCE(session, '')`)
  - `apps/humata2/daemon/svc_inbox/src/inbox/deferred.rs` (`deferred_in` and `card` accept `Caller::User` and `Caller::Script` with empty `session` and `requester: None`)
  - `apps/humata2/daemon/humatad/src/provider_service.rs` (allow `Caller::User`, `Caller::Agent`, `Caller::Script` in `request_provider_import`; refuse `Caller::Peer`; keep `SetProvidersConfig`/`ImportProvider` owner-only)
  - `apps/humata2/daemon/humatad/src/provider_service/ask.rs` (removed unused `not_for_the_user`)
  - `apps/humata2/daemon/humatad/tests/provider_import.rs` (updated user-caller test to verify `RequestProviderImport` files an Inbox card and applies only on owner approval)
  - `apps/humata2/skills/humata-remote/remote.md` (documented offline helpers + `request-import`)
  - `apps/humata2/api/CLI.md` (documented offline helpers + `request-import`)
- **Targets to build & test:**
  - `//apps/humata2/daemon/machines:machines_unit_test`
  - `//apps/humata2/daemon/machines:provider_spec_test`
  - `//apps/humata2/daemon/machines:providers_test`
  - `//apps/humata2/cli:cli_unit_test`
  - `//apps/humata2/cli:provider_cli_test`
  - `//apps/humata2/daemon/humatad:provider_import_test`
- **Remaining for main / follow-up:**
  - Wire MCP read-only twins (`provider_guide`, `provider_schema`, `provider_example`, `provider_check`, `provider_render`, `provider_parse`, `provider_request_import`) in `daemon/mcp/src/tools.rs`, `daemon/svc_mcp/src/mcp.rs`, `serve.rs`, and update `api/MCP.md`.
  - Verify remote machine forwarding for `RequestProviderImport` (if not already relayed in `serve.rs`/peer uplink) + native desktop Inbox card check.
  - Record the one-shot fresh-agent check and GAPS_REVIEW entry before 0.10.1 release.

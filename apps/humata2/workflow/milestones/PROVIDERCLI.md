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

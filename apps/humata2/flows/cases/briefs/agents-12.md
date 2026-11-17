# Brief agents-12: Refused until migrated
Output: `apps/humata2/flows/cases/agents/12-refused-until-migrated.md`

Scenario: an agent on box-1 (now at another release than the host) tries to start a session, type, attach writable, and run a CLI from the old release. Each is refused naming the Migrate item. It reads its files fine. It tries to answer the Migrate item itself.

Steps: refusal carries the item; CLI exits with migration_needed (which code?); reads work; agent can't answer the Migrate item; the user's Answer all leaves it waiting; after migrating, the same calls work.

Branches: the upgrade is cut midway; mixed versions are never a fallback (D-number? grep DECISIONS for "mixed"); identity mismatch.

Pointers: daemon/humatad/tests/upgrade_refusals.rs, upgrade_inbox.rs, upgrade_cut.rs, upgrade_identity.rs, upgrade_box.rs, upgrade_local.rs; cli/src/exit.rs; workflow/milestones/UPGRADE.md.

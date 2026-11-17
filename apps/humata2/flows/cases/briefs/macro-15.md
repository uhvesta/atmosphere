# Brief macro-15: Upgrade day
Output: `apps/humata2/flows/cases/macro/15-upgrade-day.md`

Scenario: I install a new release on this Mac. box-1 is still on the old one. The Inbox shows a VIP "Migrate box-1" item. I try to start a session on box-1 first, then answer the item, and box-1 upgrades.

Steps: one VIP Migrate item per machine at another release; starting, typing, or a writable attach on that machine is refused with its item; reads still work; a CLI at another release is refused with migration_needed; an agent can't answer the item; Answer all leaves it waiting; the upgrade runs (DaemonService.Upgrade — check not_built at daemon.rs:340 and when it applies) and holders survive.

Branches: the upgrade is cut by a restart (upgrade_cut.rs); identity changes (upgrade_identity.rs, IDENTITY_MISMATCH UI gap in OVERVIEW); the local daemon itself needs migrating (upgrade_local.rs); a box upgrade (upgrade_box.rs).

Pointers: daemon/humatad/tests/upgrade_*.rs, daemon/humatad/src/upgrade/, ui/src (grep "Migrate"), workflow/milestones/UPGRADE.md (open), memory note "every needed update is a VIP Inbox Migrate X message; no mixed-version fallbacks".

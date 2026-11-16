# UPGRADE: Any Humata found at another version is upgraded at once

- **Status:** open (2026-11-16, the owner: "We should force upgrade daemons and stuff when we detect them")
- **Depends on:** MACH (machine link and Hello `daemon_version`); STOP* and SAFE* (an upgrade re-execs the daemon the way stop/start does; it can't close until those paths hold)
- **Unblocks:** V1, ANYHOST; lets TERM and others assume same-version peers
- **Layer:** L4 · **Lane:** daemon + clients + UI

## The owner's words (2026-11-16)
"We should force upgrade daemons and stuff when we detect them." Then: "when we detect updates that should happen our inbox should get vip messages that say 'migrate x'. Same with local CLIs, schemas, remote clis, local daemons, remote daemons. Etc...."

## Outcome
Every needed update shows up as a VIP Inbox message, "Migrate <thing>": the local CLI, the store schema, a machine's CLI, the local daemon, a machine's daemon, holders and host tools. Each message has one action, Migrate, that does it, and says what it changes and what keeps running. It can't be dismissed while the mismatch stands, and nothing gets a mixed-version fallback: an action that needs the new version is refused with "Migrate <thing> first", and the refusal points to the message. When the migration is done, the message ends as "Migrated to <version>".
Migrate does the work: when the app, the host or a machine sees a humatad2, holder, CLI or host tool at another version than the app's, Migrate upgrades it: the local daemon when the app is newer, a machine's daemon and tools when it links, and the CLI the agents use. A daemon upgrade leaves running holders alone (PER-5); a holder is only replaced when no live session needs it (PER-6). Sessions keep running through an upgrade, and the machine page says what was upgraded and when. An upgrade that fails says what failed and how to fix it, and the machine stays usable read-only until it is fixed.

## Exit criterion
- On launch the app finds an older local daemon and posts "Migrate the daemon on This Mac"; Migrate upgrades it; a host upgrades a machine whose Hello names another version, over the same SSH path that installs it, and the link comes back at the new version.
- Holders and sessions survive every upgrade (same pids, scrollback); no input is lost or replayed.
- A failed or cut upgrade (fault points, power cut) leaves the old or the new version running whole, never neither.
- Each kind (local CLI, schema, remote CLI, local daemon, remote daemon, holder, host tools) posts one VIP "Migrate <thing>" message when it is detected, Migrate clears it, and a second detection never posts a duplicate.
- Fakes: the Inbox VIP message, its Migrate action, and the machine page show "Upgraded to <version>" and a failure state, matching frames signed off by the advisor.

## Read these (pointers, nothing else)
- SPEC: PER-5, PER-6, MP-*, host tool install (grep "installs", D49). DECISIONS: D49, D261/D265 (re-exec), D226 (link-up), peer_version_unsupported.
- Code: `daemon/humatad/src/machine_service*.rs` (Hello, daemon_version), `daemon/machines/` (install over SSH), `daemon/humatad/src/stop/exec.rs` (re-exec), the app's daemon launch in `desktop/`/`clients/live`.

## Out of scope
Release channels, rollbacks the user picks, upgrading third-party agents (Claude, Codex, Copilot). Linux VM runs (LINUX).

## Lane (written by the manager)

Manager: minion UPGRADE (Opus 5.5), opened 2026-11-16. Phase 1 plan; no code yet. The picks came from an Opus 5.5 high advisor (Fable is rate-limited until 2026-11-22). Main logs them as D-entries (list below), all Provisional.

### Where versions are detected today
- **App and local daemon:** `desktop/install` runs on every launch. It checks each of the three binaries by sha256 and, when the daemon changed, swaps it and runs launchd bootout+bootstrap, with no prompt. The app never compares the version of the daemon it connects to. `ServerInfo.daemon_version` is `CARGO_PKG_VERSION`, which is the same for every dev build.
- **Host and machine:** the Hello reply carries `daemon_version` (also `CARGO_PKG_VERSION`). `peer_client::open_from` checks only `api_major`/`api_minor` (`MINOR_BEHIND = 2`) and raises `peer_version_unsupported`. The SSH install (`daemon/machines/src/install.rs`) swaps `humatad2` at once. It stages the holder and CLI as `.<name>.staged` while any holder runs.
- **CLI:** sends no version at all.
- **Holder:** has its own wire `VERSION = 1`, exact match in its Hello (C1's crate). Nothing records which build a running holder is.
- **Store:** migrates on open in one transaction and refuses `SchemaTooNew`.

### Structure (plain words)
1. **One id per release.** The bundle id is the sha256 of the bundle manifest (`bundle.rs`), installed as `~/.humata2/bin/bundle.json`. A daemon reports it in a new `BuildInfo` (ServerInfo field 10, Hello reply field 11, host's Hello field 7), but only if its own binary's hash matches its platform's entry in that file. Otherwise it reports `""`, and `""` always counts as a mismatch. The CLI sends `humata-bundle` metadata. The user sees "2.0.5 (a1b2c3d)".
2. **Detection posts a VIP Inbox item; it never upgrades by itself.** `REQUEST_KIND_MIGRATE = 8`: from Humata (no requester), always VIP, with one user-only answer, Migrate (Approve), and no Deny, Always or Allow-all.
   - **Card:** `MigratePreview` (preview 14) holds the thing, machine, from/to, changes, what keeps running, the phase (waiting, migrating, failed), the last error and the sessions still waiting.
   - **De-duplication:** a partial unique index on (thing, machine) while pending, so a second detection updates the card instead of posting another.
   - **Endings:** done is APPROVED, "Migrated to <v>". A failure stays pending with the error and Migrate as retry. A mismatch that goes away by itself withdraws the item (`WITHDRAW_REASON_RESOLVED = 9`).
   - **Who holds it:** the host daemon holds every item, with one exception. An older local daemon can't hold kind 8, so for that one the app inserts its own item into the Inbox list; the daemon's row wins once one exists.
3. **Refusals instead of fallbacks.** New error kind `migration_needed` (field 60), e.g. "Migrate the daemon on gpu-box first", with a fix that points to the item.
   - **A machine at another bundle:** its link comes up in a limited mode that allows Hello, reads and the Upgrade call. Launches, typing, input attach, writes and moves are refused.
   - **An old local daemon:** the app gates its own writes.
   - **A CLI with a missing or different header:** refused everything but ServerInfo.
   - **A machine newer than the app, or a store too new:** never downgraded. The link is refused: "update this app".
4. **How each thing is upgraded.**
   - **Files:** they come from the app bundle and land verified as `.<name>.next`, locally and over the same SSH install path machines use. `.staged` is retired: renaming over a running holder or CLI is safe because the running process keeps its inode.
   - **Daemon:** Migrate calls a new `DaemonService.Upgrade{to_bundle, upgrade_id}`, local or as a peer call over the limited link. The daemon gates launches and writes, swaps, then re-execs at the installed path. This reuses STOP's exec as `exec_at(path, point)`. Exec keeps the launchd job and pid; bootout followed by a failed bootstrap would leave neither version running.
   - **Daemons that predate the Upgrade call:** a one-time `launchctl kickstart -k`, or the install script's systemd/setsid restart over SSH.
   - **Finish:** the host waits up to 60 s for a Hello at the new bundle.
5. **Holders and sessions.** A daemon upgrade never touches holders (PER-5): the new daemon reattaches them, so pids and scrollback stay the same.
   - **Which holders are old:** the daemon records the holder file's sha it spawned each session with. Holders found with no record count as old.
   - **"Migrate holders on <machine>":** lists the waiting sessions. Migrate restarts each agent session the next time it is idle; the agent resumes its conversation. Shells stay on the old holder until closed (PER-6). The item stays pending with a count until none are left.
   - **Reattaching:** it needs the holder protocol frozen at `VERSION = 1` and additive, which is C1's call.
6. **Cut safety ("old or new runs whole").** The record is `<data>/upgrade/this.json` (humata2_durable), and each step has a fault point:
   - `upgrade.files.staged`
   - `upgrade.record.written`
   - `upgrade.previous.linked` (`humatad2.previous`)
   - `upgrade.binary.swapped`
   - `upgrade.exec`
   - `upgrade.boot.counted` (attempts, fsynced first)
   - `upgrade.store.copied` (pre-upgrade copy through SAFE's last_good, only when the schema differs)
   - `upgrade.schema.migrated`
   - `upgrade.ready`
   - `upgrade.record.settled` (renamed to `last.json`, which feeds the machine page)

   **Recovery:**
   - **The new build fails to reach ready twice:** it puts `previous` back, restores the copy and re-execs the old build (`rolled_back`). The store is restored only when the new daemon never reached ready, so nothing written is lost.
   - **A cut before the swap:** the old build cleans up the `.next` files and reports "nothing changed".
7. **Schema.** Normally the schema step is one line of the daemon item: "Store schema 41 → 43 (a copy is kept first)". It gets its own item, "Migrate the store on <machine>", only when the binary is already current, for example after a SAFE restore of an older copy. The daemon then serves ServerInfo, reads and that item, and holders keep running.
8. **Grouping.** CLI, host tools (plist or unit text, our `~/.local/bin/humata` link, `bundle.json`) and schema become lines of the daemon's item when they are found with it. They get their own item only when found alone. The D137 clash (a foreign `~/.local/bin/humata`) keeps its hint and is not a migration.
9. **What the user sees.**
   - **Card:** "Migrate the daemon on gpu-box", reason "gpu-box runs 2.0.4 (9f3e2d1); this app is 2.0.5 (a1b2c3d)". It lists the changes and "3 sessions stay running (same holders)" / "The link to gpu-box drops for a few seconds". Its states read Migrate → Migrating… → "Migrated to 2.0.5 (a1b2c3d)".
   - **Failure:** "Couldn't migrate the daemon on gpu-box: 2.0.5 didn't start twice, so 2.0.4 is back", with a fix naming the log and Migrate again.
   - **Machine page:** "Upgraded to 2.0.5 · 2 min ago", or a banner "Read-only until migrated · Open message".
   - **Frames:** 77–83 in new `desktop/tests/design/upgrade.rs`:
     - 77: the VIP card in the Inbox
     - 78: the card's details
     - 79: migrating
     - 80: the failure, with the read-only banner
     - 81: Migrated / Upgraded
     - 82: a refused action
     - 83: holders waiting

### Decisions for main to log (Provisional)
- D-a: the bundle id is the version, via `BuildInfo` and the `humata-bundle` header.
- D-b: every detected update is a VIP MIGRATE Inbox item, held by the host daemon, plus an app-held item for an old local daemon.
- D-c: the proto shape: kind 8, preview 14, Approve shown as Migrate, success is APPROVED, withdraw reason RESOLVED 9.
- D-d: Migrate is the user's click; "force" means the item can't be dismissed and everything else is refused.
- D-e: `migration_needed` (60); the limited link for a machine at another bundle; the CLI is refused everything but ServerInfo.
- D-f: upgrade by `.next` files, then `Upgrade` and `exec_at`; `.staged` is retired; a one-time kickstart or SSH restart for older daemons.
- D-g: the upgrade record, its fault points and rollback.
- D-h: schema is folded into the daemon item; never downgrade.
- D-i: holder builds are recorded per session; "Migrate holders" restarts idle agent sessions; the holder protocol stays frozen and additive.
- D-j: grouping, and the definitions of local CLI, remote CLI and host tools.
- D-k: the words and frames 77–83.

Main must also amend:
- **UPGRADE exit line 1:** "On launch the app finds an older local daemon and posts 'Migrate the daemon on This Mac'; Migrate upgrades it".
- **SPEC TS-0, TS-1, PER-6:** "no prompt" becomes the Migrate item, and "never under a live holder" becomes "no running holder is replaced".

### Asks of other lanes
- **STOP:**
  - the `exec_again` → `exec_at(path, point)` refactor. On Linux `current_exe()` is "<path> (deleted)" after a rename, so STOP's own re-exec is wrong after any install.
  - exempt MIGRATE items from the stop sweep's Inbox withdraw (D264).
- **SAFE:** a named pre-upgrade copy and restore entry in `last_good.rs`, and a power-cut inventory row for upgrade.
- **TERM (D271):** it can assume same-version peers. Attach and input to a mismatched machine are refused with `migration_needed`, never replayed. A web tab running another build is told "Reload this page".
- **C1:** freeze the holder protocol at VERSION 1, additive only.
- **ANYHOST:** `Upgrade` is idempotent by `upgrade_id`.

### Tasks (failing test first; outcome sentences)
- [ ] U0 "A daemon says which release it is, and only when its binary matches the bundle" · test writer Opus · new `daemon/humatad/tests/upgrade_identity.rs` · then Haiku: new `daemon/humatad/src/upgrade/build.rs` + manifest writer in `daemon/machines/src/bundle.rs` (seam) and `desktop/install`.
- [ ] U1 "Each kind of mismatch posts one VIP Migrate item, and a second detection updates it rather than posting again" · test writer Opus · new `daemon/humatad/tests/upgrade_inbox.rs` · Sonnet: new `daemon/humatad/src/upgrade/{detect,items}.rs`; seams: inbox.proto, store index (main assigns the migration number), `inbox.rs` hook (AWAY/HOOKS-held).
- [ ] U2 "An action that needs the new version is refused with 'Migrate <thing> first' and points to the item" · Opus test · new `tests/upgrade_refusals.rs` (machine limited link, CLI header, old local daemon on the app side) · Haiku: new `src/upgrade/gate.rs`; seams: errors.proto, a tower layer next to STOP's in serve.rs, cli header.
- [ ] U3 "Migrate upgrades this Mac's daemon in place: same holders, same pids, link back at the new release" · Opus test · new `tests/upgrade_local.rs` (temp HOME, fake launchd) · Sonnet: new `src/upgrade/{run,record}.rs`, `desktop/install` `.next` mode.
- [ ] U4 "Migrate upgrades a machine over the SSH path that installed it, and the link comes back at the new release" · Opus test · new `tests/upgrade_box.rs` on the remote rig · Sonnet: `daemon/machines/src/install.rs` (`.next`, staged retired), peer `Upgrade`, wait for the Hello.
- [ ] U5 "A failed or cut upgrade leaves the old or the new release running whole, never neither" · Opus test · new `tests/upgrade_cut.rs` on SAFE's fault build and power-cut rig · Haiku: the rollback in `src/upgrade/record.rs`.
- [ ] U6 "Sessions on an old holder move to the new one when idle, shells stay until closed" · Opus test · new `tests/upgrade_holders.rs` · Sonnet: new `src/upgrade/holders.rs` (needs C1's protocol note).
- [ ] U7 "The Inbox card, its Migrate action and the machine page show migrating, migrated, failed and read-only, on fakes and live" · reducer tests in new `features/inbox/tests/migrate.rs`, `features/hosts/tests/upgrade.rs` · Haiku: clients model/fake (seams), `features/{inbox,hosts}`, `ui` views; frames 77–83 with an Opus high sign-off.

### Claims (asked of main)
- **New files:**
  - `daemon/humatad/src/upgrade.rs` + `src/upgrade/`
  - `daemon/humatad/tests/upgrade_{identity,inbox,refusals,local,box,cut,holders}.rs`
  - `features/inbox/tests/migrate.rs`
  - `features/hosts/tests/upgrade.rs`
  - `desktop/tests/design/upgrade.rs`
- **Existing files, when free:**
  - `desktop/install/` (FL closed)
  - `daemon/machines/src/{install,bundle}.rs`
  - this Lane
- **Seams for main:**
  - protos: daemon, peer, inbox, errors
  - `serve.rs`, `main.rs` (`mod upgrade;`)
  - store schema/lib
  - `inbox.rs` (AWAY/HOOKS)
  - `stop/exec.rs` (STOP)
  - `last_good.rs` (SAFE)
  - clients lib/model/daemon/fake
  - `features/app/src/lib.rs`
  - `ui` lib
  - CLI metadata
  - `desktop/src/live.rs` (the launch check)
- **Never touched:** `terminal/`, `daemon/holder/`, goldens, prototypes, `.claude/`.
- **Scratch:** `/private/tmp/claude-501/-Users-avestabarzegar-oss-atmosphere-khodr/b04a634e-8063-4fd8-9208-8df4fee17944/scratchpad/upgrade/`, with bases in `upgrade/BASES.sha256`.

### Risks
- First rollout against daemons that know none of this: the one-time kickstart or SSH restart, and the app reading `this.json` itself.
- The bundle id has to come out of the Bazel build (a manifest genrule, main).
- Holder protocol freeze and scrollback carry are C1's.
- Every UI path that assumes Approve settles a request has to be checked.
- A machine newer than the app has no Migrate action.

### Workers
- 2026-11-16 phase 2 opened (main approved; decisions D281 a–k, SPEC UP-1, claims granted).
- U0–U2 test writer (Opus high) · new `tests/upgrade_{identity,inbox,refusals}.rs` + proto/BUILD seams · scratch `upgrade/batch1/` · running
- Frames 77–83 advisor (Opus high) · signed off with changes, spec in scratch `upgrade/frames-77-83.md`. Changes for D281k: one verb ("Migrated to") on the card and machine page; "Open in Inbox"; the failure text; the holders item ends when the last agent session moves (shells listed, not blocking); UP-1 gains "Allow all/Deny all skip Migrate" and "a Migrate card leaves out IN-9 parts 5/7/8"; 10 goldens (79–81 card + machine); seams Side.gpu_box, the `Machine` param in machine_page.rs, pub(crate) fork_move::step.

- Ready to commit: no

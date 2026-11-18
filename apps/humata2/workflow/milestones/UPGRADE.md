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

Manager: minion UPGRADE (Opus 5.5), opened 2026-11-16. Phase 1 plan; no code yet. The picks came from an Opus 5.5 high advisor. Owner rule: no Fable; picks go to Opus 5.5 high, or xhigh when a call needs a lot of input. Main logs them as D-entries (list below), all Provisional.

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
- **Scratch:** `<scratchpad>/upgrade/`, with bases in `upgrade/BASES.sha256`.

### Risks
- First rollout against daemons that know none of this: the one-time kickstart or SSH restart, and the app reading `this.json` itself.
- The bundle id has to come out of the Bazel build (a manifest genrule, main).
- Holder protocol freeze and scrollback carry are C1's.
- Every UI path that assumes Approve settles a request has to be checked.
- A machine newer than the app has no Migrate action.

### Workers
- 2026-11-16 phase 2 opened (main approved; decisions D281 a–k, SPEC UP-1, claims granted).
- U0–U2 test writer (Opus high) · done, reviewed by the manager. Scratch `upgrade/batch1/` (`batch1.py --check` passes): new `api/proto/humata/v1/build.proto` (BuildInfo, BuildFile, MigrateThing, in their own file because errors.proto can't import inbox.proto), `tests/upgrade_{identity,inbox,refusals}.rs`, `tests/common/{upgrade,upgrade_rig}.rs`, and 30 additive hunks (protos; RIG_TESTS; compile-only `build: None`/`host_build: None` literals; error-kind lists in cli exit, mcp). Two corrections to D281: HelloResponse.build is 12, not 11 (11 is now STOP's `sessions_stopped`), and error field 60 stays as picked.
- Manager picks for D281 (addendum, Provisional):
  - (l) A daemon whose own bundle is "" (no bundle.json: dev or test builds) has no release to compare against, so it posts no Migrate items and refuses nothing. A release always ships bundle.json, so this isn't a mixed-version fallback, and it keeps every existing test harness unchanged. A host with an id still treats a box at "" as mismatched.
  - (m) A caller counts as a CLI when it carries a CLI credential (session, script or login token) or a `humata-bundle` header. Credential-less app and admin calls are not held to the release.
  - (n) bundle.json is read once at start from `<parent of --state>/bin/bundle.json`.
  - (o) migration_needed maps to FailedPrecondition, and the CLI exits with PRECONDITION.
- Next: U0 and U2 implementation (Haiku, high), then U1 (Sonnet, medium), once main's gate shows batch 1 compiles and fails for the right reasons.
- Frames 77–83 advisor (Opus high) · signed off with changes, spec in scratch `upgrade/frames-77-83.md`. Changes for D281k: one verb ("Migrated to") on the card and machine page; "Open in Inbox"; the failure text; the holders item ends when the last agent session moves (shells listed, not blocking); UP-1 gains "Allow all/Deny all skip Migrate" and "a Migrate card leaves out IN-9 parts 5/7/8"; 10 goldens (79–81 card + machine); seams Side.gpu_box, the `Machine` param in machine_page.rs, pub(crate) fork_move::step.

- 2026-11-16 gate_b9 on batch 1: compiles. identity 8 of 8 and inbox 7 of 7 fail because the feature is missing (no ServerInfo.build, no Migrate item). For refusals, the CLI cases fail on the missing feature. The 5 Split cases were a rig bug: a host attach to gpu's tab didn't name the session, so it wasn't relayed ("no tab … is open on this machine" at common/mod.rs:509). Fixed in scratch `upgrade/fix_rig/fix_rig.py`, which edits only upgrade_refusals.rs.
- U0 build identity (Haiku high, worker A), U2 refusals (Haiku high, worker B), U1 Migrate items (Sonnet medium, worker C) · done, reviewed and merged by the manager into one script, scratch `upgrade/batch2/batch2.py --check` (15 edited and 5 new files, full files in final/, diff in batch2.diff). Manager changes:
  - one Release in serve.rs, shared by gate, Watch and the daemon;
  - CliRelease placed before STOP's cut_streams, so cut_streams stays next to the services;
  - humata-bundle added to the relay's STRIPPED;
  - the MCP server names its bundle, or agents' MCP calls would be refused (D290 m).
- Deviations for main to log (Provisional):
  - (p) Hello's build travels in BeginRelayResponse.build=2 (daemon.proto), because GetServerInfo needs a credential at the socket.
  - (q) Migrate items are in memory only, not in the store: inbox_requests.session is a NOT NULL foreign key. The host re-detects them at every link-up, so nothing is lost, but there is no partial unique index (D281 d). The store change is deferred to U3.
  - (r) Approve answers not_built until U3.
  - (s) A same-release re-detection with an unchanged card bumps no revision.
  - (t) Only start, type and writable attach are gated for now. Writes, moves and paste_image follow in U3.
- Frame seams (Sonnet medium) · done, reviewed. Scratch `upgrade/frames-seams/frames_seams.py --check` (20 hunks in design/{sidebar,machine_page,fork_move}.rs; the goldens are meant to stay byte-identical).
- Next: main applies fix_rig, batch2 and frames_seams, then gates humatad2_unit_test, upgrade_{identity,inbox,refusals}, relay, auth and design. After that, U3 (Migrate runs: DaemonService.Upgrade, swap, exec_at) and U4 (install bundle.json).

- Main's notes (D294 logs p–t). No warnings ship, so batch2's `Migrations::pending` is now `#[cfg(test)]`. `BUNDLE_HEADER` is used by mcp.rs and relay.rs, and `short()` by items.rs. A refused dev-build login says to install the matching Humata or click Migrate (gate.rs). batch2.py was regenerated and `--check` passes.
- Batch 3 briefed in parallel (scratch `upgrade/batch3/`, contract in INTERFACES.md, base = repo + batch2/final):
  - D (Sonnet medium): U3 local Migrate. This covers `Upgrade` rpc, run.rs, record.rs, the items.rs Approve arm and upgrade_local.rs.
  - E (Sonnet medium): U4. This covers the bundle.json writer, `.next` staging in desktop/install and machines/install.rs (`.staged` retired), `stage_next_remote` and upgrade_box.rs.
  - F (Haiku high): pick q's store change. Session-less Inbox rows get a partial unique index, a migration number main assigns, persistence in inbox/migrate.rs and `kept_migrates`.
- Batch 3 · done; the manager reviewed it and merged it into scratch `upgrade/batch3/batch3.py` (41 edited, 6 new, 1 deleted). It applies after batch 2, and a proof tree of repo + batch2/final checks out. The store step has the placeholder `MIGRATION_UPGRADE_N`; 23 is taken by D285, so main assigns the number.
  - D: `Upgrade` rpc; run.rs (verify, record, previous link, swap with the bundle last, exec_at after the reply drains); record.rs; inbox/migrated.rs; Approve → MIGRATING → Approved at arrival or FAILED. Tests in upgrade_local.rs (7).
  - E: the bundle.json writer, stage_next, and stage_next_remote. desktop/install stages `.next` when a daemon is installed. Setup and takeover write bundle.json on first install and no longer swap a running machine. `.staged` and local.rs are retired. Tests in upgrade_box.rs, plus the rig helper `host_at_tools_release`.
  - F: inbox_requests.session may be NULL, with migrate columns and a partial unique index; store API and tests; persistence in inbox/migrate.rs and `kept_migrates`.
  - Manager fix: D's settle_migrate saves through F's save_migrate, because a session-less row can't go through `commit`.
- Decision changes for main to log:
  - A machine that already runs humatad2 is no longer upgraded on reconnect or launch. Files are staged, and Migrate swaps them.
  - Approve returns at once in MIGRATING. Arrival at the new bundle ends the item; there is no 60 s wait inside Approve.
  - A box's Carry (stage over SSH, peer Upgrade, wait for the Hello) is still to wire: Runner returns NotHere for other machines.
  - Withdrawn Migrate rows are never pruned.
- Next: U4's box Carry, U5's rollback, U6's holders and U7's UI.
- Batch 3 regenerated (scratch `upgrade/batch3/batch3.py --check`: 44 edited, 6 new, 1 deleted; passes on the proof tree; no drift in repo files batch 2 doesn't touch):
  - Store step is 24. SCHEMA_VERSION is MIGRATIONS.len(), so it reads 24 only once the PR store's step 23 sits before it.
  - D301–D303 are added to DECISIONS.md (base sha checked).
  - The desktop/src/live.rs seam: the app uses `Contents/Resources/manifest.json` when it ships one, outside Helpers, which release.sh signs file by file. start_live returns the staged bundle id from `staged_release` after install. main.rs prints it until U7 posts the app's own Migrate item.
- Batch 4 briefed in parallel (base = repo + batch2/final + batch3/final + fix_rig + frames_seams, scratch `upgrade/batch4/base`):
  - a (Sonnet medium): U4's box Carry. Stage with stage_next_remote, call Upgrade over the peer link, and fail the item if no Hello at the new bundle arrives in about 60 s.
  - b (Sonnet medium): U5's rollback and leftover-record clearing, as one worker because both decide this.json at boot. A store copy is taken before the schema migrates, and a start at boots>=2 restores humatad2.previous and the store copy. A record whose bundle isn't this daemon's is cleared, and a repeated upgrade_id answers idempotently.
- Batch 3 rebased on the tree with PR's a1/a2 (its step 23) and applied by main. Step 24 follows step 23; D301–D303 sit before D304.
- Batch 4 merged (scratch `upgrade/batch4/batch4.py --check`: 9 edited, 3 new; base = the tree with batch 3 applied). Nothing in it is compiled yet:
  - U4 (`upgrade/remote.rs`): `Routed` carries This Mac's Migrate locally and another machine's through `Remote`. Remote stages over SSH with `stage_next_remote`, then calls Upgrade on the link (30 s). The item fails back to pending if the machine isn't at the new bundle within 60 s.
  - U5 (`upgrade/rollback.rs`): `at_start` runs before the store opens.
    - The new build's first start copies the store.
    - Its second start without reaching ready rolls back. So does a swap cut before the bundle moved (abandoned). Undoing restores the store copy, then holder, CLI and bundle, with the daemon last, then ends the record in last.json (`ended`). It re-execs the old daemon if needed.
    - The swap now keeps all four files as `.previous`, linked before the record is written. A rollback refuses a `bundle.json.previous` that isn't the record's `from_bundle`.
    - An undone upgrade's item shows Failed on the next ready. The same upgrade_id answers idempotently.
  - Fault-point order changed: `upgrade.previous.linked` now comes before `upgrade.record.written`. New points: `upgrade.store.copied`, `upgrade.schema.migrated` and `upgrade.rollback.{store,files,ended,exec}`.
  - Open: `tests/upgrade_cut.rs` doesn't test restoring an older-schema store end to end (unit test only). An undone item turns Failed only if the old build has this code.
- Gate 12 fixes (scratch `upgrade/g12fix/g12fix.py`):
  - upgrade_local: the agent/script test's token now asks for a capability, which every script token needs.
  - Rolled back or abandoned starts: serve re-reads the release after `at_start` (the undo put the old bundle.json back, so the daemon kept reporting the new one). The undo takes the installed daemon's sha from the restored bundle instead of hashing 150 MB before the socket is bound. `put_back` drops a leftover `.<name>.rollback`.
  - verify: staged files that are one platform's whole entry in the approved bundle are swapped in, even when it isn't the platform the daemon was built for. Staging over SSH picks files by the machine's `uname` (a translated daemon, the box rig's fake Linux).
  - Asked of main: MODULE.bazel builds sha2 at opt-level 3; at fastbuild every boot's hash of the daemon costs about 1 s of CPU.
  - serve::tests::idle is the PR poll's 60 s timer (D286), not UPGRADE: UPGRADE has no timer while idle.
- Next: U6's holders and U7's UI.
- Batch 5 (scratch `upgrade/batch5/batch5.py --check`; contract in `batch5/INTERFACES.md`; nothing compiled yet). Tests first, by Opus high: `tests/upgrade_kinds.rs`, `tests/upgrade_holders.rs`, `desktop/install/tests/migrate_old_daemon.rs`, `features/inbox/tests/migrate.rs`, `features/hosts/tests/upgrade.rs`, two D372 unit tests in items.rs, and frames 77–83 (10 goldens) in `desktop/tests/design/upgrade.rs`. Then Sonnet workers:
  - D370 This Mac's daemon item from a staged release (`upgrade/local.rs`, notify on bin).
  - D371 CLI and host-tools kinds, local and remote (`upgrade/tools.rs`, BuildInfo.installed, tools-only Upgrade).
  - D372 the schema line on the daemon item.
  - D373 / U6 the holders item and idle restarts (`upgrade/holders.rs`, a `HolderBuilds` hook in revive).
  - D374 the app's own item for an older local daemon (`migrate_old_daemon` in desktop/install).
  - U7 clients model, fake, live mapping, features inbox, hosts and app, and the live ui views (D375).
- Open after batch 5: the app reconnects after migrating its old local daemon; a structured session refusal; remote holders; the frame details the model doesn't carry yet (step list, per-session restart notes, the fold).
- Batch 6 (scratch `upgrade/batch6/batch6.py --check`; applies after batch 5; contract in `batch6/INTERFACES.md` and `batch6/tc/apps/humata2/API6.md`; nothing compiled yet). Tests first by Opus high: `tests/upgrade_remote_holders.rs`, step and note cases in `upgrade_{kinds,box,holders}.rs`, `features/inbox/tests/{migrate,changes_fold}.rs`, `features/session/tests/refused.rs`, `features/app/tests/upgrade_reconnect.rs`, live mapping unit tests. Then Sonnet workers: D376 the reconnect after migrating an old local daemon, D377 the session refusal, D378 remote holders, D379 steps, holder notes and the fold, drawn in the ui.
- Open after batch 6: typing and writable attach don't set the session refusal; a machine's failed holder restart isn't shown Failed on the host; api docs (GRPC.md) not updated for the new fields.
- Batches 1–4 for gate 20 (scratch `stopup/batch{1,2,3,4}.py` and `stopup/decisions.py`, applied in order, each anchor-only with --check, validated on copies; nothing compiled by the lane).
  - Batch 1, gate-20 fixes: every release rig and `migrate_old_daemon` carry the fourth file, `stack`. A real old-daemon Migrate used to fail on it, and now `desktop/install` reads and swaps it. `stack` is now a checked host tool (`tools::TOOLS`/`THINGS`/`wants`; holder and stack share one item). The agent-upgrade test takes its shell before staging (D370). The inbox migrate tests take their ordinary requests from the standard fixture. Seams: the stack settings tests expect the dropped token, the peer probe test warms its fakes, session `git()` runs without auto gc (remove_machine_live's ENOENT), and GRPC.md documents releases, `migration_needed` and stop.
  - Batch 2: D500 machine names ("This Mac") in Migrate titles and refusals. Typing and writable attach set the session's Migrate-first refusal (D377 amended). A test that a machine on the previous store schema gets the VIP daemon item (SCHEMA_VERSION − 1, for DOCV's step 31).
  - Batch 3: D501 a box's failed holder restart is shown Failed on the host (`HoldersReport.failed`).
- 2026-11-16 audit of the six items left open after batches 5 and 6, against the live tree (stopup batches 1 and 3 `--check`: all applied; batches 2 and 4 and decisions.py: applied, later reshaped by batch 3 and other lanes, so their anchors no longer match):
  - the app reconnects after migrating its old local daemon (D376): built (`LocalDaemon::back_at`, `features/app/tests/upgrade_reconnect.rs`);
  - typing and writable attach set the session refusal (D377 amended): built (`SessionAction::Typed`, `TerminalNotice::Refused`, `features/session/tests/refused.rs`);
  - remote holders (D378): built (`MachineToHost.holders`, `tests/upgrade_remote_holders.rs`);
  - a machine's failed holder restart shows Failed on the host (D501): built (`HoldersReport.failed`, items.rs tests);
  - the step list, per-session holder notes and the fold (D379): built (`MigratePreview.steps`/`holders`, clients model, `request_card.rs`, `features/inbox/tests/changes_fold.rs`);
  - GRPC.md: was missing the fields. Scratch `upgrade/grpc_doc/grpc_doc.py --check` (one SEAM hunk in api/GRPC.md §8) documents BuildInfo and where it travels, `humata-bundle`, kind MIGRATE and MigratePreview (steps, holders), `migration_needed`, Upgrade's `thing` and tools-only arm, and `HoldersReport.failed`. Validated on a mkcopy copy (apply, then already applied). No new decisions (D640–649 unused).
- Exit criterion: every item is built. Left: main applies grpc_doc.py, gate runs of batches 1–4 and the owner's golden wave for frames 77–83.
- Ready to commit: no

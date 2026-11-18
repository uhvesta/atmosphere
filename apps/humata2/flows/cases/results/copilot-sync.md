# VERIFY lane `sync`: agent setup reaches a remote machine

Agent: copilot 1.0.113, claude-haiku-5.5 (remote session `se_843fa750…`, channel `sync-remote`).
Machine(s): `vm-sync` = `v-sync-humata2-linux` (Lima `humata2-linux`, Ubuntu, aarch64, user `vsync`, removed at the end).
Build: the rebuilt `V/bin` and `V/bin-linux` (14:42), with the Add-machine pkill fix.
Evidence root: <scratchpad>/verify/evidence/sync/

## Sync the host's Copilot, Codex and Claude Code setup to a remote machine in one automatic action

Outcome sentence: the host's skills, MCP servers and other agent config reach a remote machine with one deterministic, automatic action, and a Copilot session there sees them.

Result: PARTIAL. Add, sync, idempotence and the one-skill update work. Skills reach the VM, but not where the agents read them, and MCP servers and other config never sync. The remote Copilot does not see the synced skill.

### The one action

`humata skills sync <machine>` (cli/src/commands/hosts.rs:532). It previews (plan token), then applies with `--yes`. It also runs on each peer-link connect when `skills.auto_sync` is on, except a first sync or a new clash, which asks (SK-5).

### Steps (run on the rebuilt binaries)

1. `V/harness/lane-up.sh sync`: lane daemon restarted on the new build. The lane home still has the dummy setup from the first run: `.copilot/skills/verify-hello/SKILL.md` (HELLO-COPILOT-V1), `.copilot/mcp-config.json` (`verify-echo`, command `echo`), `.claude/skills/verify-hello/SKILL.md` (HELLO-CLAUDE-V1), `.codex/skills/verify-hello/SKILL.md` (HELLO-CODEX-V1). No owner files.
2. `V/harness/vm-user.sh sync humata2-linux`: user `vsync`.
3. `humata host add v-sync-humata2-linux --name vm-sync --trust-host-key SHA256:ZlRA…0xII` (fingerprint read on the VM): **succeeded** in one attempt, steps connect, check, install, start, all done. No workaround. Evidence `host-add-attempt-4-fixed-build.json`.
4. `humata --json skills sync vm-sync --dry-run`: first_sync true; three adds (claude, codex, copilot `verify-hello`); no clashes; the MCP config is not in the plan. Evidence `skills-plan-1.json`.
5. `humata --json skills sync vm-sync --yes`: operation succeeded. Evidence `skills-sync-1.json`, `ops-after-skills-sync-1.json`.
6. On the VM: `~/Humata/.skills` is a link to `.skills-sets/<id>`. Contents: `claude/verify-hello/SKILL.md`, `codex/verify-hello/SKILL.md`, `copilot/verify-hello/SKILL.md` (the right text for each agent) and `manifest.json`. No `~/.copilot`, `~/.claude` or `~/.codex` existed, and no `mcp-config.json` came over. Evidence `vm-after-sync-1.txt`.
7. Second sync: dry-run shows `changes: []`, `first_sync: false`. `--yes` sync succeeded, the set id did not change, and one set remains. Evidence `skills-plan-2.json`, `skills-sync-2.json`, `vm-set-before-sync-2.txt`, `vm-set-after-sync-2.txt`. Result: PASS (deterministic).
8. One change: edited the host's `.copilot/skills/verify-hello/SKILL.md` to HELLO-COPILOT-V2. Fifteen seconds later, with no sync, the VM copy still says V1 (`vm-copilot-after-edit-no-sync.txt`). That confirms G3. Dry-run shows one `update` (copilot/verify-hello). `--yes` sync: the VM copy says V2, the set id changed, one set remains. Evidence `skills-plan-3.json`, `skills-sync-3.json`, `vm-after-sync-3.txt`. Result: PASS for "reaches the VM on the next sync".
9. Remote Copilot: `humata --json session start --channel sync-remote --agent copilot --model claude-haiku-5.5 --machine vm-sync`. The session started in `~/Humata/sync-remote/sessions/sync-remote-skills`. The folder-trust prompt was answered with `say.sh` (BUG-T1 still present). Asked: "List the skills you have and the MCP servers you have configured." Answer: skills `customize-cloud-agent`, `github-pr-media`, `pr-stack`; MCP servers `github-mcp-server`. Neither `verify-hello` nor `verify-echo` appears. Evidence `copilot-remote-list-after.txt`, `session-read-remote.json`, `vm-copilot-home.txt`. Result: FAIL for "sees the synced setup" (G2).
10. Observation (not verified against the record): the remote Copilot's status bar showed `GPT-5.6 Luna`, not the requested `claude-haiku-5.5`. `session read` has no model field, so I could not confirm which model ran.
11. Cleanup: remote session stopped (`session-stop.json`); VM user `vsync` removed and confirmed gone; lane daemon stopped. No host left behind.

### Environment workaround (not a product fault, recorded so it is not mistaken for one)

- The first attempt to add `vm-sync` to a channel failed to clone `verify/demo`: the remote clone uses `https://github.com/verify/demo.git`, and the lane's `--remote-template file://` applies only on the host. For the VM only: I copied the seeded `verify/remotes/verify/demo.git` to `~/verify/remotes/verify/` and set the VM user's `url.file:///home/vsync/verify/remotes/.insteadOf https://github.com/`. The VM user is throwaway and was removed. The first channel (`sync-verify`) keeps a failed clone; the working one is `sync-remote`.

## Table

| flow | result |
| --- | --- |
| Sync the host's Copilot, Codex and Claude Code setup to a remote machine in one automatic action | PARTIAL: add, sync, second sync (no change) and one-skill update pass; MCP and other config not synced; remote Copilot does not see the synced skill |

Step detail: add PASS (fixed build); sync PASS with a skills-only scope; deterministic second sync PASS; one-skill change reaches the VM on an explicit sync PASS, not automatically (G3); remote Copilot lists skills FAIL (G2).

## Findings

**F1. Fixed (was BUG, blocked every systemd Linux add).** Add machine on a systemd Linux VM failed at "Starting humatad2" with exit 255, on every attempt before the fix. Cause: the `stray` helper's `pkill -f '[h]umatad2 serve --role machine'` matched the `sh -c` running the setup script, whose text contains the unit's `ExecStart=... humatad2 serve --role machine` (daemon/machines/src/service.rs:68, 152-153). Reproduced: `pkill-self-match-repro.txt` case B. On the fixed build the same add passes. The fix in the binary is `pkill -u "$(id -u)" -x -f '[^ ]*humatad2 serve --role machine'`. Evidence of the failure: `host-add-attempt-3.json`, `ops-after-attempt-3.json`. Still open, low: the error says "the output is below", but the output is empty when the shell kills itself. Not retested.

**G1. GAP: the sync covers only skill folders. MCP servers, plugins and other agent config do not sync (confirmed live).**
- Evidence: the plan lists only `verify-hello` skills; `.copilot/mcp-config.json` never reached the VM (`vm-after-sync-1.txt`). `daemon/humatad/src/peer_skills.rs:48-54` `AGENT_FOLDERS` is `.claude/skills`, `.codex/skills`, `.copilot/skills` only. SPEC SK-1 says "skill and plugin folders", but no plugin folder is listed. Codex `config.toml`, `CLAUDE.md`/`AGENTS.md` and Claude settings are not synced either. The only other route, scripted rules (TS-3, daemon/machines/src/sync_rules.rs), parses TOML but is never run: no `humata sync run` in cli/src/args.rs and no caller in daemon/humatad.
- Fix: either narrow SPEC SK-1 and the UI copy to "skills", or add managed sources beside `AGENT_FOLDERS`: `.copilot/mcp-config.json` and `.codex/config.toml` as single files, with the machine's agent pointed at them. Claude Code keeps user-scope MCP servers in `~/.claude.json`, which also holds per-project state, so that needs a key-level sync of `mcpServers`, not a file copy. Plugins need their own folder entry if SK-1 stays as written.

**G2. GAP: synced skills are not wired to the agents on the machine (confirmed live).**
- Evidence: the set lands in `~/Humata/.skills/<agent>/<skill>/` (peer_skills.rs:81; machines/src/skills.rs:517). Nothing links it into `~/.copilot/skills`, `~/.claude/skills` or `~/.codex/skills`, and nothing sets `COPILOT_HOME`, `CLAUDE_CONFIG_DIR` or `CODEX_HOME` for machine sessions. The remote Copilot listed no `verify-hello` (`copilot-remote-list-after.txt`). SPEC SK-4 says "agents started by Humata there are pointed at it". The VM's `~/.copilot` was created by Copilot itself at session start, with no skills folder.
- Fix: at agent launch on a machine, point the agent at the managed folder without hiding machine-local skills (SK-4 forbids overwriting them): use the agent's extra skills-dir option if it has one; otherwise a merged view. Then re-run step 9.

**G3. GAP: a host skill change does not reach a connected machine on its own (confirmed live).**
- Evidence: a V1 to V2 edit on the host was still V1 on the VM 15 seconds later, with no sync (`vm-copilot-after-edit-no-sync.txt`). SPEC SK-3 says sync runs "on connect and whenever the host's skills change". `daemon/humatad/src/skills_service.rs` (header, lines 17-18): "Nothing polls (TR-7): a sync runs when the user asks, and once each time a machine's link comes up." No file watcher on the agent folders (no notify, inotify or fsevent in skills_service.rs or peer_skills.rs).
- Fix: watch the three skill folders with the OS notifications that TR-7 already allows (FSEvents on macOS, inotify on Linux), debounced about 2 seconds, then run the `on_link_up` path for each connected machine with auto-sync on. Keep SK-5 (a first sync or a new clash still asks).

**F2. UX (low): a first-seen host key is reported as `host_key_changed`.** The first add answered `{"kind":"host_key_changed", "message": "... which Humata hasn't seen before ..."}` with `expected_fingerprint: ""`. Expected: a first-trust kind (e.g. `host_key_unknown`). Evidence: the first add output (fingerprint above).

**F3. Observation (unverified): the remote session's status bar shows `GPT-5.6 Luna`, not the requested `claude-haiku-5.5`.** `session read` shows no model field, so I could not check the record. Worth checking whether `--model` is honored for remote sessions.

**F4. Harness notes (not product).**
- `lane-up.sh` copies the owner's `~/.copilot/config.json` into each lane home (`cp -n`). The brief says never read the owner's `~/.copilot`. I did not read it, but the copy happens. Suggest copying only lane-made settings.
- Remote clones use the GitHub URL, and the lane's `--remote-template` applies only on the host. A remote channel needs a URL rewrite on the VM (see Environment workaround).

**Not verified:** the automatic on-connect sync after a host change (the link-up path) was not exercised. The code path is `on_link_up` in skills_service.rs. Also not tested: a second add of the same machine, and what happens to the synced set when the machine is removed.

## Rerun 2026-11-17 (build after D758)

Agent copilot 1.0.113, claude-haiku-5.5. Machine `vm-sync` = `v-sync-humata2-linux` (fresh VM user `vsync`, fresh tools install). Evidence: V/evidence/sync/r2/.

Steps and results:
1. Two skills `verify-hello`, `verify-echo` under the lane `.copilot/skills/` (old claude/codex dummies and mcp-config removed). PASS.
2. Remove old machine record, then `host add --trust-host-key`: connect/check/install/start done, find_agents skipped. PASS. The link-up auto-sync had already synced both skills before my explicit sync (dry-run: `changes: []`, `first_sync: false`), so the explicit `--yes` was a no-op. Note: the first sync of a new user on a re-added machine did not ask (SK-5), because the daemon kept sync state for the old record.
3. On the VM: `~/.copilot/skills/verify-hello -> /home/vsync/Humata/.skills/copilot/verify-hello` (and verify-echo), in the agent's own folder; `~/Humata/.skills` is a link to `.skills-sets/<id>`. No `~/.claude/skills` or `~/.codex/skills` (nothing to sync for those). PASS (VF-5). Evidence vm-after-sync1.txt.
4-5. Remote Copilot session (`--model claude-haiku-5.5 --machine vm-sync`): listed `verify-echo`, `verify-hello`, plus its built-ins. PASS. `session type` with empty text answered the folder-trust prompt and submitted; typing submits by itself. Evidence skills-list1.txt. Model seen: claude-haiku-5.5 (session-state events, model-session1.txt).
6. Edited verify-hello to HELLO-V2-CHANGED; dry-run: one `update`; `--yes`: set id changed, links unchanged, VM text new. Fresh session invoked the skill and answered HELLO-V2-CHANGED. PASS. Model seen: claude-haiku-5.5 (model-session2.txt).
7. Second sync with no change: dry-run `changes: []`, set id identical before/after. PASS (idempotent).

Deferred, not failed: VF-3 (MCP/plugin sync, mcp-config not part of the plan), VF-6 (host-change watcher; the V2 edit still needed an explicit sync).
channel_attempt retry: no dropped link or daemon restart occurred; not exercised.

| flow | result |
| --- | --- |
| Sync skills to a remote machine; remote Copilot sees them as links in its own folder | PASS (VF-5), 7/7 steps |

Findings (new):
- UX (low): re-adding a machine under the same name keeps the old sync state, so the "first sync asks" rule (SK-5) is skipped and a link-up sync runs on its own. Steps: `host remove vm-sync --yes`, new VM user, `host add` same name; dry-run shows `first_sync: false`.
- Harness (not product): remote channel clone still needs the insteadOf workaround on the VM (host `--remote-template` does not apply there). `channel machine add` takes positional `<CHANNEL> <MACHINE>`; the CLI `host remove` is asynchronous, `host add` with the same name right after gives name_taken until the op finishes.

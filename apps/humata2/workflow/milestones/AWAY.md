# AWAY: Keep working with the host closed

- **Status:** in progress (lane AWAY)
- **Depends on:**
  - MACH: away mode is machines without the host
  - ASK: a machine judges requests against a copy of the rules
- **Unblocks:** V1
- **Layer:** L3 · **Lane:** daemon

## Outcome
Close the host. SSH to a machine, `humata login`, `humata attach`, answer a request with `humata inbox`. Reopen the host and see it all merged.

## Exit criterion
Flow §26.10 passes. The AW-7 table runs as a boundary suite. Journals replay in order and are re-checked against the rules; the first answer wins; conflicts follow SL-6–SL-8.
- Contain agents (SEC-6): a contained agent's writes outside its session folder and `shared/`, its reads of `~/.ssh`, the `gh` config and other sessions, and any network but the daemon socket all fail, as a boundary suite on macOS. (D128)

## Read these (pointers, nothing else)
- SPEC: AW-1–AW-12, SL-5–SL-8, SEC-10–SEC-13, SE-24, SE-25, TP-9, ST-6; flow §26.10. Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: none
- Decisions: D3, D50, D14 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `daemon/humatad/src/cli_login.rs`, `daemon/humatad/src/attach.rs`, `daemon/store/src/journal.rs`, `daemon/humatad/tests/cli_login.rs`, `cli/src/login.rs`, `cli/src/commands/inbox.rs`, `cli/tests/login.rs`
- API: `api/CLI.md`, `api/BOUNDARIES.md`
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
Script tokens (SEC-14): ASK. The web client (ANYHOST).

## Lane (written by the manager)
- Design notes (Fable verdict folded in, 2026-11-16):
  - Link: new `DaemonService.PeerUplink` bidi stream on the relay allowlist, honoured only for `Caller::User(UserVia::Host)`. Host reachable exactly while it is open (newest replaces oldest); the Guard's machine Context reads that live flag. `GetServerInfo.host_reachable` plus a feed event so tests wait on state, not sleeps.
  - Policy: the host sends a PolicySnapshot (plus passphrase verifier, or an explicit "none set") before relaying any session start and on every change; the machine stores it as one proto row and loads it through `Policy::from_state`. No snapshot: default rules, away rule Wait, login refused. Local AW-12 grants are an overlay until `journal_through` covers them. GRPC §8 and §9.4/§9.7 wording amended (machine holds the verifier; journal is the one ordered channel).
  - Guard effects: WaitForHost holds the call until the stream returns, then re-runs it; Journal/LastCopy get handlers instead of the internal error.
  - Journal: an explicit per-method registry (decode to `Call`, journal kind, base revision). Methods not listed: forwarded when reachable, `host_unreachable` away. Journal only for clients sending an opt-in header (MCP, `humata`). Same idempotency key forwarded and journaled; host dedupes. Last-copy cache keyed (session, method, request), re-judged, bounded. Journal epoch guards store resets.
  - Inbox: Wait asks stay in the machine Inbox; requests, local answers and withdrawals go up only as journal entries (`inbox.request`/`inbox.answer`/`inbox.withdrawn`). Host Answer on a machine-held request goes down and waits for the machine's verdict; link down means refuse.
  - Login: away-only passphrase over the TTY; `UserVia::CliLogin` carries the presence method; lockout counted in the store before verifying; argon2 in `spawn_blocking`. Reachable: "log in from the Mac, or wait until it is away".
  - Replay: seq order, re-check, SL-6–SL-8 via the board store's history/lost-conflict/tombstones; "applied through" per machine in the same transaction; AW-9 line through the queued-message path.
  - Contain: strict SEC-6 profile; the model-API and agent-config conflict goes to DECISIONS, not widened silently; Contain on with a Linux machine refuses the launch with a message. Wired through `daemon/sessions/src/start.rs` (check resume/rebuild/revive).
  - Cut (to log): provisional ids (a journaled create returns no usable id), MachineUser forwarding, login while reachable, the `LocalAnswer` oneof, queued host answers, re-attaching a pending forwarded Ask after a drop (`host_unreachable`, retryable).
  - Order: link+policy, then flow §26.10 (SetApprovalPassphrase, login, inbox journal, replay), then AW-7 rows 2/3/6 (board registry, forwarding, last copy); containment in parallel.
- Claims: `daemon/contain/**` (new), `daemon/passphrase/**` (new), `daemon/humatad/src/{peer,peer_client,peer_link,guard}.rs`, a new `daemon/humatad/src/away/` module, `daemon/humatad/tests/away_*.rs`, `daemon/humatad/tests/flow_26_10.rs`.
- Tasks:
  - [x] T1 Contain agents sandbox wrap (SEC-6) · worker Haiku high · files `daemon/contain/src/*` · tests `daemon/contain/tests/boundary.rs` · green 2026-11-16
  - [x] T2 Approval passphrase hash, generate, lockout (SEC-13, AW-5) · worker Haiku high · files `daemon/passphrase/src/*` · tests `daemon/passphrase/tests/passphrase.rs` · green 2026-11-16
  - [x] T3 PeerUplink via the relay, dynamic host-reachable Guard, Guard effect handlers and waits, host_reachable in GetServerInfo · green 2026-11-16 (the link outlives its machine's daemon; peer_stdio probe flake was load)
    - Plan: `away/link.rs` HostLink (watch state + generation; host role always reachable; newest open replaces oldest); `away/uplink.rs` Uplinks::open refuses all but `UserVia::Host`, publishes HostLinkChanged; Guard judges `UserVia::Host` callers as reachable, others by the live link; Journal/LastCopy → host_unreachable until T5; `check_held` waits on WaitForHost; peer.rs opens PeerUplink on the proven channel before sending Resume and pipes both ways.
    - Workers: A (Haiku high) `away/` + `guard.rs`; B (Haiku high) `peer.rs` + `tests/away_link.rs`.
    - Seams: `scratchpad/away/seams_t3a.py` (proto + stand-in, applied) then `seams_t3b.py`; was `seams_t3.py` (daemon.proto rpc + `host_reachable = 9`, events.proto `HostLinkChanged host_link = 28`, events.rs, relay.rs `proven_may_call`, relay_auth.rs, daemon.rs `with_uplinks`/`peer_uplink`, serve.rs, main.rs, host_relay.rs stub).
    - D213 (provisional): machine starts away until a PeerUplink opens; host's relayed calls judged reachable; PeerUplink is the link's own call, not on the relay allowlist (ROUTED can't hold DaemonService); HostLinkChanged on TOPIC_MACHINES to the user.
  - [x] T3b SetApprovalPassphrase on the host (verifier only, snapshot pushed) · after T2 · green, committed 28702e41
    - `away/passphrase.rs`: user only and present (access_service seam), host only (a machine refuses with invalid_field), typed (≥5 words) / generated (returned once) / cleared; Argon2id hash kept under setting `approval_passphrase`, no migration. `updated`/`pending` stay empty until T4's policy copy carries the hash. 6 unit tests.
  - [ ] T4 Policy copy on the machine (store table, snapshot apply, refuse access writes away) · after T3
    - Written 2026-11-16, awaiting gate. `away/policy_copy.rs` `PolicySource`: host `snapshot(machine)` (channels, defaults, grants of that machine's sessions, passphrase hash, Contain agents), machine `apply` (forward only; grants worked out again from their targets as the Inbox restore does; away grants made on the machine kept), kept as setting `policy_copy` (hex), no migration; `restore` at machine start. Guard: copy revision watch (ms clock, bumped on any policy change and by `policy_changed`). peer_client sends the copy on Up and on each change; the machine acks `policy_revision`. Access writes on a machine already fail NeedsHost. Seam `scratchpad/away/seams_t4.py` (proto 7-10, serve, machine_service, access_service, settings_service). Tests: policy_copy unit (7), guard unit, `tests/away_policy_copy.rs` (2).
    - Left: `updated`/`pending` in SetApprovalPassphrase from the acks; owned_items (T5); host-side send covered end to end only in T9.
  - [ ] T5 Away journal of agents' host-state calls; LastCopy reads · after T3, T4
  - [ ] T6 Passphrase `humata login` on a machine while away; `humata inbox` answers there · after T2, T4
  - [ ] T7 Replay on reconnect: order, re-check, SL-6–SL-8, first answer wins · after T5
  - [x] T8 Wire Contain agents into launch/resume · after T1 · green, committed 28702e41 · D217 (network 443+DNS, own agent state, home hidden, TMPDIR, fail closed on Linux)
    - Contain crate per D217: worker Haiku high (agent_state, read_only, temp, network, `home_rules`).
    - Claims granted 2026-11-16 (settings_service contain sections, revive hook, humatad `contained.rs`, restart.rs field). Contain crate: worker done, plus `also_readable` (hook files in the hidden state folder) and the D217 Linux message. `humatad/src/contained.rs` (settings keys `security.contain_agents`, `channel.<id>.contain_agents`; `Contained` implements revive's `Contain`). Script `scratchpad/away/seams_t8.py` (--check ok): BUILD deps, revive trait/`ContainRefused`/`ReviveError::Contain`, wrap in `start_holder`, 4 `ReviveConfig` literals, main.rs, serve.rs, settings_service.
    - Follow-ups, written 2026-11-16, awaiting gate:
      - state paths match whole components only (`^<path>(/|\.|$)`), not `.claude-backup`
      - a refused contained start answers FailedPrecondition (`scratchpad/away/seams_refused.py`, --check ok)
      - D217 amendment: `contain/src/claude.rs` `seed_claude_copy` (copy at `<session>/.humata/claude`, `CLAUDE_CONFIG_DIR`, empty `CLAUDE_SECURESTORAGE_CONFIG_DIR`, `projects` linked to the real one, own project folders writable, reseed each start, own project entry kept); `Containment.env`; `contained.rs` `own_state`
    - Left: `Session.contained` (needs the launch recorded as contained; no store field yet), the setting reaching machines (T4 snapshot), `<session>/.humata/` not git-excluded, the Info card wording (UI).
  - [ ] T9 AW-7 boundary suite and flow §26.10 on the remote rig · last
- Waiting on: main for seams (argon2 workspace dep, BUILD wiring of the new crates into humatad, proto fields, store migration numbers, `not_built.rs`).
- Build gates: `contain:boundary_test` and the passphrase tests green (T1, T2)
- Ready to commit: no

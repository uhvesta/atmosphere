# AWAY: Keep working with the host closed

- **Status:** done (lane AWAY, closed 2026-11-16)
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
  - [ ] T4b (D220, Fable pick B) On a machine the user reads the rules it judges by from the policy copy, reachable or away: no `revision`, a new `copy` stamp (`PolicyCopyStamp`, revision + host time); never sent a copy reads the defaults unstamped; guard, policy and every write unchanged. Written 2026-11-16 (manager 3), awaiting gate: one script `scratchpad/away/seams_d220.py` (--check ok; proto, access_service, policy_copy `kept_stamp`, live inbox literal, 2 tests in `away_policy_copy.rs`, D220, SPEC AW-7 row, GRPC §9.3 row). Failing first: both new tests fail without the access_service hunks.
  - [x] T5 Away journal of agents' host-state calls; LastCopy reads · after T3, T4
    - (D225) A machine journals its Inbox and sends the journal to its host. Failing test first (`tests/away_inbox.rs`, 4 tests: a request answered on the machine with the passphrase login reaches the host as request then answer and an ack ends the resend; a pending one is sent and new ones follow live; unacked entries are sent again after a restart; the host's Resume skips what it holds). away/journal.rs appends on every Inbox commit on a machine (unit tests), the uplink sends after the host's Resume and on each write, and Resume/Ack journal_through ack the store. The first test also covers T6b (`humata inbox approve` on the machine with the passphrase login). Committed with D225.
  - [x] T6 Passphrase `humata login` on a machine while away; `humata inbox` answers there · after T2, T4
    - T6a (D223) The approval passphrase logs a terminal in on a machine while its host is away. Written 2026-11-16 (manager 3): failing test `tests/pending/away_login.rs` (7 tests: login, wrong try, five-try lock kept across restart, count kept across restart, refused while reachable, refused with no hash, `--json` can't prompt); seam `scratchpad/away/seams_t6.py` (--check ok: serve.rs host link into CliAuth, test dep on passphrase, D223, moves the test). Plan: BeginCliSession on a machine away → presence_required[passphrase]; proof → lockout counted in settings `approval_lockout` before verify, one at a time, spawn_blocking; Accepted → grant with `UserVia::CliLogin.method = ApprovalPassphrase`; Wrong → presence_required; Locked → passphrase_locked. Granted; written by a Sonnet worker (away/lockout.rs, cli_login.rs, identity.rs, status.rs, guard.rs) and reviewed by the manager (a new passphrase from the host clears the count). The first gate hung: the test terminal never read its screen, and on macOS an exiting shell waits for its pty's unread output. The AWAY test terminal (tests/common/away.rs) now reads its screen as it fills and keeps both pty ends out of children; away_login uses it. Committed 0fdac687.
    - [x] T6c (D217 a, Fable) A contained launch's sign-in. It never copies .credentials.json and never touches the keychain login. It tries CLAUDE_SECURESTORAGE_CONFIG_DIR (tested per Claude Code version), then a token or API key the user set for the session, and otherwise fails closed: the session stays stopped with D217's message. Failing test first, temp HOME only.
    - [x] T6d (D217 b, Fable) Flow-back: only the agreed list flows back. The copy goes with the session through archive and restore. At the session's next uncontained start, its own todos and rewind checkpoints (regular files, matched by session id) move into the real config. Failing test first, temp HOME only.
    - [x] T6b `humata inbox` answers the machine's held requests with that login: covered by `tests/away_inbox.rs` test 1 (D225).
  - [x] T7 Replay on reconnect: order, re-check, SL-6–SL-8, first answer wins · after T5 · green
    - T7a the host replays a machine's Inbox journal into mirrors, credits the machine's user, keeps an applied-through mark (D226 picks 2–5, no grants), 051a9eaf.
    - T7b a host answer to a mirror is relayed to the machine and confirmed (`inbox/relay.rs`, `tests/away_replay.rs`); T7c Always allow makes the host's grant at replay, the machine drops its own on ack, and the mark is forgotten with the machine.
    - Follow-ups: a machine's refusal of a relayed answer shows as a 10 s "unconfirmed" (no proto refusal message yet); the outstanding-relay record leaks on timeout; no host-plus-machine e2e for relay offline or timeout.
  - [x] T8 Wire Contain agents into launch/resume · after T1 · green, committed 28702e41 · D217 (network 443+DNS, own agent state, home hidden, TMPDIR, fail closed on Linux)
    - Contain crate per D217: worker Haiku high (agent_state, read_only, temp, network, `home_rules`).
    - Claims granted 2026-11-16 (settings_service contain sections, revive hook, humatad `contained.rs`, restart.rs field). Contain crate: worker done, plus `also_readable` (hook files in the hidden state folder) and the D217 Linux message. `humatad/src/contained.rs` (settings keys `security.contain_agents`, `channel.<id>.contain_agents`; `Contained` implements revive's `Contain`). Script `scratchpad/away/seams_t8.py` (--check ok): BUILD deps, revive trait/`ContainRefused`/`ReviveError::Contain`, wrap in `start_holder`, 4 `ReviveConfig` literals, main.rs, serve.rs, settings_service.
    - Follow-ups, written 2026-11-16, awaiting gate:
      - state paths match whole components only (`^<path>(/|\.|$)`), not `.claude-backup`
      - a refused contained start answers FailedPrecondition (`scratchpad/away/seams_refused.py`, --check ok)
      - D217 amendment: `contain/src/claude.rs` `seed_claude_copy` (copy at `<session>/.humata/claude`, `CLAUDE_CONFIG_DIR`, empty `CLAUDE_SECURESTORAGE_CONFIG_DIR`, `projects` linked to the real one, own project folders writable, reseed each start, own project entry kept); `Containment.env`; `contained.rs` `own_state`
    - Done since: the setting reaches machines in the policy copy (PolicySnapshot 9–10), `<session>/.humata/` is git-excluded at each contained launch (ce242d25), and a machine with no copy yet doesn't start agents uncontained (seams_containcopy). Moved to POLISH (D266): `Session.contained` and the Info card wording.
  - [x] T9 AW-7 boundary suite and flow §26.10 on the remote rig · last · green (`away_boundary_test`, `flow_26_10_test`)
    - Built rows 1, 3 (questions), 4, 5, 6 (Skip), 7 in `tests/away_boundary.rs`; flow §26.10 in `tests/flow_26_10.rs` (rig, plus the away harness). Pending rows in `tests/pending/away_boundary_pending.rs`, built per D227.
  - [x] T10 (D227) AW-7 rows 2, 3, 6 on a machine · after T7, T9 · green
    - [x] T10a Row 6: a call that needs the host waits for it under Wait (`Guard::held`, RequestAccess, start/fork/move/push). Seam `seams_row6.py`.
    - [x] T10b A machine forwards its agents' board calls; the host replays board writes with SL-6–SL-8. Seam `seams_fwd.py`.
    - [x] T10c The machine journals allowed board writes while away (`owned_items`, fallible append). T10d The last copy (forwarded read responses, `humata-stale-as-of`).
    - [x] AW-9 line (D267): a journaled write the host's rules now refuse is kept as lost in the item's history, and the session shows what was left out under the agent's terminal (`away_left_out`, `tests/away_left_out.rs`). The Machines feed reports each policy copy a machine keeps (`PolicyCopyKept`).
    - [x] Approved-Ask slice (D268): linked, the host holds the request and the machine forwards the agent's wait and withdraw; away, the machine holds it and journals the approved write with `approved_request`, which replay checks against the host's mirror; writes kept under Allow low-risk replay while the rule still lists them (`tests/away_forward.rs`, `tests/away_board_journal.rs`, `tests/away_approved_board.rs`).
- Waiting on: nothing.
- Build gates: the close gate (`scratchpad/away/close_gate.sh`: every away_* test, flow §26.10, contain boundary, store, revive, mcp, clients live, ui, native, lint) green.
- Ready to commit: yes

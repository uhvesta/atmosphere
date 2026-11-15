# Humata 2: decisions backlog

Decisions that come up during the build go here. Each one gets the question, the options, the provisional pick and why, what it affects, and how to reverse it. Work carries on with the provisional pick, and the open ones are asked together at the end of each wave. Irreversible or outward actions are never taken provisionally.

Status: **open** (waiting for the user) · **decided**.

---

## D1: Contain agents at the OS level (REVIEW C1). Open
- **Problem:** agents run as the user's uid, so they can read other clones, `~/.ssh`, the gh token and Humata's store, and open sockets, all without going through Humata. MCP rules only gate calls made through Humata.
- **Options:**
  - (a) An opt-in "Contain agents" OS sandbox per session: macOS sandbox profile, Linux namespaces with Landlock.
  - (b) Contained by default.
  - (c) Accept the gap and document it.
- **Provisional:** (a), opt-in, plus a plain statement in the UI that uncontained agents can read anything the user can.
- **Affects:** I7 daemon, the network rule, q7.
- **Reverse:** flip the default.

## D2: Who is the CLI (REVIEW C2). Open
- **Problem:** an agent can run `humata inbox approve` on its own requests.
- **Provisional (the reviewer's design):**
  - inside a session's process tree = that agent;
  - outside = an unverified user, who can only read local state;
  - writes and user-only calls need presence: confirm in the app, `humata login`, or an approval passphrase while the host is away;
  - scripts use scoped tokens (`CliAuthService`).
- **Affects:** api/CLI.md, I8.
- **Reverse:** a simpler presence rule.

## D3: Rules while the host is away (REVIEW C3). Open
- **Provisional (GRPC.md §9):**
  - the machine judges calls from a copy of the host's rules;
  - anything that needs another machine waits;
  - inbox answers are decided by the first answer, not the last writer;
  - every journaled write is checked again on the host at replay.
- **Affects:** SL-5/6, TP-9, AW-4.

## D4: Long waits vs MCP client timeouts (REVIEW C4). Open
- **Provisional:** `wait_seconds`, then a `pending` result, then `inbox_wait`. Once approved, the action runs even if no agent call is still waiting.

## D5: What may cross the bridge (q7, REVIEW C5). Open
- Blocks `network_connect`.
- **Provisional:** nothing crosses except Humata's own gRPC and file transfers. Port forwards are an explicit, approved, per-session grant.

## D6: Access-rule changes are user-only (REVIEW H2). Open
- **Provisional:** yes. Remove the ability to set "change access rules" to Allow.

## D7: Detecting the end of a turn (REVIEW H4). Open
- Needed for comment queueing, status and IN-12.
- **Provisional:** per-agent hooks where the agent offers them (Claude and Copilot hooks), with idle-output heuristics as a fallback, shown as "probably idle".

## D8: The GUI framework (OVERVIEW I2). Decided 2026-11-14: gpui-kit
- **Owner:** go with gpui-kit. Only macOS screenshots matter, so the Linux pixel-snapshot condition is dropped. GPUI gets optimised ourselves if the numbers need it.
- Research in `research/GUI.md`. The owner leans toward gpui-kit.
- **What gpui-kit is:** Longbridge's renamed gpui-component. It covers the tree, virtual list, diff, editor, markdown, palette, docks and multi-window. There's no terminal component.
- **Provisional:** gpui-kit, kept only if the spike shows:
  - near-zero idle CPU;
  - p99 scroll at or under 16 ms on the 20k-line diff on Linux;
  - a Linux headless pixel snapshot within about 3 days of extra work;
  - a decision on right-to-left text.
- **Fallback:** Iced, then egui.
- **Risks:**
  - GPUI's headless screenshots are Metal-only;
  - higher CPU in one benchmark;
  - no evidence of right-to-left support;
  - one maintainer republishes GPUI to crates.io;
  - GPUI runs its own executor, not tokio.
- **Reverse:** reducers stay framework-free, the store bridge lives in one crate, and the terminal and diff sit behind our own interfaces.

## D9: The earlier H2-0/H2-1 code (OVERVIEW I0). Open
- `apps/humata2/app` (Swift), `bindgen`, `core`, `rehearsal`.
- Deleting them is irreversible, so it isn't done provisionally.

## D10: The terminal engine (OVERVIEW I6). Decided 2026-11-14: libghostty
- The VT core is libghostty-vt: parsing, screen state and scrollback, cross-platform. It is drawn by our own GPUI terminal view.
- Humata 1's Zig-in-Bazel setup for GhosttyKit is reused to build it.
- Fallback: alacritty_terminal behind the same interface.

## D11: Deleting a channel (REVIEW H7). Open
- **Problem:** the spec can create, rename and change a channel, but nothing removes one.
- **Options:**
  - (a) **Delete channel…** sends everything of the channel to trash, after the unpushed-work check; restorable for 30 days.
  - (b) Only a channel with no sessions (live or archived) can be deleted.
  - (c) No deleting; archive every session and leave the empty channel.
- **Provisional:** (a). It matches how sessions are deleted (LD-5) and nothing is lost for 30 days. User-only; agents can never delete a channel.
- **Affects:** CH-24, CH-25, LD-6, the channel page's menu.
- **Reverse:** narrow it to (b) by refusing while the channel has sessions.

## D12: Secrets in the conversation mirror (REVIEW H8). Open
- **Problem:** the mirror copies every transcript to the Mac (LD-8). Agents often print environment values and tokens, so host variable values reach the Mac inside transcripts, against SEC-1.
- **Options:**
  - (a) Before a mirror chunk leaves the machine, replace every exact host variable value in it with `‹NAME›`. Keep the mirror as long as the session record, and delete it with the session.
  - (b) (a), and also mask strings that look like tokens (known prefixes, long high-entropy strings).
  - (c) No scrubbing; document it.
- **Provisional:** (a). Exact values are a check a test can make; pattern masking guesses and can hide real output. The spec says plainly that other secrets an agent prints are mirrored as printed.
- **Affects:** LD-21, LD-22, SEC-1.
- **Reverse:** add pattern masking as an opt-in setting.

## D13: When a comment counts as addressed (REVIEW M5). Open
- **Problem:** SM-11 marks a comment addressed "when the agent changes its passage". Reformatting, moves and nearby edits make that ambiguous.
- **Options:**
  - (a) Addressed when an agent change after sending touches any of the comment's anchored lines, ignoring changes that only alter whitespace. A passage moved unchanged is re-anchored, not addressed.
  - (b) Addressed when the quoted passage no longer appears in the file.
  - (c) Never automatic: the user marks each comment addressed.
- **Provisional:** (a). It follows the diff the user sees; (b) misses edits that keep the quoted words.
- **Affects:** SM-11, SM-16, RV-25.
- **Reverse:** switch the rule; comment statuses are recomputed from history.

## D14: Several writers on one terminal (REVIEW M11). Open
- **Problem:** the app, `humata attach`, `session_type` and queued comments can all write to one terminal at once.
- **Options:**
  - (a) Allow several writers, as tmux does. Each writer sees how many others are attached with input. `humata attach --read-only` only watches. Queued messages are pasted whole at a turn end.
  - (b) One writer at a time; others are read-only until they take over.
  - (c) Allow it but ask before a second writer starts typing.
- **Provisional:** (a). It is what users of tmux expect, and the count makes a second writer visible.
- **Affects:** SE-24, SE-25, AW-2.
- **Reverse:** make new attaches read-only by default, with a take-over action.

## D15: How the gRPC services are carried (OVERVIEW I7). Open
- **Question:** the protos define gRPC services. How do the app and the daemons reach each other?
- **Options:**
  - (a) Plain tonic servers on a Unix socket per machine (0600). SSH, or a provider's connect command, carries the bytes to that socket.
  - (b) Our own framing over SSH stdio, like Humata 1.
  - (c) tonic over TCP with TLS.
  - (d) An in-process SSH client (russh) opening the channels itself.
- **Provisional:** (a).
  - **The daemon:** `humatad2` is a stock tonic server on `~/.humata2/humatad.sock`. It runs on its own, so it survives SSH drops.
  - **The app:** one tonic `Channel` per machine, through a connector:
    - this Mac: the local socket directly;
    - over SSH: streamlocal forwarding to the remote socket;
    - any provider: its `connect … {cmd}` runs `humatad2 bridge`, which pipes stdio to the socket.
  - **Reconnecting:** the Channel calls the connector again, and streams resume from a cursor.
  - **Why:**
    - least custom code (only the connector);
    - one multiplexed HTTP/2 connection per machine, with flow control per stream;
    - no TCP port anywhere;
    - local and remote share one code path;
    - it works with any connect command.
- **Affects:** I7, I8, and a prost/tonic toolchain for Humata 2 that leaves Humata 1's alone.
- **Reverse:** the app reaches services only through generated client traits, so changing the transport touches one crate.

## D16: The GUI spike builds with cargo first (OVERVIEW I2). Open
- **Question:** gpui-kit pulls in a large tree with Metal build scripts. Bring it into Bazel through crate_universe now, or after the spike?
- **Provisional:** cargo, in `spike/gui`'s own workspace, until the spike's numbers keep gpui-kit. Then it moves to crate_universe, with the Metal shader build scripts handled there, before A1.
- **Reverse:** none needed. It's the order of work.

## D17: The new GUI binary is `desktop`, not `app` (OVERVIEW I0). Open
- **Why:** `apps/humata2/app` holds the earlier Swift code, which waits on D9.
- **Reverse:** rename the package once D9 is decided.

## D18: How Linux suites run (OVERVIEW rules). Decided 2026-11-14
- **Owner:** all tests run on this Mac. Linux runs in the `humata2-linux` Lima VM (2 CPUs, 4 GiB, 20 GiB disk), batched every few milestones.
- **How:** `testing/linux/run.sh` cross-compiles tests to static musl and runs them in the VM.

## D19: Dependency freshness. Decided 2026-11-14
- **Owner:** use the latest versions, Rust 1.100, and nothing published less than 3 days ago.
- **How:** `.cargo/config.toml` sets `global-min-publish-age = "3 days"`.

## D20: The network rule offers no Allow (I0.2 boundaries). Open
- **Problem:** IN-17 gives "Open a network connection through the Mac" the usual Allow / Ask me / Never. HO-11 says a port forward opens only when the user approves it, low-risk rules never cover it and it can't be approved while the Mac is away. Allow would open one with no approval.
- **Options:**
  - (a) The rule offers Ask me and Never only. "Always let …" on its card is the user's standing approval for that machine, host and port (IN-22).
  - (b) Keep Allow and say it overrides HO-11.
  - (c) Keep Allow but treat it as Ask me.
- **Provisional:** (a). It is the only reading that keeps HO-11 true; (c) shows a choice that does nothing.
- **Affects:** IN-17 (the row's choices), HO-11, HO-12, AW-7; `CAPABILITY_NETWORK_REACH`.
- **Reverse:** add Allow to the row and drop the check that refuses it.

## D21: Actions the IN-17 table has no row for (I0.2 boundaries). Open
- **Problem:** some agent calls cross a boundary but match no IN-17 row: reading comments sent to its own session, reading the Global board apart from other channels, restoring or rebuilding an archived session, archiving another session, and watching another session's terminal read-only.
- **Options:**
  - (a) Fold each into the nearest row: comments to its own session are a read of its own session (IN-16, no rule); the Global board and other channels share the row "Global board, or other channels and their boards: anything"; restore and rebuild go through the start or fork rows for that machine; archiving another session goes through "Stop or delete any session but itself"; a read-only watch goes through "Read another session's files".
  - (b) Add a row for each, with Ask me as default.
  - (c) Refuse each to agents until the spec names a rule.
- **Provisional:** (a). Each match has the same reach and risk as the row it joins, and the table stays short.
- **Affects:** IN-16, IN-17, SE-25, SL-7, BD-*, `CAPABILITY_GLOBAL_READ`, `CAPABILITY_FILES_READ_OTHER_SESSION`, `CAPABILITY_SESSION_START_*`, `CAPABILITY_SESSION_STOP_OTHER`; the deprecated `CAPABILITY_GLOBAL_BOARD_WRITE` and `CAPABILITY_OWN_COMMENTS_READ`.
- **Reverse:** add the rows (b) and give each its own capability; grants made under the folded rows stay with the row they were made under.

## D22: Who draws the terminal (OVERVIEW I6). Decided 2026-11-14
- **Owner:** we write our own terminal view, built only on Ghostty (libghostty-vt), gpui-kit and GPUI. No third-party GPUI terminal widget (gpui-terminal, gpui_xterm, gpui-ghostty).
- **How:** `ghostty_vt` (D10) parses and keeps the screen. A small `terminal/grid` crate holds the cell types and the `Grid` trait, so the view depends on no engine. `terminal/view` draws any `Grid` the way Ghostty's own renderer does: libghostty-vt's render state says which rows changed, only those rows are reshaped, backgrounds are painted as runs and text as shaped runs per style.
- **Tests:** the view is tested headlessly with a fake `Grid` (view snapshots of rows, cursor and selection), with image snapshots of tricky output on macOS, and with a frame-time bench.
- **Reverse:** a third-party view could still draw our `Grid`, since the view only reads that trait.

## D23: Where presence comes from when a request has no `PresenceProof` (I5 testing). Open
- **Problem:** many user-only or unverified-write RPCs (`SetMarks`, `ReorderPinned`, `SendQueuedNow`, `OpenMachineShell`, `OpenShellTab`, `Attach` with input, `SetViewState`, comment writes, `CreateItem`) have no `PresenceProof` field.
- **Options:** (a) presence belongs to the connection (CLI login or a connection-level confirm), with the field used where it already exists; (b) add the field to every such request.
- **Provisional:** (a). No proto change, and it matches SEC-11.
- **Affects:** SEC-8..11, BOUNDARIES §6, TESTING §4 and §6.
- **Reverse:** add `PresenceProof` to each request (additive).

## D24: When `offset_trimmed` applies (I5 testing). Open
- **Problem:** errors.proto has `offset_trimmed` (`OUT_OF_RANGE`, exit 11), but `Attach` falls back to a snapshot and `Subscribe` to `Resync`.
- **Provisional:** `Attach` and `Subscribe` never fail with it; only offset resumes of `ReadFile`, `WriteFile` and `Upload` do. That keeps TR-4's fresh-screen promise.
- **Reverse:** list which streams fail instead.

## D25: The MCP error shape follows the proto (I5 testing). Open
- **Problem:** MCP.md uses `retry.after` as a date-time and names like `denied.rule`; the proto has `RetryAdvice{after: Duration, same_key}`, `Denied{source, capability, …}` and `RevisionConflict{expected, actual}`.
- **Provisional:** MCP `details` use the proto's field names; enum values are lowercase without the prefix (`never_rule`); `after` is now plus the duration; `same_key` is included. One source of truth.
- **Reverse:** rename the proto fields to match MCP.md.

## D26: CLI exit codes for kinds the table doesn't list (I5 testing). Open
- **Problem:** CLI.md §7 has no row for server-side `invalid_field`, `outside_humata_folders` or `peer_version_unsupported`.
- **Provisional:** `INVALID_ARGUMENT` kinds exit 2; the other precondition and `UNIMPLEMENTED` kinds exit 7; the table lists every kind, so the exit-code test can be complete.
- **Reverse:** assign different codes.

## D27: Script tokens and Ask-me capabilities (I5 testing). Open
- **Problem:** TRACE.md says a token can't wait, so Ask-me is refused, but the CLI.md §8 example gives a token `channel_board_edit_others` (Ask-me by default), and no `Denied.Source` fits a token-scope refusal.
- **Options:** (a) a capability listed on the token is a standing approval, except Never and user-only ones, with a new `SOURCE_TOKEN_SCOPE`; (b) refuse Ask-me capabilities when the token is created.
- **Provisional:** (a). The user approves the scope when making the token, and the CLI example stays valid.
- **Reverse:** refuse them in `CreateScriptToken` with `invalid_field` and fix the example.

## D28: `WhoAmI` describes scripts and presence (I5 testing). Open
- **Problem:** `WhoAmI` can't tell a script from a user, or an unverified CLI from a logged-in one.
- **Provisional:** add `ScriptActor` to the `Actor` oneof and a presence state to `WhoAmIResponse` (additive).
- **Reverse:** drop them and the TESTING §6.3 rows that use them.

## D29: Idempotency edge cases (I5 testing). Open
- **Problem:** reusing a key with a different request is unspecified, and the MCP key (JSON-RPC id plus launch nonce) collides if `humatad mcp` restarts within one launch.
- **Provisional:** a reused key with a different request fails with `invalid_field` naming `idempotency_key`; each MCP process adds its own nonce.
- **Reverse:** document another collision rule.

## D30: Image paste over remote sessions (I6, user request 2026-11-14). Decided 2026-11-14
- **Owner:** copy, paste and image paste work for local and SSH sessions alike.
- **How:** text paste goes through Ghostty's bracketed paste; an image paste is uploaded with `TerminalService.PasteImage` into the session's paste folder on its machine (0600, removed with the session) and its path is pasted. Remote programs' OSC 52 clipboard writes reach the Mac clipboard; OSC 52 reads are never answered without the user.

## D31: Bug reports as infrastructure (I9, user request 2026-11-14). Decided 2026-11-14
- **Owner's ask:** reports are text an agent can understand, not videos or pictures. A report holds the event log and enough context to recreate what happened and where.
- **Kinds:**
  - a **recording**: interactions, actions with state diffs, view and layout tree changes, and daemon RPCs between start and stop;
  - a **snapshot**: state, view and layout trees at one moment.
- **How:** it reuses the I3/I4 recorders, so the event format is the flow format and a recording replays as a flow test. Elements are named by role, label and tree path. Secrets are scrubbed before anything is written.
- **Affects:** I9, the gate G (now I1–I9), the CLI (`humata report`), and I3's layout snapshot, which reports depend on.
- **Reverse:** drop I9 from the gate. Nothing else depends on it.

## D32: Agent identity — local OAuth or one MCP per agent (I8, user idea 2026-11-14). Provisional
- **Question:** should agents authenticate to Humata with local OAuth?
- **Options:**
  - (a) Keep what `api/MCP.md` already says: one stdio `humatad mcp` per agent process, bound to its session by a random `HUMATA_SESSION_TOKEN` that the daemon also checks against the holder's pid and start time. This is already one MCP per agent.
  - (b) Serve MCP over HTTP on localhost with OAuth 2.1, which Claude Code and Codex support for HTTP servers. You get a consent screen and revocable tokens, but every agent start needs a browser step. Agents on headless remote machines can't do that, and OAuth adds nothing a pid-bound stdio token lacks.
- **Provisional:** (a). Revisit (b) only if an agent we support can't run a stdio MCP server.
- **Affects:** I8, MCP.md §1.
- **Reverse:** add an HTTP transport beside stdio. Grants and checks stay in the daemon either way.

## D33: Gaps found while writing the access checks (I8, `daemon/access`). Open
Each item is a provisional pick in `humata2_access`. Reversing one is a local change in that crate unless noted.
1. **Script tokens and Ask-me capabilities.** TRACE §2 says a token refuses Ask me, but CLI.md's triage example lists an Ask-me capability. Pick: any capability listed in the token is allowed, because the list is the user's approval; unlisted ones are refused; `NetworkReach` is always refused (HO-11). Reverse: refuse `Tier::AskMe` in `check_script`. (Overlaps D27.)
2. **Proto sources are missing.** `Denied.Source` and `DecisionSource` lack script-token, other-channel, peer-link, user, local-read and review-push. Pick: the Rust enums have them. Reverse: add them to the protos (additive).
3. **What a CLI without presence may read.** The matrix says this machine only; BOUNDARIES §2.2 and CLI §2 say every channel. Pick: local files, session records, channel boards and channel reads; writes and the global board need presence.
4. **User-only calls while away.** Pick: answering Inbox requests with the passphrase, and typing into a terminal of a session on this machine; everything else is `HostUnreachable`.
5. **What Skip applies to.** Pick: GRPC §9.3, so Skip also refuses allowed calls that need the Mac.
6. **How Skip is reported.** Pick: `user_away`; `SOURCE_AWAY_SKIP` is unused.
7. **Asking, while away, for something only the Mac can do.** Pick: it waits (or gives `HostUnreachable` under Skip), and is never answerable on the machine.
8. **"Always let" when one file is read.** Pick: it offers the folder holding the file.
9. **Capabilities with no "Always let" target** (GlobalRead, PrReviewStart, OwnSession*, SessionMove, TrashRestore, ChannelCreate, SessionStopOther, ChannelChange). Pick: they offer none, and approving with one gives `NoStandingApproval`.
10. **Agents reaching other channels.** Pick: reading needs `GlobalRead`; anything else is refused, and the fix says to ask the user.
11. **Agents writing other sessions' or machines' folders.** Pick: refused as a hard limit; the fix says to use its own folders.
12. **Asking ahead.** Pick: refused for capabilities with no "Always let"; it still asks when the rule is Allow.
13. **Token calls with no channel** (global, network, push, machine). Pick: no channel listing is needed.
14. **Deprecated grant targets and capabilities.** Pick: not modelled.
15. **Proto `PathScope` can't say which folder a path is in.** Pick: the Rust type carries the folder kind. Reverse: add a root field to the proto.
16. **Copying into `shared/`.** Pick: the copy-in or local-fetch capability alone governs it.
17. **An agent's claimed machine not matching its session.** Pick: left to the daemon.
18. **Where rules and grants change.** Pick: only on the Mac (AW-8). A grant made on a machine is replayed on the Mac with a new id and keeps its record.

## D34: The app is only a client; many clients at once, and a web client (owner request 2026-11-14). Provisional
- **Owner's ask:** the desktop app is just a UI over the Mac's daemon. It should be possible to ship it as a web app (gpui-kit builds for wasm through `gpui_web`) and to have several open at once, such as one in a browser and one on the desktop.
- **Already in the spec:**
  - AR-2: the app, the CLI and MCP are all clients of one gRPC API.
  - AR-3: the app holds no session state that matters.
  - AR-5: view state lives on the host.
- **Gaps and provisional picks:**
  1. **Many clients at once.** Pick: any number of UI clients connect to the host daemon. Every change streams to all of them, and none of them owns anything. Durable view state (open panels, folds, pane widths) is stored on the host per client kind (desktop or web), with the last writer winning; a client can copy another's layout. Transient state (hover, the current selection, the composer draft) stays in the client. Reverse: one shared view state for all clients.
  2. **Code split.** Pick: everything in `features/`, `ui/` and `terminal/{grid,view}` builds for `wasm32-unknown-unknown`, with no file system, processes, sockets or threads. Those sit behind the `clients` traits, which have a native implementation (tonic over the Unix socket, D15) and a web one. The `desktop` crate does only native wiring: windows, menus and the keychain. A wasm build check runs in CI from I1 onward, so drift is caught at once. Reverse: drop the check; the code is unchanged.
  3. **Web transport.** Browsers can't open a Unix socket or speak plain gRPC. Pick: humatad serves gRPC-web (tonic-web) on a loopback HTTP listener. It is off by default and turned on in Settings. It checks Origin and Host (to block other sites and DNS rebinding) and accepts only paired clients. Reverse: a WebSocket bridge instead; the protos are the same either way.
  4. **Web client identity and presence.** BOUNDARIES §2.1 trusts the signed app through a keychain key, which a browser can't prove. Pick: a browser pairs once, approved from the desktop app or with `humata login` (a short code shown in both). It then holds a revocable per-device key and counts as the user, present, like the app. Pairings are listed and revocable in Settings, and every pairing is recorded. Nothing an agent can do creates a pairing. Reverse: make web clients user-unverified (reads only, writes need presence).
  5. **Terminals in a browser.** Pick: the VT engine stays in the client (D22), read through `Grid`, and libghostty-vt is built for wasm (Zig targets wasm). If that fails, the holder runs the VT and streams grid diffs, behind the same `Grid` trait. Reverse: always stream grid diffs from the holder.
  6. **A self-hosted host (owner, 2026-11-14: "make it possible to host it all and connect to the same thing").** In SPEC as AR-6 to AR-9. Pick: the host role isn't tied to a Mac. A Linux humatad can be the host, and clients reach it over SSH (desktop, CLI) or HTTPS (web) after pairing. Nothing Mac-only (keychain, signed-app check) sits in the host path; those are the desktop client's way of proving it is paired. Reverse: keep the host on the Mac and let other clients only connect to it remotely.
- **Affects:** AR-2/3/5 to AR-9, BOUNDARIES §2.1, GRPC.md (a web listener), LAYOUT (`clients` has native and web implementations), I1–I7, G.

## D37: humata2_mcp picks (OVERVIEW I8). Provisional
- **No rmcp.** A hand-written stdio JSON-RPC server; rmcp 3.10.0 failed the 3-day cooldown and builds schemas by macro. Recommendation: keep ours; revisit if MCP's protocol churns.
- **Ask on ordinary calls** returns `pending` at once, then the server waits (`WaitForAnswer`) with progress keepalives. Recommendation: keep.
- **No bind RPC.** The session token goes as gRPC metadata, checked at connect, behind `Daemon::bind`. Recommendation: keep; add an RPC only if metadata proves awkward over SSH.
- **Session token** is opaque, non-blank text. Recommendation: tighten to 32 hex characters when humatad mints them.
- **A failed bind** keeps the server up: `tools/list` works, every call says `identity_mismatch`. Recommendation: keep.
- **Cancelling a waiting call withdraws its request** (CALL_ENDED), as the agent exiting does. Recommendation: keep.
- **outputSchema is inline** (`type: object`, `anyOf` envelope or error), not `$ref`, so TS SDK and ajv clients accept it. TESTING.md §5.1's `$ref` example needs updating. Recommendation: keep.
- **Bad input is JSON-RPC -32602** with `data {tool, field, problem}` per MCP.md §3, against MCP 2025-11-25's advice to use `isError`. Recommendation: switch to `isError` results, since agents read those and retry.
- **The user-only list lives in the mcp crate**, refused locally with no audit entry. Recommendation: move it to humata2_access as `user_only_operations()` and audit the refusals.
- **Some user-only operations name no capability** (`bridge_close`, `transfer_*`, `view_state_*`, `events_subscribe`, `audit_list`). Recommendation: add capabilities in access.
- **`reason` and `wait_seconds` on every rule-governed tool**, since any rule can be set to Ask. Recommendation: keep.
- **Arguments MCP.md leaves out:** `channel_*` act on the caller's channel; `skills_sync` takes `{machine}`; `access_explain`/`inbox_request` targets mirror GrantTarget; `view_prepare` uses Board; plan-guarded tools take no plan token (the daemon plans and applies in one call). Recommendation: write these into MCP.md.
- **`journal_status` has no RPC.** Recommendation: add one to the protos.
- **`idempotentHint` is false for creating tools**, against MCP.md §6.3's `session_fork` example. Recommendation: keep false and fix the example.
- **Idempotency nonce is per process** (D29); a restarted server makes new keys. Recommendation: have the holder set `HUMATA_MCP_NONCE` so retries survive a restart.
- **Binary:** an `mcp` subcommand on `humatad2`. Recommendation: rename the binary to `humatad` once Humata 1's daemon is gone (D9).
- **Same JSON-RPC id twice** runs both; the daemon dedups by key, and a cancel stops both. Recommendation: keep.
- **A call dropped before the daemon answers** relies on the daemon withdrawing when its call future drops. Recommendation: test that in humatad when it implements `Daemon`.
- **Still to do:** `humatad2 mcp` and a gRPC `Daemon` implementation (JSON↔proto, D25 errors).

## D38: humatad2's MCP wiring, session tokens and access checks (OVERVIEW I7, I8). Provisional
1. **A call with no token** is the user. An agent that unsets `HUMATA_SESSION_TOKEN` passes as the user until a peer-credential check exists. Recommendation: add the peer pid / process-tree check before G; reverse in `identity.rs` `Authenticate::call`.
2. **Tokens** are 32 hex chars from the OS random source, one per session shell tab. Only SHA-256 hashes are kept, in a 0600 `state/session-tokens`; a token is revoked when its tab closes or its spawn fails, and pruned at load.
3. **Metadata key** is `humata-session-token`, marked sensitive and never logged.
4. **A machine-role daemon** assumes it can reach the Mac until the link is built (`Guard::new`).
5. **Policy** is in memory; each channel's default rules are added the first time one of its agents calls. Recommendation: back it with the store in the next daemon milestone.
6. **An Ask verdict outside the Inbox** gives `Unimplemented` `needs_approval` instead of queueing a request. Recommendation: queue it in the Inbox once AnswerRequest is built.
7. **Ask on InboxAsk** proceeds, since asking is itself the Inbox.
8. **AskUser** returns pending at once and ignores `wait_seconds`; the MCP server then calls WaitForAnswer, which sends a keepalive at once and every 15 s.
9. **The Inbox is in memory** and lost on restart. Recommendation: store-backed requests next.
10. **Withdrawal** always records CALL_ENDED, and only the requesting agent may withdraw (the user gets `invalid_field`).
11. **ListTabs with no session** is user-only (TerminalInput, Global).
12. **Type** picks the agent tab, else the earliest shell, and types raw text, not a bracketed paste as the MCP tool description says; TURN_ENDS isn't built. Recommendation: switch to bracketed paste to match MCP.md.
13. **Tools not built yet** return `peer_version_unsupported`; so do CancelQueuedTyping, ListQueued, AnswerRequest and AnswerAll (after the InboxAnswer check). A question can only be withdrawn or time out for now.
14. **`whoami`** reports `policy_as_of: null`, `host_reachable: true`.
15. **A dropped AskUser call** in the moment between the request and the wait leaves it pending.
16. **A daemon started with `HUMATA_SESSION_TOKEN` set** passes it to machine shells. Recommendation: strip it from shells that get no token.

## D39: humata2_machines picks (OVERVIEW I6). Provisional
Each is reversible inside `apps/humata2/daemon/machines`.
1. **Keeping humatad2 running** (PER-18, PER-19): a systemd user service with lingering; where lingering is off, the error gives the one `sudo loginctl enable-linger` command an admin must run. With no systemd, `setsid` now plus a crontab `@reboot` entry, with nothing restarting it if it dies.
2. **The link** runs `humatad2 bridge` over the provider's SSH. humatad2 doesn't have a `bridge` subcommand yet; it is the next daemon piece for I6.
3. **Providers** are TOML command templates (one shipped: `providers/lima.toml`), with an optional `run_on` machine to run them from.
4. **An unknown SSH host key** is `HostKeyUnknown`, never accepted automatically.
5. **Setup checks** read facts from the machine with one shell script (systemd, linger, setsid, crontab and the rest) before choosing how to install.
6. **Still open, picked in code and commented there:** how `{cmd}` is quoted, VM names, the name charset, the baseline environment of remote shells, the identity file, status parsing, registering in hosts.json (left to the daemon), remote macOS (not supported), and what terminals assume of the shell.

## D40: The `humata` CLI picks (OVERVIEW I8). Provisional
1. **Socket**: `HUMATA_SOCKET` alone when set; else `~/Library/Application Support/Humata/humatad.sock`, then the other paths in `src/socket.rs`.
2. **Identity metadata**: `humata-session-token` inside a session, `humata-script-token` for scripts.
3. **`--json`** prints an envelope with `ok` and the result or error. Each reply is rendered from the protocol's descriptors, so one printer covers every message: proto field names, enums lower case without their prefix, single-`value` messages as that value, RFC 3339 timestamps, durations like `"90s"`, bytes base64. Binary protobuf on stdout was rejected (owner asked, 2026-11-14): the CLI's readers are people and `jq`, and programs that want protobuf call gRPC or MCP. Reverse: add `--output binpb`.
4. **`humata report`** says the daemon can't make bug reports yet, because the API has no ReportService (D31). Recommendation: add ReportService to the proto with I9's daemon side.
5. **Errors** map each daemon error kind to an exit code and a message that says how to fix it.
6. **Tests** use an in-process fake daemon on a real Unix socket.
7. **Still open, picked in code:** id prefixes, how waiting and streams print, and where CLI.md and the proto disagree (the CLI follows the proto).

## D41: No credential, no access; admin mode only in test daemons (owner ruling 2026-11-14)
- **Ruling:** "make shell require auth before anything, except in admin mode (for test tooling and stuff, release build shells never run as admin)." The unverified read tier of SEC-9 is gone. A caller with no session token, script token, CLI login or app key is refused everything except `WhoAmI` and `CliAuthService.BeginCliSession`. SEC-9, BOUNDARIES §2.2, CLI.md §2, TRACE §2 and TESTING follow.
- **Admin mode:** humatad2's `admin` Cargo feature plus `serve --admin`. In Bazel only `humatad2_admin` (`testonly`, private) has the feature; the release `humatad2` doesn't contain the code path. With `--admin`, a token-less call is the present user, as the tests need.
- **Provisional picks**:
  1. **Error for no credential:** gRPC `Unauthenticated` with kind `presence_required`, accepted `[CLI_LOGIN]`; no proto change. Reverse: add an `unauthenticated` kind to errors.proto.
  2. **`WhoAmI` with no credential:** `actor` and `channel` unset, `presence_possible` false. Reverse: a `NoCredential` actor.
  3. **Open methods:** only the two above, so `humata status` with no credential fails. Reverse: `OPEN_TO_ANYONE` in `identity.rs`.
  4. **How open methods are marked:** a request mapper outside the interceptor puts a marker in server-side extensions, which a client can't set.
  5. **The feature alone does nothing:** `humatad2_admin` without `--admin` acts as the release daemon.
  6. **Script tokens:** the daemon doesn't check them yet, so a script-token-only call is refused as having no credential.
- **Consequences:** a release daemon serves only agents until `humata login` (SEC-10) and the app's keychain key (§2.1) exist; the app and CLI run against `humatad2_admin --admin` until then. `humata2_access` no longer has the unverified tier: `Principal::Cli` always carries how presence was shown, and the TRACE row for a caller with no credential is tested in humatad2's `tests/auth.rs`, since the policy never sees such a call.

## D42: The machine bridge (OVERVIEW I6). Provisional
- **Built:** `humatad2 bridge [--socket PATH] [--sync NONCE]` pipes stdio to the machine daemon's socket. With `--sync`, it prints `HUMATA_SYNC_<nonce>` first, which is what takeover waits for. If nobody is listening, it waits 500 ms (so a systemd restart isn't raced), then starts `serve --role machine` detached with `setsid`, never `--admin`, and waits up to 10 s for the socket. It ends when either side closes, and adds no identity: the daemon still authenticates every call (D41).
- **Spec conflict, needs the owner.** SPEC MP-7/MP-15 and GRPC.md §2, §3 and §8 say the peer link is `humatad serve --stdio --sync <nonce>`: a daemon serving PeerService on its own stdio, never on a socket. D15(a), D39 and the code use a bridge to the socket. With a bridge, once PeerService exists the daemon can't tell a peer link from a local client, so "PeerService is never on a socket" can't hold, and only Hello's enrollment HMAC would set the peer apart. **Recommendation:** build `serve --stdio` before PeerService, so the peer service is reachable only over stdio. The sync line and copy-loop tests carry over, and the bridge stays for CLI and app clients reaching a machine. Reverse: amend MP-7/MP-15 and GRPC.md to the bridge.
- **Smaller picks:**
  1. **Socket path:** `~/.humata2/humatad.sock`, shared with `serve`; GRPC.md §2 says `<root>/.humatad/humatad.sock`. Reverse: `serve::default_socket`.
  2. **A daemon the bridge starts writes nowhere,** since its stdio must stay off the stream. The error says to run `humatad2 serve --role machine` to see why. Reverse: send its stderr to a log in the state folder.
  3. **A daemon still silent after 10 s is left running,** so the next try can succeed.
- **Machines follow-up, built:** takeover no longer waits forever for the marker.
  1. **30 s by default** (`Takeover::with_sync_timeout` changes it), counted only while waiting for the marker; installing files has no deadline. The caller asks `deadline()` and calls `time_passed(now)` once it passes. Reverse: a per-step deadline.
  2. **A terminal that ends fails the takeover at once,** in any state, with how it exited (`ended(exit)`), so a bridge that dies doesn't wait out the timeout.
  3. **Both errors carry the end of the terminal's output:** at most 4 KiB and 20 lines, which may start inside an escape sequence.
  4. **Only a whole line `HUMATA_SYNC_<nonce>` for this takeover's nonce counts,** so the echoed command and another takeover's marker don't.
  5. **humatad doesn't drive a takeover yet;** it wires `deadline`, `time_passed` and `ended` in when machine onboarding is built.

## D43: `humatad2 serve --stdio`, the peer link (OVERVIEW I6, SPEC MP-7). Provisional
- **Built:** `serve --stdio [--sync NONCE]` serves gRPC on stdin and stdout as one HTTP/2 connection and exits when it closes or on SIGTERM/SIGINT. PeerService is registered only on the link, never on the socket. `--stdio` refuses `--socket` and `--admin`, and `--sync` needs `--stdio`. This follows D42's recommendation; the bridge stays for CLI and app clients.
- **Picks:**
  1. **What the link serves:** DaemonService (role always Machine) plus PeerService, TerminalService, FileService and DiffService for relaying (GRPC.md §2), the last four still "not built". No other client service. Reverse: `not_built::register_peer_link`.
  2. **Every call on the link is the peer,** whatever token it carries, and never the user, even in admin builds. The proto's Actor has no peer case, so WhoAmI returns no actor. Reverse: add a peer actor to `common.proto`.
  3. **Hello checks nothing yet:** no enrollment key exists, so it is "not built" with the rest of PeerService.
  4. **The machines crate still starts `humatad2 bridge`;** it switches to `serve --stdio` once relaying works, and the install step (D39.1) must then keep the socket daemon running.
- **Needs the owner: how the link process lives beside the machine daemon.** PER-7 allows one daemon per machine, owning the holders and the store, but PeerService.LaunchSession and relayed terminals need both. Handing the stdio descriptors to the socket daemon, or a second peer listener, would put PeerService back in reach of local processes. **Recommendation:** (a) the `--stdio` process is a second front end over the shared state: it attaches to holders through their own sockets and takes a store file lock for writes, and PER-7 reads "one socket daemon". Alternative (b): the stdio process is the machine daemon and also binds the socket, which breaks surviving SSH drops unless it detaches when the link closes.
- **Spec gaps found:**
  1. Nothing authenticates the host to the machine: any same-user process can run `serve --stdio` and be the peer. "Never on a socket" holds only against callers that can reach the socket but can't run the binary (sandboxed agents, D1). Hello's HMAC must be mutual, or the link must prove the host.
  2. In a takeover pty, stderr shares the stream, so a late error line would corrupt the link. Fix: point fd 2 at a log after the marker (needs rustix's `stdio` feature).
  3. A first start of the link and the socket daemon at the same moment could make two machine ids. Fix: create the id with an exclusive create and reread.

## D44: Storage is SQLite plus zstd-compressed protos (owner direction 2026-11-14). Provisional
- **Pick:** each daemon keeps its state in one SQLite database (`rusqlite` bundled, WAL, `synchronous=FULL` for anything acked or needed to resume, a `user_version` schema version where newer is refused and older is migrated in a transaction). Structured data gets columns and indexes. Unstructured or rarely-queried data is a serialized proto, zstd-compressed (`ruzstd`, pure Rust, already used by Humata 1), behind a header naming the codec, the message type and the format version. Blobs over about 64 KiB live in content-addressed files referenced from SQLite: the file is written and fsynced before its row commits, so a crash leaves at most an orphan, which a sweep removes.
- **Why:** queries and transactions for records, and cheap compact storage for everything else. The header lets the codec change (for example to C zstd or LZ4) without a migration.
- **Replaces:** the JSON-files store from earlier milestones.
- **Reverse:** the store crate is the only place that knows the format.

## D45: The peer link proves both ends (fixes D43's gaps). Provisional
- **Built:** Hello answers HMAC-SHA256(key, "humata-peer-machine-v1" ‖ host nonce ‖ machine nonce) with a fresh 32-byte machine nonce; the new `ProveHost` takes HMAC-SHA256(key, "humata-peer-host-v1" ‖ machine nonce ‖ host nonce). Until it succeeds, only Hello, ProveHost and WhoAmI are served; anything else is `peer_unproven`. A wrong host proof closes the link. GRPC.md §2, §3, §8, §10 and BOUNDARIES.md describe it, including the remaining risk: an uncontained agent (D1) can read the key and pose as the host.
- **Picks:**
  1. The key is `<state>/enrollment-key`, hex, at least 32 bytes; it must be a regular file the user owns with no group or other bits, opened without following links. A key with the wrong mode is refused and the fix says to add the machine again, not to chmod it. Reverse: `link_proof::KEY_FILE`, `EnrollmentKey::from_hex`.
  2. New error kind `peer_unproven` (reasons NOT_ENROLLED, KEY_UNSAFE, KEY_UNREADABLE, HOST_NOT_PROVEN, HOST_PROOF_WRONG, OUT_OF_ORDER), all UNAUTHENTICATED.
  3. One Hello per link whatever its outcome; ProveHost out of order is refused without closing the link. Host nonces are 32–64 bytes; the machine's is exactly 32.
  4. GetServerInfo is gated too.
  5. HelloResponse's `booted_at` is the link process's start; `journal_high_water` is 0 and `running` empty until the store is shared (D43). The machine doesn't check `expected_machine` or the host's version; the host decides.
  6. After the sync marker, stderr goes to `<state>/logs/peer-link.log` (0600); the log is opened before the marker so a failure to open it still shows on the terminal.
  7. The machine id is created with a temp file hard-linked into place; a loser of the race reads the winner's id. An empty id file is an error, not regenerated.
- **Not built:** the host side (sending ProveHost, checking the machine's proof, `identity_mismatch`) and the Mac writing the key at enrollment.

## D46: Machine provider picks (OVERVIEW I7). Provisional
1. **An unknown host key is learned** by an ssh connect with all authentication off and `StrictHostKeyChecking=accept-new` into a temp known_hosts, then pinned into Humata's own known_hosts as `humata2.<machine>` once the person confirms the fingerprint. Reverse: `ssh-keyscan` in `Link::scan_host_key`.
2. **A provider's `connect` tool owns its host keys:** its host-key failures are `ConnectHostKey`, without Humata's Confirm host key remedy.
3. **A takeover gives the shell 20 s** to answer before failing with `NoShell` and the end of the output (a login that ends in a menu).
4. **Teardown** removes the service and the two binaries and keeps state, folders and lingering. Reverse: `service::remove_script`.
5. **Service setup kills stray machine daemons** (`humatad2 serve --role machine`, never `serve --stdio` links) and waits up to 10 s; it waits 10 s for the user manager and 10 s plus 2 s for the unit to stay up.
6. **A systemd user service is used** when `systemctl --user` works or `/run/systemd/system` exists; otherwise setsid with a crontab boot line.
7. **Pinning replaces every key for that machine;** the daemon must pin one machine at a time.
- **Gaps:** adding Humata's key during a takeover (MP-17) isn't built; a takeover sets up no service, so an interactive-only machine doesn't survive a reboot; nothing checks that the installed humatad2 runs (no `--version`); the daemon must pass `-F` an absolute path.

## D47: Bug report library picks (OVERVIEW I9). Provisional
1. **Late daemon events** (calls, errors, log lines) are placed by their own time; they change no tree, so moving them is safe.
2. **A replay always compares state;** view and layout only when asked, since a headless window's fonts, size and terminal text can differ.
3. **The fake daemon answers** by machine and method, first unanswered in recorded order, not by request contents (which hold fresh ids and times).
4. **A report is saved at stop or snapshot,** titled with what happened; the form is written into the saved folder later, so a crash with the form open loses nothing. Masked text in the form can't be recovered, because events were scrubbed at save time.
5. **One recording at a time;** a snapshot is allowed while recording.
6. **The folder golden** is one text file per report kind: the folder name, then each file sorted with its mode and contents, recorded with `HUMATA2_SNAPSHOT=record`.
- **Left for the app and CLI:** the title-bar dot, menu and shortcut, an adapter from `Player` to the flow driver, hooks that feed real events in, and a `ReportService` the CLI's `humata report record|stop|snapshot` calls.

## D48: Inbox answers and grants (OVERVIEW I8). Provisional
- **Built:** AnswerRequest and AnswerAll (user only, even for an agent's own request; the first answer stands; a withdrawn request can't be answered; `expected_revision` is checked), RequestAccess (IN-26: an ask a grant already covers is approved at once), ListGrants and RevokeGrant.
- **Picks:**
  1. **Grants don't expire;** SPEC and access.proto define none. Reverse: add an expiry to `Grant` and the proto.
  2. **A grant records who asked, which request, where and when** (`granted_at`, from the daemon's clock).
  3. **The agent gets its grant** as a `humata.v1.Grant` in `WaitResponse.result`. MCP.md names a `grant_id` the proto lacks; add a field to reverse.
  4. **AnswerAll approve** approves access requests once and skips questions (they need a reply) and ask-aheads (approving would add a grant); skipped ids come back.
  5. **Path targets must name a session** (`<owner>/<name>/<path>`); paths outside any session's clones are refused for now.
  6. **Ignored for now:** the request's `presence` fields (presence comes from the connection, D23), idempotency keys, and RequestAccess's `wait_seconds` (it returns pending; the agent waits with WaitForAnswer).
  7. **An agent may list its own session's grants;** the peer link may not.
  8. **Requests and grants live in memory** until the store holds them (D44).
- **Left:** `last_used_at`, ending grants with their session, the MCP `inbox_request` tool calling RequestAccess, Ask verdicts becoming Inbox requests (D38.6), the rules/defaults/Explain/passphrase RPCs.

## D49: Remote tools and scripted syncs (SPEC TS-1–TS-4). Provisional

- **Asked by the owner (2026-11-14):** adding a host must put Humata's CLIs and daemons on it automatically: the cross-platform binaries embedded in the desktop app (macOS and Linux, each arch), as in Humata 1, dumped to a Humata-only path. Other things should be syncable by script.
- **Picks:** humatad2, humata2_holder and humata go to `~/.humata2/bin` on add and on every connect when checksums differ; scripted sync rules reuse the skills sync machinery (preview, secret filter, offline catch-up, atomic set) and provider-style command templates.
- **Open:** whether a sync rule may sync back from a machine (now: one way only, like SK-1).

## D50: `humata login` and `humata logout` (SEC-10, SEC-11). Provisional
- **Built:** `humata login` asks the daemon for a CLI login bound to the caller's terminal session (session id and controlling terminal); the login is confirmed from the app (ListPendingCliLogins, AnswerCliLogin) and its token (`CliSession.token`) is kept in a 0600 file keyed by that terminal session. Calls from that terminal session are the present user until the login ends (12 hours at most) or `humata logout`. A process that leaves the session (`setsid`) isn't covered.
- **Picks:**
  1. **Only the app confirms a login.** Until the app exists, the release daemon can't confirm one; tests confirm with the testonly `humatad2_admin --admin-pid`.
  2. **The login file is keyed by terminal session, not socket,** so one login covers every daemon socket that terminal reaches on this machine. Reverse: key by socket path too.
  3. **A login over the machine bridge is refused;** log in on the machine itself.
  4. **No CLI timeout:** `humata login` waits until the app answers or the user presses ^C.
- **Left:** the terminal-session check keys on pid without a start time (pid reuse is a race); an error kind for "no controlling terminal"; a CLI-login EntityKind for the audit log; `presence_possible` stays false after an admin WhoAmI.

## D51: The app installs the host's tools too (SPEC TS-0, owner direction 2026-11-14). Provisional
- **Picks:** on every launch the desktop app copies its bundled `humatad2`, `humata2_holder` and `humata` into `~/.humata2/bin` by the TS-1 rules, registers the launchd login item and puts `humata` on PATH. There is no separate host installer.
- **Open:** how `humata` gets on PATH (a symlink in `~/.local/bin` vs. a line in the shell profile vs. `/usr/local/bin` with admin rights). Recommendation: a symlink in `~/.local/bin`, plus a one-time hint if that folder isn't on PATH.

## D52: Restarts never lose state, and links push (SPEC PER-25–PER-31, TR-7–TR-10, owner direction 2026-11-14). Provisional
- **Asked by the owner:** turning off the laptop, a remote machine or both must never lose state; when something can't come back, record enough for a skill to diagnose it, repair what it safely can, and propose fixes. Daemons talk by pushing on streams in the background, not polling.
- **Picks:** restart reports reuse the bug report format; the restart doctor is a shipped skill plus `humata doctor restarts`; the store keeps a `VACUUM INTO` last good copy; the only polls are those with no OS notification, each listed with its interval in GRPC.md; keepalive is an HTTP/2 PING at most every 30 s.
- **Open:** the latency budgets in TR-10 (5 ms echo, 10 ms events over the round trip) are first guesses for benches to confirm.

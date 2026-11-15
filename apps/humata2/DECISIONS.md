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
- **Scope (owner, 2026-11-14):** restore where the user was (screen, open sessions and agents, connected machines), not exact scroll positions. A machine may need a manual reconnect; after it, the machine's state is inspected and its sessions recovered.
- **Open:** the latency budgets in TR-10 (5 ms echo, 10 ms events over the round trip) are first guesses for benches to confirm.

## D53: The review pane's picks (SPEC §9 RV-*, SM-*, PER-25, PER-27). Provisional
- **Built:** `apps/humata2/review` (`humata2_review_view`), the session's review pane: a plain model (`model`, no GPUI) and `ReviewPane`, reaching outside through three seams the desktop fills with daemon clients: `CommentSink` (TerminalService.Type), `ReviewStore` (the session's saved review) and `ReviewHistory` (RV-28–RV-30). Its GUI tests run headless on fakes of all three (`review/harness`).
- **Picks:**
  1. **One queue, to one terminal.** A comment's `Kind` (comment, question, GitHub draft) says what it is; comments and questions go to the session's agent, or for a PR review its review chat (D59), in one message per send, marked by kind. GitHub drafts are never queued or typed. Reverse: a queue per kind.
  2. **What's previewed is what's typed,** byte for byte. A send is written ahead as Sending under an idempotency key before it's typed and marked Sent after; a send found Sending after a restart is in doubt and typed again under the same key, never a new one. **Needs the daemon:** `Type` must remember keys durably and answer AlreadyTyped, or a kill between typing and saving repeats a message.
  3. **The agent's view changes apply at once** with Undo per action and Back (D59), but while the user scrolls, types or selects they wait until 800 ms of stillness, on one re-armed timer, not a poll. Acting on an agent note (Make it a draft, Edit, Drop) retires its Undo; undoing an older focus after a newer one changes nothing on screen. The same action id twice applies once.
  4. **"Agent may: Focus, Notes, Order" are per session and saved;** a switched-off action is refused with a reason the relay hands back to the agent.
  5. **A prepared review's notes become the session's notes** and its order the file list's; Revert puts the saved layout back and takes its notes away, keeping the user's comments and Viewed marks.
  6. **A comment on a whole file** is `LineRange::FILE`; questions about no code are typed in the chat itself, so the pane has no Ask chat of its own.
  7. **The base picker's choice is saved per session;** "Since review N" asks `ReviewHistory`, and Mark reviewed records exactly what's shown.
  8. **Saving:** every change to comments or the queue is saved before it's acted on; a draft being typed is saved 300 ms after typing stops (PER-25). The status shows "Saving…" until then.
  9. **Keys** (listed in `review/src/view/keys.rs`): `n`/`shift-n` next/previous unviewed file, `j`/`k` lines (`shift` extends), `]`/`[` comments, `c` comment, `a` ask, `cmd-enter` add, `s` send, `shift-s` send all, `v` viewed, `x` collapse, `m` rendered markdown, `1`–`3` image modes, `d` changed pixels, `b` Back. A sent comment's action is "Draft again".
  10. **Bazel:** ordinary GPUI only for now; a GPUI Fast build is one more target when the desktop wants it. The diff and markdown views are stubs here until `humata2_diff_view` and a `MarkdownRenderer` are embedded.
- **Open:** whether the 800 ms stillness and 300 ms draft debounce feel right on a real screen.

## D54: Links push: the polling audit's picks (SPEC TR-7–TR-10, D52). Provisional
- **Audit:** research/POLLING-AUDIT-2026-11-14.md; the polls that remain are in api/GRPC.md §2.1.
- **Keepalive:** the peer link sends an HTTP/2 PING after 30 s without traffic and ends the link when one goes unanswered for 20 s (20 s is a guess: long enough for a slow SSH hop). Local sockets need none.
- **Waiting:** `humata` holds one `WaitForAnswer` stream (`wait_seconds` at its maximum) and skips its keepalives instead of asking again every 60 s.
- **Backpressure:** the MCP server drops `notifications/progress` while 64 lines wait to be written (each only says "still waiting"); replies are never dropped. Attach acks once per quarter window, not per output chunk. `files put` streams from a blocking thread through a 2-deep queue.
- **Tests:** tokio's `test-util` feature is on in the workspace for paused-clock tests; it changes nothing unless a test pauses the clock.
- **Open:** whether to read the peer link's stdin with `AsyncFd` instead of tokio's blocking-pool stdin, once `peer_link_latency_bench` numbers say it matters; the polls in holder, revive and machines (audit) are for their owners.

## D55: Reviews since last time, agent-prepared reviews, asking about a PR (SPEC RV-28–RV-34, PR-19–PR-21, owner direction 2026-11-14). Provisional
- **Picks:**
  1. **Reviewed snapshots are git objects** (a commit plus a tree of the working tree as shown, under `refs/humata/reviewed/…`), so "since I last reviewed" is a git diff and costs nothing for unchanged files. Reverse: store file hashes in the store and diff blob by blob.
  2. **Per-file Viewed** records the content hash, as GitHub's "Viewed" does, but shows only what changed since rather than the whole file.
  3. **A diff spec is data** (base, what's compared, filters), shared by the user's base picker, `review_prepare` and the skills, so an agent's review is something the user can see and edit, not a picture.
  4. **Two shipped skills:** /render-humata-review (words → diff spec) and /order-humata-review (order, groups, notes; handed to a small model like Haiku where the agent supports subagents).
  5. ~~Ask goes to a second, read-only agent terminal~~: superseded by D59; there is no reviewing agent, so Ask goes to the one review chat.

## D56: Chunked, compressed transfers (SPEC TR-11, TR-12, owner direction 2026-11-14). Provisional
- **Picks:** per-chunk compression in our messages (a `codec` field on chunk messages), not only gRPC's transport compression, so it works over the stdio peer link, gRPC-web and stored blobs alike, and incompressible data is skipped per chunk. lz4 (lz4_flex) by default where the link is fast or local, zstd where it is slow; Auto/lz4/zstd/Off in Settings, globally and per machine; codecs negotiated at link start. 1 MiB chunks.
- **Watching:** only each live agent's conversation file and each clone's HEAD, index and branch ref; the working tree is read on demand.
- **Open:** how Auto decides a link is slow (measured throughput vs. local/remote). Recommendation: lz4 for local sockets and the first seconds of a link, then zstd when measured throughput is under 50 MB/s.

## D57: One crate picks the GUI engine (owner direction 2026-11-14). Provisional
- **Built:** `apps/humata2/engine` (`humata2_engine`) is the only place Humata 2's GPUI code names an engine: ordinary GPUI (`gpui-pre-apple`) by default, GPUI Fast (`gpui-fast-apple`) with the `fast` feature. It loads assets and the bundled fonts once (`Resources`, `init`, `application`, `headless`), says which engine a binary runs (`Backend::current()`, printed in flow artifacts and bench output), and holds the switches only GPUI Fast has (view retention, scroll layers, frame counts) behind `Unsupported` on ordinary GPUI.
- **Bazel:** every GPUI target is built once per engine (`engine` / `engine_fast`, `terminal_view` / `terminal_view_fast`, `ui_gpui` / `ui`), so both keep compiling and both run their tests.
- **No patch to GPUI or gpui-base:** where Fast retains gpui-base's root, the test frame helpers call `gpui_kit::base::test_support::begin_frame` before each draw (D58).
- **Open:** whether GPUI Fast becomes the default desktop engine. Reverse: point the desktop target at the other engine target; nothing else names one.

## D58: Tests draw natural frames and check them against fresh ones (owner direction 2026-11-14). Provisional
- **Natural frames vs a full refresh:** `frame::natural_frame` draws as the app does (only what was notified is rebuilt where views are retained); `frame::full_refresh` rebuilds everything and is a diagnostic. The input helpers (`click`, `drag`, `scroll`, `press`, `type_text`) send real platform events and draw natural frames; `settle` draws until two frames match, at most a bound, then fails with `Unsettled`.
- **Parity:** `parity::Pair` holds a subject window (natural frames, the engine's retention) and a reference built from scratch every check; both get the same input, and `check` compares elements, text, painted quads and sprites, and pixels within a channel tolerance. `against_refresh` checks one window's natural frame against itself refreshed; `humata2_flow`'s `FrameMode::Natural` runs it every step and fails with `Stale` on a mismatch.
- **Picks:**
  1. **The driver's natural mode turns scroll layers off,** so painted quads and sprites can be compared; `Pair` tests with scroll layers on check elements, text and pixels only.
  2. **Snapshots in natural mode are taken from the refreshed frame after it is proved equal,** because the text of a retained view isn't recorded again in a natural frame.
  3. **A missing `cx.notify()` is caught only on GPUI Fast;** ordinary GPUI rebuilds every view it draws, and GPUI refreshes on mouse-down over click handlers, which hides a click handler's own missing notify on both engines.
- **Reverse:** `FrameMode::FullRefresh` is the driver's default; flows opt in to natural frames.

## D59: A PR review's agent is a chat that helps the user review (SPEC PR-22–PR-26, owner direction 2026-11-14). Provisional
- **Pick:** the agent in a review session doesn't review the PR. It answers the user's questions and works the review's view: `review_context` (what the user is looking at), `review_focus` (open, scroll, select, highlight), `review_note` (notes for the user, never posted; Make it a draft), `review_prepare` (order, groups, filters) and `review_draft` only when asked. The goal is a code-level understanding for the user, not a verdict.
- **Pick:** in the review chat the agent's view changes apply at once (the user asked for them), each with Undo in the chat and Back in the view, never while the user is scrolling, typing or selecting. Agent-prepared views elsewhere still arrive as offers (VW-2).
- **Pick:** Info panel switches "Agent may: Focus, Notes, Order", all on by default.
- **Replaces:** the reviewing agent of PR-5/PR-9 and the separate Ask agent of D55.5.
- **Reverse:** make review-chat changes offers too (VW-2), or start the agent with "review this" as before.

## D60: Every compressed payload names its codec (SPEC TR-13, owner direction 2026-11-14). Provisional
- **Pick:** one self-describing frame for compressed data everywhere: on the wire the chunk message's `codec` enum (none, lz4, zstd); at rest the D44 blob header (magic, codec, message type, format version) on every compressed proto or file the daemons write. lz4 and zstd are both valid at rest, so stored data can be written with whatever the link delivered without recompressing.
- **Pick:** codecs are numbers, never strings: a one-byte id (0 none, 1 lz4, 2 zstd), the same values as the proto enum, and message types are numeric ids too. Ids are never reused once assigned (owner direction 2026-11-14).
- **Why:** readers never guess; a codec or level change needs no migration; restart doctor and bug reports can decode any file on their own.
- **Reverse:** the header lives in `humata2_chunk`, shared by wire and store.


## D61: Adding, reconnecting and removing machines in humatad2 (SPEC TS-0–TS-2, PER-27, MP-*). Provisional
- **Pick:** a machine's id is its name until machines get ids of their own; `hosts.json` in the state folder holds each machine with a state (adding, ready, removing), written atomically. The `humata` CLI, `humatad2` and the holder go to `~/.humata2/bin` on add and on every connect whose checksums differ: staged beside the old file and promoted only when no holder runs it (TS-1), so a running terminal is never swapped under. A first add needs the user to confirm the host key: `AddSshMachine` without `trust_host_key_fingerprint` fails at `connect` with `host_key_changed` (empty expected), and a fingerprint that doesn't match sends nothing. A failed add is undone at once (Try again is a fresh add); `CancelOperation`/`RetryOperation` are `not_built`. At start the daemon finishes a half-done remove and undoes a half-done add (PER-27). Operations live in memory. `ReconnectMachine` is synchronous and returns the machine. Agents see no machines and can't add or remove one; removing needs the user present and a plan token (`pl_…`, 10 minutes, single use, bound to the machine). The tools bundle is a folder with `manifest.json` (path and SHA-256 per platform and file) built by `//apps/humata2/daemon/humatad:tools` for darwin/linux × arm64/x86_64; without `--tools` or a `tools/` beside the binary the daemon offers only its own platform's files from beside itself.
- **Why:** the user sees every step and confirms what can't be undone; nothing waits on a timer it doesn't need; a crash mid-change leaves no half machine.
- **Gaps:** macOS remotes need a launchd service (only systemd user and setsid+cron are built); the host-side peer link client and its keepalive aren't built; machine state isn't streamed (EventService isn't built), so the UI re-lists after an operation; a remove plan lists no sessions or channels yet. Since 17da6d6b a reconnect restarts the service only when a file changed or the service isn't running the way setup starts it.
- **Reverse:** machine ids become their own field in `hosts.json`; operations move to the store when Retry lands.

## D62: Ideas adopted from a look at other agent-history tools (2026-11-14). Provisional
- **Adopt now (fit the spec as written, reversible):**
  1. Full-text search over the conversation mirror the host keeps (LD-8), with hits that open the exact turn; in the ⌘K palette and as a read-only MCP/CLI tool.
  2. Read-only history tools for agents (MCP and `humata`), with the same output from both, checked by tests, and stable `session#seq` references.
  3. Calls to our own `humata` tools are left out of the search index.
  4. Cleanup safety (§8, PER-27): re-check ownership just before acting, journal file hashes before moving, refuse hard links, count allocated size.
  5. Derived data (indexes, caches) carries a format version and is rebuilt when it changes; user data never lives in a rebuildable index.
  6. Watchers: an event goes to the longest matching root; a notification overflow triggers one rescan of that root (TR-12).
- **Proposed, for the owner:**
  7. Opt-in "where agents left off in this channel" note at session start (silent when there's nothing to say).
  8. Subagents an agent spawns shown as child sessions linked both ways, with copied fork history folded so it isn't counted twice.
  9. A skills page that shows when copies of one skill have drifted apart across machines.
  10. Instruction and memory files (CLAUDE.md, AGENTS.md…) per repo and machine, viewable and editable.
- **Rejected:** dashboards of status the user can't act on; pulling remote state by rsync (we push); a cross-process lock (humatad is the only writer).

## D64: Holder links, a ready pipe, coalesced output, effect backpressure, pipe-fd stdio (SPEC TR-7–TR-10, D54). Provisional
- **Holder links:** humatad keeps one connection per tab's holder (`holders.rs`) for `ListTabs` and `TypeInto`, opened on first use, dropped when the holder hangs up, a typing call fails or times out, or the tab closes. The holder has no control-only connection, so the link reads all of its tab's output to keep offsets; offsets may trail output in flight, and `earliest` is worked out from the daemon's scrollback size (exact without gaps, later than true otherwise). Reverse: a control-only holder connection that pushes offsets would cut the reads.
- **Bridge:** a daemon the bridge starts gets a hidden `--ready-fd` and writes a line once it listens; the bridge waits on that pipe (10 s), keeping one retry after 0.5 s for a daemon already starting.
- **Attach:** output already queued and contiguous joins one message, up to 64 KiB and the window's room; it never waits for more.
- **tca-rust:** UI sends stay unbounded (synchronous, never block or drop, human rate). Actions effects send are counted; `Effect::stream` awaits `Sender::ready` while 1024 (`EFFECT_QUEUE`) wait, so a fast server is held back by flow control. `Effect::run` loops that send without awaiting `ready` stay unbounded. Reverse: drop the gate in `stream.rs`.
- **Peer-link stdio:** pipes are read and written as tokio pipe fds; terminals and sockets keep tokio's stdio. Kept for p50 (135 → ~85 µs) and p99 (336 → 200–260 µs); max wasn't shown better (0.56–3.0 ms vs 1.9 ms, noisy). Making the pipe non-blocking changes it for any other process sharing the same open pipe. Reverse: `stdio()` in `peer_link.rs` returns tokio's stdio.

## D63: Moves carry all of a session's state; search covers every machine (SPEC MV-13, MV-14, LD-23–LD-25, owner direction 2026-11-14). Provisional
- **Pick:** a move carries the clones (all of git's states plus `refs/humata/…`), the session folder's own files (Markdown included), the machine store's rows for the session as self-describing records, and the agent's per-session files with paths rewritten. A hashed move manifest is checked on the new machine before the old copy goes to trash; the restart doctor can finish or undo a cut-off move.
- **Pick:** cross-machine search runs on the host's index over the mirror (works with machines offline), exposed in ⌘K, `humata sessions search` and the MCP tools `sessions_search` + `session_read`. Own channel by default; other channels Ask.
- **Reverse:** search each machine live (fresher, but offline machines drop out), or move the raw store database (simpler, but couples schema versions across machines).
- **Open:** a synthetic, read-only file system of every session's history (e.g. `~/Humata/.sessions/<channel>/<session>/<turn>.md`), so agents use `rg`, `cat` and `ls` they already know, with files fetched lazily from the host's mirror. Recommendation: yes, after `sessions_search` ships. Use FSKit on the Mac (user-space, no kernel extension, macOS 26.7 has it) and FUSE on Linux; where neither is available, the CLI and MCP still work. Search stays the fast path. The file system is for browsing and grepping a few sessions: a recursive grep over all of it reads every file, so fetched files are cached on the machine, and the folder's README tells agents to use `humata sessions search` for broad queries.
- **Open:** whether to also index the agent's own native history on each machine that Humata didn't start (sessions run outside Humata). Recommendation: not now; only Humata sessions are mirrored.

## D65: Chunked, compressed byte streams and the at-rest frame in `humata2_chunk` (SPEC TR-11, TR-13, D60, owner direction 2026-11-14). Provisional
- **Pick:** codecs none, lz4 (`lz4_flex` block format, `safe-decode` + `checked-decode`, no unsafe) and zstd (the C `zstd` crate, level 3). Chunks are at most 1 MiB, each compressed on its own with a CRC-32 of the decoded bytes, so a resumed stream may switch codec mid-file and damage is an error, never garbage.
- **Why zstd C over ruzstd:** ruzstd 0.9.1 only encodes at its Fastest level and encodes far slower than the C crate (numbers in the release bench, `bazel run -c opt //apps/humata2/chunk:codecs_bench`); its decoder reads C-made frames, so the store's ruzstd reads stay valid.
- **Skip rules:** chunks under 512 bytes, chunks that save less than 1/16, and streams whose first bytes are a known compressed format (PNG, JPEG, GIF, WebP, MP4/MOV/HEIC, WebM, Ogg, FLAC, MP3, WOFF2, zip, gzip, zstd, xz, bzip2, 7z, lz4, rar) go as none. After 4 misses in a row the encoder only retries every 8th chunk.
- **Negotiation:** ordered codec lists, best first (`ServerInfo.chunk_codecs`, `ReadFileRequest.accept_codecs`, `Hello*.chunk_codecs`); an empty list means none only. The policy lives in `humata2_chunk::policy` (`ranked`, `choose`).
- **Setting:** `<state>/compression.toml`, `compression = "auto" | "lz4" | "zstd" | "off"` globally and under `[machines.<id>]`; read per stream, a bad file is logged and read as auto. Auto is lz4 locally and on fast links, zstd under 50 MB/s measured. Off offers only none. No UI yet.
- **At rest (D60):** a 22-byte little-endian header, format 1: magic `\x89H2Z`, header format, codec id (u8), message type id (u16, chosen by the writing crate), format version (u16), decoded length (u64), CRC-32 (u32). `encode_framed` / `decode_framed` / `peek_framed`; no names anywhere.
- **Throughput (no probes, no timers):** `humata2_chunk::Throughput` learns a remote link's speed from finished transfers of at least 1 MiB: encoded bytes over first-to-last chunk. A transfer counts as the link's rate when this end spent at least half of it waiting on the link (a sender for room, a receiver for the next chunk); otherwise only when that rate is already fast, since a slow end says nothing about the link. Each new rate is averaged with the estimate, so a changed link moves Auto within a few transfers. An unmeasured link counts as fast. A daemon keeps one remote estimate (its peer link); FileService feeds it once FileService runs on that link. Reverse: drop `Throughput`, and Auto is lz4 everywhere.
- **Terminals:** `TerminalOutput` and `TerminalSnapshot` carry `codec`, `raw_size`, `crc32`; offsets, windows and acks count decoded bytes. With `AttachOpen.accept_codecs` set, a snapshot over 1 MiB goes as pieces with one offset and `more`; without it, whole and raw as before. Output is encoded where it is sent (at most 64 KiB coalesced); snapshots on a blocking thread. Coalescing and the ack window are unchanged.
- **Idempotency (WriteFile):** a key names one upload of one file. A retry with the same key gets the first upload's `FileEntry` without its bytes being read or written; one while the first still runs is `ABORTED` (wait, then retry). A failed upload forgets its key. The last 1,024 finished keys are kept in memory, so a daemon restart forgets them.
- **Left:** a host-side peer client (nothing sends `HelloRequest.chunk_codecs` yet) and per-machine throughput on the host, FileService over the peer link and other roots, `TransferService.Upload`, diff chunks, bug reports, a settings UI, a central registry of message type ids.
- **Reverse:** set `compression = "off"`; the wire fields stay and say none.

## D73: The embedded terminal's pane, tabs and keys (OVERVIEW I6, SPEC ST-12, D49, D58). Provisional
- **Crates:** `terminal/view` draws any `Grid` and turns input into host calls; new `terminal/pane` (Bazel only, like `ghostty_vt`, built per engine as `terminal_pane` and `terminal_pane_fast`) adds the `ghostty_vt` host, the find bar, the multi-line paste prompt and the tab strip. The pane talks to the program through a `Pty` trait (`write`, `resize`) and tells the app everything else as `PaneEvent`s (title, pwd, bell, clipboard write and read requests, image and file pastes, failures). Reverse: fold the pane into `features/session`.
- **Panes (D74):** a `TerminalPane` is self-contained pane content: any size, any number at once, focus by click. ⌘\ (Ctrl+Shift+\ off the Mac; Ctrl+\ stays the program's quit signal) emits `SplitRequested` and the pane tree splits with a new shell. `set_visible(false)` draws nothing and holds the program's output unparsed, parsed on show or past 1 MiB, so replies to the program's queries wait while hidden (NV-34). Reverse: parse while hidden, draw nothing.
- **Tabs:** secondary since D74, kept for a pane that still wants tabs. ⌘⇧] / ⌘⇧[ and Ctrl+Tab / Ctrl+Shift+Tab switch, ⌘1–⌘8 pick, ⌘9 is the last tab, ⌘T asks for a new shell, ⌘W closes the view (Ctrl+PageDown/PageUp, Alt+1–9, Ctrl+Shift+T and Ctrl+Shift+W off the Mac). Closing emits `Closed`; stopping the program is a separate `StopRequested`, so closing a view never kills a process. Tabs marked not closable (the session's shell) have no ✕ and ignore ⌘W. Double-click renames (Enter keeps, Escape restores); drag onto a tab moves there.
- **Scrolling:** wheel, the scrollbar (click or drag the thumb), ⌘PageUp/PageDown/Home/End on the Mac and Shift+the same elsewhere. The view stays where the user scrolled while output streams and follows output only at the bottom. On the alternate screen the wheel goes to the program (arrows when it asks for alternate scroll).
- **Selection:** a drag selects cells, a double-click a word (letters, digits and `_-./~:@%+#?&=`, so paths and URLs stay whole), a triple-click a line, Option (Alt) a block; Shift overrides a program that takes the mouse.
- **Paste:** one line goes straight through (bracketed when the program asked); with bracketed paste on, line breaks stay `\n`, as Ghostty sends them. Several lines without bracketed paste wait for a prompt that says each line break runs the line before it.
- **Find:** ⌘F (Ctrl+Shift+F) opens a bar over the pane; Rust `regex` over the history's rows (literal text escaped unless Regex is on, case-insensitive unless Case is on), at most 10,000 matches, all highlighted, the current one brighter. Enter and ⌘G go older, Shift+Enter and ⌘⇧G newer; it starts on the newest match. Matches are searched again once per batch of new output while the bar is open. Escape closes it and gives the keyboard back to the terminal. Reverse: Ghostty's own search when `ghostty_vt` exposes it.
- **Option as Alt:** the terminal's `OptionAsAlt` setting decides; GPUI reports no left or right Option, so Left-only and Right-only act as on. Reverse when GPUI exposes sided modifiers.
- **Cursor blink:** on by default for programs that ask, switchable (`set_cursor_blink`); the timer runs only while the terminal is focused and the cursor blinks.
- **Wrap:** the terminal never takes the Wrap setting (ST-12); resizing reflows in `ghostty_vt` and tells the program the new size.
- **Box drawing:** the view draws U+2500–U+257F (light, heavy, double, dashed) and the block elements U+2580–U+259F as rectangles on whole device pixels, as Ghostty does, because font glyphs left steps at corners (seen in the first snapshot). Diagonals stay with the font; rounded corners (`╭╮╯╰`) are drawn square. Reverse: arcs for rounded corners, or the font again if one aligns.
- **Tests:** flows through real input on both engines; streaming at 200 × 60 checks that views beside the terminal aren't built again (cached views on GPUI, frame counters on GPUI Fast) and that a still terminal shapes no rows; a pixel snapshot per engine and platform of tricky output, recorded with `HUMATA2_SNAPSHOT=record bazel run`.

## D76: The design goldens are the UI target (owner, 2026-11-15)
- **Pick (owner):** the 33 frames in `desktop/tests/__snapshots__/design/` (drawn by `desktop/tests/design.rs`, target `//apps/humata2/desktop:design_test`) are what the UI should look like. Screens are built to match them. A design change goes to the frames first, with the owner's OK.
- **Picks (owner):** right panel switched from a labelled strip on the right edge, plus a header toggle; terminals and files are tabs in one row; the only split is two tab groups in one session, at most one terminal per agent, and anything more is a fork (code + context, context only, code only); indented sidebar tree; a channel opens to its Sessions tree table; Board kanban, drawer, table and dashboard, and the expanded Board panel shows the kanban; Home and Inbox list what needs you first; New session runs any agent's TUI or a plain terminal.
- **Platforms (owner):** macOS, Linux and web only.
- **Revises D74:** no general pane workspace or Panes sidebar.
- **Reverse:** re-record the frames from the earlier options in c16dff55.

## D74: The window is a workspace of panes (SPEC NV-28–NV-34, owner direction 2026-11-14). Provisional
- **Revised (owner, 2026-11-15):** no pane tree. The owner disliked the pane view, and how panes changed when switching channels and sessions. The window is the left sidebar, the screen in the middle and the open session's side panel on the right; both sides resize by dragging their divider, hide past their narrowest, and reset on double-click. Split, close, focus-toward and layout-undo shortcuts and palette entries are gone (⌘W and ⌘Z are free again). The picks below are superseded where they differ; quiet chrome still holds. `features/layout` was deleted (owner, 2026-11-15). Reverse: restore it and the renderer from a7c6cf3c.
- **Pick:** panes show any one thing (terminals, sessions, diff, files, board, Inbox, review, spec, Hosts, Settings), split horizontally or vertically to any depth, moved by dragging their title, opened from a **Panes** sidebar on the right. Every screen keeps a default layout; the user's changes persist per session and screen; Reset layout.
- **Pick:** quiet chrome: no top status text or "Working" badges (a session's state is its dot), actions in ⋯/right-click/⌘K, dialogs only for decisions.
- **Why:** the owner found the drawn UI rigid, with too many buttons, labels and pop-ups.
- **Supersedes:** fixed one-side-panel session layouts and tab bars where they differ; the prototype remains the default-layout reference.
- **Pick (owner, 2026-11-14):** a non-destructive **Reset** button (closes nothing, Undo reset restores the old layout) and pane freedom as one setting, Fixed / Simple (default: up to 4 panes, two levels) / Free, enforced in one place in the layout reducer so it can be restricted or undone easily.
- **Open:** whether the left sidebar (channels and sessions) stays on the left or becomes one more pane. Recommendation: stays on the left as navigation, collapsible; everything you look at is a pane.

## D71: Restarts and power cuts in the store, holder and revive (SPEC PER-25–PER-31, TR-7, TR-9, TR-12, TR-13, D52, D54). Provisional
- **Power cuts:** new `daemon/durable` wraps every write the store and holder make (`File`, `rename`, `remove_file`, `create_dir_all`, `sync_dir`). Built with `faults`, it plays a power cut: under `HUMATA2_POWER_CUT_ROOTS` it keeps a ledger of what was written but not synced, and `cut` puts those files back as they were last synced (copies under `HUMATA2_POWER_CUT_SPARE`) and kills the process. SQLite writes go through a VFS shim over the default VFS, so the same ledger covers the database and its WAL. Not tracked: `-shm` (SQLite rebuilds it), the VFS's `xDelete`, and which files SQLite itself created. Reverse: a VM snapshot harness, if the shim ever disagrees with a real cut.
- **Tests:** `store/tests/power_cut.rs` cuts two stores at every write point against a model; `holder/tests/power_cut.rs` cuts a printing holder (SIGUSR2 in `humata2_holder_faults`, test-only); `revive/tests/kill_matrix.rs` cuts holders and the store while a launch or resume is under way.
- **Holder (TR-7, TR-9):** saved scrollback is synced half a second after output is saved (a `Notify`), and an idle holder runs no timer; a thread waits for the program and writes its exit code to a pipe, which the output reader waits on beside the terminal with one blocking `poll(2)`, so nothing wakes on a timer. revive watches a folder filtered to the one file it waits on (TR-12) instead of polling it.
- **Build:** the store keeps `libsqlite3-sys` as a dev-dependency of its own so the tests get the VFS symbols; crate_universe skips the optional dependency `rusqlite` uses.
- **Blobs (TR-13, D65):** each stored blob is a `humata2_chunk` frame (`encode_framed`/`decode_framed`: `\x89H2Z`, codec, type, version, length, CRC-32), so there is one frame at rest. The store writes zstd and reads what the frame names. Its own `H2BL` header is gone and has no reader, since nothing shipped with it. A frame of another type is damage; a newer header format or codec is `NewerFormat` (update Humata, keep the file); any other frame error is `Undecodable` (restore from a backup).
- **Last good copy (PER-31):** `Store::save_last_good` writes `VACUUM INTO` a temp file, checks it with `integrity_check`, syncs and renames it to `last-good.sqlite`. The sweep keeps the blob files the copy names. `Store::open_or_restore` runs `quick_check` on open; only a corrupt or not-a-database error, or a failed check, restores: the damaged database, `-wal` and `-shm` move to `damaged-<ms>/` (never deleted) and the copy goes in their place. The copy of the copy is written and synced before the damaged database moves, so a crash between leaves the database missing beside `.state.sqlite.restore.partial`. `Store::open` (and `open_or_restore`, which then returns `Restored`) finishes such a restore before anything else: the damaged `-wal`/`-shm` left behind join the newest `damaged-<ms>/` first, so SQLite never applies them to the copy. Without that, the next open made an empty database. It returns `Restored`: when the copy was taken, what was lost per table and key (added, changed, removed), and the tables it couldn't read. No copy: `DamagedWithoutCopy`, and the files are left alone.
- **Restart reports (PER-29):** each reconcile writes one D47 bug report folder, named `<date>-restart-at-hh-mm-ss-utc` under the reports folder humatad passes (not `restarts/<time>/`), with one action per step, the error chain and the last 50 holder log lines for each session or shell not brought back. `restart.json` sits beside the D47 files: every entry's status, reason, needed fix and step, the restore's losses, and the conversation, folder, scrollback and log files with sizes and times. Secrets scrubbed are the values of the launch environment of 4 characters or more, in paths too (`‹CONV_DIR›/c1.jsonl`); the doctor resolves them from the agent's variable names. Reverse: mask only names that look secret.
- **Resume (PER-27):** the agent is told "you were interrupted" once, even when a crash makes the resume run again. Just before typing the note, the resume saves `Interruption.told` (when, and the conversation file's length then; schema v2 adds `told_unix_ms` and `told_conversation_len` to `interruptions`). A resume redone from a `Resuming` record whose last interruption was told looks for the note in the conversation after that length (anywhere, if the file is now shorter). If it's there, the note isn't typed again and `told` carries over; if not, the agent stopped before writing it down, so it's told. When the note is skipped, the agent writes nothing, so the check is weaker: it came up, its output settled (or the timeout passed) without it exiting, and no other conversation grew. This relies on the agent saving the note's text unescaped, which holds for JSONL since the note has no quotes, backslashes or newlines. Fault points `revive.resume.telling` and `revive.resume.told` are in the kill matrix and the power-cut matrix, which check the note appears once. A shell restarted just before the daemon died is found by its holder's pid file and saved on the next reconcile.
- **Partial writes:** `Store::open` removes `.partial` temps a crash left in the state folder, in `blobs/` and in each session's `files/<session>/paste/`. It does this only at open, since one humatad owns the state folder at a time; `Store::sweep` still sweeps only `blobs/`, under the database lock, because pastes are written without it. Fault points: `store.file.temp_written` (every atomic write, before its rename) and `store.last_good.copied`, `store.restore.copied`, `store.restore.moved`.
- **Open:** MV-13/MV-14 moves should export a session's rows as the same self-describing records and carry a manifest in `restart.json`'s shape with hashes.


## D77: How humatad2 starts again: the background reconcile, `ServerInfo.last_start`, the last good copy's debounce and the daemon's own log (SPEC PER-29, PER-31, TR-7, D54, D71). Provisional
- **Serve first, reconcile beside it:** `humatad2 serve` binds its socket before it opens the store (a second daemon on the same socket stops there, before touching the state), opens the store with `Store::open_or_restore`, and starts serving while `humata2_revive::reconcile` brings sessions back in a task of its own. An agent may take 30 s to resume, longer than a bridge waits for the daemon to listen, and every other call works meanwhile. Reverse: reconcile before serving, and make the bridge wait longer.
- **`ServerInfo.last_start`:** unset until the reconcile is done, then the restart report's folder, one `NotBroughtBack` line per agent (by session) or shell (by tab) that didn't come back, saying what went wrong and what to do, and, after a restore, when the copy was taken and how many rows it changed. A reconcile that can't run at all is itself the one line. Only the user gets it (session names and folders are theirs). A client that wants it waits for it to be set; there is no push yet (to come with EventService).
- **The last good copy's debounce:** the store calls `Store::on_commit` hooks after each committed write; humatad's keeper saves a copy on a blocking thread once writes have been quiet for 30 s, or after 10 minutes of writes that never go quiet. An idle daemon runs no timer (TR-7). File events were dropped: Bazel's darwin sandbox delivers none, and the store is the only writer anyway. A write that lands while a copy is being taken gets a copy of its own.
- **The daemon's own log:** every line humatad2 writes to stderr, and a panic, also goes to `logs/humatad2.log` (0600, started over past 4 MiB). Each start moves it to `humatad2.previous.log`, and the restart report carries that one's last 50 lines in `daemon.log` (marked "(last run)") and `restart.json`'s `daemon_log`, scrubbed like the rest. The peer link (`serve --stdio`) keeps its own `peer-link.log` and isn't in it. Reverse: a structured log (JSON lines) if the doctor needs fields.

## D66: Review diffs, reviewed snapshots and the review chat's view in `humata2_review_diff` (SPEC RV-28–RV-34, PR-22–PR-26, TR-11, TR-12, D55, D59). Provisional
- **Git CLI, not gix:** every diff runs `git` on the session's machine. Measured 2026-11-15 on this Mac with the manual bench `bazel test -c opt //apps/humata2/daemon/review_diff:big_repo_bench` (20,000 files, 200 edited, one 200,000-line file; medians of 5): file list of uncommitted changes 287 ms, first chunk of the big file 35 ms, one small file's hunks 17 ms, Mark reviewed 142 ms, file list since the last review 249 ms. gix was measured by hand in a scratch release build on the same repo shape, for status only: index-worktree status with untracked files took 41–58 ms, against 0.09–0.11 s for `git status --porcelain -uall`. A full gix tree-plus-diff pipeline was not built or measured, so gix might be faster end to end; the CLI is fast enough for TR-11 and wins on what follows. The CLI also gives rename detection, `-w`/`-b`, pathspecs with `:(exclude)` and the same answers as the user's own `git diff`, which gix's blob diff doesn't match yet. Reverse: swap `git.rs` for gix behind the same `Checkout` methods.
- **The user's index is never written:** the new side is a tree written from a private copy of `.git/index` (`GIT_INDEX_FILE` in a temporary folder), so git's stat cache makes it cheap and the user's staging stays as it was. Untracked files are added to that copy by path, ignored files never.
- **Snapshots are git objects:** Mark reviewed writes a commit (tree as shown, parent HEAD) under `refs/humata/reviewed/<session>/<n>`; a pin is `refs/humata/pinned/<session>/<n>`; the last 20 unpinned are kept. They move with the clone. **TR-13 doesn't apply:** nothing here is stored compressed by Humata; git's own object format is git's.
- **MarkReviewed snapshots what was last shown:** the daemon snapshots the new tree of the last GetDiff it served per repo (the working tree if none). `MarkReviewedRequest` has no field for it, because the CLI builds that message as a literal; add one with the CLI's owner if needed.
- **Viewed is a blob id** (RV-29): unchanged blob, folded; changed, only the diff from the viewed blob; a blob git pruned shows the whole file.
- **Repos are keyed `owner/name`** (or the bare name without an owner) in specs, plans and notes.
- **Big diffs come in chunks (TR-11):** the file list carries counts and blob ids only; hunks are read per file from `git diff-tree -p` as it streams, cut at a line and byte budget with a cursor to go on. Nothing in this crate compresses; chunk transport is `humata2_chunk`'s.
- **Watching (TR-12):** `Checkout::watch_set` is HEAD, the index and the current branch ref; `stamp` says whether a diff must be computed again. Working-tree edits are read when asked.
- **Review chat state lives in the crate:** `ReviewChat` holds the view (spec, open place, order, groups, Viewed, highlights), notes for the user, Undo per change (`change-<seq>`) and Back. Notes persist as Comments with a `UserNote` destination plus `ReviewViewState`; the store and humatad own writing them. Notes are anchored to a blob and lines and go outdated when those lines change (RV-34, PR-26); a note whose lines only moved follows them.
- **Switches (PR-25):** Focus gates `review_focus`, Notes the three note tools, Order `review_prepare` in the review chat; `review_context` is never gated. A denial is `Denied` with `SOURCE_REVIEW_SWITCH_OFF`, saying the user turned it off and where to turn it on.
- **MCP:** `review_context`, `review_focus`, `review_note`, `review_note_edit`, `review_note_drop`, `review_prepare` added to the tool table (MCP.md §6.1). `ReviewChatService` is new in review.proto; humatad doesn't serve it yet.
- **Skills:** `skills/render-humata-review` (words to a diff spec) and `skills/order-humata-review` (order, groups, notes; handed to a small fast model when the agent can). Both frame the agent as helping the user understand the change, never as a reviewer.

## D67: The embedded diff view and its measurement gates (SPEC RV-17–RV-24, RV-29, RV-31) (proposed by minion C). Provisional
- **Pick:** `diff::DiffView` embeds gpui-component's virtualized `DiffState` on GPUI and GPUI Fast. It supplies repo/group headers, comparison specs, comments, notes, Viewed marks and context requests. The embedder computes the diff and answers SpecChanged/ContextNeeded by replacing the review; comment events carry the actual comparison, base commit, side, lines and quoted code.
- **Existing measurements:** OVERVIEW's earlier loaded-machine runs, on the old DiffState view, recorded GPUI scrolling with comments at p95 6.71 ms / max 10.07 ms; Fast p95 was at most 3.64 ms. These are headless measurements of the previous view, not proof of DiffView's performance or real-window GPU timings. No benches were rerun for this entry.
- **Gate still open:** point `diff:gate` and `diff:gate_fast` at DiffView and repeat the GPUI/Fast incremental-patch cases on a quiet machine. Full file mode needs its own measurement before enabling it in the app. There is no open-all-folds action: context arrives only for the fold the user asks to open.
- **Behaviour verified:** `diff:view_test` and `diff:view_fast_test` open the embedded view in a Pane and exercise comparison editing, comment anchors, missing-context responses, replacement/empty counters and Viewed folding. Navigation on a short review still exposes a dependency counter/viewport gap, recorded in OVERVIEW.
- **Reverse:** keep the review/model/event boundary and replace the embedded renderer if the gates show it cannot meet the budget.

## D78: Markdown rendering, scrolling and headless measurements (SPEC SM-3–SM-8) (proposed by minion C). Provisional
- **Pick already implemented:** gpui-kit's text view for GFM, streaming, selection and code highlighting; the same markdown AST for source anchors. Mermaid diagrams render to SVG off the UI thread, with a 64-entry source/theme cache; images resolve inside the document's base folder, with remote images disabled by default. Links emit an event for the embedder. Comments and changes are annotations around the shared source model. Reverse: retain the document/anchor/event API and replace the renderer.
- **Measured 2026-11-15 on this Mac:** `bazel test -c opt //apps/humata2/markdown/headless:timing --test_arg=--nocapture --test_output=all`, 16,000 lines / 176,572 bytes. Whole parse median 285.065 ms, worst 372.132 ms (10 runs); 64-byte append median 24.667 µs, worst 197.375 µs (2,759); one-block diff median 24.875 µs, worst 175.625 µs (10). Open plus first frame 877.159 ms (first frame 22.783 ms); frame after a 400 px scroll median 1.778 ms, worst 2.788 ms (60); append plus frame median 9.067 ms, worst 13.479 ms (200).
- **Limits:** headless test fonts, no GPU paint, concurrent test functions and other work on the Mac; scroll timing excludes the input helper's refreshes. These are neither real-window frame costs nor a quiet-machine comparison with Fast. After streaming: 5,000 blocks reused, one block / 3,490 bytes reparsed, 13 labelled blocks aligned.
- **Flow tooling:** `.scroll(target, x, y)` sends pixel deltas (negative y scrolls down) in both frame modes; the long-document Markdown flow uses it. Secondary shortcuts already parse with the actual platform OS, now covered by both engine suites.
- **Goldens reviewed:** both stored gallery images and the new unannotated render were inspected. The current monospace code differs from the stored proportional-font references; do not silently overwrite them. The existing cross-block selection flow still lacks its toolbar; Fast highlighting needs its excluded repin, and the annotated gallery's long margin comment clips. These remain in OVERVIEW.

## D79: A minimal Rust formatting policy (proposed by minion C). Provisional
- **Proposal:** root `rustfmt.toml` sets `edition = "2024"` and `max_width = 100`; other choices remain the stable defaults. It matches the workspace edition and avoids unstable options.
- **Scope:** format touched files with the Bazel-managed rustfmt target. This change formats only the two new GUI test files; it does not rewrite the existing tree or turn on a new enforcement gate. The owner decides how to migrate existing files before repository-wide enforcement.
- **Verified:** `bazel run @rules_rust//tools/upstream_wrapper:rustfmt -- --check apps/humata2/diff/tests/view.rs apps/humata2/testing/flow/tests/scroll.rs` accepts the configuration and both files. Reverse: remove the config; no behavioural code depends on it.

## D80: When a review plan applies at once, and notes on a hunk (proposed by minion B). Provisional
- **Pick:** `review_prepare` (PrepareView with a review plan) applies at once, through `ReviewChat::prepare` with its undo id, when the session's review chat is open, which the daemon takes to mean the app has reported the review view (ReportReviewView) at least once; the store keeps that flag. Anywhere else it would be a draft view, which isn't built, so it answers not built. Recommendation: keep it, and have the app say when it closes the review pane so the flag can clear.
- **Notes on a hunk:** CommentAnchor has no hunk target, so `review_note`'s `hunk_at` is a note on that one line. Recommendation: add a hunk target to CommentAnchor when the pane draws hunk notes differently.
- Reverse: both live in `daemon/humatad/src/review.rs` and `src/mcp.rs`; the store's `view_reported` would go unused.

## D81: What Cancel and Retry do for machine operations (proposed by minion B). Provisional
- **Retry:** GRPC.md §7 says Retry reruns from the failed step, keeping steps done. A failed add is undone at once (nothing it did survives), so a retried `ADD_MACHINE` runs every step again, under the same operation id with its version going on; a failed `REMOVE_MACHINE` runs its removal again (the entry stays `Removing` until it works). Recommendation: keep it until a kind has steps whose work survives a failure, then resume from the failed step there.
- **Cancel on an ended operation** answers with the operation as it ended (not an error), so a cancel that races the end is harmless. **Retry on anything not failed** fails with `invalid_field` on `id` (FAILED_PRECONDITION), since the error kinds have none for "wrong state". Recommendation: keep both; add an error kind if clients need to branch on it.
- **Removal is not cancellable:** its point of no return is `stop_daemon`, the first step this build runs, so `cancellable` is false from the start and Cancel answers `not_cancellable`. Recommendation: make it cancellable before `stop_daemon` once `dispose:` steps run.
- Reverse: `daemon/humatad/src/operations.rs` (CancelOperation, RetryOperation) and `machine_service.rs` (`launch`); the machines crate's `Drive::cancelled`.

## D82: The event feed, kept in memory, starting with machines (proposed by minion B). Provisional
- **Pick:** EventService.Subscribe serves an in-memory feed of the last 1024 events. Sequence numbers start at the daemon's start time in microseconds, so they keep growing across restarts without being stored, and an `after` from an earlier run is always older than the oldest kept and gets `Resync`. Recommendation: keep it until a client needs events across a restart without refetching; then store the feed (and its numbers) in the store.
- **Machines first:** the daemon compares the machine list with what the feed last told after each step and each change it makes, and sends Created, Updated (any field ListMachines shows, reachability included) or Deleted (with the last state). Machine events go to the user only, and a subscription narrowed to channels or sessions leaves them out unless it names TOPIC_MACHINES. Recommendation: keep it; add each other topic as its service writes.
- `Event.by` is unset for now. Recommendation: fill it from the call that made the change once more than machines are on the feed.
- Reverse: `daemon/humatad/src/events.rs`, `Work::publish_machines` in `machine_service.rs`, and EventService back in `not_built.rs`.

## D83: The host's peer link client: its key, retries and what it shows (proposed by minion D). Provisional
- **Key:** the host keeps its copy of each machine's enrollment key as hex in `<state>/peer-keys/<machine>`, 0600, read with the same checks the machine uses. Nothing writes it yet: a machine without one gets no link. Recommendation: write it at enrollment (the add's install step), then move it to the login keychain.
- **Retries:** a dropped link or failed dial waits 5 s, then 10, 20 and 30 s at most (HO-5), back to 5 s once a link comes up. A failure another dial can't fix (wrong machine proof, the machine refusing as not enrolled, an unsafe or wrong key, an old API, no key on the host) stops the retries until an add or Reconnect starts the link again. Recommendation: keep it.
- **Keepalive:** the client pings after 30 s with nothing read and ends the link when a ping goes 20 s unanswered, the same timers as the machine's end (TR-9).
- **What it shows:** a link that comes up makes the machine ONLINE and one that goes down OFFLINE, on the feed; a wrong machine proof is also OFFLINE, since Reachability has no identity-mismatch value, and the error and its fix go to the log. Recommendation: add an IDENTITY_MISMATCH reachability (or a `problem` on Machine) so the app can say "add it again" (LD-12).
- **Minimal Uplink:** each side sends UplinkResume from nothing, then the host reads and drops what the machine sends; no journal is mirrored. One Uplink per link. Hello sends no chunk codecs and the machine id is the name (D61).
- Reverse: `daemon/humatad/src/peer_client.rs`, `Work::start_link`/`link_up`/`link_down` in `machine_service.rs`, `Link::peer` and `Machines::peer_link` in the machines crate.

## D84: Enrollment at add time, and what a machine shows about its link (proposed by minion E). Provisional
- **When the key is written:** every add makes a fresh 32-byte key (hex) inside the `install` step, before the files, so the step list is unchanged. The machine gets it on stdin into a 0600 temp file renamed over `~/.humata2/state/enrollment-key`; the host writes `<state>/peer-keys/<machine>` (0600 in a 0700 folder, synced, renamed) only once setup has finished. A Reconnect keeps the key, and enrolls again only when the host has no copy (a machine added before keys existed), without reinstalling or restarting. Removal and an undone add delete the host's copy; the machine's copy goes with its state folder, which removal leaves alone (as it does today). Recommendation: keep it, and move the host's copy to the login keychain with the app's keychain work.
- **Identity mismatch:** a new `REACHABILITY_IDENTITY_MISMATCH = 7` (a compatible enum addition) for a wrong machine proof or a machine refusing the host's proof; no retry is planned. Recommendation: keep it rather than a `problem` field on Machine, since it replaces Online/Offline the way HOST_KEY_CHANGED does.
- **Times:** `last_seen_at` is set while the link is up and when it drops (the last moment it was known up); `next_retry_at` is the next dial, unset when up or when retries stopped; `daemon_version` is the machine's Hello and stays after the link drops. Recommendation: keep it.
- Reverse: `daemon/machines/src/enroll.rs`, `Setup::with_enrollment`, `Machines::peer_key`, `Seen` in `machine_service.rs`, and the enum value (left reserved).

## D85: What ListComments lists, and what "other session" compares against (proposed by minion E). Provisional
- **ListComments:** the review chat's notes are the only comments built, so it lists those, oldest first, for the session named (an agent without one gets its own). Reading needs `ReadSession`, as GetReviewContext does. Notes have no CommentStatus (they are never sent), so a status filter lists none, and the empty filter lists them all; `draft_count` is 0 and every note comes in one page, as the other List calls do. Recommendation: keep it until comments to the agent exist, then give notes their own filter value rather than a status.
- **Other session (RV-1):** per repo, the other session's clone (`<its folder>/<repo>`) is fetched into this one with `git fetch +HEAD:refs/humata/other/<session>` (its clone is only read) and this clone is compared with that commit, as the proto's "its branch, per repo" says. Its uncommitted changes aren't included, and only a session on the same machine works. The caller must be able to read both sessions' clones. Recommendation: keep it; for uncommitted work, snapshot the other side's working tree the way Mark reviewed does, once someone asks for it.
- Reverse: `list_comments` and `other_session_spec` in `daemon/humatad/src/review.rs`, `Checkout::fetch_session_head` in review_diff.

## D86: Review Undo and idempotency keys kept through a restart (proposed by minion F). Provisional
- **Pick:** the review's saved state (`ReviewState.history`, a `ReviewHistory`) holds the undo history of the review chat's last 256 changes (focus, a note added, edited or dropped, a plan's order) and each session's last 256 keyed answers (a note, a dropped note, a plan, a focus, Mark reviewed). After a restart Undo of an older change works, an undone change stays undone, and a repeated idempotency key returns the first answer instead of acting again. Older changes answer "nothing to undo" with the reason. Recommendation: keep it; raise the limits if a long review hits them.
- Keys are now kept per session (the raw key, no prefix), and a key is remembered before the change is published or saved, so a crash between the two never acts twice once saved. The WatchReviewChanges feed and Back stay memory only.
- Reverse: field 6 and the `ReviewHistory`/`ReviewUndo`/`ReviewAnswer` messages in `api/proto/humata/store/v1/store.proto`, `ReviewRecord::history`, `ReviewChat::with_undoable`/`undoable` in review_diff, and `saved`/`restored` in `daemon/humatad/src/review.rs`.

## D87: Draft views: who sees them, how they change state, and where they're kept (proposed by minion F). Provisional
- **Pick:** PrepareView outside an open review chat makes a draft (`vw_` id, revision 1) for the caller's own session; only that session's agent prepares, revises and marks ready, and only it sees its drafts (ListViews hides them from the user, VW-14). MarkReady turns a draft into Waiting; after that the agent can't revise it and only the user answers with SetViewState (Later, Not now, Opened, Reverted, Opened and closed, Kept; never back to Draft). A board view must be a scope of the session's channel; MCP's `"global"` board is refused. Recommendation: keep it.
- The store keeps each view as a `View` blob (the PreparedView bytes and its last 16 idempotency keys) in a `views` table that goes with its session, so a restart keeps drafts and answers and a repeated key returns the first view.
- Open: offers aren't pushed to the app (no event topic or Journal entry), `include_forks` is ignored, views live on the daemon that runs the session, not the Mac (VW-12), and there's no CLI.
- Reverse: the fifth migration and `views.rs` in `daemon/store`, `View` in `api/proto/humata/store/v1/store.proto`, and `daemon/humatad/src/review/views.rs` with the ViewService arms in `review.rs`.

## D88: A note going outdated is a review change nobody can undo (proposed by minion F). Provisional
- **Pick:** when a GetDiff marks a note outdated or moves its lines, each such note is a ReviewChange on WatchReviewChanges with tool `"diff"`, the note as it now is, a new seq and an empty undo_id; Undo can't take it back, since the code changed, not the agent. Recommendation: keep it; the app shows it without Undo.
- Reverse: `ChangeTool::Diff` and `ReviewChat::refresh_notes` in `daemon/review_diff/src/chat.rs`, and `refresh_notes` in `daemon/humatad/src/review.rs`.

## D89: Views on the event feed, and narrowing a feed to channels or sessions (proposed by minion F). Provisional
- **Pick:** every saved change to a view goes on the feed as an Event with the PreparedView: Created when the agent prepares it, Updated when it's revised, offered (MarkReady) or answered (SetViewState); a call that changes nothing isn't told. The user sees a view from its offer on, so the first event they get is the Updated that made it Waiting; the session's agent sees its own session's views, drafts included, as ListViews lists them. Recommendation: keep it.
- Each feed event can carry the channel and session it belongs to (kept beside it, not on the wire). A subscription naming channels or sessions gets the events in any of them; naming topics keeps only those. Machines still belong to none.
- Reverse: `Scope`/`Published`/`publish_in` and the View arm of `Filter` in `daemon/humatad/src/events.rs`, the `told` argument of `put` in `daemon/humatad/src/review/views.rs`, and Review's `feed`.

## D90: Inbox requests on the event feed (proposed by minion F). Provisional
- **Pick:** a new question or ask-ahead goes on the feed as a Created Event with the InboxRequest; each answer (AnswerRequest, AnswerAll) and each withdrawal is an Updated one. A call that changes nothing (a repeated withdrawal, a repeat with the same idempotency key, an ask-ahead a grant already covers) isn't told. The user sees every request, a session's agent its own session's, as ListRequests lists them; TOPIC_INBOX and the channels or sessions asked for narrow it. Recommendation: keep it.
- The grants approvals make are on the feed too (D91).
- Reverse: `Inbox::tell` and its calls in `daemon/humatad/src/inbox.rs`, the Request arm of `Filter` in `daemon/humatad/src/events.rs`, and the feed passed to `Inbox::new` in `serve.rs`.

## D91: Grants on the event feed (proposed by minion F). Provisional
- **Pick:** the grant an approval makes (approving an ask-ahead, or "Always let …") is a Created Event with the Grant, told just before the Updated one for the answered request, so an app that sees the answer already has the grant. RevokeGrant tells a Deleted one carrying the grant as it last stood. The user sees every grant, a session's agent its own session's, as ListGrants lists them; TOPIC_ACCESS and the channels or sessions asked for narrow it. Recommendation: keep it.
- Not told yet: a grant going suspended or back when a rule turns Never or away from it (rules can't be set over gRPC yet), and grants ended with their session or channel (nothing ends them yet).
- Reverse: `Inbox::tell_answer` in `daemon/humatad/src/inbox.rs`, Access's `feed` and the publish in `revoke_grant` in `daemon/humatad/src/access_service.rs`, and the Grant arm of `Filter` in `daemon/humatad/src/events.rs`.

## D92: Review chat notes on the event feed (proposed by minion F). Provisional
- **Pick:** after each review chat change is saved, the feed is told how the session's notes differ from when it was last told: a new note is Created, a changed one (edited, outdated, moved, restored by Undo) Updated, a gone one (dropped, or taken away by an Undo) Deleted carrying the note as it last stood. Comparing the notes, not reading each change, covers every path, an Undo of a plan that added notes included. A restart starts from the kept notes, so they aren't told again. The Comment has no undo id on the feed; Undo stays with the caller's answer and WatchReviewChanges. Recommendation: keep it.
- The user sees every session's notes; an agent only its own session's, though ListComments lets it read any session it may read. Widening that needs the feed to check ReadSession per event.
- Reverse: `told`, `tell_notes` and `notes_now` in `daemon/humatad/src/review.rs` (publish takes the SessionRecord), and the Comment arm of `Filter` in `daemon/humatad/src/events.rs`.

## D93: The MCP `inbox_request` tool (proposed by minion F). Provisional
- **Pick:** `inbox_request` calls RequestAccess with the call's idempotency key and waits as `inbox_ask` does. Approved, it answers `{decision: "approved", grant_id}`; an ask a grant already covers is approved at once with that grant's id; a denial is the tool error `denied` with the user's note. A `board` target of `session` or `channel` is the caller's own; `global` is the global board. A capability name this humatad2 doesn't know is `invalid_field`, telling the agent to pick from the tool's list or update Humata. Recommendation: keep it.
- Reverse: `request_access`, `grant_target` and the `access` module in `daemon/humatad/src/mcp.rs`, and the grant id in `outcome`.

## D94: Operations on the event feed (proposed by minion F). Provisional
- **Pick:** an operation starting is a Created Event with the Operation; each change after (a step, the end, a retry starting over) is an Updated one with the whole Operation, told while it's held so the feed keeps the order its version does. One an agent started is filed under the agent's channel and session; the user's belong nowhere, so a feed narrowed to channels or sessions doesn't get them. The user hears of every operation, an agent of its own session's, as GetOperation finds them. Recommendation: keep it.
- A step's progress is one event each; nothing coalesces them yet.
- Reverse: `Tracked::tell` and its calls in `daemon/humatad/src/operations.rs`, `Operations::new` in `serve.rs`, and the Operation arm of `Filter` in `daemon/humatad/src/events.rs`.

## D95: Who made each change on the event feed (proposed by minion F). Provisional
- **Pick:** an Event's `by` is the caller whose call made the change: the agent, with its session, machine and title, or the user. It is unset when the daemon made it on its own: a machine's reachability or any machine change (they follow from operations), an operation's steps and end, a question timing out. An Inbox request is the asker's when made and when withdrawn (only the asker withdraws, D38.10), the answerer's once answered; the grant an approval makes is the answerer's. A review chat note change is the caller's, the user's when showing a diff outdates notes. A user's CLI login says no `via`, since the daemon doesn't know here whether the login is on the host. Recommendation: keep it.
- Reverse: `identity::actor` (moved from `operations.rs`) and the `by` argument of `Feed::publish` and `publish_in` in `daemon/humatad/src/events.rs`, with each caller's value.

## D96: The Inbox and grants kept across a restart (proposed by minion F). Provisional
- **Pick:** each Inbox request is one row of the store's `inbox_requests` table, keyed by its id and gone with its session. The row holds the request as it stands, the idempotency keys that made it, the target an ask-ahead named as the agent sent it, and the grant its answer made, with how the user showed presence, where they answered and when it was revoked. Every change is saved before the asker's wait or the feed hears of it; when the save fails the answer is refused and a grant it made is taken back. On start the Inbox comes back as it was. Pending requests stay pending, though their waits ended with the old process, so the agent waits again or repeats its call with the same key. Each grant still standing is rebuilt by asking the policy again what the saved target names, so it is exactly as narrow as it was; new grant ids start above any ever given out. A request whose session, or what its target names, is gone is withdrawn as SESSION_DELETED and its grant dropped. A row the store can't read is left out and said in the log. Recommendation: keep it.
- Rules aren't saved: they can't be set over gRPC yet, so they're always the defaults.
- Reverse: the migration adding the `inbox_requests` table, `daemon/store/src/inbox.rs`, `InboxEntry`, `InboxGrant` and `GrantedBy` in `api/proto/humata/store/v1/store.proto`, `daemon/humatad/src/inbox/kept.rs`, `Inbox::commit` and `Inbox::revoked` in `inbox.rs`, and Access's `inbox` in `access_service.rs`.

## D97: ListViews with no session (proposed by minion F). Provisional
- **Pick:** `humata views` with no `--session` lists, for the user, every session's offered views on the daemon it asks, newest first; drafts stay the agent's (VW-14). An agent naming no session gets its own session's, drafts included, as before; any other caller must name one. `humata view open|later|not-now|revert|keep` sends SetViewState with an idempotency key, so a retry acts once. Recommendation: keep it.
- `include_forks` still lists only the session's own views: no session record says which session it was forked from until forking is built.
- Reverse: the `(None, Caller::User(_))` arm and `every_offered` in `daemon/humatad/src/review/views.rs`.



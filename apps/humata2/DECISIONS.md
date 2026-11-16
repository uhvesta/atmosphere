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

## D9: The earlier H2-0/H2-1 code (OVERVIEW I0). Settled: delete, owner 2026-11-15
- **Owner, 2026-11-15:** delete the four folders and their Bazel targets (TIDY); git history keeps them.
- `apps/humata2/app` (Swift), `bindgen`, `core`, `rehearsal`.
- Deleting them is irreversible, so it isn't done provisionally.

## D10: The terminal engine (OVERVIEW I6). Decided 2026-11-14: libghostty
- The VT core is libghostty-vt: parsing, screen state and scrollback, cross-platform. It is drawn by our own GPUI terminal view.
- Humata 1's Zig-in-Bazel setup for GhosttyKit is reused to build it.
- Fallback: alacritty_terminal behind the same interface.

## D11: Deleting a channel (REVIEW H7). Settled (a), owner 2026-11-15
- **Problem:** the spec can create, rename and change a channel, but nothing removes one.
- **Options:**
  - (a) **Delete channel…** sends everything of the channel to trash, after the unpushed-work check; restorable for 30 days.
  - (b) Only a channel with no sessions (live or archived) can be deleted.
  - (c) No deleting; archive every session and leave the empty channel.
- **Provisional:** (a). It matches how sessions are deleted (LD-5) and nothing is lost for 30 days. User-only; agents can never delete a channel.
- **Affects:** CH-24, CH-25, LD-6, the channel page's menu.
- **Reverse:** narrow it to (b) by refusing while the channel has sessions.
- **Owner, 2026-11-15:** (a) as frame 71 draws it: everything goes to each host's trash for 30 days, and a session with unpushed work blocks the delete until its own "Delete that work" is ticked.

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

## D17: The new GUI binary is `desktop`, not `app` (OVERVIEW I0). Settled 2026-11-15 (main)
- **Why:** `apps/humata2/app` held the earlier Swift code. D9 deleted it (TIDY, 4a636632), and every target, test and doc already says `desktop`, so it stays `desktop`.
- **Reverse:** rename the package; nothing outside `apps/humata2` refers to it.

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
- **Pick (owner):** the frames in `desktop/tests/__snapshots__/design/` (33 when picked, 43 since D100; drawn by `desktop/tests/design.rs`, target `//apps/humata2/desktop:design_test`) are what the UI should look like. Screens are built to match them. A design change goes to the frames first, with the owner's OK.
- **Picks (owner):** right panel switched from a labelled strip on the right edge, plus a header toggle; terminals and files are tabs in one row; the only split is two tab groups in one session, at most one terminal per agent, and anything more is a fork (code + context, context only, code only); indented sidebar tree; a channel opens to its Sessions tree table; Board kanban, drawer, table and dashboard, and the expanded Board panel shows the kanban; Home and Inbox list what needs you first; New session runs any agent's TUI or a plain terminal.
- **Platforms (owner):** macOS, Linux and web only.
- **New session revision (owner, 2026-11-15; recorded by minion C1):** choose the agent or Terminal, then launch it with its own configured defaults. Remove the separate Opus/model row; model selection stays in the agent UI. This revises frame31 and SE-2; the approved frame source/image is retained as the earlier reference.
- **Revises D74:** no general pane workspace or Panes sidebar.
- **SPEC tidied to this (minion J1, 2026-11-15):** NV-1 (the icon rail) is removed, and the rail is gone from NV-4, IN-1, HO-5, FM-15 and flow 26.4; NV-2 lists the sidebar as the frames draw it. SE-2, FK-3, PR-4 and ST-15 no longer pick a model. SE-27–SE-29 define the two tab groups and the second agent (D103). The frames this decision leaves out of date (29, 31 and others) are listed in D113.
- **Reverse:** re-record the frames from the earlier options in c16dff55.

## D74: The window is a workspace of panes (SPEC NV-28–NV-34, owner direction 2026-11-14). Provisional
- **Revised (owner, 2026-11-15):** no pane tree. The owner disliked the pane view, and how panes changed when switching channels and sessions. The window is the left sidebar, the screen in the middle and the open session's side panel on the right; both sides resize by dragging their divider, hide past their narrowest, and reset on double-click. Split, close, focus-toward and layout-undo shortcuts and palette entries are gone (⌘W and ⌘Z are free again). The picks below are superseded where they differ; quiet chrome still holds. `features/layout` was deleted (owner, 2026-11-15). Reverse: restore it and the renderer from a7c6cf3c.
- **SPEC tidied to the revision (minion J1, 2026-11-15):** NV-28 (any pane shows anything), NV-29 (the Panes sidebar), NV-30 (moving, closing and splitting panes), NV-31 (per-screen default layouts), NV-31a (Reset) and NV-35 (Fixed / Simple / Free) are removed from SPEC, with the sentence that said this section wins over the drawn layouts. NV-32–NV-34 stay under their IDs, rewritten for what still holds: quiet chrome, dialogs only for decisions, hidden tabs cost nothing. New: NV-36 (three columns, no pane tree), NV-37 (full screen and its shortcuts), SE-27 (two tab groups). NV-12–NV-17 now say columns, not panes.
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

## D98: An agent hears of the notes it may read (proposed by minion F). Provisional
- **Pick:** a Comment event reaches an agent when it may read the note's session at that moment: its own always, another one in its channel by default (ChannelRead), one in another channel only with a GlobalRead grant. A check that would ask the user counts as no, so subscribing never makes an Inbox request. This widens D92 to match ListComments. Recommendation: keep it.
- The check runs per event and per agent subscriber, so a grant revoked later stops the next note, not ones already sent.
- Reverse: `reads_session` and the Comment arm of `Filter` in `daemon/humatad/src/events.rs`, and the guard and machine passed to `EventSvc::new` in `daemon/humatad/src/serve.rs`.

## D99: Sessions show when they were last used (owner direction 2026-11-15)
- **Pick:** SE-26. "Last used" is the user's last typing, opening or answer, not the agent's last output, so a long-running agent doesn't make a forgotten session look fresh. Listed sessions can show it and sort by it, for cleanup.
- Approved frames (18 etc.) are not changed for this now; rows gain it when the screens are next redrawn.

## D100: The Desk, empty states and markdown review (frames 34–43). Decided (owner sign-off 2026-11-15)
- **Pick:** DK-1–DK-9. The agent answers a comment in its terminal; it can also reply in the thread with the MCP tool `comment_reply` (DK-9), built after the rest. Owner direction: tedious MCP and tool calls like this go to cheap subagents (Haiku for Claude Code, Luna for GPT-based agents) with the smallest context window and medium reasoning effort where the CLI allows, written as TP-17 into Humata's skills and tool descriptions. The owner asked for a place, beside Home, Inbox and Board, for ephemeral sessions that manage things (the global board, items and projects, documents), and for barebones screens with no channel or no machine. The owner named it Desk.
- Scratch folders go 7 days after close unless kept; picked folders are never deleted. Moving a Desk session copies its folder and restarts the agent with its conversation.
- Done at sign-off: frames renamed without `-draft-`; the Desk row is in the sidebar of every frame (01–43); frame 33 (first run) stays alongside 34 (empty Home).
- Reverse: drop `desktop/tests/design/desk.rs`, the `desk`, `empty` and `fresh` fields in `design/sidebar.rs`, frames 34–43 and SPEC DK-1–DK-9.
- **SPEC tidied (minion J1, 2026-11-15):** the IDs are unchanged. DK-6 now sits in §22 beside NV-11, and DK-7–DK-9 in §11 with the comment rules; DK-8's sentence on how a comment is sent moved into SM-9 (D104). DK-10–DK-14 add the Desk rules these frames don't settle (D108).

## D101: Resolving and archiving comments (owner direction 2026-11-15)
- **Pick:** SM-17–SM-21. Resolved comments never come back on their own; addressed ones offer Resolve, singly or all at once. Agents may resolve, but only with a reply saying what they did, under their own agent access rule. Archive is the user's alone: it hides a comment everywhere without calling it dealt with, and keeps it under Archived in Info and in the Journal.
- Frames for the addressed, resolved, archived states and the Archived list are still to be drawn for the owner's review.
- Reverse: drop SM-17–SM-21 and the `comment_resolve` tool.

## D102: The missing Fork dialog layout (proposed by minion C1). Provisional
- **Pick pending owner review:** use frame31's dialog framing for Fork: a title, the three FK-2 kinds with consequences, the parent's current point/agent/host/repos/branch preview and Cancel/Create. Toolkit buttons make kind selection keyboard reachable. The selected card has a blue border; the primary action uses the D76 blue, in light/dark. A pending request disables edits/duplicate submission; failure keeps the draft for retry. There is no approved Fork-dialog frame among01–43, so this is a reversible presentation, not a golden match. Recommend retaining it until the owner approves or supplies a frame.
- The initial fake client can fork only from now on the same agent/host. FK-3 earlier-turn/agent/host choices, precise clone/conversation preview and real context-only file ownership are still gaps; they are not represented by pretending that a selector works. The New session model row remains removed per direct owner direction in D76/SE-2.
- **Look at:** `desktop/tests/__snapshots__/native/fork-dialog.png` and `fork-dialog-dark.png`. Choice buttons sit inside consequence cards; spacing/focus outlines are provisional. Existing approved frames and sources unchanged by C1.
- **Reverse:** remove the Fork presentation in `ui/src/overlays.rs` and its bridge observation/input; keep the independently tested fake fork metadata and in-flight reducer guard if desired.

## D103: A second agent in a session (frame 14, SPEC SE-27–SE-29) (proposed by minion J1). Settled, owner 2026-11-15
- **Problem:** D76 and frame 14 allow two agents in one session, one terminal per agent, but SPEC defined a session as one agent. Nothing said which agent sets the status, has turns, gets queued comments, is the MCP identity, or owns the Inbox cards and grants.
- **Options:**
  - (a) The agent the session started with is its main agent and keeps all of those. An added agent is a terminal that runs an agent: its own conversation (pinned, resumed, moved), a running/exited dot, nothing queued for it, and its calls to Humata's tools refused with a pointer to forking.
  - (b) Both agents act as the session and share its grants; cards name which one asked, and each comment picks its agent.
  - (c) Drop frame 14: a second agent is always a fork.
- **Provisional:** (a), as advisor A1 proposed. It adds no new model and fits "anything more is a fork".
- **Recommendation:** confirm (a); move to (b) only if a second agent turns out to need Humata's tools.
- **Affects:** SE-27–SE-29, the Session concept row, SEC-15, SM-9.
- **Reverse:** give the added agent its own session token; nothing stored changes shape.

## D104: One markdown review model, and no spec mode (SPEC §11) (proposed by minion J1). Settled, owner 2026-11-15
- **Problem:** Markdown was specified twice: spec mode (SM-1–SM-12: a session mode, Rendered/Source, a contents list, margin comments) and markdown review (DK-7–DK-9, frames 39, 41–43: any markdown tab, Preview/Source/Side by side, comments on source lines).
- **Pick:** the newer owner sign-offs win (D100, D101, the frames). §11 is now one model: any markdown tab in any session shows Preview, Source or Side by side; comments go on source lines and show inline under them; statuses are draft → queued → sent → addressed → resolved, plus archived; one message shape, `[comment on <path>:<lines>]` then the text (frame 43).
- **Dropped from SPEC:** SM-1 (spec mode as a session mode), SM-2 (a session's set of spec files), SM-5 (highlighting passages changed since the user last read them; the Diff panel's "Since I last reviewed" shows a Markdown file's changes like any file's), and SM-4's contents list (no frame draws one). "Spec mode" is gone from the concept table, IN-17 and MV-13.
- **Rewritten:** SM-4, SM-6 (no comment from a selection in the rendered page), SM-8 (inline, not in a margin), SM-9–SM-11, RV-4, RV-5, RV-15, RV-20, RV-26 and flow 26.20. The send format is stated once, in SM-9. IN-17 gains a row for SM-19's resolve rule.
- **Picks neither a frame nor the owner settles:**
  1. A comment is a draft from its first keystroke. **Comment** sends that one; **Send N comments** sends every draft at once. This is what lets frame 01 ("1 draft comment", "Send 2 comments") and frame 42 (a Comment button, "sent to claude") be one model.
  2. A comment made in a diff adds its side, base and quoted code to that shape (RV-27).
  3. Reopen puts a comment back to draft whoever resolved it.
- **Affects, beyond SPEC:** spec mode has an API that nothing in SPEC asks for any more: `SetSpecMode` and the `spec_mode` field in `api/proto/humata/v1/sessions.proto`, the MCP tool `session_spec_mode` (`daemon/mcp/src/tools.rs`, `api/MCP.md`), `humata session spec` (`cli/src/commands/mod.rs`, `api/CLI.md`), and the SM rows of `api/TRACE.md`. They are left as they are until the owner confirms this entry; removing them is a daemon-side task.
- **Recommendation:** confirm. Bring back a contents list or SM-5 only with a frame.
- **Reverse:** restore SM-1, SM-2, SM-5 and the old SM-4, SM-6, SM-8 from the commit before this entry.

## D105: The Inbox card keeps its full context in the frames' layout (SPEC IN-3, IN-6–IN-9, IN-13, IN-14) (proposed by minion J1). Settled, owner 2026-11-15
- **Problem:** IN-9 required eight parts and a grant for exactly one target. Frames 03 and 26 draw a shorter card with Allow / Deny / Reply… and a checkbox "Allow this for the rest of the session", and a Waiting / Answered / All switch.
- **Pick:** the owner's rule (2026-11-14: a request carries who asks, what it was doing, the agent's reason, an exact preview, the rule and the undo cost) outranks a frame for what a card carries; the frames win on layout and labels.
  - Labels follow the frames: **Allow**, **Deny**, **Reply…**, **Allow all n**; **Waiting / Answered / All** replaces Show answered; the Allowed without asking log sits under the requests in the session panel (frame 03).
  - Content stays whole: "What it was doing" (left out in the session panel, where the terminal is beside the card), the rule with Change this rule, Open session ↗ in the wide inboxes, and an undo line shown only when the action can't be undone.
  - The checkbox is a grant for exactly IN-22's target and its label names that target. A session-wide grant is the unsafe reading of the frame's wording, so until the frames are redrawn that wording stands with the narrow meaning.
  - **Reply…** asks for a note and denies with it.
- **Recommendation:** redraw frames 03, 04, 26 and 27 (D113), as advisor A1 also proposed.
- **Reverse:** cut IN-9's parts 5–7 and let the card be exactly what the frames draw.

## D106: Boards as frames 21–24 draw them (SPEC BD-11–BD-16) (proposed by minion J1). Settled, owner 2026-11-15
- **Problem:** D76 picked the kanban, drawer, table and dashboard, but §13 defined only columns, a drawer and scope pickers.
- **Pick:** §13 now defines the header (Scope, Board / Table / Dashboard, Filter, New item), the Table, the Dashboard, several sessions per item (BD-16), Depends on (BD-14) and requests answered in place on the item (BD-15, the same answer as its Inbox card).
  - Dependencies are links: ✓ when done, loops refused, nothing blocked by them.
  - Every figure on the Dashboard opens the items it counts, "Done this week" and "Done per day" included, so the rule that every screen leads to an action still holds.
- **Not taken:** advisor A1 would cut the Dashboard as state with no action. D76 names it as the owner's pick, so it stays, with the rule above.
- **Recommendation:** confirm. Decide later whether starting a session on an item with open dependencies should say so.
- **Reverse:** BD-14, BD-15 and BD-16 can each be dropped alone.

## D107: The Hosts page shows disk, and two numbers (SPEC HO-3, LD-19, LD-20) (proposed by minion J1). Settled, owner 2026-11-15
- **Problem:** frame 28 draws a disk bar per machine and "11 GB free: new clones stop at 5 GB" with Free space…; LD-19 and LD-20 said low disk shows only where it blocks an action and nowhere else.
- **Pick:** the frame stands. The owner's rule allows state the user can fix from where it is shown, Free space… sits beside it, and the Hosts columns were already kept on purpose (NV-8). LD-19 and LD-20 now say: where it blocks an action, and on Hosts; no notice on sessions, the sidebar or Home.
- **Numbers:** new clones stop when a machine would have less than 5 GB free (the frame's text); the bar turns red and the line shows above 85% used (as the frame is drawn).
- **Not taken:** advisor A1 would remove the bar and the warning from frame 28.
- **Recommendation:** keep the frame; make the two numbers settings only if they get in the way.
- **Reverse:** redraw frame 28 without disk and restore the old LD-19 and LD-20.

## D108: Desk rules the frames don't settle (SPEC DK-2, DK-3, DK-10–DK-14) (proposed by minion J1). Settled, owner 2026-11-15
- **Problem:** the Desk has no channel, but access rules, the trash, moves and paths all hang off channels.
- **Picks:**
  1. **Rules:** a Desk session uses the defaults for new channels; DK-4's two switches are that session's grants on top (DK-10). No new settings screen is needed. The other option is a rule table of the Desk's own.
  2. **Closed, the 7 days and the trash:** Close is the Desk's Archive but leaves the folder. Seven days later a scratch folder that wasn't kept is deleted through the trash, so it is still restorable for 30 days: the 7 days and the 30 days are two steps of one path. A folder with uncommitted or unpushed work is held back and says why (DK-11).
  3. **No Fork, no board of its own, no branch;** its requests show under a Desk group in the global Inbox (DK-12). Frames 37–43 draw no Fork button.
  4. **Copy to a channel…** copies the file into that channel's `shared/` (DK-13).
  5. **Folder:** `<root>/desk/<name>` under the machine's Humata root, and `desk` is a reserved folder name. Frames 37–40 print `~/humata/desk/…` in lower case, which is a second root on Linux (D113).
  6. **Picked folders** are the one exception to FC-1; a move goes to any online machine by MV-13 and MV-14 and leaves a picked original in place.
- **Not taken:** advisor A1 would drop the 7 days and the Closed state for the plain 30-day trash. D100 and frames 36 and 40 have both, so they stay.
- **Also fixed:** OVERVIEW's "moving a Desk session into a channel" matched no rule; it now names DK-3 (move to a machine) and DK-13 (copy a document to a channel).
- **Recommendation:** confirm 1–6.
- **Reverse:** each pick is one DK rule.

## D109: First launch (frame 33) and the empty Home (frame 34) (SPEC NV-11, DK-6) (proposed by minion J1). Settled, owner 2026-11-15
- **Problem:** both are "no channels", and nothing said when each shows. D100 kept both.
- **Pick:** frame 33 shows once, on the first launch of a fresh install; after its dialog closes, Home with Get set up (frame 34) is the empty state from then on.
- **Recommendation:** retire frame 33 and NV-11 and keep only 34, as advisor A1 proposed. Frame 34 has every action 33 has plus the Desk, and 33's blank sidebar is the one frame without the Desk row D100 put in every frame. It needs the owner, because D100 kept 33.
- **Reverse:** drop NV-11; DK-6 already covers the empty state.

## D110: Undo after Archive is a delayed delete (SPEC MV-15, frame 32) (proposed by minion J1). Settled, owner 2026-11-15
- **Problem:** frame 32's toast offers Undo after an archive, but archiving deletes the clones (LD-1), and SPEC had only Restore, which re-clones what was pushed.
- **Pick:** as advisor A1 proposed, the session is archived at once and its clones are deleted when the toast goes (10 seconds, or when the app quits). Undo before then is exact and free; after it, only Restore. A restart in between finishes the delete.
- **Recommendation:** keep. Reword the toast to "frees 1.4 GB", since the disk is not free until the window passes (D113).
- **Reverse:** make Undo a Restore, and say in the toast that unpushed work is gone.

## D111: Settings sections, and no Panes setting (SPEC ST-18–ST-20, frame 29) (proposed by minion J1). Settled, owner 2026-11-15
- **Problem:** frame 29 lists General, Agents, Machines, Layout, Agent access, Storage, Notifications and Keyboard, and draws Layout › Panes (Fixed / Simple / Free). §24 had neither the sections nor Notifications or Keyboard, and D76 dropped the pane workspace the Panes setting belonged to.
- **Pick:** ST-18 names the sections and says which entries each holds; Layout keeps Appearance, Terminal font, Sidebar shows Pinned and Long lines, and has no Panes choice. ST-19 (Notifications) and ST-20 (Keyboard) say the least that makes each section an action.
- **Note:** the app has a Fixed / Simple switch for one or two tab groups. SPEC has no such setting: a session may always split into two groups (SE-27).
- **Recommendation:** redraw frame 29 without Panes (D113), then remove the switch.
- **Reverse:** add a setting that turns tab groups off.

## D112: Wording and small behaviours taken from the frames (proposed by minion J1). Settled, owner 2026-11-15
- **Taken from the frames into SPEC:**
  - one table of status marks (hollow ring for idle, red ✕ for exited, grey dot for offline) and "<machine> is offline" in place of "Host offline";
  - **Terminal** as the label for a plain shell; **Full screen** / **Show the terminal** in place of Widen; **Repos and machines**; **Add a GitHub repo…**; a **VIP** badge in place of a "[VIP]" prefix;
  - **Open big** (a tab in Humata) and **Open in editor** (the user's own editor) are both kept, since frame 06 draws both; a file opened big gets its own tab and the side panel stays open (RV-15);
  - the Sessions tab's Filter field, Open / Archived / All switch and "Idle · PR #491" (CH-12, LD-3); a folded channel's mark and count (CH-16);
  - Home's Needs you, Working and Recent groups with one button per row (HM-1, HM-2); a PR link is pasted into Search (⌘K), since Home has no field (PR-1);
  - the queue bar with Show (SE-23), Upload… in Files (RV-35), and the header's agent · machine and buttons (SE-6).
- **Kept against a frame:** the offer bar keeps **Not now** (the owner's wording of 2026-11-14); frame 03 draws only Open it and Later (D113).
- **Small picks:** Home's New session asks for the channel first; the queue's list has Remove; `humatad` names the daemon in SPEC while the binaries are `humatad2` and `humata2_holder` (D37); "the Mac" is "the host" outside the worked examples.
- **Not taken from advisor A1:** retiring "Open big" (frame 06 draws it); storing views on the session's daemon to match what is built (the owner put views on the host, SL-1, so VW-12 stays and the code has the gap); a "Not in 1.0" list (that belongs to the milestone step, and scripted syncs were the owner's ask).
- **Recommendation:** confirm in one go.

## D113: Frames to change, waiting on the owner's OK (collected by minion J1). Open
A frame changes only with the owner's OK (D76). SPEC already follows the newer decision in each case and names the frame.
1. **29 Settings:** remove the Panes choice (Fixed / Simple / Free) and its note under Layout (D76, D111).
2. **31 New session:** remove the "Opus" model picker beside Runs (D76, SE-2).
3. **03, 04 session Inbox:** add the rule line with Change this rule to the card; word the checkbox so it names its exact target (IN-14); add **Not now** to the offer bar (VW-2).
4. **26, 27 Inbox:** the same card changes, plus "What it was doing" and Open session ↗; show where Allow all n / Deny all and the Allowed without asking log sit (D105).
5. **33 First run:** retire it, or give it the sidebar every other frame has (D109).
6. **37–40 Desk:** print the folder as `~/Humata/desk/…` (D108).
7. **32 Toast:** "frees 1.4 GB" in place of "freed" (D110).
8. **05 Info:** "Claude Code · Opus" shows a model Humata no longer picks; drop it unless the agent reports it.
9. **01, 02, 16:** made-up data only: the header says Working while the sidebar row is orange (01, 02), and the header keeps the parent's title when a fork is selected (16).
10. **18 and other session lists:** a Last used column (D99 already says so).
11. **57, 59 Add machine login, machine page:** wrap the login step's two lines inside the dialog, draw the login terminal at a fixed height, fill the disk bar to the used share, give variable chips Set value and Remove with New variable beneath (D176, HO-7).
- **Frames still missing** (SPEC defines the screen, no frame draws it): New channel and clone progress, the channel's Inbox and Agent access tabs, Add machine and the interactive login, the machine page, Storage, the file manager and transfers, Fork (D102), Move, Archive with unpushed work, an archived session, the PR review dialog, review session and Submit review, a prepared view's tab and its Showing bar, "Couldn't resume" and the restart report line, comment states (D101), Delete channel, Clean up…, the comment box in a diff.

## D114: The queued-message list under the terminal (proposed by minion C1). Provisional
- **Pick pending owner review:** retain frame03/04's compact queue bar. Show expands a scrollable list within that bar, capped at200px, with each message and Send now / Remove; Hide collapses it. A fallback turn ending or failed send keeps a visible Send now. Toolkit controls retain keyboard focus. This extends the approved bar without a new modal; the frames do not draw its expanded state.
- Delivery retains the saved payload until acknowledged. Failed delivery preserves ordering and pauses that session's automatic retry; the person can retry explicitly. Only hook-backed Claude/Copilot Ended automatically flushes, per SE-21–23. Read-only observers are excluded from the reported per-terminal writer count. The fake transport cannot prove real bracketed-paste serialization, durable delivery deduplication or real attach presence; those remain daemon integration work.
- **Outbound scope (owner clarification in chat, 2026-11-15; wording proposed by minion C1):** Humata tracks comments/messages it has written down but has not delivered to the terminal; review comments are batched. The UI says Waiting to send and counts comments for a comment-only batch. The expanded list explains that Humata holds these messages before sending them to the terminal. Once delivery is acknowledged, the batch leaves this list. Nothing here reports or controls the agent CLI’s internal queue.
- **Reverse:** replace only the expandable list presentation after an approved frame; keep the queue correctness fixes.

## D115: Channels and session-start state are plain store tables (proposed by minion FL-D). Provisional
- `channels`, `channel_repos` (main clone cloning/ready/failed), `channel_machines`, `session_details` (start step, model, first message, operation, revision) and `session_repos` are STRICT plain-column tables that cascade with their channel or session. No new store protos; `SessionRecord` unchanged.
- **Reverse:** move them into proto records with a migration that reads these tables.

## D116: Where channel and session folders go (proposed by minion FL-D). Provisional
- `humatad2 serve --channels` sets the channels root, default `<--root>/Humata`. A channel is `<root>/<folder>/{main,sessions,shared}`; a session folder is `sessions/<branch slug without the prefix>`. Clones go to `<dest>.partial` and are renamed into place, so a crash never leaves a half clone that looks finished.
- `--agents` (JSON agent list) and `--remote-template` are hidden test flags in every build, not admin-gated. **Reverse:** gate them behind the `admin` feature.

## D117: CreateChannel's operation and a failed clone (proposed by minion FL-D). Provisional
- Kind ADD_REPO with steps `record`, then `clone_main:<owner/name>` per repo. GRPC.md §7 lists `clone_main:<machine>…, record`; the code records first so a crash mid-clone leaves a channel that recovery can finish. GRPC.md to be aligned when the owner agrees.
- A failed clone fails the operation and keeps the channel with that repo marked Failed (frame 45 has no failed state yet; see FRAMES1).

## D118: Recovering half-done starts after a restart (proposed by minion FL-D). Provisional
- A start that hadn't reached `launch` is finished forward; if that fails its clones and record are removed and LastStart says so. From `launch` on, revive owns it. Recovery runs after reconcile in the same task, and only over ids the store held before the daemon served anything (so it never races a live start).
- A resume revive couldn't do (`Revived::ResumeFailed`) now gets a LastStart `not_brought_back` line (PER-15, PER-29).

## D119: Branch prefix, default agent (proposed by minion FL-D). Provisional
- A new channel's branch prefix is empty until Settings or the gh login exist (ST-13). The default agent is `--default-agent` (claude).

## D120: Other machines in CreateChannel wait for MACH (proposed by minion FL-D). Provisional
- Any machine but this one answers UNIMPLEMENTED naming MACH; an empty list means this machine.

## D121: StartSession answers early (proposed by minion FL-D). Provisional
- It answers once the record is written; the START_SESSION operation runs on and targets the channel. Cheap checks run before the idempotency key is used.

## D122: Renaming a channel keeps its folder (proposed by minion FL-D). Provisional
- RenameChannel changes the display name only (FC-1).

## D123: Stop and resume (proposed by minion FL-D). Provisional
- StopSession acts only on Running (holder shutdown, then Stopped); Launching and Resuming answer busy. Resume reuses kind START_SESSION with steps `launch`, `verify` (no RESUME_SESSION kind). ResumeSession supports MODE_RESUME only; fresh-with-summary answers not built.

## D124: Session status mapping (proposed by minion FL-D). Provisional
- Launching/Resuming → STARTING, Running → WORKING, Exited → EXITED, Stopped → IDLE, ResumeFailed → RESUME_FAILED; an unfinished start shows STARTING.
- Known gap: an agent that exits while the daemon runs stays WORKING until the next reconcile or resume (needs a holder-exit watcher; FL follow-up).

## D125: The app's Channels and Sessions client traits (proposed by minion FL-D, agreed by the main agent). Provisional
- The app's one `Daemon` trait gains two separate traits so FL can fake each service: `Channels` (create_channel, channel, channels, rename_channel) and `Sessions` (preview_branch_names, start_session, session, sessions, rename_session, stop_session, resume_session). Operations are watched through OperationService; terminals stay on `Terminals`. Signatures are in workflow/milestones/FL-D.md's lane report; FL implements them in `clients/` (seam: the main agent applies the `clients/src/lib.rs` lines).

## D126: Terminal focus and outbound messages (proposed by minion C1; owner direction recorded 2026-11-15)
- **Mouse and keyboard:** the embedded terminal supports normal mouse interaction, selection, scrolling and program-requested mouse reports. Keyboard input goes only to the focused terminal. Palette/form/find editing and another terminal's focus cannot leak keystrokes into it. Existing TerminalPane mouse/focus tests and a native palette-isolation regression cover the current fake transport.
- **What Humata can know:** only its own unsent drafts and outbound messages can be shown as Waiting to send. Once terminal delivery is acknowledged, remove that item. Do not claim access to an agent CLI's internal queue or confuse transport delivery with agent processing.
- **Individual or batch:** the owner permits sending each comment as it is written or keeping drafts for Send N comments. Explicit Send writes to the terminal; the CLI determines the meaning and timing. An app-held wait-until-turn-end mode is optional, not mandatory for Send. This supersedes mandatory hold wording in D7 and clarifies D114; SPEC SE-23/SM-10 now record it.
- **Checkpoint limitation:** the review integration being handed off retains the previous batch-only and hook-end hold behavior. Individual Send/direct explicit dispatch is the first next task, not completed behavior. The current Waiting to send UI is Humata's buffer only. D114's expanded-list appearance remains Provisional; no golden frame was changed.
- **Priority:** finish the whole original prompt/frame audit; half/full resizing is secondary. New session chooses the agent or a plain terminal with its defaults, without a redundant model field (already implemented per earlier owner direction).

## D127: How an agent's Ask waits for the person, and where rules are kept (proposed by minion ASK). Provisional
1. Pending is a gRPC error: FailedPrecondition with `Error.kind = ApprovalPending{request}` (kind 57), not a field on every response. The CLI exits 12 `pending` (or waits with `--wait`); MCP answers pending at once with `wait_seconds: 0`, else waits up to 45 s (TP-11).
2. On Allow the call runs whether or not anyone waits; its result is kept 24 h, in memory only. After a restart an approved Ask can't run: the answer is kept and a wait says to repeat the call with the same key. A pending deferred request is withdrawn on restart. **Reverse:** persist calls as replayable requests.
3. If the rule became Never before the answer, Allow is refused (IN-21). Stopping a session withdraws its pending requests (TP-15).
4. **Owner decided 2026-11-15:** reads are allowed by default. GlobalRead (an agent's ListChannels, session info) defaults to Allow unless the person sets Ask or Never; writes and destructive calls still ask. (Batch 1 shipped the default as Ask; batch 2 changes it, and TESTING §5.5's "Ask me" tier uses a set rule instead.)
5. Reads carry no idempotency key, so a repeated read asks again; TP-14 (same key, same request) holds only for calls with a key. **Reverse:** match a pending read by caller and card.
6. Rules are stored as raw ChannelRules/DefaultRules proto bytes in store tables `access_channel_rules` and `access_default_rules`, restored at start. Only channels with a rule set are saved; deleting a channel doesn't drop its row yet. Rules reads are user-only; agents see their own access through Explain.
7. Deferred cards show an AccessAheadPreview and offer Always allow (IN-10, IN-14; batch 2). The store marks such a request with `InboxEntry.deferred` instead of a "no preview" marker.
8. An added agent (SE-29) is refused at the interceptor by a 4th word in the token file, not a `Caller` variant; WhoAmI shows it as unauthenticated. End to end it needs `TabKind::AddedAgent` and an OpenAgentTab RPC (later seam).
9. SEC-15 (process-tree check) moves to ASK batch 2: turning it on breaks tests that use a tab token from the test process.
10. mcp.rs encodes file bytes with a small base64 of its own (no base64 crate in the daemon's deps).
11. Batch 2: SEC-15 checks that a tab token comes from a process in that tab's process tree. On a mismatch the tab's token is revoked and IDENTITY_MISUSED is logged; `--admin` test daemons skip the check. **Follow-up:** cache the holder pid.
12. Batch 2: channel, session and terminal handlers that write defer to the Inbox; streams never defer. `Terminals` wraps `Arc<Core>` (Deref) so a deferred call can run later.
13. Follow-ups: global board writes need their own Ask capability when BOARD is built; grants end on archive/delete later.
14. ASK batch 3 (app side): the Agent access reducer lives in the settings crate; a second change to a rule row while the first is still saving is ignored; "Change this rule" sends go-to-channel then the Agent access tab as two actions (fold into one action if the order proves unreliable); the grant checkbox is drawn by hand until the kit has one.
15. The comment-resolve row (`own_comments_read`, default Allow) stays in IN-17 and frame 49 even though the proto marks that capability deprecated. **Ask the owner:** drop the row, or show it as "Always · no setting" (recommended).
16. The proto comment says `global_read` defaults to Ask me; SPEC and item 4 say Allow. Fix the proto comment.

## D128: Homes for the items brought into 1.0, and small picks (advised by a Fable agent, 2026-11-15). Settled
- The owner brought Contain agents (SEC-6), script tokens (SEC-14) and Delete channel (CH-24) into 1.0; only the VM tier (PER-8–PER-10) stays out. The owner asked that a Fable agent give opinions in place of the owner's.
1. Delete channel → FORK (it reuses MV-9's unpushed check and the trash; needs MACH and ASK). Script tokens → ASK, as a late daemon batch (one more caller kind through the same guard). Contain agents → AWAY (needs MACH and ASK's rules); LINUX adds the containment suite. Exit lines added to each milestone.
2. D127.15: the `own_comments_read` row shows as "Always · no setting" like IN-17's first row; the rule is dropped from the settings reducer and fixtures (the proto never checks it).
3. The approved frames' "typed when this turn ends" wording follows D126: the Desk comment box (`desk.rs:805`) uses frame 50's wording, and the session's queued bar (`session.rs:284`) becomes SE-23's "Waiting to send". This supersedes LOOK1's L2 recommendation.
4. D113 items 1–10 are all fine. Item 5: retire frame 33 and NV-11 (frame 34 covers it, D109). Item 4: keep "Deny all", since the owner already approved it in frame 48.
5. D113 item 9, frames 01/02 (Fable advisor for minion W1): the frames keep the plain working state. When the session's terminal is working, its own request isn't waiting: its sidebar row (and the pinned row) is green with Inbox 1, and the rail's Inbox badge is 1 (only the fork's Push try-brin waits; badges count the session plus its forks, CH-12). Needs-you frames keep orange and 2. One switch in the mock data (`Side::asking`, derived from the terminal state) drives row, pinned row and rail so they can't disagree. Ruling (Fable judge, via main): a session's dot is Needs you only when it has its own unanswered request; forks' requests only add to the ✉ badge. So Try BRIN index (its Push request waits) is Needs you everywhere it is drawn (sidebar, 18, board, machine page, Home 25 and the restart Home, where it moves to the Needs you group with Review). 16's header shows the selected fork's title, keeps "Claude Code · devbox-1", and its rail has no Inbox badge.

## D129: Linux has no pixel goldens; web checks them (owner suggestion, advised by a Fable agent, 2026-11-15). Settled

Pixel goldens are recorded on macOS and checked on web. Linux runs every non-image suite, the view-tree snapshots and one launch smoke test: the real binary under X11 and Wayland, frame 01 non-blank, bundled fonts with no missing-glyph boxes, scale 2. Web stands in for Linux layout, not for the Linux platform (fontconfig, shaping, Wayland/X11 and fractional scale are what the smoke test covers). Risk noted for LINUX: GPUI may not draw in the GPU-less static-musl VM and may need glibc plus a software renderer. Applies to LINUX.md, ANYHOST.md and OVERVIEW's N1 item.

## D130: A sidebar row with inbox requests reads Needs you (minion LOOK1, advised by a Fable agent, 2026-11-15). Provisional
1. A session's row dot (sidebar, pinned rows, and the folded-channel rollup of CH-16) is Needs you (orange) whenever the row's inbox badge is non-zero; otherwise it is the run status. The header and tabs always show the run status. Rollup order: needs you, exited, working, the rest. An exited session with pending requests is orange in the sidebar and red ✕ "Agent exited" in its header (frame 05); it turns red once the requests are answered. (Superseded 2026-11-15: an exited session is red ✕ in the sidebar as in its header; see D131 as amended.)
2. This makes frames 01–05, 16 and 17 consistent as drawn, so D113 item 9 keeps only frame 16's header-title fix.
3. The diff footer in frame 01 said "Send 2 comments" over "1 draft comment" (one comment already sent). Sending counts only drafts, as frame 50 does; frame 01 now reads "Send 1 comment". Frames 01 and 11 gain the per-draft Send of D126.
- **Reverse:** drop `row_status` in `ui/src/sidebar.rs`, then redraw 01, 03, 05, 16 and 17 with the owner's OK.

## D131: Row dots follow the session's own requests (Fable judge, 2026-11-15). Settled
A session's dot is Needs you while its agent is waiting on an unanswered request of its own, that is, while a call of that session is held for the answer (TP-11, TP-12); otherwise it is the run status. A request the agent has stopped waiting on (it took "pending" and went on, ended its turn, or exited) still counts in the ✉ badges and the Inbox but does not change the dot (amended 2026-11-15 by the frame ruling, see the last entry of this file's LOOK2 wave). Forks' requests only add to the ✉ badge and never recolour an ancestor. D128.5 stands but loses its "Try BRIN stays Working" sentence: Try BRIN is Needs you everywhere because its agent is waiting on the push. D130.1 now reads "while its agent waits on its own request" instead of "badge non-zero"; an exited session is red ✕ in the sidebar as in its header (frame 05 changed to match), and D130.2 is dropped. Frame 16's selected fork is Partial BRIN on Claude Code · devbox-1, and frame 18 was corrected to match. SPEC CH-16 gains this rule. In the UI the dot is simply the session's status, since the daemon reports Needs you while a call of the session is held on its own request; `row_status` is gone. The daemon holds Needs you through the short gap between a "pending" return and the agent's next wait on the same request, so the dot does not blink green every wait window.

## D132: Script tokens' smaller picks (proposed by minion ASK, 2026-11-15). Provisional
For SEC-14 script tokens: a script may not subscribe to events. A lifetime of 0 days means 30, and more than 90 is refused. Idempotency replay comes only from memory while the daemon runs. A script sees only its own operations. An unusable token on an open method counts as no credential. Expired tokens stay listed until they are revoked. Capabilities the daemon doesn't know are dropped when the tokens load. The Actor gains `ScriptActor { token_id, name }`, and test daemons take `--script-token-seconds` so tests can see a token expire.

## L14: The side-panel board lists active lanes first (minion LOOK1, 2026-11-15). Provisional
The compact board in a session's side panel lists In progress, In review, To do, Backlog, Done, and only lanes that hold items; the full-screen board keeps every lane in workflow order. Golden 07 now shows frame 12's items so the two frames agree. Reverse: drop the compact branch in `ui/src/panels.rs` `board()`.

## L15: A terminal pane lays out early output at its drawn size (minion LOOK1, 2026-11-15). Provisional
Output that reaches a pane before its first draw is parsed once a draw has measured the cells and fitted the grid, so a session whose program never redraws (offline, exited) shows its output at the real size. After 3 draws without a cell size, or past 1 MiB held, it parses at the size it has, so a pane can never stay blank. Reverse: drop `unfitted` in `terminal/pane/src/pane.rs`.

## L16: A terminal's scrollbar shows only away from the newest output (minion LOOK1, advised by a Fable agent, 2026-11-15). Provisional
The terminal view draws and hit-tests its scrollbar thumb only while the view is back in the history, or while the thumb is held in a drag. At rest on the newest output (the goldens' state) no thumb is drawn, and a press at the right edge selects or reaches the program like anywhere else; wheel, trackpad and the history keys still scroll, and the thumb appears after the first row. Full-screen programs leave history behind on every redraw, so the old rule (any history) showed a thumb in every session. No timers, so captures stay deterministic. Reverse: `scrollbar_shown` in `terminal/view/src/view.rs` back to `bar.scrollable()`.

## L17: Frame 12's asking card and frame 06's capture (minion LOOK1, 2026-11-15). Provisional
1. The full-screen board's asking card reads "Asks to move to Done" with a Review button that opens the request in the Inbox panel; the compact board keeps the Needs you dot. Done cards fade only on the compact board.
2. Selected sidebar session rows are semibold like unread ones (goldens 01 and 16).
3. Native frame 06 is laid out while Ledger runs and the link drops just before the capture (an offline program can't redraw when the layout changes, which drew its input box twice); the offline transcript stops at the running psql line, without the spinner line.
4. Native frame 16 opens the selected fork (D128.5: its header shows Partial BRIN on posted_at and its rail has no Inbox badge). The design fixture's Partial BRIN runs Claude Code on devbox-1 (D131) and, as a Shared fork, lists Ledger's changes and comments.


## D133: Bug reports in the app and the CLI (I9, lane BUG). Provisional
Picked by a Fable advisor 2026-11-15; extends D47's "left for the app and CLI".
1. **The CLI reaches the app, not the daemon:** the app serves a Unix socket at `~/.humata2/app.sock` (mode 0600, `HUMATA_APP_SOCKET` overrides; tests pass a temp path) with one JSON line per request and reply. Only the app has the trees. The first live app keeps the socket; a later one binds only when it is stale. Any process of this user can reach it, agents included, without the daemon's checks; accepted because the three verbs only write a scrubbed folder. Reverse: swap the CLI's live `report()` for a daemon ReportService at gate G; `ReportRequest`/`ReportReply` stay.
2. **Shortcuts:** ⇧⌘R starts or stops a recording, ⇧⌘S takes a snapshot. Help › Report a Bug holds Start/Stop Recording and Take Snapshot. Reverse: rebind.
3. **The title-bar dot is a button**, "Stop recording". The stop gesture itself (click or shortcut) is not recorded, so a replay doesn't end with a stray stop.
4. **Comments are pinned by picking from a searchable list** of the labelled elements in the saved view tree, so every pin exists in `view.txt`. A pick mode on the live window is a later addition.
5. **The form opens by itself** after a stop or snapshot started in the app. After one from the CLI, or a snapshot taken while recording, the app shows a toast "Report saved · Add details" that opens it. The folder keeps its first name (title, else "recording"/"snapshot"), so copied paths stay valid.
6. **Seeded secrets on fakes:** `HUMATA_SESSION_TOKEN`, `HUMATA_TOKEN`, `GH_TOKEN`, `GITHUB_TOKEN`, and any variable ending `_TOKEN`, `_KEY`, `_SECRET`, `_PASSWORD` whose value is at least 8 characters; host variable values as they are set during the run (the keychain has no read, by design). No paste folders on fakes yet.
7. **Kept terminal text:** a checked command "Keep This Terminal's Text in Reports" marks the focused terminal before the stop (scrubbing happens at save). One at a time, not saved, default none.
8. **Reports folder:** `<app data folder>/reports/<date>-<slug>/`, folders 0700.

## D134: The live client implements today's Daemon and Terminals traits (minion FL, advised by a Fable agent, 2026-11-15). Provisional
FL's `clients/live` implements the existing `Daemon` + `Terminals` traits; the D125 split into Channels/Sessions/… waits until after FL (it would touch every feature and the fake mid-change and buys FL nothing; the parity cases move over mechanically). Calls humatad2 doesn't serve yet should answer a distinct `DaemonError` variant (`NotServedYet { what }`) rather than `Refused`, so tests and the UI can tell "not in this release" from a real refusal without reading text; until the main agent adds that variant to `clients/src/daemon.rs`, the live client answers `Refused` from one helper. Reverse: split the traits later and re-slice one impl block.

## D135: The app proves itself to a release daemon by its code signature (minion FL, advised by a Fable agent, 2026-11-15). Provisional
For 0.1 the daemon checks the socket peer: it takes the peer's audit token (`LOCAL_PEERTOKEN`, not a pid) and asks Security.framework whether the process meets a requirement compiled into the daemon (Developer ID anchor, our team ID, the app's bundle identifier). A pass is `Caller::User(UserVia::App)` with no header and nothing stored. It refuses peers without hardened runtime or with `get-task-allow`, `disable-library-validation` or `allow-dyld-environment-variables`, and any peer inside a session holder's tree. The release app takes no action from argv, environment or URL without a click. Updates need no re-pairing; a second copy is accepted (D34.1); an ad-hoc dev build is refused with an error saying to use a signed build or a dev daemon with `--admin`. The keychain key (Ed25519, `EnrollClient` only over a peer-checked connection, `humata-client-key` header signing a daemon nonce; on the host socket key and peer check both required) is deferred to AR-9 pairing. Not fixed by either: the same uid can replace `~/.humata2/bin` and the launchd plist. Needs BOUNDARIES §2.1 and D34.6 amended. Depends on the 0.1 release being Developer ID signed with hardened runtime; if it isn't, write the gap down instead of shipping a key. Reverse: one macOS-only function and a `Caller` branch.

## D136: Live session status is synthesized in one module until humatad publishes session events (minion FL, advised by a Fable agent, 2026-11-15). Provisional
Until humatad's SessionService/ChannelService publish on the event feed (files held by ASK), `clients/live` emits `Event::Session` from a fresh GetSession after its own calls and on each attach's `TerminalState`, all in one module. Sessions with no attach don't update in the sidebar until then; no polling. A DAG task after ASK's daemon files are free deletes the module and turns on the parity case "session events come from the daemon". If daemon events gain a sequence number, keeping read-your-writes updates becomes safe. Reverse: delete the module and its tests.

## D137: `humata` goes on PATH as a symlink in ~/.local/bin with a one-time hint (minion FL, advised by a Fable agent, 2026-11-15). Provisional
The installer links `~/.local/bin/humata` to `~/.humata2/bin/humata`: no admin prompt, no profile edit. It never overwrites a `~/.local/bin/humata` that isn't its own link (Humata 1 may own one); it reports the clash instead. Whether the user's shell sees the folder is asked of the login shell (`$SHELL -ilc 'command -v humata'`, with a timeout), not the app's launchd PATH. If it doesn't, the app shows a one-time dismissible hint with the per-shell line to add (`echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zprofile`; `~/.bash_profile` for bash; `fish_add_path ~/.local/bin` for fish) and a Copy button. Sessions already get `~/.humata2/bin` on PATH (TS-1). Reverse: remove the link.

## D138: The daemon's comment model and how a batch is typed once (minion REVIEW, 2026-11-15). Provisional
- **One model on the daemon:** `humata2_review_diff::CommentLedger` holds a session's comments for its agent, whatever view made them (SM-13): repo, path, side, lines, quoted lines, and for a diff its base (RV-27). Statuses are draft, waiting, sent, addressed and resolved; archived is a stamp beside the status, so Unarchive puts a comment back where it was (SM-20, SM-21). Replies are a thread on the comment (DK-9, SM-19).
- **Kept in tables of their own** (`comments`, keyed by session and comment id, with a delivery-state column and an index on session and archived; `comment_deliveries` for the batches being typed; `comment_numbers` so an id is never reused). Each change writes only the rows it changed, in one transaction, so a draft edit never rewrites the session's review blob and its last shown diffs (main agent, 2026-11-15: performance is a standing owner priority; the first draft of this entry kept them in the review blob). Reverse: none needed.
- **Typed once:** Send saves the batch as Sending, with a key that is never reused and the exact text, before anything is typed; the terminal is asked to type that key once. After a restart a batch still Sending is in doubt: the daemon types it again only when the terminal can say for certain the key never arrived. When it can't say, the batch is Unconfirmed and the user chooses Send again or takes it back; Humata never types an in-doubt batch on its own. The terminal's holder outlives the daemon, so it is the one to remember keys (next batch).
- **Send types at once** (D126): no wait for a turn end. Comments are "waiting to send" only while Humata holds them because nothing could be typed (no terminal, or its shell exited).
- **Addressed (SM-16, D13):** any change seen in the clone to a sent comment's lines counts as the agent's, since the daemon can't yet tell the user's own edits apart. Whitespace-only differences don't count. Reverse: once edits made through Humata are recorded, leave those out.
- **Rules for the agent's two tools:** `comment_reply` and `comment_resolve` each get a capability, Allow by default, on comments typed into the agent's own session (IN-17 gains a row for replies beside the one for resolving). Reverse: one shared rule.
- **The message** follows SM-9 exactly: the repo leads the path only when the session has more than one, for every kind of comment. The app's fake leads with the repo on every diff comment; it is brought in line when `features/review` joins (REVIEW, after LOOK1).
- **Batch 3 picks (minion REVIEW2, 2026-11-15):** `comments_list` is served with the two tools, since the typed message carries no ids: it shows sent, waiting and addressed comments with their threads, never drafts or resolved ones. A reply or resolve naming another session's comment is refused before the access check. Only the agent replies in a thread for now; ReplyToComment from the user stays not built. Reverse: each is one arm (`daemon/humatad/src/mcp.rs` `comments_list`, `review/comments.rs` `agent_comment_change` and `reply_to_user_comment`).

## D139: The peer link relays to the machine's daemon; the host is seen there as the user (settles D43's open question) (minion MACH, advised by a Fable agent, 2026-11-15). Provisional
- **Pick:** `humatad2 serve --stdio` stays a proven relay with no store and no holders. After ProveHost it forwards an allowlist of ordinary client calls to the machine's socket daemon, which runs the same SessionService, ChannelService and TerminalService code as for a local call and takes them as the user's, marked as from the host. PeerService stays off every socket. Calls that need only the filesystem (ApplySkills, Probe) run in the link process. So "a remote session is a local session plus a link": the host writes its record first (SL-1) with an id it mints, then relays StartSession; a start naming an existing id returns that session.
- **Why:** every service refuses the peer caller today, so user-level calls need no handler changes; D43's option (a) gives the store and holders two writers; a link process that is the daemon dies with the SSH connection.
- **How the daemon knows the link:** a header carrying HMAC-SHA256(enrollment key, "humata-peer-relay-v1" ‖ machine id ‖ link pid), honoured only on a machine-role daemon, only for allowlisted methods (Terminal, File, Diff; Session Start/Stop/Resume/Get/List/PreviewBranchNames; Channel Create/Get; Operation Get/Watch/Cancel; EventService.Subscribe), only when the kernel's peer pid matches and is outside every session's process tree. A bad header is refused, never treated as an ordinary caller.
- **Risk, stated plainly:** any uncontained process of the same user can read the key, forge the header and make allowlisted calls as the user without `humata login`. This is D45's class of risk, now reaching user-level session and terminal calls. Inbox answers, access rules, settings and logins stay out of reach. Recommendation: keep it for 1.0 and move the key to the keychain or a kernel-checked credential with SAFE.
- **Gives up:** peer.proto's LaunchSession, StopHolder, ResumeHolder and PrepareClone stay unbuilt (the coarse calls replace them); the machine keeps channel and session rows as a launch cache, not as truth; each terminal byte is copied once more.
- **Reports with no polling:** when the Uplink opens, the link subscribes to the socket daemon's event feed, sends a status (and pointer) report per session, then one per event; every new link starts from a full snapshot.
- **Open:** the daemon-to-host half of the Uplink (forwarded calls, journal, policy) is AWAY's; a remote session's MCP board calls still reach the machine's own store; FC-7 bundles aren't placed yet.
- **Reverse:** the host's calls on the link don't depend on how the link reaches the daemon: swap the relay's inside for a shared-state front end or a private listener (`daemon/humatad/src/relay.rs`, the header arm in `identity.rs`).

## D140: Skills sync engine, scripted rule config and the app's Machines client (minion MACH, 2026-11-15). Provisional
- **Synced set layout:** on a machine `~/Humata/.skills` is a relative symlink to `~/Humata/.skills-sets/<set id>`; a sync writes the whole new set beside the old one, checks every file's SHA-256, and switches with one rename of the link (SK-9). The old set is deleted right after the switch, so a reader that opened the old set a moment before can find a file gone and must read again. Reverse: keep the previous set until the next sync (`daemon/machines/src/skills.rs`, `Incoming::finish`).
- **Never synced, whatever the excludes say (SK-6):** `.env`, `.env.*`, `*.pem`, `*.key`, `*.p12`, `*.pfx`, `id_rsa*`, `id_ed25519*`, `id_ecdsa*`, `.netrc`, `.npmrc`, `credentials`, `credentials.*`, `*.token`, `token`, `.token`; symlinks are never followed. Reverse: `SECRET_NAMES`.
- **Clash picks** live on the machine in `~/Humata/.skills-choices.json`; a real `.skills` folder the user made is refused and never deleted.
- **Scripted rules (TS-3, TS-4):** `[rules.<name>]` in TOML with `source` or `command`, `destination`, optional `before`, `after`, `exclude`; the name `skills` is reserved; a destination must be under the home and outside `~/Humata/.skills*` and `~/.humata2`. One-way only (D49's open point stays).
- **The app's client:** a separate `Machines` trait (`clients/src/machines.rs`) with a scripted fake, taken by the Add machine and machine page reducers as an `Arc` rather than a new field on `Clients`. Reverse: fold it into `Clients` when the live client implements it.

## D141: The Desk in the app's model: a reserved channel id, a record beside the session, its own client (minion DESK, 2026-11-15). Provisional
- **Problem:** a Desk session belongs to no channel (DK-1), but the app's `Session` has a channel, and every session screen reads sessions from the workspace. The Desk also knows things no other session has: a scratch or picked folder, the two access switches, closed with days left.
- **A Desk session is a `Session` whose channel is the reserved id `desk`** (`humata2_clients::desk::channel()`). DK-2 already reserves `desk` as a folder name, so no channel can have it. The session page, terminals, panels, pin, VIP and search work unchanged, and channel trees and pages never list it. Not taken: making `Session.channel` optional (every session literal and every channel lookup changes, across files other lanes hold). Reverse: `desk::is_desk` is the one test; swap it for an optional channel when the daemon's session message has one.
- **Close is `archived` plus a `Closed` record** (when, days left), so lists that hide archived sessions hide closed Desk sessions without knowing about the Desk (DK-11).
- **What only the Desk knows is a `DeskSession` per session in `Workspace.desk`**, with the documents list, carried by load and Resync like `session_options` and `session_files`. Changes are announced with `Event::Resync`, as the board and channel fakes do; no new event kind. Reverse: move the fields onto `Session` once model.rs is free.
- **A separate `DeskClient` trait** (`clients/src/desk.rs`), implemented by the fake daemon and handed to reducers as an `Arc`, as D140 does for `Machines`. Reverse: fold it into `Clients` when the live client serves the Desk.
- **Clean up… sends the list it showed** and the daemon deletes only if that list still holds; otherwise it answers with the new list and deletes nothing (NV-27). Rows that free space and lose no work start ticked, as frame 55 draws; a row whose work changed loses its tick.
- **The dialog names the scratch folder** and sends the name (`FolderChoice::Scratch { name }`), as New session sends its branch; the daemon adds `-2`, `-3` only when the name is taken (FC-14). By default the name is the title's FC-13 slug.
- **Last used (SE-26)** is kept on the Desk record as an order number and a label until the other session lists show it; then it moves onto `Session`.
- **Open:** frames 37–40 print `plan-q4` and `brin-rfc` for the titles "Plan Q4 board" and "BRIN rollout RFC", which are not their FC-13 slugs. What the dialog shows by default goes to a Fable advisor before the views are built; the model fits either answer.

## D142: Files, trash and transfers on one machine (minion FILES early lane, 2026-11-15). Provisional
- **Where it lives:** a new sync, gRPC-free crate `daemon/transfers` (`humata2_transfers`), like `daemon/sessions`: the trash and the transfer engine, with fault points and a kill matrix. humatad only wires it (`files/roots.rs`, `files/ops.rs`, `trash_service.rs`, `transfer_service.rs`). The engine reads and writes only through an `End` trait, so a machine reached over the link becomes a second implementation after MACH. Reverse: fold the crate into humatad.
- **Nothing half-made is ever visible (FM-6, PER-27):** a transfer copies into a hidden `.humata-transfer-<id>` folder inside the destination and renames the top-level names into place only after every file's SHA-256, re-read from the destination, matches. Cancel or undo removes that folder. Listings hide it. A failed transfer keeps it so Resume continues; Cancel removes it.
- **Confirmed offset (FM-8):** bytes are confirmed when fsynced and journaled, every 8 MiB or 64 files, never per chunk. A resume truncates to the journaled offset and re-hashes that prefix on both ends; a difference restarts the file. Reverse: sync per chunk (slower).
- **Moves (FM-14):** on one file system a move is a rename; otherwise copy, verify, then the source goes to its machine's trash.
- **Existing names:** folders never merge. Fail, Replace (the old one goes to trash) or Keep both ("name 2"). The request must say which; nothing is defaulted.
- **Links and special files:** a symlink is copied as a link and never followed; a socket, fifo or device refuses the transfer before anything is written.
- **Trash (LD-6, LD-7):** `<channels>/.trash/<id>/{item.json,content}`; restore never overwrites. Items past 30 days are removed when the trash is next used and at daemon start, not by a timer (TR-9). Reverse: one timer armed for the oldest item.
- **Who may see the trash:** listing and planning an empty are user-only, so agents can't enumerate deleted files; restore needs the trash-restore capability. Open: a file trashed from a session clone can be restored by another session of the same channel that holds that capability.
- **Low disk (LD-19, D107):** a transfer that would leave less than 5 GiB free is refused before anything is written, with needed and free in the error.
- **The app:** a `FileManager` client beside `Daemon` with its own types in `clients/src/file_manager.rs` (no model.rs change), a fake first; the live one comes with FL/MACH.
- **Open, needs a proto line:** FM-10 lets a pane show the user's home on a machine, but `FileLocation` names only Humata roots. Proposed: a user-only `MachineHome home = 7` root (agents can never name it). Not built.

## D143: Start fresh with a summary (PER-15) (minion FL, advised by a Fable agent, 2026-11-15). Provisional
1. **The summary, without an LLM:** the daemon makes a plain digest of the old JSONL: every user message (each capped), the last few assistant text turns, no tool output, about 8 KiB in all. It types the digest, then the old transcript's path ("read it if you need more") and the PER-14 `git status` line. If the conversation file is gone, it uses the tail of the saved scrollback (PER-17) as plain text, plus the title and board item, and says the transcript was lost and this is terminal output. If nothing is left, it seeds only the title, board item and `git status`, and says so. The failed session carries a `summary_source` (transcript, scrollback, none) so the button can say "Start fresh (nothing left to summarize)".
2. **API:** `SessionService.StartFreshSession(StartFreshSessionRequest { IdempotencyKey idempotency_key = 1; SessionId session_id = 2; }) returns (Operation)`.
   - It keeps the same session id, folder, branch and board items. A new conversation id is pinned and fsynced before launch (PER-11/12), then `start_argv` runs.
   - The old conversation id and file go to a previous-conversations list on the record and are never deleted. The scrollback stays above a "Started fresh" marker line.
   - A repeated key returns the same Operation. The seed goes through the write-ahead Sending/Sent path, so a kill retypes it once.
   - If the fresh launch fails, the session stays ResumeFailed with the new reason and the old id stays pinned. The new id is promoted only on success.
   - Allowed only from ResumeFailed (an Exited session has its conversation intact: Resume or Fork). It is refused for FolderMissing and AgentNotInstalled, with their fixes.
3. **The seed is typed and sent automatically.** The button names the summary, and the seed shows in the scrollback. A `PreviewStartFresh` RPC can come later if people want to see the seed before it is sent.
- **Unchecked:** whether the agent record can hold a previous-conversations list, and whether the resume readiness check can be reused before typing the seed.
- **Amended (minion FL, 2026-11-15):** there's no new RPC. The proto already has `ResumeSessionRequest.Mode::MODE_START_FRESH_WITH_SUMMARY`, so Start fresh is `ResumeSession` with that mode. It has the same rules as point 2: idempotency key, the same session id, and allowed only from ResumeFailed. The previous-conversations list is a new store table, `previous_conversations`. Main applies it as a schema seam. `Session.resume_failed = 29` and `Session.interrupted_at = 30` carry the reason and the restart time the frame needs.

## D144: The relay proves itself once per connection with the host's own proof, not a header (amends D139) (minion MACH, ruled by a Fable agent, 2026-11-15). Provisional
- **Problem:** D139's credential was a header, HMAC(enrollment key, machine id ‖ link pid). It never changes, so it replays after a pid is reused, and any same-user process that reads the key file can make it.
- **Baseline, stated plainly:** an uncontained process of the same user can already type into every terminal through the holders' sockets, read and write clones, trash and the store, and replace `~/.humata2/bin/humatad2` (D135 says so). BOUNDARIES §1 puts that process outside what Humata defends against; a contained agent (SEC-6) can't read the key. So the relay gives such a process no new power, only calls recorded as the user's.
- **Pick:** accept the readable key until SAFE, but make the credential fresh and bound to the host. Two DaemonService calls on the machine's socket, open to anyone: `BeginRelay` returns a 32-byte nonce kept in that connection's state; the link uses it as Hello's machine nonce, so the host's ProveHost is bound to it. `CompleteRelay(host_nonce, proof)` is honoured only when, in order: the daemon's role is machine; the peer pid is known from accept and has the daemon's uid; the pid is outside every session's process tree; the nonce was issued on this connection, is unused and under 30 s old (any attempt uses it up); the key passes D45's file checks and the host's proof verifies in constant time. After that, on that connection only: a call carrying any credential header is refused, a method off D139's allowlist is refused, and an allowed call is the user's, marked as from the host. A daemon restart forgets the proof: the link exits and the host dials again (D83).
- **Not taken:** checking the link process's executable or that sshd is its ancestor (the real binary serves anyone holding the key, `ssh localhost` fakes the ancestry, and a provider's connect command has no fixed ancestor); hiding the key (the Linux user keyring is readable by the same user, the login keychain is locked in SSH sessions, a root helper needs sudo).
- **Later (SAFE):** the host makes an Ed25519 pair per machine and keeps the private half in its keychain; the machine stores only the public key and ProveHost becomes a signature over the same bytes. Only `link_proof.rs`, `enroll.rs` and one new dependency change, so this design is not a dead end.
- **What is left:** until SAFE, an uncontained same-user process outside a session tree that reads the key can complete the relay and make allowlisted calls as the user without `humata login`. Inbox answers, access rules, settings, logins and tokens stay out of reach. SEC-9, BOUNDARIES §2.5 and GRPC.md §8 should name this credential.
- **Needs from main:** two calls in `daemon.proto` (`BeginRelay`, `CompleteRelay`); a connection-scoped slot where the socket keeps peer info; a caller value for "the user, via the host".
- **Reverse:** delete the two calls and restore D139's header arm in `identity.rs`; the relay's inside is untouched.

## D145: Storage measuring, retention decisions, Go to file and the far end of a transfer (minion FILES early lane, 2026-11-15; adds to D142). Provisional
- **Sizes (LD-13, LD-14):** a row's size is what deleting it frees: allocated blocks, a hard-linked file counted once, links never followed, measured only when asked. A session's row leaves out its build caches, which get their own rows.
- **What counts as a build cache (LD-15):** a folder holding `CACHEDIR.TAG`; `target/` beside `Cargo.toml`; `node_modules/` or `.next/` beside `package.json`; `.build/` beside `Package.swift`; `.gradle/` or `build/` beside a Gradle build file; `__pycache__/`; `.venv/` beside `pyproject.toml` or `requirements.txt`; at most four levels down, never inside `.git` or another cache. Clear takes only a path that is such a cache right now, renames it aside in one step and then deletes it. A running session's cache needs `confirm_running`. Reverse: let the user add names in Settings.
- **Trim logs:** keeps the last 1 MiB of each log, cut at a line start. A process that holds the old log open keeps writing to the removed file until it reopens, so its lines between the trim and the reopen are lost. Open: have the daemon and holders reopen their logs after a trim.
- **Who may:** GetStorage and TrimLogs take the machine-settings capability (user-only); ClearBuildCache takes own-session-manage. There is no storage capability. Reverse: add one.
- **Retention decisions (LD-16, LD-17):** a pure function from the rules, facts about each session and the time to: what to archive, what was skipped and why, and the next moment something becomes due, so one timer is armed and nothing polls (TR-9). Last activity is the latest of output, input, opened and created. A session due under both rules is archived under the PR-review rule. Work that was never reported counts as not safe and is skipped. Carrying decisions out waits for ArchiveSession and for activity times in the store; neither exists yet.
- **Go to file (RV-11):** candidates are what git tracks or would add (so ignored files and build output never show); a folder that isn't a work tree is walked, capped at 50,000 files. A query matches when its characters appear in order, ranked by exact name, name prefix, name, path, then loose match. A clone that isn't on disk yet is skipped.
- **The far end of a transfer (FM-9):** a resumable `WriteFile` isn't enough for a machine reached over the link, because a transfer builds the whole tree in a hidden staging folder and renames it into place at the end (D142). Proposed for after MACH's relay: a small additive `TransferEndService` (plan a tree, free space, write at an offset into staging, check a staged prefix, finish a file, commit a top-level name, discard). Text in the lane's wiring notes; not applied.
- **The home root (FM-10, settles D142's open item as a proposal):** `MachineHome home = 7` on `FileLocation` and a `Root::Home` in the policy that every agent is refused for as outside Humata's folders, no grant covers and scripts can't name. Exact lines in the wiring notes; not applied.

## D146: A link given a state folder and no socket relays nothing (minion MACH, agreed by the main agent, 2026-11-15). Provisional
- **Problem:** with the relay (D139, D144) every `humatad2 serve --stdio` connects to the machine's socket at Hello and starts a daemon when none listens. Tests and benches start links with a temp `--state` and no `--socket`; with the default socket they would start a real daemon on `~/.humata2/humatad.sock`.
- **Pick:** the link reaches a socket only when `--socket` is given, or when neither `--socket` nor `--state` is given (production: both default under `~/.humata2`). With an explicit `--state` and no `--socket` the relay is off: Hello uses a random nonce and relayed calls answer unavailable. Tests of the relay pass both flags.
- **Not taken:** deriving the socket from the state folder (a second convention to keep in step with `serve`'s default).
- **Reverse:** the `args.stdio` branch in `daemon/humatad/src/serve.rs`.

## D147: LOOK2 picks for frames 18–35, and the rulings on made-up data that disagrees (minion LOOK2, advised by a Fable agent, 2026-11-15). Provisional
- **L1 (Desk pieces):** the Desk row in the sidebar, "Use the Desk / Open Desk" and "New desk session" (frame 34) are DESK's, as LOOK1 ruled for its frames: the route doesn't exist yet and a control with no action breaks NV-7. Reverse: draw them disabled now.
- **L2 (Inbox card):** the request card's inside in frames 26 and 27 stays as ASK built it; LOOK2 matches the list, the switch, the split and the card's frame.
- **L3 (toast):** `ui/src/overlay_toasts.rs` passed to LOOK2 when BUG closed.
- **L4 (controls with no action yet):** the channel's Pin and "…" (frames 18, 35), "Add a GitHub repo…" (frames 20, 35) are drawn disabled until their actions exist (CH-6, a channel pin, a channel menu). "Free space…" on Hosts stays disabled until MACH's action.
- **L5 (Last used):** the Sessions table draws "Last used ↓" as frame 18 does. On fakes the times are text and the fixture is already newest first; sorting by it comes with real timestamps (FL). `Session.last_used` is empty on the live client until the daemon reports it.
- **L6 (empty channel in the sidebar):** a channel with no session shows "No sessions yet" under it unless it is folded (frame 35); its chevron folds it. "Add a machine…" shows under Hosts while This Mac is the only machine (frames 34, 35).
- **L7 (Sessions count):** the channel's Sessions tab hides its count at zero, as frame 35 draws it.
- **L8 (shortcuts sheet):** the five pane-workspace shortcuts D76 dropped (split, close and focus panes, undo a layout change) had no bindings and are off the sheet; ⌘, opens Settings.
- **Rulings on data (the fixture follows the frames unless that breaks another frame):**
  1. Home (25) leaves Partial BRIN out of Working, against its own sidebar and D131: **change the frame** (Working · 3 with the Partial BRIN row; frames 44 and 45 share the list).
  2. Home's "# infra · gpu-box" on Benchmark BRIN: gpu-box is offline in every frame, so the session stays on devbox-1. **App rule:** a Home row that isn't a fork names its machine when it isn't This Mac (HM-1). **Change the frame** to "· devbox-1" there and on Ledger, Upgrade Postgres 17 and API rate limiter (also frame 47).
  3. Try BRIN's Home line: **fixture** ("Asks to push try-brin to origin", 9m). "fork of Ledger schema" is the frame's shorthand: accepted.
  4. Home's Recent: **kept as built**: the last event of each session not listed above, one row per session. The frame's rows and wording are that, not the channel Activity feed HM-2 names.
  5. Channel (18, 48) says 2 waiting where the Inbox (26, 27) draws 3 for the same channel: **change frames 18–20, 48, 49** (Inbox 3; ✉ 1 on Try BRIN and Refund retries; no fold on Refund retries).
  6. Repos and machines (20) contradicts 18: **change the frame** (This Mac 2 sessions and 2 clones; no gpu-box column).
  7. Hosts (28): **fixture** for "last seen 14:02"; the session counts (2, 4, 1) can't be reached by any workspace the other frames draw: **change the frame** to 3, 6 and none.
  8. Inbox (26, 27): **fixture** (order by age; two answered requests for All; the pr-reviews ages). The pr-reviews group and "Allowed without asking · 12" are accepted differences (the frame draws 5 of the 7 it counts; the log is ASK's).
  9. Boards: **fixture** (INF-1's chip is a label, not a live session) and **app rule** (By session lists by need). Dashboard figures are computed from the items and stay; frame 24 draws a busier board than 21 and 23.
  10. Palette (30): **change the frame** ("· working": its own page is the working session); "used <when>" comes from `Session.last_used`; the extra results are real matches and stay.
- **For the owner, one wave:** the frame changes in 1, 2, 5, 6, 7 and 10, plus one question the advisor raised: D131 makes a session Needs you when its own request waits, yet the frames (and the fixture) have waiting requests from Working, exited and idle sessions (Refund retries, Benchmark BRIN, Upgrade Postgres 17, #482). Inbox 7 in frames 01–17 depends on them, so nothing changes until that is answered.
- **Added after the second pass (2026-11-15):**
  - **L9:** the item drawer keeps "Start a session for this item" under History (BD-3; it acts), though frame 22 leaves it out: for the owner's wave.
  - **L10:** Settings' Terminal font stays a stepper (− Menlo 12 +) in a pop-up-shaped box until a font menu exists; frame 29 draws a pop-up: for the owner's wave.
  - **L11:** one push button for every frame button (`ui/src/widgets.rs` `push*`): 24 pt high, radius 6, a 12 pt medium label drawn as a child, because the kit draws its own label smaller and the buttons came out 4 pt narrow.
  - **L12:** with the Desk built, an empty Home offers "Use the Desk / Open Desk" and "New desk session" (frame 34); L1 now covers nothing in frames 18–35.
  - **L13:** accepted differences that stay: the New session dialog's First message box starts 13 pt right of the frame's (the frame's label shrinks beside the long placeholder) and its text sits 6 pt further in (the kit's text area pad).
- **Reverse:** each line stands alone; the fixture edits are in `clients/src/fixture/`.

## D148: A scratch folder keeps the name it started with, and the sidebar lists every open Desk session in start order (minion DESK, ruled by a Fable agent, 2026-11-15). Decided (frame changes approved by the owner's design advisor, golden ruling items 10 and 11, 2026-11-15)
- **Problem:** two places where the Desk frames and the spec disagree. Frames 37–40 print `~/Humata/desk/plan-q4` and `brin-rfc` for "Plan Q4 board" and "BRIN rollout RFC", whose FC-13 slugs are `plan-q4-board` and `brin-rollout-rfc`. Frame 36's page lists three open sessions and its sidebar draws two under Desk.
- **The folder's name** is the FC-13 slug of the title the session starts with, with FC-14's suffix when a Desk folder of that name exists on that machine, and it stays when the session is renamed (as FC-17 does for a branch and D122 for a channel's folder). No predictable rule gives both `plan-q4` and `brin-rfc` from those titles, so frames 37–39, 41 and 43 stand as sessions that were renamed after they started; the fixture says so.
- **The dialog shows the path the folder will get** and has no field for the name: it follows the title as typed, a blank title uses the slug of the title the session will get ("New Codex session"), and it shows the suffix when a Desk session the app knows on the picked machine already works there. The daemon has the last word. `NewForm.name` and `NewAction::Name` are removed; `DeskState::scratch_preview` is what the view reads.
- **Under the sidebar's Desk row: every open Desk session**, on the Desk page and on a Desk session's page. Any filter (agents only, recent only, a cap) leaves the session you are on without its row. They are listed in the order they were started and a row does not move when its session is used: frame 38 types into the second row and it stays second. The Desk page keeps Sort: last used.
- **Frames changed (approved, golden ruling items 10 and 11; main records):** frame 40's caption reads `~/Humata/desk/plan-q4-board · deleted 7 days after you close it unless kept` (the note is shortened so the line still fits; in the app the line wraps for a long name and the path is never cut); frames 36–43 and 55 draw a third row under Desk, "Tidy infra items" with the idle ring, under BRIN rollout RFC. Both are in the drawing code (`desktop/tests/design/desk.rs`, `desktop/tests/design/sidebar.rs`).
- **SPEC wording for main to apply:** DK-1: "…a **Desk** row under Board. While the Desk page or a Desk session is showing, every open Desk session is listed under it, in the order they were started; a row does not move when its session is used." DK-2, in place of the parenthesis: "`<root>/desk/<name>`. The name is the FC-13 slug of the title the session starts with, with FC-14's suffix when a Desk folder of that name exists on that machine: a session started as "Plan Q4" works in `~/Humata/desk/plan-q4`. The folder keeps its name when the session is renamed." DK-14, added: "Under **A new scratch folder** the dialog shows the path the folder will get. It has no field for the name."
- **Gives up:** short hand-picked folder names in the dialog; a short Desk list when many sessions are left open; recency order in the sidebar.
- **Reverse:** a name field in the dialog (a frame 40 change) and `FolderChoice::Scratch { name }` already carries it; one filter or one sort in `features/sidebar/src/lib.rs`.

## D149: How 0.1 is bundled, signed and released (minion FL, advised by a Fable agent, 2026-11-15). Provisional
- **Problem:** T10 has to ship a signed 0.1 and install it in /Applications. The app is a Rust GPUI binary, so rules_apple's macos_application (which wants a Swift/ObjC main) doesn't fit. The release daemon accepts the app only as a Developer ID app of the built team and bundle ID, with the hardened runtime and no injection entitlements (D135).
- **Picks:**
  - The bundle comes from `app/app.bzl` `humata2_app`, as the tree artifact `Humata2.app`. It holds MacOS/humata2 and Helpers/{humatad2,humata2_holder,humata}, and Helpers comes from `daemon/humatad:tools_mac` (darwin-arm64 only). The dev build is signed ad hoc. The release copy is named "Humata 2.app", because spaces in Bazel output names are fragile.
  - Info.plist sets: bundle ID HUMATA2_BUNDLE_ID from @lifr_identity, executable humata2, name "Humata 2", and LSMinimumSystemVersion 26.0. The version is set by the release script.
  - `tools/release/release.sh`:
    - It signs inside out with Developer ID, `--options runtime` and no entitlements.
    - It checks the signature against the daemon's own requirement, using the team the daemon was built with, and fails if that team is empty.
    - It notarizes and staples the app, then uploads a draft `humata2-v<version>`.
    - The draft holds the app zip, the three Mac binaries and SHA256SUMS. There are no Linux builds while MACH is out of scope.
    - `check.sh` verifies the draft as downloaded, and only then is the release published.
  - `install.sh` downloads the published zip and checks it against SHA256SUMS and Gatekeeper. It then replaces "/Applications/Humata 2.app" without opening it.
  - The first version is 0.1.0.
- **Gives up:** the signed app is never launched by the scripts. GPUI under the hardened runtime, the first-launch install and the D135 peer check are checked only statically; the owner's first open is the real test. There's no THIRD_PARTY_NOTICES and no icon yet.
- **Reverse:** a Swift shim around the Rust binary would allow macos_application. Linux builds go into Resources/tools when MACH ships.

## D150: Channels, sessions and terminals on another machine: the smaller picks (minion MACH, 2026-11-15). Provisional
- **Step names:** a channel's operation on the host gets, per remote machine, `record:<machine>` and `clone_main:<machine>:<owner/name>`, extending D117; this machine's steps keep D117's names. Reverse: `channel_steps` in `daemon/humatad/src/remote_work.rs`.
- **An offline machine:** creating a channel doesn't fail on it: its steps wait and the call is made once when its link returns (CH-19). Starting, stopping or resuming a session there answers the machine-offline error at once and queues nothing (CH-4).
- **Ids:** the host mints the channel and session ids and sends them in `humata-host-assigned-id`; the machine uses that id only from a caller that came through the proven relay (D144), and a call naming an id it already has returns that channel or session instead of making a second one.
- **Lost work after a machine restart:** a start the machine forgot is asked again under the same id (at most three times per link); a lost resume fails the host's operation, saying to resume again, rather than resuming twice.
- **What a remote session shows:** its last reported status; OFFLINE while its machine's link is down, with when it was last seen. "n sessions reattached" (HO-6) counts sessions that were active when the link dropped and have reported since.
- **A remote terminal while its link is down (SE-16):** the client's stream stays open and gets one state saying the machine is offline and a disconnected notice; typing is dropped, never queued; on return the host re-attaches from the last offset it delivered and says reconnected. A resume the machine rejects is retried once from the start, so the client gets a fresh screen. The count of caught-up lines isn't sent (the host never decompresses chunks, TR-11): the client works it out from the offset.
- **Interactive machines (MP-16):** marked by a file `<state>/interactive/<machine>`; a takeover installs no service, so after a reboot such a machine needs a new login; the login opens for provider creates that reach by `connect` only.
- **Reverse:** each is local to `remote_work.rs`, `remote_terminal.rs` or `daemon/machines/src/login.rs`.

## D151: The app on the daemon's comment model: client, saving drafts, hand-over, counts (minion REVIEW3, advised by a Fable agent, 2026-11-15). Provisional
- **Client:** a separate `CommentsClient` trait (`clients/src/comments.rs`) with its own `watch` stream, handed to reducers as an `Arc` as D140 and D141 do, so a draft edit never reloads the workspace. Reverse: fold it into `Clients` when the live client serves CommentService.
- **Saving a draft (SM-9):** the box creates the daemon draft on its first non-blank text (one create key per box) and saves edits after 400 ms without typing, and at once on Comment, Send N or closing the box. One write per comment is in flight at a time; later text waits behind it, so the app's own edits never go stale. A stale refusal re-sends once, only if the reloaded comment is still an unarchived draft; otherwise the text stays in the box with the error. Closing a blank box drops only a draft that box created; an existing comment gets back the text it had when the box opened. Reverse: the debounce constant and one arm each.
- **Hand-over of drafts saved by older builds** (`ReviewSaved.queued`, `ReviewSaved.drafts`): each becomes a daemon draft once, keyed on a hash of session, repo, path, line, text and how many identical ones came before (not the index, which shifts as entries go), and leaves saved state only after its create answers; on any error but session-not-found it stays saved. A blank box is skipped. The repo comes from the comment's context, then the selected repo, then the only repo whose diff has the path; only failing all three is it handed over detached. Reverse: none needed after one release; the `serde(default)` fields stay.
- **§26.20 step 3 is stale:** D126 and D138 came later; an explicit send types at once and shows sent, and "waiting to send" means Humata holds it (no terminal). Flow tests follow D126, plus a case with no terminal (waiting, then Take back). The reducer reads the status from the Send answer and never assumes sent. SPEC §26.20 step 3 and review.proto's "now or at turn end" and "waiting for the turn end" comments need rewording (main's files).
- **Archive (SM-20):** an open box is saved and closed first; the toast shows after Archive answers, and its Undo comes back to the comments reducer as its own action (Unarchive); a failed Unarchive shows an error toast. Reverse: drop the toast's undo.
- **Counts (SM-15):** the daemon's `draft_count`, one copy per session in the reducer, re-listed after a watch gap rather than kept from deltas. Send N's N is the drafts actually held and sent, counted separately (PR drafts may make the two differ).
- **Show resolved (SM-12):** per session, saved across restarts; resolved comments are fetched only while it is on. Reverse: delete the saved field.

## D152: The "[Humata] …" line comes from the holder, not a shell wrapper (minion FL, advised by a Fable agent, 2026-11-15). Provisional
- **Problem:** revive (`announced`) and start fresh (`marked`) wrapped the program in `/bin/sh -c 'printf …; exec "$@"'` to print the line that explains a restart. A stored argv then held the wrapper, so it could be wrapped again on a relaunch, and the line depended on a shell.
- **Pick:** `humata2_holder::spawn_with_banner(spec, text)` passes `--banner <text>`. The holder writes the text into its scrollback and its save log once the program has started and before the program's first byte. The text goes on lines of its own, with a blank line after it; control characters are dropped. `HolderSpec` is unchanged. `launch_agent` takes `line: Option<&str>`, and revive's shell revive uses the banner too.
- **Also:** revive's Launching branch keeps the saved output (it calls `stop_holder`, not `shutdown`) when the session has previous conversations, so a kill during a start fresh doesn't wipe the old conversation's terminal.
- **Gives up:** a holder binary from an older build refuses the flag; the tools are bundled, as with `--memory-only`. A relaunch after a kill while the fresh agent was still launching shows the relaunch line, not "Started fresh"; the seed carries the context.
- **Reverse:** `banner_bytes` and the push in holder `server.rs` `start()`.

## D153: A start fresh keeps its first message until it is typed (minion FL, advised by a Fable agent, 2026-11-15). Provisional
- **Pick:** a store table `fresh_seeds` (one row per session: conversation id, seed, time). The row is put before the launch and dropped once keyed input answers Written or Repeat, or the terminal can't take keyed input. After a restart, `retype_pending` types each row whose agent runs on that conversation and whose conversation file doesn't already hold the tag `[start fresh <id>]` that leads the typed text. A row for a conversation the agent left is dropped. A second start fresh of a conversation that never got its message reuses the row's message rather than summarizing an empty conversation.
- **Also (T14):** revive doesn't tell an agent it was interrupted while a kept first message is pending for its conversation, so the seed is that conversation's first word. `Session.summary_source` says what Start fresh would summarize, and the button says so when nothing is left.
- **Gives up:** nothing beyond the table's upkeep.
- **Reverse:** drop the table in a later migration, and drop `keep_seed`, `forget_seed` and `retype_pending` in `session_service/start_fresh.rs`.

## D154: The release check starts the signed app headless (minion FL, advised by a Fable agent, 2026-11-15). Provisional
- **Pick:** `humata2 --version` prints "Humata 2 <CFBundleShortVersionString>" read from the bundle's Info.plist, and exits before any window, install or daemon. `tools/release/check.sh` runs the downloaded app's binary directly (not `open -g`) with HOME set to a scratch folder, then checks three things:
  - The output names the release's version.
  - The app didn't write `~/.humata2`.
  - The exit code is 0. A 137 (killed by the code-signing check) gets its own message.
- **Gives up:** this proves the signed binary loads and runs under the hardened runtime. It doesn't cover LaunchServices, App Translocation, GPUI and Metal, or the first-launch install; the owner's first open still tests those. The Info.plist scan is plain XML, and a binary plist fails the check loudly rather than passing.
- **Reverse:** the `--version` branch in `desktop/src/main.rs` and the probe block in `check.sh`.

## D155: Board items in the daemon: keys, sessions, Depends on, asks and conflicts (minion BOARD, advised by a Fable agent, 2026-11-15). Provisional
- **Writes elsewhere:** `CAPABILITY_GLOBAL_BOARD_WRITE = 7` is no longer deprecated (amends D21 and D127.4 for writes). Add, edit, move and delete on the Global board or another channel's boards are Ask me by default, whoever made the item; reads there stay GLOBAL_READ. The Agent access frame (IN-17) needs a row for it: a design golden, so it waits for the owner's OK.
- **Item keys:** the prefix is the first 3 ASCII letters or digits of the channel's name, uppercased, fixed by the first item and kept through renames; a name with fewer than 2 gets `CH`. A prefix another channel holds is lengthened with the name's next letter, then a digit. The Global board uses `GLB`. Session boards share their channel's counter, and numbers are never reused. `BoardItemId` stays opaque.
- **Sessions (BD-16):** `repeated sessions` on the item, written with `sessions_add` and `sessions_remove`. `assigned_session` and `clear_assignment` are deprecated and refused as an invalid field. Global items may hold any session.
- **Depends on (BD-14):** `depends_on_add` and `depends_on_remove`. The caller must be able to read each target, and an unreadable item is answered exactly like a missing one. A loop, a self edge included, fails with `Error.dependency_loop` (58, FailedPrecondition, CLI exit 7) naming the cycle. Deleting an item drops the edges to it and writes a history entry on each dependent.
- **Conflicts (SL-7):** a write without a revision never conflicts; the app's drags send none. A write with one conflicts only if a field it sets changed since; the session and dependency lists never conflict. Approving an ask runs the write fresh; if it went stale meanwhile, the approved call ends with a revision conflict.
- **Asks in place (BD-15):** `BoardItem.pending` is filled for the user only. The app words the buttons from the change: Move to X / Keep in Y, Apply edit / Keep as is, Delete item / Keep item. Create asks show in the Inbox only. Deleting an item withdraws its asks with `WITHDRAW_REASON_ITEM_DELETED`. Items aren't re-sent on the feed when an ask comes or goes; the app refreshes on Inbox events. Asks don't survive a daemon restart, as other deferred calls don't.
- **Ranks:** fractional strings, one rank space per column; a stale `after` counts as unset (the top).
- **Also:** a fork's board starts empty. Items of archived or trashed sessions and channels are hidden and come back on restore; a purge deletes them, drops their inbound edges and withdraws their asks. History is structured (kind, field, before, after, cause) beside the rendered line. SE-3 records the started session as the change's cause; the StartSession hook is a later seam.
- **Gives up:** lists that mix scopes (a channel with its sessions, Global with channels) drop the boards the caller can't read rather than failing.
- **Reverse:** the board tables in a later migration; `board_service` in humatad; re-deprecate capability 7.

## D156: Transfers to and from another machine (minion FILES, 2026-11-15; adds to D142, D145). Provisional
- **The far end mirrors the engine's steps:** `TransferEndService` (additive, in files.proto) has one call per step the engine takes on a local disk: plan a tree, free space, exists, make a folder or link, write at an offset (synced before it answers with the confirmed offset), hash a prefix, finish a file, move without replacing, sync a folder, remove what a transfer staged. Reads reuse `ReadFile`; a moved source goes to that machine's trash with `MoveToTrash`. It is for the user and the host's relayed calls, never an agent; every location is still judged like a FileService call, and remove only works inside a staging folder. Reverse: fold the calls into FileService.
- **The engine doesn't change shape:** the host hands it one end that routes per path: real paths go to the local disk, and an entry on another machine is named by a made-up path, `/@humata/<machine>/<hex of the location>/<path>`, that an end over MACH's link turns back into calls. Machine ↔ machine goes through the host with both ends remote (FM-9). Cost: error text for a remote entry shows that made-up path until the service rewrites it.
- **Offline waits, it doesn't fail (FM-8, FM-13):** a step that finds the link down ends the attempt with an offline problem; the API shows waiting-for-machine with the machine named, and the transfer resumes from its confirmed offset when that machine's link comes up, by push, with no timer. A transfer started toward a machine that is offline is accepted and waits, since the host can't check the destination and the intent is clear. Reverse: refuse at start with the machine-offline error.
- **The host judges, the machine trusts (D144):** the host checks the caller's access to both places before it starts, because the machine takes a relayed call as the user's.
- **The live file manager client** lives in `clients/live` and maps errors by kind, never by text. Stopgaps to replace: the local UTC offset is read by running `date +%z`; a disk-full start is caught by waiting 300 ms for the first state (the failure's kind should ride on the tray row instead); a broken watch stream reopens on a backoff.

## D157: Wiring remote channels, sessions and terminals into the services: the picks (minion MACH, 2026-11-15). Provisional
- **The host's record of a remote session** is written before the start is relayed (SL-1) by a new `Starter::place_session`, with its start step saved as done, so the host's own recovery never launches it locally after a restart. Until the machine's first report arrives it shows Idle. Reverse: a start step of its own for "placed on another machine".
- **Who may start there:** the existing `SessionStartOtherMachine` capability with the machine named in the target.
- **A start that fails:** the link is checked before any record is written, so an offline machine leaves nothing. A start the machine refuses removes the host's record. If the call fails after the machine did create the session, the host removes its record and the machine keeps an orphan; a later start with a new id doesn't reuse it. Recommendation: have the host ask the machine for the id before removing.
- **A channel the machine already has** (same host-assigned id) is joined again: repos it lacks are added, failed main clones are tried again, a clone still running is left alone.
- **The channel's operation** is ended once, by one task that waits for the local clones and every remote machine's outcome; `clone_in_background` now returns its result instead of ending the operation.
- **Only the host relays:** a machine-role daemon is built without the remote parts, so its local path and its old answers are unchanged.
- **Not wired yet:** PreviewBranchNames for another machine, a remote machine shell with no session, AddRepo and AddMachine on an existing channel, and the "n sessions reattached" message on Reconnect (HO-6).
- **Store:** table `remote_session_reports` (migration 13), the last reported status and pointers per remote session, kept after the host's session row is gone.
- **Reverse:** the seam's edits in `channel_service.rs`, `session_service.rs`, `session_service/remote.rs`, `terminal.rs`, `serve.rs`; `daemon/humatad/src/remote_work.rs`.
- **Follow-up for FL (agreed with the main agent, 2026-11-15):** AddRepo and AddMachine on an existing channel are not served on the host, so flow §26.21 as the spec writes it can't run. MACH's stage I closes on the create-while-offline form of the same rule (CH-19: a machine that is offline clones when it returns), which `daemon/humatad/tests/remote.rs` covers; the AddRepo form stays in that file as an ignored test. When FL serves the two calls, they take the path CreateChannel takes (the channel's full repo list to `RemoteWork::create_channel_on`, the same per-machine steps) and the ignore comes off.

## D158: The frame wave after LOOK2's audit: twelve rulings (Fable design advisor for the owner, 2026-11-15). Settled
The owner handed the wave to a Fable advisor ("either of these UIs look fine, consult with Fable"), so its rulings are approved frame changes. The ruling, with reasons, is kept in the session scratchpad as `golden-ruling.md`; this entry is the record. It settles the "for the owner" lines of D147.
1. **Home (25; 32, 44, 45 share the list):** Working · 3, Partial BRIN on posted_at first.
2. **Home (25) and the restart Home (47):** Benchmark BRIN is on devbox-1, and rows that aren't forks name their machine when it isn't This Mac (HM-1): Ledger schema migration, Upgrade Postgres 17, API rate limiter.
3. **Channel (18, 19, 20, 48, 49):** Inbox 3; ✉ 1 on Try BRIN index and Refund retries; no fold on Refund retries; "Allow all 3" and the refund row in 48.
4. **Repos and machines (20):** This Mac 2 sessions, acme/ledger 2 clones, no gpu-box column.
5. **Hosts (28):** 3 sessions, 6 sessions, none. The machine page (59) follows: Sessions on it · 6 with the two infra sessions, folders 4 and 2, API rate limiter idle.
6. **Palette (30):** "· working".
7. **The dot rule is B:** a session is Needs you only while its agent is waiting on its own unanswered request (a call held for the answer, TP-11, TP-12). A request the agent stopped waiting on still counts in the ✉ badges, the Inbox and Home's Review, but doesn't change the dot. So sessions that work, idle or exited with a request waiting keep their run status in every frame, and frame 05's sidebar row for the exited Ledger session becomes the red ✕ like its header. CH-16 and D131 are reworded; D130.1's exited-orange sentence is dropped. The daemon holds Needs you through the gap between a "pending" return and the agent's next wait on the same request, so the dot doesn't blink (for the daemon lanes: ASK, FL).
8. **Item drawer (22):** "Start a session for this item" is drawn, last, after History (BD-3); the app already has it there.
9. **Settings (29):** the frame stays; the app builds the pop-up: sizes 10–20 for the current family ("Menlo 10" … "Menlo 20"), set directly. A family section comes when there is a second family. The − / + stepper goes (replaces D147 L10).
10. **New desk session (40):** the caption's folder is `plan-q4-board` (DESK).
11. **Desk rows (36–43, 55):** a third row, "Tidy infra items" (DESK).
12. **Agent access (49):** "Global board, or other channels and their boards: read" is Allow, and a new row under it, "Global board, or other channels' boards: change", is Ask me (D155). IN-17's table and the app's rules gain the row. The machine page (59) shows Ask me for the same rule.
- **App side:** the fixture already matches 1–6 except where D147 moved it; item 7 needs no view change (the dot is the status the daemon reports); item 9 is `SettingsAction::{FontMenu, SetFontSize}`; item 12 is one row in `clients/src/fixture/rules.rs`.
- **Reverse:** each frame change is a few literals in `desktop/tests/design/`; re-record from the commit before this wave.

## D159: Bug reports from the palette, draw mode and recorded RPCs (minion BUG2, advised by a Fable agent, 2026-11-15; adds to D31, D133). Provisional
- **Palette commands:** "Start recording a bug report" ⇧⌘R (reads "Stop recording" while one runs), "Take a snapshot for a bug report" ⇧⌘S, "Report a bug…" (stops a running recording, else takes a snapshot; the form opens either way), "Open the last bug report" (listed only once a report exists this run). They act like the menu items and keys; the snapshot waits two frames so the palette is not in it. Palette steps that lead to a report command are cut from the recording, so a replay does not re-run them. Reverse: drop the rewind and let the steps show.
- **Sheet entry:** a "Marks" section under the pinned comments with a quiet button "Draw on the window"; with strokes the header reads "Marks · 2" and lists each as "1. a circle around …" with a ghost "Remove". Reverse: count on the button only.
- **Toolbar:** a floating pill, top-centre at y = TOP_BAR + 12, 36 high, radius 10, `look.card` fill, 1px `look.line` border, the sheet's shadow. Pen (always selected, accent; the only tool, so a state indicator), divider, ghost "Undo" and "Clear" (disabled with no marks), 12px `look.secondary` hint "Draw around what's wrong · Esc to finish", primary "Done". ⌘Z undoes; Esc is Done; no redo. Reverse: drop Pen until a second tool exists.
- **Strokes:** one red `0xff453a` in both themes, 3px, round caps and joins, opaque; the stroke in progress looks the same. Each finished stroke has a 16px red badge with a white 10px numeral at its first point (clamped inside the window), matching its number in the prompt and renumbered after undo or remove. No dim or tint on the window; crosshair over the layer, arrow over the toolbar. The sheet hides while drawing and the layer takes every pointer event. Reverse: a 4% tint; no badges.
- **Shapes:** a path shorter than 6px is a dot (radius 4; "a dot on …"); a path that ends near its start is a loop ("a circle around …", the outermost labelled elements whose centre lies inside it, else the smallest one under its centre); anything else is a line ("a line across …", the smallest labelled element under each point, in order). At most five elements per stroke, then "and N more". Covers come from the snapshot's saved view tree and layout, so they match `view.txt`.
- **After Done:** strokes are not drawn over the window (the scrim returns) and come back on re-entry.
- **Accessible labels:** layer `Role::Canvas` "Drawing layer"; toolbar `Role::Toolbar` "Draw tools"; buttons "Pen" (selected), "Undo last mark", "Clear all marks", "Finish drawing" (visible text Undo / Clear / Done); sheet "Draw on the window", "Remove mark {n}". Bare "Undo" and "Done" would clash with the toast's and the sheet's.
- **Folder:** `drawing.json` (window size, strokes with shape, points in window points and covered elements) when there are strokes, removed when none; `report.md` adds a "## Drawings" section and a Summary line only then, so a folder without strokes is unchanged. The copied prompt now carries the Comments and Drawings sections. Element text keeps the `Role "label" in Role "label"` form. Pictures and video came later the same day (D160).
- **RPCs:** the live client records each call's method, time to response headers and status (`ok`, the gRPC code's snake_case name, `unavailable` for a transport error); never bodies. A status in trailers after a stream starts is not seen. Calls are kept only while recording; replay on fakes ignores them (D47 state compare unchanged).
- **Found on the way:** `look.del` is the pale diff background, so the title-bar "Stop recording" text in it is near-invisible in light mode; it moves to the stroke red.
- **Replays stay faithful (g58):** report commands run from the palette aren't kept in its recent picks, and the recorded state writes the palette's query and cursor only while it is open (it clears its query on every open). The recording rewinds report commands out, so nothing a replay can redo may depend on them.

## D160: Bug reports carry engine-rendered pictures and video (owner, 2026-11-15, supersedes D31's no-pictures rule; format advised by a Fable agent, minion BUG2). Provisional
- **Rendered by the engine, never captured:** pictures come from gpui's offscreen `Window::render_to_image()`, the path the native snapshot tests use, so no Screen Recording permission and no OS capture or encode API is involved.
- **Snapshot:** `window.png` (device pixels) of the window as the view tree was saved. A failed render keeps the text report and says so.
- **Marks:** `marks.svg` sized to the window in points, `window.png` as its bottom layer, strokes as red paths with numbered badges and pinned comments as numbered outlines at their elements; written when there are strokes or comments, removed when neither. The text summary (D159) stays, so an agent can read the report without the pictures.
- **Secrets in pixels:** every view-tree element whose text contains a seeded secret is painted over (its whole box) in the PNG and in every video frame before anything is written.
- **Video format:** APNG `video.png` (the `png` crate the `image` crate already resolves; pure Rust, no build script), RGBA8, fast compression, plays once; still viewers show the first frame. Plus `video.jsonl`, one line per frame (index, ms from the start, changed rect), for agents. Rejected: GIF (256 colours band text), AV1 (slow without an assembler, needs a muxer), H.264 (C). Reverse: swap the encoder behind the frame store.
- **Sampling:** 5 fps at 1x points; a static screen extends the last frame's delay instead of adding one; each later frame is cropped to what changed and blended over. The UI thread only renders and hands the frame to a background thread on a 2-slot channel (a full channel drops the sample).
- **Only changed frames (g58):** a sample is drawn only when the frame's view or layout differs from the last sample's, so a still screen draws nothing; an animation that changes no tree (a caret, a spinner) isn't filmed. `window.png` always shows the end.
- **Cap:** the last 180 s (900 frames) or 48 MiB of stored frames; older deltas fold into a base frame, as recording events fold into the start. Frames are held zstd-compressed and the APNG is written once at stop.
- **Prompt:** names `window.png`, `marks.svg`, `video.png` and `video.jsonl` when present; a folder without them reads as before.
- **New direct crates (main repins):** `png = "0.18.1"` (already in the lock under `image`); `image`, `zstd` are workspace deps already. Note zstd is the C library (per the root Cargo.toml comment), already vendored for chunk; `ruzstd` is the pure-Rust fallback if that is unwanted here.

## D161: The app follows comments through CommentService.WatchComments (minion REVIEW3, 2026-11-15). Provisional
The daemon had no feed of its comments, and the app never polls, so `CommentsClient::watch` needs a stream. `WatchComments(session)` streams every change the daemon saves, in order: a changed comment as it is now, or a dropped id. The daemon already works these out when it saves (the put and removed records of one save), so the feed comes from the same point and can't disagree with what is stored. The first event on every stream is `resync`, so the listing the app makes after it misses nothing. A later `resync` means the stream fell behind its buffer and the app lists again. The call is user only, like the other calls the app makes for the user. This is a reversible manager pick, made under the waves rule.

## D162: The relay's allowlist gains TransferEndService (adds to D139) (minion MACH, asked by minion FILES, 2026-11-15). Provisional
- **Pick:** `humata.v1.TransferEndService` joins the services the host may call whole through a machine's link, beside Terminal, File, Trash and Diff. It is a transfer's far end (FM-8, FM-13): the host's TransferService drives it; on the machine it answers the user and relayed host calls only.
- **Why:** a transfer between machines stages and writes on the far machine through the same proven relay as everything else (D144); no second path.
- **Reverse:** one entry in `WHOLE_SERVICES` and one in `ROUTED` in `daemon/humatad/src/relay.rs` (the routed table leads with the allowlisted services, so the indices after it move together).

## D163: AddRepo and AddMachine on a channel take CreateChannel's path (minion FL, advised by a Fable agent, 2026-11-15; serves D157's follow-up). Provisional
- **Path:** both calls update the host's channel record, clone what this machine lacks, and ask each other machine to make the channel under the host's id with the channel's full repo list (`RemoteWork::create_channel_on`), so a machine rejoins exactly as for CreateChannel (D150). The answer is one Operation (kind ADD_REPO or ADD_MACHINE, target the channel) with CreateChannel's steps: `record`, `clone_main:<owner/name>`, then `record:<machine>` and `clone_main:<machine>:<owner/name>` per other machine; Ready clones are Done at once.
- **Which machines:** AddRepo asks every machine in the channel's record other than the host, online or not; an offline machine's steps show Waiting from the start and run when its link returns (CH-19, flow 26.21 step 2). AddMachine asks only the named machine.
- **Adding what is already there is a retry, not a refusal:** a repo or machine the channel already has runs the same path, so a failed main clone is retried by adding it again; a clone still running is left alone. The repo or machine is listed once.
- **Access:** ChannelChange on the channel. User calls pass; for agents the rule is Never by default (refused at once with the rule named) and goes through the Inbox only once the user sets it to Ask me.
- **Gap, shared with CreateChannel:** ChannelSvc has no machine registry, so a machine the host has never linked shows Waiting until it links. Fix both calls when the registry is reachable from ChannelSvc; check then that RemoveMachine clears such a step.
- **Unchanged:** existing sessions don't change (CH-20); Add repo to this session is AddRepoToSession (CH-21), not served here.

## D166

**The session document tab (minion REVIEW3, 2026-11-15). Provisional; reversible picks, frames 39, 41, 42 and 43.**

- **Which tabs:** a file tab whose path ends in `.md` or `.markdown` is a document tab. Other file tabs keep today's plain source view.
- **Modes:** Preview, Source and Side by side, picked in the tab row's right end while a document tab is selected in that group. Each session remembers the mode per file, across restarts. A new document opens in Preview. Frame 39 has no mode switch; it was drawn before the modes. The switch shows there too, and the owner can say otherwise.
- **Where comments are written:** on source lines only. Clicking a line's number or gutter opens the comment box on that line, and shift-click extends it to a range. The page in Preview and Side by side shows where comments are, by lighting the block and showing a count, but writes none ("Comment on source lines; the page shows where they are").
- **The box:** `You · line N` or `You · lines N–M` with a Draft label, the text, "Saved as a draft; Send types it into <agent> straight away", then Cancel and Comment. Comment sends that comment at once (`CommentsAction::Comment`, D151). Clicking away keeps the draft. "Send N comments" shows in the link bar while there are drafts and no box is open.
- **Link scrolling:** on by default, per file. While on, the page follows the source: the block at the top of the source is brought to the top of the page.
- **Long lines:** the source pane follows Settings › Long lines (Wrap or Scroll, §26.14).
- **Documents outside every clone:** a desk session's own files (`rfc-brin.md`) have no repository. They are anchored with an empty repo, and the daemon takes an anchor that names no repository as the session folder's file when the path is a file there outside every clone. Before this, such an anchor was refused in a session with two clones. The message names just the path: `[comment on rfc-brin.md:7]`.

## D164: A channel names only machines the host has, clones one call at a time, and skips a removed machine's steps (minion FL, asked by main, 2026-11-15; closes D163's gap). Provisional
- **Machine list:** ChannelSvc holds the host's machine list (`machine_service::Work`). CreateChannel (`machines[n]`) and AddMachine (`machine`) refuse a machine the list lacks with NotFound, kind Machine, saying to add it on the Machines page first; nothing is saved. A machine's own daemon has no list and keeps refusing other machines as before. A list that can't be read is that error, not a refusal.
- **AddRepo** asks only the channel's machines the list still has: a machine removed from Humata stays in the record until the channel drops it, and has nothing to clone.
- **Removed while waiting:** when MachineService.RemoveMachine forgets a machine, a channel setup waiting for it ends that machine's part as Gone and its steps that hadn't ended show Skipped. The operation ends on the other machines' outcomes (D150).
- **One clone at a time per channel:** the local main clones of a channel run under a per-channel lock in the daemon. A second CreateChannel-rejoin or add while a clone runs waits for it instead of cloning into the same folder, then sets each of its listed steps to how that clone ended (Done, or Failed with the reason). A clone that failed is retried by the next add (D163).
- **Left:** ChannelService.RemoveMachine (dropping a machine from a channel's record) isn't served yet; the list is read from its file on each check.

## D165: The Files page stays simple; what it needs across machines (minion FILES, owner direction 2026-11-15; main clones advised by a Fable agent). Provisional
- **Owner:** no single tree across machines, no drag between panes. The Files page is two panes with a machine picker each, new folder, rename, Copy, Move, delete to trash and files dropped in from Finder. An OS-level mount is its own milestone.
- **Another machine's pane:** the app asks the host; the host judges the call with the same rule as a local one, then forwards it over the machine's link with the location's machine cleared and none of the caller's headers, and names the machine again in what it returns. A machine whose link is down answers machine-offline. Reverse: the app talks to each machine itself.
- **Trash on another machine:** item ids listed through the host come back as `<machine>:<id>`, so a restore, which carries only an id, finds its machine with no proto change. The host can't see what the item was, so it judges a restore as for an item with no channel and the machine judges again. Reverse: a machine field on the restore request.
- **A new folder with a taken name is refused**, a folder included, so the person is told nothing was made. A retry is safe through its idempotency key. Changed from D142's first cut, where an existing folder came back as success.
- **A pane starts at the machine's home folder.** Humata's own folders sit under it, so they are reachable by walking.
- **A main clone is refused by where the path lands, not by the root it was named through** (advised by a Fable agent, 2026-11-15; was open). The daemon already refuses every caller, the user included, a change named by the main clone root. A path that reaches the same folder through the home folder or the channel folder now gets that same refusal: new folder, rename, delete, Copy or Move into it, and Move out of it, which deletes the source. The clone's own folder counts, not only what is inside. Listing and Copy out stay. The refusal names the repository and says to make the change in a session's clone, or to remove the repository from the channel (CH-22) if the clone itself should go. Nothing is shown before the action: no warning, badge or confirm.
- **Why refuse, not warn or allow:** Humata fetches a main clone on its own and starts every session clone from it (FC-2, FC-3), and nothing makes a Ready main clone again once it is renamed or deleted. Allowed, the damage shows later as a session that won't start, at an action that has nothing to do with the rename. A warning costs a dialog and leaves the same damage. A refusal is one rule in one place and shows only at the action it blocks. Finder and the user's own shell are not Humata's to police and stay as they are. Reverse: allow it through home, and have session start clone a missing main clone again.
- **Left:** nothing for the folders above: renaming, deleting or moving away a folder that holds a main clone (`main/`, the channel's folder, Humata's root) gets the same refusal, through the home folder and through the channel's folder; making something new beside the main clones is still allowed. Built in the daemon's resolver (`files/roots.rs`): a path is followed through symlinks as far as it exists and compared with every channel's main clones. Still open: a symlink that points at a main clone can't itself be renamed or deleted through Humata, since its path lands on the clone; on another machine the check runs on that machine when the call arrives.

## D167: What the login dialog says about why it opened (frame 57) (minion MACH, ruled by a Fable agent, 2026-11-15). Provisional
- **Problem:** frame 57's intro reads "corp-ssh is asking for a verification code, so Humata opened it here. Finish logging in, then press Done." The app printed the terminal's raw prompt line in front of a fixed sentence, which reads badly and can't say the frame's sentence.
- **Pick:** the sentence is "{who} {clause}, so Humata opened it here. Finish logging in, then press Done." `{who}` is the provider's id (`corp-ssh`); for a plain SSH host it is `ssh`, the program Humata ran. The clause follows why the login opened: a code "is asking for a verification code" (the frame, exact); a password "is asking for a password"; a passphrase "is asking for a passphrase"; a yes/no "is asking a yes or no question"; a device login "is asking you to sign in on a web page"; a command that hung "didn't finish connecting on its own"; a provider marked interactive "is set up to log in by hand". Anything else, or the person pressing Open interactively themselves: "Humata opened the login to {machine} here. Finish logging in, then press Done." The sentence is fixed when the terminal opens.
- **Where the kind is decided:** the daemon, whose prompt detector already knows which rule matched; the app never reads the prompt line to guess. Kinds: password, passphrase, code, confirm, device; unset means other. The add's "needs a login" carries either the prompt's kind and line, or that the command hung; "set up to log in by hand" and "the person asked" are the app's own.
- **Texts** stay in the view; the daemon sends data only.
- **Reverse:** show the prompt line in quotes as the clause ("corp-ssh printed "Verification code:", so Humata opened it here. …") and drop the kinds.

## D168: What the Log in step says once the login worked (frame 58) (minion MACH, ruled by a Fable agent, 2026-11-15). Provisional
- **Problem:** "Finished in the terminal; corp-ssh accepted the code" was used for every provider login, which is wrong for a password or a web sign-in.
- **Pick:** "Finished in the terminal; {tail}". The kind is the login's first prompt if one arrived, else the reason the terminal opened (D167). A code: "{who} accepted the code" (the frame, exact); a password: "{who} accepted the password"; a passphrase: "{who} accepted the passphrase"; a web sign-in: "{who} accepted the sign-in"; a yes/no, a hang, a provider set to log in by hand, the person's own choice, or unknown: "{machine} answered". `{who}` as in D167, so a plain SSH host reads "ssh accepted the password".
- **A rule bent on purpose:** "accepted" is inferred from a prompt followed by the machine answering; the app can't see the credential being accepted. Kept because the frame's wording is the target for the code case. If strict wording is wanted, every case becomes "Finished in the terminal; {machine} answered".
- **Reverse:** the one function that builds the tail in `ui/src/overlay_add_machine.rs` (or the reducer, wherever the step's detail is set).

## D169: How humatad2 learns a turn ended (SE-21, SE-22) (minion FL, ruled by a Fable agent, 2026-11-15). Provisional
- **Problem:** a running agent was always WORKING. Nothing heard Claude's turn ends, and nothing applied the output fallback.
- **Hooks:** claude starts and resumes with `--settings <state>/claude-hooks.json`. The file is rewritten at every daemon start and before each launch, with the absolute (canonical) `humata` path and this daemon's socket, and not rewritten when unchanged. Nothing is written into the user's clone. The hooks are all async, and each runs `HUMATA_SOCKET=<socket> <humata> turn <state> >/dev/null 2>&1 || true`, so a CLI error can't block a prompt and no output reaches the context. `humata turn` waits at most 2 s for the daemon and always exits 0:
  - SessionStart (startup|resume): idle
  - UserPromptSubmit, PostToolBatch: working
  - Stop, StopFailure: idle
  - PermissionRequest, Elicitation: needs input
  - PermissionDenied, ElicitationResult: working (a grant has no hook of its own; the next tool batch or output says so)
- **Transport:** a new `SessionService.ReportTurn { TurnState state }`, for the caller's own session only (Caller::Agent). User, script and peer callers are refused. Repeating the current state is a no-op. Each report carries the time its process started; the daemon (and the saved column) keeps the newest per launch and drops an older one, since async hooks can overtake each other. A hook heard while the agent is launching or resuming is kept for up to 60 s and taken up when it runs. It's CLI only, with no MCP tool.
- **Fallback:** watch.rs's attach re-arms a 3 s timer on each output, armed at attach. When it fires, the status is PROBABLY_IDLE; output brings it back to WORKING. Input isn't tracked: the TUI echoes it as output.
- **Status rule:**
  - no hook heard since this launch: the fallback decides;
  - a hook said idle: IDLE; a hook said needs input: NEEDS_INPUT, until output comes back after a quiet spell (the user answered), then it counts as working;
  - a hook said working: WORKING, until 10 s with no output, then PROBABLY_IDLE. Stop doesn't fire on a user interrupt, and a hook sent while the daemon is down is lost.
- **Kept across restarts:** the last hook state is a nullable column on `agents`, cleared at each launch and resume. The quiet timer is in memory only.
- **Events:** every status change publishes a Session event; repeats publish nothing. Shells stay RUNNING_UNKNOWN_TURN. `AgentKind.turn_end` (hooks | output | none, default output) fills `Session.turn_end`; claude is hooks. A `none` kind shows RUNNING_UNKNOWN_TURN and has no timers.
- **Deferred:** Copilot's hook format isn't verified and Copilot isn't in the agents table, so it uses the fallback. `When::TurnEnds` typing (REVIEW, D126) gets nothing yet; no unused notify is kept.
- **Gives up:** sessions started before this change keep their stored argv without `--settings`, so they use the fallback. An older claude that rejects an unknown event drops the whole file and falls back. The 10 s net assumes Claude's spinner keeps printing while it works. A blocking Stop hook or `/goal` continues the turn past our idle until the next UserPromptSubmit or PostToolBatch says working. A watch that loses its holder keeps its entry, so a holder that comes back (Reattached) can be watched twice for a moment.
- **Reverse:** drop `--settings` from claude's args, the two `agents` columns, ReportTurn and `humata turn`; `session_status` goes back to WORKING. The store's schema version was bumped, so an older build refuses a store this one has opened; going back means restoring a store from before it.

## D170: Search over this host's conversations and pins on the host: where they live (minion FIND, 2026-11-15; serves LD-8, LD-21–LD-25, NV-20–NV-26, D62, D63). Provisional
- **Mirror on this host:** new crate `daemon/mirror` reads each local session's agent conversation file (the `conversation_file` of its agent kind) from the byte offset it stopped at, drops calls to Humata's own tools and their results (`mcp__humata…`, Bash `humata …`), scrubs host variable values to `‹NAME›`, and appends to `<state>/mirror/<session>/`. It catches a session up when that session's Session event is published (turn ends, stops, resumes) and once per session at daemon start: it follows events, never scans on a timer. Other machines' mirrors come over MACH's link later into the same store; FORK's moves and rebuilds read it.
- **Index:** SQLite FTS5 in `<state>/search/index.db`, with a format version in `user_version`; missing, damaged or another version means it is rebuilt from the mirror files. User data never lives only in the index (D62.5). One hit per (session, turn); every word must match, the last also as a prefix; query characters are literal.
- **API:** a new `HistoryService { SearchSessions, ReadTurns }` in its own `history.proto`, so SessionService's code is untouched; MCP `sessions_search`/`session_read` and `humata sessions search`/`humata session read` call it. Until the SESSIONS_SEARCH_OTHER capability lands, an agent searches only its own channel and asking for another channel is refused, not Asked.
- **Pins and VIP:** new crate `daemon/marks`, its own SQLite file `<state>/marks.db` (no store migration). MarkService is user-only and publishes TOPIC_MARKS; deleting or forgetting a thing clears its marks (idempotent).
- **App:** `History` and `MarksClient` traits with fakes in `clients`, reducers in a new `features/find`; the palette's "Conversations" section and the sidebar's Pinned rows are view files LOOK2 places (frame 30 has no conversation hits; drawn like its Sessions rows, the snippet in place of the status).
- **Reverse:** move the mirror into the store's database (one file, but couples the index to store migrations), or put the two RPCs on SessionService; marks into the store with a migration.

## D171: The app icon is a fork of sessions (minion FL, advised by a Fable agent, 2026-11-15). Provisional
- **Pick:** candidate C from `app/icon/draw_icon.py`: on a dark squircle, a blue root node with a cursor bar in it branches to a green node and an orange node. The node colours are the app's status colours. A, a prompt chevron, read as a generic terminal. B, rows with status dots, read as a to-do app.
- **Gives up:** a three-node branch may still remind people of git clients. B is the fallback if that's how it reads.
- **Reverse:** set `CANDIDATE` in `app/icon/BUILD.bazel`.

## D172: Undo after Archive is a Restore before the clones go; a fork's parent is shown, never rewritten (minion FORK, 2026-11-15; serves MV-7, MV-15, LD-2, LD-4, FK-13, D110). Provisional
- **Pick:** ArchiveSession records the archive first, then stops the agent, writes the diff summary and frees the clones once the 10-second undo window closes; the daemon keeps the timer, and a restart in between frees them at once. RestoreSession before the free is the Undo: the row goes and the agent resumes in place, uncommitted work included. After the free, Restore clones the branch from the remote, or a new branch from main when it was never pushed, and resumes the conversation. There is no separate Undo call (GRPC.md §7: "restore undoes").
- **Fork tree:** the store keeps each fork's true origin and never rewrites it. Session.fork_origin.parent and a parent's forks show the nearest ancestor that isn't archived, so FK-13's "forks move up" and Restore putting them back need no bookkeeping.
- **Archive choices:** with uncommitted or unpushed work, plain ARCHIVE is refused (failed precondition naming the work) and ARCHIVE_AND_DELETE_WORK needs the plan's token.
- **Tests:** admin builds read `HUMATA2_ARCHIVE_UNDO_MS` for the window; release builds always wait 10 seconds.
- **Gives up:** "delete at once when the app quits" (MV-15) isn't done by the daemon: the app has no quit hook to the daemon yet, so the clones go after the window or at the next restart.
- **Reverse:** add an explicit UndoArchive RPC; or rewrite fork origins on archive and keep a moved-from list for Restore.

## D173: The Desk frames follow one workspace: the board's lanes, the rail's badges, the shell's cursor, the closed sessions (minion DESK, ruled by the owner's design advisor, 2026-11-15). Decided
- **Problem:** the native captures of frames 36, 37, 38, 40 and 55 differ from the goldens where the frames' made-up data disagrees with the one workspace the app draws from. The owner delegated golden picks to a Fable advisor; these are its four rulings.
- **Principle:** the Desk frames are the design workspace plus what the Desk sessions did to it (two items added, one request asked). The design workspace doesn't move, so frames 21–27 stay. Closed Desk sessions were closed in the past, so they are in every Desk frame.
- **1. The Board panel (frame 37)** lists every board's items in the lane order of every other session's Board panel (L14): In progress, In review, To do, Backlog, Done, empty lanes left out; the session's additions first in their lane; Done faded. One shared order, `ItemStatus::COMPACT`, read by both panels. The frame's panel is redrawn from the workspace's items and clips at the window's edge; its terminal line reads "6 items · 1 in progress · 1 in review".
- **2. The rail's badges:** frame 37 keeps Inbox 1, backed by one waiting request from Plan Q4 board ("Create channel web-revamp"; the session keeps working, D158 item 7); its Diff 2 goes. Frames 38, 39 and 41–43 draw no Inbox badge; 41 no Diff badge; 42 and 43 draw whatever the app counts for their comments. The sidebar's Inbox count reads 8 on the Desk frames (36–43, 55). Not taken: answering another channel's request in the fixture to hold 7.
- **3. The shell's cursor (frame 38):** the frame gains a block cursor after the prompt; a live shell in the front tab shows one, and the fixture stays.
- **4. The closed sessions (frames 36, 40, 55):** the Desk has four closed sessions in every frame, so the page behind Clean up… lists the four the dialog lists. `fixture::desk` holds all four and `fixture::clean_up` goes. A closed row says what becomes of its folder: "deleted in N days"; "scratch folder · kept"; a picked folder's path; "not deleted · <the unpushed work>". Rows without Keep leave its space empty so "last used" lines up. The dialog stays as drawn.
- **Left as approved, flagged by the advisor:** Clean up… pre-ticks a kept scratch folder (D141's tick rule; Keep stops the timer, Clean up is the manual delete, the trash holds it 30 days).
- **Frames redrawn in the drawing code (main records):** 36–43 and 55 (the sidebar's 8); 36, 40, 55 (four Closed rows); 37 (panel, terminal line, rail); 38 (cursor, rail); 39, 41–43 (rail).
- **Drawn and recorded (2026-11-15):** all four rulings are in the drawing code; goldens 36-43 and 55 were recorded by minion DESK on the advisor's approval.
- **Reverse:** each ruling stands alone: point `DeskState::board` back at `ItemStatus::ALL`; delete the request and pass the old literals; hide the cursor in the fixture; move the three closed sessions back under a `clean_up` fixture.

## D174: An approved agent skills sync is refused as a whole when one machine needs a plan (minion MACH, agreed by the main agent, 2026-11-15). Provisional
- **Behaviour kept:** SyncSkills for several machines is refused, naming the machine, when any of them is on its first sync or has a new name clash and the call carries no plan token for it (SK-5, SK-11). This holds for an agent's call the user approved in the Inbox too: the approval lets the call through the access rule, it doesn't stand in for the user reviewing that machine's plan, and an agent can't preview. Nothing syncs, the other machines included.
- **Why:** one rule for the user and for an approved agent; a partial sync would need its own way of saying which machines were left.
- **Reverse:** skip the machines that need asking and sync the rest, naming the skipped ones in the operation (`SkillsSvc::sync` in `daemon/humatad/src/skills_service.rs`).

## D175: A plain SSH add that needs a login is an auth failure, with a pointer to providers (minion MACH, agreed by the main agent, 2026-11-15). Provisional
- **Behaviour kept:** adding a plain SSH host whose connect asks for a password, passphrase or code ends as an SSH auth failure (the CLI's exit code for it is unchanged), not as "needs a login"; the interactive login opens only for a provider's machine (MP-12–MP-17).
- **Added:** the failure's fix now also says that a machine needing a login each time is added through its provider, which can open the login.
- **Why:** a plain host is expected to work with keys; changing the error's kind would change what scripts branch on.
- **Reverse:** map `ManageError::LoginNeeded` to `LoginNeedsUser` in `daemon/humatad/src/machine_service.rs` (`error_of`) and let `LoginOpen.operation` find an SSH add (its undo unpins the host key today, so the login would need the key kept).

## D176: The Add machine login step wraps its text; frame 57 draws it past the dialog (minion MACH, ruled by a Fable advisor, 2026-11-15). Decided
- **Problem:** frame 57 (`desktop/tests/__snapshots__/design/57-add-machine-login.png`) draws two lines, the reason the terminal opened and the "Reconnecting will need you" notice, running past the dialog's right edge. That is a mistake in how the frame is drawn, not a look to copy.
- **Pick:** the app wraps both lines inside the dialog (`ui/src/overlay_add_machine.rs`). The frame is left as it is until the owner OKs redrawing it (D76).
- **Also differs from the frames on purpose:** the login terminal is a real terminal with a fixed height, where the frame's box is as tall as its text; the machine page's disk bar fills to the used share (the frame under-draws it); a variable's chip carries Set value and Remove, and New variable sits under them (HO-7), which frame 59 doesn't draw.
- **Recommendation:** redraw frames 57 and 59 from the app when the owner next reviews frames.
- **Ruling (Fable advisor):** all four picks stand: the wrap (a drawing error in frame 57), the fixed-height terminal (a live terminal can't shrink to its text and the dialog mustn't jump), the disk bar agreeing with its numbers, and the variable chip's Set value and Remove (HO-7). Frames 57 and 59 join D113's list until the owner OKs the redraw (D76).
- **Reverse:** drop the wrap on the two text boxes.

## D177: The item drawer's list edits, ask wording and stale writes (minion BOARD, advised by a Fable agent, 2026-11-15). Provisional
- **List edits without changing frame 22:** the Sessions and Depends on rows get a trailing "+" ("Add session", "Add dependency"); each session line and dependency chip gets an "×" ("Remove session <name>", "Remove dependency <ID>"). They are always in the accessibility tree and the tab order, painted only on hover or keyboard focus, and take no layout space at rest. The old "link" picking mode goes once the "+" picker lands.
- **Picker:** one popover under the row, a filter field over a list; arrows, Enter adds, Esc closes; stays open for more adds. Sessions: the item's channel's sessions (a Global item: all, grouped by channel). Dependencies: any item as "ID Title", leaving out itself, existing edges and anything that would close a loop.
- **Ask wording (BD-15):** a move is frame 22's "<Agent> asks to move this to X" with Move to X / Keep in Y; an edit is "<Agent> asks to edit this" with Apply edit / Keep as is and one "Field: before → after" line per change under the quoted reason; a delete is "<Agent> asks to delete this" with Delete item / Keep item, the first in the destructive style. Open session always.
- **SL-7 in the app:** drags send no revision (D155). The list edits send none either, since lists never conflict (D155); a refusal refreshes the item and leaves the picker open to redo. No title or description editor in the drawer this milestone (the golden draws none): deferred to the owner's next wave.
- **History** is read when a drawer opens (one call), not with every item at load.
- **Reverse:** a visible "Edit" button per row; the editor when the owner OKs a frame for it.

## D178: The live Inbox client, bounded waits, the tab's socket, the card's operation id and the session panel's card (minion ASK, batch 5, 2026-11-15). Provisional
- **Live client (`clients/live/src/inbox.rs`):** the IN-22 grant target each pending request offers is kept by request id in the client's state, so Always allow sends that exact target (the app's model carries only its words). Answers carry the revision last seen; a stale one is refused by the daemon and shown as such. Rules and grant events re-read that channel's rules.
- **`--wait N` (CLI.md §6):** a call the daemon answers `pending` at once (RequestAccess, AskUser, a deferred call) waits up to N seconds on WaitForAnswer, then exits 12 with the request still waiting. `--wait 0` never waits; with no `--wait`, a terminal waits until answered and `--json` not at all (`cli/src/call.rs` `Ctx::wait`).
- **HUMATA_SOCKET:** a session tab's shell gets the daemon's socket (made absolute) beside its session token, on open and when a restart brings the tab back, so `humata` in the tab reaches the daemon that started it even off the default socket. Machine shells get neither. (`identity::tab_env`)
- **Operation id (TP-16, IN-9.2):** `InboxRequest.operation` (field 21) is named by the daemon from the gRPC method it routed, never from anything the agent sends: the service's area, then the method in snake case without the area's word (`ChannelService/ListChannels` → `channel.list`, `InboxService/RequestAccess` → `inbox.request_access`). Most match the CLI's and MCP's ids; some don't (the CLI calls an ask-ahead `inbox.request`).
- **Session Inbox panel:** draws the shared IN-9 card (`request_card::card`, narrow) for the picked request (or the first). The others fold to a title, age and who line that select on click, as design golden 03 draws. The card's who line names the daemon's operation id, or else the rule's tool name. Its tick is a CheckBox. (Revised the same day: drawing every card open pushed the second request's buttons out of view.)
- **Reverse:** operation id: a client-sent header, or a table in the daemon from method to the MCP id; socket: drop the second pair in `tab_env`; panel: bring back `panel_inbox::request_card`.

## D179: Live diffs are fetched per session when a review surface needs them, not in load (minion REVIEW, batch 11, ruled by a Fable advisor, 2026-11-15). Provisional
- **Pick:** `Daemon::diffs(session, base)` and `Daemon::mark_reviewed(session)` on the client trait. The Review reducer fetches the open session's diffs when its page opens, again when its turn ends (it leaves Working) while it is open, and after the user changes the base (RV-1) or marks it reviewed (RV-6). `load()` stays cheap: the live client reads no diffs in it. One fetch per session in flight; a later trigger queues one more; the old diff stays on screen until the new one lands. TR-12 already says working-tree changes are read when needed, not watched.
- **Failure:** shown only in that session's Diff panel body, never as a toast, mark or Inbox item, and never as "No changes yet". If nothing was fetched before, the panel names the error with Try again. If an earlier diff exists, the panel keeps it under one line, "Couldn't refresh. Showing the diff from <time>", with Try again. While the session is still being set up, the panel shows a quiet line saying its clone is being made.
- **Known limit:** the live client reads every file's hunks with the list. Fable's advice is the list first and hunks for the files shown; that is left for when big diffs are benched.
- **Reverse:** fill `workspace.diffs` in `LiveDaemon::load()` from `session_diffs` and drop the trait methods.

## D180: Agent access gets one row for changing the Global board and other channels' boards, Ask me by default (asked by main, ruled by the owner's advisor, 2026-11-15; closes D155's open frame question). Decided (owner's advisor)
- **Problem:** D155 brought back `CAPABILITY_GLOBAL_BOARD_WRITE` and left its row in the Agent access list waiting for the owner's OK, since the list is a design golden (D76). The row was drawn anyway in freeze 1: frame 49 (`desktop/tests/design/channel_tabs.rs` and its PNG), SPEC IN-17 and `clients/src/fixture/rules.rs` all carry it. It needs a ruling on whether it stays, what it says, its default and its place.
- **Pick:** it gets a row, and the row as drawn stands.
  - **Label:** "Global board, or other channels' boards: change". It pairs with the row above it, "…: read". "Change" means add, edit, move and delete, whoever made the item, as the daemon checks it (`board_need` in `daemon/access/src/policy.rs`). A denial names the rule as "change the global board or other channels' boards" (`Capability::label`).
  - **Description:** none. No row in the list has one, and the pair of rows says it. The tab's header line already explains Allow, Ask me and Never.
  - **Default:** Ask me. The power includes deleting other people's items on boards outside the agent's channel, so nothing happens until the person approves; the sibling "Channel board: edit, move or delete anyone else's items" is Ask me for the same reason. All three settings are offered. An approval may be made standing for one board (Always allow, IN-22), shown as a grant line under the row like the one frame 49 draws under "Read another session's files".
  - **Place:** eighth row, straight after "Global board, or other channels and their boards: read" and before "Write into shared/ on its machine". The board rows stay together, own board to channel board to everything else, with read before change. The list is grouped by subject, not by default, as the channel board's Ask me row already shows.
  - **Settings › Agent access (ST-4):** the same row, same place, same default, since it is the same list. Frame 29 draws only the nav entry, so it doesn't change.
  - **Desk (DK-4, DK-10):** the "Read and change every channel's board" switch allows both rows, read and change. DK-10 still says "the row"; it should say the two rows.
- **Frames:** nothing to redraw. Frame 49 already shows the row with Ask me selected; this ruling is the OK D155 was waiting for. If Settings › Agent access is ever drawn as its own frame, it draws this list unchanged.
- **Gives up:** the verbs aren't spelled out as they are in the channel board rows, so a reader has to take "change" to include add and delete. A Never default for the delete alone isn't possible while one capability covers all four; a channel that wants that sets the row to Never.
- **Reverse:** reword to "…: add, edit, move or delete items" in SPEC IN-17, the frame, the fixture and `Capability::label`, and re-record frame 49; or split delete into its own capability with its own row, Never by default.

## D181: Context-only and before-turn forks (advised by a Fable agent, 2026-11-15). Settled

**Part 1 superseded by D185** (context-only forks share and edit the parent's code); parts 2 and 3 stand, GiveOwnCopy included.

1. **Context-only fork (FK-7, IN-20).** The fork's cwd is its own session folder, created empty and with no clone records. Its branch is picked at fork time and reused by GiveOwnCopy. Humata's own calls refuse writes to the parent's clones through `SessionKind::ContextOnlyFork` (`Refusal::ParentReadOnly`), which is now wired from the fork origin, not always `Ordinary`. Agent file tools hit a blocking PreToolUse hook on `Write|Edit|MultiEdit|NotebookEdit`. It canonicalises `file_path` and exits 2 with "ask for your own copy" when the path is under `HUMATA_READ_ONLY_CLONES`, which the launch env sets. The check is local, so it can't fail open with the daemon down. **Bash is not covered**: blocking it needs containment, which is an owner decision if wanted. chmod, read-only mounts and seatbelt were rejected. Claude gets `--add-dir <parent clone>` to read without prompts.
2. **Turns (FK-3).** ListTurns parses the agent's conversation file with the mirror's rule (a `user` line with text, not meta, not sidechain); one `humata2_mirror` function gives index, first line, time and offset. A before-turn fork copies the byte prefix up to turn n. Only agent kinds that name a conversation format (`claude-jsonl`) can fork at a turn; others list no turns and refuse. `before_turn = 1` and values past the last turn are refused. The code is not rolled back (FK-2: "as they are right now"); the preview and the FK-10 line say so. `before_turn` is stored in ForkOrigin.
3. **Order.** A seam first (`before_turn` on ForkRequest, `ForkCopy::ContextOnly`); then turns and context-only; GiveOwnCopy last, gated `OwnSessionManage`, copying through `.partial`, with no relaunch and the agent pid kept. FK-6 (another agent), FK-8 and the UI come later. The proto copies all repos for GiveOwnCopy; the `<repo>` argument in CLI.md and MCP.md is the doc mismatch to fix.
- **Reverse:** each part stands alone. Containment for Bash would replace the hook; a per-turn code snapshot would add rollback.

## D183: Mark reviewed sits in the Diff panel's base menu, and a failed refresh names no time (minion REVIEW, batch 11, 2026-11-15). Provisional
- **Pick:** the "against <base>" chip opens a menu: origin/main, Uncommitted only, Since I last reviewed, Since the fork from its parent (forks only), the branches of the channel's other sessions, then **Mark reviewed** (RV-1, RV-6). Frames 01, 39 and 41–43 draw only the chip, so the panel adds no button they don't have. A failed refresh says "Couldn't refresh. Showing the last diff read." with Try again. D179 asked for the read's time, but the reducer's clock counts from launch and has no time of day.
- **Gives up:** Mark reviewed is one click further than the prototype's footer button, and the refresh line doesn't say how old the diff is.
- **Reverse:** move Mark reviewed to the footer beside Send comments and re-record the frames that draw the footer; give the Review reducer a wall clock and say "from <HH:MM>".

## D182: Daemon Desk sessions: record shape, DeskService, Desk access kind, close and the 7-day delete (asked by minion DESK, ruled by a Fable advisor, 2026-11-15). Provisional
- **Problem:** the app's Desk (DK-1–DK-14) runs only on the fake daemon; humatad serves none of it. Seven picks were asked: record shape, API, access, close and the 7 days, the scratch name, the CLI, the workspace. Four facts in the code shape the answers: the policy compares channels by id and loads rules and grants by channel; ArchiveSession frees the session folder after its undo window; DeleteSession is not built; no session records "last used" yet.
- **Pick:**
  1. **Record: agreed, with changes.** A Desk session is a `SessionRecord` with channel `desk` and `folder` set to its path, plus one `desk_sessions` row. Why: tabs, resume, conversation, pins, Inbox and restart recovery are reused as they are.
     - Drop `name` from the row: it is the last part of `folder`, and one source is enough.
     - Add `deleted_unix_ms` (a tombstone) and `held_work` (the work that made the 7-day delete skip it).
     - Closed lives only in this row. Never write an `ArchiveRow` for a Desk session: `finish_archives_after` and `free_folder` would free its folder.
     - Reserve `desk` in `names::channel_folder` and as a channel id. StartDeskSession refuses when a channel already owns `<root>/desk`.
  2. **API: a new `humata.v1.DeskService`, agreed, but declare only what batch 1 serves.** Why: Close must not go through ArchiveSession, and a Desk start has no channel, branch or clones to thread through `start_fresh`.
     - Eight calls: StartDeskSession, CloseDeskSession, ReopenDeskSession, KeepDeskFolder, SetDeskAccess, ListDeskSessions, PlanDeskCleanUp, CleanUpDesk.
     - AddDeskRepo, RemoveDeskRepo, MoveDeskSession and CopyDeskDocument are added to the proto when they are built. Until then the live client keeps answering "not in this release" itself.
     - Start takes an idempotency key, so a retry never makes `plan-q4-2`. It answers the session and the folder's final path.
     - List sends facts, not labels: kind, kept, the two switches, `last_used_unix_ms`, `closed_unix_ms`, `delete_at_unix_ms` (unset when kept, picked or held) and `held_work`. It runs no git.
     - The clean-up token is a digest of the closed sessions listed and the work each holds; sizes are not in it. CleanUpDesk recomputes it, and on a mismatch deletes nothing and answers the new list (NV-27). There is no plan map and no expiry.
     - Every call is user-only (app, or CLI with presence) and refuses peers. The one exception is Keep on the caller's own session (pick 6). No call asks through the Inbox in batch 1.
  3. **Access: `SessionKind::Desk { boards, channels_and_machines }`, agreed; no pseudo-channel per session.**
     - **Never wins.** DK-10 calls the switches grants, and the policy's order is Never, then grants, then the rule (IN-22). A switch that is on by default must not undo an explicit Never. The refusal names the new-channel default as the rule to change.
     - A switch that is on allows, after the Never check: `boards` covers GlobalRead and GlobalBoardWrite; `channels_and_machines` covers ChannelCreate and MachineProvision, never MachineRemove. It gets a `Source` of its own so the record says why.
     - A switch that is off adds nothing: the new-channel default decides, and it does not force Ask.
     - One private lookup in the policy maps the channel id `desk` to the new-channel rules. `check_agent`, `check_ask_ahead`, `approve`, `replay_grant`, `from_state` and `is_suspended` all use it. Without it a stored Desk grant stops the policy loading after a restart; test that it loads.
     - For a Desk agent no channel is its own. Every `== me.channel` test in `policy.rs` goes through one helper that is false for `SessionKind::Desk`, or two Desk sessions become channel-mates (ChannelRead, FilesReadOtherSession, notes, stop). Test with two Desk sessions.
     - A Desk session has no board. Its scratch folder is its own place (`Root::Clone`, IN-16).
     - Only the user calls SetDeskAccess (IN-23). It applies from the agent's next call (IN-21).
     - Close ends the session's standing grants, as Archive does.
  4. **Close and the 7 days: agreed, with changes.**
     - Close takes the session's lock, stops the agent as `stop_agent` does, shuts every other tab's holder, sets `closed_unix_ms` and leaves the folder.
     - Reopen clears closed and resumes through `run_resume`. A Terminal session gets a new shell. It refuses when the folder is gone.
     - While a Desk session is closed, ResumeSession and new tabs refuse and name Reopen.
     - SessionService's Archive, Restore, Fork, PreviewFork, GiveOwnCopy, Push and Move refuse a Desk session and name the Desk action.
     - Expiry is a wall-clock sweep, not one sleep to the deadline: a monotonic sleep may not count the time the Mac is asleep. It runs at start, after each Close and hourly.
     - Under the session's lock it re-reads the row and acts only on a closed, unkept, scratch, undeleted session past `closed + 7d`.
     - The work check looks at the folder and the git folders directly under it. Work found is written to `held_work`, the session is skipped and it is checked again next sweep (LD-17).
     - Delete, for expiry and Clean up alike: the scratch folder goes to the machine's Bin (30 days, LD-7), and the row gets `deleted_unix_ms`, which hides the session from every list.
     - The delete removes only a direct child of `<root>/desk`, after canonicalising and refusing symlinks, whatever the row says. It has the same tests as `remove_session_folder`.
     - The test hook `HUMATA2_DESK_EXPIRE_MS` is agreed, admin builds only.
  5. **Scratch name: the client's name is not sent at all.** The daemon derives it from the title with `names::branch_slug` (FC-13, falling back to `session`) and has the last word (D148). Why: a field the daemon ignores misleads, and one slug function can't drift from the branch rule.
     - `-2`, `-3` are added when the folder exists on disk or any desk row, tombstoned ones included, holds the name.
     - The folder is claimed with a single `create_dir` at 0700; "already exists" means try the next suffix.
     - A picked folder must be absolute, exist, be a directory and, once canonicalised, sit outside the whole Humata root, not only outside the channel trees.
     - Humata never deletes, trashes or moves a picked folder. Clean up tombstones the session and leaves the folder.
  6. **CLI: `humata desk keep [<session>]`, agreed, but not behind OwnSessionManage.** Why: an Ask would make the person approve a command they typed, and a keep only stops a delete, so it widens nothing.
     - The user may keep any Desk session from the app or the CLI with presence; outside a session the argument is required.
     - Inside a Desk session's tree (agent or person, the daemon can't tell them apart) the caller may keep that session's own scratch folder with no rule and no ask (IN-16).
     - Any other session's agent is refused. Nobody can un-keep; Clean up is the user's delete.
  7. **Workspace: agreed.** The live client fills `Workspace.desk` from ListDeskSessions and takes Desk sessions from ListSessions, which hides tombstoned ones.
     - Days left are derived from `delete_at_unix_ms`, not from a 7 in the client.
     - Every desk change sends the session's Updated event.
     - `documents` and `clone_branches` stay empty in batch 1; sizes come only from PlanDeskCleanUp.
  8. **Batch 1 serves:**
     - the eight calls, on the Mac's own machine;
     - scratch or picked folders, running an agent or Terminal; a start with repos or on another machine is refused naming that part, and starts nothing;
     - the sweep, the Desk kind and the two policy helpers, the reserved name, and the SessionService refusals;
     - `humata desk keep`;
     - the live `DeskClient` for start, close, reopen, keep, set_access, clean_up_list and clean_up;
     - `last_used_unix_ms`, stamped by Start, Reopen, Close, Keep and SetDeskAccess.
- **Gives up:**
  - In batch 1 the live Desk has no repos, Move, Documents or Copy to a channel, and no Desk sessions on other machines.
  - A deleted session's folder can be restored from the trash as a folder, but the session can't be brought back until DeleteSession and its restore are built.
  - Its record and conversation file stay on disk, hidden, until then.
  - "Last used" does not move on typing or opening until SE-26 has a home for every session.
  - An agent can't close or start Desk sessions, even with an ask.
  - An agent can make its own scratch folder outlive the 7 days; the Closed row says "kept" and Clean up pre-ticks it (D173).
  - A boards switch that is on does nothing where the new-channel default is Never, and the Info panel doesn't say so until the agent is refused.
  - A picked folder is not reachable through Humata's own file calls for the agent.
- **Reverse:**
  - Drop the `desk_sessions` table and the service; nothing else stores Desk state.
  - To let a switch beat Never, move its check above the Never check in `check_agent`.
  - To gate keep, send the own-session case through OwnSessionManage.
  - A name field in the dialog becomes one optional field on StartDeskSession.
  - The tombstone becomes a real delete when DeleteSession lands; restore clears the flag.
  - The four later calls are additive in the proto.

## D184: A context-only fork's shell commands are not blocked; the user is told (advised by a Fable agent, 2026-11-15). Settled

**Superseded by D185**: there is nothing to warn about.

SEC-6 containment isn't built (`contained: false`), so a context-only fork's Bash tool can still write the parent's clones. Accept that as SEC-5 already does for every uncontained session, and say so where the user decides: an uncontained context-only fork carries a shell-write warning flag in the fork dialog's state and its Info panel's state. Ordinary sessions, code + context forks, contained context-only forks and a fork after **Give it its own copy** carry none. A Bash PreToolUse check was rejected: it can't tell reads from writes and is easy to walk past. Holding the milestone for containment was rejected too. SPEC FK-7, SEC-3 and SEC-6 are amended to match. When SEC-6 lands, a contained context-only fork's shell reads of the parent's clones succeed and its writes fail.
- **Reverse:** build SEC-6 containment for context-only forks first, and drop the warning.

## D185: A context-only fork works in its parent's code and may edit it (owner, 2026-11-15). Decided (owner)
- **Owner:** "convo only copy being able to edit code is intended cause what if i wanna run 2 agents on the same source code is the idea, code + convo is what if i want my own code and my own convo, neither is what if i want a fresh agent session AT this source code though tbh that's just convo only copy + a `/clear` basically".
- **Pick:** the kinds are defined by what is copied. **Code + context** copies both: own clones on a new branch, and the conversation. **Context only** copies the conversation and shares the code: the fork's cwd is the parent's clones and it reads and writes them like the parent, so two agents can work on one tree. **Code only** stays: own clones and a fresh agent. A fresh agent in the parent's own code gets no separate kind; it is a context-only fork followed by `/clear` (reversible pick by main).
- **Supersedes:** q3, the read-only rule in FK-2, FK-7 and IN-20, D181 part 1 (own empty folder, `Refusal::ParentReadOnly`, the PreToolUse write block), and D184 (the shell-write warning; there is nothing to warn about). Give it its own copy (FK-9) stays: it moves a context-only fork onto its own clones.
- **Gives up:** nothing stops two agents in one tree from editing the same file at once; the diff and Changes panels show both agents' edits together.
- **Reverse:** bring back FK-7's write refusal for context-only forks.

## D186: A search hit opens a read-only Conversation tab on the matching entry (minion FIND, ruled by a Fable agent, 2026-11-15). Provisional
- **Problem:** LD-24 says a hit "opens that turn", but sessions are terminals: there is no turn-to-row mapping, scrollback is bounded, and the machine may be offline (LD-23).
- **Pick:** picking a hit navigates to the session (Back still works) and shows one `Conversation` tab in the focused tab group, read from the host's mirror. It is plain, selectable text grouped by turn, with roles You, the agent's name and Tool; tool text is monospace and there is no composer. It lands on the hit's entry (`seq`, carried on `SearchHit`), scrolled into view and marked. The first read is the hit's turn ±2; **Show earlier turns** and **Show later turns** each add 5. A failed read shows Retry. One tab per session: a new hit re-targets it. It has ✕, and it isn't in the + menu yet. The target is saved in `SessionView`; entries are re-read on restore. No golden changes. It is the same surface LD-3 calls "its conversation" for archived sessions.
- **Not taken:** scrolling the terminal (no mapping; fails offline); a side panel or popover (changes every session golden, lost on Back); Markdown rendering and query highlighting; live-following.
- **Reverse:** remove `Tab::Conversation`, its state, actions and view. A saved UI holding the tab must then load as no tab (tolerant deserialiser).
- **Docs to follow:** SE-8, LD-24, LD-3 wording.

## D187: How a context-only fork works in its parent's code, and how Give it its own copy moves it (FORK manager, 2026-11-15). Settled (reversible)
1. A context-only fork launches in its parent's session folder, so its cwd holds the parent's clones. Its `repos` list the parent's clones (same path and branch, not main clones). Its conversation is a copy under its own id. Humata's access treats the parent's clones as the fork's own (IN-16), and there is no refusal.
2. **Give it its own copy** copies the parent's clones as they are now into the fork's own folder on a new branch, moves the conversation file to the path the agent keys on for the new folder (as FK-12 does), then resumes the agent there. The agent's conversation is kept, but its process is new: the cwd can't change under a running agent.
- **Reverse:** keep the agent running in the parent's tree and only tell it the new paths.

## D188: The Fork dialog's Context only card text after D185 (FORK manager; picked by a Fable 5.2 high subagent; re-record OK'd by main under D185, 2026-11-15). Settled (reversible)
- The card reads "An agent that knows the conversation and works in this session's files, with no copies, so its edits land here." Its title stays "Context only".
- Why: it has the same shape as the sibling cards, contrasts with their "New copies", and ends on the consequence.
- Golden 62-fork-dialog was re-recorded.

## D189: The host reads other machines' conversations from its saved offset; the relay allowlist gains HistoryService/ReadMirror (amends D139) (minion FIND, ruled by a Fable agent, 2026-11-15). Provisional
- **Problem:** the host mirrors only its own sessions (D170), so search (LD-8, LD-23) finds nothing an agent said on another machine. peer.proto has a push path for this (`MirrorChunk`, `UplinkAck.mirror`, `UplinkResume`), but nothing builds it. The link process keeps no store (D139), so it can't find or read a conversation file. A push would also need send offsets and resend windows on the machine beside the offset the host already saves.
- **Pick: the host pulls, as designed, with the changes below.** One new unary call, `HistoryService/ReadMirror`, goes through the relay. The machine's socket daemon reads complete lines of the session's conversation file from the offset the host gives, scrubs them (LD-21) and answers. The host's mirror keeps the one offset, in the session's `state.json`. The push messages in peer.proto stay unbuilt.
- **Amends D139:** the relay allowlist grows by one method, `HistoryService/ReadMirror`. Why: the conversation file and the host variable values are both on the machine and known only to its socket daemon, and the relay is the only way the host reaches that daemon. `SearchSessions` and `ReadTurns` stay off the list. D144's rules are unchanged, and its list of what a same-user process holding the key can reach gains this call. That process can already read the conversation files as they are, so it gains nothing.
- **Why this fits the spec:** the read is keyed by an offset and resumes from it over the machine's existing link (LD-8, TR-4). Nothing polls (TR-7, TR-9): the host reads when a machine's link comes up and when the machine's pushed session report arrives. Values are replaced on the machine before the bytes reach the link process (LD-21, SEC-1, HO-13).
- **Changes to the design (the implementer follows these):**
  1. **Caller.** The handler's first step refuses every caller but `Caller::User(UserVia::Host)` with permission denied: the app, a CLI login, an agent, a script and `Caller::Peer` are all refused. A test covers each. The relay's allowlist test names `SearchSessions` and `ReadTurns` as refused. `HistoryService` goes at `ROUTED[9]`, after EventService, and the off-list refusal's text names the new call.
  2. **No path from the request.** The only file opened is the `conversation_file` of the session's agent row in the machine's own store. `source` is compared for equality and never read as a path.
  3. **Scrubbing fails closed.** The handler gets its `Scrubber` from one function that returns a `Result`. When the machine can't read its host variable values, the call fails and sends nothing; it never falls back to an empty scrubber. Until SetHostVariable is built that function returns an empty one.
  4. **What is scrubbed.** Every string, every object key, and every number whose text holds a value (the number becomes the scrubbed string). A line that isn't JSON is left out and the offset moves past it.
  5. **A read always moves forward.** The machine treats `max_bytes` of 0, or more than 1 MiB, as 1 MiB, and `data` is at most 1 MiB before encoding (TR-11). When a complete line follows `offset`, the read takes at least that line, however long it is. A line whose scrubbed form is over 1 MiB has each string longer than 64 KiB cut to its first 64 KiB and a note of the bytes left out. If it is still over 1 MiB it is left out, the offset moves past it, and the machine logs the session and the line's size. So `more` always comes with a `next_offset` past `offset`. The host stops and logs if an answer says `more` without moving.
  6. **Compression (TR-11, TR-13).** The request carries `repeated ChunkCodec accept_codecs = 5`. The response carries `ChunkCodec codec = 7`, `uint32 raw_size = 8` and `optional uint32 crc32 = 9`, as `FileChunk` does. The host lists its codecs as `remote_end.rs` does, and the machine picks one as `files.rs` does.
  7. **The file's identity** (`source`) is the machine's id, the device, the inode and the birth time when the file system gives one. It never holds the size or a modification or change time: those move on every append, and every read would then look like a rewrite. An empty `source` matches only at offset 0.
  8. **A rewritten file is one mirror write.** On `rewritten` the host saves the new `source`, sets the source offset to 0 and appends the data in one call and one state write, with `from` 0. There is no separate `restart_source` followed by a `take` at the old offset: that `take` would be dropped as stale. What was mirrored before stays (LD-9), and seq and turn numbers go on.
  9. **Triggers only.** A read that fails is logged and waits for the next link-up or session report. There is no retry, timer or backoff. The host subscribes to both before its first pass.
  10. **One machine never holds up another (TR-6).** Reads for another machine run in a task of their own for each machine, one session at a time. The follower that mirrors this host's sessions never waits on one.
  11. **Docs.** A comment on `MirrorChunk`, `MirrorOffset` and `UplinkAck.mirror` in peer.proto says they are unused and names this decision; GRPC.md §8 names ReadMirror as a call only the relay makes. Both are main's files.
  12. **Tests beyond the design's three:** a truncated file and a replaced file each keep the old entries and add the new ones; a line over 1 MiB doesn't stop the lines after it; a value in a key, in a number and in a string that JSON escapes is replaced. They check what is mirrored, not wording.
- **Gives up:**
  - The mirror of another machine's session moves when the machine reports on the session (a turn ends, the agent stops or resumes) and when its link comes up, not line by line. This host's sessions already work that way (D170).
  - A report can arrive before the agent has written its last line; that line is mirrored on the next report.
  - When an agent rewrites its conversation into a new file, the entries are mirrored a second time.
  - A text over 64 KiB inside a line over 1 MiB is cut; a pasted image is the usual cause, and the mirror keeps no images anyway.
  - The host learns the length in bytes of what a machine's file holds, so roughly how long a replaced value was. peer.proto's comment had offsets count scrubbed bytes.
  - A file swapped in place for a longer one with the same identity is not noticed. That takes an outside tool, since agents only append.
  - This host's own sessions keep the truncation gap they have today; the same identity check can close it later.
- **Open, for the lane that builds moves and restores (MV-2, LD-4):** a move or restore puts a copy of the conversation on a machine. That code must set the mirror's source offset to the copy's length, or the carried history is mirrored twice.
- **Later, with no change to the call:** the machine watches each live conversation file (TR-12) and publishes that it grew, as one more trigger.
- **Not taken:** the push path (reasons under Problem); letting the host scrub (values would leave the machine, against SEC-1); a path or file name in the request.
- **Reverse:** take `HistoryService/ReadMirror` off `LISTED_METHODS` and `ROUTED` in `daemon/humatad/src/relay.rs`, and drop the handler and the host's remote branch in `history_service.rs`. A swap to the push path changes only the host's follower and the machine's handler; the mirror's `take` stays.

## D190: A sent comment whose passage is gone becomes a detached draft (minion REVIEW, advised by a Fable agent, 2026-11-15). Provisional

- **Problem:** SM-7 says a comment whose passage is gone offers Re-attach or Drop, and flow 26.20 re-attaches a sent comment and sends it again. But `place()` reported "gone" as "changed", so a sent comment became Addressed with an empty "after". Sending refuses anything but drafts, so re-attach-and-send could not happen. The "after" was also read at the old line numbers, so lines added above an edited passage showed unrelated text.
- **Pick:** After the Same and Moved checks, `place()` scores each window of the quote's length by the words it shares with the quote (maximal alphanumeric runs, case kept, as a multiset). The best window wins, ties going to the window nearest the anchor, then the earlier one. If at least half the quote's words survive, the passage is Changed at that window, and its lines are the "after". Otherwise, or when the file is empty, it is Gone. A quote with no words is Changed if its lines are still inside the file, else Gone.
- **Ledger:**
  - A Sent comment whose passage changed becomes Addressed, with the anchor moved to the window and the quote kept as the "before".
  - A Sent comment whose passage is gone becomes a detached Draft: `sent_at` and `addressed` are cleared; text, replies and id are kept. This mirrors `reopen`, and the re-send is a fresh delivery.
  - A Draft or Waiting comment detaches.
  - Addressed, Resolved and archived comments never change (SM-17).
  - A detached comment can't be sent (`Detached`: re-attach or drop it first), and it isn't counted in the draft count.
- **Not taken:** keeping "gone" as Addressed with an empty "after" (contradicts SM-7 and flow 26.20, and "Resolve N addressed" would sweep it away).
- **Open:** a deleted file still leaves its comments as they are, because the pass skips files it can't read.
- **Reverse:** drop `Placement::Gone` and `CommentLedger::gone`, and send everything to `changed` again.

## D191: An agent's permission hook holds a stream to its own daemon, which raises one Inbox card (minion HOOKS, advised by a Fable agent, 2026-11-16)

- **Problem:** Claude Code, Codex and Copilot each run a blocking PermissionRequest hook before their TUI asks to run a tool. Answering in the Inbox has to reach that hook. The ways open were reusing `AskUser` (a question card), a new streaming call with an approval card, and a CLI that knows each agent's payload versus one that forwards it.
- **Pick:**
  1. **Transport:** a new server-streaming `SessionService.Hook { agent, event, payload bytes }`, for `Caller::Agent` only. It sends keepalives, then once the bytes the hook prints. `humata hook <agent> permission` reads stdin (1 MiB cap), forwards it, prints what comes back and always exits 0; with the daemon down it prints nothing.
  2. **Per-agent knowledge sits in the daemon,** in the pure crate `daemon/agent_hooks` (`humata2_agent_hooks`): payload parsing, the answer each agent reads, and the hook config. One build holds all three, so the CLI and the daemon can't disagree. Payloads it can't read whole (another shape, a guessed field, a Copilot sandbox bypass, an unknown event, over the cap) give empty output: no decision, never allow or deny.
  3. **The card:** kind SESSION, title "<agent> wants to run <tool>", a `CommandPreview` with the whole command, patch or tool input, and Approve / Deny. A deny note goes to the agent as its message. No "Always allow": the agent's own rules are a different thing from a Humata grant (deferred). The Inbox entry gets a real discriminator (`permission`) rather than another meaning of `access: None`; Answer all skips these cards.
  4. **Lifetime:** the card exists while a hook holds the stream. When the last stream for it drops (the agent killed the hook, the TUI took over), the daemon withdraws it as CALL_ENDED. An answer commits only while a stream is alive, else it gets the withdrawn error. A rerun of the same ask (session plus a hash of tool and input, plus Codex's turn id) attaches to the pending card. Cards are not saved when raised, so a restart leaves none pending; their answers and withdrawals are saved as history.
  5. **Status:** D169's async `turn needs-input` hook stays beside the blocking one. Both start together, so its report is older than the next PostToolBatch and loses to it.
- **Not taken:** `AskUser` (labels a Bash approval a question; a reply that means deny; unary, so a killed hook can't withdraw); a CLI that parses payloads (skew between CLI and daemon splits one feature).
- **Risks pinned:** clap exits 2 on an unknown subcommand and Codex and Copilot read exit 2 as a deny, so every hook command ends `|| true`, with stdout kept. Only PermissionRequest is hooked, not PreToolUse too, so one ask is one card.
- **Open:** a remote machine's agent reaches its own machine's daemon; carrying the normalized card over the host link is for the remote-work lane. Until then a remote machine's hook gets no decision, and the TUI asks.
- **Reverse:** drop the Hook RPC, `humata hook`, `inbox/permission.rs`, the `permission` field and the blocking hook entries; the async status hooks stay.

## D192: The permission hook holds for an hour, the daemon ending it first (minion HOOKS, advised by a Fable agent, 2026-11-16)

- **Problem:** Copilot's hook timeout defaults to 30 s and Claude's and Codex's to 600 s. A short hold withdraws the card before a push is read; the fallback after it is the TUI prompt, where a user who is away is stuck anyway.
- **Pick:** every agent's config states `timeout` (`timeoutSec` for Copilot) explicitly: 3600 s for the permission hook, 10 s for turn hooks (3 s for Codex's Interrupt, its cap). The daemon ends the stream with no decision at 3590 s (`DAEMON_HOLD`); the CLI caps its own wait at 3595 s, so the daemon always ends first. No Setting.
- **Gives up:** answering in the TUI while the hold runs. If that hurts, add "Answer in terminal" to the card before shortening the hold. If an agent clamps the timeout, its kill fails open and the dropped stream withdraws the card.
- **Reverse:** change `HOLD_SECONDS` in `humata2_agent_hooks`.

## D193: Codex hooks go in by `-c` without the trust bypass; Copilot's by a session plugin (minion HOOKS, advised by a Fable agent, 2026-11-16). Provisional

- **Codex:** launched with `-c hooks=<inline table>` (the session-flags layer). Codex runs a non-managed hook only once the user has trusted it, so the user runs `/hooks` in Codex once and trusts Humata's. `--dangerously-bypass-hook-trust` is not used: it also runs the user's own untrusted hooks. A per-session `CODEX_HOME` is not used: it moves auth and writes trust into a folder Humata owns. The command is kept stable (the `humata` path, the socket, the timeouts), since any change costs every user a re-trust. Settings › Agents shows a fixable step on the Codex row ("Run /hooks in Codex once and trust Humata's hooks") until a hook from this command is heard, and again when the command changes; no guessed status.
- **Copilot:** launched with `--plugin-dir <state>/copilot-plugin`, holding `plugin.json` and `hooks.json` to the documented schema. `COPILOT_HOME` is not used (it moves auth and state). Revert to output only if a live launch shows Copilot warning or prompting about the plugin.
- **Both:** the kinds keep `turn_end: output` until a live check shows the hooks run; the status rule already lets a heard hook win over the fallback. The live checks are owed (GAPS_REVIEW).
- **Reverse:** drop `{codex_hooks}` and `{copilot_plugin}` from the kinds' args.

## D194: A deleted channel's records travel in its trash item as a store-made side file (minion FORK, advised by a Fable agent, 2026-11-16). Provisional

- **Records:** before trashing, the store copies the channel's rows (channel, sessions with their details, tabs and agents, board items, rules, ...) into `<channel folder>/.humata-deleted.sqlite`, with the same schema and stamped with `user_version`. One transaction then removes them from the store, and `Trash::put` takes the folder. Restoring a Channel item moves the folder back, migrates the side file with the store's own migrations, imports it in one transaction with every session archived, and removes the file. A restored folder that holds the file but has no channel row is finished at start. Not "mark deleted" (every reader would have to skip such rows, `channels.folder` UNIQUE would block reuse, and emptying the trash by hand would orphan rows). Not JSON (store records aren't serde, and the schema can drift in 30 days). A test fails when a schema table is neither copied nor declared global. `board_prefixes` and `board_counters` stay, so item keys are never reused. DeleteSession (LD-5) can reuse this.
- **Requests:** pending requests are withdrawn with `WITHDRAW_REASON_CHANNEL_DELETED` before the snapshot, and stay withdrawn after a restore.
- **Name reuse:** a channel in the trash doesn't hold its name. If its folder is taken at restore, it comes back as the first free `<folder>-2` (name and folder), with session paths rewritten at import. This needs a restore-to-path in `Trash`.
- **Other machines (first cut):** refused with not_built while another of the channel's machines has live sessions or is offline (the plan names it in `offline_machines`; the fix is to bring it online or forget it). Online machines with no live sessions send their channel folder to their own trash through the forwarded file delete. `trash_item` is the host's.
- **Reverse:** a store-side "deleted" mark instead, with a trash-sweep hook.

## D195: Sessions cross machines through a host-only carry service and the host's journal (minion FORK, advised by a Fable agent, 2026-11-16). Provisional

- **Calls:** a new `SessionCarryService` in sessions.proto (Pack, Unpack, Release, Materialize) refuses every caller but `UserVia::Host`, with one test for each caller, as D189.1 does. Pack and Release aren't user actions, so they don't go in SessionService. TransferEndService answers users (D162). peer.proto's calls run in the link process, which has no store (D139). Each call is keyed by the host's operation id and safe to repeat. This amends D139.
- **Engine:** a separate transfers engine instance, kept out of the tray. A move shows its step on the session (MV-4), and Cancel goes through the operation, which undoes it (MV-3). `<state>/carry` holds journals and manifests only. The host relays the carried bytes and saves none (SEC-1).
- **Rebuild's conversation:** the mirror keeps the raw scrubbed lines, but only for other machines' sessions, with the codec named (TR-13). Rebuild resumes from those lines (SL-4, 26.23). FK-6's transcript and summary are the fallback when:
  - the agent can't resume,
  - a line was left out (D189.5), or
  - the resume check fails.
  Sessions mirrored before this change are backfilled. The lost-work note says that `‹NAME›` stands for a host variable.
- **Context-only fork onto another machine (FK-8):** allowed, with no mount. The working folder starts empty, and the first typed line names the parent's machine, its folder and the file tools. Writing to another machine's files needs a new Ask capability beside FILES_READ_OTHER_MACHINE, with its Settings switch. This part is deferred until after the first cut.
- **Crash mid-move:** the host's journal decides, in the restart report format (MV-14).
  - The machines never resume a session paused for a move on their own, and never delete an incoming copy on their own. The host's abort does both.
  - If the host was killed between "resume there" and "switched", it asks the target. An agent running there means the host finishes the move; otherwise it aborts.
  - The source resumes only after the target acknowledges the abort, or when the journal shows resume was never sent.
  - If the target's link drops mid-copy, the host aborts and resumes the source (MV-3).
- **Fork copy:** a snapshot into staging on the source first. Copying a live clone gives a torn `.git` and stale hashes, or keeps the parent paused through link drops. A fork carries the branch and the uncommitted files, without ignored files. Only a move keeps ignored files (MV-13). LD-19 checks the free space on both machines and names the one that is short.
- **Desk:** a fork stays refused (D108.3, DK-12). A Desk move is owed (DK-3, MV-1, D108.6): the scratch folder is carried, a picked folder is copied and left, there is no branch, and any online machine can take it. It stays refused in the first cut and comes in a later slice on the same carry.
- **Order:** rig, pointers, raw mirror lines, Forget, carry core, carry service, host driver, Move, remote fork (code and code+context), remote archive and restore, RemoveMachine, Rebuild, then context-only and own-copy elsewhere, then the Desk move.
  - Restore always takes the conversation from the host (LD-4), never from the source.
  - Never cut: the manifest hashes, undo, crash recovery, LD-19, and MV-13's store rows.
- **Reverse:** each pick stands alone.

## D196: Branch prefix settings, an edited branch at start, and "Rename the branch too" (minion FORK, advised by a Fable agent, 2026-11-16). Provisional

- **SettingsService serves only `branch_prefix` for now.**
  - Reads give the defaults for every other section. A write that sets any other section refuses the whole call with not_built, naming the section.
  - **Storage:** a new settings table (key, value, revision), added in a new migration step. No row means the default.
  - **Empty channel override:** `channels` gains `branch_prefix_set`, so an empty per-channel override is possible (FC-12). Existing non-empty rows get 1. `ChannelRecord.branch_prefix` becomes `Option<String>`.
  - **Default prefix:** the host's `gh` login plus "/", cached in memory and refreshed with the check facts. It is never stored. A start never waits on gh and never fails because of it.
  - **Access:** writes are user-only with presence (`require_present(SettingsChange)`); agents are refused with user_only. `expected_revision` is checked. A prefix that can't begin a valid ref is invalid_field.
  - **Owed:** the SettingsChanged event.
- **Edited branch at start (FC-15):** `StartSessionRequest.branch = 11` is used as typed for every included repo, with no prefix added.
  - A bad ref name is invalid_field.
  - A taken name is name_taken, with the free name as the suggestion. Humata never adds a suffix silently.
  - The folder stays the title's slug.
  - **Owed:** the same field on ForkSessionRequest.
- **Rename the branch too (FC-17):** `RenameSessionRequest.rename_branch = 5`.
  - The new name follows the same rule, built from the new title. The session's own branch doesn't count as taken, and the same name changes nothing.
  - Any pushed repo refuses the whole call with FAILED_PRECONDITION invalid_field on `rename_branch`, naming the repos. Pushed means an upstream is set, or `refs/remotes/origin/<branch>` exists. This is checked on the session's machine at call time.
  - An offline machine gives machine_offline, and nothing is renamed, the title included.
  - The order is: the branches first (rolled back on failure), then one record write. The folder keeps its name.
  - The dialog offers the tick when every `pointers[].upstream` is empty.
  - Review, Desk and context-only fork sessions refuse the tick.
- **Which prefix:** the channel's override if set (even empty), else the global setting, else the gh default. It is resolved at each preview, start, fork and branch rename. Existing branches never change.
- **Reverse:** drop the two proto fields and return SettingsService to `not_built.rs`.

## D197: Until the Review a pull request dialog exists, a pasted PR offers Review this PR only when a session already reviews it (minion FIND, main, 2026-11-16). Provisional
- **Problem:** KP-4/PR-1 say a pasted PR link offers **Review this PR**, which opens Review a pull request (frame 67). That dialog belongs to the PR milestone, which is blocked (REVIEW, ASK, FRAMES2) and needs a host-side `gh` fetch (PR-3). Offered for any PR, the entry could only toast "start a session to review it": a control that does nothing (actionable UIs only).
- **Pick:** the palette offers Review this PR for a pasted PR only when a session here already reviews that PR (`workspace.pulls[].session`); picking it opens that session. Other pasted PRs offer no review entry and search as usual.
- **Undo:** the PR milestone brings the entry back for every pasted PR, opening the dialog.

## D198: "Since I last reviewed" in a repo never reviewed shows everything since origin/main (minion REVIEW, advised by a Fable agent, 2026-11-16). Provisional
- **Problem:** Flow 26.3 picks "Since I last reviewed" in a fresh session and expects three repo groups with totals. The daemon refused instead ("no review of <repo> yet … press Mark reviewed first"), so the panel showed an error and no diff.
- **Pick:** when the session has no snapshot of a repo and origin/main resolves, "Since I last reviewed" diffs that repo from its merge base with origin/main, as the origin/main base does. The base chip still reads "Since I last reviewed", and the repo's comparing label starts "not reviewed yet:". Repos that have a snapshot still diff from it. Without origin/main the old refusal stands.
- **Undo:** drop the fallback in `review_diff` `Checkout::base_tree`/`summary`; the flow then presses Mark reviewed first.

## D199: Settings › Agents › "Ask for permission in the Inbox", one switch that is on by default (minion HOOKS, owner's request via main, 2026-11-16). Provisional
- **Problem:** with D191, every agent's permission ask waits in the Inbox. Some people want their agents to ask in the agent's own prompt, as they did before Humata.
- **Pick:** one switch in Settings › Agents, styled like the other toggle rows (title, subtitle, toggle on the right). It is checked against golden 29-settings.png: the frame shows only Layout and has no per-agent rows, so this is one switch for every agent, not one per agent. The setting lives on the host (`SettingsService`, `Settings.agents.permissions_in_agent`, store key `agents.permissions_in_agent`). Proto3 makes false the default, which is the Inbox. When the switch is off, the daemon answers every permission `Hook` at once with no decision and raises no card, so the agent's own prompt asks. The hook stays installed. The daemon reads the setting at each ask, so running sessions follow a change with no restart. Turn ends and needs-input notices don't change. Opening Settings › Agents reads the daemon's value. A refused change goes back to the old value and says why. The setting shares the one Settings revision (the newest of its sections' saves).
- **Undo:** drop `AgentSettings` from settings.proto and the check in `hook_of`. Asks then always go to the Inbox.

## D200: A sent batch of comments is pasted, then submitted with return (minion REVIEW, 2026-11-16). Provisional
- **Problem:** SM-9 types a batch as one bracketed paste, and RV-5 says Send N comments "sends" it as one message and marks the session working. But nothing pressed return, so the paste sat in the agent's input line and live flow 26.3 never saw the agent get the comments.
- **Pick:** the daemon's terminal sink types the paste, waits 100 ms (an agent CLI reads a return right behind a paste as part of it, as with the start-fresh seed), then types `\r`, each keyed once (`<key>`, `<key>/return`). If the return fails, the batch is Unconfirmed, and sending it again presses only return because the paste's key has already arrived. After a restart, "arrived" still checks the paste's key, so a batch whose return was lost reads as sent.
- **Undo:** drop the return in `review/sink.rs` `TerminalSink::paste`; the user then presses Enter in the terminal.

## D201: OpenCode runs as its own TUI attached to a per-session server that a `humata opencode` wrapper starts, and the daemon is a second client of that server (minion HOOKS, advised by a Fable agent, 2026-11-16). Provisional
- **Problem:** OpenCode v2 (pinned for tests at v2.3.0, published 2026-11-13) dropped the TUI's own port. The v1 routes are gone too, and every route is now under `/api` (facts in HOOKS-research.md › OpenCode, verified against v2.3.0). Humata still needs turn ends and Inbox permission asks from it without guessing, while the TUI stays the agent.
- **Pick:**
  - **Process:**
    - OpenCode becomes a built-in kind whose program is `humata opencode`.
    - The wrapper runs `opencode serve --hostname 127.0.0.1 --port 0` with stdin from /dev/null. It reads the `server listening on <url>` line, keeps draining serve's output to a capped log, and runs the TUI as `opencode --server <url> -s <id>`.
    - Both processes stay in the holder's process group, so they survive daemon restarts and close with the PTY.
    - The wrapper kills serve when the TUI exits. If serve dies, it ends the TUI and exits non-zero.
  - **Password:** fresh random per start. It goes to serve as `OPENCODE_SERVER_PASSWORD` and to the TUI as `OPENCODE_PASSWORD`, by env only.
    - It is written with the url, the OpenCode session id, serve's pid and a format version to `<session state>/opencode.json`, 0600, atomically.
    - The daemon reads that file after a restart.
  - **Conversation:** OpenCode mints its own `ses_…` id. The wrapper creates the session (`POST /api/session`, with metadata naming Humata's conversation) and records the id. On resume it checks the id still exists, and creates and re-maps the session if it doesn't.
  - **Rules:** none. The user's own OpenCode config decides what asks.
  - **Link:** each session gets a task in humatad that subscribes to `GET /api/event`.
    - Turns:
      - `session.execution.started` on any session of that server means Working.
      - When nothing is active after `succeeded`, `interrupted` or `failed`, that means Idle.
      - `permission.asked` means NeedsInput.
    - Cards: `permission.asked` raises an Inbox card unless D199's switch is off.
    - Answers:
      - Allow replies `once`; "Always allow" stays deferred.
      - Deny replies `reject` with the note as `message`.
      - No answer sends no reply, so the TUI's own ask stays.
    - Withdrawals: when the TUI answers first (`permission.replied`, or a 404 on our reply), the card is withdrawn.
    - On connect it opens SSE first, then reconciles with `GET /api/session/active` and the pending asks, deduped by id.
    - `turn_end: server` is a new `TurnEnd` variant. The output-quiet fallback applies while the link is down.
  - **Input:**
    - Stop uses `POST /api/session/{id}/interrupt`, falling back to the PTY when the link is down.
    - Prompts stay typed into the PTY: one send path for every agent, and nothing beside the TUI drives the conversation.
  - **Binary:** the user's `opencode` on PATH, so their login and data are used. Below 2.3.0, or with no URL in time, the wrapper runs the plain TUI on the output fallback and records a check fact that names the version found and the upgrade.
  - **HTTP:** `hyper` (client, http1), `http-body-util` and `hyper-util`, at the versions already in Cargo.lock, in one small crate shared by the wrapper and the daemon.
- **Accepted:** the agent's tools run as the same user and could read the password, the same boundary as `humata hook`. `/health` and the web UI are open on loopback; only `/api` is guarded, and nothing binds beyond 127.0.0.1.
- **Known gap:** `/new` in the TUI moves off the mapped session, so resume returns to the old one.
- **Undo:** drop the built-in kind and the link. A user-supplied OpenCode kind stays on the output fallback.
- **Amended the same day (D201a, advised by a second Fable agent):** the wrapper runs the link itself, not humatad.
  - **Calls:** it holds `HUMATA_SESSION_TOKEN`, so it calls the daemon as the session's agent.
    - Turns go through `ReportTurn`, as `humata turn` does, and the kind's `turn_end` is `hooks`, so there is no new `TurnEnd` variant and no proto.
    - Each `permission.asked` opens the existing `SessionService.Hook` stream with agent `opencode` and the ask as payload. `agent_hooks` parses it, and renders the decision as OpenCode's reply body (empty: no reply).
    - The card key includes OpenCode's permission id, so a reopened stream finds the same card and two identical asks get two cards.
  - **Withdrawal:** `permission.replied` (the TUI answered first), a 404 on the reply or the run ending drops the stream, and the daemon withdraws the card as it does for every hook. D199 is read per ask as before.
  - **Reconnecting:** a transport error on the stream reopens it with backoff while the ask is still pending. An empty final reply (D199 off, or the hold ran out) is never reopened.
  - **Turn after an answer:** the wrapper reports Working on `permission.replied`, since no `execution.started` follows a reply.
  - **The link's own failures:** it reconnects SSE and reconciles from the active sessions and the pending asks. A failure of the link never ends the TUI.
  - **Session id:** OpenCode keeps an id it is given when it starts `ses_` (round 3), and a repeat create returns the same session. So the wrapper names the session `ses_` plus the first 26 hex digits of Humata's pinned conversation id, creates it idempotently on every start, and attaches the TUI with `-s`. There is no mapping to keep, and resume is the same command.
  - **Accepted (round 3):**
    - Tools inherit serve's env, so the model can read `OPENCODE_SERVER_PASSWORD`. That password guards only this loopback server and dies with it.
    - Serve shares the user's OpenCode database, so Humata's sessions also show in their own OpenCode lists.
    - `--service` is never used, so the user's default TUI never attaches to Humata's server.
  - **Stop:** always typed into the PTY, as for every agent. The daemon has no HTTP client and reads no password file. The state file is the wrapper's own, for resume and for killing a leftover serve.
  - **Why:** it reuses the hooks path built for D191 (identity, cards, D199, hold, withdrawal) instead of a second way to raise cards inside the daemon, and the daemon stays out of OpenCode's server.

## D202: Settings › Agents shows Codex's trust step until Humata hears one of its hooks (minion HOOKS, advised by a Fable agent, 2026-11-16). Provisional
- **Problem:** D193 runs Codex's hooks only once the user trusts them in Codex's `/hooks`. Until they do, Codex sends no turn ends and no Inbox asks, and nothing says why.
- **Pick:**
  - **The fingerprint:** a sha256 of the exact `-c hooks=…` value the daemon fills in. It includes the `humata` path, the socket and the timeouts, because Codex keys its trust on the hook's hash.
  - **Heard:** a `Hook` RPC or a `ReportTurn` from a session whose kind's args carry that value. Either one stores the fingerprint under `agents.codex_hooks.heard`. Turn hooks count as heard because a permissive Codex may never ask.
  - **Shown:** the daemon sets `AgentSettings.codex_hooks_need_trust` (output only; writes ignore it) while some kind carries the value and the stored fingerprint differs from it. A changed command shows the step again with no extra logic.
  - **In the app:** while the flag is set, Settings › Agents shows one more row, "Trust Humata's hooks in Codex", with the fix "Start a Codex session and run /hooks once, then trust Humata's". The row has no dismiss: it clears when a hook is heard and is re-read whenever Agents opens.
  - **Narrowed from the Fable pick:** the step shows from the start, not from the first launch. That spares a launch record in session_service.rs, and starting a Codex session is part of the fix.
- **Undo:** drop the field and `session_service/codex_trust.rs`. The step then never shows.

## D203: The relay allowlist gains SessionCarryService, for the host only (amends D139) (minion FORK, 2026-11-16). Provisional
- **Problem:** Move takes a session from this Mac to a box or from box to box, and both ends are machines. Only each machine's socket daemon has the session's clone, holder and conversation file, and the host reaches it only through the relay (D139).
- **Pick:** the relay allowlist grows by one service, SessionCarryService (`Pack`, `Unpack`, `Release`, `Abort`), so it lists 30 methods. A machine honours these only from the host's proven relay connection (D144), never from the user or an agent on its own socket. D144's other rules are unchanged.
- **Why not the peer link's PeerService:** carry needs the machine's store and holders, and the link process has none (D139).
- **Reverse:** remove the service from the allowlist and from serve registration. Move then fails with a clear error and nothing else changes.

## D204: The relay allowlist gains SessionService/RenameSession (amends D139) (minion FORK, 2026-11-16). Provisional
- **Problem:** renaming a box session's branch has to rename the branch in the session's clone, and that clone exists only on the box.
- **Pick:** the host relays `RenameSession` to the box's socket daemon, so the allowlist grows by one method. D144's rules are unchanged. A same-user process holding the key could already change that clone directly, so this gives it nothing new.
- **Reverse:** remove the method from the allowlist. Renaming then changes only the host's record, and the box keeps the old branch name.

## D205: The relay allowlist gains PlanArchive, ArchiveSession and RestoreSession (amends D139) (minion FORK, 2026-11-16). Provisional
- **Problem:** archiving a box session, restoring it, and removing a machine that still has live sessions all have to stop agents and pack or unpack work that exists only on the box.
- **Pick:** the host relays these three SessionService methods to the box's socket daemon. D144's rules are unchanged. MV-9 still applies on the box: archiving over unpushed work asks first.
- **Reverse:** remove the three methods from the allowlist. Archive, restore and removal then work only for this Mac's own sessions, and for a box session they fail with an error that names the box.

## D206: A reusable terminal replica shares a persistent host; SSH reattaches its existing holder (proposed by minion C1, owner request, 2026-11-16). Provisional

- **Problem:** an embedded terminal must be a real PTY, preserve its process through viewer loss, expose generic input/metadata/hooks, and render the same local/SSH sessions in desktop and browser clients.
- **Pick:** `terminal_replica` is a single-terminal GPUI entity over supplied ordered messages, without fonts, sockets, tabs or app notification policy. The demo composes these same entities. The host owns the authoritative Ghostty VT, terminal replies, controller leases and local/SSH holder attachments. Only the controller writes input or resizes; mirrors can select, scroll, copy and pan independently.
- **Recovery:** a live holder owns the running process and bounded output replay. The host records private bounded codec-tagged checkpoints and minimal UUID/name/endpoint/layout metadata. LZ4 is preferred, with explicit size-budget fallback to Zlib and a caller-extensible codec registry. Ordinary input and launch argv are excluded from memory-only descriptors. A disk checkpoint restores display/parser state, not arbitrary process memory after a reboot.
- **SSH:** loss disables input and retries the same holder with 1–30 s capped backoff, at most four concurrent attempts globally. A retirement barrier resolves old acknowledgements; unresolved writes become uncertain and are never replayed. Gaps or ambiguous reply/resize state require explicit recovery. Stock Mosh is not bundled: its visible-screen synchronization cannot carry this ordered binary holder/checkpoint protocol. It would also leave the browser's forwarding requirement. See `terminal/host/TRANSPORT.md` for primary sources, verified outage/reroute behavior and limits; this does not promise predictive echo or UDP roaming.
- **Embedding:** durable UUIDs, explicit and automatic names, bounded attributes, fresh same-endpoint/cwd siblings, raw/paste injection and queued/written/uncertain receipts are generic APIs. No receipt means agent completion. Versioned provider adapters consume supplied structured events, never infer completion from transcript text, and expose hooks without issuing OS notifications. Host shortcuts and inactive-view memory saver are opt-in. Parking releases a replica after an ordered detach while its host parser/process continue; allocator high-water pages may remain reserved.
- **Input:** keyboard input requires terminal focus. Async clipboard reads use bounded FIFO barriers so later Enter cannot overtake a paste; focus loss, failures, refused paste or lost authority cancel buffered input. Copy exports selected plain text. Real local/SSH and physical browser clipboard fixtures verify Unicode/hard newlines and exact paste-back. Further Neovim work is deferred by the owner.
- **Permissions:** D199 remains the host-wide choice between Inbox and the agent's own prompt for future asks. Separate local `permission_request_alerts`, default off and saved with UI preferences, controls extra in-app toasts only. It never grants, denies, removes or reroutes an approval. Existing held Inbox asks remain pending when D199 changes.
- **Adoption boundary:** the standalone experiment is not a replacement for the main daemon's terminal API. Mac browser stress/reload and Linux painting/clipboard/reload pass with optimized assets on the unchanged 721 MiB VM. Debug viewer reloads OOM-killed Chromium, so release acceptance uses `-c opt`. Full platform/agent trust/IME/long-duration certification remains separate work.
- **Undo:** remove the experiment packages and their narrow Bazel/toolchain/index wiring; retain the main app's existing terminal transport. The permission alert field can be ignored by older UI saves; D199 is independent.
- Owner's pick (via Fable 5.2, 2026-11-16): mosh is skipped for good. Predictive local echo goes on the backlog after 1.0: built in our own terminal view so desktop and browser share it, shown only above a measured latency threshold, never for hidden input, never replaying input. A QUIC daemon link is not added to the backlog; reopen only if measurements over real SSH show stalls that reattach and backoff can't fix.

## D207: The host alone recovers a move when a box restarts mid-move (minion FORK, Fable pick, 2026-11-16). Provisional
- **Problem:** a box that restarts mid-move cannot know whether the host is still driving the move: it has no journal, and links run host to box. A box-side drop of incoming stages could delete the only copy, or fail a move that would have finished.
- **Pick:** a box does nothing to incoming stages at startup. The host's journal is the record. When the host's take-back on the target fails (the box is offline), the journal stays open, so the host retries the undo when that box's link comes back (a push, no polling). After a successful switch, the host also removes the target's stage, never its session. A paused agent on the source is saved Stopped, so a restart never resumes it.
- **Gap:** if the host's journal itself is lost, the target's stages are orphaned. That becomes a doctor action the user confirms, never an automatic delete.
- **Reverse:** a box-side startup rule that drops incoming stages whose agent isn't running.

## D208: A context-only fork runs only on its parent's machine (FORK manager; picked by a Fable 5.2 high subagent, 2026-11-16). Settled (reversible)
- **Pick:** a context-only fork works in its parent's clones (D185), and agent CLIs work only in a local folder with their own file tools and shell, so it runs where those clones are. Asked for onto another machine, ForkSession and PreviewFork refuse it as a failed precondition that says to fork code + context there instead. FK-8 is amended to say so. In FK-3, the machine row offers only the parent's machine for context only, and picking another machine switches the kind to code + context with a one-line reason.
- **Why:** a network mount of the parent's folder (B) puts every git status, grep and build on a network filesystem, hangs the agent when the link drops, needs FUSE on every machine, and brings back the mounts D181 turned down. A two-way sync (C) makes two trees, which is what D185 says context only is not, and adds conflicts, `.git` index races and watchers on both sides. Running the agent on the parent's machine under the other machine's label (D) is a misleading status. What "another machine" is wanted for is that machine's compute or toolchain, and only a local copy gives that, which code + context already does (FK-12).
- Same machine only also amends D185 and D187.
- **Reverse:** restore FK-8's cross-machine clause and build a link-carried mount of the parent's session folder on the target, gated by the cross-machine read and write rules.

## D209: The palette hides an empty Conversations group (LOOK2 manager; picked by a Fable 5.2 high subagent, 2026-11-16). Settled (reversible)
- **Pick:** the palette shows its Conversations group only when it has hits, a next page or an error. While a search loads with no hits, nothing is shown if the palette has other entries. With no entries and no hits it shows one line: "Searching every machine's conversations…" while loading, otherwise a "Nothing matches" line that suggests fewer letters (and fewer words for a query of two or more characters).
- **Why:** frame 30 has no empty group, and an empty header with a spinner under real entries is clutter that offers nothing to act on.
- **Reverse:** return the group whenever a conversation search runs.

## D210: Two LOOK2 comparison picks (LOOK2 manager, 2026-11-16). Provisional
- Frame 32's capture keeps Home's Back button, because the archive that raises the toast leaves a trail; look_test masks it.
- The fixture's `rate` session stays on this Mac, though ruling 2 would put it on devbox-1. Moving it changes the Hosts counts, and the difference is within look_test's budget.

## D211: A moved or rebuilt conversation is mirrored once, by entry count (FORK manager; picked by a Fable 5.2 high subagent, 2026-11-16). Settled (reversible)
- **Pick:** `Mirror::rebase_source(session, machine, skip)` runs on the host just before the record says a session is on its new machine, both for a move and for a rebuild.
  - The mirror reads the new file from its start and drops its first `skip` entries.
  - For a move, `skip` defaults to the entries read from the old file, kept as `source_entries`. A rebuild passes `entry_count` of the lines it sent, which can be fewer while a raw backfill runs.
  - From then on, the mirror takes reads only from that machine (`""` is this host's own catch_up), so a stale read of the old file adds nothing.
  - The raw lines start over with the new file.
  - A failed move rebases back to the old machine. Rebasing twice is safe because `source_entries` holds the skipped entries.
  - This amends D189's open note, "set the source offset to the copy's length".
- **Why:**
  - A move or rebuild rewrites the old folder's paths, so the new file's byte length is unknown to the host.
  - Bytes from the target (B) need proto changes on two paths and still miss the lines a rebuilt agent writes before the host asks.
  - An entry count survives path rewrites and scrubbing, and it covers the mirror's lag: what the old file had beyond the mirror is still added.
  - A machine guard is used instead of an epoch because a session never moves to the machine it is on.
- **Open:** a restore of a box archive onto another machine doesn't rebase yet.
- **Reverse:** have the target report its new file's identity and length before the agent starts, and set the offset to that.

## D212: A fork that runs another agent gets the conversation as a handover file plus a summary (FORK manager; picked by a Fable 5.2 high subagent, 2026-11-16). Settled (reversible)
- **Pick:** FK-6.
  - A context fork (CODE_AND_CONTEXT or CONTEXT_ONLY) with an agent other than its parent's, or of a parent whose agent keeps no conversation file (OpenCode), starts the new agent on a fresh conversation.
  - The parent's conversation, cut before `before_turn` when one is given, is copied to `<fork folder>/.humata/handover-<fork id>.jsonl`. That is outside the clones, and named per fork so context-only forks in the parent's folder don't collide.
  - The first message is a plain digest (D143's, no model) of that file, or of the end of the saved terminal output when there is no transcript. It names the handover file relative to the agent's folder.
  - It is kept as a FreshSeed row before the launch and typed with keyed input as `[handover <conversation>] …`, so a restart types it once (D153).
  - The fork's model is the new agent's default, not the parent's, and the access check names the new agent.
  - The preview sets `seeded_with_summary` and says what the new agent will and won't know.
  - A CODE_ONLY fork with another agent just starts that agent fresh. Another agent on another machine stays not built.
- **Why:**
  - Humata's own state folder is unreadable to a contained agent (SEC-6).
  - The parent's own file keeps growing past the fork point.
  - `first_message` is typed unkeyed, so a restart could lose or repeat it.
- **Open:** deleting a context-only fork doesn't remove its handover file from the parent's folder yet.
- **Reverse:** restore the agent refusal in `check_fork`.

## D213: How a machine knows its host is reachable (AWAY manager, 2026-11-16). Provisional
- A machine's daemon starts away and counts its host reachable exactly while the peer link holds a `DaemonService.PeerUplink` open; a newer one ends the older. The host's daemon is always reachable.
- Calls the host relays (`UserVia::Host`) are judged with the host reachable, since they come through the link itself.
- PeerUplink is the link process's own call on its proven connection, not a relayed one: the host can't relay it, and it isn't added to the relay allowlist (amends the AWAY verdict's "on the relay allowlist"; the routed table can't hold DaemonService, which the link serves itself).
- Changes are published as `HostLinkChanged` on TOPIC_MACHINES to the user, and `GetServerInfo.host_reachable` says the current state, so tests wait on state, not sleeps.
- Until the away journal lands (AWAY T5), calls the policy would journal or answer from the last copy are refused `host_unreachable`, not an internal error.
- Board's Filter placeholder keeps the input theme's muted colour: gpui-component's Input takes it from the theme on every render, a theme-wide change would touch every input, and frame 24 is within budget (0.006%). (Added to D210, LOOK2, 2026-11-16.)

## D215: A stream that fails after its headers is recorded a second time (BUG2 manager; picked by a Fable 5.2 high subagent, 2026-11-16). Provisional
- The live client's recording layer (`clients/live/src/recorded.rs`) still tells each call once, at its headers.
- A stream that later fails is told a second time: a non-ok `grpc-status` in its trailers, or a body that breaks (its tonic status code). The second call has the same method and start time; its duration is the time until the failure.
- A stream that ends well, or that the app drops or cancels, adds nothing, so a recording doesn't end with a stray "cancelled" for the event watch.
- **Why:** the agent reading a report learns what failed. A long-lived watch still open when the recording stops stays visible, and a healthy stream doesn't double the events. No new field is needed.
- **Reverse:** drop `Streaming::watch` from `Recorded::call`.

## D216: Session boards in the app, a session's forks, and what hiding an archived session's items means (BOARD manager; picked by a Fable 5.2 high subagent, 2026-11-16). Provisional
- **Daemon, hidden (amends D155's "Also"):** an item is hidden when its board is an archived session's board. Hidden items are left out of every ListItems view and its counts; GetItem, GetItemHistory and every write answer them exactly like a missing item, a new Depends-on edge to one included. Stored edges to a hidden item stay (the loop check uses them) but aren't answered. An item merely worked on by an archived session stays visible. Restore brings everything back with no bookkeeping. Trashed sessions and channels: the store moves a deleted channel's rows out (D194), so nothing else is hidden yet.
- **Daemon, SessionWithForks (BD-5):** the session's board plus every descendant fork's by true parent, at any depth, walking through archived forks but leaving out their boards; an archived root lists nothing.
- **App model:** `Scope::Session { channel, session }` with `Scope::channel()`; the live client maps it from `BoardItem.board`. Exhaustive matches make every site choose.
- **App, channel board scope (BD-5, BD-6):** a persisted `ChannelView { Channel (default), WithSessions, SessionAndForks(id), OneSession(id) }` in the board's `View`; `BoardState.scope` stays the page. A saved session that's gone shows as Channel without rewriting the save. New item follows BD-6. Forks are computed from `Session.parent` in the workspace (the daemon's shown parent), any depth.
- **App, Global board (BD-7) and the session panel (BD-8):** the Global board leaves session items out. The session panel shows the session's channel's board together with that session's own board items, the session's own first in each lane; the header names the channel and New item there creates a channel item.
- **Golden conflict (settled by a second Fable 5.2 high pick, 2026-11-16):** frames 07/12 draw Ledger's panel with channel items (channel subtitle, channel labels, other sessions' chips), as frame 37 draws the Desk panel, so BD-8 is reworded to match them rather than the frames redrawn. No golden is owed a redraw: 07, 12, 21–24 and 37 stand, and the native 07/12 setup keeps the shared channel fixtures (no re-homing). SPEC BD-8 and SE-11 are reworded to match (main applies the seam). Still open for the owner's next wave, blocking nothing: no frame draws a session-board item inside the panel; until one does, it reuses frame 37's accent outline.
- **Gives up:** the session panel is not a private per-session list (a session-only view lives in the channel board's Scope picker); an archived session's board is unreadable and takes no new items until restore; a loop refusal may name a hidden key; session items on a channel board show only once a session scope is picked.
- **Reverse:** drop the hidden predicate and the descendant walk in `board_service.rs`; drop `Scope::Session` and `ChannelView`.
- Owner's pick (via Fable 5.2, 2026-11-16): settled. A session's own items in its Board panel come first in their lane with frame 37's accent outline. No label, chip, divider or section header. One session item is added to the panel fixture when 07/12 are next redrawn; no redraw is owed now.

## D214: Sessions that share a folder are archived and freed without touching each other's work (FORK manager; picked by a Fable 5.2 high subagent, 2026-11-16). Settled (reversible)
- **Problem:** a context-only fork works in its parent's folder (D187). Archiving it listed the parent's uncommitted work as its own, and freeing it deleted the parent's folder and clones.
- **Pick:**
  - Work check (plan, archive, summary): a session has no work of its own while another session of the same machine with the same folder has no archive row. So it gets plain Archive. A sharer that is archiving or archived doesn't count, so the last live sharer gets the full check and choices.
  - Free: while another sharer isn't freed, only the session's own handover file (`.humata/handover-<id>.jsonl`) goes and the row is marked freed. The last sharer to be freed removes the folder as before.
  - Frees of sharers run one at a time under a lock keyed by the folder, so the last one sees the others freed.
  - Restore needs nothing new: it clones only repos whose `.git` is missing.
- **Why:** archive and free are the only removal path, so refusing to archive a context fork would leave it unremovable without copies it never wanted. This also covers archiving the parent while its context fork runs.
- **Reverse:** refuse to archive a context-only fork until Give it its own copy or its parent is archived, and drop the sharer lookups.

## D217: What a contained agent may reach (AWAY manager; picked by a Fable 5.2 high subagent, 2026-11-16). Provisional
- **Problem:** SEC-6 allows only the daemon's socket and writes only in the session folder and `shared/`. Every agent needs its model's HTTPS API and writes its own config and login state in the home folder, so a contained agent as written couldn't work.
- **Pick:**
  - **Network:** outbound TCP to port 443 on any host, DNS (the resolver's socket and port 53) and the daemon's socket. Inbound, other ports (22 included) and local TCP stay denied. SEC-6 and HO-12 read "Its only network connections are HTTPS and its daemon's socket." A host-allowlist proxy is deferred past 1.0, because it would also break package fetches and custom providers. `git push` and `gh` failing is intended: pushes and PR actions go through Humata (PR-8, IN-24).
  - **Agent state:** the launched agent's own config folders, from a fixed list per agent, are readable and writable. Other agents' config folders are hidden. Keychain logins stay reachable.
  - **Home:** non-hidden home folders (Documents, Desktop, Downloads, other repos) and a fixed credential list are hidden: `~/.ssh`, gh, `~/.gnupg`, `~/.aws`, `~/.kube`, `~/.docker`, `~/.netrc`, `~/.npmrc`, `~/.git-credentials` and gcloud. Toolchain dot-folders, git config and `~/Library` stay readable, except Humata's own folders.
  - **Temp files:** `TMPDIR` points inside the session's own space.
  - **Scope:** both the global setting and the channel override are in 1.0. The channel control reads "Contain agents: Default (On/Off) · On · Off".
  - **Linux:** a contained launch or resume fails closed and never falls back to uncontained; the same holds for any profile error. The session stays stopped with its history intact. The message reads: "Can't start this session contained: Contain agents isn't available on Linux machines yet. Start it on a Mac, or turn Contain agents off for this channel. Agents there will then be able to read and write anything your user can on <machine>."
  - **Shown:** only on the Info panel's "What this agent may do" card (SE-20), from `Session.contained`. When the setting is on and the session isn't contained yet, the card reads "Not contained yet. Applies when this session next resumes."
- **Gaps, unverified, to test:**
  - A contained agent can edit its own agent's settings or hooks, which later uncontained sessions load. Separable settings files are read-only; `~/.claude.json` can't be.
  - Keychain items that open without a prompt (such as git's stored HTTPS credential) may be readable.
- **Reverse:** drop the 443 and DNS rules, the agent-state allowances and the home hides, which brings back SEC-6 as written.
- **Amendment (picked by a Fable 5.2 high subagent, 2026-11-16; Provisional): a contained Claude Code runs on a per-session copy of its config.** This closes the `~/.claude.json` gap.
  - **How:** a contained launch or resume sets `CLAUDE_CONFIG_DIR` to a folder in the session's own space. The real `~/.claude/` and `~/.claude.json` become read-only to the contained agent. Uncontained sessions use the real config as before.
  - **Login:** `CLAUDE_SECURESTORAGE_CONFIG_DIR` is set to empty so the usual keychain login is still found. This variable is undocumented and seen only in the 2.1.325 binary, so it must be tested.
  - **Transcripts stay shared:** the copy's `projects` links to `~/.claude/projects`, and only this session's own project folder there is writable. Pin, resume, mirror, carry and archive read the usual path. That the agent writes through the link is unverified and must be tested.
  - **Doesn't flow back to the user's real config (for the owner's wave):**
    - settings changed in the session
    - per-project trust and allowed tools
    - MCP servers added
    - todos
    - rewind checkpoints
    - usage counters
    - onboarding state

    Only the conversation transcript flows back.
  - **Reseeded at every launch and resume, from the user's:** `.claude.json`, settings, `CLAUDE.md`, hooks, agents, commands, skills and plugins. `.credentials.json` is never copied.
  - **Kept across resumes:** todos, checkpoints, and this session's own project entry (trust, allowed tools).
  - **Fails closed:** a seed error stops the launch, like any profile error.
  - **Info card:** "Uses a copy of your Claude Code settings, refreshed each time this session starts. Settings, MCP servers and permissions it changes stay in this session. Its conversation is saved as usual."
  - **Accepted loss:** if Contain is turned off later, the session keeps its conversation but loses the todos and checkpoints from its contained runs.
  - **Reverse:** drop the two variables and give write access back to both paths.
- Owner's pick (via Fable 5.2, 2026-11-16), the keychain gap: Humata never copies `.credentials.json` and never duplicates or rewrites the user's keychain login for the copy. Order at a contained launch: (1) the usual keychain login through `CLAUDE_SECURESTORAGE_CONFIG_DIR`, tested per Claude Code version; (2) otherwise a token or API key the user has already set for the session (`CLAUDE_CODE_OAUTH_TOKEN`, `ANTHROPIC_API_KEY`); (3) otherwise the launch fails closed, the session stays stopped with its history, and the message says: "Can't start this session contained: Claude Code's login isn't reachable from its contained settings copy. Set a Claude Code token for this channel, or turn Contain agents off for this channel. Agents there will then be able to read and write anything your user can on <machine>." No fallback to uncontained and no login prompt inside the session.
- Owner's pick (via Fable 5.2, 2026-11-16), flow-back: confirmed as listed. Settings, hooks, per-project trust and allowed tools, MCP servers, plugins, usage counters and onboarding state stay in the copy and never reach the real config, with no merge offer. The accepted loss is removed: the copy is kept with the session through archive and restore, and when the session next starts uncontained the daemon (not the agent) moves that session's own todos and rewind checkpoints into the real config, regular files only, by session id. The Info card wording stands.

## D219: The Desk frames in look_test: the terminal grid is masked, the header cluster and panel border are fixed (DESK manager; picked by a Fable 5.2 high subagent, 2026-11-16). Provisional
- **Problem:** look_test now checks Desk frames 36, 37, 38, 40 and 55 (39 is REVIEW's). 36, 40 and 55 match within budget (at most 0.05%). 37 and 38 missed it (3.7% and 1.4%), for three shared reasons that frames 01–17 show too.
- **1. The terminal grid (masked).** The grid draws whole-pixel cells (D22; `ui/src/terminals.rs` `font_theme` uses Menlo 11.5 so a cell is 7 px), and the goldens draw Menlo 12 text at 7.2. Long lines drift, "⎿" loses its space, and the fake agent's input box sits about 20 pt high. look_test masks the whole terminal body on 37 and 38 (`TERMINAL_BODY`): the cell pitch moves every row, so masking only the text rows would wrongly suggest the rest matches. native_test's recordings still cover the grid. The mask goes when a shared fix lands; that fix also brings 01–17 into look_test.
- **2. The session header's right cluster (fixed).** `chrome::action` put each 15 pt icon in a 16 pt box, so the cluster sat 0.5–3 pt left of the goldens. The box becomes 15 pt (`ui/src/chrome.rs`).
- **3. The side panel's border (fixed).** The panel drew its border inside its own width, after the divider's 1 pt. So the panel was 1 pt narrow and its content sat 1 pt right. The panel now spans the divider's point (`ui/src/panels.rs`, width + 1 with a -1 margin).
- **Why not mask 2 and 3:** they are layout defects that no ruling accepts. Masking them would hide a real bug. Leaving 37 and 38 out would drop two of the Desk's five frames from its own check.
- **Reverse:** put the 16 pt box and the inner border back, and mask the header cluster and the panel on 37 and 38 under this decision.

## D223: Logging in on a machine with the approval passphrase while its host is away (AWAY manager, 2026-11-16). Provisional
- **Problem:** flow §26.10 step 2 has the user SSH to a machine with the Mac closed, run `humata login` and type the approval passphrase (SEC-10, SEC-13, AW-5). BeginCliSession refused the passphrase as not built, and a CLI login named no presence method, so the guard took every login for an app-confirmed one.
- **Pick:** on a machine whose host is away, BeginCliSession with no proof asks for the passphrase (`presence_required`, accepted: approval passphrase). With the passphrase, the machine checks it against the hash its policy copy holds. A right one makes a login bound to the terminal session, like an app-confirmed one, that names the passphrase as its presence (`UserVia::CliLogin.method`), so it can answer what the machine's inbox holds (AW-5). A wrong one is `presence_required` again (the CLI exits 5). The fifth wrong try in a row locks the prompt for 15 minutes (`passphrase_locked`, naming the machine and the end). The count and the lock are kept in the store before the hash is checked, so a restart or a crash mid-check counts the try, and tries are checked one at a time. The check runs off the async runtime (Argon2id).
- **Refused:** on a machine whose host is reachable ("log in from the Mac, or wait until it is away"); on a machine never sent a passphrase (set one in Settings › Security on the Mac, then connect this machine once so it gets it); on the host (the passphrase is for machines while the host is away; confirm in the app). A session's own terminal is refused before any passphrase is asked for (SEC-12), as on the host.
- **Unchanged:** logins still live in memory, so a daemon restart ends them. An app-confirmed login from a machine is still not built.
- **Reverse:** BeginCliSession refuses the passphrase again and `UserVia::CliLogin` drops `method`.

## D220: On a machine, the user reads the rules it judges by from its policy copy (AWAY manager; picked by a Fable 5.2 high subagent, 2026-11-16). Provisional
- **Problem:** reading the rules (GetChannelRules, GetDefaultRules) is the user's AccessChange, which needs the Mac, so a user logged in on a machine while the Mac is away was refused `host_unreachable`, though the machine holds the rules it judges by. Answering Allow or Always allow there (AW-5, AW-12) without seeing the rule is a blind answer. ListGrants and Explain already answer from the held policy.
- **Pick:** on a machine, a user's two rule reads are answered from the kept policy copy, whether the host is reachable or away (one path, so it doesn't flip with the link; the machine's own store revisions are never the host's). The answer has no `revision`, so nothing can be written from it, and a new `copy` stamp (`PolicyCopyStamp`: the copy's revision and the host's time when it made it). A machine never sent a copy answers the defaults with no stamp. A channel the copy doesn't hold reads as the defaults and adds nothing to the policy.
- **Unchanged:** the guard, the policy and `AccessChange` (still needs the host). Every write (SetChannelRule, SetAwayRule, SetDefaultRules, SetApprovalPassphrase, RevokeGrant) is refused as before. Agents, scripts and the peer get the same refusals. On the host, reads are as before.
- **Wording (for the CLI, T6):** copy held, host away: "These are the rules this machine last got from your Mac, {age} ago ({time}). It judges by them until the Mac is back. Rules can only be changed on the Mac." Copy held, host reachable: "These are the rules your Mac sent this machine at {time}. Rules can only be changed on the Mac." No copy: "This machine hasn't received rules from your Mac yet, so it judges by the defaults (While I'm away: Wait). Connect it to the Mac to send your rules."
- **Open:** the age is from the host's clock when it made the copy, not when the machine got it.
- **Reverse:** the two Get handlers call `require_user` again on a machine; the `copy` field stays unset.

## D218: Picks in FORK's cross-machine third slice (FORK manager, 2026-11-16). Provisional
- A context-only fork of a box session onto that box goes through the relay: the host relays ForkSession to the box, which runs its own local context fork. The host records the fork only once the box's operation ends, under the id the box picked. Carry isn't used because it packs copies, and a context fork has none. **Reverse:** the box takes the host's id, which allows a journal on the host.
- A remote archive whose link is lost fails the host's operation. The host then checks the box when its link comes back and writes its row if the box archived. **Reverse:** keep the operation open until the box answers.
- Restoring a box archive on this Mac while the box is away takes the conversation from the host's mirror, and also does so when the link is lost during the copy step. The box's copy is released when its link is next up.
- Not yet durable: the three follow-ups above were kept in memory, so a host restart in between lost them. D222 and D221 make them durable.
- The host's archive row takes the files from the box's own summary, and the unpushed subjects and pushed sha from the pointers the box last reported.
- Owner's pick (via Fable 5.2, 2026-11-16): (1) relay context fork recorded under the box's id: confirmed, with D221's journal. (2) A lost link fails the host's operation and is reconciled on link-up: confirmed; the failure must say the box may still have archived and Humata will check when it reconnects. (3) Restore from the host's mirror while the box is away: confirmed, but the release changes. On link-up the host first compares the box's archive with what it restored; any later turns or unpushed or uncommitted work on the box are brought over, and the box's copy is released only after that succeeds. Until then it is kept. (4) Durable through D221 and D222: confirmed. (5) Archive row sources: confirmed; where the box's summary carries pointers, they win over the last reported ones.

## D222: A remote archive's and a remote restore's follow-ups survive a host restart (FORK manager; picked by a Fable 5.2 high subagent, 2026-11-16). Provisional
- Before the host asks a box to archive a session, it writes `<session>.reconcile.json` in the carry's `remote_archive` folder (`Carry::remote_archive_folder`). The file holds the session, the machine, the pointers the box last reported and the actor. The box's operation id is added once the box answers. On each link-up of that box, the host waits for that operation to end, reads the box's archive summary, and writes its own archive row only if the box archived the session. An operation the box no longer knows counts as ended, because the box's restart recovery finished or undid it. The file goes once a summary is read or the box is forgotten.
- Restoring a box archive on this Mac from the host's mirror while the box is away writes `<session>.release.json` before the switch, and drops any reconcile file for that session. On the next link-up the host releases the box's copy, and forgets what the box reported of the session, but only while the host's record still names this Mac. Otherwise the file goes and nothing is released.
- The folder comes from the carry, which owns `<state>/carry/`, and not from the store root's parent. **Reverse:** derive it from the store root.
- A failure that isn't the link's keeps the file. It is retried on the box's next link-up or the next host start, whichever comes first. There is no timer.
- Known gap, deferred: if the host restarted between asking the box and getting its first answer, the file has no operation id. A summary read while the box is still archiving then says "not archived", and the host's row is never written. Fable's pick was to ask the box again. That needs the archive's plan token and choice again, so it is left open. The window is one round trip.
- Box-1's reports never move the host's record of a session restored here back to box-1. They only update the host's view of box-1's copy, which is used only while the record names box-1.

## D221: A context-only fork of a box session, made on that box, survives a link loss or a host restart (FORK manager; picked by a Fable 5.2 high subagent, 2026-11-16). Provisional
- The host first journals the fork in `<state>/carry/forks/<op>.json` (`ForkJournal.context`: the title, the turn to fork before, the agent, the model, and the box's operation id once known). The relayed ForkSession carries the key `host-fork-<host op id>`. **Reverse:** a random key kept in the journal.
- On link-up after a link loss or a restart, the host watches the known operation, or asks again with the same key if the id is unknown. When it ends, the host writes its records once; an existing fork origin skips the write. A refusal or a failed fork drops the journal. An operation the box no longer knows drops the journal, and a log line says to look on the box.
- Known gap, accepted: if the box restarted before its first answer reached the host, asking again can make a second fork, because the box keeps keys in memory only. This needs a crash on both sides and leaves a visible extra fork that can be deleted. It closes when the box keeps the key in the fork's own record and dedupes on it.

## D224: A box's copy of a session restored on this Mac is compared before it is released, and kept when it has more (FORK manager; picked by a Fable 5.2 high subagent, 2026-11-16). Provisional
- When the link comes back to a machine that still holds the archived copy of a session restored here from the host's mirror, the host compares before it releases. The copy has nothing more only if three things hold. Its turns match the restored ones, prefix-equal by identity. Its archive summary lists no unpushed subjects and no changed files. Each repo's last pushed sha is the restored sha or an ancestor of it, checked in the host's clone. An RPC error, a missing summary or field, or an unknown sha counts as "has more", and the copy is kept.
- First slice: with nothing more the copy is released (trashed, records dropped). With more, the copy and the release journal are kept, the log says what the machine has more of, and the next link-up compares again. Releasing is a delete, so it waits for the comparison.
- Next slice: what the copy has more of is carried, with the move carry, into `<session folder>/.humata/from-<machine>-<op>/` on the host, outside any repo. The carried records stay plain files and are never registered. The live clones and conversation are not touched. The session gets one notice that names what was brought and offers to discard the side folder. Release follows only once the carry is unpacked and checked on the host and the notice is written. If the machine reports its clones already freed, the same carry brings what remains, and the notice names the summarized work that is gone.
- Deferred: Release carrying the compared fingerprint (turn count and last turn id), so the machine refuses if its copy changed since. It needs a proto change.
- Built in the first slice: the machine's turns are read through ReadMirror relayed to it, because ListTurns returns nothing on the rig. Any changed file in the machine's summary counts as more, the rule taken literally, so a copy with pushed changes is kept until the carry slice. Freed clones aren't detected yet: no RPC says so, so such a copy is kept. A rewrite of the machine's conversation file while it is being read is retried once; a second rewrite counts as more. **Reverse:** compare the summary's files with the restored clone.
- D218 pick (2): a remote archive whose link is lost says the machine may still have archived the session and that Humata checks when it reconnects. Pick (5): the archive row takes the pointers from the machine's own GetSession answer when it has any, and the last reported ones otherwise.

## D230: What a box's copy has more of is carried beside the restored session before the copy is released (FORK manager; picked by a Fable 5.2 high subagent, 2026-11-16). Provisional
- When the comparison of D224 finds more on the machine, or can't tell, the host carries the machine's copy into `<session folder>/.humata/from-<machine>-<op>/`, where `<op>` is the restore's own operation, so a retry lands in the same place. It is built in `from-<machine>-<op>.partial`, checked against the pack's manifest, given a `NOTICE.md`, and only then renamed; the final name means complete. A `.partial` is removed at the start of each attempt. **Reverse:** a fresh id kept in the release journal.
- Layout: the machine's stage is kept as packed (`stage/manifest.json`, `stage/records.sqlite`, `stage/agent/`, `stage/tabs/`), so the hashes can be checked again later; the machine's session folder, when it still has one, is `folder/`. Nothing is registered and the live clones and conversation are untouched. **Reverse:** a rename later is a plain move.
- The copy is always carried on "more", including when the only finding is summarized work: the turn comparison is by marks, not bytes, and the machine's records are the only evidence of that work. **Reverse:** skip the conversation when a byte fingerprint matches (needs D224's deferred proto change).
- The notice is `NOTICE.md` plus one log line. It names what was brought, what the comparison found, and each repo's changed files and unpushed subjects from the machine's summary. Work is called gone only when no session folder came along; a repo pushed past the restore is on the remote, not lost. Discarding is deleting the folder. Follow-up: one `[Humata]` line at the session's next agent start points at the notice. **Reverse:** a Session field and a discard RPC once main changes the protos.
- A failed carry or check keeps the machine's copy and the journal, removes the `.partial`, unstages on the machine, and tries again at the next link-up or host start, with no timer. Release happens only after the rename.
